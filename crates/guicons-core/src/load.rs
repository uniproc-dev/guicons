//! Entry point for loading a manifest file (and any files it `[include]`s)
//! from disk.
//!
//! This is the only module that touches the filesystem or knows about
//! multiple files; parsing the contents of a single already-read document is
//! [`crate::parse`]'s job.

use crate::diagnostics::{Diagnostics, ManifestError};
use crate::model::{IconEntry, IconManifest, ProviderSchema};
use crate::parse::{collect_entries, parse_defaults, parse_providers, resolve_providers};
use crate::paths::{canonicalize_or_self, find_workspace_root, resolve_entry_path};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml_span::value::{Table, ValueInner};

/// Parse a manifest (and any files it `[include]`s), collecting every
/// problem found instead of stopping at the first one.
///
/// Always returns a best-effort [`IconManifest`]: entries that failed to
/// parse are simply omitted, everything else is kept. Check whether the
/// returned error list is empty to know if the manifest is fully valid.
pub fn load_icon_manifest(manifest_path: &Path) -> (IconManifest, Vec<ManifestError>) {
    let mut seen = Vec::new();
    let mut source_paths = Vec::new();
    let mut errors = Vec::new();
    let manifest =
        load_icon_manifest_inner(manifest_path, &mut seen, &mut source_paths, &mut errors, None);
    (manifest, errors)
}

/// Like [`load_icon_manifest`], but parses `content` for the root document
/// instead of reading `manifest_path` from disk - for editor tooling that
/// wants diagnostics against unsaved buffer content. Any `[include]`d files
/// are still read from disk as usual (they aren't the document being edited).
pub fn load_icon_manifest_from_str(
    manifest_path: &Path,
    content: &str,
) -> (IconManifest, Vec<ManifestError>) {
    let mut seen = Vec::new();
    let mut source_paths = Vec::new();
    let mut errors = Vec::new();
    let manifest = load_icon_manifest_inner(
        manifest_path,
        &mut seen,
        &mut source_paths,
        &mut errors,
        Some(content),
    );
    (manifest, errors)
}

/// Convenience wrapper for `build.rs`: parse the manifest and panic with
/// every collected error if it isn't fully valid.
pub fn load_icon_manifest_or_panic(manifest_path: &Path) -> IconManifest {
    let (manifest, errors) = load_icon_manifest(manifest_path);
    if !errors.is_empty() {
        let mut message = format!(
            "failed to load icon manifest ({} error{}):\n",
            errors.len(),
            if errors.len() == 1 { "" } else { "s" }
        );
        for error in &errors {
            message.push_str("  - ");
            message.push_str(&error.to_string());
            message.push('\n');
        }
        panic!("{message}");
    }
    manifest
}

fn empty_manifest(manifest_path: &Path) -> IconManifest {
    IconManifest {
        manifest_path: manifest_path.to_path_buf(),
        workspace_root: manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        source_paths: vec![manifest_path.to_path_buf()],
        entries: Vec::new(),
        providers: std::collections::HashMap::new(),
    }
}

