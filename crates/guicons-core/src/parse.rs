//! Turns one already-parsed `toml_span` document into `IconEntry` values.
//!
//! Nothing here touches the filesystem or knows about `[include]` — that's
//! [`crate::load`]'s job. This module only walks the table tree produced by
//! `toml_span::parse` and validates/extracts entries from it.

use crate::diagnostics::Diagnostics;
use crate::model::{IconEntry, IconEntrySource, IconManifest, ManifestDefaults, ProviderSchema};
use crate::paths::resolve_workspace_path;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use toml_span::de_helpers::TableHelper;
use toml_span::value::{Table, Value, ValueInner};
use toml_span::Span;

const ENTRY_KEYS: &[&str] = &[
    "file",
    "iconify",
    "url",
    "glyph",
    "windows-ico",
    "dynamic",
    "root",
];
pub(crate) const RESERVED_TOP_LEVEL: &[&str] = &["defaults", "include", "providers"];

fn take_table<'de>(value: &mut Value<'de>) -> Option<Table<'de>> {
    match value.take() {
        ValueInner::Table(table) => Some(table),
        _ => None,
    }
}

fn split_family_and_size(path: &[String]) -> (String, Option<u16>) {
    if let Some((last, rest)) = path.split_last() {
        if let Ok(size) = last.parse::<u16>() {
            return (rest.join("-"), Some(size));
        }
    }
    (path.join("-"), None)
}

pub(crate) fn collect_entries(
    path: Vec<String>,
    table: Table<'_>,
    workspace_root: &Path,
    defaults: &ManifestDefaults,
    diags: &mut Diagnostics,
    acc: &mut Vec<IconEntry>,
) {
    let mut table = table;
    if let Some(mut variants_value) = table.remove("variants") {
        let key_prefix = path.join("-");
        let (family, explicit_size) = split_family_and_size(&path);
        let variants_span = variants_value.span;
        let Some(variants_table) = take_table(&mut variants_value) else {
            diags.error(
                Some(variants_span.into()),
                format!("`variants` in `{key_prefix}` must be a table"),
            );
            return;
        };

        for (variant_key, mut variant_value) in variants_table {
            let variant = variant_key.name.to_string();
            let entry_span = variant_value.span;
            match take_table(&mut variant_value) {
                Some(variant_table) => {
                    if let Some(entry) = parse_entry(
                        format!("{key_prefix}-{variant}"),
                        family.clone(),
                        Some(variant),
                        variant_table,
                        entry_span,
                        workspace_root,
                        defaults,
                        explicit_size,
                        diags,
                    ) {
                        acc.push(entry);
                    }
                }
                None => diags.error(
                    Some(entry_span.into()),
                    format!("variant `{key_prefix}.{variant}` must be an inline table"),
                ),
            }
        }
        return;
    }

    let is_entry = table
        .keys()
        .any(|key| ENTRY_KEYS.contains(&key.name.as_ref()));
    if is_entry {
        let key = path.join("-");
        let (family, explicit_size) = split_family_and_size(&path);
        let table_span = table_span(&table);
        if let Some(entry) = parse_entry(
            key,
            family,
            None,
            table,
            table_span,
            workspace_root,
            defaults,
            explicit_size,
            diags,
        ) {
            acc.push(entry);
        }
        return;
    }

    for (key, mut value) in table {
        let key_name = key.name.to_string();
        if path.is_empty() && RESERVED_TOP_LEVEL.contains(&key_name.as_str()) {
            continue;
        }
        let value_span = value.span;
        let Some(sub_table) = take_table(&mut value) else {
            diags.error(
                Some(value_span.into()),
                format!("unexpected value at icon manifest group {path:?}: `{key_name}`"),
            );
            continue;
        };
        let mut next_path = path.clone();
        next_path.push(key_name);
        collect_entries(next_path, sub_table, workspace_root, defaults, diags, acc);
    }
}

