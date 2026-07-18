//! UniFFI bindings for `../ide-plugin`'s Kotlin/JVM side.
//!
//! Every function here is a thin pass-through to `guicons-core`,
//! converting only what UniFFI's type set requires (its own mirror
//! `enum`s/`struct`s, `usize`/`u16` <-> `u32`, `Path`/`PathBuf` <->
//! `String`) - no logic of its own worth testing independently.

uniffi::setup_scaffolding!();

use guicons_core::rust_macro::MacroKind as CoreMacroKind;
use guicons_core::selector::IconSelector as CoreIconSelector;
use guicons_core::{IconEntry, IconEntrySource, IconManifest};
use std::path::Path;

#[derive(uniffi::Enum)]
pub enum MacroKind {
    Icon,
    IconKey,
    IconData,
}

impl From<CoreMacroKind> for MacroKind {
    fn from(kind: CoreMacroKind) -> Self {
        match kind {
            CoreMacroKind::Icon => MacroKind::Icon,
            CoreMacroKind::IconKey => MacroKind::IconKey,
            CoreMacroKind::IconData => MacroKind::IconData,
        }
    }
}

#[derive(uniffi::Record)]
pub struct MacroCallSite {
    pub kind: MacroKind,
    pub arg_text: String,
    /// UTF-8 byte offsets into the same `text` passed to
    /// [`macro_call_at`]/whatever produced this - not a UTF-16 code-unit
    /// offset some JVM editor APIs might use natively; converting is the
    /// caller's job, same as `guicons-lsp` already has to do for its own
    /// LSP-protocol UTF-16 positions.
    pub arg_start: u32,
    pub arg_end: u32,
}

/// Finds the guicons macro call (if any) whose argument range contains
/// `offset` (a UTF-8 byte offset into `text`).
#[uniffi::export]
pub fn macro_call_at(text: String, offset: u32) -> Option<MacroCallSite> {
    guicons_core::rust_macro::macro_call_at(&text, offset as usize).map(|site| MacroCallSite {
        kind: site.kind.into(),
        arg_text: site.arg_text,
        arg_start: site.arg_range.start as u32,
        arg_end: site.arg_range.end as u32,
    })
}

#[derive(uniffi::Enum)]
pub enum IconSelector {
    FamilyVariant { family: String, size: Option<u16>, variant: Option<String> },
    Iconify { id: String },
}

impl From<CoreIconSelector> for IconSelector {
    fn from(selector: CoreIconSelector) -> Self {
        match selector {
            CoreIconSelector::FamilyVariant { family, size, variant } => {
                IconSelector::FamilyVariant { family, size, variant }
            }
            CoreIconSelector::Iconify(id) => IconSelector::Iconify { id },
        }
    }
}

/// Interprets an `icon!`/`icon_key!`/`icon_data!` call's raw argument text
/// (as returned in [`MacroCallSite::arg_text`]) - `None` if it isn't
/// valid selector syntax at all.
#[uniffi::export]
pub fn parse_selector(raw: String) -> Option<IconSelector> {
    guicons_core::selector::parse_selector(&raw).ok().map(Into::into)
}

/// See `guicons_core::manifest_path_for_rust_file` - this is a thin
/// UniFFI-string wrapper over it, shared with `guicons-lsp`'s own
/// crate-scoped manifest lookup so the two can't drift apart.
#[uniffi::export]
pub fn find_manifest_for_rust_file(rust_file_path: String) -> Option<String> {
    guicons_core::manifest_path_for_rust_file(Path::new(&rust_file_path)).map(|path| path.to_string_lossy().into_owned())
}

#[derive(uniffi::Record)]
pub struct ResolvedEntry {
    pub key: String,
    pub family: String,
    pub size: Option<u16>,
    pub variant: Option<String>,
    /// Human-readable description of the source (`` file `x` ``/
    /// `` iconify `x` ``/...), matching the wording `guicons-lsp`'s own
    /// hover already uses.
    pub source_description: String,
    /// Absolute path to the backing asset file - only present for a
    /// `file` source.
    pub source_file: Option<String>,
    /// `"prefix:name"` - only present for an `iconify` source. A caller
    /// that wants to preview one of these has to actually fetch/cache it
    /// first (`guicons-net`'s job, not this crate's - see
    /// `ensure_iconify_icon_cached`), same as the icon browser's own
    /// Iconify tab already does for entries it finds by browsing/
    /// searching rather than reading out of a manifest.
    pub iconify_id: Option<String>,
    /// The manifest file this entry was actually declared in - the root
    /// `icons.gui.toml`, or one of its `[link]`d files, rendered for
    /// display (relative when possible). Distinct from `source_file` (the
    /// icon *asset*, e.g. an `.svg`) - this is where the
    /// `[family]`/`[family.variant]` table itself lives.
    pub declared_in_file: String,
    /// Same file as `declared_in_file`, but the raw absolute path - for a
    /// caller that wants to actually open it (e.g. a "declared in" link),
    /// not just show it.
    pub declared_in_file_path: String,
    /// UTF-8 byte offsets of this entry's own table within
    /// `declared_in_file_path` (`IconEntry::span()`) - not just "which
    /// file", but exactly where in it, for a caller that wants to
    /// navigate straight to/highlight this entry in an already-open
    /// editor on that file, the same offset convention
    /// [`macro_call_at`]/[`entry_at_offset`] already use.
    pub declared_in_span_start: u32,
    pub declared_in_span_end: u32,
}