fn load_icon_manifest_inner(
    manifest_path: &Path,
    seen: &mut Vec<PathBuf>,
    source_paths: &mut Vec<PathBuf>,
    errors: &mut Vec<ManifestError>,
    content_override: Option<&str>,
) -> IconManifest {
    let manifest_path = canonicalize_or_self(manifest_path);

    if seen.contains(&manifest_path) {
        errors.push(ManifestError {
            file: manifest_path.clone(),
            span: None,
            message: "recursive icon manifest include".to_string(),
        });
        return empty_manifest(&manifest_path);
    }
    seen.push(manifest_path.clone());
    source_paths.push(manifest_path.clone());

    let owned_content;
    let content = match content_override {
        Some(content) => content,
        None => match fs::read_to_string(&manifest_path) {
            Ok(content) => {
                owned_content = content;
                &owned_content
            }
            Err(e) => {
                errors.push(ManifestError {
                    file: manifest_path.clone(),
                    span: None,
                    message: format!("failed to read file: {e}"),
                });
                seen.pop();
                return empty_manifest(&manifest_path);
            }
        },
    };

    let mut root_value = match toml_span::parse(content) {
        Ok(value) => value,
        Err(e) => {
            errors.push(ManifestError {
                file: manifest_path.clone(),
                span: Some(e.span.into()),
                message: format!("TOML syntax error: {e}"),
            });
            seen.pop();
            return empty_manifest(&manifest_path);
        }
    };

    let root_span = root_value.span;
    let mut root_table = match root_value.take() {
        ValueInner::Table(table) => table,
        _ => {
            errors.push(ManifestError {
                file: manifest_path.clone(),
                span: Some(root_span.into()),
                message: "manifest root must be a table".to_string(),
            });
            seen.pop();
            return empty_manifest(&manifest_path);
        }
    };

    let manifest_dir = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let workspace_root = find_workspace_root(&manifest_path).unwrap_or_else(|| manifest_dir.clone());

    let defaults_value = root_table.remove("defaults");
    let defaults = {
        let mut diags = Diagnostics {
            file: &manifest_path,
            errors,
        };
        parse_defaults(defaults_value, &workspace_root, &manifest_dir, &mut diags)
    };

    let providers_value = root_table.remove("providers");
    let own_providers = {
        let mut diags = Diagnostics {
            file: &manifest_path,
            errors,
        };
        let declarations = parse_providers(providers_value, &mut diags);
        resolve_providers(declarations, &mut diags)
    };

    let mut entries = Vec::new();
    let mut providers = HashMap::new();
    collect_includes(&root_table, &manifest_path, seen, source_paths, errors, &mut entries, &mut providers);
    // A file's own `[providers.*]` take precedence over anything an
    // `[include]`d file declared under the same name.
    providers.extend(own_providers);

    let own_entries_start = entries.len();
    {
        let mut diags = Diagnostics {
            file: &manifest_path,
            errors,
        };
        collect_entries(
            Vec::new(),
            root_table,
            &workspace_root,
            &defaults,
            &mut diags,
            &mut entries,
        );
    }
    // `collect_entries` doesn't know which file it's parsing (that's the
    // whole point of the parse/load split) - entries from `[include]`d
    // files already have `file` set correctly by their own recursive
    // `load_icon_manifest_inner` call, so only stamp the ones this level
    // just added for its own root table.
    for entry in &mut entries[own_entries_start..] {
        entry.file = manifest_path.clone();
    }

    entries.sort_by(|a, b| a.key().cmp(b.key()));
    seen.pop();

    IconManifest {
        manifest_path,
        workspace_root,
        source_paths: source_paths.clone(),
        entries,
        providers,
    }
}

fn collect_includes(
    table: &Table<'_>,
    manifest_path: &Path,
    seen: &mut Vec<PathBuf>,
    source_paths: &mut Vec<PathBuf>,
    errors: &mut Vec<ManifestError>,
    entries_acc: &mut Vec<IconEntry>,
    providers_acc: &mut HashMap<String, ProviderSchema>,
) {
    let Some(include_value) = table.get("include") else {
        return;
    };
    let Some(include_table) = include_value.as_table() else {
        errors.push(ManifestError {
            file: manifest_path.to_path_buf(),
            span: Some(include_value.span.into()),
            message: "`[include]` must be a table".to_string(),
        });
        return;
    };

    let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));

    for (name, value) in include_table.iter() {
        let Some(path) = value.as_str() else {
            errors.push(ManifestError {
                file: manifest_path.to_path_buf(),
                span: Some(value.span.into()),
                message: format!("`include.{name}` must be a string"),
            });
            continue;
        };
        let child = resolve_entry_path(base, path);
        let child_manifest = load_icon_manifest_inner(&child, seen, source_paths, errors, None);
        entries_acc.extend(child_manifest.entries);
        providers_acc.extend(child_manifest.providers);
    }
}
