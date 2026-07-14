use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn find_workspace_root(manifest_path: &Path) -> Option<PathBuf> {
    let start = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    find_workspace_root_from(start)
}

pub(crate) fn resolve_workspace_path(workspace_root: &Path, value: &str) -> PathBuf {
    resolve_entry_path(workspace_root, value)
}

pub(crate) fn resolve_entry_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

pub fn canonicalize_or_self(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Walks up from `start` looking for the nearest `Cargo.toml` that declares
/// `[workspace]` or `[package]`, i.e. the crate/workspace root.
///
/// Public because it's needed by more than just this parser: `guicons`'s
/// `build.rs` codegen and `guicons-fetch`'s icon cache both need to find the
/// same root (to locate `icons.gui.toml`/`.cache/guicons` respectively) and
/// previously each carried their own copy of this exact walk.
pub fn find_workspace_root_from(start: &Path) -> Option<PathBuf> {
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
