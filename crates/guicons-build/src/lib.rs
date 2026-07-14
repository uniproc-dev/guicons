mod generate;
mod materialize;
mod paths;

use guicons_core::IconManifest;
use materialize::materialize_icons;
use std::path::PathBuf;

pub use guicons_net::ALLOW_NETWORK_ENV;

fn load_icon_manifest(manifest_path: &std::path::Path) -> IconManifest {
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

/// What to generate from the manifest. File names are fixed, not
/// caller-configurable: `guicons`'s `include_icons!()` macro hardcodes
/// `env!("OUT_DIR")` + `icons.rs`, so letting the Rust registry land
/// anywhere else would silently break it.
pub enum Emit {
    /// `OUT_DIR/icons.rs` - the typed registry consumed by `include_icons!()`.
    Rust,
    /// `OUT_DIR/icons.slint` - the `Icon` component and per-icon assets.
    Slint,
}

pub struct IconBuild {
    manifest_path: PathBuf,
    out_dir: PathBuf,
    materialized_root: PathBuf,
    emit_rust: bool,
    emit_slint: bool,
}

impl IconBuild {
    pub fn new(manifest_path: impl Into<PathBuf>) -> Self {
        let out_dir = paths::out_dir();
        Self {
            manifest_path: manifest_path.into(),
            materialized_root: out_dir.clone(),
            out_dir,
            emit_rust: false,
            emit_slint: false,
        }
    }

    pub fn auto() -> Self {
        Self::new(paths::workspace_manifest_path())
    }

    pub fn materialized_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.materialized_root = root.into();
        self
    }

    pub fn emit(mut self, target: Emit) -> Self {
        match target {
            Emit::Rust => self.emit_rust = true,
            Emit::Slint => self.emit_slint = true,
        }
        self
    }

    pub fn build(self) {
        let manifest = load_icon_manifest(&self.manifest_path);
        let icons = materialize_icons(&manifest, &self.materialized_root);

        if self.emit_rust {
            let out_file = self.out_dir.join("icons.rs");
            generate::generate_rust_icon_registry_from_materialized(
                manifest.manifest_path(),
                &out_file,
                &icons,
            );
        }

        if self.emit_slint {
            let out_file = self.out_dir.join("icons.slint");
            generate::generate_slint_icon_global_from_materialized(&out_file, &icons);
        }
    }
}
