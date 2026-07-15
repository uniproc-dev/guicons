use crate::{IconData, IconSource};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// `None` for `Glyph` data - Slint's `Image` has no notion of a font glyph,
/// use [`glyph_from_data`] for that case instead.
pub fn image_from_data(data: IconData) -> Option<Image> {
    match data {
        IconData::Svg(bytes) => Image::load_from_svg_data(bytes).ok(),
        IconData::Png(bytes) => image_from_png_bytes(bytes),
        IconData::Glyph { .. } => None,
    }
}

/// `IconSource::Dynamic` bytes are always SVG - resolvers that produce
/// dynamic data (e.g. `FsResolver`) only ever read `.svg` files.
pub fn image_from_source(source: IconSource) -> Option<Image> {
    match source {
        IconSource::Static(data) => image_from_data(data),
        IconSource::Dynamic(bytes) => Image::load_from_svg_data(&bytes).ok(),
    }
}

pub fn glyph_from_data(data: IconData) -> Option<(&'static str, char)> {
    match data {
        IconData::Glyph { codepoint, font_family } => Some((font_family, codepoint)),
        _ => None,
    }
}

fn image_from_png_bytes(bytes: &[u8]) -> Option<Image> {
    let decoded = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(decoded.as_raw(), width, height);
    Some(Image::from_rgba8(buffer))
}