fn table_span(table: &Table<'_>) -> Span {
    table.values().fold(Span::new(usize::MAX, 0), |acc, value| {
        Span::new(acc.start.min(value.span.start), acc.end.max(value.span.end))
    })
}

fn parse_entry(
    key: String,
    family: String,
    variant: Option<String>,
    table: Table<'_>,
    table_span: Span,
    workspace_root: &Path,
    defaults: &ManifestDefaults,
    explicit_size: Option<u16>,
    diags: &mut Diagnostics,
) -> Option<IconEntry> {
    let mut th = TableHelper::from((table, table_span));

    let has_file = th.contains("file");
    let has_iconify = th.contains("iconify");
    let has_url = th.contains("url");
    let has_glyph = th.contains("glyph");

    let root: Option<String> = th.optional("root");
    let file: Option<String> = th.optional("file");
    let iconify: Option<String> = th.optional("iconify");
    let url: Option<String> = th.optional("url");
    let glyph: Option<String> = th.optional("glyph");
    let windows_ico: Option<String> = th.optional("windows-ico");
    let dynamic: bool = th.optional("dynamic").unwrap_or(false);

    if let Err(err) = th.finalize(None) {
        diags.push_deser_error(err);
        return None;
    }

    let roots: Vec<PathBuf> = root
        .map(|value| vec![resolve_workspace_path(workspace_root, &value)])
        .unwrap_or_else(|| defaults.roots.clone());

    let resolved_size = explicit_size.or(defaults.size);

    let default_iconify = if has_iconify {
        None
    } else {
        default_iconify_id(&family, variant.as_deref(), resolved_size, defaults)
    };
    let iconify = iconify.or_else(|| default_iconify.clone());

    let explicit_source_count = [has_file, has_iconify, has_url, has_glyph]
        .into_iter()
        .filter(|present| *present)
        .count();
    let source_count = explicit_source_count + usize::from(default_iconify.is_some());

    if source_count != 1 {
        diags.error(
            Some(table_span.into()),
            format!(
                "icon manifest entry `{key}` must define exactly one source (file/iconify/url/glyph), found {source_count}"
            ),
        );
        return None;
    }

    let source = if let Some(path) = file {
        IconEntrySource::File(resolve_file_from_roots(&roots, &path))
    } else if let Some(id) = iconify {
        IconEntrySource::Iconify(id)
    } else if let Some(url) = url {
        IconEntrySource::Url(url)
    } else if let Some(glyph) = glyph {
        IconEntrySource::Glyph(glyph)
    } else {
        diags.error(
            Some(table_span.into()),
            format!("icon manifest entry `{key}` has no usable source"),
        );
        return None;
    };

    let windows_ico = windows_ico.map(|value| resolve_file_from_roots(&roots, &value));

    Some(IconEntry {
        key,
        family,
        variant,
        size: resolved_size,
        source,
        dynamic,
        windows_ico,
        span: table_span.into(),
    })
}

pub(crate) fn parse_defaults(
    defaults_value: Option<Value<'_>>,
    workspace_root: &Path,
    manifest_dir: &Path,
    diags: &mut Diagnostics,
) -> ManifestDefaults {
    let fallback_roots = || vec![manifest_dir.to_path_buf(), workspace_root.to_path_buf()];

    let Some(mut defaults_value) = defaults_value else {
        return ManifestDefaults {
            roots: fallback_roots(),
            ..ManifestDefaults::default()
        };
    };
    let span = defaults_value.span;
    let Some(table) = take_table(&mut defaults_value) else {
        diags.error(Some(span.into()), "`[defaults]` must be a table");
        return ManifestDefaults {
            roots: fallback_roots(),
            ..ManifestDefaults::default()
        };
    };

    let mut th = TableHelper::from((table, span));
    let root: Option<String> = th.optional("root");
    let roots_field: Option<Vec<String>> = th.optional("roots");
    let provider: Option<String> = th.optional("provider");
    let size: Option<u16> = th.optional("size");
    if let Err(err) = th.finalize(None) {
        diags.push_deser_error(err);
    }

    let mut roots = Vec::new();
    if let Some(value) = root {
        roots.push(resolve_workspace_path(workspace_root, &value));
    }
    if let Some(values) = roots_field {
        roots.extend(values.iter().map(|value| resolve_workspace_path(workspace_root, value)));
    }
    if roots.is_empty() {
        roots = fallback_roots();
    }

    ManifestDefaults {
        roots,
        provider,
        size,
    }
}

