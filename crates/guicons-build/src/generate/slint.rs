use super::shared::{rust_variant_name, write_if_changed};
use crate::materialize::{MaterializedIcon, MaterializedIconBackend};
use std::path::Path;

pub(crate) fn generate_slint_icon_global_from_materialized(out_file: &Path, icons: &[MaterializedIcon]) {
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

fn slint_component_name(key: &str) -> String {
    format!("{}Icon", rust_variant_name(key))
}

fn escape_slint_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

fn relative_or_absolute_icon_path(base_file: &Path, target: &Path) -> String {
    if let Some(base_dir) = base_file.parent() {
        if let Some(relative) = pathdiff::diff_paths(target, base_dir) {
            return relative.to_string_lossy().to_string();
        }
    }
    target.to_string_lossy().to_string()
}
