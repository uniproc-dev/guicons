use std::fs;
use std::path::Path;

pub(super) fn write_if_changed(path: &Path, content: &str) {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing != content {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(path, content).unwrap_or_else(|e| panic!("Failed to write {}: {e}", path.display()));
    }
}

/// A manifest key (`settings-filled`) as a Rust/Slint `UpperCamelCase`
/// type-name fragment - used both for Rust builder struct names and Slint
/// component names, so it lives here rather than in either codegen module.
pub(super) fn rust_variant_name(key: &str) -> String {
    let mut result = String::new();
    for segment in key.split(['.', '-', '_']) {
        if segment.is_empty() {
            continue;
        }
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_ascii_uppercase());
            result.push_str(chars.as_str());
        }
    }
    if result.is_empty() {
        "Unknown".to_string()
    } else if result.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("Icon{result}")
    } else {
        result
    }
}
