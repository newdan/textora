fn production_part(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

#[test]
fn physical_consumers_do_not_read_logical_dimensions_directly() {
    let consumers = [
        ("app_init.rs", include_str!("app_init.rs")),
        ("app_window.rs", include_str!("app_window.rs")),
        ("app_lifecycle.rs", include_str!("app_lifecycle.rs")),
        ("app_renderer.rs", include_str!("app_renderer.rs")),
        ("app_reshape.rs", include_str!("app_reshape.rs")),
        ("app_scroll.rs", include_str!("app_scroll.rs")),
        ("app_search.rs", include_str!("app_search.rs")),
        ("app_dispatch.rs", include_str!("app_dispatch.rs")),
        ("render_pipeline.rs", include_str!("../../appkit-shell/src/render_pipeline.rs")),
        ("mouse.rs", include_str!("mouse.rs")),
        ("dispatch/editor.rs", include_str!("dispatch/editor.rs")),
        ("dispatch/mouse.rs", include_str!("dispatch/mouse.rs")),
    ];
    for (path, source) in consumers {
        let production = production_part(source);
        for field in
            ["font_size", "line_height", "status_bar_height", "gutter_padding", "toc_width"]
        {
            let forbidden = format!("self.settings.{field}");
            assert!(
                !production.contains(&forbidden),
                "{path}: found {forbidden} in production code"
            );
        }
    }
}

#[test]
fn mutable_dpi_compatibility_api_is_gone() {
    let app_source = include_str!("app.rs");
    let ui_settings_source = include_str!("../../ui/src/settings.rs");
    for forbidden in [
        "pub dpi_scale:",
        "fn apply_scale(",
        "fn logical_font_size(",
        "fn logical_line_height(",
        "from_physical_settings",
        "impl From<&Settings> for UiMetrics",
    ] {
        assert!(!app_source.contains(forbidden), "app.rs still contains: {forbidden}");
        assert!(
            !ui_settings_source.contains(forbidden),
            "ui/settings.rs still contains: {forbidden}"
        );
    }
}

#[test]
fn settings_view_fields_roundtrip_through_toml() {
    let mut settings = ui::settings::Settings::new();
    settings.set_theme_mode(ui::settings::ThemeMode::Dark);
    settings.set_view_mode(ui::view_mode::ViewMode::Tabs);
    settings.set_font_family("Iosevka".into());
    settings.set_font_size(18.0);
    settings.set_line_height_ratio(1.5);
    settings.set_tab_width(8);
    settings.set_word_wrap(false);
    settings.set_show_line_numbers(false);
    settings.set_show_status_bar(true);

    let directory = tempfile::tempdir().expect("temporary settings directory must be created");
    let path = directory.path().join("settings.toml");
    let mut persisted = crate::settings_io::PersistedSettings::default();
    persisted.apply_editor_settings(&settings);
    crate::settings_io::save(&path, &persisted).expect("settings fixture must be serializable");
    let loaded = crate::settings_io::load(&path).expect("settings fixture must load");

    assert_eq!(loaded.theme_mode, settings.theme_mode);
    assert_eq!(loaded.view_mode, settings.view_mode);
    assert_eq!(loaded.font_family, settings.font_family);
    assert_eq!(loaded.font_size, settings.font_size);
    assert_eq!(loaded.line_height_ratio, settings.line_height_ratio);
    assert_eq!(loaded.tab_width, settings.tab_width);
    assert_eq!(loaded.word_wrap, settings.word_wrap);
    assert_eq!(loaded.show_line_numbers, settings.show_line_numbers);
    assert_eq!(loaded.show_status_bar, settings.show_status_bar);
}