/// A single `[providers.<name>]` (or `[providers.<name>.override]`) entry,
/// before it's checked against the builtin set - that check needs
/// `resolve_providers`, which knows about builtins; this only knows what
/// shape the TOML was in.
pub(crate) enum ProviderDeclaration {
    Full { schema: ProviderSchema, span: toml_span::Span },
    Override { variants: Option<Vec<String>>, sizes: Option<Vec<u16>>, span: toml_span::Span },
}

pub(crate) fn parse_providers(
    providers_value: Option<Value<'_>>,
    diags: &mut Diagnostics,
) -> HashMap<String, ProviderDeclaration> {
    let Some(mut providers_value) = providers_value else {
        return HashMap::new();
    };
    let span = providers_value.span;
    let Some(table) = take_table(&mut providers_value) else {
        diags.error(Some(span.into()), "`[providers]` must be a table");
        return HashMap::new();
    };

    let mut providers = HashMap::new();
    for (name_key, mut value) in table {
        let name = name_key.name.to_string();
        let entry_span = value.span;
        let Some(mut entry_table) = take_table(&mut value) else {
            diags.error(
                Some(entry_span.into()),
                format!("`providers.{name}` must be a table"),
            );
            continue;
        };

        if let Some(mut override_value) = entry_table.remove("override") {
            if !entry_table.is_empty() {
                diags.error(
                    Some(entry_span.into()),
                    format!(
                        "`providers.{name}` can't mix its own fields with `providers.{name}.override` - use one or the other"
                    ),
                );
                continue;
            }
            let override_span = override_value.span;
            let Some(override_table) = take_table(&mut override_value) else {
                diags.error(Some(override_span.into()), format!("`providers.{name}.override` must be a table"));
                continue;
            };

            let mut th = TableHelper::from((override_table, override_span));
            let variants: Option<Vec<String>> = th.optional("variants");
            let sizes: Option<Vec<u16>> = th.optional("sizes");
            if let Err(err) = th.finalize(None) {
                diags.push_deser_error(err);
                continue;
            }

            providers.insert(name, ProviderDeclaration::Override { variants, sizes, span: entry_span });
            continue;
        }

        let mut th = TableHelper::from((entry_table, entry_span));
        let variants: Option<Vec<String>> = th.optional("variants");
        let sizes: Option<Vec<u16>> = th.optional("sizes");
        if let Err(err) = th.finalize(None) {
            diags.push_deser_error(err);
            continue;
        }

        providers.insert(
            name,
            ProviderDeclaration::Full {
                schema: ProviderSchema {
                    variants: variants.unwrap_or_default(),
                    sizes: sizes.unwrap_or_default(),
                },
                span: entry_span,
            },
        );
    }
    providers
}

