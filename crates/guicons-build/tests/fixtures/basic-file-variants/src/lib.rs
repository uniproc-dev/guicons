guicons::include_icons!();

#[cfg(test)]
mod tests {
    use super::icons;

    #[test]
    fn generated_registry_exposes_keys_families_and_variants() {
        assert_eq!(icons::ALL_KEYS.len(), 9);

        let key = icons::key_from_dynamic_family_variant("settings", None, Some("filled")).unwrap();
        assert_eq!(icons::name_for_key(key), Some("settings-filled"));
        assert_eq!(key, icons::keys::SETTINGS_FILLED);
        assert_eq!(guicons::icon!("settings/filled"), icons::keys::SETTINGS_FILLED);
        assert_eq!(guicons::icon!(settings.regular), icons::keys::SETTINGS_REGULAR);

        assert_eq!(
            icons::key_from_family_variant(
                icons::families::SETTINGS,
                None,
                Some(icons::variants::REGULAR)
            ),
            Some(icons::keys::SETTINGS_REGULAR)
        );

        assert_eq!(icons::key_from_name("docker"), Some(icons::keys::DOCKER));
    }

    #[test]
    fn generated_registry_embeds_file_assets() {
        match icons::data_for(icons::keys::SETTINGS_FILLED).unwrap() {
            guicons::IconData::Svg(bytes) => assert!(bytes.starts_with(b"<svg")),
            other => panic!("expected svg icon data, got {other:?}"),
        }
    }

    #[test]
    fn icon_macro_resolves_a_bare_iconify_literal_from_cache() {
        match guicons::icon!("testset:gear") {
            guicons::IconData::Svg(bytes) => {
                assert!(bytes.starts_with(b"<svg"));
                assert!(String::from_utf8_lossy(bytes).contains("circle"));
            }
            other => panic!("expected svg icon data, got {other:?}"),
        }
    }

    /// `[toolbar.16]`/`[toolbar.24]` have *different* variant sets (16 is
    /// filled-only, 24 has filled+regular) - the generated `size_16()`/
    /// `size_24()` steps must return distinct builder types, each only
    /// exposing the variants that actually exist at that size.
    #[test]
    fn size_axis_produces_per_size_variant_builders() {
        assert_eq!(icons::toolbar().size_16().filled(), icons::keys::TOOLBAR_16_FILLED);
        assert_eq!(icons::toolbar().size_24().filled(), icons::keys::TOOLBAR_24_FILLED);
        assert_eq!(icons::toolbar().size_24().regular(), icons::keys::TOOLBAR_24_REGULAR);
    }

    /// `toolbar` repeats the `filled` variant at two different sizes -
    /// `key_from_family_variant`/`key_from_dynamic_family_variant`/`icon!`
    /// must disambiguate by size instead of returning whichever one
    /// happens to come first (that used to be a literal unreachable match
    /// arm in the generated code).
    #[test]
    fn family_variant_lookup_is_disambiguated_by_size() {
        assert_eq!(
            icons::key_from_family_variant(icons::families::TOOLBAR, Some(16), Some(icons::variants::FILLED)),
            Some(icons::keys::TOOLBAR_16_FILLED)
        );
        assert_eq!(
            icons::key_from_family_variant(icons::families::TOOLBAR, Some(24), Some(icons::variants::FILLED)),
            Some(icons::keys::TOOLBAR_24_FILLED)
        );
        assert_eq!(
            icons::key_from_dynamic_family_variant("toolbar", Some(16), Some("filled")),
            Some(icons::keys::TOOLBAR_16_FILLED)
        );
        assert_eq!(
            icons::key_from_dynamic_family_variant("toolbar", Some(24), Some("filled")),
            Some(icons::keys::TOOLBAR_24_FILLED)
        );
        assert_eq!(guicons::icon!(toolbar.16.filled), icons::keys::TOOLBAR_16_FILLED);
        assert_eq!(guicons::icon!(toolbar.24.filled), icons::keys::TOOLBAR_24_FILLED);
        assert_eq!(guicons::icon!("toolbar/16/filled"), icons::keys::TOOLBAR_16_FILLED);
        assert_eq!(guicons::icon!("toolbar/24/regular"), icons::keys::TOOLBAR_24_REGULAR);
    }

    /// `[logo.16]`/`[logo.32]` have no variants - `size_N()` should return
    /// an `IconKey` directly, skipping the variant-builder step entirely.
    #[test]
    fn size_axis_without_variants_skips_the_variant_builder() {
        assert_eq!(icons::logo().size_16(), icons::keys::LOGO_16);
        assert_eq!(icons::logo().size_32(), icons::keys::LOGO_32);
        assert_eq!(guicons::icon!(logo.16), icons::keys::LOGO_16);
        assert_eq!(guicons::icon!(logo.32), icons::keys::LOGO_32);
    }

    #[test]
    fn glyph_source_is_embedded_as_glyph_data() {
        assert_eq!(icons::spinner(), icons::keys::SPINNER);
        match icons::data_for(icons::keys::SPINNER).unwrap() {
            guicons::IconData::Glyph { codepoint, font_family } => {
                assert_eq!(codepoint, '\u{E001}');
                assert_eq!(font_family, "FixtureIconFont");
            }
            other => panic!("expected glyph icon data, got {other:?}"),
        }
    }
}
