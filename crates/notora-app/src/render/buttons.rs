//! 手绘外壳操作按钮的指针反馈，产品事件仍由外壳原有路由负责。

use super::{
    NotoraShell, OverlayState, event_pointer_position, paint_navigation_visibility_button,
    paint_note_tool_button,
};
use ui::button::ButtonVisualState;
use ui::{Event, EventCtx, Rect};

impl NotoraShell {
    pub(super) fn update_chrome_button_pointer(
        &mut self,
        event: &Event,
        overlay: Option<OverlayState>,
        context: &mut EventCtx<'_>,
    ) -> bool {
        if super::event_is_keyboard(event) {
            return false;
        }
        let next = event_pointer_position(event).and_then(|(px, py)| {
            let rect = self.chrome_button_at(px, py, overlay)?;
            let state = if matches!(event, Event::MouseDown { button: ui::MouseButton::Left, .. }) {
                ButtonVisualState::Pressed
            } else {
                ButtonVisualState::Hovered
            };
            Some((rect, state))
        });
        if next.is_some() {
            context.cursor_hint = Some(winit::window::CursorIcon::Pointer);
        }
        if next == self.active_chrome_button {
            return false;
        }
        self.active_chrome_button = next;
        true
    }

    fn chrome_button_at(&self, px: f32, py: f32, overlay: Option<OverlayState>) -> Option<Rect> {
        if overlay.is_some_and(|overlay| {
            overlay != OverlayState::None && !self.modal_input_is_ready(overlay)
        }) {
            return None;
        }
        if self.settings_overlay_open()
            || self.new_workspace_dialog_open
            || self.encrypted_note_dialog_open
            || self.new_document_menu_open
            || self.editor_pane.has_open_popup()
            || (self.mindmap_style_panel_open && self.mindmap_style_panel_rect.contains(px, py))
        {
            return None;
        }
        let contains = |rect: &Rect| rect.w > 0.0 && rect.h > 0.0 && rect.contains(px, py);
        if self.save_conflict_actions.is_some() {
            return overlay
                .is_none_or(|overlay| overlay == OverlayState::SaveConflict)
                .then(|| self.save_conflict_button_rects.iter().copied().find(contains))
                .flatten();
        }
        if self.confirmation_action.is_some() {
            return overlay
                .is_none_or(super::is_confirmation_overlay)
                .then(|| {
                    [self.confirmation_cancel_rect, self.confirmation_confirm_rect]
                        .into_iter()
                        .find(contains)
                })
                .flatten();
        }
        if overlay.is_some_and(|overlay| overlay != OverlayState::None) {
            return None;
        }
        self.note_toolbar_buttons
            .iter()
            .map(|button| button.rect)
            .chain([
                self.compact_navigation_rect,
                self.compact_back_rect,
                self.navigation_collapse_rect,
                self.navigation_expand_rect,
            ])
            .find(contains)
    }

    fn chrome_button_state(&self, rect: Rect) -> ButtonVisualState {
        self.active_chrome_button
            .filter(|(target, _)| *target == rect)
            .map_or(ButtonVisualState::Normal, |(_, state)| state)
    }

    pub(super) fn paint_note_tool_button(
        &self,
        context: &mut ui::PaintCtx<'_>,
        rect: Rect,
        label: &str,
        icon: Option<&str>,
    ) {
        paint_note_tool_button(context, rect, label, icon, self.chrome_button_state(rect));
    }

