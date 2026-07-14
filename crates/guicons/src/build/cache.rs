use super::paths::{current_dir, find_workspace_root_from_cwd};
use super::ALLOW_NETWORK_ENV;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) fn iconify_cache_path(id: &str) -> PathBuf {
    let (provider, name) = id
        .split_once(':')
        .unwrap_or_else(|| panic!("Iconify source must be `<set>:<name>`, got `{id}`"));
    workspace_cache_dir().join(provider).join(format!("{name}.svg"))
}

pub(crate) fn url_cache_path(url: &str) -> PathBuf {
    workspace_cache_dir().join("url").join(format!("{}.svg", sha256_hex(url)))
}

pub(crate) fn ensure_cached(cache_path: &Path, url: &str) {
    if cache_path.exists() {
        return;
    }
    if env::var_os(ALLOW_NETWORK_ENV).is_none() {
        panic!(
            "Icon `{url}` is missing from cache at {}. Run a fetch step or set {ALLOW_NETWORK_ENV}=1.",
            cache_path.display()
        );
    }
    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let response = ureq::get(url)
        .call()
        .unwrap_or_else(|e| panic!("Failed to download `{url}`: {e}"));
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("Failed to read `{url}`: {e}"));
    fs::write(cache_path, bytes)
        .unwrap_or_else(|e| panic!("Failed to write cache file {}: {e}", cache_path.display()));
}

pub(crate) fn iconify_url(id: &str) -> String {
    let (set, name) = id
        .split_once(':')
        .unwrap_or_else(|| panic!("Iconify source must be `<set>:<name>`, got `{id}`"));
    format!("https://api.iconify.design/{set}/{name}.svg")
}

fn workspace_cache_dir() -> PathBuf {
    find_workspace_root_from_cwd()
        .unwrap_or_else(current_dir)
        .join(".cache")
        .join("guicons")
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
