use super::load_icon_manifest;
use super::materialize::{
    materialize_icons, ImageKind, MaterializedIcon, MaterializedIconBackend,
};
use guicons_core::rust_const_name;
use std::fs;
use std::path::Path;

pub fn generate_rust_icon_registry(manifest_path: &Path, out_file: &Path, build_out_dir: &Path) {
    let manifest = load_icon_manifest(manifest_path);
    let icons = materialize_icons(&manifest, build_out_dir);
    generate_rust_icon_registry_from_materialized(manifest_path, out_file, &icons);
}

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
    let family_variant_arms = icons
        .iter()
        .filter(|icon| icon.variant.is_some())
        .map(|icon| {
            let variant = icon.variant.as_ref().unwrap();
            format!(
                "        (families::{}, Some(variants::{})) => Some(keys::{}),",
                rust_const_name(&icon.family),
                rust_const_name(variant),
                rust_const_name(&icon.key)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dynamic_family_variant_arms = icons
        .iter()
        .filter(|icon| icon.variant.is_some())
        .map(|icon| {
            let variant = icon.variant.as_ref().unwrap();
            format!(
                "        (\"{}\", Some(\"{}\")) => Some(keys::{}),",
                icon.family,
                variant,
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

pub fn key_from_family_variant(family: IconFamily, variant: Option<IconVariant>) -> Option<IconKey> {{
    match (family, variant) {{
{family_variant_arms}
        _ => None,
    }}
}}

pub fn key_from_dynamic_family_variant(family: &str, variant: Option<&str>) -> Option<IconKey> {{
    match (family, variant) {{
{dynamic_family_variant_arms}
        _ => None,
    }}
}}

pub fn key_from_ref(icon: IconRef<'_>) -> Option<IconKey> {{
    match icon {{
        IconRef::Key(key) => Some(key),
        IconRef::Name(name) => key_from_name(name),
        IconRef::FamilyVariant {{ family, variant }} => key_from_family_variant(family, variant),
        IconRef::DynamicFamilyVariant {{ family, variant }} => key_from_dynamic_family_variant(family, variant),
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
        manifest_path.display()
    );

    write_if_changed(out_file, &generated);
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