/// The provider schemas guicons ships with, parsed from
/// `builtin_providers.gui.toml` through this same module rather than
/// hardcoded as Rust literals - so it's one less thing to keep in sync by
/// hand, and a manifest author can see exactly what it looks like.
fn builtin_providers() -> &'static HashMap<String, ProviderSchema> {
    static BUILTIN: std::sync::OnceLock<HashMap<String, ProviderSchema>> = std::sync::OnceLock::new();
    BUILTIN.get_or_init(|| {
        const SOURCE: &str = include_str!("builtin_providers.gui.toml");
        let root = toml_span::parse(SOURCE).expect("builtin_providers.gui.toml is valid TOML");
        let mut errors = Vec::new();
        let mut diags = Diagnostics {
            file: Path::new("<builtin_providers.gui.toml>"),
            errors: &mut errors,
        };
        let declarations = parse_providers(Some(root), &mut diags);
        assert!(errors.is_empty(), "builtin_providers.gui.toml failed to parse: {errors:?}");

        declarations
            .into_iter()
            .map(|(name, declaration)| match declaration {
                ProviderDeclaration::Full { schema, .. } => (name, schema),
                ProviderDeclaration::Override { .. } => {
                    unreachable!("builtin_providers.gui.toml can't `.override` itself")
                }
            })
            .collect()
    })
}

/// Names of the providers with a built-in schema (`fluent`, `ph`, etc.) -
/// for editor tooling offering completion on `[providers.<name>]`.
pub fn builtin_provider_names() -> impl Iterator<Item = &'static str> {
    builtin_providers().keys().map(String::as_str)
}

/// Resolves one manifest's `[providers.*]` declarations against the builtin
/// set: a bare `[providers.X]` redefining a builtin name is rejected (use
/// `.override` instead, so it's clear which one was meant); `.override`
/// fields replace the builtin's per-field, inheriting whatever it didn't
/// specify; builtins the file didn't mention at all pass through unchanged.
pub(crate) fn resolve_providers(
    declarations: HashMap<String, ProviderDeclaration>,
    diags: &mut Diagnostics,
) -> HashMap<String, ProviderSchema> {
    let builtin = builtin_providers();
    let mut resolved = HashMap::new();

    for (name, declaration) in declarations {
        match declaration {
            ProviderDeclaration::Full { schema, span } => {
                if builtin.contains_key(&name) {
                    diags.error(
                        Some(span.into()),
                        format!(
                            "`providers.{name}` is a built-in provider; use `[providers.{name}.override]` to customize it instead of redefining it"
                        ),
                    );
                    continue;
                }
                resolved.insert(name, schema);
            }
            ProviderDeclaration::Override { variants, sizes, span } => match builtin.get(&name) {
                Some(base) => {
                    resolved.insert(
                        name,
                        ProviderSchema {
                            variants: variants.unwrap_or_else(|| base.variants.clone()),
                            sizes: sizes.unwrap_or_else(|| base.sizes.clone()),
                        },
                    );
                }
                None => {
                    diags.error(
                        Some(span.into()),
                        format!("`providers.{name}.override` used, but `{name}` isn't a built-in provider"),
                    );
                }
            },
        }
    }

    for (name, schema) in builtin {
        resolved.entry(name.clone()).or_insert_with(|| schema.clone());
    }

    resolved
}

/// Reverses `default_iconify_id`: given a raw iconify id and a provider
/// schema declared in the manifest, splits it back into `family`, `size`,
/// and `variant`. Returns `None` if the provider has no `[providers.X]`
/// entry - callers decide what to fall back to (e.g. ask for `--name`).
///
/// Icon set naming conventions vary too much to assume fixed positions:
/// Fluent always has both `-size-variant`, Phosphor has no size and its
/// bare name is unsuffixed, Material Symbols stacks multiple suffix tokens.
/// So suffixes are matched by membership in `schema.variants`/`schema.sizes`,
/// not by position, and the variant suffix match is greedy-longest so a
/// multi-segment variant like `outline-rounded` isn't torn apart.
pub fn decompose_iconify_id(
    id: &str,
    manifest: &IconManifest,
) -> Option<(String, Option<u16>, Option<String>)> {
    let (provider, name) = id.split_once(':')?;
    let schema = manifest.provider(provider)?;

    let segments: Vec<&str> = name.split('-').collect();

    let mut variant: Option<String> = None;
    let mut remaining = segments.len();
    for suffix_len in (1..=segments.len()).rev() {
        let candidate = segments[segments.len() - suffix_len..].join("-");
        if schema.variants.iter().any(|known| *known == candidate) {
            variant = Some(candidate);
            remaining = segments.len() - suffix_len;
            break;
        }
    }

    let mut family_segments = segments[..remaining].to_vec();

    let mut size: Option<u16> = None;
    if let Some(last) = family_segments.last() {
        if let Ok(parsed) = last.parse::<u16>() {
            if schema.sizes.contains(&parsed) {
                size = Some(parsed);
                family_segments.pop();
            }
        }
    }

    if family_segments.is_empty() {
        return None;
    }

    Some((family_segments.join("-"), size, variant))
}

