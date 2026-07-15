use std::ops::Range;
use tower_lsp::lsp_types::{Position, Range as LspRange};

/// Converts between byte offsets (what `guicons_core` spans use) and LSP
/// `Position`s (UTF-16 line/character), for one snapshot of a document's text.
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self { line_starts }
    }

    pub fn position(&self, text: &str, offset: usize) -> Position {
        let offset = offset.min(text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(line) => line - 1,
        };
        let line_start = self.line_starts[line];
        let character = text[line_start..offset].encode_utf16().count() as u32;
        Position { line: line as u32, character }
    }

    pub fn offset(&self, text: &str, position: Position) -> Option<usize> {
        let line_start = *self.line_starts.get(position.line as usize)?;
        let line_end = self
            .line_starts
            .get(position.line as usize + 1)
            .copied()
            .unwrap_or(text.len());
        let line_text = &text[line_start..line_end.min(text.len())];

        let mut utf16_count = 0u32;
        for (byte_idx, ch) in line_text.char_indices() {
            if utf16_count >= position.character {
                return Some(line_start + byte_idx);
            }
            utf16_count += ch.len_utf16() as u32;
        }
        Some(line_start + line_text.len())
    }

    pub fn range(&self, text: &str, span: Range<usize>) -> LspRange {
        LspRange {
            start: self.position(text, span.start),
            end: self.position(text, span.end),
        }
    }
}
