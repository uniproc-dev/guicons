mod completion;
mod diagnostics;
mod goto_definition;
mod hover;
mod iconify_completion;
mod manifest_text;
mod position;
mod references;
mod rename;

use guicons_core::{IconEntrySource, IconManifest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, ClientSocket, LanguageServer, LspService};

fn describe_source(source: &IconEntrySource, manifest: &IconManifest) -> String {
    match source {
        IconEntrySource::File(path) => format!("file `{}`", manifest.display_path(path)),
        IconEntrySource::Iconify(id) => format!("iconify `{id}`"),
        IconEntrySource::Url(url) => format!("url `{url}`"),
        IconEntrySource::Glyph(spec) => format!("glyph `{spec}`"),
    }
}

pub struct Backend {
    pub(crate) client: Client,
    documents: RwLock<HashMap<Url, String>>,
    /// Set from `initialize`'s `root_uri`/`workspace_folders` - needed at
    /// `initialized` time (builtin-provider cache warmup) when no document
    /// has been opened yet to derive a workspace root from.
    workspace_root: RwLock<Option<PathBuf>>,
    /// Whether to publish raw `TOML syntax error: ...` diagnostics -
    /// configurable (`initializationOptions: { "reportTomlSyntaxErrors":
    /// true }`), **off by default**: an editor with its own TOML language
    /// support (e.g. JetBrains IDEs, almost always installed alongside
    /// this LSP) already reports these, and having both report the same
    /// syntax error twice is just noise. Semantic diagnostics (unknown
    /// field, missing file, ...) aren't affected by this flag.
    report_toml_syntax_errors: std::sync::atomic::AtomicBool,
    /// Every `icons.gui.toml` found under the workspace root, keyed by its
    /// canonicalized path - populated by a full scan at `initialized()`
    /// time and kept fresh via `did_change`/`did_change_watched_files`.
    /// Needed for `.rs`-side hover: unlike a `.gui.toml` document (which
    /// *is* the manifest it's diagnosed against), a `.rs` file has no
    /// manifest of its own - `hover_rust` looks up this file's own crate's
    /// entry by key (see there for why "own crate's", not "any manifest
    /// found").
    manifests: RwLock<HashMap<PathBuf, IconManifest>>,
    /// Extra directory names to skip during the manifest scan, on top of
    /// `DEFAULT_SKIP_DIRS` - configurable via `initializationOptions:
    /// { "manifestScanIgnoreDirs": [...] }` for whatever a given
    /// workspace's own build/tooling directories `DEFAULT_SKIP_DIRS`
    /// doesn't already cover.
    extra_skip_dirs: RwLock<Vec<String>>,
}

/// Constructs the `tower_lsp` service pair (server-side handle + client
/// socket) - the same shape [`tower_lsp::Server`] runs over stdio in
/// `main`, split out so tests can drive it over an in-memory duplex pipe.
pub fn service() -> (LspService<Backend>, ClientSocket) {
    LspService::new(|client| Backend {
        client,
        documents: RwLock::new(HashMap::new()),
        workspace_root: RwLock::new(None),
        report_toml_syntax_errors: std::sync::atomic::AtomicBool::new(false),
        manifests: RwLock::new(HashMap::new()),
        extra_skip_dirs: RwLock::new(Vec::new()),
    })
}

/// Directory names never worth descending into during a manifest scan -
/// build output, VCS internals, and dependency/package-manager caches
/// from the ecosystems a `guicons`-using project is realistically likely
/// to sit next to. Extendable per-workspace via `initializationOptions:
/// { "manifestScanIgnoreDirs": [...] }` (see `Backend::extra_skip_dirs`)
/// rather than edited here, for anything this list doesn't cover.
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

/// Recursively finds every `icons.gui.toml` under `root` - `[link]`d
/// files aren't collected separately, since `load_icon_manifest` already
/// pulls those in as part of loading the root manifest that references
/// them. Skips `DEFAULT_SKIP_DIRS` plus whatever `extra_skip_dirs` the
/// client configured.
fn find_manifest_files(root: &Path, extra_skip_dirs: &[String]) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_skipped_dir(entry, extra_skip_dirs))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == std::ffi::OsStr::new("icons.gui.toml"))
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

