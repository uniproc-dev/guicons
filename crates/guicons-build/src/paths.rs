use guicons_core::find_workspace_root_from;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn out_dir() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR should be set"))
}

pub(crate) fn workspace_manifest_path() -> PathBuf {
    find_workspace_root_from(&current_dir())
        .unwrap_or_else(|| panic!("Could not find workspace root from {}", current_dir().display()))
        .join("icons.gui.toml")
}

pub(crate) fn canonicalize_existing(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| panic!("Failed to resolve {}: {e}", path.display()))
}

pub(crate) fn current_dir() -> PathBuf {
    env::current_dir().expect("Current directory should be available")
}
