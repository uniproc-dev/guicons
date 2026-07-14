use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// A single problem found while parsing a manifest.
///
/// Carries enough location info (file + byte span) to render as an editor
/// diagnostic, which is the whole point of not panicking here: the same
/// parser backs both `build.rs` (which turns every [`ManifestError`] into a
/// single fatal panic message) and, eventually, an LSP (which wants to show
/// every problem in the file at once, not just the first one it tripped on).
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

    /// Records a single `toml_span` parse/deserialize error, carrying its span across.
    pub(crate) fn push_toml_error(&mut self, error: toml_span::Error) {
        self.error(Some(error.span.into()), error.to_string());
    }

    /// Records every error accumulated by a `toml_span::TableHelper` pass.
    pub(crate) fn push_deser_error(&mut self, error: toml_span::DeserError) {
        for error in error.errors {
            self.push_toml_error(error);
        }
    }
}
