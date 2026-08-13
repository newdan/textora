//! PopupMenu — merged from old `ui/src/popup_menu.rs` + `ui/src/widgets/popup_menu.rs`.

mod types;

pub use types::{ContextMenuAction, OverflowEntry, PopupMenu, PopupMenuAction, PopupMenuItem};

use crate::core::geom::Rect;
use crate::core::widget::{
    Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Widget, WidgetAction,
};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole,
};
use std::any::Any;
use winit::window::CursorIcon;

const POPUP_MENU_ACCESSIBILITY_ID: AccessibilityId = AccessibilityId(0x706f_7075_706d_656e);

/// 弹出菜单的操作结果（上行给 app 层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupOutcome {
    /// 用户选中了某个菜单项。
    Selected(PopupMenuAction),
    /// 用户点击了菜单外区域 / 按 Escape 关闭菜单。
    Dismiss,
}

/// 包装 PopupMenu 的 Widget，用于 UiShell::overlays。
pub struct PopupMenuWidget {
    menu: PopupMenu,
    rect: Rect,
    hovered: Option<usize>,
    highlighted: Option<usize>,
}

impl PopupMenuWidget {
    pub fn new(mut menu: PopupMenu) -> Self {
        let highlighted = menu
            .items
            .iter()
            .position(|item| item.is_active && item.is_selectable())
            .or_else(|| menu.items.iter().position(PopupMenuItem::is_selectable));
        let rect = menu.menu_rect;
        let dx = -rect.x;
        let dy = -rect.y;
        menu.menu_rect.x += dx;
        menu.menu_rect.y += dy;
        for r in &mut menu.item_rects {
            r.x += dx;
            r.y += dy;
        }
        Self { menu, rect: Rect::new(0.0, 0.0, rect.w, rect.h), hovered: None, highlighted }
    }

    /// 暴露内部菜单引用（供 app 层 downcast 后读取）。
    pub fn menu(&self) -> &PopupMenu {
        &self.menu
    }

    pub fn highlighted_index(&self) -> Option<usize> {
        self.highlighted
    }

    fn first_selectable_index(&self) -> Option<usize> {
        self.menu.items.iter().position(PopupMenuItem::is_selectable)
    }

    fn last_selectable_index(&self) -> Option<usize> {
        self.menu.items.iter().rposition(PopupMenuItem::is_selectable)
    }

    fn move_highlight(&mut self, forward: bool) {
        let item_count = self.menu.items.len();
        if item_count == 0 {
            self.highlighted = None;
            return;
        }
        let Some(current) = self.highlighted else {
            self.highlighted =
                if forward { self.first_selectable_index() } else { self.last_selectable_index() };
            return;
        };
        for distance in 1..=item_count {
            let index = if forward {
                (current + distance) % item_count
            } else {
                (current + item_count - distance % item_count) % item_count
            };
            if self.menu.items[index].is_selectable() {
                self.highlighted = Some(index);
                return;
            }
        }
        self.highlighted = None;
    }

    fn activate_highlighted(&self) -> WidgetAction {
        let Some(index) = self.highlighted else {
            return WidgetAction::Consumed;
        };
        let item = &self.menu.items[index];
        if !item.is_selectable() {
            return WidgetAction::Consumed;
        }
        WidgetAction::Popup(PopupOutcome::Selected(item.action.clone()))
    }
}

