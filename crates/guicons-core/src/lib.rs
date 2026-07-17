//! Manifest model and parser for `guicons`.
//!
//! Parsing is split in two: [`parse`] turns one already-read `toml_span`
//! document into entries, while [`load`] is the entry point that reads
//! files from disk and resolves `[link]`.

mod diagnostics;
mod graph;
mod iconify_providers;
mod load;
mod manifest_scan;
mod model;
mod naming;
mod parse;
mod paths;
pub mod rust_macro;
pub mod selector;

pub use diagnostics::ManifestError;
pub use iconify_providers::{is_known_iconify_provider_prefix, known_iconify_provider_prefixes};
pub use load::{load_icon_manifest, load_icon_manifest_from_str, load_icon_manifest_or_panic};
pub use manifest_scan::find_manifest_files;
pub use model::{IconEntry, IconEntrySource, IconManifest, ProviderSchema};
pub use naming::{rust_const_name, rust_fn_name, rust_variant_name};
pub use parse::{builtin_provider_names, decompose_iconify_id, parse_glyph_spec, try_parse_glyph_spec};
pub use paths::{canonicalize_or_self, find_workspace_root_from, manifest_path_for_rust_file};
