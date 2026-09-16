use windows_reactor::{FontIcon, Image, ImageIcon, LayoutControl, View};

pub const DEFAULT_ICON_SIZE: f64 = 16.0;

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
