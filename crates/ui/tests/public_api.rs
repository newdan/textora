//! 正向公共 API 测试：验证语义根级模块路径可用。
//!
//! 从"外部 crate"视角（integration test）导入，确保 re-export 正确。

use ui::core::{Event, Rect, Widget};
use ui::gutter::RenderContext;
use ui::render_geom::AdvanceCacheEntry;
use ui::scrollbar::{ScrollbarAction, ScrollbarInput, ScrollbarWidget};
use ui::settings::{Settings, UiMetrics};
use ui::settings_view::SettingsView;
use ui::sidebar::{SidebarAction, SidebarSettingsInput, SidebarWidget, SidebarWidgetInput};
use ui::tab_bar::{TabBarAction, TabBarWidget, TabBarWidgetInput, TabInfo};
use ui::theme::{ThemeLoadError, ThemeRegistry, ThemeSource};
use ui::viewport::{LineMap, ScrollAnchor};

fn assert_widget<T: Widget>() {}
fn assert_debug<T: std::fmt::Debug>() {}
fn assert_public_type<T>() {}

/// Fixture implementing `LineMap` for trait-bound compile check.
struct PublicLineMapFixture;

impl LineMap for PublicLineMapFixture {
    fn map_line_count(&self) -> usize {
        0
    }
    fn map_total_rows(&self) -> usize {
        0
    }
    fn map_display_to_doc(&self, _display_row: usize) -> usize {
        0
    }
    fn map_doc_to_display(&self, _doc_line: usize) -> usize {
        0
    }
    fn visual_line_count(&self, _doc_line: usize) -> u16 {
        1
    }
}

fn assert_line_map<T: LineMap>() {}

#[test]
fn semantic_public_modules_compile_for_external_consumers() {
    // Widget trait bound
    assert_widget::<TabBarWidget>();
    assert_widget::<SidebarWidget>();
    assert_widget::<ScrollbarWidget>();
    assert_widget::<SettingsView>();

    // Debug trait bound
    assert_debug::<ThemeLoadError>();

    // core re-exports
    let _event: Option<Event> = None;
    let _rect = Rect::ZERO;

    // settings 构造
    let settings = Settings::new();
    let metrics = UiMetrics::from_settings(&settings, 2.0);
    let behavior = SidebarSettingsInput::from(&settings);

    // tab_bar 输入构造
    let tab = TabInfo {
        title: "tab".into(),
        file_path: None,
        is_dirty: false,
        pinned: true,
        language: String::new(),
    };
    let _tab_input = TabBarWidgetInput {
        tabs: vec![tab.clone()],
        active_index: Some(0),
        back_enabled: false,
        forward_enabled: false,
        screen_size_px: (800.0, 600.0),
        hovered_index: None,
        scroll_offset_px: 0.0,
        metrics,
    };

    // sidebar 输入构造
    let _sidebar_input = SidebarWidgetInput {
        tabs: vec![tab],
        active_index: Some(0),
        traffic_light_inset_px: (68.0, 0.0),
        screen_size_px: (800.0, 600.0),
        metrics: UiMetrics::from_settings(&settings, 2.0),
        settings: behavior,
    };

    // scrollbar 输入构造
    let _scrollbar_input =
        ScrollbarInput { viewport_height_px: 40.0, total_display_rows: 100, scroll_top_rows: 10.0 };

    // theme 构造
    let _registry = ThemeRegistry::new();
    let _source = ThemeSource {
        id: "sample".into(),
        path: "sample.toml".into(),
        content: "is_dark = true".into(),
    };

    // action enum 路径验证
    let _actions: (Option<TabBarAction>, Option<SidebarAction>, Option<ScrollbarAction>) =
        (None, None, None);
}

#[test]
fn domain_module_types_are_publicly_accessible() {
    // gutter::RenderContext
    assert_public_type::<RenderContext<'static>>();

    // render_geom::AdvanceCacheEntry
    assert_public_type::<AdvanceCacheEntry>();

    // viewport::ScrollAnchor
    assert_public_type::<ScrollAnchor>();

    // viewport::LineMap (trait bound check)
    let _ = assert_line_map::<PublicLineMapFixture>;
}
