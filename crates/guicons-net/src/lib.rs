//! Icon cache resolution and network fetch, shared by `guicons-build`'s
//! codegen and `guicons-macros`' `icon!("set:name")` literal form. Both key
//! the same on-disk cache by the same string, so an icon later added to the
//! manifest keeps resolving to what an existing `icon!(...)` call already got.

use guicons_core::{canonicalize_or_self, find_workspace_root_from};
use sha2::{Digest, Sha256};
use std::env;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const ALLOW_NETWORK_ENV: &str = "GUICONS_ALLOW_NETWORK";

/// Overrides where [`global_cache_dir`] points - lets tests (and anyone
/// who wants their global icon-name cache somewhere other than the OS
/// default) redirect it without actually touching `~/.cache` or its
/// platform equivalents.
pub const CACHE_DIR_ENV: &str = "GUICONS_CACHE_DIR";

#[derive(Debug)]
pub struct DownloadError(String);

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DownloadError {}

pub fn iconify_cache_path(start: &Path, id: &str) -> PathBuf {
    let (provider, name) = split_iconify_id(id);
    workspace_cache_dir(start).join(provider).join(format!("{name}.svg"))
}

pub fn url_cache_path(start: &Path, url: &str) -> PathBuf {
    workspace_cache_dir(start).join("url").join(format!("{}.svg", sha256_hex(url)))
}

/// Iconify's *name listings* (`prefix -> [names]`, used to power
/// `iconify = "..."` completion) live in the OS-wide cache dir rather than
/// a workspace's `.cache/guicons` - the LSP and IDE plugin want the same
/// listing regardless of which repo happens to be open, so there's no
/// reason to redownload and restore it per repo. The actual icon SVGs
/// fetched via [`iconify_cache_path`]/[`ensure_cached`] stay
/// workspace-rooted, same as ever - those *are* build/preview output a
/// given repo owns (and `guicons-build`'s fixtures pre-seed them on
/// purpose), unlike a plain list of names.
pub fn global_collection_cache_path(provider: &str) -> PathBuf {
    global_cache_dir().join("_collections").join(format!("{provider}.json"))
}

/// Global-cache counterpart of [`cached_collection_names`].
pub fn global_cached_collection_names(provider: &str) -> Option<Vec<String>> {
    let content = fs::read_to_string(global_collection_cache_path(provider)).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(flatten_names(&json))
}

/// Global-cache counterpart of [`download_collection`].
pub fn global_download_collection(provider: &str) -> bool {
    let dest = global_collection_cache_path(provider);
    if dest.exists() {
        return true;
    }
    download(&collection_url(provider), &dest).is_ok()
}

/// Public so the IDE plugin can list what's already cached (e.g. which
/// providers' collections it can offer without a network round-trip)
/// without duplicating this OS-specific lookup on the Kotlin side.
pub fn global_cache_dir() -> PathBuf {
    if let Some(dir) = env::var_os(CACHE_DIR_ENV) {
        return PathBuf::from(dir);
    }
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".guicons-cache")).join("guicons")
}

pub fn ensure_cached(cache_path: &Path, url: &str) {
    if cache_path.exists() {
        return;
    }
    if env::var_os(ALLOW_NETWORK_ENV).is_none() {
        panic!(
            "Icon `{url}` is missing from cache at {}. Run a fetch step or set {ALLOW_NETWORK_ENV}=1.",
            cache_path.display()
        );
    }
    download(url, cache_path).unwrap_or_else(|e| panic!("{e}"));
}

/// Downloads `url` and writes it to `dest`, regardless of whether `dest`
/// already exists or `GUICONS_ALLOW_NETWORK` is set - callers that need
/// those checks (like `ensure_cached`) do them first.
pub fn download(url: &str, dest: &Path) -> Result<(), DownloadError> {
    let bytes = fetch_bytes(url)?;
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(dest, bytes)
        .map_err(|e| DownloadError(format!("Failed to write cache file {}: {e}", dest.display())))
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>, DownloadError> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| DownloadError(format!("Failed to download `{url}`: {e}")))?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| DownloadError(format!("Failed to read `{url}`: {e}")))?;
    Ok(bytes)
}

/// Fetches `id`'s SVG bytes for preview/browsing purposes - `None` on a
/// network failure. Deliberately stateless: this crate's only caller for
/// it is `guicons-ffi` (in turn only called by the IDE plugin), so any
/// caching belongs on that single caller's side rather than here - see
/// `IconPreviewCache` in the IDE plugin's Kotlin, which is what actually
/// avoids redownloading/rewriting on every scroll/repaint, TTL'd so a
/// long-running IDE process doesn't accumulate every SVG a user has ever
/// scrolled past. Never written to any repo's `.cache/guicons` either way
/// - an icon a user actually keeps still gets its own on-disk entry once
/// `guicons-build`/`guicons fetch` needs it, independently of this.
pub fn fetch_iconify_icon_preview(id: &str) -> Option<Vec<u8>> {
    fetch_bytes(&iconify_url(id)).ok()
}

pub fn iconify_url(id: &str) -> String {
    let (set, name) = split_iconify_id(id);
    format!("https://api.iconify.design/{set}/{name}.svg")
}

pub fn collection_cache_path(workspace_root: &Path, provider: &str) -> PathBuf {
    workspace_cache_dir(workspace_root).join("_collections").join(format!("{provider}.json"))
}

fn collection_url(provider: &str) -> String {
    format!("https://api.iconify.design/collection?prefix={provider}&pretty=0")
}

/// Icon names already cached on disk for `provider`, or `None` if its
/// collection hasn't been fetched yet.
pub fn cached_collection_names(workspace_root: &Path, provider: &str) -> Option<Vec<String>> {
    let content = fs::read_to_string(collection_cache_path(workspace_root, provider)).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(flatten_names(&json))
}

/// Downloads `provider`'s full collection listing into the cache if it
/// isn't there already - `true` if it's cached by the time this returns
/// (whether it already was, or this fetch succeeded), `false` on a
/// network failure.
pub fn download_collection(workspace_root: &Path, provider: &str) -> bool {
    let dest = collection_cache_path(workspace_root, provider);
    if dest.exists() {
        return true;
    }
    download(&collection_url(provider), &dest).is_ok()
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

/// Searches across every icon set via `api.iconify.design/search` - the
/// same endpoint iconify.design's own site search uses, so this mirrors
/// its behavior (fuzzy matching, etc.) rather than reimplementing any of
/// it here.
pub fn search_icons(query: &str, limit: usize) -> Result<Vec<String>, DownloadError> {
    let url = format!("https://api.iconify.design/search?query={query}&limit={limit}");
    let response = ureq::get(&url).call().map_err(|e| DownloadError(format!("Failed to search `{query}`: {e}")))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| DownloadError(format!("Failed to read search response for `{query}`: {e}")))?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| DownloadError(format!("Failed to parse search response for `{query}`: {e}")))?;
    let icons = json
        .get("icons")
        .and_then(serde_json::Value::as_array)
        .map(|list| list.iter().filter_map(serde_json::Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    Ok(icons)
}

fn split_iconify_id(id: &str) -> (&str, &str) {
    id.split_once(':')
        .unwrap_or_else(|| panic!("Iconify source must be `<set>:<name>`, got `{id}`"))
}

fn workspace_cache_dir(start: &Path) -> PathBuf {
    find_workspace_root_from(start)
        .unwrap_or_else(|| canonicalize_or_self(start))
        .join(".cache")
        .join("guicons")
}

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}
