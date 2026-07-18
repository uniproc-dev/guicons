//! `textDocument/rename` ("rename symbol") triggered from a manifest
//! entry's `[family]`/`[family.N]` header - renames the family across
//! every header declaring it (in `[link]`d files too) and every
//! `icon!`/`icon_key!`/`icon_data!` call site across the workspace that
//! resolves to it, in one workspace-wide edit. Reuses
//! [`crate::references`]'s workspace-scoped `.rs` scan/match logic - the
//! set of call sites a rename has to touch is exactly the same set
//! `references_impl` already finds.
//!
//! Deliberately scoped to *family* renames only, and only a header that's
//! a single bracket segment (`[docker]`, `[docker.24]`) - a header can
//! also be written as dotted TOML table nesting (`[nav.bar]`, joined into
//! the compound family name `nav-bar`), and there's no unambiguous way to
//! redistribute an arbitrary new name typed by the user back across that
//! segment split. `prepare_rename_impl` simply refuses to offer rename at
//! all on a multi-segment header rather than guess.

use crate::position::LineIndex;
use crate::references::find_rust_files;
use crate::Backend;
use guicons_core::rust_macro::all_macro_calls;
use guicons_core::selector::{parse_selector, IconSelector};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use tower_lsp::jsonrpc::{Error, Result};
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn prepare_rename_impl(&self, params: TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
        if !path.to_string_lossy().ends_with(".gui.toml") {
            return Ok(None);
        }
        let Some(text) = self.document_text(&uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };
        let Some((_family, byte_range)) = single_segment_family_header_at(&text, offset) else { return Ok(None) };

        Ok(Some(PrepareRenameResponse::Range(index.range(&text, byte_range))))
    }

    pub(crate) async fn rename_impl(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_family = params.new_name.trim().to_string();
        if new_family.is_empty() || new_family.contains(['.', '/']) || new_family.chars().any(char::is_whitespace) {
            return Err(Error::invalid_params("family name can't be empty or contain `.`, `/`, or whitespace"));
        }

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
        if !path.to_string_lossy().ends_with(".gui.toml") {
            return Ok(None);
        }
        let Some(text) = self.document_text(&uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };
        let Some((old_family, _)) = single_segment_family_header_at(&text, offset) else { return Ok(None) };
        if old_family == new_family {
            return Ok(None);
        }

        // Matched against the document actually being edited (`path`), not
        // just "any manifest in the workspace declaring this family name" -
        // two unrelated crates can each have their own `docker` family, and
        // only the one the user is looking at should be touched.
        let manifests = self.manifests.read().await;
        let target = manifests
            .iter()
            .find(|(_, manifest)| manifest.entries().iter().any(|entry| entry.family() == old_family && entry.file() == path))
            .map(|(root, manifest)| {
                let files: HashSet<PathBuf> = manifest
                    .entries()
                    .iter()
                    .filter(|entry| entry.family() == old_family)
                    .map(|entry| entry.file().to_path_buf())
                    .collect();
                (root.clone(), files)
            });
        drop(manifests);
        let Some((root_manifest, declaring_files)) = target else { return Ok(None) };

        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        for file in declaring_files {
            // Reuse the request's own `uri` verbatim for the file the
            // rename was actually triggered from, rather than re-deriving
            // one from `file` (a canonicalized path) via
            // `Url::from_file_path` - the two can disagree byte-for-byte
            // even when they name the same file, if canonicalizing takes
            // a different route back out than the client's own URI did
            // (a temp-dir junction/symlink on some CI runners is exactly
            // this: canonicalize resolves through it, the client's URI
            // never did). A `changes` key the client's own open document
            // doesn't match is an edit that silently applies to nothing.
            let file_uri = if file == path { Some(uri.clone()) } else { Url::from_file_path(&file).ok() };
            let Some(file_uri) = file_uri else { continue };
            let Some(file_text) = self.document_text_or_disk(&file_uri, &file).await else { continue };
            let file_index = LineIndex::new(&file_text);
            let edits: Vec<TextEdit> = header_family_ranges(&file_text, &old_family)
                .into_iter()
                .map(|range| TextEdit::new(file_index.range(&file_text, range), new_family.clone()))
                .collect();
            if !edits.is_empty() {
                changes.entry(file_uri).or_default().extend(edits);
            }
        }

        let Some(workspace_root) = self.workspace_root().await else { return Ok(None) };
        let extra_skip_dirs = self.extra_skip_dirs.read().await.clone();
        for rs_file in find_rust_files(&workspace_root, &extra_skip_dirs) {
            let Some(governing_manifest) = guicons_core::manifest_path_for_rust_file(&rs_file) else { continue };
            if guicons_core::canonicalize_or_self(&governing_manifest) != root_manifest {
                continue;
            }
            let Ok(rs_uri) = Url::from_file_path(&rs_file) else { continue };
            let Some(rs_text) = self.document_text_or_disk(&rs_uri, &rs_file).await else { continue };
            let rs_index = LineIndex::new(&rs_text);

            let mut edits = Vec::new();
            for site in all_macro_calls(&rs_text) {
                let Ok(IconSelector::FamilyVariant { family, .. }) = parse_selector(&site.arg_text) else { continue };
                if family != old_family {
                    continue;
                }
                let Some(new_arg_text) = rename_family_in_arg_text(&site.arg_text, &old_family, &new_family) else { continue };
                edits.push(TextEdit::new(rs_index.range(&rs_text, site.arg_range.clone()), new_arg_text));
            }
            if !edits.is_empty() {
                changes.entry(rs_uri).or_default().extend(edits);
            }
        }

        Ok(Some(WorkspaceEdit { changes: Some(changes), ..Default::default() }))
    }
}

