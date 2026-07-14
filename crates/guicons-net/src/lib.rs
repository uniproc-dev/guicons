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

#[derive(Debug)]
pub struct DownloadError(String);

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DownloadError {}

pub fn iconify_cache_path(start: &Path, id: &str) -> PathBuf {
    let (provider, name) = id
        .split_once(':')
        .unwrap_or_else(|| panic!("Iconify source must be `<set>:<name>`, got `{id}`"));
    workspace_cache_dir(start).join(provider).join(format!("{name}.svg"))
}

pub fn url_cache_path(start: &Path, url: &str) -> PathBuf {
    workspace_cache_dir(start).join("url").join(format!("{}.svg", sha256_hex(url)))
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
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let response = ureq::get(url)
        .call()
        .map_err(|e| DownloadError(format!("Failed to download `{url}`: {e}")))?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| DownloadError(format!("Failed to read `{url}`: {e}")))?;
    fs::write(dest, bytes)
        .map_err(|e| DownloadError(format!("Failed to write cache file {}: {e}", dest.display())))
}

pub fn iconify_url(id: &str) -> String {
    let (set, name) = id
        .split_once(':')
        .unwrap_or_else(|| panic!("Iconify source must be `<set>:<name>`, got `{id}`"));
    format!("https://api.iconify.design/{set}/{name}.svg")
}

fn workspace_cache_dir(start: &Path) -> PathBuf {
    find_workspace_root_from(start)
        .unwrap_or_else(|| canonicalize_or_self(start))
        .join(".cache")
        .join("guicons")
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
