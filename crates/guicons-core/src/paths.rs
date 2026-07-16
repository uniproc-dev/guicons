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

/// The `icons.gui.toml` governing `rust_file` - its own crate's manifest
/// (via `find_workspace_root_from`, which stops at the nearest ancestor
/// `Cargo.toml` rather than climbing to a cargo *workspace* root), not
/// any manifest that happens to exist elsewhere. In a multi-crate
/// workspace, a `.rs` file must only ever resolve against its own
/// crate's manifest - never a different crate's, even if one is sitting
/// right next to it (a real bug once, in `guicons-lsp`'s hover).
/// `None` if there's no `Cargo.toml` above `rust_file`, or no
/// `icons.gui.toml` beside it.
pub fn manifest_path_for_rust_file(rust_file: &Path) -> Option<PathBuf> {
    let crate_root = find_workspace_root_from(rust_file.parent()?)?;
    let manifest = crate_root.join("icons.gui.toml");
    manifest.is_file().then_some(manifest)
}
