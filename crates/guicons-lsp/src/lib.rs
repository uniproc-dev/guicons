mod position;

use position::LineIndex;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, ClientSocket, LanguageServer, LspService};

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
    fn path_for_uri(uri: &Url) -> Option<PathBuf> {
        uri.to_file_path().ok()
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

        let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);
        let Some(entry) = manifest.entries().iter().find(|entry| entry.span().contains(&offset)) else {
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
        lines.push(format!("- source: `{:?}`", entry.source()));

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: lines.join("\n"),
            }),
            range: Some(index.range(&text, entry.span())),
        }))
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