fn default_iconify_id(
    family: &str,
    variant: Option<&str>,
    size: Option<u16>,
    defaults: &ManifestDefaults,
) -> Option<String> {
    let provider = defaults.provider.as_ref()?;
    let mut name = family.to_string();
    if let Some(size) = size {
        name.push('-');
        name.push_str(&size.to_string());
    }
    if let Some(variant) = variant {
        name.push('-');
        name.push_str(variant);
    }
    Some(format!("{provider}:{name}"))
}

pub(crate) fn resolve_file_from_roots(roots: &[PathBuf], value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    roots
        .iter()
        .map(|root| root.join(path))
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| roots.first().unwrap_or(&PathBuf::from(".")).join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Diagnostics;

    fn workspace_root() -> PathBuf {
        PathBuf::from("/workspace")
    }

    fn with_root() -> ManifestDefaults {
        ManifestDefaults {
            roots: vec![workspace_root()],
            ..ManifestDefaults::default()
        }
    }

    fn entries_for(toml: &str, defaults: ManifestDefaults) -> (Vec<IconEntry>, Vec<String>) {
        let mut root = toml_span::parse(toml).expect("valid toml");
        let table = take_table(&mut root).expect("root must be a table");
        let mut errors = Vec::new();
        let mut entries = Vec::new();
        {
            let mut diags = Diagnostics {
                file: Path::new("test.gui.toml"),
                errors: &mut errors,
            };
            collect_entries(Vec::new(), table, &workspace_root(), &defaults, &mut diags, &mut entries);
        }
        (entries, errors.into_iter().map(|e| e.message).collect())
    }

    fn defaults_for(toml: &str) -> (ManifestDefaults, Vec<String>) {
        let mut root = toml_span::parse(toml).expect("valid toml");
        let mut table = take_table(&mut root).expect("root must be a table");
        let defaults_value = table.remove("defaults");
        let mut errors = Vec::new();
        let defaults = {
            let mut diags = Diagnostics {
                file: Path::new("test.gui.toml"),
                errors: &mut errors,
            };
            parse_defaults(defaults_value, &workspace_root(), &workspace_root(), &mut diags)
        };
        (defaults, errors.into_iter().map(|e| e.message).collect())
    }

    #[test]
    fn simple_file_entry() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            file = "settings.svg"
            "#,
            with_root(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn variants_produce_two_entries_sharing_a_family() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            variants.filled = { file = "settings-filled.svg" }
            variants.regular = { file = "settings-regular.svg" }
            "#,
            with_root(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn nested_groups_build_a_dashed_key() {
        let (entries, errors) = entries_for(
            r#"
            [icons.navigation.back]
            file = "back.svg"
            "#,
            with_root(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn unsupported_field_is_rejected() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            file = "settings.svg"
            bogus = "oops"
            "#,
            with_root(),
        );
        assert!(entries.is_empty());
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn two_sources_is_an_error() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            file = "settings.svg"
            url = "https://example.com/a.svg"
            "#,
            with_root(),
        );
        assert!(entries.is_empty());
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn no_source_is_an_error() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            dynamic = true
            "#,
            with_root(),
        );
        assert!(entries.is_empty());
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn wrong_field_type_is_rejected() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            file = "settings.svg"
            dynamic = "yes"
            "#,
            with_root(),
        );
        assert!(entries.is_empty());
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn variants_must_be_a_table() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            variants = "nope"
            "#,
            with_root(),
        );
        assert!(entries.is_empty());
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn variant_entry_must_be_an_inline_table() {
        let (entries, errors) = entries_for(
            r#"
            [settings]
            variants.filled = "settings-filled.svg"
            "#,
            with_root(),
        );
        assert!(entries.is_empty());
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn default_provider_fills_in_iconify_id() {
        let defaults = ManifestDefaults {
            provider: Some("fluent".to_string()),
            size: Some(24),
            ..ManifestDefaults::default()
        };
        let (entries, errors) = entries_for(
            r#"
            [settings]
            variants.filled = {}
            "#,
            defaults,
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn explicit_iconify_overrides_default_provider() {
        let defaults = ManifestDefaults {
            provider: Some("fluent".to_string()),
            ..ManifestDefaults::default()
        };
        let (entries, errors) = entries_for(
            r#"
            [settings]
            iconify = "phosphor:gear"
            "#,
            defaults,
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn defaults_merge_root_and_roots() {
        let (defaults, errors) = defaults_for(
            r#"
            [defaults]
            root = "a"
            roots = ["b", "c"]
            "#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(defaults);
    }

    #[test]
    fn defaults_fall_back_to_manifest_and_workspace_dirs_when_empty() {
        let (defaults, errors) = defaults_for("");
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(defaults);
    }

    #[test]
    fn defaults_reject_unsupported_field() {
        let (_, errors) = defaults_for(
            r#"
            [defaults]
            bogus = 1
            "#,
        );
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn numeric_path_segment_becomes_size_not_family() {
        let (entries, errors) = entries_for(
            r#"
            [settings.20]
            variants.filled = { file = "settings-20-filled.svg" }

            [settings.24]
            variants.filled = { file = "settings-24-filled.svg" }
            "#,
            with_root(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn numeric_path_segment_becomes_size_for_flat_entry() {
        let (entries, errors) = entries_for(
            r#"
            [settings.20]
            file = "settings-20.svg"
            "#,
            with_root(),
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn explicit_size_feeds_default_iconify_id() {
        let defaults = ManifestDefaults {
            provider: Some("fluent".to_string()),
            ..ManifestDefaults::default()
        };
        let (entries, errors) = entries_for(
            r#"
            [settings.20]
            variants.filled = {}
            "#,
            defaults,
        );
        assert!(errors.is_empty(), "{errors:?}");
        insta::assert_debug_snapshot!(entries);
    }

    #[test]
    fn defaults_reject_wrong_field_type() {
        let (_, errors) = defaults_for(
            r#"
            [defaults]
            size = "big"
            "#,
        );
        insta::assert_debug_snapshot!(errors);
    }

    fn providers_for(toml: &str) -> (HashMap<String, ProviderSchema>, Vec<String>) {
        let mut root = toml_span::parse(toml).expect("valid toml");
        let mut table = take_table(&mut root).expect("root must be a table");
        let providers_value = table.remove("providers");
        let mut errors = Vec::new();
        let providers = {
            let mut diags = Diagnostics {
                file: Path::new("test.gui.toml"),
                errors: &mut errors,
            };
            let declarations = parse_providers(providers_value, &mut diags);
            resolve_providers(declarations, &mut diags)
        };
        (providers, errors.into_iter().map(|e| e.message).collect())
    }

    fn schema(variants: &[&str], sizes: &[u16]) -> ProviderSchema {
        ProviderSchema {
            variants: variants.iter().map(|v| v.to_string()).collect(),
            sizes: sizes.to_vec(),
        }
    }

    fn manifest_with_providers(providers: HashMap<String, ProviderSchema>) -> IconManifest {
        IconManifest {
            manifest_path: PathBuf::from("icons.gui.toml"),
            workspace_root: PathBuf::from("/workspace"),
            source_paths: Vec::new(),
            entries: Vec::new(),
            providers,
        }
    }

    #[test]
    fn parses_a_custom_provider_schema() {
        let (providers, errors) = providers_for(
            r#"
            [providers.acme]
            variants = ["outline", "solid"]
            sizes = [16, 24]
            "#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let acme = providers.get("acme").expect("acme provider");
        assert_eq!(acme.variants, vec!["outline", "solid"]);
        assert_eq!(acme.sizes, vec![16, 24]);
    }

    #[test]
    fn provider_without_variants_or_sizes_defaults_to_empty() {
        let (providers, errors) = providers_for(
            r#"
            [providers.acme]
            "#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let acme = providers.get("acme").expect("acme provider");
        assert!(acme.variants.is_empty());
        assert!(acme.sizes.is_empty());
    }

    #[test]
    fn providers_entry_must_be_a_table() {
        let (_, errors) = providers_for(
            r#"
            providers = "nope"
            "#,
        );
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn provider_schema_rejects_wrong_field_type() {
        let (_, errors) = providers_for(
            r#"
            [providers.acme]
            sizes = "big"
            "#,
        );
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn builtin_providers_are_available_even_when_unmentioned() {
        let (providers, errors) = providers_for("");
        assert!(errors.is_empty(), "{errors:?}");
        let fluent = providers.get("fluent").expect("builtin fluent provider");
        assert_eq!(fluent.variants, vec!["regular", "filled"]);
        assert_eq!(fluent.sizes, vec![16, 20, 24, 28, 32, 48]);
        for name in ["ph", "material-symbols", "heroicons", "bi", "tabler"] {
            assert!(providers.contains_key(name), "missing builtin `{name}`");
        }
    }

    #[test]
    fn redefining_a_builtin_provider_without_override_is_an_error() {
        let (providers, errors) = providers_for(
            r#"
            [providers.fluent]
            variants = ["custom"]
            "#,
        );
        assert_eq!(
            providers.get("fluent").expect("builtin fluent provider").variants,
            vec!["regular", "filled"],
            "rejected redefinition must not replace the builtin"
        );
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn override_replaces_only_the_fields_it_specifies() {
        let (providers, errors) = providers_for(
            r#"
            [providers.fluent.override]
            variants = ["regular", "filled", "light"]
            "#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let fluent = providers.get("fluent").expect("fluent provider");
        assert_eq!(fluent.variants, vec!["regular", "filled", "light"]);
        assert_eq!(fluent.sizes, vec![16, 20, 24, 28, 32, 48]);
    }

    #[test]
    fn override_on_a_non_builtin_name_is_an_error() {
        let (_, errors) = providers_for(
            r#"
            [providers.acme.override]
            variants = ["solid"]
            "#,
        );
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn mixing_own_fields_with_override_is_an_error() {
        let (_, errors) = providers_for(
            r#"
            [providers.fluent]
            variants = ["custom"]
            override = { variants = ["light"] }
            "#,
        );
        insta::assert_debug_snapshot!(errors);
    }

    #[test]
    fn decompose_fluent_style_size_and_variant() {
        let mut providers = HashMap::new();
        providers.insert("fluent".to_string(), schema(&["regular", "filled"], &[16, 20, 24, 28, 32, 48]));
        let manifest = manifest_with_providers(providers);
        insta::assert_debug_snapshot!(decompose_iconify_id(
            "fluent:add-square-multiple-24-regular",
            &manifest
        ));
    }

    #[test]
    fn decompose_phosphor_style_bare_default_has_no_suffix() {
        let mut providers = HashMap::new();
        providers.insert("ph".to_string(), schema(&["thin", "light", "bold", "fill", "duotone"], &[]));
        let manifest = manifest_with_providers(providers);
        insta::assert_debug_snapshot!(decompose_iconify_id("ph:acorn", &manifest));
    }

    #[test]
    fn decompose_phosphor_style_with_variant_suffix() {
        let mut providers = HashMap::new();
        providers.insert("ph".to_string(), schema(&["thin", "light", "bold", "fill", "duotone"], &[]));
        let manifest = manifest_with_providers(providers);
        insta::assert_debug_snapshot!(decompose_iconify_id("ph:acorn-bold", &manifest));
    }

    #[test]
    fn decompose_material_symbols_style_compound_variant_matches_greedily() {
        let mut providers = HashMap::new();
        providers.insert(
            "material-symbols".to_string(),
            schema(&["outline", "rounded", "sharp", "outline-rounded", "outline-sharp"], &[]),
        );
        let manifest = manifest_with_providers(providers);
        insta::assert_debug_snapshot!(decompose_iconify_id(
            "material-symbols:3d-rotation-outline-rounded",
            &manifest
        ));
    }

    #[test]
    fn decompose_bootstrap_style_leading_digit_name_is_not_mistaken_for_size() {
        let mut providers = HashMap::new();
        providers.insert("bi".to_string(), schema(&["fill"], &[]));
        let manifest = manifest_with_providers(providers);
        insta::assert_debug_snapshot!(decompose_iconify_id("bi:0-circle", &manifest));
    }

    #[test]
    fn decompose_unknown_provider_returns_none() {
        let manifest = manifest_with_providers(HashMap::new());
        assert_eq!(decompose_iconify_id("unknown:whatever", &manifest), None);
    }

    #[test]
    fn decompose_id_without_colon_returns_none() {
        let manifest = manifest_with_providers(HashMap::new());
        assert_eq!(decompose_iconify_id("no-colon-here", &manifest), None);
    }

    mod decompose_proptests {
        use super::*;
        use proptest::prelude::*;

        fn schema_strategy() -> impl Strategy<Value = ProviderSchema> {
            (
                prop::collection::vec("[a-z]{2,6}", 0..4),
                prop::collection::vec(1u16..100, 0..4),
            )
                .prop_map(|(variants, sizes)| ProviderSchema { variants, sizes })
        }

        proptest! {
            /// Building an id the same way `default_iconify_id` does, then
            /// decomposing it with the same schema, must recover the
            /// original family/size/variant - as long as the family itself
            /// doesn't happen to look like one of the schema's own suffixes
            /// (an inherent, expected ambiguity of suffix-stripping, not
            /// something to paper over here).
            #[test]
            fn round_trips_through_build_and_decompose(
                family_segments in prop::collection::vec("[a-z]{2,8}", 1..3),
                schema in schema_strategy(),
                use_variant in any::<bool>(),
                use_size in any::<bool>(),
            ) {
                prop_assume!(family_segments.iter().all(|segment| !schema.variants.contains(segment)));

                let family = family_segments.join("-");
                let variant = if use_variant { schema.variants.first().cloned() } else { None };
                let size = if use_size { schema.sizes.first().copied() } else { None };

                let mut name = family.clone();
                if let Some(size) = size {
                    name.push('-');
                    name.push_str(&size.to_string());
                }
                if let Some(variant) = &variant {
                    name.push('-');
                    name.push_str(variant);
                }
                let id = format!("provider:{name}");

                let mut providers = HashMap::new();
                providers.insert("provider".to_string(), schema);
                let manifest = manifest_with_providers(providers);

                prop_assert_eq!(decompose_iconify_id(&id, &manifest), Some((family, size, variant)));
            }

            /// No input string, however malformed, should panic - the point
            /// of a suffix-membership-based parser is that "no match" is
            /// just `None`/unchanged, never a crash.
            #[test]
            fn never_panics_on_arbitrary_input(id in ".*", schema in schema_strategy()) {
                let mut providers = HashMap::new();
                providers.insert("provider".to_string(), schema);
                let manifest = manifest_with_providers(providers);
                let _ = decompose_iconify_id(&id, &manifest);
            }
        }
    }
}