#[derive(uniffi::Enum)]
pub enum ResolveOutcome {
    Found(ResolvedEntry),
    NotFound,
    /// The manifest itself failed to load (syntax/schema errors) - distinct
    /// from `NotFound` so a caller can tell "your `icon!` call has a typo"
    /// apart from "your `icons.gui.toml` is broken", which would otherwise
    /// both look like a missing entry.
    ManifestInvalid { errors: Vec<String> },
}

fn describe_entry(entry: &IconEntry, manifest: &IconManifest) -> ResolvedEntry {
    let (source_description, source_file, iconify_id) = match entry.source() {
        IconEntrySource::File(path) => {
            (format!("file `{}`", manifest.display_path(path)), Some(path.to_string_lossy().into_owned()), None)
        }
        IconEntrySource::Iconify(id) => (format!("iconify `{id}`"), None, Some(id.clone())),
        IconEntrySource::Url(url) => (format!("url `{url}`"), None, None),
        IconEntrySource::Glyph(spec) => (format!("glyph `{spec}`"), None, None),
    };

    let span = entry.span();
    ResolvedEntry {
        key: entry.key().to_string(),
        family: entry.family().to_string(),
        size: entry.size(),
        variant: entry.variant().map(str::to_string),
        source_description,
        source_file,
        iconify_id,
        declared_in_file: manifest.display_path(entry.file()),
        declared_in_file_path: entry.file().to_string_lossy().into_owned(),
        declared_in_span_start: span.start as u32,
        declared_in_span_end: span.end as u32,
    }
}

/// Loads `manifest_path` and looks up the entry matching
/// `family`/`size`/`variant`. A manifest that fails to load at all
/// surfaces as `ManifestInvalid`, not silently as `NotFound` - this is a
/// best-effort preview lookup, not a validator; use `icons check` for the
/// full diagnostic list.
#[uniffi::export]
pub fn resolve_family_variant(
    manifest_path: String,
    family: String,
    size: Option<u16>,
    variant: Option<String>,
) -> ResolveOutcome {
    let (manifest, errors) = guicons_core::load_icon_manifest(Path::new(&manifest_path));
    if !errors.is_empty() {
        return ResolveOutcome::ManifestInvalid { errors: errors.iter().map(ToString::to_string).collect() };
    }

    let Some(entry) = manifest.entry_for_family_variant(&family, size, variant.as_deref()) else {
        return ResolveOutcome::NotFound;
    };

    ResolveOutcome::Found(describe_entry(entry, &manifest))
}

/// The entry (if any) whose table `offset` - a UTF-8 byte offset into
/// `toml_file_path`'s own text, same convention as [`macro_call_at`]'s -
/// falls inside, for syncing an IDE's manifest browser to wherever the
/// caret is sitting inside an `icons.gui.toml` (or one of its `[link]`d
/// files) itself - the reverse of syncing it from an `icon!(...)` call in
/// a `.rs` file. `manifest_path` is always the *root* manifest (loading
/// pulls in every `[link]`d file's entries too); `toml_file_path` is
/// whichever specific file the caret is actually in right now - almost
/// always the same file, but not when the caret's in a `[link]`d file
/// included by a different root.
#[uniffi::export]
pub fn entry_at_offset(manifest_path: String, toml_file_path: String, offset: u32) -> Option<ResolvedEntry> {
    let (manifest, errors) = guicons_core::load_icon_manifest(Path::new(&manifest_path));
    if !errors.is_empty() {
        return None;
    }
    let file = guicons_core::canonicalize_or_self(Path::new(&toml_file_path));
    let offset = offset as usize;
    let entry = manifest.entries().iter().find(|entry| entry.file() == file && entry.span().contains(&offset))?;
    Some(describe_entry(entry, &manifest))
}

