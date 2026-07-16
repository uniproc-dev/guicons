//! UniFFI bindings for `../ide-plugin`'s Kotlin/JVM side.
//!
//! Every function here is a thin pass-through to `guicons-core`,
//! converting only what UniFFI's type set requires (its own mirror
//! `enum`s/`struct`s, `usize`/`u16` <-> `u32`, `Path`/`PathBuf` <->
//! `String`) - no logic of its own worth testing independently.

uniffi::setup_scaffolding!();

use guicons_core::rust_macro::MacroKind as CoreMacroKind;
use guicons_core::selector::IconSelector as CoreIconSelector;
use guicons_core::IconEntrySource;
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
    /// `file` source. Rendering `iconify`/`url`/`glyph` sources would
    /// also need `guicons-net`'s cache/fetch logic ported over, which
    /// this crate deliberately doesn't do yet.
    pub source_file: Option<String>,
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

    let (source_description, source_file) = match entry.source() {
        IconEntrySource::File(path) => {
            (format!("file `{}`", manifest.display_path(path)), Some(path.to_string_lossy().into_owned()))
        }
        IconEntrySource::Iconify(id) => (format!("iconify `{id}`"), None),
        IconEntrySource::Url(url) => (format!("url `{url}`"), None),
        IconEntrySource::Glyph(spec) => (format!("glyph `{spec}`"), None),
    };

    ResolveOutcome::Found(ResolvedEntry {
        key: entry.key().to_string(),
        family: entry.family().to_string(),
        size: entry.size(),
        variant: entry.variant().map(str::to_string),
        source_description,
        source_file,
    })
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
