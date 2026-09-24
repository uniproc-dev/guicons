use crate::{Color, IconData};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, PoisonError};
use windows_reactor::{FontIcon, Image, ImageIcon, LayoutControl, View};

pub const DEFAULT_ICON_SIZE: f64 = 16.0;

static PAINTED_FILES: LazyLock<Mutex<HashMap<usize, String>>> = LazyLock::new(Default::default);

impl From<windows_reactor::Color> for Color {
    fn from(color: windows_reactor::Color) -> Self {
        Self { r: color.r, g: color.g, b: color.b }
    }
}

/// What `icon!(...)` expands to for an icon declared with `paint`.
///
/// WinUI loads SVG only from a file, so the painted SVG is written once per
/// color under the temp directory.
pub fn painted_icon_builder(template: &'static [u8], color: impl Into<Color>) -> IconBuilder {
    let data = IconData::PaintedSvg { template, color: color.into() };
    let path = data.svg_bytes().map(painted_file).unwrap_or_default();
    icon_builder(path)
}

fn painted_file(svg: &'static [u8]) -> String {
    let mut files = PAINTED_FILES.lock().unwrap_or_else(PoisonError::into_inner);
    files
        .entry(svg.as_ptr() as usize)
        .or_insert_with(|| write_painted_file(svg).map(|path| path.to_string_lossy().into_owned()).unwrap_or_default())
        .clone()
}

fn write_painted_file(svg: &[u8]) -> std::io::Result<PathBuf> {
    let mut hasher = DefaultHasher::new();
    svg.hash(&mut hasher);
    let dir = std::env::temp_dir().join("guicons");
    let path = dir.join(format!("painted-{:016x}.svg", hasher.finish()));
    if std::fs::read(&path).is_ok_and(|existing| existing == svg) {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir)?;
    let partial = dir.join(format!("painted-{:016x}.{}.tmp", hasher.finish(), std::process::id()));
    std::fs::write(&partial, svg)?;
    std::fs::rename(&partial, &path)?;
    Ok(path)
}

/// `Image` for a file path or URI; an unset `Image` if the source is rejected.
pub fn image_from_path(path: &str) -> Image {
    let image = Image::new();
    let image = if path.contains("://") {
        image.source(path)
    } else {
        image.source_file(path)
    };
    image.unwrap_or_default()
}

/// `ImageIcon` for a file path or URI; an unset `ImageIcon` if the source is rejected.
pub fn image_icon_from_path(path: &str) -> ImageIcon {
    let icon = ImageIcon::new();
    let icon = if path.contains("://") {
        icon.source(path)
    } else {
        icon.source_file(path)
    };
    icon.unwrap_or_default()
}

/// What `icon!(...)` expands to under the `windows-reactor` feature.
pub struct IconBuilder {
    path: String,
    width: Option<f64>,
    height: Option<f64>,
}

pub fn icon_builder(path: impl Into<String>) -> IconBuilder {
    IconBuilder { path: path.into(), width: None, height: None }
}

impl IconBuilder {
    pub fn size(mut self, size: f64) -> Self {
        self.width = Some(size);
        self.height = Some(size);
        self
    }

    pub fn width(mut self, width: f64) -> Self {
        self.width = Some(width);
        self
    }

    pub fn height(mut self, height: f64) -> Self {
        self.height = Some(height);
        self
    }

    /// An `ImageIcon` for an icon slot (`.icon(...)`), sized only if a size was set.
    pub fn build(self) -> View {
        image_icon_from_path(&self.path).width(self.width).height(self.height).into()
    }

    /// A standalone `Image`, [`DEFAULT_ICON_SIZE`] unless a size was set.
    pub fn build_element(self) -> View {
        image_from_path(&self.path)
            .width(self.width.unwrap_or(DEFAULT_ICON_SIZE))
            .height(self.height.unwrap_or(DEFAULT_ICON_SIZE))
            .into()
    }
}

impl From<IconBuilder> for View {
    fn from(builder: IconBuilder) -> Self {
        builder.build()
    }
}

/// A `FontIcon` for `codepoint`, rendered in the platform symbol font.
pub fn glyph_icon(codepoint: char) -> View {
    FontIcon::new().glyph(codepoint.to_string()).into()
}
