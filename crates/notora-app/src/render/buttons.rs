//! 手绘外壳操作按钮的指针反馈，产品事件仍由外壳原有路由负责。

use super::{
    NotoraAction, NotoraShell, OverlayState, TrashOperation, event_pointer_position,
    paint_navigation_visibility_button, paint_note_tool_button,
};
use ui::button::{ButtonStyle, ButtonVisualState};
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
        action: Option<&NotoraAction>,
    ) {
        let destructive = action.is_some_and(is_permanent_deletion);
        let style = if destructive {
            ButtonStyle::destructive(context.theme.settings_theme())
        } else {
            ButtonStyle::from_theme(context.theme)
        };
        let icon = icon.or(destructive.then_some("trash-2"));
        paint_note_tool_button(context, rect, label, icon, self.chrome_button_state(rect), &style);
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

fn is_permanent_deletion(action: &NotoraAction) -> bool {
    matches!(
        action,
        NotoraAction::TrashOperationRequested(
            TrashOperation::Empty | TrashOperation::PermanentlyDelete { .. }
        ) | NotoraAction::TrashPermanentDeletionConfirmed
    )
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn trash_actions_expose_restore_and_delete_icons() {
        let buttons =
            note_toolbar_buttons(&NavigationScope::Trash, Some(NoteId::generate()), false);
        assert_eq!(buttons[0].icon, Some("undo-2"));
        assert_eq!(buttons[1].icon, Some("trash-2"));
        let empty = note_toolbar_buttons(&NavigationScope::Trash, None, false);
        assert_eq!(empty[0].icon, Some("trash-2"));
    }

    #[test]
    fn permanent_delete_and_confirmation_use_danger_foreground() {
        use ui::core::paint::{DrawCmd, DrawList};

        let theme = ui::theme::test_theme();
        let rect = Rect::new(10.0, 10.0, 96.0, 28.0);
        let mut shell = NotoraShell::new();
        shell.note_toolbar_buttons = vec![RenderedToolbarButton {
            rect,
            icon: Some("trash-2"),
            label: "删除".to_owned(),
            action: NotoraAction::TrashOperationRequested(TrashOperation::PermanentlyDelete {
                note_id: NoteId::generate(),
            }),
        }];
        let mut shaper = shaping::Shaper::new().expect("delete button test requires fonts");
        for confirmation in [false, true] {
            if confirmation {
                shell.note_toolbar_buttons.clear();
                shell.confirmation_confirm_rect = rect;
                shell.confirmation_action = Some(NotoraAction::TrashPermanentDeletionConfirmed);
            }
            let mut draw_list = DrawList::new();
            let mut context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);
            context.shaper = Some(&mut shaper);
            let action = if confirmation {
                shell.confirmation_action.as_ref()
            } else {
                Some(&shell.note_toolbar_buttons[0].action)
            };
            shell.paint_note_tool_button(&mut context, rect, "删除", None, action);
            let expected = theme.application_theme().button_danger_foreground;
            assert!(
                draw_list.cmds.iter().any(|command| matches!(command,
                    DrawCmd::TextLayout { color, .. } if *color == expected
                )),
                "永久删除与最终确认必须使用危险操作文字色"
            );
            assert!(
                draw_list.cmds.iter().any(|command| matches!(command,
                    DrawCmd::FillTriangle { color, .. } if *color == expected
                )),
                "删除图标应与危险操作文字同色"
            );
        }
    }

    #[test]
    fn recover_cancel_and_external_file_clear_keep_standard_foreground() {
        use ui::core::paint::{DrawCmd, DrawList};

        let theme = ui::theme::test_theme();
        let shell = NotoraShell::new();
        let mut shaper = shaping::Shaper::new().expect("standard action test requires fonts");
        for action in [
            NotoraAction::TrashOperationRequested(TrashOperation::Restore {
                note_id: NoteId::generate(),
            }),
            NotoraAction::TrashRestoreWithRenamedPathConfirmed,
            NotoraAction::OverlayDismissed,
            NotoraAction::ExternalFilesClearRequested,
        ] {
            let mut draw_list = DrawList::new();
            let mut context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);
            context.shaper = Some(&mut shaper);
            shell.paint_note_tool_button(
                &mut context,
                Rect::new(10.0, 10.0, 96.0, 28.0),
                "操作",
                Some("undo-2"),
                Some(&action),
            );
            for command in &draw_list.cmds {
                if let DrawCmd::TextLayout { color, .. } | DrawCmd::FillTriangle { color, .. } =
                    command
                {
                    assert_eq!(*color, theme.application_theme().text_primary);
                }
            }
        }
    }

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