#[derive(uniffi::Enum)]
pub enum ListManifestEntriesOutcome {
    Found { entries: Vec<ResolvedEntry> },
    ManifestInvalid { errors: Vec<String> },
}

/// Every entry in `manifest_path`, for icon-browser UIs that let a user
/// pick from what's already declared rather than typing a selector from
/// memory - the sibling read to [`resolve_family_variant`]'s single-entry
/// lookup.
#[uniffi::export]
pub fn list_manifest_entries(manifest_path: String) -> ListManifestEntriesOutcome {
    let (manifest, errors) = guicons_core::load_icon_manifest(Path::new(&manifest_path));
    if !errors.is_empty() {
        return ListManifestEntriesOutcome::ManifestInvalid { errors: errors.iter().map(ToString::to_string).collect() };
    }

    let entries = manifest.entries().iter().map(|entry| describe_entry(entry, &manifest)).collect();
    ListManifestEntriesOutcome::Found { entries }
}

/// guicons' own built-in provider schemas (Fluent, Phosphor, Material
/// Symbols, Heroicons, Bootstrap Icons, Tabler, ...) - each one *is* an
/// iconify.design prefix, so this doubles as a sensible, curated default
/// list for an Iconify-browsing UI's provider picker, without needing a
/// network round-trip to iconify.design's own (much larger, unfiltered)
/// collection list.
#[uniffi::export]
pub fn builtin_provider_names() -> Vec<String> {
    guicons_core::builtin_provider_names().map(str::to_string).collect()
}

/// The exact Slint component name `guicons-build` generates for a
/// manifest entry's `key` (`docker-filled` -> `DockerFilledIcon`) - same
/// `rust_variant_name` + `"Icon"` suffix `guicons-build/src/generate/
/// slint.rs::slint_component_name` builds, called through rather than
/// duplicated so a caller predicting this name (an icon browser UI
/// offering an `IconName {}` Slint snippet to copy, say) can't drift from
/// what codegen actually emits. Only meaningful for an entry that's
/// actually in the manifest and will get materialized/generated - not
/// for an arbitrary iconify.design id nothing's added yet, which has no
/// corresponding Slint component on disk at all.
#[uniffi::export]
pub fn slint_component_name(key: String) -> String {
    format!("{}Icon", guicons_core::rust_variant_name(&key))
}

/// Every `icons.gui.toml` anywhere under `workspace_root` - a directory
/// walk, not free, so `async` like the network-backed functions below (a
/// caller building a reverse "which manifest owns this file" index over a
/// whole workspace shouldn't have to do it on its own UI thread).
#[uniffi::export(async_runtime = "tokio")]
pub async fn list_workspace_manifests(workspace_root: String) -> Vec<String> {
    tokio::task::spawn_blocking(move || {
        guicons_core::find_manifest_files(Path::new(&workspace_root), &[])
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    })
    .await
    .unwrap_or_default()
}

// Everything below talks to api.iconify.design - `async`, not the plain
// synchronous style the rest of this crate uses, so a blocking network
// call on the Kotlin side can't ever freeze the caller's coroutine
// dispatcher. UniFFI turns an `async fn` into a Kotlin `suspend fun`
// itself (the foreign side supplies the executor/polls the future) -
// `async_runtime = "tokio"` just gives this crate's side a runtime to
// hand the actual blocking `ureq` call off to via `spawn_blocking`,
// since `guicons_net`'s HTTP client is synchronous.

/// The OS-wide cache dir every iconify-related FFI function here reads
/// from/writes to - exposed so the icon browser can list what's already
/// cached (e.g. the `.json` files under its `_collections` subdir)
/// without duplicating this OS-specific lookup on the Kotlin side.
#[uniffi::export]
pub fn global_iconify_cache_dir() -> String {
    guicons_net::global_cache_dir().to_string_lossy().into_owned()
}

