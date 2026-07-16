use crate::manifest_text::{family_header_at, keyword_at, provider_name_at};
use crate::position::LineIndex;
use crate::{describe_source, Backend};
use guicons_core::{IconEntry, IconManifest};
use std::path::Path;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

/// Hover text for a raw `"provider:name"` iconify literal in a `.rs`
/// `icon!(...)` call - links out to the icon's own page on Iconify's
/// site (where it can actually be looked at) rather than just restating
/// that it's an iconify id, which is already visible from the call site
/// itself and tells the reader nothing new.
pub(crate) fn iconify_literal_hover(id: &str) -> String {
    match id.split_once(':') {
        Some((prefix, name)) => format!("**{id}** - [view on Iconify](https://icon-sets.iconify.design/{prefix}/{name}/)"),
        None => format!("**{id}** - not in `provider:name` form, will never resolve"),
    }
}

/// Key/family/variant/size/source markdown lines for a single entry -
/// shared by the `.rs`-side and TOML-side hover, which both land on the
/// same entry via different routes (a macro call's selector vs. cursor
/// position in the manifest text) but describe it identically once found.
fn entry_hover_lines(entry: &IconEntry, manifest: &IconManifest) -> Vec<String> {
    let mut lines = vec![format!("**{}**", entry.key())];
    lines.push(format!("- family: `{}`", entry.family()));
    if let Some(variant) = entry.variant() {
        lines.push(format!("- variant: `{variant}`"));
    }
    if let Some(size) = entry.size() {
        lines.push(format!("- size: `{size}`"));
    }
    lines.push(format!("- source: {}", describe_source(entry.source(), manifest)));
    lines
}

impl Backend {
    /// Hover for `icon!`/`icon_key!`/`icon_data!` call sites in a `.rs`
    /// document - the Rust-side counterpart to the TOML `hover()` body.
    /// Unlike a `.gui.toml` document, a `.rs` file has no manifest of its
    /// own - but it does belong to a specific *crate*, and that's exactly
    /// what picks the manifest: `guicons-macros` itself resolves
    /// `icon!(...)` at compile time against `CARGO_MANIFEST_DIR/icons.gui.toml`
    /// (the crate root, not the cargo *workspace* root - a distinction
    /// `find_workspace_root_from` already embodies, since it stops at the
    /// nearest ancestor `Cargo.toml` rather than climbing to one with
    /// `[workspace]`). In a multi-crate workspace, a `.rs` file must only
    /// ever resolve against *its own* crate's manifest - never some other
    /// crate's, even if one happens to be sitting in `self.manifests`.
    pub(crate) async fn hover_rust(&self, path: &Path, uri: &Url, position: Position) -> Result<Option<Hover>> {
        let Some(text) = self.document_text(uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };

        let Some(site) = guicons_core::rust_macro::macro_call_at(&text, offset) else { return Ok(None) };
        let Ok(selector) = guicons_core::selector::parse_selector(&site.arg_text) else { return Ok(None) };
        let range = Some(index.range(&text, site.arg_range));

        match selector {
            guicons_core::selector::IconSelector::Iconify(id) => Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: iconify_literal_hover(&id),
                }),
                range,
            })),
            guicons_core::selector::IconSelector::FamilyVariant { family, size, variant } => {
                let Some(manifest) = self.manifest_for_rust_file(path).await else { return Ok(None) };
                let Some(entry) = manifest.entry_for_family_variant(&family, size, variant.as_deref()) else {
                    return Ok(None);
                };

                let lines = entry_hover_lines(entry, &manifest);

                Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: lines.join("\n") }),
                    range,
                }))
            }
        }
    }

    pub(crate) async fn hover_impl(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
        if path.extension().is_some_and(|ext| ext == "rs") {
            return self.hover_rust(&path, &uri, position).await;
        }
        let Some(text) = self.document_text(&uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };

        if let Some((keyword, doc)) = keyword_at(&text, offset) {
            let value = format!("**{keyword}**\n\n{}\n\n```toml\n{}\n```", doc.description, doc.example);
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }),
                range: None,
            }));
        }

        let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);

        if let Some(name) = provider_name_at(&text, offset) {
            if let Some(schema) = manifest.provider(&name) {
                let is_builtin = guicons_core::builtin_provider_names().any(|builtin| builtin == name);
                let overridden = text.contains(&format!("[providers.{name}.override]"));
                let origin = match (is_builtin, overridden) {
                    (true, true) => "built-in provider, overridden in this file",
                    (true, false) => "built-in provider",
                    (false, _) => "custom provider",
                };
                let variants = if schema.variants.is_empty() { "(none)".to_string() } else { schema.variants.join(", ") };
                let sizes = if schema.sizes.is_empty() {
                    "(none)".to_string()
                } else {
                    schema.sizes.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
                };
                let value = format!("**{name}** - {origin}\n\n- variants: {variants}\n- sizes: {sizes}");
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }),
                    range: None,
                }));
            }
        }

        if let Some((family, size)) = family_header_at(&text, offset) {
            let variants: Vec<_> = manifest
                .entries()
                .iter()
                .filter(|entry| entry.file() == path && entry.family() == family && (size.is_none() || entry.size() == size))
                .collect();
            if !variants.is_empty() {
                let mut lines = vec![format!("**{family}**")];
                for entry in variants {
                    let variant = entry.variant().unwrap_or("(no variant)");
                    let size_suffix = entry.size().map(|s| format!(" @ {s}")).unwrap_or_default();
                    lines.push(format!("- `{variant}`{size_suffix}: {}", describe_source(entry.source(), &manifest)));
                }
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: lines.join("\n"),
                    }),
                    range: None,
                }));
            }
        }

        let Some(entry) = manifest
            .entries()
            .iter()
            .find(|entry| entry.file() == path && entry.span().contains(&offset))
        else {
            return Ok(None);
        };

        let lines = entry_hover_lines(entry, &manifest);

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: lines.join("\n"),
            }),
            range: Some(index.range(&text, entry.span())),
        }))
    }
}
