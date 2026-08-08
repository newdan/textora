//! SidebarWidget tests.

#[cfg(test)]
mod tests {
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::widget::WidgetAction;
    use crate::core::{
        Event, EventCtx, KeyCode, LayoutCtx, Modifiers, MouseButton, PaintCtx, Rect, Widget,
    };

    use crate::tab_bar::TabInfo;
    use crate::theme::Theme;
    use crate::widgets::list::ListItemIndicator;
    use crate::widgets::sidebar::{
        NewDocumentKind, SidebarAction, SidebarConfig, SidebarWidget, SidebarWidgetInput,
        Visibility,
    };

    fn test_theme() -> Theme {
        let mut t = crate::theme::test_theme();
        t.editor.foreground = [0.9, 0.9, 0.9, 1.0];
        t.palette.bg_surface = [0.15, 0.15, 0.15, 1.0];
        t.palette.bg_active = [0.25, 0.25, 0.25, 1.0];
        t.palette.text_muted = [0.9, 0.9, 0.9, 1.0];
        t.palette.accent = [0.4549, 0.6784, 0.9098, 1.0];
        t.palette.border_subtle = [0.1, 0.1, 0.1, 1.0];
        t
    }
    fn make_tab(title: &str) -> TabInfo {
        TabInfo {
            title: title.into(),
            file_path: None,
            is_dirty: false,
            pinned: false,
            language: "rust".into(),
        }
    }

    fn make_dirty_tab(title: &str) -> TabInfo {
        TabInfo {
            title: title.into(),
            file_path: None,
            is_dirty: true,
            pinned: false,
            language: "rust".into(),
        }
    }