impl Backend {
    /// Exposed for tests driving the server through the real
    /// `initialize` request, to confirm `initializationOptions` was
    /// actually read - not part of the LSP surface itself.
    #[doc(hidden)]
    pub fn reports_toml_syntax_errors(&self) -> bool {
        self.report_toml_syntax_errors.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Canonicalized to match `IconEntry::file()`, which `guicons_core`
    /// also stamps with a canonicalized path.
    fn path_for_uri(uri: &Url) -> Option<PathBuf> {
        uri.to_file_path().ok().map(|path| guicons_core::canonicalize_or_self(&path))
    }

    async fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root.read().await.clone()
    }

    async fn document_text(&self, uri: &Url) -> Option<String> {
        self.documents.read().await.get(uri).cloned()
    }

    /// Falls back to reading `path` from disk when it isn't an open,
    /// tracked document - needed for jumping *into* a manifest file the
    /// user hasn't opened themselves (goto-definition from a `.rs`
    /// `icon!(...)` call), unlike every other place this server reads a
    /// document's text, which only ever needs the currently-open one.
    async fn document_text_or_disk(&self, uri: &Url, path: &Path) -> Option<String> {
        if let Some(text) = self.document_text(uri).await {
            return Some(text);
        }
        std::fs::read_to_string(path).ok()
    }

    /// Resolves the `icons.gui.toml` governing `.rs` file `path` (its own
    /// crate's manifest - see `hover_rust`'s doc comment for why not "any
    /// manifest lying around") and returns a clone of it, if known.
    /// Cloning (not holding the `RwLock` read guard across `.await`
    /// points in callers) is cheap enough here - manifests are typically
    /// small, and this only runs on interactive requests, not hot paths.
    async fn manifest_for_rust_file(&self, path: &Path) -> Option<IconManifest> {
        let manifest_path = guicons_core::canonicalize_or_self(&guicons_core::manifest_path_for_rust_file(path)?);
        self.manifests.read().await.get(&manifest_path).cloned()
    }

    async fn set_document_text(&self, uri: Url, text: String) {
        self.documents.write().await.insert(uri, text);
    }

    /// Rescans the workspace for `icons.gui.toml` files and replaces the
    /// whole `manifests` table - cheap enough (a local directory walk plus
    /// parsing) to just redo wholesale rather than track individual
    /// additions/removals.
    async fn scan_workspace_manifests(&self, workspace_root: &Path) {
        let extra_skip_dirs = self.extra_skip_dirs.read().await.clone();
        let mut manifests = HashMap::new();
        for manifest_path in find_manifest_files(workspace_root, &extra_skip_dirs) {
            let (manifest, _) = guicons_core::load_icon_manifest(&manifest_path);
            manifests.insert(guicons_core::canonicalize_or_self(&manifest_path), manifest);
        }
        *self.manifests.write().await = manifests;
    }

    /// Keeps `manifests` in sync with an open `.gui.toml` document's live
    /// buffer content (not just what's on disk) - mirrors how
    /// `publish_diagnostics_for` already treats the open document as the
    /// source of truth over the file on disk.
    async fn refresh_manifest_if_relevant(&self, path: &Path, text: &str) {
        if path.file_name() != Some(std::ffi::OsStr::new("icons.gui.toml")) {
            return;
        }
        let (manifest, _) = guicons_core::load_icon_manifest_from_str(path, text);
        self.manifests.write().await.insert(path.to_path_buf(), manifest);
    }

    /// Warms the cache for any provider this manifest declares that isn't
    /// already one of the builtins warmed at startup - builtins are known
    /// statically, but a manifest's own `[providers.<name>]` entries only
    /// become visible once a document with them is open.
    async fn warm_custom_providers(&self, uri: &Url) {
        let Some(path) = Self::path_for_uri(uri) else { return };
        let Some(text) = self.document_text(uri).await else { return };

        let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);
        let custom: Vec<String> = manifest
            .provider_names()
            .filter(|name| !guicons_core::builtin_provider_names().any(|builtin| builtin == *name))
            .map(str::to_string)
            .collect();
        iconify_completion::warm_provider_caches(self.client.clone(), custom);
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let root = params
            .root_uri
            .as_ref()
            .and_then(|uri| uri.to_file_path().ok())
            .or_else(|| {
                params
                    .workspace_folders
                    .as_ref()
                    .and_then(|folders| folders.first())
                    .and_then(|folder| folder.uri.to_file_path().ok())
            });
        *self.workspace_root.write().await = root;