/// Icon names already cached on disk for `provider` - empty if its
/// collection hasn't been fetched yet (see [`download_iconify_collection`]).
/// Backed by the OS-wide cache dir, not the calling project's workspace -
/// the icon browser fetches the same provider listing regardless of which
/// repo happens to be open, so there's no reason to redownload it per repo.
#[uniffi::export(async_runtime = "tokio")]
pub async fn cached_iconify_collection_names(provider: String) -> Vec<String> {
    tokio::task::spawn_blocking(move || guicons_net::global_cached_collection_names(&provider))
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Downloads `provider`'s full collection listing into the cache if it
/// isn't there already - `true` once it's cached (whether it already was,
/// or this fetch succeeded), `false` on a network failure.
#[uniffi::export(async_runtime = "tokio")]
pub async fn download_iconify_collection(provider: String) -> bool {
    tokio::task::spawn_blocking(move || guicons_net::global_download_collection(&provider)).await.unwrap_or(false)
}

/// Same `/search` endpoint iconify.design's own site search uses - empty
/// `Vec` on a network failure, not an error, since this only ever backs a
/// best-effort browse/search UI.
#[uniffi::export(async_runtime = "tokio")]
pub async fn search_iconify_icons(query: String, limit: u32) -> Vec<String> {
    tokio::task::spawn_blocking(move || guicons_net::search_icons(&query, limit as usize))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
}

/// Fetches `id`'s (`"prefix:name"`) SVG bytes for the browser's preview
/// pane/grid thumbnails - `None` on a network failure. Stateless on this
/// side of the FFI boundary: never written to a repo's `.cache/guicons`,
/// and not cached here either - the Kotlin caller (`IconPreviewCache`)
/// owns caching this, since it's the only consumer this function has.
/// Browsing/searching can touch far more icons than a user ever keeps, so
/// that cache is deliberately TTL'd and in-memory rather than an on-disk
/// one - an icon actually kept in the manifest still gets its own on-disk
/// entry once `guicons-build`/`guicons fetch` needs it, independently of
/// this.
#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_iconify_icon_preview(id: String) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || guicons_net::fetch_iconify_icon_preview(&id)).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn macro_call_at_finds_a_dotted_path_call() {
        let text = "fn f() { let x = icon!(docker.filled); }".to_string();
        let offset = text.find("docker.filled").unwrap() as u32;
        let site = macro_call_at(text, offset).expect("a call site");
        assert!(matches!(site.kind, MacroKind::Icon));
        assert_eq!(site.arg_text, "docker.filled");
    }

    #[test]
    fn parse_selector_handles_a_dotted_path() {
        let selector = parse_selector("docker.filled".to_string()).expect("a selector");
        match selector {
            IconSelector::FamilyVariant { family, size, variant } => {
                assert_eq!(family, "docker");
                assert_eq!(size, None);
                assert_eq!(variant.as_deref(), Some("filled"));
            }
            IconSelector::Iconify { .. } => panic!("expected FamilyVariant"),
        }
    }

    #[test]
    fn parse_selector_handles_an_iconify_literal() {
        let selector = parse_selector("\"mdi:home\"".to_string()).expect("a selector");
        match selector {
            IconSelector::Iconify { id } => assert_eq!(id, "mdi:home"),
            IconSelector::FamilyVariant { .. } => panic!("expected Iconify"),
        }
    }

    #[test]
    fn find_manifest_for_rust_file_finds_the_crate_roots_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"fixture\"\nversion = \"0.0.0\"\n").unwrap();
        std::fs::write(dir.path().join("icons.gui.toml"), "[docker]\nfile = \"docker.svg\"\n").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let rust_file = dir.path().join("src/main.rs");
        std::fs::write(&rust_file, "fn main() {}").unwrap();

        let found = find_manifest_for_rust_file(rust_file.to_string_lossy().into_owned());

        assert_eq!(
            found.map(PathBuf::from).map(|p| p.canonicalize().unwrap()),
            Some(dir.path().join("icons.gui.toml").canonicalize().unwrap())
        );
    }

    #[test]
    fn resolve_family_variant_finds_a_file_sourced_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("docker.svg"), "<svg/>").unwrap();
        let manifest_path = dir.path().join("icons.gui.toml");
        std::fs::write(&manifest_path, "[docker]\nfile = \"docker.svg\"\n").unwrap();

        let outcome = resolve_family_variant(manifest_path.to_string_lossy().into_owned(), "docker".to_string(), None, None);

        let ResolveOutcome::Found(entry) = outcome else { panic!("expected Found, got a different outcome") };
        assert_eq!(entry.key, "docker");
        assert!(entry.source_description.contains("docker.svg"));
        assert!(entry.source_file.is_some());
    }

    #[test]
    fn resolve_family_variant_reports_manifest_invalid_for_a_broken_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("icons.gui.toml");
        std::fs::write(&manifest_path, "[docker\nfile = \"docker.svg\"\n").unwrap();

        let outcome = resolve_family_variant(manifest_path.to_string_lossy().into_owned(), "docker".to_string(), None, None);

        let ResolveOutcome::ManifestInvalid { errors } = outcome else { panic!("expected ManifestInvalid, got a different outcome") };
        assert!(!errors.is_empty());
    }
}
