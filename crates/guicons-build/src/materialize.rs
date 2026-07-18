use super::paths::canonicalize_existing;
use guicons_core::{IconEntrySource, IconManifest};
use guicons_net::{ensure_cached, iconify_cache_path, iconify_url, url_cache_path};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct MaterializedIcon {
    pub(crate) key: String,
    pub(crate) family: String,
    pub(crate) variant: Option<String>,
    pub(crate) size: Option<u16>,
    pub(crate) dynamic: bool,
    pub(crate) backend: MaterializedIconBackend,
}

#[derive(Clone, Debug)]
pub(crate) enum MaterializedIconBackend {
    Image { path: PathBuf, kind: ImageKind },
    Glyph { font_family: String, codepoint: char },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageKind {
    Svg,
    Png,
}

/// Iconify/URL cache paths are rooted at `manifest.workspace_root()`, not
/// the build script's own `current_dir()` - those two only coincide when
/// `IconBuild::auto()`'s discovery finds `icons.gui.toml` sitting directly
/// alongside the calling crate. `IconBuild::new(path)` lets a manifest live
/// anywhere else (a monorepo root two crates up, say), and `current_dir()`
/// is always the compiling crate's own directory regardless - using it here
/// scattered a separate `.cache/guicons` per crate, invisible to `guicons
/// fetch`/the LSP diagnostics that already resolve the cache dir from the
/// manifest's real location the correct way.
pub(crate) fn materialize_icons(manifest: &IconManifest, build_out_dir: &Path) -> Vec<MaterializedIcon> {
    let icons_dir = build_out_dir.join("icons");
    let _ = fs::create_dir_all(&icons_dir);

    manifest
        .entries()
        .iter()
        .map(|entry| {
            let backend = match entry.source() {
                IconEntrySource::File(path) => {
                    let output_path =
                        icons_dir.join(format!("{}.{}", output_stem(entry.key()), image_ext(path)));
                    copy_if_changed(&canonicalize_existing(path), &output_path);
                    MaterializedIconBackend::Image {
                        kind: image_kind(&output_path),
                        path: output_path,
                    }
                }
                IconEntrySource::Iconify(id) => {
                    let output_path = icons_dir.join(format!("{}.svg", output_stem(entry.key())));
                    let cached = iconify_cache_path(manifest.workspace_root(), id);
                    ensure_cached(&cached, &iconify_url(id));
                    copy_if_changed(&cached, &output_path);
                    MaterializedIconBackend::Image {
                        kind: ImageKind::Svg,
                        path: output_path,
                    }
                }
                IconEntrySource::Url(url) => {
                    let output_path = icons_dir.join(format!("{}.svg", output_stem(entry.key())));
                    let cached = url_cache_path(manifest.workspace_root(), url);
                    ensure_cached(&cached, url);
                    copy_if_changed(&cached, &output_path);
                    MaterializedIconBackend::Image {
                        kind: ImageKind::Svg,
                        path: output_path,
                    }
                }
                IconEntrySource::Glyph(glyph) => {
                    let (font_family, codepoint) = guicons_core::parse_glyph_spec(glyph, entry.key());
                    MaterializedIconBackend::Glyph {
                        font_family,
                        codepoint,
                    }
                }
            };

            MaterializedIcon {
                key: entry.key().to_string(),
                family: entry.family().to_string(),
                variant: entry.variant().map(str::to_string),
                size: entry.size(),
                dynamic: entry.dynamic(),
                backend,
            }
        })
        .collect()
}

pub(crate) fn output_stem(key: &str) -> String {
    key.replace(['.', '_'], "-")
}

fn copy_if_changed(src: &Path, dest: &Path) {
    let src_bytes =
        fs::read(src).unwrap_or_else(|e| panic!("Failed to read {}: {e}", src.display()));
    let existing = fs::read(dest).unwrap_or_default();
    if existing != src_bytes {
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(dest, src_bytes)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", dest.display()));
    }
}

fn image_ext(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("svg") {
        "png" => "png",
        _ => "svg",
    }
}

fn image_kind(path: &Path) -> ImageKind {
    match image_ext(path) {
        "png" => ImageKind::Png,
        _ => ImageKind::Svg,
    }
}
