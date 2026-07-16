use crate::manifest_text::{include_target_at, offset_line_overlaps};
use crate::position::LineIndex;
use crate::Backend;
use guicons_core::IconEntrySource;
use std::path::Path;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    /// Goto-definition for `icon!`/`icon_key!`/`icon_data!` call sites in
    /// a `.rs` document - jumps to the entry's declaration in
    /// `icons.gui.toml` (or one of its `[link]`d files - `entry.file()`
    /// already tracks exactly which one, same as the TOML-side
    /// `goto_definition` body relies on). Nothing to jump to for a raw
    /// iconify literal - there's no manifest declaration for one.
    pub(crate) async fn goto_definition_rust(
        &self,
        path: &Path,
        uri: &Url,
        position: Position,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let Some(text) = self.document_text(uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };

        let Some(site) = guicons_core::rust_macro::macro_call_at(&text, offset) else { return Ok(None) };
        let Ok(guicons_core::selector::IconSelector::FamilyVariant { family, size, variant }) =
            guicons_core::selector::parse_selector(&site.arg_text)
        else {
            return Ok(None);
        };

        let Some(manifest) = self.manifest_for_rust_file(path).await else { return Ok(None) };
        let Some(entry) = manifest.entry_for_family_variant(&family, size, variant.as_deref()) else {
            return Ok(None);
        };

        let entry_file = entry.file().to_path_buf();
        let entry_span = entry.span();
        let Ok(target_uri) = Url::from_file_path(&entry_file) else { return Ok(None) };

        let Some(entry_text) = self.document_text_or_disk(&target_uri, &entry_file).await else { return Ok(None) };
        let entry_index = LineIndex::new(&entry_text);
        let range = entry_index.range(&entry_text, entry_span);

        Ok(Some(GotoDefinitionResponse::Scalar(Location::new(target_uri, range))))
    }

    pub(crate) async fn goto_definition_impl(&self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
        if path.extension().is_some_and(|ext| ext == "rs") {
            return self.goto_definition_rust(&path, &uri, position).await;
        }
        let Some(text) = self.document_text(&uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };

        let start_of_file = Range::new(Position::new(0, 0), Position::new(0, 0));

        if let Some(target) = include_target_at(&text, offset) {
            let base = path.parent().unwrap_or_else(|| Path::new("."));
            let resolved = guicons_core::canonicalize_or_self(&base.join(target));
            if let Ok(target_uri) = Url::from_file_path(&resolved) {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location::new(target_uri, start_of_file))));
            }
        }

        let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);
        let Some(entry) = manifest
            .entries()
            .iter()
            .find(|entry| entry.file() == path && offset_line_overlaps(&text, offset, entry.span()))
        else {
            return Ok(None);
        };
        if let IconEntrySource::File(source_path) = entry.source() {
            if let Ok(target_uri) = Url::from_file_path(source_path) {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location::new(target_uri, start_of_file))));
            }
        }

        Ok(None)
    }
}
