use guicons_core::load_icon_manifest;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
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
    let mut keys: Vec<_> = manifest.entries().iter().map(|e| e.key().to_string()).collect();
    keys.sort();
    assert_eq!(keys, vec!["back", "logo"]);
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
    assert!(
        errors.iter().any(|e| e.message.contains("recursive")),
        "{errors:?}"
    );
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
    assert!(
        errors.iter().any(|e| e.message.contains("must be a table")),
        "{errors:?}"
    );
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
    assert!(
        errors.iter().any(|e| e.message.contains("must be a string")),
        "{errors:?}"
    );
}

#[test]
fn missing_manifest_file_produces_an_error_not_a_panic() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.gui.toml");
    let (manifest, errors) = load_icon_manifest(&missing);
    assert!(manifest.entries().is_empty());
    assert!(
        errors.iter().any(|e| e.message.contains("failed to read file")),
        "{errors:?}"
    );
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
    let keys: Vec<_> = manifest.entries().iter().map(|e| e.key()).collect();
    assert_eq!(keys, vec!["alpha", "middle", "zebra"]);
}
