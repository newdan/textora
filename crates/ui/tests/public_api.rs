//! 正向公共 API 测试：验证语义根级模块路径可用。
//!
//! 从"外部 crate"视角（integration test）导入，确保 re-export 正确。

use ui::button::{Button, ButtonStyle};
use ui::checkbox::Checkbox;
use ui::core::{Event, Rect, Widget};
use ui::gutter::RenderContext;
use ui::label::{Label, LabelStyle};
use ui::list::{ListItem, ListStyle, ListWidget, Orientation};
use ui::popup_menu::{PopupMenu, PopupMenuAction, PopupMenuItem, PopupMenuWidget};
use ui::render_geom::AdvanceCacheEntry;
use ui::scrollbar::{ScrollbarAction, ScrollbarInput, ScrollbarWidget};
use ui::settings::{Settings, UiMetrics};
use ui::settings_view::SettingsView;
use ui::sidebar::{SidebarAction, SidebarSettingsInput, SidebarWidget, SidebarWidgetInput};
use ui::splitter::{SplitterInput, SplitterWidget};
use ui::switch::Switch;
use ui::tab_bar::{TabBarAction, TabBarWidget, TabBarWidgetInput, TabInfo};
use ui::text_box::TextBox;
use ui::theme::{ThemeLoadError, ThemeRegistry, ThemeSource};
use ui::tooltip::{TooltipHint, TooltipWidget};
use ui::viewport::{LineMap, ScrollAnchor};
use ui::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityOrientation, AccessibilityRole, AccessibilityState,
    AccessibilityTree, AccessibilityValidationError, ChildEventRouter, ControlAction,
    FocusDirection, Modifiers, WidgetAction, next_focus_target,
};

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
fn child_event_router_is_a_stable_public_container_capability() {
    let first = ui::WidgetId(301);
    let second = ui::WidgetId(302);
    let mut router = ChildEventRouter::default();
    router.set_focused_target(Some(first));

    assert_eq!(
        next_focus_target(router.focused_target(), &[first, second], FocusDirection::Forward),
        Some(second)
    );
    assert_eq!(router.cycle_focus(&[first, second], FocusDirection::Forward), Some(second));
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

#[test]
fn toggle_controls_expose_synchronized_state_api() {
    let mut checkbox = Checkbox::new(ui::WidgetId(101), false);
    assert!(!checkbox.checked());
    assert!(checkbox.is_enabled());
    checkbox.set_checked(true);
    checkbox.set_enabled(false);
    assert!(checkbox.checked());
    assert!(!checkbox.is_enabled());

    let mut switch = Switch::new(ui::WidgetId(102), true);
    assert!(switch.checked());
    assert!(switch.is_enabled());
    switch.set_checked(false);
    switch.set_enabled(false);
    assert!(!switch.checked());
    assert!(!switch.is_enabled());
}

#[test]
fn splitter_focus_is_an_explicit_public_capability() {
    let id = ui::WidgetId(103);
    let pointer_only = SplitterWidget::new();
    assert_eq!(pointer_only.id(), None);

    let focusable = SplitterWidget::with_id(id);
    assert_eq!(focusable.id(), Some(id));
    assert!(focusable.is_focusable());
}

#[test]
fn list_focus_is_an_explicit_public_capability() {
    let style = ListStyle {
        row_h_logical: 24.0,
        item_w_logical: 120.0,
        pad_x_logical: 8.0,
        pad_y_logical: 4.0,
        font_size_logical: 13.0,
        bg: [0.0; 4],
        item_active_bg: [0.0; 4],
        item_hover_bg: [0.0; 4],
        item_fg: [0.0; 4],
        item_active_fg: [0.0; 4],
        item_hover_fg: [0.0; 4],
        item_accent: [0.0; 4],
        separator: [0.0; 4],
        indicator_color: [0.0; 4],
    };
    let id = ui::WidgetId(104);
    let list = ListWidget::new(style, Orientation::Vertical).with_id(id);

    assert_eq!(list.id(), Some(id));
    assert!(list.is_focusable());
    assert_eq!(list.focused_index(), None);
}

#[test]
fn foundational_controls_and_inputs_construct_from_semantic_public_paths() {
    let theme = ui::Theme::resolve_builtin(ui::ThemeMode::Light, winit::window::Theme::Light);
    let button = Button::new(ui::WidgetId(201), ButtonStyle::from_theme(&theme));
    let label = Label::new("Status", LabelStyle::default());
    let text_box = TextBox::with_id(ui::WidgetId(202));
    let checkbox = Checkbox::new(ui::WidgetId(203), false);
    let switch = Switch::new(ui::WidgetId(204), true);
    let list = ListWidget::new(ListStyle::from_theme(&theme), Orientation::Vertical)
        .with_id(ui::WidgetId(205));
    let splitter = SplitterWidget::with_id(ui::WidgetId(206));
    let scrollbar = ScrollbarWidget::new();
    let popup = PopupMenuWidget::new(PopupMenu {
        items: vec![PopupMenuItem::action("Open", PopupMenuAction::OpenSettingsFile)],
        item_rects: vec![Rect::new(0.0, 0.0, 120.0, 32.0)],
        menu_rect: Rect::new(0.0, 0.0, 120.0, 32.0),
        screen_size: (800.0, 600.0),
        show_checkmarks: false,
    });
    let (tooltip, _) = TooltipWidget::new(
        &TooltipHint {
            label: "Description".into(),
            target_rect: Rect::new(10.0, 10.0, 20.0, 20.0),
        },
        1.0,
        800.0,
        600.0,
    );

    assert_widget::<Button>();
    assert_widget::<Label>();
    assert_widget::<TextBox>();
    assert_widget::<Checkbox>();
    assert_widget::<Switch>();
    assert_widget::<ListWidget>();
    assert_widget::<SplitterWidget>();
    assert_widget::<ScrollbarWidget>();
    assert_widget::<PopupMenuWidget>();
    assert_widget::<TooltipWidget>();
    let _controls =
        (button, label, text_box, checkbox, switch, list, splitter, scrollbar, popup, tooltip);
    let _inputs = (ListItem::default(), SplitterInput::default(), ScrollbarInput::default());
}

#[test]
fn interaction_and_accessibility_protocols_have_stable_root_paths() {
    let id = AccessibilityId(301);
    let request = AccessibilityActionRequest::new(id, AccessibilityAction::Focus);
    let context = AccessibilityContext::new(4.0, 8.0);
    let node = AccessibilityNode::new(
        id,
        AccessibilityRole::Button,
        context.screen_bounds(Rect::new(0.0, 0.0, 20.0, 20.0)),
    );

    assert_eq!(request.target, id);
    assert_eq!(node.role, AccessibilityRole::Button);
    assert_public_type::<AccessibilityOrientation>();
    assert_public_type::<AccessibilityState>();
    assert_public_type::<AccessibilityTree>();
    assert_public_type::<AccessibilityValidationError>();
    assert_public_type::<ControlAction>();
    assert_public_type::<Modifiers>();
    assert_public_type::<WidgetAction>();
}
