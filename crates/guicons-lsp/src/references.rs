//! `textDocument/references` ("find usages") triggered from a manifest
//! entry - the reverse direction of [`crate::goto_definition`]'s `.rs` ->
//! manifest jump. Only meaningful starting from a `.gui.toml` file: an
//! entry itself has no "definition" to jump to from `.rs`, but it very
//! much has usages worth finding (every `icon!`/`icon_key!`/`icon_data!`
//! call site that resolves to it).

use crate::manifest_text::offset_line_overlaps;
use crate::position::LineIndex;
use crate::{Backend, DEFAULT_SKIP_DIRS};
use guicons_core::rust_macro::all_macro_calls;
use guicons_core::selector::{parse_selector, IconSelector};
use std::path::{Path, PathBuf};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn references_impl(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
        if !path.to_string_lossy().ends_with(".gui.toml") {
            return Ok(None);
        }
        let Some(text) = self.document_text(&uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };

        // Looked up against the already-scanned `self.manifests` (keyed by
        // *root* manifest path) rather than re-parsing `path` alone - an
        // entry declared in a `[link]`d file still needs its owning root's
        // full entry list to be found by `file() == path`, and the match
        // also hands back which root every candidate `.rs` file has to
        // resolve through to count as a real usage.
        let manifests = self.manifests.read().await;
        let target = manifests.iter().find_map(|(root, manifest)| {
            manifest
                .entries()
                .iter()
                .find(|entry| entry.file() == path && offset_line_overlaps(&text, offset, entry.span()))
                .map(|entry| (root.clone(), entry.family().to_string(), entry.size(), entry.variant().map(str::to_string)))
        });
        drop(manifests);
        let Some((root_manifest, family, size, variant)) = target else { return Ok(None) };

        let Some(workspace_root) = self.workspace_root().await else { return Ok(None) };
        let extra_skip_dirs = self.extra_skip_dirs.read().await.clone();
        let mut locations = Vec::new();
        for rs_file in find_rust_files(&workspace_root, &extra_skip_dirs) {
            let Some(governing_manifest) = guicons_core::manifest_path_for_rust_file(&rs_file) else { continue };
            if guicons_core::canonicalize_or_self(&governing_manifest) != root_manifest {
                continue;
            }

            let Ok(rs_uri) = Url::from_file_path(&rs_file) else { continue };
            let Some(rs_text) = self.document_text_or_disk(&rs_uri, &rs_file).await else { continue };

            let rs_index = LineIndex::new(&rs_text);
            for site in all_macro_calls(&rs_text) {
                let Ok(IconSelector::FamilyVariant { family: f, size: s, variant: v }) = parse_selector(&site.arg_text) else {
                    continue;
                };
                if f != family || s != size || v != variant {
                    continue;
                }
                let range = rs_index.range(&rs_text, site.arg_range.clone());
                locations.push(Location::new(rs_uri.clone(), range));
            }
        }

        Ok(Some(locations))
    }
}

/// Every `.rs` file under `root`, same `DEFAULT_SKIP_DIRS`/`extra_skip_dirs`
/// pruning as [`find_manifest_files`] - kept separate rather than
/// generalizing that helper's file-name filter, since the two callers'
/// extensions (`icons.gui.toml` vs `.rs`) aren't worth a shared parameter
/// for a single-line predicate.
pub(crate) fn find_rust_files(root: &Path, extra_skip_dirs: &[String]) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_skipped_dir(entry, extra_skip_dirs))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file() && entry.path().extension().is_some_and(|ext| ext == "rs"))
        .map(walkdir::DirEntry::into_path)
        .collect()
}

fn is_skipped_dir(entry: &walkdir::DirEntry, extra_skip_dirs: &[String]) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name();
    DEFAULT_SKIP_DIRS.iter().any(|skip| name == std::ffi::OsStr::new(skip))
        || extra_skip_dirs.iter().any(|skip| name == std::ffi::OsStr::new(skip.as_str()))
}
