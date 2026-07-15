use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// A single problem found while parsing a manifest, with enough location
/// info (file + byte span) to point at it in an editor.
#[derive(Clone, Debug)]
pub struct ManifestError {
    pub file: PathBuf,
    pub span: Option<Range<usize>>,
    pub message: String,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.span {
            Some(span) => write!(
                f,
                "{}:{}..{}: {}",
                self.file.display(),
                span.start,
                span.end,
                self.message
            ),
            None => write!(f, "{}: {}", self.file.display(), self.message),
        }
    }
}

impl std::error::Error for ManifestError {}

pub(crate) struct Diagnostics<'a> {
    pub(crate) file: &'a Path,
    pub(crate) errors: &'a mut Vec<ManifestError>,
}

impl Diagnostics<'_> {
    pub(crate) fn error(&mut self, span: Option<Range<usize>>, message: impl Into<String>) {
        self.errors.push(ManifestError {
            file: self.file.to_path_buf(),
            span,
            message: message.into(),
        });
    }

    pub(crate) fn push_toml_error(&mut self, error: toml_span::Error) {
        self.error(Some(error.span.into()), format_toml_error(&error));
    }

    pub(crate) fn push_deser_error(&mut self, error: toml_span::DeserError) {
        for error in error.errors {
            self.push_toml_error(error);
        }
    }
}

/// `toml_span::Error`'s own `Display` embeds `{:?}`-formatted spans
/// (`Span { start: .., end: .. }`) straight into a couple of its
/// messages, unreadable to a human reading a diagnostic - reformat those
/// specific cases; everything else already reads fine as-is.
fn format_toml_error(error: &toml_span::Error) -> String {
    if let toml_span::ErrorKind::UnexpectedKeys { keys, expected } = &error.kind {
        let keys = keys.iter().map(|(name, _)| format!("`{name}`")).collect::<Vec<_>>().join(", ");
        let expected = expected.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ");
        return format!("unexpected field(s): {keys} (expected one of: {expected})");
    }
    error.to_string()
}
