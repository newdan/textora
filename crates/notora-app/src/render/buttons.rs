//! Notora 外壳按钮的布局、绘制和输入路由。

use super::{NotoraAction, NotoraShell, OverlayState, TrashOperation, event_pointer_position};
use ui::button::{Button, ButtonStyle};
use ui::core::widget::{ControlAction, WidgetId};
use ui::{Event, EventCtx, LayoutCtx, Rect, Widget, WidgetAction};

const CHROME_BUTTON_ID_BASE: u64 = 0x6e6f_746f_6368_0000;
const TOOLBAR_BUTTON_ID_BASE: u64 = 0x6e6f_746f_7462_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChromeButtonKey {
    Toolbar(usize),
    CompactNavigation,
    CompactBack,
    NavigationCollapse,
    NavigationExpand,
    Settings,
    ConfirmationCancel,
    ConfirmationConfirm,
    SaveConflict(usize),
}

impl ChromeButtonKey {
    fn widget_id(self) -> WidgetId {
        let offset = match self {
            Self::Toolbar(index) => return WidgetId(TOOLBAR_BUTTON_ID_BASE + index as u64),
            Self::Settings => return super::SETTINGS_BUTTON_ID,
            Self::CompactNavigation => 0,
            Self::CompactBack => 1,
            Self::NavigationCollapse => 2,
            Self::NavigationExpand => 3,
            Self::ConfirmationCancel => 5,
            Self::ConfirmationConfirm => 6,
            Self::SaveConflict(index) => 7 + index as u64,
        };
        WidgetId(CHROME_BUTTON_ID_BASE + offset)
    }
}

pub(super) struct ChromeButton {
    key: ChromeButtonKey,
    widget: Button,
}

struct ChromeButtonSpec {
    key: ChromeButtonKey,
    rect: Rect,
    text: Option<String>,
    accessibility_label: String,
    icon: Option<String>,
    style: ButtonStyle,
}

