//! Real end-to-end tests: invokes the actual compiled `icons` binary as a
//! subprocess and asserts on its exit code and stdout/stderr, unlike
//! `add.rs`/`fetch.rs` (which call `guicons_cli::{add, fetch}` directly as
//! library functions - useful for testing that logic, but never exercise
//! argument parsing, exit codes, or the binary's actual printed output).

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

fn icons(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_icons"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run the icons binary")
}

#[test]
fn check_exits_zero_and_prints_ok_for_a_valid_manifest() {
    let dir = tempdir().unwrap();
    write(dir.path(), "docker.svg", "<svg/>");
    write(dir.path(), "icons.gui.toml", "[docker]\nfile = \"docker.svg\"\n");

    let output = icons(dir.path(), &["check"]);

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("OK"), "{stdout}");
    assert!(stdout.contains('1'), "should mention the one icon found: {stdout}");
}

#[test]
fn check_exits_nonzero_and_prints_a_diagnostic_for_an_invalid_manifest() {
    let dir = tempdir().unwrap();
    write(dir.path(), "icons.gui.toml", "[docker]\nfile = \"docker.svg\"\nfile1 = \"docker.svg\"\n");

    let output = icons(dir.path(), &["check"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected field"), "{stderr}");
    assert!(stderr.contains("file1"), "{stderr}");
    assert!(stderr.contains("1 error"), "{stderr}");
}

#[test]
fn check_exits_nonzero_for_a_missing_manifest_file() {
    let dir = tempdir().unwrap();

    let output = icons(dir.path(), &["check", "--manifest", "does-not-exist.gui.toml"]);

    assert!(!output.status.success());
}

#[test]
fn add_writes_a_new_entry_and_prints_the_key() {
    let dir = tempdir().unwrap();
    write(dir.path(), "logo.svg", "<svg/>");

    let output = icons(dir.path(), &["add", "logo.svg", "--name", "my-logo"]);

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-logo"), "{stdout}");

    let (manifest, errors) = guicons_core::load_icon_manifest(&dir.path().join("icons.gui.toml"));
    assert!(errors.is_empty(), "{errors:?}");
    assert!(manifest.entry_for_key("my-logo").is_some());
}

#[test]
fn add_without_force_fails_on_a_duplicate_key() {
    let dir = tempdir().unwrap();
    write(dir.path(), "logo.svg", "<svg/>");
    write(dir.path(), "icons.gui.toml", "[my-logo]\nfile = \"logo.svg\"\n");

    let output = icons(dir.path(), &["add", "logo.svg", "--name", "my-logo"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--force"), "{stderr}");
}