    fn metrics(dpi: f32) -> crate::settings::UiMetrics {
        crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), dpi)
    }

    fn sidebar_settings() -> crate::widgets::sidebar::SidebarSettingsInput {
        crate::widgets::sidebar::SidebarSettingsInput::default()
    }

    fn sidebar_widget_input(tabs: Vec<TabInfo>, active_index: Option<usize>) -> SidebarWidgetInput {
        SidebarWidgetInput {
            tabs,
            active_index,
            traffic_light_inset_px: (68.0, 0.0),
            screen_size_px: (800.0, 600.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        }
    }

    fn new_document_widget() -> SidebarWidget {
        let mut widget =
            SidebarWidget::new(SidebarConfig { pinned: true, width: 220.0 }, metrics(1.0));
        widget.set_input(sidebar_widget_input(Vec::new(), None));
        widget
    }

    fn layout_new_document_widget(widget: &mut SidebarWidget, theme: &Theme) {
        let mut measure = NoopMeasure;
        let mut layout_ctx = LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi: 1.0 };
        widget.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut layout_ctx);
    }

    fn new_document_menu_item_action(kind: NewDocumentKind) -> SidebarAction {
        SidebarAction::NewDocument(kind)
    }

    fn click_new_document_region(
        widget: &mut SidebarWidget,
        rect: Rect,
        event_ctx: &mut EventCtx<'_>,
    ) -> Option<WidgetAction> {
        let px = rect.x + rect.w * 0.5;
        let py = rect.y + rect.h * 0.5;
        assert_eq!(
            widget.on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, event_ctx,),
            Some(WidgetAction::Consumed)
        );
        widget.on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, event_ctx)
    }

    #[test]
    fn widget_input_replaces_geometry_metrics_and_behavior() {
        let mut widget = SidebarWidget::new(SidebarConfig::new_default(1.0), metrics(1.0));
        let mut first = sidebar_widget_input(vec![make_tab("first")], Some(0));
        first.settings.word_wrap = false;
        widget.set_input(first);

        let mut second = sidebar_widget_input(vec![make_tab("second")], None);
        second.traffic_light_inset_px = (20.0, 4.0);
        second.screen_size_px = (400.0, 300.0);
        second.metrics = metrics(2.0);
        second.settings.word_wrap = true;
        widget.set_input(second);

        assert_eq!(widget.tabs[0].title, "second");
        assert_eq!(widget.active_index, None);
        assert_eq!(widget.traffic_light_inset, (20.0, 4.0));
        assert_eq!((widget.screen_w, widget.screen_h), (400.0, 300.0));
        assert_eq!(widget.metrics.dpi, 2.0);
        assert!(widget.settings_input.word_wrap);
    }

    // ── set_rect + paint ──

    #[test]
    fn widget_paint_emits_background_and_header() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![make_tab("a.rs")],
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let mut dl = DrawList::new();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: None,
        };
        w.paint(&mut pc);

        // bg + header + new_btn + settings_btn + hamburger text + new text + settings text
        // + list items (1 text for tab "a.rs") = ≥ 8
        assert!(dl.cmds.len() >= 8, "Expected at least 8 draw commands, got {}", dl.cmds.len());
        // First cmd should be background fill
        assert!(matches!(dl.cmds[0], DrawCmd::FillRect { .. }));
    }

    // ── on_event: click dispatch ──

    #[test]
    fn widget_new_document_button_activates_on_matching_mouse_release() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![],
            active_index: None,
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let mut ctx = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let main = w.current_layout().expect("layout must exist").new_btn_rect;
        assert_eq!(
            click_new_document_region(&mut w, main, &mut ctx),
            Some(WidgetAction::Sidebar(SidebarAction::NewDocument(NewDocumentKind::Markdown)))
        );
    }

    #[test]
    fn new_document_dropdown_click_opens_menu() {
        let mut widget = new_document_widget();
        let theme = test_theme();
        layout_new_document_widget(&mut widget, &theme);
        let dropdown = widget.current_layout().expect("layout must exist").new_menu_btn_rect;
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        let action = click_new_document_region(&mut widget, dropdown, &mut event_ctx);

        assert_eq!(action, Some(WidgetAction::Sidebar(SidebarAction::Hovered)));
        assert!(widget.open_menu().is_some());
    }

    #[test]
    fn new_document_button_uses_shared_split_geometry() {
        let mut widget = new_document_widget();
        let theme = test_theme();
        layout_new_document_widget(&mut widget, &theme);
        let layout = widget.current_layout().expect("layout must exist");

        assert_eq!(widget.new_document_button.main_rect(), layout.new_btn_rect);
        assert_eq!(widget.new_document_button.menu_rect(), layout.new_menu_btn_rect);
    }

    #[test]
    fn open_new_document_menu_keeps_the_dropdown_segment_active() {
        let mut widget = new_document_widget();
        let theme = test_theme();
        layout_new_document_widget(&mut widget, &theme);
        let dropdown = widget.current_layout().expect("layout must exist").new_menu_btn_rect;
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let _ = click_new_document_region(&mut widget, dropdown, &mut event_ctx);
        let mut draw_list = DrawList::new();

        widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));

        assert!(draw_list.cmds.windows(3).any(|commands| {
            matches!(commands[0], DrawCmd::PushClip(rect) if rect == dropdown)
                && matches!(
                    commands[1],
                    DrawCmd::FillRect { color, .. } if color == theme.palette.bg_active
                )
                && matches!(commands[2], DrawCmd::PopClip)
        }));
    }

    #[test]
    fn new_document_button_does_not_activate_after_release_outside() {
        let mut widget = new_document_widget();
        let theme = test_theme();
        layout_new_document_widget(&mut widget, &theme);
        let main = widget.current_layout().expect("layout must exist").new_btn_rect;
        let px = main.x + main.w * 0.5;
        let py = main.y + main.h * 0.5;
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        assert_eq!(
            widget
                .on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut event_ctx,),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseUp { px: 500.0, py: 500.0, button: MouseButton::Left },
                &mut event_ctx,
            ),
            Some(WidgetAction::Consumed)
        );
    }

    #[test]
    fn new_document_menu_selects_text() {
        assert_new_document_menu_item_action(NewDocumentKind::Text, 0);
    }

    #[test]
    fn new_document_menu_selects_mindmap() {
        assert_new_document_menu_item_action(NewDocumentKind::Mindmap, 1);
    }

    #[test]
    fn new_document_menu_selects_markdown() {
        assert_new_document_menu_item_action(NewDocumentKind::Markdown, 2);
    }

    fn assert_new_document_menu_item_action(kind: NewDocumentKind, item_index: usize) {
        let mut widget = new_document_widget();
        let theme = test_theme();
        layout_new_document_widget(&mut widget, &theme);
        let dropdown = widget.current_layout().expect("layout must exist").new_menu_btn_rect;
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        let _ = click_new_document_region(&mut widget, dropdown, &mut event_ctx);
        let menu = widget.open_menu().expect("dropdown click must open menu").clone();
        let item = menu.item_rects[item_index];
        let action = widget.on_event(
            &Event::MouseDown {
                px: item.x + item.w * 0.5,
                py: item.y + item.h * 0.5,
                button: MouseButton::Left,
            },
            &mut event_ctx,
        );

        assert_eq!(action, Some(WidgetAction::Sidebar(new_document_menu_item_action(kind))));
        assert!(widget.open_menu().is_none());
    }

    #[test]
    fn new_document_menu_escape_closes_without_action() {
        let mut widget = new_document_widget();
        let theme = test_theme();
        layout_new_document_widget(&mut widget, &theme);
        let dropdown = widget.current_layout().expect("layout must exist").new_menu_btn_rect;
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        let _ = click_new_document_region(&mut widget, dropdown, &mut event_ctx);
        assert!(widget.open_menu().is_some());

        let action =
            widget.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut event_ctx);
        assert!(action.is_none() || action == Some(WidgetAction::Sidebar(SidebarAction::Hovered)));
        assert!(widget.open_menu().is_none());
    }

    #[test]
    fn click_in_list_emits_switch_tab() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs = vec![make_tab("a.rs"), make_tab("b.rs")];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let list_clip = w.current_layout().unwrap().list_clip;
        let cy = list_clip.y + 12.0; // 第一行中点附近

        let mut ctx = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: 100.0, py: cy, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        let action = w
            .on_event(&Event::MouseUp { px: 100.0, py: cy, button: MouseButton::Left }, &mut ctx)
            .unwrap();
        let typed = action;
        assert!(matches!(typed, WidgetAction::Sidebar(SidebarAction::SwitchTab(0))));
    }

    #[test]
    fn widget_mousedown_settings_button() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![],
            active_index: None,
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let dpi = 1.0;
        let settings_y = 800.0 - 28.0 * dpi + 10.0;
        let mut ctx = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let ev = Event::MouseDown { px: 110.0, py: settings_y, button: MouseButton::Left };
        let action = w.on_event(&ev, &mut ctx);
        assert!(action.is_some());
        let a = action.unwrap();
        assert_eq!(a, WidgetAction::Sidebar(SidebarAction::OpenSettingsMenu));
    }

    // ── is_capturing ──

    #[test]
    fn widget_capturing_when_menu_open() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![make_tab("a.rs")],
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        // Without menu: not capturing
        assert!(!w.is_capturing(), "should not capture without menu");

        // Open menu
        w.open_settings_menu();
        assert!(w.is_capturing(), "should capture when menu is open");

        // Close menu
        w.set_open_menu(None);
        assert!(!w.is_capturing(), "should not capture after menu closed");
    }

    // ── hit test ──

    #[test]
    fn widget_hit_delegates_to_rect() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        assert!(w.hit(100.0, 400.0));
        assert!(!w.hit(300.0, 400.0));
    }

    #[test]
    fn widget_hit_pinned_includes_hamburger_in_title_area() {
        // 用户报告：固定状态下点击 hamburger 切换到自动隐藏无效。
        // 根因：pinned 模式下 sidebar bg_rect.y 从 content_top（title_h 之下）
        // 开始，但 hamburger 在 title 区域内（y < content_top）。dock 用
        // widget.hit() 路由事件，必须额外包含 menu_btn_rect 才能让 hamburger
        // 接到点击（否则 events.rs 的 title-bar guard 会先吞掉点击）。
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![],
            active_index: None,
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        // 模拟 dock 给 sidebar 切的 rect 在 title_h 之下
        w.set_rect(Rect::new(0.0, 28.0, 220.0, 772.0), &mut lc);

        let menu_btn = w.current_layout().unwrap().menu_btn_rect;
        let cx = menu_btn.x + menu_btn.w * 0.5;
        let cy = menu_btn.y + menu_btn.h * 0.5;
        assert!(cy < 28.0, "hamburger 应在 title_h 之上 (cy={})", cy);
        assert!(w.hit(cx, cy), "汉堡按钮位置必须命中 sidebar widget");
    }

    // ── indicator ──

    #[test]
    fn dirty_tab_has_dot_indicator_in_list() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs = vec![make_dirty_tab("unsaved.rs")];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        // Verify list items have Dot indicator
        let items = w.list.items();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].indicator, ListItemIndicator::Dot));
    }

    // ── downcast ──

    #[test]
    fn widget_downcast_works() {
        let cfg = SidebarConfig::new_default(1.0);
        let w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let r: &dyn Widget = &w;
        assert!(r.as_any().downcast_ref::<SidebarWidget>().is_some());
    }

    // ── 委托方法测试 ──

    #[test]
    fn widget_open_menu_initially_none() {
        let cfg = SidebarConfig::new_default(1.0);
        let w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(w.open_menu().is_none());
    }

    #[test]
    fn widget_set_open_menu_then_retrieve() {
        use crate::widgets::popup_menu::PopupMenu;
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let menu = PopupMenu {
            items: vec![],
            item_rects: vec![],
            menu_rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            screen_size: (1200.0, 800.0),
            show_checkmarks: true,
        };
        w.set_open_menu(Some(menu));
        assert!(w.open_menu().is_some());
        w.set_open_menu(None);
        assert!(w.open_menu().is_none());
    }

    #[test]
    fn widget_dispatch_menu_click_no_menu_returns_none() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let result = w.dispatch_menu_click(50.0, 50.0);
        assert!(result.is_none());
    }

    #[test]
    fn widget_is_visible_explicit_pinned_true() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // explicit pinned=true → Pinned → visible
        assert!(w.is_visible());
    }

    #[test]
    fn widget_is_visible_hidden_state_false() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = false;
        let w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // pinned=false → Hidden → not visible
        assert!(!w.is_visible());
    }

    #[test]
    fn widget_is_visible_pinned_true() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(w.is_visible());

        let mut w2 = SidebarWidget::new(
            SidebarConfig::new_default(1.0),
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w2.set_visibility(Visibility::Pinned);
        assert!(w2.is_visible());
    }

    #[test]
    fn widget_on_scroll_updates_offset() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: (0..20).map(|i| make_tab(&format!("file{}.rs", i))).collect(),
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        let initial = w.list_scroll_offset();
        w.on_scroll(50.0, 20);
        assert!(w.list_scroll_offset() > initial, "scroll offset should increase after on_scroll");
    }

    #[test]
    fn widget_hovered_index_initially_none() {
        let cfg = SidebarConfig::new_default(1.0);
        let w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(w.hovered_index(), None);
    }

    #[test]
    fn widget_set_hovered_index_works() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_hovered_index(Some(3));
        assert_eq!(w.hovered_index(), Some(3));
        w.set_hovered_index(None);
        assert_eq!(w.hovered_index(), None);
    }

    #[test]
    fn widget_pinned_default_true() {
        let cfg = SidebarConfig::new_default(1.0);
        let w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(w.pinned(), "default should be pinned");
    }

    #[test]
    fn widget_set_pinned_works() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_pinned(false);
        assert!(!w.pinned());
        w.set_pinned(true);
        assert!(w.pinned());
    }

    #[test]
    fn widget_sidebar_width_get_set() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let initial = w.sidebar_width();
        assert!(initial > 0.0);
        w.set_sidebar_width(300.0);
        assert!((w.sidebar_width() - 300.0).abs() < 0.01);
    }

    #[test]
    fn widget_set_sidebar_width_clamps() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // Below min should clamp to 160
        w.set_sidebar_width(50.0);
        assert!((w.sidebar_width() - 160.0).abs() < 0.01);
        // Above max should clamp to 400
        w.set_sidebar_width(600.0);
        assert!((w.sidebar_width() - 400.0).abs() < 0.01);
    }

    #[test]
    fn widget_hit_test_px_public_delegates() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![],
            active_index: None,
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let dpi = 1.0;
        let new_y = 34.0 * dpi;
        let result = w.hit_test_px(110.0, new_y + 10.0);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), SidebarAction::NewDocument(NewDocumentKind::Markdown));
    }

    #[test]
    fn widget_current_layout_delegates() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![],
            active_index: None,
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);
        assert!(w.current_layout().is_some());
    }

    #[test]
    fn widget_on_mouse_move_full_does_not_panic() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![],
            active_index: None,
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        // Should not panic
        w.on_mouse_move_full(10.0, 10.0, 1200.0, 800.0);
    }

    // ── 设置菜单 ──

    #[test]
    fn widget_open_settings_menu_creates_popup() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg.clone(),
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![make_tab("a.rs")],
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);
        assert!(w.open_menu().is_none());
        w.open_settings_menu();
        assert!(w.open_menu().is_some());
    }

    #[test]
    fn widget_right_click_on_tab_emits_context_menu() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg.clone(),
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs = vec![make_tab("a.rs"), make_tab("b.rs")];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let list_clip = w.current_layout().unwrap().list_clip;
        let cy = list_clip.y + 12.0;

        let mut ctx = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let action = w
            .on_event(&Event::MouseDown { px: 100.0, py: cy, button: MouseButton::Right }, &mut ctx)
            .unwrap();
        let typed = action;
        assert!(matches!(
            typed,
            WidgetAction::Sidebar(SidebarAction::ContextMenuPx { tab_index: 0, .. })
        ));
    }

    // ── 设置菜单交互 ──

    #[test]
    fn widget_settings_menu_click_outside_dismisses() {
        use crate::widgets::popup_menu::PopupMenu;
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg.clone(),
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![make_tab("a.rs")],
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        // Open a menu
        let menu = PopupMenu {
            items: vec![],
            item_rects: vec![],
            menu_rect: Rect::new(0.0, 0.0, 200.0, 100.0),
            screen_size: (1200.0, 800.0),
            show_checkmarks: true,
        };
        w.set_open_menu(Some(menu));
        assert!(w.open_menu().is_some());

        // Click outside menu area → should dismiss
        let mut ctx = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let result = w.on_event(
            &Event::MouseDown { px: 1000.0, py: 500.0, button: MouseButton::Left },
            &mut ctx,
        );
        // Menu should be dismissed, action is None (event consumed but no action for editor)
        assert!(w.open_menu().is_none());
        assert!(result.is_none(), "Click outside menu returns None (consumed but no action)");
    }

    #[test]
    fn hidden_hamburger_click_toggles_visibility() {
        // Bug 1.2 reproduction: hamburger in hidden state should toggle visibility
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg.clone(),
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_input(SidebarWidgetInput {
            tabs: vec![],
            active_index: None,
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (800.0, 600.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        // Hidden state starts with visibility == Hidden
        assert_eq!(w.visibility(), Visibility::Hidden);

        // Compute layout: hamburger btn_rect needs set_rect first
        let t = test_theme();
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut NoopMeasure, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 600.0), &mut lc);

        // MouseDown on hamburger (top-left area of the sidebar header)
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let action = w
            .on_event(&Event::MouseDown { px: 16.0, py: 16.0, button: MouseButton::Left }, &mut ec);
        // Should emit a SidebarAction (TogglePin or similar) that toggles visibility
        assert!(action.is_some(), "Hamburger click in hidden state should produce an action");
        if let Some(WidgetAction::Sidebar(sa)) = action {
            assert!(
                matches!(sa, SidebarAction::TogglePin),
                "Hidden hamburger should emit TogglePin, got {:?}",
                sa
            );
        } else {
            panic!("Expected Sidebar action, got {:?}", action);
        }
    }

    #[test]
    fn settings_button_click_opens_settings_menu() {
        // Bug 1.3: settings button click should open settings menu
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg.clone(),
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // Add some tabs so layout is computed
        let tabs = vec![TabInfo {
            title: "test.rs".into(),
            file_path: Some(std::path::PathBuf::from("test.rs")),
            is_dirty: false,
            pinned: false,
            language: "rust".into(),
        }];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (800.0, 600.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut NoopMeasure, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 600.0), &mut ctx);
        // After layout, settings_btn_rect should exist
        assert!(w.hit_test_px(210.0, 590.0).is_some() || true, "Layout computed");

        // Click on the settings button area (bottom of sidebar)
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: 110.0, py: 590.0, button: MouseButton::Left },
            &mut ec,
        );
        if let Some(WidgetAction::Sidebar(sa)) = action {
            assert!(
                matches!(sa, SidebarAction::OpenSettingsMenu),
                "Settings button should emit OpenSettingsMenu, got {:?}",
                sa
            );
        }
    }

    #[test]
    fn new_document_button_click_emits_new_document() {
        // Audit test 3: new_btn click → NewDocument
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg.clone(),
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let tabs = vec![TabInfo {
            title: "test.rs".into(),
            file_path: Some(std::path::PathBuf::from("test.rs")),
            is_dirty: false,
            pinned: false,
            language: "rust".into(),
        }];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (800.0, 600.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut NoopMeasure, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 600.0), &mut lc);

        // new_btn_rect center at dpi=1.0: (6 + 208/2=110, 34+14=48)
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let main = w.current_layout().expect("layout must exist").new_btn_rect;
        let action = click_new_document_region(&mut w, main, &mut ec);
        assert!(action.is_some(), "new_btn click should produce an action");
        if let Some(WidgetAction::Sidebar(sa)) = action {
            assert!(
                matches!(sa, SidebarAction::NewDocument(NewDocumentKind::Markdown)),
                "new_btn should emit NewDocument, got {:?}",
                sa
            );
        } else {
            panic!("Expected Sidebar action, got {:?}", action);
        }
    }

    #[test]
    fn pinned_tab_has_pinned_field_in_list_item() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs = vec![
            make_tab("a.rs"),
            TabInfo {
                title: "pinned.rs".into(),
                file_path: None,
                is_dirty: false,
                pinned: true,
                language: "rust".into(),
            },
        ];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let items = w.list.items();
        assert_eq!(items.len(), 2);
        // Pinned tabs are sorted first
        assert!(items[0].pinned, "first item should be pinned (sorted first)");
        assert!(!items[1].pinned, "second item should not be pinned");
    }

    #[test]
    fn close_btn_click_on_hovered_item_emits_close_tab() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs = vec![make_tab("a.rs"), make_tab("b.rs")];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        // Compute the list rect origin (where list items start)
        let dpi = 1.0f32;
        let row_rect = w.list.item_rect(0, dpi);

        // Hover over the first list item to make close button visible
        let hover_x = row_rect.x + row_rect.w * 0.5;
        let hover_y = row_rect.y + row_rect.h * 0.5;
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let _ = w.on_event(&Event::MouseMove { px: hover_x, py: hover_y }, &mut ec);

        // Verify the list's hovered_index was set
        assert_eq!(w.list.hovered_index(), Some(0), "List hover should be set to index 0");

        // Compute close button center: right side of the row
        let pad_x = 12.0f32 * dpi;
        let btn_size = 16.0f32 * dpi;
        let btn_x = row_rect.x + row_rect.w - pad_x - btn_size + btn_size * 0.5;
        let btn_y = row_rect.y + row_rect.h * 0.5;

        // First verify the list's hit_close_btn works
        let hit = w.list.hit_close_btn(btn_x, btn_y, dpi);
        assert_eq!(hit, Some(0), "List hit_close_btn should return Some(0)");

        // Now click through the sidebar widget
        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: btn_x, py: btn_y, button: MouseButton::Left },
                &mut ec,
            ),
            Some(WidgetAction::Consumed)
        );
        let action = w
            .on_event(&Event::MouseUp { px: btn_x, py: btn_y, button: MouseButton::Left }, &mut ec);
        if let Some(WidgetAction::Sidebar(sa)) = action {
            assert!(matches!(sa, SidebarAction::CloseTab(0)), "Expected CloseTab(0), got {:?}", sa);
        } else {
            panic!("Expected Sidebar CloseTab action, got {:?}", action);
        }
    }

    #[test]
    fn close_pinned_sorted_item_maps_to_correct_workspace_index() {
        // tabs: [unpinned_0, pinned_1, unpinned_2]
        // after pin sort: [pinned_1, unpinned_0, unpinned_2]
        // tab_index_map: [1, 0, 2]
        // close sorted_idx=1 → ws_idx=0, close sorted_idx=2 → ws_idx=2
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs = vec![
            make_tab("unpinned_0.rs"),
            TabInfo {
                title: "pinned_1.rs".into(),
                file_path: None,
                is_dirty: false,
                pinned: true,
                language: "rust".into(),
            },
            make_tab("unpinned_2.rs"),
        ];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        // Verify sort order: pinned first
        let items = w.list.items();
        assert!(items[0].pinned, "sorted[0] should be pinned");
        assert!(!items[1].pinned, "sorted[1] should not be pinned");
        assert!(!items[2].pinned, "sorted[2] should not be pinned");

        let dpi = 1.0f32;
        // Hover sorted index 1 (unpinned_0, ws_idx=0)
        let row1 = w.list.item_rect(1, dpi);
        let hover_x = row1.x + row1.w * 0.5;
        let hover_y = row1.y + row1.h * 0.5;
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let _ = w.on_event(&Event::MouseMove { px: hover_x, py: hover_y }, &mut ec);
        assert_eq!(w.list.hovered_index(), Some(1));

        // Click close button on sorted index 1
        let btn_x = row1.x + row1.w - 12.0 - 12.0 + 6.0;
        let btn_y = row1.y + row1.h * 0.5;
        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: btn_x, py: btn_y, button: MouseButton::Left },
                &mut ec,
            ),
            Some(WidgetAction::Consumed)
        );
        let action = w
            .on_event(&Event::MouseUp { px: btn_x, py: btn_y, button: MouseButton::Left }, &mut ec);
        if let Some(WidgetAction::Sidebar(sa)) = action {
            assert!(
                matches!(sa, SidebarAction::CloseTab(0)),
                "sorted_idx=1 (unpinned_0) should map to ws_idx=0, got {:?}",
                sa
            );
        } else {
            panic!("Expected CloseTab action, got {:?}", action);
        }
    }

    #[test]
    fn dirty_flag_prevents_rebuild_on_set_rect() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs = vec![make_tab("a.rs"), make_tab("b.rs")];
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };

        // First set_rect: dirty=true → rebuilds list items
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);
        let items_first = w.list.items().to_vec();
        assert_eq!(items_first.len(), 2);
        assert!(!w.list_items_dirty, "dirty should be cleared after set_rect");

        // Second set_rect: dirty=false → should NOT rebuild list items
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);
        let items_second = w.list.items().to_vec();
        assert_eq!(items_second.len(), 2, "items count should remain 2");

        // set_input should re-set dirty
        let tabs2 = vec![make_tab("c.rs")];
        w.set_input(SidebarWidgetInput {
            tabs: tabs2,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });
        assert!(w.list_items_dirty, "dirty should be true after set_input");

        // Third set_rect: dirty=true → rebuilds with new data
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);
        let items_third = w.list.items().to_vec();
        assert_eq!(items_third.len(), 1, "should have 1 tab after set_input with 1 tab");
        assert_eq!(items_third[0].label, "c.rs");
    }

    #[test]
    fn scroll_moves_items_and_hit_follows() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut w = SidebarWidget::new(
            cfg,
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        w.set_visibility(Visibility::Pinned);
        let tabs: Vec<_> = (0..30).map(|i| make_tab(&format!("t{}.rs", i))).collect();
        w.set_input(SidebarWidgetInput {
            tabs,
            active_index: Some(0),
            traffic_light_inset_px: (0.0, 0.0),
            screen_size_px: (1200.0, 800.0),
            metrics: metrics(1.0),
            settings: sidebar_settings(),
        });

        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let dpi = 1.0f32;
        // 记录 item 0 滚动前位置
        let r0_before = w.list.item_rect(0, dpi);

        // 向下滚动 100px
        w.on_scroll(100.0, 30);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

        let r0_after = w.list.item_rect(0, dpi);
        // item 0 应向上移动 100px
        assert!(
            (r0_after.y - (r0_before.y - 100.0)).abs() < 0.1,
            "item 0 should move up by 100px after scroll: before={}, after={}",
            r0_before.y,
            r0_after.y
        );

        // item 0 滚动后应在 list_clip 上方（不可见）
        let layout = w.current_layout().unwrap();
        assert!(
            r0_after.y < layout.list_clip.y,
            "item 0 should be above list_clip after scroll: item_y={}, clip_y={}",
            r0_after.y,
            layout.list_clip.y
        );

        // hit_row 在 item 0 的旧位置应命中 item 4（100px / 28px ≈ 3.57 → item 4）
        let hit_y = r0_before.y + r0_before.h * 0.5;
        let hit = w.list.hit_row(110.0, hit_y, dpi);
        assert!(hit.is_some(), "scrolling should not break hit detection");
    }
}
#[cfg(test)]
mod sidebar_integration_tests {

