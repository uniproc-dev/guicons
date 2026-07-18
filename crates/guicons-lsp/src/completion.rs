use crate::iconify_completion::{self, IconifyContext};
use crate::manifest_text::{
    defaults_root, iconify_field_at, path_field_at, section_kind_at, word_prefix_span, PathFieldKind, SectionKind,
};
use crate::position::LineIndex;
use crate::Backend;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

/// Where `file`/`windows-ico` completion should look for candidates:
/// `defaults.root` (if declared) resolved against the workspace root,
/// same as `guicons_core` resolves it - falling back to the manifest's
/// own directory.
fn resolve_file_base_dir(manifest_path: &Path, text: &str) -> PathBuf {
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let Some(root) = defaults_root(text) else {
        return manifest_dir.to_path_buf();
    };
    let root_path = Path::new(&root);
    if root_path.is_absolute() {
        return root_path.to_path_buf();
    }
    guicons_core::find_workspace_root_from(manifest_path)
        .map(|workspace_root| workspace_root.join(root_path))
        .unwrap_or_else(|| manifest_dir.join(root_path))
}

impl Backend {
    pub(crate) async fn completion_impl(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
        let Some(text) = self.document_text(&uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };

        let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_prefix = text[line_start..offset].trim_start();

        // An explicit replacement range, instead of leaving the client to
        // guess a word boundary - which can otherwise reach back across
        // the previous line's newline when `offset` is at column 0.
        let replace_range = index.range(&text, word_prefix_span(&text, offset));
        let make_item = |name: String, detail: &'static str| CompletionItem {
            label: name.clone(),
            detail: Some(detail.to_string()),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range: replace_range, new_text: name })),
            ..Default::default()
        };

        if line_prefix.starts_with('[') && line_prefix.contains("providers.") {
            let items = guicons_core::builtin_provider_names()
                .map(|name| make_item(name.to_string(), "built-in provider"))
                .collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }

        if let Some((kind, quote_span, typed)) = path_field_at(&text, offset) {
            let base_dir = match kind {
                PathFieldKind::Includes => path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")),
                PathFieldKind::File => resolve_file_base_dir(&path, &text),
            };
            let allowed_extensions: &[&str] = match kind {
                PathFieldKind::Includes => &["toml"],
                PathFieldKind::File => &["svg", "png"],
            };
            let range = index.range(&text, quote_span);
            let mut items = Vec::new();
            if let Ok(read_dir) = std::fs::read_dir(&base_dir) {
                for dir_entry in read_dir.flatten() {
                    let name = dir_entry.file_name().to_string_lossy().into_owned();
                    if !name.starts_with(&typed) {
                        continue;
                    }
                    let is_dir = dir_entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
                    if !is_dir && !allowed_extensions.iter().any(|ext| name.ends_with(&format!(".{ext}"))) {
                        continue;
                    }
                    items.push(CompletionItem {
                        label: name.clone(),
                        kind: Some(if is_dir { CompletionItemKind::FOLDER } else { CompletionItemKind::FILE }),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: name })),
                        ..Default::default()
                    });
                }
            }
            return Ok(Some(CompletionResponse::Array(items)));
        }

        if let Some((quote_span, typed)) = iconify_field_at(&text, offset) {
            let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);

            let (items, is_incomplete) = match iconify_completion::classify(quote_span, &typed) {
                IconifyContext::Provider { range, typed } => {
                    let range = index.range(&text, range);
                    let mut names: BTreeSet<String> = guicons_core::builtin_provider_names().map(str::to_string).collect();
                    names.extend(manifest.provider_names().map(str::to_string));
                    let items = names
                        .into_iter()
                        .filter(|name| name.starts_with(&typed))
                        .map(|name| CompletionItem {
                            label: format!("{name}:"),
                            detail: Some("provider".to_string()),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range,
                                new_text: format!("{name}:"),
                            })),
                            ..Default::default()
                        })
                        .collect();
                    (items, false)
                }
                IconifyContext::Name { provider, range, typed } => {
                    let range = index.range(&text, range);
                    let names = iconify_completion::cached_names(&provider);
                    // Some collections run into the thousands of names
                    // (e.g. `mdi` has ~7500) - sending every match on each
                    // keystroke is wasted bandwidth and a slow render on
                    // the client, so cap the response and mark it
                    // incomplete: the client is expected to re-request as
                    // the user narrows `typed` further, per the LSP spec's
                    // `CompletionList.isIncomplete`.
                    const LIMIT: usize = 200;
                    let mut matches = names.into_iter().flatten().filter(|name| name.starts_with(&typed));
                    let items: Vec<CompletionItem> = matches
                        .by_ref()
                        .take(LIMIT)
                        .map(|name| CompletionItem {
                            label: name.clone(),
                            detail: Some("icon".to_string()),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text: name })),
                            ..Default::default()
                        })
                        .collect();
                    let is_incomplete = matches.next().is_some();
                    (items, is_incomplete)
                }
            };
            return Ok(Some(CompletionResponse::List(CompletionList { is_incomplete, items })));
        }

        if line_prefix.starts_with("variants.") {
            let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);
            let mut variants = BTreeSet::new();
            for provider_name in guicons_core::builtin_provider_names() {
                if let Some(schema) = manifest.provider(provider_name) {
                    variants.extend(schema.variants.iter().cloned());
                }
            }
            let items = variants.into_iter().map(|variant| make_item(variant, "variant")).collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // A bare key being typed (possibly partially) at the start of a
        // line - not yet past `=`, `.`, or `[`.
        let is_bare_key_prefix = line_prefix.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_');
        if is_bare_key_prefix {
            let fields: &[&str] = match section_kind_at(&text, offset) {
                SectionKind::TopLevel => &["defaults", "link", "providers"],
                SectionKind::Defaults => &["root", "provider", "size"],
                SectionKind::Link => &["includes"],
                SectionKind::Provider => &["variants", "sizes"],
                SectionKind::Entry => &["file", "iconify", "url", "glyph", "windows-ico", "dynamic", "root", "variants"],
            };
            let items = fields.iter().map(|name| make_item(name.to_string(), "field")).collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }

        Ok(None)
    }
}
