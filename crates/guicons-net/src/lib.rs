//! Icon cache resolution and network fetch, shared by `guicons-build`'s
//! codegen (materializing icons declared in `icons.gui.toml`) and
//! `guicons-macros`' `icon!("set:name")` literal form (embedding an icon
//! that isn't declared in any manifest at all). Both key the same on-disk
//! cache by the exact same string (an iconify id, or a URL), so adding an
//! icon to the manifest later doesn't change what an existing `icon!(...)`
//! call site resolves to - they just happen to share a cache entry.

use guicons_core::{canonicalize_or_self, find_workspace_root_from};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const ALLOW_NETWORK_ENV: &str = "GUICONS_ALLOW_NETWORK";

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