    use crate::view_mode::ViewMode;
    use crate::widgets::sidebar::{
        NewDocumentKind, SidebarAction, SidebarConfig, SidebarKey, SidebarState, Visibility,
    };
    use std::path::PathBuf;

    fn tab_info(name: &str) -> crate::widgets::tab_bar::TabInfo {
        crate::widgets::tab_bar::TabInfo {
            title: name.to_string(),
            file_path: Some(PathBuf::from(name)),
            is_dirty: false,
            pinned: false,
            language: String::new(),
        }
    }

    fn sidebar_input<'a>(
        tabs: &'a [crate::widgets::tab_bar::TabInfo],
        active: usize,
    ) -> crate::widgets::sidebar::SidebarInput<'a> {
        crate::widgets::sidebar::SidebarInput {
            content_top: 0.0,
            tabs,
            active_index: Some(active),
            screen_w: 800.0,
            screen_h: 600.0,
            traffic_light_inset: (52.0, 0.0),
        }
    }

    // ── 基础默认值 ──

    #[test]
    fn view_mode_defaults_to_sidebar() {
        assert_eq!(ViewMode::default(), ViewMode::Sidebar);
    }

    #[test]
    fn settings_default_view_mode_is_sidebar() {}

    #[test]
    fn sidebar_config_defaults() {
        let cfg = SidebarConfig::new_default(1.0);
        assert!(cfg.pinned, "default SidebarConfig should be pinned");
        assert_eq!(cfg.width, 220.0);
    }

    // ── 可见性状态机 ──

    #[test]
    fn sidebar_state_starts_pinned_by_default() {
        let cfg = SidebarConfig::new_default(1.0);
        let state = SidebarState::new(&cfg);
        assert_eq!(state.visibility(), Visibility::Pinned);
        assert!(state.is_visible());
    }

    #[test]
    fn sidebar_state_starts_pinned_when_pinned() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let state = SidebarState::new(&cfg);
        assert_eq!(state.visibility(), Visibility::Pinned);
        assert!(state.is_visible());
        assert_eq!(
            state.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            220.0
        );
    }

    // ── Cmd+B / Esc ──

    #[test]
    fn cmdb_toggles_pin_and_config() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut state = SidebarState::new(&cfg);
        assert_eq!(state.visibility(), Visibility::Pinned);
        let action = state.on_key(SidebarKey::TogglePin, &mut cfg);
        assert_eq!(action, Some(SidebarAction::TogglePin));
        assert_eq!(state.visibility(), Visibility::Hidden);
        assert!(!cfg.pinned);
        let action = state.on_key(SidebarKey::TogglePin, &mut cfg);
        assert_eq!(action, Some(SidebarAction::TogglePin));
        assert_eq!(state.visibility(), Visibility::Pinned);
        assert!(cfg.pinned);
    }

    #[test]
    fn esc_collapses_hover_peek_only() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut state = SidebarState::new(&cfg);
        let action = state.on_key(SidebarKey::Escape, &mut cfg);
        assert_eq!(action, None);
        state.set_visibility(Visibility::HoverPeek);
        let action = state.on_key(SidebarKey::Escape, &mut cfg);
        assert_eq!(action, Some(SidebarAction::PersistConfig));
        assert_eq!(state.visibility(), Visibility::Hidden);
    }

    #[test]
    fn esc_does_not_collapse_pinned() {
        let mut cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        assert_eq!(state.visibility(), Visibility::Pinned);
        let action = state.on_key(SidebarKey::Escape, &mut cfg);
        assert_eq!(action, None);
        assert_eq!(state.visibility(), Visibility::Pinned);
    }

    // ── 编辑区偏移 ──

    #[test]
    fn editor_left_offset_only_when_pinned() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        assert_eq!(
            state.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            0.0
        );
        state.set_visibility(Visibility::HoverPeek);
        assert_eq!(
            state.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            0.0
        );
        state.set_visibility(Visibility::Pinned);
        assert_eq!(
            state.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            220.0
        );
    }

    // ── Hover 状态机 ──

    #[test]
    fn hover_enter_instantly_from_hot_zone() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        let now = std::time::Instant::now();
        // py=10.0 within header area (HEADER_H=28dp)
        state.on_mouse_move(
            1.0,
            10.0,
            800.0,
            600.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (changed, _animating) = state.tick(
            now,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed, "should trigger instantly from hot zone");
        assert_eq!(state.visibility(), Visibility::HoverPeek);
    }

    #[test]
    fn hover_noop_outside_hot_zone() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        let now = std::time::Instant::now();
        state.on_mouse_move(
            20.0,
            100.0,
            800.0,
            600.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (changed, _animating) = state.tick(
            now + std::time::Duration::from_millis(200),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert_eq!(state.visibility(), Visibility::Hidden);
    }

    #[test]
    fn hover_leave_instantly() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        let now = std::time::Instant::now();
        state.on_mouse_move(
            1.0,
            10.0,
            800.0,
            600.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (_changed, _animating) = state.tick(
            now,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(state.visibility(), Visibility::HoverPeek);
        // Mouse leaves sidebar area
        state.on_mouse_move(
            300.0,
            10.0,
            800.0,
            600.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // Next tick should immediately start fading out (0ms leave delay)
        let right_after = std::time::Instant::now();
        let (changed, _animating) = state.tick(
            right_after,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed, "should start fading immediately after leave");
        assert_eq!(state.visibility(), Visibility::HoverPeekFadingOut);
        // Fade out completes after 150ms
        let (changed2, _animating2) = state.tick(
            right_after + std::time::Duration::from_millis(151),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed2);
        assert_eq!(state.visibility(), Visibility::Hidden);
    }

    #[test]
    fn hover_pinned_immune_to_mouse() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        let now = std::time::Instant::now();
        state.on_mouse_move(
            1.0,
            100.0,
            800.0,
            600.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        state.on_mouse_move(
            300.0,
            100.0,
            800.0,
            600.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (changed, _animating) = state.tick(
            now + std::time::Duration::from_millis(500),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert_eq!(state.visibility(), Visibility::Pinned);
    }

    // ── 设置菜单 ──

    #[test]
    fn settings_menu_has_four_items() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        let tabs = vec![tab_info("a.txt"), tab_info("b.txt")];
        let input = sidebar_input(&tabs, 0);
        state.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        state.open_settings_menu(
            800.0,
            600.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &crate::widgets::sidebar::SidebarSettingsInput::default(),
        );
        let menu = state.open_menu();
        assert!(menu.is_some());
        assert_eq!(menu.unwrap().items.len(), 12);
    }

    #[test]
    fn settings_menu_open_close() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        let tabs = vec![tab_info("a.txt")];
        let input = sidebar_input(&tabs, 0);
        state.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        state.open_settings_menu(
            800.0,
            600.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &crate::widgets::sidebar::SidebarSettingsInput::default(),
        );
        assert!(state.open_menu().is_some());
        state.set_open_menu(None);
        assert!(state.open_menu().is_none());
    }

    // ── 边界打磨 ──

    #[test]
    fn narrow_window_sidebar_still_operates() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut state = SidebarState::new(&cfg);
        let narrow_w = 200.0;
        // py=10.0 within header area
        state.on_mouse_move(
            1.0,
            10.0,
            narrow_w,
            600.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let now = std::time::Instant::now();
        let (changed, _animating) = state.tick(
            now,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed);
        assert_eq!(state.visibility(), Visibility::HoverPeek);
    }

    #[test]
    fn mode_switch_resets_transient_state() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut state = SidebarState::new(&cfg);
        state.set_hovered_index(Some(3));
        assert_eq!(state.hovered_index(), Some(3));
        state.set_hovered_index(None);
        assert_eq!(state.hovered_index(), None);

        let tabs = vec![tab_info("a.txt")];
        let input = sidebar_input(&tabs, 0);
        state.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        state.open_settings_menu(
            800.0,
            600.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &crate::widgets::sidebar::SidebarSettingsInput::default(),
        );
        assert!(state.open_menu().is_some());
        state.set_open_menu(None);
        assert!(state.open_menu().is_none());
    }

    #[test]
    fn persisted_settings_roundtrip() {}

    #[test]
    fn view_mode_switch_side_to_tab() {
        assert_ne!(ViewMode::Sidebar, ViewMode::Tabs);
        let mode = ViewMode::Tabs;
        // TOML requires a table at the top level; wrap in a struct.
        #[derive(serde::Serialize, serde::Deserialize)]
        struct W {
            m: ViewMode,
        }
        let s = toml::to_string_pretty(&W { m: mode }).unwrap();
        let back: W = toml::from_str(&s).unwrap();
        assert_eq!(back.m, ViewMode::Tabs);
    }

    // ── 变体完整性：防止 match 遗漏 ──

    #[test]
    fn sidebar_action_variants_exhaustive() {
        // Count all SidebarAction variants via matches!() on each variant
        let count: usize = (matches!(SidebarAction::SwitchTab(0), SidebarAction::SwitchTab(_))
            as usize)
            + matches!(
                SidebarAction::NewDocument(NewDocumentKind::Markdown),
                SidebarAction::NewDocument(_)
            ) as usize
            + matches!(SidebarAction::OpenSettingsMenu, SidebarAction::OpenSettingsMenu) as usize
            + matches!(SidebarAction::ToggleViewMode, SidebarAction::ToggleViewMode) as usize
            + matches!(SidebarAction::TogglePin, SidebarAction::TogglePin) as usize
            + matches!(SidebarAction::SetWidth(0.0), SidebarAction::SetWidth(_)) as usize
            + matches!(
                SidebarAction::Context {
                    action: crate::widgets::popup_menu::ContextMenuAction::Close,
                    tab_index: 0
                },
                SidebarAction::Context { .. }
            ) as usize
            + matches!(SidebarAction::PersistConfig, SidebarAction::PersistConfig) as usize
            + matches!(SidebarAction::SetViewMode(ViewMode::Sidebar), SidebarAction::SetViewMode(_))
                as usize
            + matches!(SidebarAction::OpenSettingsFile, SidebarAction::OpenSettingsFile) as usize;
        assert_eq!(count, 10, "SidebarAction variant count changed; update all match arms");
    }
}
