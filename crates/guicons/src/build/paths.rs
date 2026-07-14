use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn out_dir() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR should be set"))
}

pub(crate) fn workspace_manifest_path() -> PathBuf {
    find_workspace_root_from_cwd()
        .unwrap_or_else(|| panic!("Could not find workspace root from {}", current_dir().display()))
        .join("icons.gui.toml")
}

pub(crate) fn find_workspace_root_from_cwd() -> Option<PathBuf> {
    find_workspace_root_from(&current_dir())
}

pub(crate) fn canonicalize_existing(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|e| panic!("Failed to resolve {}: {e}", path.display()))
}

pub(crate) fn canonicalize_or_self(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn current_dir() -> PathBuf {
    env::current_dir().expect("Current directory should be available")
}

fn find_workspace_root_from(start: &Path) -> Option<PathBuf> {
    let mut current = canonicalize_or_self(start);
    loop {
        let cargo_toml = current.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = fs::read_to_string(&cargo_toml).ok()?;
            if content.contains("[workspace]") || content.contains("[package]") {
                return Some(current);
            }
        }
        current = current.parent()?.to_path_buf();
    }
}
