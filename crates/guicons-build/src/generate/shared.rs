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

