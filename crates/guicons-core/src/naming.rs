/// Converts a manifest key (e.g. `settings-filled`) into a Rust `SCREAMING_SNAKE_CASE`
/// identifier fragment, shared by the codegen in `guicons` and the `guicons::icon!` macro
/// so the two never drift apart on what a given key's constant is named.
pub fn rust_const_name(key: &str) -> String {
    key.replace(['.', '-'], "_").to_ascii_uppercase()
}