impl NotoraShell {
    pub(super) fn synchronize_chrome_buttons(
        &mut self,
        confirmation_label: Option<&str>,
        context: &mut LayoutCtx<'_>,
    ) {
        let theme = context.theme;
        let mut specs = Vec::new();
        for (index, input) in self.note_toolbar_buttons.iter().enumerate() {
            let style = if is_permanent_deletion(&input.action) {
                ButtonStyle::destructive(theme.settings_theme())
            } else {
                ButtonStyle::from_theme(theme)
            };
            specs.push(ChromeButtonSpec {
                key: ChromeButtonKey::Toolbar(index),
                rect: input.rect,
                text: Some(input.label.clone()),
                accessibility_label: input.label.clone(),
                icon: input.icon.map(str::to_owned),
                style: style.toolbar_item(),
            });
        }
        let action_style = ButtonStyle::from_theme(theme);
        let ghost_style = ButtonStyle::ghost(theme.settings_theme());
        specs.extend([
            ChromeButtonSpec {
                key: ChromeButtonKey::CompactNavigation,
                rect: self.compact_navigation_rect,
                text: Some("笔记库".into()),
                accessibility_label: "笔记库".into(),
                icon: None,
                style: action_style.clone(),
            },
            ChromeButtonSpec {
                key: ChromeButtonKey::CompactBack,
                rect: self.compact_back_rect,
                text: Some("返回".into()),
                accessibility_label: "返回".into(),
                icon: None,
                style: action_style.clone(),
            },
            ChromeButtonSpec {
                key: ChromeButtonKey::NavigationCollapse,
                rect: self.navigation_collapse_rect,
                text: None,
                accessibility_label: "收起笔记库".into(),
                icon: Some("chevron-left".into()),
                style: ghost_style.clone(),
            },
            ChromeButtonSpec {
                key: ChromeButtonKey::NavigationExpand,
                rect: self.navigation_expand_rect,
                text: None,
                accessibility_label: "展开笔记库".into(),
                icon: Some("chevron-right".into()),
                style: ghost_style,
            },
            ChromeButtonSpec {
                key: ChromeButtonKey::Settings,
                rect: self.settings_rect,
                text: Some("设置".into()),
                accessibility_label: "设置".into(),
                icon: Some("settings".into()),
                style: ButtonStyle::ghost(theme.settings_theme()),
            },
            ChromeButtonSpec {
                key: ChromeButtonKey::ConfirmationCancel,
                rect: self.confirmation_cancel_rect,
                text: Some("取消".into()),
                accessibility_label: "取消".into(),
                icon: None,
                style: action_style.clone(),
            },
            ChromeButtonSpec {
                key: ChromeButtonKey::ConfirmationConfirm,
                rect: self.confirmation_confirm_rect,
                text: Some(confirmation_label.unwrap_or("确认").into()),
                accessibility_label: confirmation_label.unwrap_or("确认").into(),
                icon: self
                    .confirmation_action
                    .as_ref()
                    .filter(|action| is_permanent_deletion(action))
                    .map(|_| "trash-2".into()),
                style: if self.confirmation_action.as_ref().is_some_and(is_permanent_deletion) {
                    ButtonStyle::destructive(theme.settings_theme())
                } else {
                    action_style.clone()
                },
            },
        ]);
        for (index, rect) in self.save_conflict_button_rects.iter().copied().enumerate() {
            specs.push(ChromeButtonSpec {
                key: ChromeButtonKey::SaveConflict(index),
                rect,
                text: Some(["重新载入", "保存副本", "重试", "取消"][index].into()),
                accessibility_label: ["重新载入", "保存副本", "重试", "取消"][index].into(),
                icon: None,
                style: action_style.clone(),
            });
        }

        let mut previous_buttons = std::mem::take(&mut self.chrome_buttons);
        for spec in specs.into_iter().filter(|spec| spec.rect.w > 0.0 && spec.rect.h > 0.0) {
            let previous_index = previous_buttons.iter().position(|entry| entry.key == spec.key);
            let mut widget = previous_index
                .map(|index| previous_buttons.swap_remove(index).widget)
                .unwrap_or_else(|| Button::new(spec.key.widget_id(), spec.style.clone()));
            widget.set_style(spec.style);
            widget.set_text(spec.text);
            widget.set_accessibility_label(Some(spec.accessibility_label));
            widget.set_icon(spec.icon);
            widget.set_icon_size(
                if matches!(
                    spec.key,
                    ChromeButtonKey::NavigationCollapse | ChromeButtonKey::NavigationExpand
                ) {
                    super::SIDEBAR_ICON_SIZE_LOGICAL
                } else {
                    super::NOTE_TOOL_ICON_SIZE_LOGICAL
                },
            );
            widget.set_rect(spec.rect, context);
            self.chrome_buttons.push(ChromeButton { key: spec.key, widget });
        }
    }

    pub(super) fn paint_note_toolbar(&self, context: &mut ui::PaintCtx<'_>) {
        let main_rect = self.new_note_button.main_rect();
        let new_note_rect = Rect::new(
            main_rect.x,
            main_rect.y,
            self.new_note_button.menu_rect().right() - main_rect.x,
            main_rect.h,
        );
        let segments: Vec<_> = self
            .note_toolbar_buttons
            .iter()
            .map(|button| button.rect)
            .chain([new_note_rect])
            .collect();
        ButtonStyle::from_theme(context.theme).paint_toolbar_group(context, &segments);
        self.new_note_button.paint(context);
        for index in 0..self.note_toolbar_buttons.len() {
            self.paint_chrome_button(context, ChromeButtonKey::Toolbar(index));
        }
    }

    pub(super) fn dispatch_chrome_button_event(
        &mut self,
        event: &Event,
        overlay: Option<OverlayState>,
        context: &mut EventCtx<'_>,
    ) -> (bool, Option<NotoraAction>) {
        if !matches!(
            event,
            Event::MouseMove { .. }
                | Event::MouseDown { .. }
                | Event::MouseUp { .. }
                | Event::PointerLeave
                | Event::InteractionCancel
        ) {
            return (false, None);
        }
        let pointer = event_pointer_position(event);
        let mut changed = false;
        let mut triggered_key = None;
        for index in 0..self.chrome_buttons.len() {
            let key = self.chrome_buttons[index].key;
            let allowed = self.chrome_button_available(key, overlay, pointer);
            let routed_event = if allowed { event } else { &Event::InteractionCancel };
            let result = self.chrome_buttons[index].widget.on_event(routed_event, context);
            changed |= result.is_some();
            if allowed
                && matches!(result, Some(WidgetAction::Control(ControlAction::Activated { .. })))
            {
                triggered_key = Some(key);
            }
        }
        (changed, triggered_key.and_then(|key| self.chrome_button_action(key)))
    }

