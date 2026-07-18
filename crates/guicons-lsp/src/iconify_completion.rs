//! Provider-name/icon-name completion for `iconify = "..."` values,
//! backed by a disk cache of api.iconify.design's collection listings
//! (the cache-path/parsing logic itself lives in `guicons_net`, shared
//! with `guicons-ffi`'s icon-browser use of the same cache). Rooted at the
//! OS-wide cache dir, not any one workspace - the same provider listing is
//! useful across every project a client has open, so there's no reason to
//! redownload and re-store it per repo.
//!
//! Warmed entirely in the background ([`warm_provider_caches`]) - never
//! from `completion()` itself, which only ever reads whatever is already
//! on disk ([`cached_names`]). If a provider's cache isn't warm yet,
//! completion for it is simply empty this time; there's no blocking
//! network call hiding inside a completion request.

use std::ops::Range;
use tower_lsp::lsp_types::notification::Progress;
use tower_lsp::lsp_types::request::WorkDoneProgressCreate;
use tower_lsp::lsp_types::{
    NumberOrString, ProgressParams, ProgressParamsValue, WorkDoneProgress, WorkDoneProgressBegin,
    WorkDoneProgressCreateParams, WorkDoneProgressEnd,
};
use tower_lsp::Client;

/// Icon names already cached on disk for `provider`, or `None` if its
/// collection hasn't been warmed (or fetched) yet.
pub fn cached_names(provider: &str) -> Option<Vec<String>> {
    guicons_net::global_cached_collection_names(provider)
}

async fn warm_one(provider: &str) {
    let provider = provider.to_string();
    let _ = tokio::task::spawn_blocking(move || guicons_net::global_download_collection(&provider)).await;
}

/// Spawns background warmup for every name in `providers` (deduplicated
/// by the caller), reporting a `window/workDoneProgress` span around it.
/// LSP clients are required to tolerate progress notifications for a
/// token they never asked for, so this is safe to send unconditionally
/// even if the client ends up ignoring it.
pub fn warm_provider_caches(client: Client, providers: Vec<String>) {
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
            warm_one(provider).await;
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
