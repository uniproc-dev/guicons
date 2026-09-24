use crate::{Color, IconData, IconSource};
use egui::ImageSource;
use egui::load::Bytes;
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

static URIS: LazyLock<Mutex<HashMap<(usize, usize), &'static str>>> = LazyLock::new(Default::default);

/// `None` for `Glyph` data; use [`glyph_from_data`] for that case.
///
/// Rendering needs an egui image loader for the format, e.g. `egui_extras`
/// with its `svg` feature and `egui_extras::install_image_loaders`.
pub fn image_source_from_data(data: IconData) -> Option<ImageSource<'static>> {
    let (bytes, extension) = match data {
        IconData::Png(bytes) => (bytes, "png"),
        other => (other.svg_bytes()?, "svg"),
    };
    Some(ImageSource::Bytes {
        uri: Cow::Borrowed(uri(bytes, extension)),
        bytes: Bytes::Static(bytes),
    })
}

/// `IconSource::Dynamic` bytes are always SVG.
pub fn image_source_from_source(source: IconSource) -> Option<ImageSource<'static>> {
    match source {
        IconSource::Static(data) => image_source_from_data(data),
        IconSource::Dynamic(bytes) => {
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            Some(ImageSource::Bytes {
                uri: Cow::Owned(format!("bytes://guicons/dynamic-{:016x}.svg", hasher.finish())),
                bytes: Bytes::Shared(Arc::from(bytes)),
            })
        }
    }
}

pub fn glyph_from_data(data: IconData) -> Option<(&'static str, char)> {
    match data {
        IconData::Glyph { codepoint, font_family } => Some((font_family, codepoint)),
        _ => None,
    }
}

impl From<egui::Color32> for Color {
    fn from(color: egui::Color32) -> Self {
        let [r, g, b, _] = color.to_srgba_unmultiplied();
        Self { r, g, b }
    }
}

fn uri(bytes: &'static [u8], extension: &'static str) -> &'static str {
    let mut uris = URIS.lock().unwrap_or_else(PoisonError::into_inner);
    uris.entry((bytes.as_ptr() as usize, bytes.len()))
        .or_insert_with(|| format!("bytes://guicons/{:p}-{}.{extension}", bytes.as_ptr(), bytes.len()).leak())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    const PNG: &[u8] = b"\x89PNG";
    const TEMPLATE: &[u8] = b"<svg><path fill=\"currentColor\"/></svg>";

    fn uri<'a>(source: &'a ImageSource<'static>) -> &'a str {
        match source {
            ImageSource::Bytes { uri, .. } => uri,
            other => panic!("expected bytes source, got {other:?}"),
        }
    }

    fn bytes<'a>(source: &'a ImageSource<'static>) -> &'a [u8] {
        match source {
            ImageSource::Bytes { bytes, .. } => bytes,
            other => panic!("expected bytes source, got {other:?}"),
        }
    }

    fn painted(color: Color) -> ImageSource<'static> {
        let colors = crate::ThemeColors { light: color, dark: color };
        image_source_from_data(IconData::PaintedSvg { template: TEMPLATE, colors, color: None }).unwrap()
    }

    #[test]
    fn static_uri_extension_matches_the_format() {
        let svg = image_source_from_data(IconData::Svg(SVG)).unwrap();
        let png = image_source_from_data(IconData::Png(PNG)).unwrap();
        assert!(uri(&svg).starts_with("bytes://guicons/") && uri(&svg).ends_with(".svg"));
        assert!(uri(&png).ends_with(".png"));
    }

    #[test]
    fn static_uri_is_stable_for_the_same_data() {
        let first = image_source_from_data(IconData::Svg(SVG)).unwrap();
        let second = image_source_from_data(IconData::Svg(SVG)).unwrap();
        assert_eq!(uri(&first), uri(&second));
    }

    #[test]
    fn unpainted_svg_is_passed_through() {
        let source = image_source_from_data(IconData::Svg(TEMPLATE)).unwrap();
        assert_eq!(bytes(&source), TEMPLATE);
    }

    #[test]
    fn painted_svg_is_drawn_in_its_color() {
        let white = painted(Color::rgb(255, 255, 255));
        let red = painted(Color::rgb(255, 0, 0));
        assert_eq!(bytes(&white), b"<svg><path fill=\"#ffffff\"/></svg>");
        assert_eq!(bytes(&red), b"<svg><path fill=\"#ff0000\"/></svg>");
        assert_ne!(uri(&white), uri(&red));
        assert_eq!(uri(&white), uri(&painted(Color::rgb(255, 255, 255))));
    }

    #[test]
    fn dynamic_uri_depends_on_content() {
        let first = image_source_from_source(IconSource::Dynamic(b"<svg a/>".to_vec())).unwrap();
        let same = image_source_from_source(IconSource::Dynamic(b"<svg a/>".to_vec())).unwrap();
        let other = image_source_from_source(IconSource::Dynamic(b"<svg b/>".to_vec())).unwrap();
        assert_eq!(uri(&first), uri(&same));
        assert_ne!(uri(&first), uri(&other));
        assert!(uri(&first).ends_with(".svg"));
    }

    #[test]
    fn glyph_is_not_an_image() {
        let glyph = IconData::Glyph { codepoint: '\u{E001}', font_family: "Icons" };
        assert!(image_source_from_data(glyph).is_none());
        assert_eq!(glyph_from_data(glyph), Some(("Icons", '\u{E001}')));
    }

    #[test]
    fn color32_converts_without_premultiplication() {
        assert_eq!(Color::from(egui::Color32::from_rgb(10, 20, 30)), Color::rgb(10, 20, 30));
    }
}