    pub(super) fn chrome_button_is_capturing(&self) -> bool {
        self.chrome_buttons.iter().any(|button| button.widget.is_capturing())
    }

    pub(super) fn chrome_button_available(
        &self,
        key: ChromeButtonKey,
        overlay: Option<OverlayState>,
        pointer: Option<(f32, f32)>,
    ) -> bool {
        if overlay.is_some_and(|overlay| {
            overlay != OverlayState::None && !self.modal_input_is_ready(overlay)
        }) {
            return false;
        }
        if self.settings_overlay_open()
            || self.new_workspace_dialog_open
            || self.encrypted_note_dialog_open
            || self.new_document_menu_open
            || self.editor_pane.has_open_popup()
            || pointer.is_some_and(|(px, py)| {
                self.mindmap_style_panel_open && self.mindmap_style_panel_rect.contains(px, py)
            })
        {
            return false;
        }
        if self.save_conflict_actions.is_some() {
            return overlay.is_none_or(|overlay| overlay == OverlayState::SaveConflict)
                && matches!(key, ChromeButtonKey::SaveConflict(_));
        }
        if self.confirmation_action.is_some() {
            return overlay.is_none_or(super::is_confirmation_overlay)
                && matches!(
                    key,
                    ChromeButtonKey::ConfirmationCancel | ChromeButtonKey::ConfirmationConfirm
                );
        }
        if overlay.is_some_and(|overlay| overlay != OverlayState::None) {
            return false;
        }
        !matches!(
            key,
            ChromeButtonKey::SaveConflict(_)
                | ChromeButtonKey::ConfirmationCancel
                | ChromeButtonKey::ConfirmationConfirm
        )
    }

    fn chrome_button_action(&self, key: ChromeButtonKey) -> Option<NotoraAction> {
        match key {
            ChromeButtonKey::Toolbar(index) => {
                self.note_toolbar_buttons.get(index).map(|input| input.action.clone())
            }
            ChromeButtonKey::CompactNavigation => Some(NotoraAction::CompactNavigationRequested),
            ChromeButtonKey::CompactBack => Some(NotoraAction::CompactBackRequested),
            ChromeButtonKey::NavigationCollapse | ChromeButtonKey::NavigationExpand => {
                Some(NotoraAction::NavigationPaneVisibilityToggled)
            }
            ChromeButtonKey::Settings => Some(NotoraAction::OpenSettings),
            ChromeButtonKey::ConfirmationCancel => Some(NotoraAction::OverlayDismissed),
            ChromeButtonKey::ConfirmationConfirm => self.confirmation_action.clone(),
            ChromeButtonKey::SaveConflict(index) => {
                self.save_conflict_actions.as_ref()?.get(index).cloned()
            }
        }
    }

