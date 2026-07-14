//! Turns one already-parsed `toml_span` document into `IconEntry` values.
//!
//! Nothing here touches the filesystem or knows about `[include]` — that's
//! [`crate::load`]'s job. This module only walks the table tree produced by
//! `toml_span::parse` and validates/extracts entries from it.

use crate::diagnostics::Diagnostics;
use crate::model::{IconEntry, IconEntrySource, ManifestDefaults};
use crate::paths::resolve_workspace_path;
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

pub(crate) fn check_reserved_top_level(table: &Table<'_>, diags: &mut Diagnostics) {
    if let Some(value) = table.get("providers") {
        if value.as_table().is_none() {
            diags.error(Some(value.span.into()), "`[providers]` must be a table");
        }
    }
}

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
}
