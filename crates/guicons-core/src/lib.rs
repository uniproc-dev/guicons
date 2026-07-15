//! Manifest model and parser for `guicons`.
//!
//! Parsing is split in two: [`parse`] turns one already-read `toml_span`
//! document into entries, while [`load`] is the entry point that reads
//! files from disk and resolves `[include]`.

mod diagnostics;
mod load;
mod model;
mod naming;
mod parse;
mod paths;

pub use diagnostics::ManifestError;
pub use load::{load_icon_manifest, load_icon_manifest_or_panic};
pub use model::{parse_glyph_spec, IconEntry, IconEntrySource, IconManifest, ProviderSchema};
pub use naming::{rust_const_name, rust_fn_name};
pub use parse::decompose_iconify_id;
pub use paths::{canonicalize_or_self, find_workspace_root_from};