/// If the line at `offset` is a `[family]`/`[family.N]` header made of a
/// *single* bracket segment (an optional trailing numeric size segment
/// aside), returns the family name and the byte range of just that name
/// within the header - `None` for `[link]`/`[defaults]`/`[providers.*]`,
/// and `None` for a dotted multi-segment header (`[nav.bar]`), which this
/// module's rename deliberately doesn't support (see module doc comment).
fn single_segment_family_header_at(text: &str, offset: usize) -> Option<(String, Range<usize>)> {
    let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[offset..].find('\n').map(|i| offset + i).unwrap_or(text.len());
    let line = &text[line_start..line_end];
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() || inner == "link" || inner == "defaults" || inner.starts_with("providers") {
        return None;
    }

    let family_part = match inner.rsplit_once('.') {
        Some((rest, last)) if last.parse::<u16>().is_ok() => rest,
        _ => inner,
    };
    if family_part.is_empty() || family_part.contains('.') {
        return None;
    }

    let bracket_rel = line.find('[')?;
    let family_start = line_start + bracket_rel + 1;
    Some((family_part.to_string(), family_start..family_start + family_part.len()))
}

/// Every single-segment `[target_family]`/`[target_family.N]` header's
/// family-name byte range in `text` - a family's variants can be split
/// across more than one such header (different sizes, or just written as
/// separate tables), all of which a rename has to touch.
fn header_family_ranges(text: &str, target_family: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut pos = 0usize;
    for raw_line in text.split_inclusive('\n') {
        let line_start = pos;
        pos += raw_line.len();
        let line = raw_line.trim_end_matches(['\n', '\r']);
        let trimmed = line.trim();
        let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else { continue };
        if inner.is_empty() || inner == "link" || inner == "defaults" || inner.starts_with("providers") {
            continue;
        }
        let family_part = match inner.rsplit_once('.') {
            Some((rest, last)) if last.parse::<u16>().is_ok() => rest,
            _ => inner,
        };
        if family_part != target_family {
            continue;
        }
        let Some(bracket_rel) = line.find('[') else { continue };
        let family_start = line_start + bracket_rel + 1;
        ranges.push(family_start..family_start + family_part.len());
    }
    ranges
}

