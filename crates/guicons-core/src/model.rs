use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct IconManifest {
    pub(crate) manifest_path: PathBuf,
    pub(crate) workspace_root: PathBuf,
    pub(crate) source_paths: Vec<PathBuf>,
    pub(crate) entries: Vec<IconEntry>,
    pub(crate) providers: HashMap<String, ProviderSchema>,
}

/// The known variants/sizes for one icon provider, from `[providers.<name>]`.
/// Lets `decompose_iconify_id` tell which trailing `-segment`s of a pasted
/// iconify id are suffixes versus part of the icon's own name.
#[derive(Clone, Debug, Default)]
pub struct ProviderSchema {
    pub variants: Vec<String>,
    pub sizes: Vec<u16>,
}

#[derive(Clone, Debug)]
pub struct IconEntry {
    pub(crate) key: String,
    pub(crate) family: String,
    pub(crate) variant: Option<String>,
    pub(crate) size: Option<u16>,
    pub(crate) source: IconEntrySource,
    pub(crate) dynamic: bool,
    pub(crate) windows_ico: Option<PathBuf>,
    pub(crate) span: Range<usize>,
    pub(crate) file: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconEntrySource {
    File(PathBuf),
    Iconify(String),
    Url(String),
    Glyph(String),
}

/// Parses a `[glyph]` entry's `font-family:codepoint` spec, where
/// `codepoint` is either a single literal character or a `U+XXXX` hex escape.
pub fn parse_glyph_spec(spec: &str, context: &str) -> (String, char) {
    let Some((font_family, codepoint)) = spec.split_once(':') else {
        panic!("Glyph manifest entry `{context}` must use `font-family:codepoint`, got `{spec}`");
    };
    let font_family = font_family.trim();
    let codepoint = codepoint.trim();
    let ch = if codepoint.chars().count() == 1 {
        codepoint.chars().next().unwrap()
    } else {
        let normalized = codepoint
            .strip_prefix("U+")
            .or_else(|| codepoint.strip_prefix("u+"))
            .unwrap_or(codepoint);
        let scalar = u32::from_str_radix(normalized, 16)
            .unwrap_or_else(|_| panic!("Glyph manifest entry `{context}` has invalid codepoint `{codepoint}`"));
        char::from_u32(scalar)
            .unwrap_or_else(|| panic!("Glyph manifest entry `{context}` has non-scalar codepoint `{codepoint}`"))
    };
    (font_family.to_string(), ch)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ManifestDefaults {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) provider: Option<String>,
    pub(crate) size: Option<u16>,
}

impl IconManifest {
    pub fn entries(&self) -> &[IconEntry] {
        &self.entries
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn source_paths(&self) -> &[PathBuf] {
        &self.source_paths
    }

    pub fn entry_for_key(&self, key: &str) -> Option<&IconEntry> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    pub fn entry_for_family_variant(
        &self,
        family: &str,
        size: Option<u16>,
        variant: Option<&str>,
    ) -> Option<&IconEntry> {
        self.entries.iter().find(|entry| {
            entry.family == family && entry.size == size && entry.variant.as_deref() == variant
        })
    }

    pub fn provider(&self, name: &str) -> Option<&ProviderSchema> {
        self.providers.get(name)
    }
}

impl IconEntry {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }

    /// From a `[family.N]` numeric path segment, or inherited from
    /// `defaults.size` - not a separate manifest keyword.
    pub fn size(&self) -> Option<u16> {
        self.size
    }

    pub fn source(&self) -> &IconEntrySource {
        &self.source
    }

    pub fn dynamic(&self) -> bool {
        self.dynamic
    }

    pub fn windows_ico(&self) -> Option<&Path> {
        self.windows_ico.as_deref()
    }

    /// Byte range of this entry's table in the manifest source (the
    /// variant's inline table, or the flat entry's table) - for editor
    /// tooling that needs to map a cursor position back to an entry.
    /// Only meaningful together with [`Self::file`] - spans from
    /// different files (e.g. across `[link]`) can overlap numerically.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }

    /// The specific manifest file this entry was declared in - the root
    /// manifest, or one of its `[link]`d files.
    pub fn file(&self) -> &Path {
        &self.file
    }
}