        if let Some(report) = params
            .initialization_options
            .as_ref()
            .and_then(|options| options.get("reportTomlSyntaxErrors"))
            .and_then(serde_json::Value::as_bool)
        {
            self.report_toml_syntax_errors.store(report, std::sync::atomic::Ordering::Relaxed);
        }

        if let Some(extra_dirs) = params
            .initialization_options
            .as_ref()
            .and_then(|options| options.get("manifestScanIgnoreDirs"))
            .and_then(|value| serde_json::from_value::<Vec<String>>(value.clone()).ok())
        {
            *self.extra_skip_dirs.write().await = extra_dirs;
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
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

        // Warms the builtin providers' icon-name caches immediately, not
        // lazily on first `didOpen` - by the time a user actually types
        // into an `iconify = "..."` field the cache is very likely warm
        // already, so `completion()` never has to touch the network.
        let providers: Vec<String> = guicons_core::builtin_provider_names().map(str::to_string).collect();
        iconify_completion::warm_provider_caches(self.client.clone(), providers);
        if let Some(workspace_root) = self.workspace_root().await {
            self.scan_workspace_manifests(&workspace_root).await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.set_document_text(params.text_document.uri.clone(), params.text_document.text).await;
        self.warm_custom_providers(&params.text_document.uri).await;
        if let Some(path) = Self::path_for_uri(&params.text_document.uri) {
            if let Some(text) = self.document_text(&params.text_document.uri).await {
                self.refresh_manifest_if_relevant(&path, &text).await;
            }
        }
        self.publish_diagnostics_for(params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.set_document_text(params.text_document.uri.clone(), change.text).await;
        }
        self.warm_custom_providers(&params.text_document.uri).await;
        if let Some(path) = Self::path_for_uri(&params.text_document.uri) {
            if let Some(text) = self.document_text(&params.text_document.uri).await {
                self.refresh_manifest_if_relevant(&path, &text).await;
            }
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
        if let Some(workspace_root) = self.workspace_root().await {
            self.scan_workspace_manifests(&workspace_root).await;
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.hover_impl(params).await
    }

    async fn goto_definition(&self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>> {
        self.goto_definition_impl(params).await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.completion_impl(params).await
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.references_impl(params).await
    }

    async fn prepare_rename(&self, params: TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>> {
        self.prepare_rename_impl(params).await
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        self.rename_impl(params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diagnostics::{closest_file_name, levenshtein, missing_file_diagnostics, should_report_error, unresolved_iconify_diagnostics};
    use hover::iconify_literal_hover;
    use position::LineIndex;
    use tempfile::tempdir;

    #[test]
    fn missing_file_gets_a_did_you_mean_suggestion_for_a_close_typo() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("docker.svg"), "<svg/>").unwrap();

        let target = dir.path().join("dokcer.svg");
        let suggestion = closest_file_name(&target).expect("a suggestion");
        assert_eq!(suggestion, "docker.svg");
    }

    #[test]
    fn missing_file_gets_no_suggestion_when_nothing_is_close() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("completely-unrelated-name.png"), "").unwrap();

        let target = dir.path().join("x.svg");
        assert_eq!(closest_file_name(&target), None);
    }

    #[test]
    fn missing_file_diagnostics_reports_entries_whose_file_does_not_exist() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("docker.svg"), "<svg/>").unwrap();
        let content = "[docker]\nfile = \"docker.svg\"\n\n[missing]\nfile = \"dokcer.svg\"\n";
        let path = dir.path().join("icons.gui.toml");
        std::fs::write(&path, content).unwrap();

        let (manifest, errors) = guicons_core::load_icon_manifest_from_str(&path, content);
        assert!(errors.is_empty(), "{errors:?}");

        let index = LineIndex::new(content);
        let diagnostics = missing_file_diagnostics(content, &guicons_core::canonicalize_or_self(&path), &manifest, &index);

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("dokcer.svg"), "{}", diagnostics[0].message);
        assert!(diagnostics[0].message.contains("did you mean `docker.svg`"), "{}", diagnostics[0].message);
    }

