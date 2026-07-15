use guicons_core::{load_icon_manifest, load_icon_manifest_from_str, IconEntry, IconEntrySource, ManifestError};
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

/// Renders `path` relative to `dir` with `/` separators, for stable snapshots.
fn rel(dir: &Path, path: &Path) -> String {
    let dir_canon = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let path_canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
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
        [link]
        includes = ["icons/nav.gui.toml"]

        [logo]
        file = "logo.svg"
        "#,
    );

    let (manifest, errors) = load_icon_manifest(&root);
    assert!(errors.is_empty(), "{errors:?}");
    insta::assert_debug_snapshot!(summarize_entries(dir.path(), manifest.entries()));
}

#[test]
fn include_merges_child_manifest_providers() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "icons/nav.gui.toml",
        r#"
        [providers.acme-nav]
        variants = ["thin", "light", "bold", "fill", "duotone"]

        [back]
        file = "back.svg"
        "#,
    );
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [providers.acme-root]
        variants = ["regular", "filled"]
        sizes = [24]

        [link]
        includes = ["icons/nav.gui.toml"]
        "#,
    );

    let (manifest, errors) = load_icon_manifest(&root);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(manifest.provider("acme-root").is_some());
    assert!(manifest.provider("acme-nav").is_some());
    assert_eq!(manifest.provider("acme-nav").unwrap().variants, vec!["thin", "light", "bold", "fill", "duotone"]);
}

#[test]
fn a_files_own_provider_wins_over_an_included_ones_same_name() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "icons/nav.gui.toml",
        r#"
        [providers.acme]
        variants = ["from-include"]
        "#,
    );
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [providers.acme]
        variants = ["from-root"]

        [link]
        includes = ["icons/nav.gui.toml"]
        "#,
    );

    let (manifest, errors) = load_icon_manifest(&root);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(manifest.provider("acme").unwrap().variants, vec!["from-root"]);
}

#[test]
fn cyclic_include_is_reported_and_does_not_hang() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "b.gui.toml",
        r#"
        [link]
        includes = ["a.gui.toml"]
        "#,
    );
    let a = write(
        dir.path(),
        "a.gui.toml",
        r#"
        [link]
        includes = ["b.gui.toml"]
        "#,
    );

    let (_, errors) = load_icon_manifest(&a);
    insta::assert_debug_snapshot!(summarize_errors(dir.path(), &errors));
}

#[test]
fn link_section_must_be_a_table() {
    let dir = tempdir().unwrap();
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        link = "nope"
        "#,
    );
    let (_, errors) = load_icon_manifest(&root);
    insta::assert_debug_snapshot!(summarize_errors(dir.path(), &errors));
}

#[test]
fn link_includes_must_be_an_array() {
    let dir = tempdir().unwrap();
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [link]
        includes = "nope"
        "#,
    );
    let (_, errors) = load_icon_manifest(&root);
    insta::assert_debug_snapshot!(summarize_errors(dir.path(), &errors));
}

#[test]
fn link_includes_entries_must_be_strings() {
    let dir = tempdir().unwrap();
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [link]
        includes = [5]
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
        [link]
        includes = ["icons/nav.gui.toml"]

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

#[test]
fn load_from_str_prefers_the_given_content_over_the_on_disk_file() {
    let dir = tempdir().unwrap();
    let root = write(
        dir.path(),
        "icons.gui.toml",
        r#"
        [docker]
        file = "docker.svg"
        "#,
    );

    let unsaved_content = r#"
    [docker]
    file = "docker.svg"

    [settings]
    file = "settings.svg"
    "#;
    let (manifest, errors) = load_icon_manifest_from_str(&root, unsaved_content);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(manifest.entry_for_key("settings").is_some(), "should reflect unsaved content, not the file on disk");

    let (on_disk_manifest, errors) = load_icon_manifest(&root);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(on_disk_manifest.entry_for_key("settings").is_none());
}

#[test]
fn load_from_str_still_resolves_includes_from_disk() {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "icons/nav.gui.toml",
        r#"
        [back]
        file = "back.svg"
        "#,
    );
    let root_path = dir.path().join("icons.gui.toml");

    let unsaved_content = r#"
    [link]
    includes = ["icons/nav.gui.toml"]

    [docker]
    file = "docker.svg"
    "#;
    let (manifest, errors) = load_icon_manifest_from_str(&root_path, unsaved_content);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(manifest.entry_for_key("docker").is_some());
    assert!(manifest.entry_for_key("back").is_some());
}

#[test]
fn entries_carry_the_file_they_were_declared_in_even_across_includes() {
    let dir = tempdir().unwrap();
    let nav = write(
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
        [link]
        includes = ["icons/nav.gui.toml"]

        [docker]
        file = "docker.svg"
        "#,
    );

    let (manifest, errors) = load_icon_manifest(&root);
    assert!(errors.is_empty(), "{errors:?}");

    let root_canon = fs::canonicalize(&root).unwrap_or(root);
    let nav_canon = fs::canonicalize(&nav).unwrap_or(nav);

    assert_eq!(manifest.entry_for_key("docker").unwrap().file(), root_canon);
    assert_eq!(manifest.entry_for_key("back").unwrap().file(), nav_canon);
}
