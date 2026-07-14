guicons::include_icons!();

#[cfg(test)]
mod tests {
    use super::icons;

    #[test]
    fn generated_registry_exposes_keys_families_and_variants() {
        assert_eq!(icons::ALL_KEYS.len(), 3);

        let key = icons::key_from_dynamic_family_variant("settings", Some("filled")).unwrap();
        assert_eq!(icons::name_for_key(key), Some("settings-filled"));
        assert_eq!(key, icons::keys::SETTINGS_FILLED);
        assert_eq!(guicons::icon!("settings/filled"), icons::keys::SETTINGS_FILLED);
        assert_eq!(guicons::icon!(settings.regular), icons::keys::SETTINGS_REGULAR);

        assert_eq!(
            icons::key_from_family_variant(
                icons::families::SETTINGS,
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
}
