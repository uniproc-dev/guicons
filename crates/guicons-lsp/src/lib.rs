mod manifest_text;
mod position;

use guicons_core::{IconEntrySource, IconManifest};
use manifest_text::{family_header_at, include_target_at, keyword_at, provider_name_at};
use position::LineIndex;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, ClientSocket, LanguageServer, LspService};

/// Renders `path` for display: relative to the workspace root or the
/// manifest's own directory when possible (both are "not noise"), falling
/// back to the absolute path only if neither contains it. Always
/// forward-slashed - `Path::display()` on Windows keeps `\`, and
/// `{:?}`/Debug escapes it as `\\`, both of which are just noise here.
fn display_path(path: &Path, manifest: &IconManifest) -> String {
    if let Ok(rel) = path.strip_prefix(manifest.workspace_root()) {
        return normalize_slashes(rel);
    }
    if let Some(manifest_dir) = manifest.manifest_path().parent() {
        if let Ok(rel) = path.strip_prefix(manifest_dir) {
            return normalize_slashes(rel);
        }
    }
    normalize_slashes(path)
}

fn normalize_slashes(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn describe_source(source: &IconEntrySource, manifest: &IconManifest) -> String {
    match source {
        IconEntrySource::File(path) => format!("file `{}`", display_path(path, manifest)),
        IconEntrySource::Iconify(id) => format!("iconify `{id}`"),
        IconEntrySource::Url(url) => format!("url `{url}`"),
        IconEntrySource::Glyph(spec) => format!("glyph `{spec}`"),
    }
}

pub struct Backend {
    client: Client,
    documents: RwLock<HashMap<Url, String>>,
}

/// Constructs the `tower_lsp` service pair (server-side handle + client
/// socket) - the same shape [`tower_lsp::Server`] runs over stdio in
/// `main`, split out so tests can drive it over an in-memory duplex pipe.
pub fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::new(|client| Backend { client, documents: RwLock::new(HashMap::new()) })
}

impl Backend {
    /// Canonicalized to match `IconEntry::file()`, which `guicons_core`
    /// also stamps with a canonicalized path.
    fn path_for_uri(uri: &Url) -> Option<PathBuf> {
        uri.to_file_path().ok().map(|path| guicons_core::canonicalize_or_self(&path))
    }

    async fn document_text(&self, uri: &Url) -> Option<String> {
        self.documents.read().await.get(uri).cloned()
    }

    async fn set_document_text(&self, uri: Url, text: String) {
        self.documents.write().await.insert(uri, text);
    }

    async fn publish_diagnostics_for(&self, uri: Url) {
        let Some(path) = Self::path_for_uri(&uri) else { return };
        let Some(text) = self.document_text(&uri).await else { return };

        let (_, errors) = guicons_core::load_icon_manifest_from_str(&path, &text);
        let index = LineIndex::new(&text);
        let diagnostics = errors
            .iter()
            .filter(|error| error.file == path)
            .map(|error| Diagnostic {
                range: match &error.span {
                    Some(span) => index.range(&text, span.clone()),
                    None => Range::new(Position::new(0, 0), Position::new(0, 0)),
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: error.message.clone(),
                source: Some("guicons".to_string()),
                ..Default::default()
            })
            .collect();

        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // `register_capability` is a server-to-client request awaiting the
        // client's response - spawned so a client that never answers it
        // (or a test harness with nothing driving the other end) can't
        // block `initialized` itself, and by extension every request
        // processed after it.
        let client = self.client.clone();
        tokio::spawn(async move {
            let registration = Registration {
                id: "guicons-watch-manifests".to_string(),
                method: "workspace/didChangeWatchedFiles".to_string(),
                register_options: serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                    watchers: vec![FileSystemWatcher {
                        glob_pattern: GlobPattern::String("**/*.gui.toml".to_string()),
                        kind: None,
                    }],
                })
                .ok(),
            };
            let _ = client.register_capability(vec![registration]).await;
        });
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.set_document_text(params.text_document.uri.clone(), params.text_document.text).await;
        self.publish_diagnostics_for(params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.set_document_text(params.text_document.uri.clone(), change.text).await;
        }
        self.publish_diagnostics_for(params.text_document.uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.publish_diagnostics_for(params.text_document.uri).await;
    }

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
        let uris: Vec<Url> = self.documents.read().await.keys().cloned().collect();
        for uri in uris {
            self.publish_diagnostics_for(uri).await;
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
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

        let mut lines = vec![format!("**{}**", entry.key())];
        lines.push(format!("- family: `{}`", entry.family()));
        if let Some(variant) = entry.variant() {
            lines.push(format!("- variant: `{variant}`"));
        }
        if let Some(size) = entry.size() {
            lines.push(format!("- size: `{size}`"));
        }
        lines.push(format!("- source: {}", describe_source(entry.source(), &manifest)));

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: lines.join("\n"),
            }),
            range: Some(index.range(&text, entry.span())),
        }))
    }

    async fn goto_definition(&self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
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
            .find(|entry| entry.file() == path && entry.span().contains(&offset))
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

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let Some(path) = Self::path_for_uri(&uri) else { return Ok(None) };
        let Some(text) = self.document_text(&uri).await else { return Ok(None) };
        let index = LineIndex::new(&text);
        let Some(offset) = index.offset(&text, position) else { return Ok(None) };

        let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_prefix = text[line_start..offset].trim_start();

        if line_prefix.starts_with('[') && line_prefix.contains("providers.") {
            let items = guicons_core::builtin_provider_names()
                .map(|name| CompletionItem::new_simple(name.to_string(), "built-in provider".to_string()))
                .collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }

        if line_prefix.starts_with("variants.") {
            let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);
            let mut variants = BTreeSet::new();
            for provider_name in guicons_core::builtin_provider_names() {
                if let Some(schema) = manifest.provider(provider_name) {
                    variants.extend(schema.variants.iter().cloned());
                }
            }
            let items = variants
                .into_iter()
                .map(|variant| CompletionItem::new_simple(variant, "variant".to_string()))
                .collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }

        Ok(None)
    }
}
