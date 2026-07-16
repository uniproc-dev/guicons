//! `cargo run --bin uniffi-bindgen -- generate --library <path-to-cdylib>
//! --language kotlin --out-dir <dir>` - generates the Kotlin bindings
//! consumed by `../ide-plugin`. Not part of the library build itself;
//! only needed when regenerating bindings after this crate's exported
//! API changes.
fn main() {
    uniffi::uniffi_bindgen_main()
}
