guicons::include_icons!();

slint::include_modules!();

#[cfg(test)]
mod tests {
    use super::icons;

    #[test]
    fn generated_registry_exposes_keys_families_and_variants() {
        assert_eq!(icons::ALL_KEYS.len(), 9);

        let key = icons::key_from_dynamic_family_variant("settings", None, Some("filled")).unwrap();
        assert_eq!(icons::name_for_key(key), Some("settings-filled"));
        assert_eq!(key, icons::keys::SETTINGS_FILLED);
        assert_eq!(guicons::icon_key!("settings/filled"), icons::keys::SETTINGS_FILLED);
        assert_eq!(guicons::icon_key!(settings.regular), icons::keys::SETTINGS_REGULAR);

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
    fn icon_data_macro_resolves_a_bare_iconify_literal_from_cache() {
        match guicons::icon_data!("testset:gear") {
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
        assert_eq!(guicons::icon_key!(toolbar.16.filled), icons::keys::TOOLBAR_16_FILLED);
        assert_eq!(guicons::icon_key!(toolbar.24.filled), icons::keys::TOOLBAR_24_FILLED);
        assert_eq!(guicons::icon_key!("toolbar/16/filled"), icons::keys::TOOLBAR_16_FILLED);
        assert_eq!(guicons::icon_key!("toolbar/24/regular"), icons::keys::TOOLBAR_24_REGULAR);
    }

    /// `[logo.16]`/`[logo.32]` have no variants - `size_N()` should return
    /// an `IconKey` directly, skipping the variant-builder step entirely.
    #[test]
    fn size_axis_without_variants_skips_the_variant_builder() {
        assert_eq!(icons::logo().size_16(), icons::keys::LOGO_16);
        assert_eq!(icons::logo().size_32(), icons::keys::LOGO_32);
        assert_eq!(guicons::icon_key!(logo.16), icons::keys::LOGO_16);
        assert_eq!(guicons::icon_key!(logo.32), icons::keys::LOGO_32);
    }

    #[test]
    fn icon_data_macro_resolves_a_manifest_selector_directly_to_data() {
        match guicons::icon_data!(settings.filled) {
            guicons::IconData::Svg(bytes) => assert!(bytes.starts_with(b"<svg")),
            other => panic!("expected svg icon data, got {other:?}"),
        }
    }

    /// This fixture has the `slint` feature active, so `icon!` (unlike
    /// `icon_data!` above) auto-targets `slint::Image` directly - no
    /// `image_from_data` wrapping needed at the use site.
    #[test]
    fn icon_macro_auto_targets_native_slint_image_when_slint_feature_is_active() {
        let image: slint::Image = guicons::icon!(settings.filled);
        assert!(image.size().width > 0);
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

    /// Covers both Slint integration paths in one test - winit only allows
    /// one event loop per process, so instantiating a top-level component
    /// in more than one test (each running on its own thread) crashes with
    /// "EventLoop can't be recreated"; both instantiations must share a
    /// thread.
    ///
    /// 1. Constructing `super::AppWindow` at all proves the codegen'd
    ///    `icons.slint` (a separate file in `OUT_DIR`, never fed through the
    ///    Slint compiler by `guicons-build` itself) is valid Slint markup -
    ///    `AppWindow` only exists because `ui/main.slint` successfully
    ///    `import`ed it at build time.
    /// 2. An inline `slint::slint!` component exercises
    ///    `guicons::slint::{image_from_data, glyph_from_data}`, the "no
    ///    manifest, no codegen" runtime path.
    #[test]
    fn slint_integration_covers_generated_file_import_and_runtime_data_conversion() {
        let _window = super::AppWindow::new().unwrap();

        slint::slint! {
            export component IconProbe inherits Rectangle {
                in property <image> icon-source;
                in property <string> glyph-text;
                in property <string> glyph-font-family;
            }
        }

        let probe = IconProbe::new().unwrap();

        let svg_data = super::icons::data_for(super::icons::keys::SETTINGS_FILLED).unwrap();
        let image = guicons::slint::image_from_data(svg_data).expect("svg data should decode");
        probe.set_icon_source(image.clone());
        assert_eq!(probe.get_icon_source().size(), image.size());

        let glyph_data = super::icons::data_for(super::icons::keys::SPINNER).unwrap();
        let (font_family, codepoint) = guicons::slint::glyph_from_data(glyph_data).expect("glyph data");
        probe.set_glyph_text(codepoint.to_string().into());
        probe.set_glyph_font_family(font_family.into());
        assert_eq!(probe.get_glyph_text(), "\u{E001}");
        assert_eq!(probe.get_glyph_font_family(), "FixtureIconFont");
    }
}
