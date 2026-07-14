use crate::{IconData, IconSource};
use slint::Image;

pub fn image_from_data(data: IconData) -> Image {
    match data {
        IconData::Svg(bytes) => Image::load_from_svg_data(bytes).unwrap_or_default(),
        IconData::Png(_) => Image::default(),
        IconData::Glyph { .. } => Image::default(),
    }
}

pub fn image_from_source(source: IconSource) -> Image {
    match source {
        IconSource::Static(data) => image_from_data(data),
        IconSource::Dynamic(bytes) => Image::load_from_svg_data(&bytes).unwrap_or_default(),
    }
}
