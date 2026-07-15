//! Provider-name/icon-name completion for `iconify = "..."` values,
//! backed by a disk cache of api.iconify.design's collection listings.
//!
//! Warmed entirely in the background ([`warm_provider_caches`]) - never
//! from `completion()` itself, which only ever reads whatever is already
//! on disk ([`cached_names`]). If a provider's cache isn't warm yet,
//! completion for it is simply empty this time; there's no blocking
//! network call hiding inside a completion request.

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::notification::Progress;
use tower_lsp::lsp_types::request::WorkDoneProgressCreate;
use tower_lsp::lsp_types::{
    NumberOrString, ProgressParams, ProgressParamsValue, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressCreateParams, WorkDoneProgressEnd,
};
use tower_lsp::Client;

fn collection_cache_path(workspace_root: &Path, provider: &str) -> PathBuf {
    workspace_root.join(".cache").join("guicons").join("_collections").join(format!("{provider}.json"))
}

fn collection_url(provider: &str) -> String {
    format!("https://api.iconify.design/collection?prefix={provider}&pretty=0")
}

/// Icon names already cached on disk for `provider`, or `None` if its
/// collection hasn't been warmed (or fetched) yet.
pub fn cached_names(workspace_root: &Path, provider: &str) -> Option<Vec<String>> {
    let content = fs::read_to_string(collection_cache_path(workspace_root, provider)).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(flatten_names(&json))
}

/// `{"uncategorized": [...], "categories": {"...": [...]}}` (the shape
/// `api.iconify.design/collection` responds with) flattened into one list
/// - which field(s) a given icon set uses varies per provider.
fn flatten_names(json: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(list) = json.get("uncategorized").and_then(serde_json::Value::as_array) {
        names.extend(list.iter().filter_map(serde_json::Value::as_str).map(str::to_string));
    }
    if let Some(categories) = json.get("categories").and_then(serde_json::Value::as_object) {
        for list in categories.values().filter_map(serde_json::Value::as_array) {
            names.extend(list.iter().filter_map(serde_json::Value::as_str).map(str::to_string));
        }
    }
    names
}

async fn warm_one(workspace_root: &Path, provider: &str) {
    let dest = collection_cache_path(workspace_root, provider);
    if dest.exists() {
        return;
    }
    let url = collection_url(provider);
    let _ = tokio::task::spawn_blocking(move || guicons_net::download(&url, &dest)).await;
}

/// Spawns background warmup for every name in `providers` (deduplicated
/// by the caller), reporting a `window/workDoneProgress` span around it.
/// LSP clients are required to tolerate progress notifications for a
/// token they never asked for, so this is safe to send unconditionally
/// even if the client ends up ignoring it.
pub fn warm_provider_caches(client: Client, workspace_root: PathBuf, providers: Vec<String>) {
    if providers.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let token = NumberOrString::String("guicons/iconify-warmup".to_string());
        let _ = client
            .send_request::<WorkDoneProgressCreate>(WorkDoneProgressCreateParams { token: token.clone() })
            .await;
        client
            .send_notification::<Progress>(ProgressParams {
                token: token.clone(),
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(WorkDoneProgressBegin {
                    title: "guicons: warming up icon name cache".to_string(),
                    cancellable: Some(false),
                    message: Some(format!("{} provider(s)", providers.len())),
                    percentage: None,
                })),
            })
            .await;

        for provider in &providers {
            warm_one(&workspace_root, provider).await;
        }

        client
            .send_notification::<Progress>(ProgressParams {
                token,
                value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(WorkDoneProgressEnd { message: None })),
            })
            .await;
    });
}

/// What the text already typed inside an `iconify = "..."` value resolves
/// to - either still picking a provider (no `:` yet) or, once one's been
/// typed, picking a name within it. Each variant carries the byte range
/// completion should actually replace (only the relevant sub-part of what
/// was typed, not the whole value).
pub enum IconifyContext {
    Provider { range: Range<usize>, typed: String },
    Name { provider: String, range: Range<usize>, typed: String },
}

/// Splits `typed` (the text already in the quotes, from
/// [`crate::manifest_text::iconify_field_at`]) on its first `:`, mapping
/// the byte offsets back onto `full_span` - the "resolve the package"
/// layer, separate from resolving names *within* one (see
/// [`cached_names`]).
pub fn classify(full_span: Range<usize>, typed: &str) -> IconifyContext {
    match typed.split_once(':') {
        None => IconifyContext::Provider { range: full_span, typed: typed.to_string() },
        Some((provider, name_typed)) => {
            let name_start = full_span.start + provider.len() + 1;
            IconifyContext::Name {
                provider: provider.to_string(),
                range: name_start..full_span.end,
                typed: name_typed.to_string(),
            }
        }
    }
}