    #[test]
    fn unresolved_iconify_diagnostics_notes_an_uncached_icon() {
        let dir = tempdir().unwrap();
        let content = "[docker]\niconify = \"mdi:home\"\n";
        let path = dir.path().join("icons.gui.toml");
        std::fs::write(&path, content).unwrap();

        let (manifest, errors) = guicons_core::load_icon_manifest_from_str(&path, content);
        assert!(errors.is_empty(), "{errors:?}");

        let index = LineIndex::new(content);
        let diagnostics =
            unresolved_iconify_diagnostics(content, &guicons_core::canonicalize_or_self(&path), &manifest, &index);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::INFORMATION));
        assert!(diagnostics[0].message.contains("mdi:home"), "{}", diagnostics[0].message);
        assert!(diagnostics[0].message.contains("isn't cached locally"), "{}", diagnostics[0].message);
    }

    #[test]
    fn unresolved_iconify_diagnostics_is_silent_once_cached() {
        let dir = tempdir().unwrap();
        let content = "[docker]\niconify = \"mdi:home\"\n";
        let path = dir.path().join("icons.gui.toml");
        std::fs::write(&path, content).unwrap();
        let cache_path = dir.path().join(".cache/guicons/mdi/home.svg");
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, "<svg/>").unwrap();

        let (manifest, errors) = guicons_core::load_icon_manifest_from_str(&path, content);
        assert!(errors.is_empty(), "{errors:?}");

        let index = LineIndex::new(content);
        let diagnostics =
            unresolved_iconify_diagnostics(content, &guicons_core::canonicalize_or_self(&path), &manifest, &index);

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn find_manifest_files_skips_default_noise_directories() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("icons.gui.toml"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/some-package")).unwrap();
        std::fs::write(dir.path().join("node_modules/some-package/icons.gui.toml"), "").unwrap();

        let found = find_manifest_files(dir.path(), &[]);

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0], dir.path().join("icons.gui.toml"));
    }

    #[test]
    fn find_manifest_files_also_skips_configured_extra_directories() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("icons.gui.toml"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("vendored")).unwrap();
        std::fs::write(dir.path().join("vendored/icons.gui.toml"), "").unwrap();

        let found_without_config = find_manifest_files(dir.path(), &[]);
        assert_eq!(found_without_config.len(), 2, "{found_without_config:?}");

        let found_with_config = find_manifest_files(dir.path(), &["vendored".to_string()]);
        assert_eq!(found_with_config, vec![dir.path().join("icons.gui.toml")]);
    }

    #[test]
    fn levenshtein_matches_known_distances() {
        assert_eq!(levenshtein("docker", "docker"), 0);
        assert_eq!(levenshtein("docker", "dokcer"), 2);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn iconify_literal_hover_links_to_the_icon_on_iconifys_own_site() {
        let value = iconify_literal_hover("mdi:home");
        assert!(value.contains("mdi:home"), "{value}");
        assert!(value.contains("https://icon-sets.iconify.design/mdi/home/"), "{value}");
    }

    #[test]
    fn iconify_literal_hover_flags_a_malformed_id_without_a_broken_link() {
        let value = iconify_literal_hover("no-colon-here");
        assert!(!value.contains("icon-sets.iconify.design"), "{value}");
        assert!(value.contains("will never resolve"), "{value}");
    }

    #[test]
    fn toml_syntax_errors_are_suppressed_only_when_configured_off() {
        let message = "TOML syntax error: expected a right bracket, found a newline";
        assert!(should_report_error(message, true));
        assert!(!should_report_error(message, false));
    }

    #[test]
    fn semantic_errors_are_never_suppressed_by_the_toml_syntax_error_setting() {
        let message = "unexpected field(s): `bogus` (expected one of: `file`)";
        assert!(should_report_error(message, true));
        assert!(should_report_error(message, false));
    }
}