impl Widget for PopupMenuWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        self.menu.paint(ctx, self.hovered.or(self.highlighted));
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let mut root = AccessibilityNode::new(
            POPUP_MENU_ACCESSIBILITY_ID,
            AccessibilityRole::Menu,
            ctx.screen_bounds(self.rect),
        )
        .with_name("弹出菜单");
        for (index, (item, item_rect)) in
            self.menu.items.iter().zip(&self.menu.item_rects).enumerate()
        {
            let role = if item.is_separator {
                AccessibilityRole::Separator
            } else {
                AccessibilityRole::MenuItem
            };
            let mut child = AccessibilityNode::new(
                POPUP_MENU_ACCESSIBILITY_ID.child(index as u64 + 1),
                role,
                ctx.screen_bounds(*item_rect),
            )
            .with_disabled(!item.enabled)
            .with_selected(item.is_active)
            .with_focused(self.highlighted == Some(index));
            if !item.label.is_empty() {
                child = child.with_name(item.label.clone());
            }
            if item.is_selectable() {
                child = child.with_action(AccessibilityAction::Activate);
            }
            root.children.push(child);
        }
        Some(root)
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if request.action != AccessibilityAction::Activate {
            return None;
        }
        let index = (0..self.menu.items.len())
            .find(|index| POPUP_MENU_ACCESSIBILITY_ID.child(*index as u64 + 1) == request.target)?;
        let item = self.menu.items.get(index)?;
        item.is_selectable()
            .then(|| WidgetAction::Popup(PopupOutcome::Selected(item.action.clone())))
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            Event::MouseMove { px, py } => {
                let inside = self.rect.contains(*px, *py);
                if inside {
                    ctx.cursor_hint = Some(CursorIcon::Default);
                }
                self.hovered = if inside {
                    self.menu
                        .item_rects
                        .iter()
                        .enumerate()
                        .find(|(index, r)| {
                            r.contains(*px, *py) && self.menu.items[*index].is_selectable()
                        })
                        .map(|(i, _)| i)
                } else {
                    None
                };
                None
            }
            Event::PointerLeave | Event::InteractionCancel => {
                self.hovered.take().map(|_| WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if let Some(action) = self.menu.hit_test_px(*px, *py) {
                    Some(WidgetAction::Popup(PopupOutcome::Selected(action.clone())))
                } else if !self.rect.contains(*px, *py) {
                    Some(WidgetAction::Popup(PopupOutcome::Dismiss))
                } else {
                    Some(WidgetAction::Consumed)
                }
            }
            Event::KeyDown(KeyCode::Escape, _) => Some(WidgetAction::Popup(PopupOutcome::Dismiss)),
            Event::KeyDown(key, modifiers) => {
                if *modifiers != crate::core::widget::Modifiers::NONE {
                    return Some(WidgetAction::Consumed);
                }
                self.hovered = None;
                match key {
                    KeyCode::Up => self.move_highlight(false),
                    KeyCode::Down => self.move_highlight(true),
                    KeyCode::Home => self.highlighted = self.first_selectable_index(),
                    KeyCode::End => self.highlighted = self.last_selectable_index(),
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        return Some(self.activate_highlighted());
                    }
                    _ => return Some(WidgetAction::Consumed),
                }
                Some(WidgetAction::Consumed)
            }
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::widget::{Event, EventCtx, KeyCode, Modifiers, MouseButton, Widget};
    use crate::settings::Settings;
    use crate::tab_bar::{NavButtonLayout, TabBarCtx, TabBarLayout, TabEntry, TabIndicator};

    fn make_ctx() -> TabBarCtx {
        TabBarCtx { screen_w: 1200.0, screen_h: 800.0, dpi: 1.0 }
    }

    fn menu_from_items(items: Vec<PopupMenuItem>) -> PopupMenu {
        const ITEM_HEIGHT_PX: f32 = 20.0;
        const MENU_WIDTH_PX: f32 = 120.0;
        let item_rects = (0..items.len())
            .map(|index| {
                Rect::new(0.0, index as f32 * ITEM_HEIGHT_PX, MENU_WIDTH_PX, ITEM_HEIGHT_PX)
            })
            .collect();
        PopupMenu {
            menu_rect: Rect::new(0.0, 0.0, MENU_WIDTH_PX, items.len() as f32 * ITEM_HEIGHT_PX),
            items,
            item_rects,
            screen_size: (MENU_WIDTH_PX, 800.0),
            show_checkmarks: false,
        }
    }

    fn key_result(widget: &mut PopupMenuWidget, key: KeyCode) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        widget.on_event(&Event::KeyDown(key, Modifiers::NONE), &mut ctx)
    }

    #[test]
    fn accessibility_exposes_menu_items_disabled_state_and_selected_action() {
        let menu = menu_from_items(vec![
            PopupMenuItem::action("禁用", PopupMenuAction::ToggleLineNumbers).with_enabled(false),
            PopupMenuItem::action("自动换行", PopupMenuAction::ToggleWordWrap).with_active(true),
        ]);
        let mut widget = PopupMenuWidget::new(menu);
        let node = widget
            .accessibility_node(&crate::core::AccessibilityContext::new(10.0, 20.0))
            .expect("popup menu should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::Menu);
        assert_eq!(node.children.len(), 2);
        assert!(node.children[0].state.disabled);
        assert!(node.children[0].actions.is_empty());
        assert_eq!(node.children[1].name.as_deref(), Some("自动换行"));
        assert_eq!(node.children[1].state.selected, Some(true));
        assert!(node.children[1].state.focused);
        assert_eq!(node.children[1].bounds, Rect::new(10.0, 40.0, 120.0, 20.0));
        assert_eq!(
            widget.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.children[1].id,
                crate::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::Popup(PopupOutcome::Selected(PopupMenuAction::ToggleWordWrap)))
        );
    }

    // ── PopupMenu 类型测试（从旧 popup_menu.rs 合并）──

    #[test]
    fn hit_test_returns_none_for_outside_position() {
        let ctx = make_ctx();
        let menu =
            PopupMenu::context_px(0, (600.0, 400.0), (ctx.screen_w, ctx.screen_h), false, 1.0);
        assert!(menu.hit_test_px(6.0, 0.0).is_none());
    }

    #[test]
    fn hit_test_returns_action_for_item_center() {
        let ctx = make_ctx();
        let menu =
            PopupMenu::context_px(0, (600.0, 400.0), (ctx.screen_w, ctx.screen_h), false, 1.0);
        let first = menu.item_rects[0];
        let cx = first.x + first.w * 0.5;
        let cy = first.y + first.h * 0.5;
        let hit = menu.hit_test_px(cx, cy);
        assert!(hit.is_some());
    }

    #[test]
    fn overflow_does_not_panic() {
        let layout = TabBarLayout {
            tabs: vec![TabEntry {
                index: 0,
                title: "a very long filename that should be truncated.rs".into(),
                indicator: TabIndicator::None,
                pinned: false,
                preview: false,
                disambiguation: None,
                rect_px: Rect::ZERO,
                close_rect_px: Rect::ZERO,
            }],
            clip_left_px: 0.0,
            clip_right_px: 0.0,
            overflow: false,
            scroll_offset: 0.0,
            max_scroll: 0.0,
            nav_buttons: NavButtonLayout {
                back_rect_px: Rect::ZERO,
                forward_rect_px: Rect::ZERO,
                back_enabled: false,
                forward_enabled: false,
            },
            dropdown_rect_px: Rect::ZERO,
            overflow_left_rect_px: Rect::ZERO,
            overflow_right_rect_px: Rect::ZERO,
            new_tab_rect_px: Rect::ZERO,
            fade_left_rect_px: Rect::ZERO,
            fade_right_rect_px: Rect::ZERO,
            left_arrow_disabled: false,
            right_arrow_disabled: false,
            pinned_total_width: 0.0,
        };
        let ctx = make_ctx();
        let dd = layout.dropdown_rect_px;
        let entries: Vec<OverflowEntry> = layout
            .tabs
            .iter()
            .map(|e| OverflowEntry { tab_index: e.index, title: e.title.clone() })
            .collect();
        let menu = PopupMenu::overflow_px(&entries, dd, (ctx.screen_w, ctx.screen_h), 0, 1.0);
        assert!(!menu.items.is_empty());
        assert_eq!(menu.items.len(), menu.item_rects.len());
    }

    #[test]
    fn context_menu_contains_expected_actions() {
        let ctx = make_ctx();
        let menu =
            PopupMenu::context_px(5, (600.0, 400.0), (ctx.screen_w, ctx.screen_h), false, 1.0);
        assert_eq!(menu.items.len(), 7);

        let actions: Vec<_> = menu.items.iter().map(|i| &i.action).collect();
        assert!(matches!(
            actions[0],
            PopupMenuAction::Context { action: ContextMenuAction::Close, tab_index: 5 }
        ));
        assert!(matches!(
            actions[1],
            PopupMenuAction::Context { action: ContextMenuAction::CloseOthers, .. }
        ));
        assert!(matches!(
            actions[2],
            PopupMenuAction::Context { action: ContextMenuAction::CloseRight, .. }
        ));
        assert!(matches!(
            actions[3],
            PopupMenuAction::Context { action: ContextMenuAction::CloseAll, .. }
        ));
        assert!(menu.items[4].is_separator, "item 4 should be a separator");
        assert!(matches!(
            actions[5],
            PopupMenuAction::Context { action: ContextMenuAction::CopyPath, .. }
        ));
        assert!(matches!(
            actions[6],
            PopupMenuAction::Context { action: ContextMenuAction::TogglePin, .. }
        ));
    }

    // ── PopupMenuWidget 测试（从旧 widgets/popup_menu.rs 合并）──

    #[test]
    fn widget_hit_returns_true_for_menu_rect() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (100.0, 100.0), (1200.0, 800.0), false, 1.0);
        let widget = PopupMenuWidget::new(menu);
        let cx = widget.rect.x + widget.rect.w * 0.5;
        let cy = widget.rect.y + widget.rect.h * 0.5;
        assert!(widget.hit(cx, cy));
    }

    #[test]
    fn widget_on_event_escape_dismisses() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (100.0, 100.0), (1200.0, 800.0), false, 1.0);
        let mut widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let result = widget.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut ctx);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), WidgetAction::Popup(PopupOutcome::Dismiss)));
    }

    #[test]
    fn widget_on_event_click_outside_dismisses() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let result = widget.on_event(
            &Event::MouseDown { px: -10.0, py: -10.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), WidgetAction::Popup(PopupOutcome::Dismiss)));
    }

    #[test]
    fn widget_on_event_click_on_item_selects() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let r = widget.menu.item_rects[0];
        let cx = r.x + r.w * 0.5;
        let cy = r.y + r.h * 0.5;
        let result = widget
            .on_event(&Event::MouseDown { px: cx, py: cy, button: MouseButton::Left }, &mut ctx);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), WidgetAction::Popup(PopupOutcome::Selected(_))));
    }

    #[test]
    fn widget_mouse_move_updates_hover() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let r = widget.menu.item_rects[0];
        let cx = r.x + r.w * 0.5;
        let cy = r.y + r.h * 0.5;
        let result = widget.on_event(&Event::MouseMove { px: cx, py: cy }, &mut ctx);
        assert!(result.is_none());
    }

    #[test]
    fn keyboard_highlight_prefers_an_active_selectable_item() {
        let menu = menu_from_items(vec![
            PopupMenuItem::separator(PopupMenuAction::ToggleLineNumbers),
            PopupMenuItem::action("禁用", PopupMenuAction::ToggleWordWrap).with_enabled(false),
            PopupMenuItem::action("当前", PopupMenuAction::ToggleStatusBar).with_active(true),
            PopupMenuItem::action("其他", PopupMenuAction::OpenSettingsFile),
        ]);

        let widget = PopupMenuWidget::new(menu);

        assert_eq!(widget.highlighted_index(), Some(2));
    }

    #[test]
    fn keyboard_navigation_skips_disabled_items_and_separators_and_wraps() {
        let menu = menu_from_items(vec![
            PopupMenuItem::separator(PopupMenuAction::ToggleLineNumbers),
            PopupMenuItem::action("禁用", PopupMenuAction::ToggleWordWrap).with_enabled(false),
            PopupMenuItem::action("第一个", PopupMenuAction::ToggleStatusBar),
            PopupMenuItem::separator(PopupMenuAction::OpenSettingsFile),
            PopupMenuItem::action("禁用二", PopupMenuAction::ToggleLineNumbers).with_enabled(false),
            PopupMenuItem::action("最后一个", PopupMenuAction::ToggleWordWrap),
        ]);
        let mut widget = PopupMenuWidget::new(menu);

        assert_eq!(widget.highlighted_index(), Some(2));
        assert_eq!(key_result(&mut widget, KeyCode::Down), Some(WidgetAction::Consumed));
        assert_eq!(widget.highlighted_index(), Some(5));
        assert_eq!(key_result(&mut widget, KeyCode::Down), Some(WidgetAction::Consumed));
        assert_eq!(widget.highlighted_index(), Some(2));
        assert_eq!(key_result(&mut widget, KeyCode::Up), Some(WidgetAction::Consumed));
        assert_eq!(widget.highlighted_index(), Some(5));
        assert_eq!(key_result(&mut widget, KeyCode::Home), Some(WidgetAction::Consumed));
        assert_eq!(widget.highlighted_index(), Some(2));
        assert_eq!(key_result(&mut widget, KeyCode::End), Some(WidgetAction::Consumed));
        assert_eq!(widget.highlighted_index(), Some(5));
        assert_eq!(
            key_result(&mut widget, KeyCode::Enter),
            Some(WidgetAction::Popup(PopupOutcome::Selected(PopupMenuAction::ToggleWordWrap)))
        );
    }

    #[test]
    fn all_disabled_menu_consumes_keys_without_selecting() {
        let menu = menu_from_items(vec![
            PopupMenuItem::action("禁用", PopupMenuAction::ToggleLineNumbers).with_enabled(false),
            PopupMenuItem::separator(PopupMenuAction::ToggleWordWrap),
        ]);
        let mut widget = PopupMenuWidget::new(menu);

        assert_eq!(widget.highlighted_index(), None);
        for key in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Tab,
        ] {
            assert_eq!(key_result(&mut widget, key), Some(WidgetAction::Consumed));
            assert_eq!(widget.highlighted_index(), None);
        }
    }

    #[test]
    fn single_item_and_long_menu_navigation_remain_in_bounds() {
        let mut single = PopupMenuWidget::new(menu_from_items(vec![PopupMenuItem::action(
            "唯一",
            PopupMenuAction::ToggleLineNumbers,
        )]));
        for key in [KeyCode::Up, KeyCode::Down, KeyCode::Home, KeyCode::End] {
            assert_eq!(key_result(&mut single, key), Some(WidgetAction::Consumed));
            assert_eq!(single.highlighted_index(), Some(0));
        }
        assert_eq!(
            key_result(&mut single, KeyCode::Char(' ')),
            Some(WidgetAction::Popup(PopupOutcome::Selected(PopupMenuAction::ToggleLineNumbers)))
        );

        let long_items = (0..100)
            .map(|index| {
                PopupMenuItem::action(format!("菜单项 {index}"), PopupMenuAction::SwitchTab(index))
                    .with_enabled(index % 7 != 0)
            })
            .collect();
        let mut long_menu = PopupMenuWidget::new(menu_from_items(long_items));
        for _ in 0..250 {
            assert_eq!(key_result(&mut long_menu, KeyCode::Down), Some(WidgetAction::Consumed));
            let index = long_menu
                .highlighted_index()
                .expect("long menu should retain a selectable highlight");
            assert_ne!(index % 7, 0);
            assert!(index < 100);
        }
    }

    #[test]
    fn disabled_item_rejects_mouse_hover_and_activation() {
        let menu = menu_from_items(vec![
            PopupMenuItem::action("禁用", PopupMenuAction::ToggleLineNumbers).with_enabled(false),
            PopupMenuItem::action("可用", PopupMenuAction::ToggleWordWrap),
        ]);
        let mut widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(widget.on_event(&Event::MouseMove { px: 10.0, py: 10.0 }, &mut ctx), None);
        assert_eq!(widget.hovered, None);
        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 10.0, py: 10.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
    }

    #[test]
    fn popup_lifecycle_clears_pointer_hover_without_losing_keyboard_highlight() {
        let menu = menu_from_items(vec![
            PopupMenuItem::action("第一个", PopupMenuAction::ToggleLineNumbers),
            PopupMenuItem::action("第二个", PopupMenuAction::ToggleWordWrap),
        ]);
        let mut widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(widget.on_event(&Event::MouseMove { px: 10.0, py: 30.0 }, &mut ctx), None);
        assert_eq!(widget.hovered, Some(1));
        assert_eq!(widget.highlighted_index(), Some(0));
        assert_eq!(widget.on_event(&Event::PointerLeave, &mut ctx), Some(WidgetAction::Consumed));
        assert_eq!(widget.hovered, None);
        assert_eq!(widget.highlighted_index(), Some(0));

        let _ = widget.on_event(&Event::MouseMove { px: 10.0, py: 30.0 }, &mut ctx);
        assert_eq!(
            widget.on_event(&Event::InteractionCancel, &mut ctx),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(widget.on_event(&Event::InteractionCancel, &mut ctx), None);
        assert_eq!(
            key_result(&mut widget, KeyCode::Enter),
            Some(WidgetAction::Popup(PopupOutcome::Selected(PopupMenuAction::ToggleLineNumbers)))
        );
    }

    // ── paint 测试 ──

    #[test]
    fn paint_context_menu_emits_shadow_bg_border_and_text() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        widget.paint(&mut pc);

        // Print all command types for debugging
        for (i, cmd) in dl.cmds.iter().enumerate() {
            let kind = match cmd {
                DrawCmd::FillRect { .. } => "FillRect",
                DrawCmd::StrokeRect { .. } => "StrokeRect",
                DrawCmd::TextLayout { .. } => "Text",
                _ => "Other",
            };
            eprintln!("cmd[{}]: {}", i, kind);
        }

        // Border + Background + separator fill + 6 * text = 2 + 1 + 6 = 9
        assert!(dl.cmds.len() >= 9, "expected at least 9 draw commands, got {}", dl.cmds.len());
        // First is border (fill_rounded outer)
        assert!(matches!(dl.cmds[0], DrawCmd::FillRect { .. }));
        // Second is background (fill_rounded)
        assert!(matches!(dl.cmds[1], DrawCmd::FillRect { .. }));
        // Fourth is first item fill (no hover/active → no extra fill, just text)
        // Actually: context menu items are not active and not hovered,
        // so there should be no fill for them, only text
        // Let's just check there are text commands for each item
        let text_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).count();
        assert_eq!(text_count, 6, "should have 6 text commands (separator has no text)");
    }

    #[test]
    fn paint_hover_item_gets_hover_highlight() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };

        // Simulate hover on first item by calling paint with hovered=Some(0)
        // We need to call menu.paint directly since PopupMenuWidget.paint uses internal state
        widget.menu.paint(&mut pc, Some(0));

        // The first item's FillRect should use menu_hover color
        // Command order: border, bg, [item_fill, item_text] * n
        // item 0 fill at index 2
        if let DrawCmd::FillRect { color, .. } = &dl.cmds[2] {
            assert_eq!(
                *color, theme.palette.sidebar_hover_bg,
                "hovered item should use menu_hover color"
            );
        } else {
            panic!("expected FillRect for hovered item");
        }
    }

    #[test]
    fn paint_active_item_gets_highlight_background() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut menu = menu;
        menu.items[0].is_active = true;
        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        menu.paint(&mut pc, None);

        // Active item should get menu_selected background highlight
        let has_selected_bg = dl.cmds.iter().any(|cmd| {
            if let DrawCmd::FillRect { color, .. } = cmd {
                *color == theme.palette.sidebar_active_bg
            } else {
                false
            }
        });
        assert!(has_selected_bg, "active item should have menu_selected background highlight");
    }

    #[test]
    fn paint_active_item_no_checkmark() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut menu = menu;
        menu.items[0].is_active = true;
        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        menu.paint(&mut pc, None);

        // Active item should NOT render a checkmark character
        let has_checkmark = dl.cmds.iter().any(|cmd| {
            if let DrawCmd::TextLayout { layout, .. } = cmd {
                layout.text == "\u{2713}"
            } else {
                false
            }
        });
        assert!(!has_checkmark, "active item should NOT render checkmark");
    }

    #[test]
    fn paint_overflow_menu_item_count_matches() {
        let _ = Box::leak(Box::new(Settings::new()));
        let entries: Vec<OverflowEntry> =
            (0..5).map(|i| OverflowEntry { tab_index: i, title: format!("tab_{}", i) }).collect();
        let menu = PopupMenu::overflow_px(
            &entries,
            Rect::new(900.0, 0.0, 60.0, 28.0),
            (1200.0, 800.0),
            0,
            1.0,
        );
        let widget = PopupMenuWidget::new(menu);
        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        widget.paint(&mut pc);

        // All items get 1 text each (no checkmark) = 5 total
        let text_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).count();
        assert_eq!(text_count, 5, "all items have 1 text each (no checkmark)");
    }

    // ── show_checkmarks 测试 ──

    #[test]
    fn context_menu_show_checkmarks_false() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        assert!(!menu.show_checkmarks, "context menu should have show_checkmarks=false");
    }

    #[test]
    fn overflow_menu_show_checkmarks_false() {
        let _ = Box::leak(Box::new(Settings::new()));
        let entries: Vec<OverflowEntry> =
            vec![OverflowEntry { tab_index: 0, title: "test.rs".into() }];
        let menu = PopupMenu::overflow_px(
            &entries,
            Rect::new(900.0, 0.0, 60.0, 28.0),
            (1200.0, 800.0),
            0,
            1.0,
        );
        assert!(!menu.show_checkmarks, "overflow menu should have show_checkmarks=false");
    }

    #[test]
    fn paint_show_checkmarks_active_item_renders_checkmark() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut menu = menu;
        menu.show_checkmarks = true;
        menu.items[0].is_active = true;
        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        menu.paint(&mut pc, None);

        // Active item should render a checkmark character when show_checkmarks=true
        let has_checkmark = dl.cmds.iter().any(|cmd| {
            if let DrawCmd::TextLayout { layout, .. } = cmd {
                layout.text == "\u{2713}"
            } else {
                false
            }
        });
        assert!(has_checkmark, "active item should render checkmark when show_checkmarks=true");
    }

    #[test]
    fn paint_show_checkmarks_inactive_item_no_checkmark() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut menu = menu;
        menu.show_checkmarks = true;
        // items[0] is_active defaults to false
        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        menu.paint(&mut pc, None);

        // Inactive item should NOT render a checkmark
        let has_checkmark = dl.cmds.iter().any(|cmd| {
            if let DrawCmd::TextLayout { layout, .. } = cmd {
                layout.text == "\u{2713}"
            } else {
                false
            }
        });
        assert!(!has_checkmark, "inactive item should NOT render checkmark");
    }

    #[test]
    fn paint_show_checkmarks_text_offset_right() {
        let _ = Box::leak(Box::new(Settings::new()));
        let menu = PopupMenu::context_px(0, (400.0, 200.0), (1200.0, 800.0), false, 1.0);
        let mut menu_no_check = menu.clone();
        menu_no_check.show_checkmarks = false;
        let mut menu_with_check = menu;
        menu_with_check.show_checkmarks = true;

        let theme = crate::theme::test_theme();

        // Paint without checkmarks
        let mut dl1 = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc1 = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl1,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        menu_no_check.paint(&mut pc1, None);

        // Paint with checkmarks
        let mut dl2 = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc2 = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl2,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        menu_with_check.paint(&mut pc2, None);

        // Get text x positions for first item
        let get_first_text_x = |dl: &DrawList| -> f32 {
            for cmd in &dl.cmds {
                if let DrawCmd::TextLayout { x, .. } = cmd {
                    return *x;
                }
            }
            panic!("no text command found");
        };

        let x1 = get_first_text_x(&dl1);
        let x2 = get_first_text_x(&dl2);
        assert!(
            x2 > x1,
            "text with checkmarks should be offset to the right (x1={}, x2={})",
            x1,
            x2
        );
    }
}
