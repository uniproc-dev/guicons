use crate::{Color, IconData, IconSource};
use iced::widget::{image, svg};

pub fn svg_handle_from_data(data: IconData) -> Option<svg::Handle> {
    data.svg_bytes().map(svg::Handle::from_memory)
}

impl From<iced::Color> for Color {
    fn from(color: iced::Color) -> Self {
        let [r, g, b, _] = color.into_rgba8();
        Self { r, g, b }
    }
}

/// `IconSource::Dynamic` bytes are always SVG - resolvers that produce
/// dynamic data (e.g. `FsResolver`) only ever read `.svg` files.
pub fn svg_handle_from_source(source: IconSource) -> Option<svg::Handle> {
    match source {
        IconSource::Static(data) => svg_handle_from_data(data),
        IconSource::Dynamic(bytes) => Some(svg::Handle::from_memory(bytes)),
    }
}

pub fn image_handle_from_data(data: IconData) -> Option<image::Handle> {
    match data {
        IconData::Png(bytes) => Some(image::Handle::from_bytes(bytes)),
        _ => None,
    }
}

pub fn glyph_from_data(data: IconData) -> Option<(&'static str, char)> {
    match data {
        IconData::Glyph { codepoint, font_family } => Some((font_family, codepoint)),
        _ => None,
    }
}
