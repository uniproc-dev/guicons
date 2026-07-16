mod rust;
mod shared;
mod slint;

pub(crate) use rust::generate_rust_icon_registry_from_materialized;
pub(crate) use slint::generate_slint_icon_global_from_materialized;

#[cfg(test)]
mod tests {
    use crate::materialize::{ImageKind, MaterializedIcon, MaterializedIconBackend};
    use std::path::Path;

    fn icon(key: &str, family: &str, variant: Option<&str>, size: Option<u16>) -> MaterializedIcon {
        MaterializedIcon {
            key: key.to_string(),
            family: family.to_string(),
            variant: variant.map(str::to_string),
            size,
            dynamic: false,
            backend: MaterializedIconBackend::Image { path: Path::new("unused.svg").to_path_buf(), kind: ImageKind::Svg },
        }
    }

    #[test]
    fn family_without_variant_or_size_is_a_bare_fn() {
        let icons = vec![icon("docker", "docker", None, None)];
        insta::assert_snapshot!(super::rust::generate_builders(&icons));
    }

    #[test]
    fn family_with_variants_only_is_a_two_tier_builder() {
        let icons = vec![
            icon("settings-filled", "settings", Some("filled"), None),
            icon("settings-regular", "settings", Some("regular"), None),
        ];
        insta::assert_snapshot!(super::rust::generate_builders(&icons));
    }

    #[test]
    fn family_with_a_shared_variant_set_across_sizes() {
        let icons = vec![
            icon("settings-20-filled", "settings", Some("filled"), Some(20)),
            icon("settings-20-regular", "settings", Some("regular"), Some(20)),
            icon("settings-24-filled", "settings", Some("filled"), Some(24)),
            icon("settings-24-regular", "settings", Some("regular"), Some(24)),
        ];
        insta::assert_snapshot!(super::rust::generate_builders(&icons));
    }

    #[test]
    fn family_with_a_different_variant_set_per_size_gets_distinct_types() {
        let icons = vec![
            icon("settings-16-filled", "settings", Some("filled"), Some(16)),
            icon("settings-24-filled", "settings", Some("filled"), Some(24)),
            icon("settings-24-regular", "settings", Some("regular"), Some(24)),
        ];
        insta::assert_snapshot!(super::rust::generate_builders(&icons));
    }

    #[test]
    fn family_with_a_single_icon_per_size_skips_the_variant_builder() {
        let icons = vec![icon("logo-16", "logo", None, Some(16)), icon("logo-32", "logo", None, Some(32))];
        insta::assert_snapshot!(super::rust::generate_builders(&icons));
    }

    #[test]
    fn sizeless_icon_sharing_a_family_with_sized_ones_gets_a_default_method() {
        let icons =
            vec![icon("settings", "settings", None, None), icon("settings-24-filled", "settings", Some("filled"), Some(24))];
        insta::assert_snapshot!(super::rust::generate_builders(&icons));
    }
}
