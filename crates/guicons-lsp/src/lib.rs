mod iconify_completion;
mod manifest_text;
mod position;

use guicons_core::{IconEntrySource, IconManifest};
use iconify_completion::IconifyContext;
use manifest_text::{
    defaults_root, family_header_at, iconify_field_at, include_target_at, keyword_at, offset_line_overlaps,
    path_field_at, provider_name_at, section_kind_at, word_prefix_span, PathFieldKind, SectionKind,
};
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

/// Whether a manifest error should be published, given the
/// `reportTomlSyntaxErrors` setting - only raw TOML syntax errors are
/// ever suppressed, never semantic ones (unknown field, missing source, ...).
fn should_report_error(message: &str, report_toml_syntax_errors: bool) -> bool {
    report_toml_syntax_errors || !message.starts_with("TOML syntax error:")
}

/// `file`/`windows-ico` targets that don't exist on disk - not caught by
/// `guicons_core` at all (it only validates manifest *shape*), so this is
/// entirely an editor-tooling-side check.
fn missing_file_diagnostics(text: &str, path: &Path, manifest: &IconManifest, index: &LineIndex) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for entry in manifest.entries() {
        if entry.file() != path {
            continue;
        }
        if let IconEntrySource::File(target) = entry.source() {
            diagnostics.extend(missing_file_diagnostic(target, "file", entry.span(), text, index, manifest));
        }
        if let Some(ico) = entry.windows_ico() {
            diagnostics.extend(missing_file_diagnostic(ico, "windows-ico", entry.span(), text, index, manifest));
        }
    }
    diagnostics
}

fn missing_file_diagnostic(
    target: &Path,
    field: &str,
    span: std::ops::Range<usize>,
    text: &str,
    index: &LineIndex,
    manifest: &IconManifest,
) -> Option<Diagnostic> {
    if target.exists() {
        return None;
    }
    let mut message = format!("`{field}` not found: `{}`", display_path(target, manifest));
    if let Some(suggestion) = closest_file_name(target) {
        message.push_str(&format!(" - did you mean `{suggestion}`?"));
    }
    Some(Diagnostic {
        range: index.range(text, span),
        severity: Some(DiagnosticSeverity::ERROR),
        message,
        source: Some("guicons".to_string()),
        ..Default::default()
    })
}

/// `iconify = "provider:name"` entries whose SVG isn't cached locally yet -
/// same check `guicons-cli::check` does, mirrored here so the editor
/// doesn't fall behind the CLI. Informational, not a warning/error: not
/// being cached yet doesn't mean the id is wrong, just unconfirmed
/// without a network fetch (`icons fetch`), which neither `check` nor
/// this diagnostic ever does itself.
fn unresolved_iconify_diagnostics(text: &str, path: &Path, manifest: &IconManifest, index: &LineIndex) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for entry in manifest.entries() {
        if entry.file() != path {
            continue;
        }
        let IconEntrySource::Iconify(id) = entry.source() else { continue };

        let message = if id.split_once(':').is_none() {
            format!("iconify id `{id}` isn't in `provider:name` form - it will never resolve")
        } else {
            let cache_path = guicons_net::iconify_cache_path(manifest.workspace_root(), id);
            if cache_path.exists() {
                continue;
            }
            format!(
                "iconify icon `{id}` isn't cached locally yet - can't confirm it resolves without a network fetch (`icons fetch` or `GUICONS_ALLOW_NETWORK=1`)"
            )
        };

        diagnostics.push(Diagnostic {
            range: index.range(text, entry.span()),
            severity: Some(DiagnosticSeverity::INFORMATION),
            message,
            source: Some("guicons".to_string()),
            ..Default::default()
        });
    }
    diagnostics
}

/// Closest sibling file by name (Levenshtein distance), for a "did you
/// mean" suggestion - `None` if nothing in the directory is plausibly a
/// typo of `target`'s name (rather than just unrelated).
fn closest_file_name(target: &Path) -> Option<String> {
    let dir = target.parent()?;
    let target_name = target.file_name()?.to_string_lossy().to_string();
    let mut best: Option<(usize, String)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let distance = levenshtein(&target_name, &name);
        if best.as_ref().is_none_or(|(best_distance, _)| distance < *best_distance) {
            best = Some((distance, name));
        }
    }
    let (distance, name) = best?;
    (distance <= target_name.len().div_ceil(2).max(2)).then_some(name)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

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