    pub(super) fn paint_chrome_button(&self, context: &mut ui::PaintCtx<'_>, key: ChromeButtonKey) {
        if let Some(button) = self.chrome_buttons.iter().find(|entry| entry.key == key) {
            button.widget.paint(context);
        }
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

    fn synchronize_buttons(shell: &mut NotoraShell, theme: &ui::Theme) {
        let mut measure = ui::NoopMeasure;
        let mut context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi: 1.0 };
        shell.synchronize_chrome_buttons(Some("删除"), &mut context);
    }

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
    fn toolbar_button_activates_only_when_released_inside() {
        let theme = ui::theme::test_theme();
        let mut shell = NotoraShell::new();
        let action = NotoraAction::OpenExternalFileDialogRequested;
        shell.note_toolbar_buttons = vec![RenderedToolbarButton {
            rect: Rect::new(10.0, 10.0, 80.0, 28.0),
            icon: Some("folder-open"),
            label: "打开".to_owned(),
            action: action.clone(),
        }];
        synchronize_buttons(&mut shell, &theme);
        let mut context = EventCtx::new(&theme, 1.0);
        let press = Event::MouseDown { px: 30.0, py: 20.0, button: ui::MouseButton::Left };
        assert_eq!(shell.dispatch_chrome_button_event(&press, None, &mut context).1, None);
        assert!(shell.chrome_buttons[0].widget.is_capturing());

        let move_outside = Event::MouseMove { px: 140.0, py: 90.0 };
        assert_eq!(shell.dispatch_chrome_button_event(&move_outside, None, &mut context).1, None);
        assert!(shell.chrome_buttons[0].widget.is_capturing());

        let release = Event::MouseUp { px: 140.0, py: 90.0, button: ui::MouseButton::Left };
        assert_eq!(shell.dispatch_chrome_button_event(&release, None, &mut context).1, None);
        assert!(!shell.chrome_buttons[0].widget.is_capturing());

        shell.dispatch_chrome_button_event(&press, None, &mut context);
        let release_inside = Event::MouseUp { px: 30.0, py: 20.0, button: ui::MouseButton::Left };
        assert_eq!(
            shell.dispatch_chrome_button_event(&release_inside, None, &mut context).1,
            Some(action)
        );
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
            synchronize_buttons(&mut shell, &theme);
            let mut context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);
            context.shaper = Some(&mut shaper);
            let key = if confirmation {
                ChromeButtonKey::ConfirmationConfirm
            } else {
                ChromeButtonKey::Toolbar(0)
            };
            shell.paint_chrome_button(&mut context, key);
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
        let mut shell = NotoraShell::new();
        let mut shaper = shaping::Shaper::new().expect("standard action test requires fonts");
        for action in [
            NotoraAction::TrashOperationRequested(TrashOperation::Restore {
                note_id: NoteId::generate(),
            }),
            NotoraAction::TrashRestoreWithRenamedPathConfirmed,
            NotoraAction::OverlayDismissed,
            NotoraAction::ExternalFilesClearRequested,
        ] {
            shell.note_toolbar_buttons = vec![RenderedToolbarButton {
                rect: Rect::new(10.0, 10.0, 96.0, 28.0),
                icon: Some("undo-2"),
                label: "操作".to_owned(),
                action,
            }];
            synchronize_buttons(&mut shell, &theme);
            let mut draw_list = DrawList::new();
            let mut context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);
            context.shaper = Some(&mut shaper);
            shell.paint_chrome_button(&mut context, ChromeButtonKey::Toolbar(0));
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
        synchronize_buttons(&mut shell, &theme);
        let toolbar_pointer = Event::MouseMove { px: 20.0, py: 20.0 };
        assert!(
            !shell
                .dispatch_chrome_button_event(
                    &toolbar_pointer,
                    Some(OverlayState::Settings),
                    &mut context
                )
                .0,
            "模态尚未渲染时也不能悬停背景工具栏"
        );
        shell.save_conflict_actions = Some(std::array::from_fn(|_| NotoraAction::OverlayDismissed));
        shell.save_conflict_button_rects[0] = Rect::new(100.0, 100.0, 80.0, 28.0);
        synchronize_buttons(&mut shell, &theme);
        assert!(
            !shell
                .dispatch_chrome_button_event(
                    &toolbar_pointer,
                    Some(OverlayState::SaveConflict),
                    &mut context
                )
                .0
        );
        assert!(
            shell
                .dispatch_chrome_button_event(
                    &Event::MouseMove { px: 110.0, py: 110.0 },
                    Some(OverlayState::SaveConflict),
                    &mut context
                )
                .0
        );
        assert_eq!(context.cursor_hint, Some(winit::window::CursorIcon::Pointer));
        assert_eq!(
            shell
                .dispatch_chrome_button_event(
                    &Event::MouseDown { px: 20.0, py: 20.0, button: ui::MouseButton::Left },
                    Some(OverlayState::SaveConflict),
                    &mut context,
                )
                .1,
            None
        );
        shell.save_conflict_actions = None;
        shell.confirmation_action = Some(NotoraAction::OverlayDismissed);
        shell.confirmation_confirm_rect = Rect::new(200.0, 200.0, 80.0, 28.0);
        synchronize_buttons(&mut shell, &theme);
        let confirmation = Some(OverlayState::TrashPermanentDeletionConfirmation {
            operation: TrashOperation::Empty,
        });
        shell.dispatch_chrome_button_event(&toolbar_pointer, confirmation, &mut context);
        let (changed, action) = shell.dispatch_chrome_button_event(
            &Event::MouseMove { px: 210.0, py: 210.0 },
            confirmation,
            &mut context,
        );
        assert!(changed);
        assert_eq!(action, None);
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
        synchronize_buttons(&mut shell, &theme);
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
        synchronize_buttons(&mut shell, &theme);
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
