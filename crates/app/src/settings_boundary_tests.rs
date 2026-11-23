use syn::visit::{self, Visit};

const LOGICAL_DIMENSIONS: &[&str] =
    &["font_size", "line_height", "status_bar_height", "gutter_padding", "toc_width"];

fn is_test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute.parse_args::<syn::Ident>().is_ok_and(|condition| condition == "test")
    })
}

#[derive(Default)]
struct LogicalDimensionAccesses(Vec<String>);

impl<'ast> Visit<'ast> for LogicalDimensionAccesses {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if !is_test_only(&module.attrs) {
            visit::visit_item_mod(self, module);
        }
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if !is_test_only(&function.attrs) {
            visit::visit_item_fn(self, function);
        }
    }

    fn visit_item_impl(&mut self, implementation: &'ast syn::ItemImpl) {
        if !is_test_only(&implementation.attrs) {
            visit::visit_item_impl(self, implementation);
        }
    }

    fn visit_impl_item_fn(&mut self, method: &'ast syn::ImplItemFn) {
        if !is_test_only(&method.attrs) {
            visit::visit_impl_item_fn(self, method);
        }
    }

    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if let syn::Member::Named(dimension) = &field.member
            && LOGICAL_DIMENSIONS.iter().any(|name| dimension == name)
            && let syn::Expr::Field(settings) = field.base.as_ref()
            && matches!(&settings.member, syn::Member::Named(name) if name == "settings")
            && matches!(settings.base.as_ref(), syn::Expr::Path(path) if path.path.is_ident("self"))
        {
            self.0.push(dimension.to_string());
        }
        visit::visit_expr_field(self, field);
    }
}

fn production_logical_dimension_accesses(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("settings boundary must inspect valid Rust source");
    let mut accesses = LogicalDimensionAccesses::default();
    accesses.visit_file(&file);
    accesses.0
}

#[test]
fn settings_check_finds_production_access_after_test_items() {
    let source = r#"
        #[cfg(test)]
        use crate::test_fixture;

        #[cfg(test)]
        mod tests {
            impl App { fn fixture(&self) { let _ = self.settings.line_height; } }
        }

        impl App {
            #[cfg(test)]
            fn fixture(&self) { let _ = self.settings.status_bar_height; }

            fn render(&self) {
                let _ = self . settings . font_size;
                let _ = "self.settings.gutter_padding";
            }
        }
    "#;

    assert_eq!(production_logical_dimension_accesses(source), ["font_size"]);
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
        let accesses = production_logical_dimension_accesses(source);
        assert!(accesses.is_empty(), "{path}: reads logical dimensions directly: {accesses:?}");
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
