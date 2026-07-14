mod generate;
mod materialize;
mod paths;

use std::path::PathBuf;

pub use generate::generate_rust_icon_registry;
pub use guicons_core::{IconEntry, IconEntrySource, IconManifest};
pub use guicons_net::ALLOW_NETWORK_ENV;
pub use materialize::{materialize_icons, ImageKind, MaterializedIcon, MaterializedIconBackend};

pub fn load_icon_manifest(manifest_path: &std::path::Path) -> IconManifest {
    let (manifest, errors) = guicons_core::load_icon_manifest(manifest_path);

    for source_path in manifest.source_paths() {
        println!("cargo:rerun-if-changed={}", source_path.display());
    }
    println!("cargo:rerun-if-env-changed={ALLOW_NETWORK_ENV}");

    if !errors.is_empty() {
        panic!(
            "guicons manifest at {} has errors: {:#?}",
            manifest_path.display(),
            errors
        );
    }

    manifest
}

pub struct IconBuild {
    manifest_path: PathBuf,
    materialized_root: PathBuf,
    rust_registry_out: Option<PathBuf>,
    slint_global_out: Option<PathBuf>,
}

impl IconBuild {
    pub fn new(manifest_path: impl Into<PathBuf>) -> Self {
        let out_dir = paths::out_dir();
        Self {
            manifest_path: manifest_path.into(),
            materialized_root: out_dir,
            rust_registry_out: None,
            slint_global_out: None,
        }
    }

    pub fn auto() -> Self {
        Self::new(paths::workspace_manifest_path())
    }

    pub fn materialized_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.materialized_root = root.into();
        self
    }

    pub fn emit_rust_registry(mut self, out_file: impl Into<PathBuf>) -> Self {
        self.rust_registry_out = Some(out_file.into());
        self
    }

    pub fn emit_slint_global(mut self, out_file: impl Into<PathBuf>) -> Self {
        self.slint_global_out = Some(out_file.into());
        self
    }

    pub fn run(self) {
        let manifest = load_icon_manifest(&self.manifest_path);
        let icons = materialize_icons(&manifest, &self.materialized_root);

        if let Some(out_file) = &self.rust_registry_out {
            generate::generate_rust_icon_registry_from_materialized(
                manifest.manifest_path(),
                out_file,
                &icons,
            );
        }

        if let Some(out_file) = &self.slint_global_out {
            generate::generate_slint_icon_global_from_materialized(out_file, &icons);
        }
    }
}
