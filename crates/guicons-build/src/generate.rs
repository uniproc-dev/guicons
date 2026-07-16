use super::materialize::{ImageKind, MaterializedIcon, MaterializedIconBackend};
use guicons_core::{rust_const_name, rust_fn_name};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub(crate) fn generate_rust_icon_registry_from_materialized(
    manifest_path: &Path,
    out_file: &Path,
    icons: &[MaterializedIcon],
) {
    let key_consts = icons
        .iter()
        .map(|icon| {
            format!(
                "    pub const {}: IconKey = IconKey::new(\"{}\");",
                rust_const_name(&icon.key),
                icon.key
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let all_keys = icons
        .iter()
        .map(|icon| format!("keys::{}", rust_const_name(&icon.key)))
        .collect::<Vec<_>>()
        .join(", ");
    let key_from_name_arms = icons
        .iter()
        .map(|icon| format!("        \"{}\" => Some(keys::{}),", icon.key, rust_const_name(&icon.key)))
        .collect::<Vec<_>>()
        .join("\n");
    let name_for_key_arms = icons
        .iter()
        .map(|icon| format!("        keys::{} => Some(\"{}\"),", rust_const_name(&icon.key), icon.key))
        .collect::<Vec<_>>()
        .join("\n");

    let families = unique_strings(icons.iter().map(|icon| icon.family.as_str()));
    let variants = unique_strings(icons.iter().filter_map(|icon| icon.variant.as_deref()));
    let family_consts = families
        .iter()
        .map(|family| format!("    pub const {}: IconFamily = IconFamily::new(\"{}\");", rust_const_name(family), family))
        .collect::<Vec<_>>()
        .join("\n");
    let variant_consts = variants
        .iter()
        .map(|variant| format!("    pub const {}: IconVariant = IconVariant::new(\"{}\");", rust_const_name(variant), variant))
        .collect::<Vec<_>>()
        .join("\n");
    // Keyed on (family, size, variant) together, not just (family, variant):
    // a family can repeat the same variant at more than one size (see
    // `generate_builders`), and matching on family+variant alone would
    // produce unreachable, silently-wrong duplicate match arms for those.
    let family_variant_arms = icons
        .iter()
        .map(|icon| {
            let size_pattern = rust_option_pattern(icon.size.map(|size| size.to_string()));
            let variant_pattern =
                rust_option_pattern(icon.variant.as_ref().map(|variant| format!("variants::{}", rust_const_name(variant))));
            format!(
                "        (families::{}, {size_pattern}, {variant_pattern}) => Some(keys::{}),",
                rust_const_name(&icon.family),
                rust_const_name(&icon.key)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dynamic_family_variant_arms = icons
        .iter()
        .map(|icon| {
            let size_pattern = rust_option_pattern(icon.size.map(|size| size.to_string()));
            let variant_pattern = rust_option_pattern(icon.variant.as_ref().map(|variant| format!("\"{variant}\"")));
            format!(
                "        (\"{}\", {size_pattern}, {variant_pattern}) => Some(keys::{}),",
                icon.family,
                rust_const_name(&icon.key)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let data_arms = icons
        .iter()
        .map(|icon| match &icon.backend {
            MaterializedIconBackend::Image { path, kind } if !icon.dynamic => {
                let path = path.to_string_lossy().replace('\\', "\\\\");
                let ctor = match kind {
                    ImageKind::Svg => "Svg",
                    ImageKind::Png => "Png",
                };
                format!(
                    "        keys::{} => Some(IconData::{ctor}(include_bytes!(\"{path}\"))),",
                    rust_const_name(&icon.key)
                )
            }
            MaterializedIconBackend::Glyph { font_family, codepoint } => {
                format!(
                    "        keys::{} => Some(IconData::Glyph {{ codepoint: {}, font_family: \"{}\" }}),",
                    rust_const_name(&icon.key),
                    char_literal(*codepoint),
                    escape_rust_string(font_family)
                )
            }
            _ => format!("        keys::{} => None,", rust_const_name(&icon.key)),
        })
        .collect::<Vec<_>>()
        .join("\n");

    let builders = generate_builders(icons);

    let generated = format!(
        r#"// AUTO-GENERATED from {}
use guicons::{{IconData, IconFamily, IconKey, IconRef, IconVariant}};

pub mod keys {{
    use super::IconKey;

{key_consts}
}}

pub mod families {{
    use super::IconFamily;

{family_consts}
}}

pub mod variants {{
    use super::IconVariant;

{variant_consts}
}}

{builders}
pub const ALL_KEYS: &[IconKey] = &[{all_keys}];

pub fn name_for_key(key: IconKey) -> Option<&'static str> {{
    match key {{
{name_for_key_arms}
        _ => None,
    }}
}}

pub fn key_from_name(name: &str) -> Option<IconKey> {{
    match name {{
{key_from_name_arms}
        _ => None,
    }}
}}

pub fn key_from_family_variant(family: IconFamily, size: Option<u16>, variant: Option<IconVariant>) -> Option<IconKey> {{
    match (family, size, variant) {{
{family_variant_arms}
        _ => None,
    }}
}}

pub fn key_from_dynamic_family_variant(family: &str, size: Option<u16>, variant: Option<&str>) -> Option<IconKey> {{
    match (family, size, variant) {{
{dynamic_family_variant_arms}
        _ => None,
    }}
}}

pub fn key_from_ref(icon: IconRef<'_>) -> Option<IconKey> {{
    match icon {{
        IconRef::Key(key) => Some(key),
        IconRef::Name(name) => key_from_name(name),
        IconRef::FamilyVariant {{ family, size, variant }} => key_from_family_variant(family, size, variant),
        IconRef::DynamicFamilyVariant {{ family, size, variant }} => key_from_dynamic_family_variant(family, size, variant),
    }}
}}

pub fn data_for(key: IconKey) -> Option<IconData> {{
    match key {{
{data_arms}
        _ => None,
    }}
}}

pub fn resolve<'a>(icon: impl Into<IconRef<'a>>) -> Option<IconData> {{
    key_from_ref(icon.into()).and_then(data_for)
}}
"#,
        manifest_file_name(manifest_path)
    );

    write_if_changed(out_file, &generated);
}

fn generate_builders(icons: &[MaterializedIcon]) -> String {
    let mut families: BTreeMap<&str, Vec<&MaterializedIcon>> = BTreeMap::new();
    for icon in icons {
        families.entry(icon.family.as_str()).or_default().push(icon);
    }

    families
        .into_iter()
        .map(|(family, members)| generate_family_builder(family, &members))
        .collect::<Vec<_>>()
        .join("\n")
}

fn generate_family_builder(family: &str, members: &[&MaterializedIcon]) -> String {
    let mut unsized_icons: Vec<&MaterializedIcon> = Vec::new();
    let mut sized: BTreeMap<u16, Vec<&MaterializedIcon>> = BTreeMap::new();
    for icon in members {
        match icon.size {
            Some(size) => sized.entry(size).or_default().push(*icon),
            None => unsized_icons.push(icon),
        }
    }

    let fn_name = rust_fn_name(family);
    let type_name = rust_variant_name(family);

    if sized.is_empty() {
        return generate_leaf(&fn_name, &format!("{type_name}Builder"), &unsized_icons);
    }

    let builder_type = format!("{type_name}Builder");
    let mut methods = generate_group_methods(&unsized_icons);
    let mut nested_types = String::new();

    for (size, group) in &sized {
        if group.len() == 1 && group[0].variant.is_none() {
            methods.push_str(&format!(
                "    pub const fn size_{size}(self) -> IconKey {{\n        keys::{}\n    }}\n",
                rust_const_name(&group[0].key)
            ));
            continue;
        }

        let size_type = format!("{type_name}{size}VariantBuilder");
        let variant_methods = generate_group_methods(group);
        nested_types.push_str(&format!(
            "pub struct {size_type};\n\nimpl {size_type} {{\n{variant_methods}}}\n\n"
        ));
        methods.push_str(&format!(
            "    pub const fn size_{size}(self) -> {size_type} {{\n        {size_type}\n    }}\n"
        ));
    }

    format!(
        "{nested_types}pub struct {builder_type};\n\npub const fn {fn_name}() -> {builder_type} {{\n    {builder_type}\n}}\n\nimpl {builder_type} {{\n{methods}}}\n"
    )
}

fn generate_leaf(fn_name: &str, builder_type: &str, icons: &[&MaterializedIcon]) -> String {
    if let [icon] = icons {
        if icon.variant.is_none() {
            return format!(
                "pub const fn {fn_name}() -> IconKey {{\n    keys::{}\n}}\n",
                rust_const_name(&icon.key)
            );
        }
    }

    let methods = generate_group_methods(icons);
    format!(
        "pub struct {builder_type};\n\npub const fn {fn_name}() -> {builder_type} {{\n    {builder_type}\n}}\n\nimpl {builder_type} {{\n{methods}}}\n"
    )
}

fn generate_group_methods(icons: &[&MaterializedIcon]) -> String {
    icons
        .iter()
        .map(|icon| {
            let method_name = match icon.variant.as_deref() {
                Some(variant) => rust_fn_name(variant),
                None => "default".to_string(),
            };
            format!(
                "    pub const fn {method_name}(self) -> IconKey {{\n        keys::{}\n    }}\n",
                rust_const_name(&icon.key)
            )
        })
        .collect()
}

pub(crate) fn generate_slint_icon_global_from_materialized(
    out_file: &Path,
    icons: &[MaterializedIcon],
) {
    let components = icons
        .iter()
        .filter_map(|icon| match &icon.backend {
            MaterializedIconBackend::Image { path, .. } => {
                let component_name = slint_component_name(&icon.key);
                let path = relative_or_absolute_icon_path(out_file, path);
                let path = escape_slint_string(&path.replace('\\', "/"));
                Some(format!(
                    "export component {component_name} inherits Image {{\n    source: @image-url(\"{path}\");\n}}\n"
                ))
            }
            MaterializedIconBackend::Glyph { font_family, codepoint } => {
                let component_name = slint_component_name(&icon.key);
                Some(format!(
                    "export component {component_name} inherits Text {{\n    text: \"{}\";\n    font-family: \"{}\";\n    horizontal-alignment: center;\n    vertical-alignment: center;\n    font-size: Math.min(self.width, self.height);\n    wrap: no-wrap;\n}}\n",
                    escape_slint_string(&codepoint.to_string()),
                    escape_slint_string(font_family)
                ))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let cases = icons
        .iter()
        .map(|icon| {
            let component_name = slint_component_name(&icon.key);
            match &icon.backend {
                MaterializedIconBackend::Image { .. } => format!(
                    "    if (root.name == \"{}\") : {component_name} {{\n        width: parent.width;\n        height: parent.height;\n        image-fit: contain;\n        colorize: root.colorize;\n    }}",
                    icon.key
                ),
                MaterializedIconBackend::Glyph { .. } => format!(
                    "    if (root.name == \"{}\") : {component_name} {{\n        width: parent.width;\n        height: parent.height;\n        color: root.colorize;\n    }}",
                    icon.key
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let generated = format!(
        "// AUTO-GENERATED - do not edit manually\n{components}\nexport component Icon inherits Rectangle {{\n    in property <string> name;\n    in property <color> colorize: transparent;\n\n{cases}\n}}\n"
    );
    write_if_changed(out_file, &generated);
}

fn manifest_file_name(manifest_path: &Path) -> String {
    manifest_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| manifest_path.display().to_string())
}

fn write_if_changed(path: &Path, content: &str) {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing != content {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(path, content)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", path.display()));
    }
}

fn slint_component_name(key: &str) -> String {
    format!("{}Icon", rust_variant_name(key))
}

fn rust_variant_name(key: &str) -> String {
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

fn rust_option_pattern(inner: Option<String>) -> String {
    match inner {
        Some(inner) => format!("Some({inner})"),
        None => "None".to_string(),
    }
}

fn unique_strings<'a>(values: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut values = values.collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn char_literal(ch: char) -> String {
    format!("{ch:?}")
}

fn escape_rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn escape_slint_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn relative_or_absolute_icon_path(base_file: &Path, target: &Path) -> String {
    if let Some(base_dir) = base_file.parent() {
        if let Some(relative) = pathdiff::diff_paths(target, base_dir) {
            return relative.to_string_lossy().to_string();
        }
    }
    target.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(key: &str, family: &str, variant: Option<&str>, size: Option<u16>) -> MaterializedIcon {
        MaterializedIcon {
            key: key.to_string(),
            family: family.to_string(),
            variant: variant.map(str::to_string),
            size,
            dynamic: false,
            backend: MaterializedIconBackend::Image {
                path: Path::new("unused.svg").to_path_buf(),
                kind: ImageKind::Svg,
            },
        }
    }

    #[test]
    fn family_without_variant_or_size_is_a_bare_fn() {
        let icons = vec![icon("docker", "docker", None, None)];
        insta::assert_snapshot!(generate_builders(&icons));
    }

    #[test]
    fn family_with_variants_only_is_a_two_tier_builder() {
        let icons = vec![
            icon("settings-filled", "settings", Some("filled"), None),
            icon("settings-regular", "settings", Some("regular"), None),
        ];
        insta::assert_snapshot!(generate_builders(&icons));
    }

    #[test]
    fn family_with_a_shared_variant_set_across_sizes() {
        let icons = vec![
            icon("settings-20-filled", "settings", Some("filled"), Some(20)),
            icon("settings-20-regular", "settings", Some("regular"), Some(20)),
            icon("settings-24-filled", "settings", Some("filled"), Some(24)),
            icon("settings-24-regular", "settings", Some("regular"), Some(24)),
        ];
        insta::assert_snapshot!(generate_builders(&icons));
    }

    #[test]
    fn family_with_a_different_variant_set_per_size_gets_distinct_types() {
        let icons = vec![
            icon("settings-16-filled", "settings", Some("filled"), Some(16)),
            icon("settings-24-filled", "settings", Some("filled"), Some(24)),
            icon("settings-24-regular", "settings", Some("regular"), Some(24)),
        ];
        insta::assert_snapshot!(generate_builders(&icons));
    }

    #[test]
    fn family_with_a_single_icon_per_size_skips_the_variant_builder() {
        let icons = vec![
            icon("logo-16", "logo", None, Some(16)),
            icon("logo-32", "logo", None, Some(32)),
        ];
        insta::assert_snapshot!(generate_builders(&icons));
    }

    #[test]
    fn sizeless_icon_sharing_a_family_with_sized_ones_gets_a_default_method() {
        let icons = vec![
            icon("settings", "settings", None, None),
            icon("settings-24-filled", "settings", Some("filled"), Some(24)),
        ];
        insta::assert_snapshot!(generate_builders(&icons));
    }
}