    pub(super) fn paint_navigation_visibility_button(
        &self,
        context: &mut ui::PaintCtx<'_>,
        rect: Rect,
        icon: &str,
    ) {
        paint_navigation_visibility_button(context, rect, icon, self.chrome_button_state(rect));
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn modal_buttons_receive_hover_without_highlighting_the_background_toolbar() {
        let theme = ui::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let mut shell = NotoraShell::new();
        shell.note_toolbar_buttons = vec![RenderedToolbarButton {
            rect: Rect::new(10.0, 10.0, 64.0, 28.0),
            label: "打开".to_owned(),
            icon: Some("folder-open"),
            action: NotoraAction::OpenExternalFileDialogRequested,
        }];
        let toolbar_pointer = Event::MouseMove { px: 20.0, py: 20.0 };
        assert!(
            !shell.update_chrome_button_pointer(
                &toolbar_pointer,
                Some(OverlayState::Settings),
                &mut context
            ),
            "模态尚未渲染时也不能悬停背景工具栏"
        );
        shell.save_conflict_actions = Some(std::array::from_fn(|_| NotoraAction::OverlayDismissed));
        shell.save_conflict_button_rects[0] = Rect::new(100.0, 100.0, 80.0, 28.0);
        assert!(!shell.update_chrome_button_pointer(
            &toolbar_pointer,
            Some(OverlayState::SaveConflict),
            &mut context
        ));
        assert!(shell.update_chrome_button_pointer(
            &Event::MouseMove { px: 110.0, py: 110.0 },
            Some(OverlayState::SaveConflict),
            &mut context
        ));
        assert_eq!(context.cursor_hint, Some(winit::window::CursorIcon::Pointer));
        assert_eq!(
            shell.active_chrome_button,
            Some((shell.save_conflict_button_rects[0], ButtonVisualState::Hovered))
        );
        shell.save_conflict_actions = None;
        shell.confirmation_action = Some(NotoraAction::OverlayDismissed);
        shell.confirmation_confirm_rect = Rect::new(200.0, 200.0, 80.0, 28.0);
        let confirmation = Some(OverlayState::TrashPermanentDeletionConfirmation {
            operation: TrashOperation::Empty,
        });
        shell.update_chrome_button_pointer(&toolbar_pointer, confirmation, &mut context);
        assert_eq!(shell.active_chrome_button, None);
        shell.update_chrome_button_pointer(
            &Event::MouseMove { px: 210.0, py: 210.0 },
            confirmation,
            &mut context,
        );
        assert_eq!(
            shell.active_chrome_button,
            Some((shell.confirmation_confirm_rect, ButtonVisualState::Hovered))
        );
    }

    #[test]
    fn toolbar_hover_does_not_consume_editor_keyboard_or_ime_input() {
        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        shell.note_toolbar_buttons = vec![RenderedToolbarButton {
            rect: Rect::new(10.0, 10.0, 64.0, 28.0),
            label: "打开".to_owned(),
            icon: Some("folder-open"),
            action: NotoraAction::OpenExternalFileDialogRequested,
        }];
        for event in [
            Event::KeyDown(ui::KeyCode::Char('a'), ui::Modifiers::NONE),
            Event::ImeCommit("输入".to_owned()),
        ] {
            shell.route_event(
                &Event::MouseMove { px: 20.0, py: 20.0 },
                FocusTarget::Editor,
                &theme,
                1.0,
            );
            let route = shell.route_event(&event, FocusTarget::Editor, &theme, 1.0);
            assert!(!route.consumed, "按钮悬停不能吞掉编辑器输入: {event:?}");
            assert!(route.actions.is_empty());
        }
    }

    #[test]
    fn file_toolbar_hover_requests_repaint_and_pointer_cursor() {
        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        shell.note_toolbar_buttons = vec![RenderedToolbarButton {
            rect: Rect::new(10.0, 10.0, 64.0, 28.0),
            label: "打开".to_owned(),
            icon: Some("folder-open"),
            action: NotoraAction::OpenExternalFileDialogRequested,
        }];
        let route = shell.route_event(
            &Event::MouseMove { px: 20.0, py: 20.0 },
            FocusTarget::Editor,
            &theme,
            1.0,
        );
        assert!(route.consumed, "进入操作按钮应触发重绘");
        assert_eq!(route.cursor_hint, Some(winit::window::CursorIcon::Pointer));
        assert!(route.actions.is_empty());
        let route = shell.route_event(&Event::PointerLeave, FocusTarget::Editor, &theme, 1.0);
        assert!(route.consumed, "离开窗口应清除操作按钮悬停");
    }
}