pub struct Backend {
    client: Client,
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
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else { continue };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            if is_dir {
                let name = entry.file_name();
                let skipped = DEFAULT_SKIP_DIRS.iter().any(|skip| name == std::ffi::OsStr::new(skip))
                    || extra_skip_dirs.iter().any(|skip| name == std::ffi::OsStr::new(skip.as_str()));
                if !skipped {
                    stack.push(path);
                }
            } else if entry.file_name() == std::ffi::OsStr::new("icons.gui.toml") {
                found.push(path);
            }
        }
    }
    found
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

    async fn set_document_text(&self, uri: Url, text: String) {
        self.documents.write().await.insert(uri, text);
    }

    async fn publish_diagnostics_for(&self, uri: Url) {
        let Some(path) = Self::path_for_uri(&uri) else { return };
        let Some(text) = self.document_text(&uri).await else { return };

        let (manifest, errors) = guicons_core::load_icon_manifest_from_str(&path, &text);
        let index = LineIndex::new(&text);
        let report_syntax_errors = self.report_toml_syntax_errors.load(std::sync::atomic::Ordering::Relaxed);
        let mut diagnostics: Vec<Diagnostic> = errors
            .iter()
            .filter(|error| error.file == path)
            .filter(|error| should_report_error(&error.message, report_syntax_errors))
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
        diagnostics.extend(missing_file_diagnostics(&text, &path, &manifest, &index));
        diagnostics.extend(unresolved_iconify_diagnostics(&text, &path, &manifest, &index));

        self.client.publish_diagnostics(uri, diagnostics, None).await;
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
    async fn hover_rust(&self, path: &Path, uri: &Url, position: Position) -> Result<Option<Hover>> {
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
                    value: format!(
                        "**{id}** - raw iconify id, resolved directly through `guicons-net`'s cache - no manifest entry for this one"
                    ),
                }),
                range,
            })),
            guicons_core::selector::IconSelector::FamilyVariant { family, size, variant } => {
                let Some(crate_root) = path.parent().and_then(guicons_core::find_workspace_root_from) else {
                    return Ok(None);
                };
                let manifest_path = guicons_core::canonicalize_or_self(&crate_root.join("icons.gui.toml"));

                let manifests = self.manifests.read().await;
                let Some(manifest) = manifests.get(&manifest_path) else { return Ok(None) };
                let Some(entry) = manifest.entry_for_family_variant(&family, size, variant.as_deref()) else {
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
                lines.push(format!("- source: {}", describe_source(entry.source(), manifest)));

                Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: lines.join("\n") }),
                    range,
                }))
            }
        }
    }

    /// Warms the cache for any provider this manifest declares that isn't
    /// already one of the builtins warmed at startup - builtins are known
    /// statically, but a manifest's own `[providers.<name>]` entries only
    /// become visible once a document with them is open.
    async fn warm_custom_providers(&self, uri: &Url) {
        let Some(path) = Self::path_for_uri(uri) else { return };
        let Some(text) = self.document_text(uri).await else { return };
        let Some(workspace_root) = self.workspace_root().await else { return };

        let (manifest, _) = guicons_core::load_icon_manifest_from_str(&path, &text);
        let custom: Vec<String> = manifest
            .provider_names()
            .filter(|name| !guicons_core::builtin_provider_names().any(|builtin| builtin == *name))
            .map(str::to_string)
            .collect();
        iconify_completion::warm_provider_caches(self.client.clone(), workspace_root, custom);
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
        if let Some(workspace_root) = self.workspace_root().await {
            let providers: Vec<String> = guicons_core::builtin_provider_names().map(str::to_string).collect();
            iconify_completion::warm_provider_caches(self.client.clone(), workspace_root.clone(), providers);
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

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
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
                    let names = match self.workspace_root().await {
                        Some(workspace_root) => iconify_completion::cached_names(&workspace_root, &provider),
                        None => None,
                    };
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

#[cfg(test)]
mod tests {
    use super::*;
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
