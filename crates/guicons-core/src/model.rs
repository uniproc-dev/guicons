use std::collections::HashMap;
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconEntrySource {
    File(PathBuf),
    Iconify(String),
    Url(String),
    Glyph(String),
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
}
