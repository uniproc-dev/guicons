use guicons_core::{load_icon_manifest, IconEntry, IconEntrySource, ManifestError};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

/// Renders `path` relative to `dir` with `/` separators, so snapshots stay
/// stable across platforms and across the random tempdir name on every run.
/// Canonicalizes both sides first: some paths here are canonicalized deeper
/// in `load_icon_manifest` (picking up Windows' `\\?\` prefix) and some
/// aren't (e.g. a file that was never found), so comparing raw strings
/// would miss the match.
fn rel(dir: &Path, path: &Path) -> String {
    let dir_canon = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let path_canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // Canonicalizing a path that doesn't exist on disk (e.g. a manifest that
    // failed to load) falls back to the raw form, which won't share the
    // `dir_canon` prefix (Windows adds a `\\?\` prefix on canonicalization)
    // - so also try stripping the raw, non-canonicalized `dir` as a fallback.
    if let Ok(suffix) = path_canon.strip_prefix(&dir_canon) {
        return suffix.display().to_string().replace('\\', "/");
    }
    if let Ok(suffix) = path.strip_prefix(dir) {
        return suffix.display().to_string().replace('\\', "/");
    }
    path.display().to_string().replace('\\', "/")
}

#[derive(Debug)]
#[allow(dead_code)] // fields exist only to show up in the debug snapshot
struct EntrySummary {
    key: String,
    family: String,
    variant: Option<String>,
    dynamic: bool,
    source: SourceSummary,
}

#[derive(Debug)]
#[allow(dead_code)] // variants exist only to show up in the debug snapshot
enum SourceSummary {
    File(String),
    Iconify(String),
    Url(String),
    Glyph(String),
}

fn summarize_entries(dir: &Path, entries: &[IconEntry]) -> Vec<EntrySummary> {
    entries
        .iter()
        .map(|entry| EntrySummary {
            key: entry.key().to_string(),
            family: entry.family().to_string(),
            variant: entry.variant().map(str::to_string),
            dynamic: entry.dynamic(),
            source: match entry.source() {
                IconEntrySource::File(path) => SourceSummary::File(rel(dir, path)),
                IconEntrySource::Iconify(id) => SourceSummary::Iconify(id.clone()),
                IconEntrySource::Url(url) => SourceSummary::Url(url.clone()),
                IconEntrySource::Glyph(glyph) => SourceSummary::Glyph(glyph.clone()),
            },
        })
        .collect()
}

#[derive(Debug)]
#[allow(dead_code)] // fields exist only to show up in the debug snapshot
struct ErrorSummary {
    file: String,
    span: Option<(usize, usize)>,
    message: String,
}

fn summarize_errors(dir: &Path, errors: &[ManifestError]) -> Vec<ErrorSummary> {
    errors
        .iter()
        .map(|error| ErrorSummary {
            file: rel(dir, &error.file),
            span: error.span.as_ref().map(|span| (span.start, span.end)),
            message: error.message.clone(),
        })
        .collect()
}

#[test]
fn include_merges_child_manifest_entries() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "icons/nav.gui.toml",
        r#"
        [back]
        file = "back.svg"
        "#,
    );
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [include]
        nav = "icons/nav.gui.toml"

        [logo]
        file = "logo.svg"
        "#,
    );

    let (manifest, errors) = load_icon_manifest(&root);
    assert!(errors.is_empty(), "{errors:?}");
    insta::assert_debug_snapshot!(summarize_entries(dir.path(), manifest.entries()));
}

#[test]
fn cyclic_include_is_reported_and_does_not_hang() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "b.gui.toml",
        r#"
        [include]
        a = "a.gui.toml"
        "#,
    );
    let a = write(
        dir.path(),
        "a.gui.toml",
        r#"
        [include]
        b = "b.gui.toml"
        "#,
    );

    let (_, errors) = load_icon_manifest(&a);
    insta::assert_debug_snapshot!(summarize_errors(dir.path(), &errors));
}

#[test]
fn include_section_must_be_a_table() {
    let dir = tempdir().unwrap();
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        include = "nope"
        "#,
    );
    let (_, errors) = load_icon_manifest(&root);
    insta::assert_debug_snapshot!(summarize_errors(dir.path(), &errors));
}

#[test]
fn include_target_must_be_a_string() {
    let dir = tempdir().unwrap();
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [include]
        nav = 5
        "#,
    );
    let (_, errors) = load_icon_manifest(&root);
    insta::assert_debug_snapshot!(summarize_errors(dir.path(), &errors));
}

#[test]
fn missing_manifest_file_produces_an_error_not_a_panic() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.gui.toml");
    let (manifest, errors) = load_icon_manifest(&missing);
    assert!(manifest.entries().is_empty());
    insta::assert_debug_snapshot!(summarize_errors(dir.path(), &errors));
}

#[test]
fn entries_are_sorted_by_key_across_includes() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "icons/nav.gui.toml",
        r#"
        [middle]
        file = "middle.svg"
        "#,
    );
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [include]
        nav = "icons/nav.gui.toml"

        [zebra]
        file = "z.svg"

        [alpha]
        file = "a.svg"
        "#,
    );
    let (manifest, errors) = load_icon_manifest(&root);
    assert!(errors.is_empty(), "{errors:?}");
    insta::assert_debug_snapshot!(summarize_entries(dir.path(), manifest.entries()));
}