/// Rewrites a macro call's argument text with `old_family` replaced by
/// `new_family`, preserving everything else verbatim - the `.variant`/
/// `.size.variant` suffix (dotted-path form), the `/variant`/`/size/variant`
/// suffix (quoted `"family/variant"` form), and any trailing
/// `, module = ident` clause. `None` only if `arg_text` doesn't actually
/// start with `old_family` the way the caller's already-parsed selector
/// claimed it would - defensive, not expected to trigger in practice.
fn rename_family_in_arg_text(arg_text: &str, old_family: &str, new_family: &str) -> Option<String> {
    let (selector_part, module_part) = match arg_text.split_once(',') {
        Some((before, after)) => (before, Some(after)),
        None => (arg_text, None),
    };
    let leading_ws_len = selector_part.len() - selector_part.trim_start().len();
    let trailing_ws_len = selector_part.len() - selector_part.trim_end().len();
    let leading_ws = &selector_part[..leading_ws_len];
    let trailing_ws = &selector_part[selector_part.len() - trailing_ws_len..];
    let trimmed = selector_part[leading_ws_len..selector_part.len() - trailing_ws_len].to_string();

    let new_trimmed = if let Some(inner) = trimmed.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        let rest = inner.strip_prefix(old_family)?;
        if !(rest.is_empty() || rest.starts_with('/')) {
            return None;
        }
        format!("\"{new_family}{rest}\"")
    } else {
        let rest = trimmed.strip_prefix(old_family)?;
        if !(rest.is_empty() || rest.starts_with('.')) {
            return None;
        }
        format!("{new_family}{rest}")
    };

    let new_selector_part = format!("{leading_ws}{new_trimmed}{trailing_ws}");
    Some(match module_part {
        Some(module) => format!("{new_selector_part},{module}"),
        None => new_selector_part,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_a_bare_family() {
        assert_eq!(rename_family_in_arg_text("docker", "docker", "container").as_deref(), Some("container"));
    }

    #[test]
    fn renames_a_family_keeping_its_variant() {
        assert_eq!(
            rename_family_in_arg_text("docker.filled", "docker", "container").as_deref(),
            Some("container.filled")
        );
    }

    #[test]
    fn renames_a_family_keeping_size_and_variant() {
        assert_eq!(
            rename_family_in_arg_text("docker.24.filled", "docker", "container").as_deref(),
            Some("container.24.filled")
        );
    }

    #[test]
    fn renames_a_family_keeping_a_trailing_module_clause() {
        assert_eq!(
            rename_family_in_arg_text("docker.filled, module = icons2", "docker", "container").as_deref(),
            Some("container.filled, module = icons2")
        );
    }

    #[test]
    fn renames_a_quoted_literal_family_keeping_its_variant() {
        assert_eq!(
            rename_family_in_arg_text("\"docker/filled\"", "docker", "container").as_deref(),
            Some("\"container/filled\"")
        );
    }

    #[test]
    fn does_not_rename_a_family_that_only_shares_a_prefix() {
        assert_eq!(rename_family_in_arg_text("docker-compose.filled", "docker", "container"), None);
    }

    #[test]
    fn single_segment_family_header_finds_the_plain_case() {
        let text = "[docker]\nfile = \"docker.svg\"\n";
        let (family, range) = single_segment_family_header_at(text, 2).expect("a family header");
        assert_eq!(family, "docker");
        assert_eq!(&text[range], "docker");
    }

    #[test]
    fn single_segment_family_header_strips_a_trailing_size_segment() {
        let text = "[docker.24]\nfile = \"docker.svg\"\n";
        let (family, range) = single_segment_family_header_at(text, 2).expect("a family header");
        assert_eq!(family, "docker");
        assert_eq!(&text[range], "docker");
    }

    #[test]
    fn single_segment_family_header_refuses_a_dotted_multi_segment_header() {
        let text = "[nav.bar]\nfile = \"nav-bar.svg\"\n";
        assert_eq!(single_segment_family_header_at(text, 2), None);
    }

    #[test]
    fn single_segment_family_header_ignores_non_entry_tables() {
        assert_eq!(single_segment_family_header_at("[link]\nincludes = []\n", 2), None);
        assert_eq!(single_segment_family_header_at("[defaults]\nroot = \"x\"\n", 2), None);
        assert_eq!(single_segment_family_header_at("[providers.fluent]\n", 2), None);
    }

    #[test]
    fn header_family_ranges_finds_every_header_sharing_the_family_across_sizes() {
        let text = "[docker]\nfile = \"a.svg\"\n\n[docker.24]\nfile = \"b.svg\"\n";
        let ranges = header_family_ranges(text, "docker");
        assert_eq!(ranges.len(), 2, "{ranges:?}");
        for range in &ranges {
            assert_eq!(&text[range.clone()], "docker");
        }
    }
}
