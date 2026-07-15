use miette::{LabeledSpan, MietteDiagnostic, NamedSource, Report, Severity};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Loads `manifest_path` and turns every [`guicons_core::ManifestError`]
/// into a pretty-printable [`miette::Report`] - a source-code snippet with
/// the offending span underlined, when the error has one. Returns the
/// number of entries that parsed successfully alongside the reports (empty
/// if the manifest is fully valid).
pub fn check(manifest_path: &Path) -> (usize, Vec<Report>) {
    let (manifest, errors) = guicons_core::load_icon_manifest(manifest_path);
    let mut source_cache: HashMap<PathBuf, String> = HashMap::new();

    let reports = errors
        .iter()
        .map(|error| {
            let mut diagnostic = MietteDiagnostic::new(error.message.clone()).with_severity(Severity::Error);
            if let Some(span) = &error.span {
                diagnostic = diagnostic.with_label(LabeledSpan::at(span.start..span.end, "here"));
            }

            let mut report = Report::new(diagnostic);
            if error.span.is_some() {
                let source = source_cache
                    .entry(error.file.clone())
                    .or_insert_with(|| fs::read_to_string(&error.file).unwrap_or_default())
                    .clone();
                report = report.with_source_code(NamedSource::new(display_path(&error.file), source));
            }
            report
        })
        .collect();

    (manifest.entries().len(), reports)
}

/// Relative to the current directory when possible (both shorter and
/// free of Windows' `\\?\` verbatim-path prefix, which `canonicalize`
/// adds and which `Path::display()` otherwise shows verbatim).
fn display_path(path: &Path) -> String {
    let Ok(cwd) = std::env::current_dir() else {
        return path.display().to_string();
    };
    let cwd = fs::canonicalize(&cwd).unwrap_or(cwd);
    path.strip_prefix(&cwd).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn check_reports_zero_errors_for_a_valid_manifest() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("docker.svg"), "<svg/>").unwrap();
        let path = dir.path().join("icons.gui.toml");
        fs::write(&path, "[docker]\nfile = \"docker.svg\"\n").unwrap();

        let (entry_count, reports) = check(&path);
        assert_eq!(entry_count, 1);
        assert!(reports.is_empty());
    }

    #[test]
    fn check_reports_a_pretty_diagnostic_for_an_unknown_field() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("icons.gui.toml");
        fs::write(&path, "[docker]\nfile = \"docker.svg\"\nfile1 = \"docker.svg\"\n").unwrap();

        let (_, reports) = check(&path);
        assert_eq!(reports.len(), 1);
        let rendered = format!("{:?}", reports[0]);
        assert!(rendered.contains("unexpected field"), "{rendered}");
        assert!(rendered.contains("file1"), "{rendered}");
    }
}
