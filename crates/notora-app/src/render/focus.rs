//! 文本控件焦点、输入法状态与光标闪烁。

use std::time::Instant;

use crate::encrypted_note_dialog::EncryptedNoteDialog;
use crate::encrypted_note_unlock::EncryptedNoteUnlock;
use ui::text_box::TextBox;
use ui::{Rect, Widget};

use crate::FocusTarget;
use crate::new_workspace_dialog::NewWorkspaceDialog;

use super::{GLOBAL_SEARCH_BOX_ID, NAVIGATION_TREE_ID, NotoraShell};

impl NotoraShell {
    pub(crate) fn synchronize_focus(&mut self, focus_target: FocusTarget, _now: Instant) {
        let editor_focused_id = match focus_target {
            FocusTarget::EditorTitle => Some(ui::editor_header::EDITOR_HEADER_TITLE_ID),
            FocusTarget::EditorTag => Some(ui::tag_editor::TAG_EDITOR_INPUT_ID),
            _ => None,
        };
        self.editor_pane.set_keyboard_focus(editor_focused_id);
        let focused_text_input = if focus_target == FocusTarget::EditorTag
            && self.editor_pane.tag_editor_has_keyboard_focus()
        {
            Some(FocusTarget::EditorTag)
        } else {
            match focus_target {
                FocusTarget::NavigationSearch | FocusTarget::EditorTitle => Some(focus_target),
                FocusTarget::Editor if self.encrypted_note_unlock_open => Some(focus_target),
                FocusTarget::NavigationTree if self.navigation_tree.input().editor.is_some() => {
                    Some(FocusTarget::NavigationTree)
                }
                FocusTarget::Overlay
                    if self.new_workspace_dialog_open
                        || self.encrypted_note_dialog_open
                        || self.settings_overlay_open =>
                {
                    Some(FocusTarget::Overlay)
                }
                _ => None,
            }
        };
        self.focused_text_input = focused_text_input;
        self.search_box.set_keyboard_focus(
            (focus_target == FocusTarget::NavigationSearch).then_some(GLOBAL_SEARCH_BOX_ID),
        );
        self.navigation_tree.set_keyboard_focus(
            (focus_target == FocusTarget::NavigationTree).then_some(NAVIGATION_TREE_ID),
        );
        if let Some(unlock) = self.encrypted_note_unlock.as_mut() {
            unlock.set_keyboard_focus(
                focus_target == FocusTarget::Editor && self.encrypted_note_unlock_open,
            );
        }
    }

    fn focused_autonomous_text_input(&self) -> Option<&TextBox> {
        match self.focused_text_input? {
            FocusTarget::Overlay if self.encrypted_note_dialog_open => {
                self.encrypted_note_dialog.as_ref()?.focused_text_input()
            }
            FocusTarget::Editor if self.encrypted_note_unlock_open => {
                self.encrypted_note_unlock.as_ref()?.focused_text_input()
            }
            _ => None,
        }
    }

    pub(crate) fn focused_text_input_ime_allowed(&self) -> bool {
        self.focused_autonomous_text_input().is_none_or(TextBox::ime_allowed)
    }

    pub(crate) fn advance_text_cursor_blink(&mut self, now: Instant) -> bool {
        match self.focused_text_input {
            Some(FocusTarget::NavigationSearch) => self.search_box.advance_cursor_blink(now),
            Some(FocusTarget::NavigationTree) => {
                self.navigation_tree.advance_editor_cursor_blink(now)
            }
            Some(FocusTarget::EditorTitle | FocusTarget::EditorTag) => {
                self.editor_pane.advance_focused_cursor_blink(now)
            }
            Some(FocusTarget::Overlay) if self.encrypted_note_dialog_open => self
                .encrypted_note_dialog
                .as_mut()
                .is_some_and(|dialog| dialog.advance_cursor_blink(now)),
            Some(FocusTarget::Overlay) if self.new_workspace_dialog_open => self
                .new_workspace_dialog
                .as_mut()
                .is_some_and(|dialog| dialog.advance_cursor_blink(now)),
            Some(FocusTarget::Overlay) if self.settings_overlay_open => {
                self.settings_overlay.advance_cursor_blink(now)
            }
            Some(FocusTarget::Editor) if self.encrypted_note_unlock_open => self
                .encrypted_note_unlock
                .as_mut()
                .is_some_and(|unlock| unlock.advance_cursor_blink(now)),
            _ => false,
        }
    }

    pub(crate) fn next_text_cursor_blink_at(&self) -> Option<Instant> {
        match self.focused_text_input {
            Some(FocusTarget::NavigationSearch) => self.search_box.next_cursor_blink_at(),
            Some(FocusTarget::NavigationTree) => self.navigation_tree.next_editor_cursor_blink_at(),
            Some(FocusTarget::EditorTitle | FocusTarget::EditorTag) => {
                self.editor_pane.next_focused_cursor_blink_at()
            }
            Some(FocusTarget::Overlay) if self.encrypted_note_dialog_open => {
                self.encrypted_note_dialog.as_ref()?.focused_text_input()?.next_cursor_blink_at()
            }
            Some(FocusTarget::Overlay) if self.new_workspace_dialog_open => {
                self.new_workspace_dialog.as_ref()?.next_cursor_blink_at()
            }
            Some(FocusTarget::Overlay) if self.settings_overlay_open => {
                self.settings_overlay.next_cursor_blink_at()
            }
            Some(FocusTarget::Editor) if self.encrypted_note_unlock_open => {
                self.encrypted_note_unlock.as_ref()?.focused_text_input()?.next_cursor_blink_at()
            }
            _ => None,
        }
    }

    pub(crate) fn focused_text_input_ime_cursor_rect(&self) -> Option<Rect> {
        match self.focused_text_input? {
            FocusTarget::NavigationSearch => Some(self.search_box.ime_cursor_rect()),
            FocusTarget::EditorTitle | FocusTarget::EditorTag => {
                self.editor_pane.focused_ime_cursor_rect()
            }
            FocusTarget::NavigationTree => self.navigation_tree.ime_cursor_rect(),
            FocusTarget::Overlay if self.encrypted_note_dialog_open => {
                self.encrypted_note_dialog.as_ref().and_then(EncryptedNoteDialog::ime_cursor_rect)
            }
            FocusTarget::Overlay if self.new_workspace_dialog_open => {
                self.new_workspace_dialog.as_ref().and_then(NewWorkspaceDialog::ime_cursor_rect)
            }
            FocusTarget::Overlay if self.settings_overlay_open => {
                self.settings_overlay.ime_cursor_rect()
            }
            FocusTarget::Overlay => None,
            FocusTarget::Editor => {
                self.encrypted_note_unlock.as_ref().and_then(EncryptedNoteUnlock::ime_cursor_rect)
            }
            FocusTarget::CardList | FocusTarget::DocumentSearch => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn search_box_is_focused(&self) -> bool {
        self.search_box.is_focused()
    }

    #[cfg(test)]
    pub(crate) fn search_box_rect(&self) -> Rect {
        self.search_box.rect()
    }
}
