use std::path::{Path, PathBuf};

/// Common noise directories that are never worth descending into looking
/// for a manifest - mirrors `guicons-lsp`'s own list (kept separate/
/// duplicated rather than shared, so this crate doesn't need to touch
/// `guicons-lsp`'s tests to add this).
const DEFAULT_SKIP_DIRS: &[&str] = &[
    "target",
    ".git",
    ".hg",
    ".svn",
    ".cache",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "out",
    "bin",
    "obj",
    ".idea",
    ".vscode",
    ".vs",
    "venv",
    ".venv",
    "__pycache__",
    ".next",
    ".nuxt",
    "coverage",
    ".terraform",
    ".gradle",
];

/// Recursively finds every manifest under `root` - matched by the
/// `.gui.toml` suffix, not a fixed name like `icons.gui.toml`. That's a
/// convention (what `guicons init`/`guicons-cli` name the *root* manifest
/// they scaffold, and what `manifest_path_for_rust_file` looks for beside
/// a crate's `Cargo.toml`), not a hard rule the format itself enforces -
/// nothing stops a project from calling its root manifest `app.gui.toml`,
/// and `[link] includes` files are never `icons.gui.toml` themselves
/// (that'd make every crate's root manifest ambiguous with any file it
/// includes). `[link]`d files aren't collected as separate *root*
/// candidates here regardless of name, since `load_icon_manifest` already
/// pulls those in as part of loading whichever root manifest references
/// them. Skips `DEFAULT_SKIP_DIRS` plus whatever `extra_skip_dirs` the
/// caller wants ignored.
pub fn find_manifest_files(root: &Path, extra_skip_dirs: &[String]) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_skipped_dir(entry, extra_skip_dirs))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file() && is_gui_toml(entry.file_name()))
        .map(walkdir::DirEntry::into_path)
        .collect()
}

fn is_gui_toml(file_name: &std::ffi::OsStr) -> bool {
    file_name.to_str().is_some_and(|name| name.ends_with(".gui.toml"))
}

fn is_skipped_dir(entry: &walkdir::DirEntry, extra_skip_dirs: &[String]) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name();
    DEFAULT_SKIP_DIRS.iter().any(|skip| name == std::ffi::OsStr::new(skip))
        || extra_skip_dirs.iter().any(|skip| name == std::ffi::OsStr::new(skip.as_str()))
}
