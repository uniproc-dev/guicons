//! Manifest-driven icon system for native Rust GUI applications.
//!
//! `guicons` is the runtime API. Build-time codegen (parsing `icons.gui.toml`,
//! materializing SVG assets, generating Rust/Slint registries) lives in the
//! separate `guicons-build` crate, used from `build.rs`.

#[cfg(feature = "slint")]
pub mod slint;

#[cfg(feature = "iced")]
pub mod iced;

#[cfg(feature = "windows-reactor")]
pub mod windows_reactor;

#[cfg(feature = "egui")]
pub mod egui;

mod paint;

#[cfg(feature = "macros")]
pub use guicons_macros::{icon, icon_data, icon_key};

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

#[macro_export]
macro_rules! include_icons {
    () => {
        $crate::include_icons!(icons);
    };
    ($module:ident) => {
        #[allow(dead_code, unused_imports)]
        mod $module {
            include!(concat!(env!("OUT_DIR"), "/icons.rs"));
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IconKey(&'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IconFamily(&'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IconVariant(&'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconRef<'a> {
    Key(IconKey),
    Name(&'a str),
    FamilyVariant {
        family: IconFamily,
        size: Option<u16>,
        variant: Option<IconVariant>,
    },
    DynamicFamilyVariant {
        family: &'a str,
        size: Option<u16>,
        variant: Option<&'a str>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconData {
    Svg(&'static [u8]),
    Png(&'static [u8]),
    Glyph {
        codepoint: char,
        font_family: &'static str,
    },
    /// An SVG declared with `paint`: `template` keeps its `currentColor`,
    /// drawn in `color` if set, otherwise in the current [`theme`]'s color.
    PaintedSvg {
        template: &'static [u8],
        colors: ThemeColors,
        color: Option<Color>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

static THEME: AtomicU8 = AtomicU8::new(Theme::Light as u8);

/// Picks which of the manifest's `paint` colors icons are drawn in.
/// Icons built before the call keep their color.
pub fn set_theme(theme: Theme) {
    THEME.store(theme as u8, Ordering::Relaxed);
}

pub fn theme() -> Theme {
    match THEME.load(Ordering::Relaxed) {
        value if value == Theme::Dark as u8 => Theme::Dark,
        _ => Theme::Light,
    }
}

/// A painted icon's colors per theme, from the manifest's `paint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThemeColors {
    pub light: Color,
    pub dark: Color,
}

impl ThemeColors {
    pub const fn get(self, theme: Theme) -> Color {
        match theme {
            Theme::Light => self.light,
            Theme::Dark => self.dark,
        }
    }
}

/// The color a painted icon's `currentColor` is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl From<[u8; 3]> for Color {
    fn from([r, g, b]: [u8; 3]) -> Self {
        Self { r, g, b }
    }
}

impl From<(u8, u8, u8)> for Color {
    fn from((r, g, b): (u8, u8, u8)) -> Self {
        Self { r, g, b }
    }
}

impl IconData {
    /// The same icon in `color` regardless of theme; `None` if it wasn't declared with `paint`.
    pub fn with_color(self, color: impl Into<Color>) -> Option<Self> {
        match self {
            Self::PaintedSvg { template, colors, .. } => {
                Some(Self::PaintedSvg { template, colors, color: Some(color.into()) })
            }
            _ => None,
        }
    }

    /// The color a painted icon is drawn in right now.
    pub fn color(self) -> Option<Color> {
        match self {
            Self::PaintedSvg { colors, color, .. } => Some(color.unwrap_or_else(|| colors.get(theme()))),
            _ => None,
        }
    }

    /// SVG bytes ready to render, painted if the icon is; `None` for PNG and glyphs.
    pub fn svg_bytes(self) -> Option<&'static [u8]> {
        match self {
            Self::Svg(bytes) => Some(bytes),
            Self::PaintedSvg { template, .. } => Some(paint::painted(template, self.color()?)),
            Self::Png(_) | Self::Glyph { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconSource {
    Static(IconData),
    Dynamic(Vec<u8>),
}

pub trait IconResolver {
    fn resolve(&self, key: IconKey) -> Option<IconSource>;
}

pub struct StaticResolver<F> {
    resolve: F,
}

#[derive(Clone, Debug, Default)]
pub struct FsResolver {
    roots: Vec<PathBuf>,
}

#[derive(Default)]
pub struct ChainResolver<'a> {
    resolvers: Vec<&'a dyn IconResolver>,
}

impl IconKey {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl IconFamily {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl IconVariant {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl<'a> IconRef<'a> {
    pub const fn family_variant(family: IconFamily, size: Option<u16>, variant: Option<IconVariant>) -> Self {
        Self::FamilyVariant { family, size, variant }
    }

    pub const fn dynamic_family_variant(family: &'a str, size: Option<u16>, variant: Option<&'a str>) -> Self {
        Self::DynamicFamilyVariant { family, size, variant }
    }
}

impl<F> StaticResolver<F> {
    pub const fn new(resolve: F) -> Self {
        Self { resolve }
    }
}

impl<F> IconResolver for StaticResolver<F>
where
    F: Fn(IconKey) -> Option<IconData>,
{
    fn resolve(&self, key: IconKey) -> Option<IconSource> {
        (self.resolve)(key).map(IconSource::Static)
    }
}

impl FsResolver {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
        }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }
}

impl IconResolver for FsResolver {
    fn resolve(&self, key: IconKey) -> Option<IconSource> {
        let file_name = format!("{}.svg", key.as_str().replace(['.', '_'], "-"));
        self.roots.iter().find_map(|root| {
            let path = root.join(&file_name);
            std::fs::read(path).ok().map(IconSource::Dynamic)
        })
    }
}

impl<'a> ChainResolver<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, resolver: &'a dyn IconResolver) {
        self.resolvers.push(resolver);
    }
}

impl IconResolver for ChainResolver<'_> {
    fn resolve(&self, key: IconKey) -> Option<IconSource> {
        self.resolvers.iter().find_map(|resolver| resolver.resolve(key))
    }
}

impl<'a> From<IconKey> for IconRef<'a> {
    fn from(value: IconKey) -> Self {
        Self::Key(value)
    }
}

impl<'a> From<&'a str> for IconRef<'a> {
    fn from(value: &'a str) -> Self {
        Self::Name(value)
    }
}
