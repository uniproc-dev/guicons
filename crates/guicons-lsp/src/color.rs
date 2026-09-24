use crate::manifest_text::paint_values;
use crate::position::LineIndex;
use crate::Backend;
use guicons_core::PaintColor;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

fn to_lsp_color(color: PaintColor) -> Color {
    let channel = |value: u8| f32::from(value) / 255.0;
    Color { red: channel(color.r), green: channel(color.g), blue: channel(color.b), alpha: 1.0 }
}

fn from_lsp_color(color: Color) -> PaintColor {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    PaintColor { r: channel(color.red), g: channel(color.green), b: channel(color.blue) }
}

/// A swatch for every `paint = "#rgb"`/`"#rrggbb"` value, covering the text inside the quotes.
pub(crate) fn document_colors(text: &str) -> Vec<ColorInformation> {
    let index = LineIndex::new(text);
    paint_values(text)
        .into_iter()
        .filter_map(|(span, value)| {
            let color = PaintColor::parse(&value).ok()?;
            Some(ColorInformation { range: index.range(text, span), color: to_lsp_color(color) })
        })
        .collect()
}

/// `#rrggbb` written over `range`.
pub(crate) fn color_presentations(color: Color, range: Range) -> Vec<ColorPresentation> {
    let hex = from_lsp_color(color).to_hex();
    vec![ColorPresentation {
        label: hex.clone(),
        text_edit: Some(TextEdit { range, new_text: hex }),
        additional_text_edits: None,
    }]
}

impl Backend {
    pub(crate) async fn document_color_impl(&self, params: DocumentColorParams) -> Result<Vec<ColorInformation>> {
        let uri = params.text_document.uri;
        let Some(path) = Self::path_for_uri(&uri) else { return Ok(Vec::new()) };
        if path.extension().is_some_and(|ext| ext == "rs") {
            return Ok(Vec::new());
        }
        let Some(text) = self.document_text(&uri).await else { return Ok(Vec::new()) };
        Ok(document_colors(&text))
    }
}
