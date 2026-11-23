//! Keyboard input → AppCommand mapping.
//!
//! Translates winit `KeyEvent` into product and document commands.

use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::menu_handler::{AppCommand, EditorShortcut};
use appkit_core::edit_command::EditCommand;

/// Translate a winit key event into an AppCommand.
///
/// IMPORTANT: Callers MUST apply IME guard before using the result.
/// During IME composition (preedit_text is non-empty, or key is NamedKey::Process),
/// InsertChar commands from this function should be suppressed — see
/// `events::handle_keyboard` for the canonical IME guard implementation.
/// Ime::Commit is the authoritative insertion path during composition.
/// Returns `None` if the key combination is not bound to any command.
pub(crate) fn key_to_app_command(key: &Key, mods: ModifiersState) -> Option<AppCommand> {
    let ctrl = mods.control_key();
    let super_ = mods.super_key();
    let alt = mods.alt_key();
    let shift = mods.shift_key();

    match key {
        // ── Escape ──
        Key::Named(NamedKey::Escape) => Some(AppCommand::EditorShortcut(EditorShortcut::Escape)),

        // ── F3: Find next/prev ──
        Key::Named(NamedKey::F3) => {
            if shift {
                Some(AppCommand::EditorShortcut(EditorShortcut::FindPrev))
            } else {
                Some(AppCommand::EditorShortcut(EditorShortcut::FindNext))
            }
        }

        // ── Arrow keys ──
        Key::Named(NamedKey::ArrowLeft) => {
            if super_ {
                if shift {
                    Some(AppCommand::Edit(EditCommand::ExtendToLineStart))
                } else {
                    Some(AppCommand::Edit(EditCommand::MoveToLineStart))
                }
            } else if alt {
                if shift {
                    Some(AppCommand::Edit(EditCommand::ExtendWordLeft))
                } else {
                    Some(AppCommand::Edit(EditCommand::MoveWordLeft))
                }
            } else if shift {
                Some(AppCommand::Edit(EditCommand::ExtendLeft))
            } else {
                Some(AppCommand::Edit(EditCommand::MoveLeft))
            }
        }
        Key::Named(NamedKey::ArrowRight) => {
            if super_ {
                if shift {
                    Some(AppCommand::Edit(EditCommand::ExtendToLineEnd))
                } else {
                    Some(AppCommand::Edit(EditCommand::MoveToLineEnd))
                }
            } else if alt {
                if shift {
                    Some(AppCommand::Edit(EditCommand::ExtendWordRight))
                } else {
                    Some(AppCommand::Edit(EditCommand::MoveWordRight))
                }
            } else if shift {
                Some(AppCommand::Edit(EditCommand::ExtendRight))
            } else {
                Some(AppCommand::Edit(EditCommand::MoveRight))
            }
        }
        Key::Named(NamedKey::ArrowUp) => {
            if super_ {
                if shift {
                    Some(AppCommand::Edit(EditCommand::ExtendToDocStart))
                } else {
                    Some(AppCommand::Edit(EditCommand::MoveToDocStart))
                }
            } else if alt {
                Some(AppCommand::EditorShortcut(EditorShortcut::PrevChapter))
            } else if shift {
                Some(AppCommand::Edit(EditCommand::ExtendUp))
            } else {
                Some(AppCommand::Edit(EditCommand::MoveUp))
            }
        }
        Key::Named(NamedKey::ArrowDown) => {
            if super_ {
                if shift {
                    Some(AppCommand::Edit(EditCommand::ExtendToDocEnd))
                } else {
                    Some(AppCommand::Edit(EditCommand::MoveToDocEnd))
                }
            } else if alt {
                Some(AppCommand::EditorShortcut(EditorShortcut::NextChapter))
            } else if shift {
                Some(AppCommand::Edit(EditCommand::ExtendDown))
            } else {
                Some(AppCommand::Edit(EditCommand::MoveDown))
            }
        }

        // ── Home/End ──
        Key::Named(NamedKey::Home) => {
            if shift {
                Some(AppCommand::Edit(EditCommand::ExtendToLineStart))
            } else {
                Some(AppCommand::Edit(EditCommand::MoveToLineStart))
            }
        }
        Key::Named(NamedKey::End) => {
            if shift {
                Some(AppCommand::Edit(EditCommand::ExtendToLineEnd))
            } else {
                Some(AppCommand::Edit(EditCommand::MoveToLineEnd))
            }
        }

        // ── Page Up/Down ──
        Key::Named(NamedKey::PageUp) => {
            if shift {
                Some(AppCommand::Edit(EditCommand::ExtendToDocStart))
            } else {
                Some(AppCommand::Edit(EditCommand::PageUp))
            }
        }
        Key::Named(NamedKey::PageDown) => {
            if shift {
                Some(AppCommand::Edit(EditCommand::ExtendToDocEnd))
            } else {
                Some(AppCommand::Edit(EditCommand::PageDown))
            }
        }

        // ── Enter ──
        Key::Named(NamedKey::Enter) => Some(AppCommand::Edit(EditCommand::InsertNewline)),

        // ── Tab ──
        Key::Named(NamedKey::Tab) => Some(AppCommand::Edit(EditCommand::Tab)),

        // ── Backspace/Delete ──
        Key::Named(NamedKey::Backspace) => Some(AppCommand::Edit(EditCommand::Backspace)),
        Key::Named(NamedKey::Delete) => Some(AppCommand::Edit(EditCommand::DeleteForward)),

        // ── Space ──
        Key::Named(NamedKey::Space) => {
            Some(AppCommand::Edit(EditCommand::InsertChar(" ".to_string())))
        }

        // ── Character input ──
        Key::Character(c) => {
            if super_ {
                // Cmd+key shortcuts
                match c.as_str() {
                    "s" | "S" if shift => Some(AppCommand::Edit(EditCommand::SaveAs)),
                    "s" | "S" => Some(AppCommand::Edit(EditCommand::Save)),
                    "z" | "Z" if shift => Some(AppCommand::Edit(EditCommand::Redo)),
                    "z" | "Z" => Some(AppCommand::Edit(EditCommand::Undo)),
                    "a" | "A" => Some(AppCommand::Edit(EditCommand::SelectAll)),
                    "c" | "C" => Some(AppCommand::Edit(EditCommand::Copy)),
                    "x" | "X" => Some(AppCommand::Edit(EditCommand::Cut)),
                    "v" | "V" if shift => Some(AppCommand::Edit(EditCommand::PastePlainText)),
                    "v" | "V" => Some(AppCommand::Edit(EditCommand::Paste)),
                    "f" | "F" if shift || alt => {
                        Some(AppCommand::EditorShortcut(EditorShortcut::FindReplace))
                    }
                    "f" | "F" => Some(AppCommand::EditorShortcut(EditorShortcut::Find)),
                    "o" | "O" => Some(AppCommand::EditorShortcut(EditorShortcut::OpenFile)),
                    "t" | "T" if shift => {
                        Some(AppCommand::EditorShortcut(EditorShortcut::ToggleToc))
                    }
                    "t" | "T" => Some(AppCommand::EditorShortcut(EditorShortcut::NewTab)),
                    "w" | "W" => Some(AppCommand::EditorShortcut(EditorShortcut::CloseTab)),
                    "m" | "M" if shift => {
                        Some(AppCommand::EditorShortcut(EditorShortcut::ToggleView))
                    }
                    "b" | "B" => {
                        if super_ && !ctrl && !alt {
                            Some(AppCommand::EditorShortcut(EditorShortcut::ToggleSidebarPin))
                        } else {
                            None
                        }
                    }
                    "[" | "{" if shift => Some(AppCommand::EditorShortcut(EditorShortcut::PrevTab)),
                    "]" | "}" if shift => Some(AppCommand::EditorShortcut(EditorShortcut::NextTab)),
                    "[" => Some(AppCommand::EditorShortcut(EditorShortcut::NavigateBack)),
                    "]" => Some(AppCommand::EditorShortcut(EditorShortcut::NavigateForward)),
                    "1" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(0))),
                    "2" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(1))),
                    "3" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(2))),
                    "4" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(3))),
                    "5" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(4))),
                    "6" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(5))),
                    "7" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(6))),
                    "8" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(7))),
                    "9" => Some(AppCommand::EditorShortcut(EditorShortcut::SwitchTab(8))),
                    _ => None,
                }
            } else if ctrl {
                // Ctrl+key (terminal-style, mostly unused on macOS)
                match c.as_str() {
                    "v" | "V" if shift => Some(AppCommand::Edit(EditCommand::PastePlainText)),
                    "v" | "V" => Some(AppCommand::Edit(EditCommand::Paste)),
                    "z" | "Z" => Some(AppCommand::Edit(EditCommand::Undo)),
                    "y" | "Y" => Some(AppCommand::Edit(EditCommand::Redo)),
                    "a" | "A" => Some(AppCommand::Edit(EditCommand::MoveToLineStart)),
                    "e" | "E" => Some(AppCommand::Edit(EditCommand::MoveToLineEnd)),
                    "h" | "H" => Some(AppCommand::Edit(EditCommand::Backspace)),
                    "d" | "D" => Some(AppCommand::Edit(EditCommand::DeleteForward)),
                    "k" => None, // kill line — future
                    _ => None,
                }
            } else {
                Some(AppCommand::Edit(EditCommand::InsertChar(c.to_string())))
            }
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key, NamedKey};

    fn no_mods() -> ModifiersState {
        ModifiersState::empty()
    }
    fn cmd() -> ModifiersState {
        ModifiersState::SUPER
    }
    fn alt() -> ModifiersState {
        ModifiersState::ALT
    }
    fn ctrl() -> ModifiersState {
        ModifiersState::CONTROL
    }
    fn ctrl_shift() -> ModifiersState {
        ModifiersState::CONTROL | ModifiersState::SHIFT
    }
    fn shift() -> ModifiersState {
        ModifiersState::SHIFT
    }
    fn cmd_shift() -> ModifiersState {
        ModifiersState::SUPER | ModifiersState::SHIFT
    }

    // ── Arrow keys ──

    #[test]
    fn arrow_left_basic() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowLeft), no_mods()),
            Some(AppCommand::Edit(EditCommand::MoveLeft))
        );
    }

    #[test]
    fn arrow_right_basic() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowRight), no_mods()),
            Some(AppCommand::Edit(EditCommand::MoveRight))
        );
    }

    #[test]
    fn arrow_up_basic() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowUp), no_mods()),
            Some(AppCommand::Edit(EditCommand::MoveUp))
        );
    }

    #[test]
    fn arrow_down_basic() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowDown), no_mods()),
            Some(AppCommand::Edit(EditCommand::MoveDown))
        );
    }

    #[test]
    fn cmd_arrow_left_line_start() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowLeft), cmd()),
            Some(AppCommand::Edit(EditCommand::MoveToLineStart))
        );
    }

    #[test]
    fn cmd_arrow_right_line_end() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowRight), cmd()),
            Some(AppCommand::Edit(EditCommand::MoveToLineEnd))
        );
    }

    #[test]
    fn alt_arrow_left_word() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowLeft), alt()),
            Some(AppCommand::Edit(EditCommand::MoveWordLeft))
        );
    }

    #[test]
    fn alt_arrow_right_word() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowRight), alt()),
            Some(AppCommand::Edit(EditCommand::MoveWordRight))
        );
    }

    #[test]
    fn cmd_arrow_up_doc_start() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowUp), cmd()),
            Some(AppCommand::Edit(EditCommand::MoveToDocStart))
        );
    }

    #[test]
    fn cmd_arrow_down_doc_end() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowDown), cmd()),
            Some(AppCommand::Edit(EditCommand::MoveToDocEnd))
        );
    }

    // ── Home/End ──

    #[test]
    fn home_key() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::Home), no_mods()),
            Some(AppCommand::Edit(EditCommand::MoveToLineStart))
        );
    }

    #[test]
    fn end_key() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::End), no_mods()),
            Some(AppCommand::Edit(EditCommand::MoveToLineEnd))
        );
    }

    // ── Page Up/Down ──

    #[test]
    fn page_up() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::PageUp), no_mods()),
            Some(AppCommand::Edit(EditCommand::PageUp))
        );
    }

    #[test]
    fn page_down() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::PageDown), no_mods()),
            Some(AppCommand::Edit(EditCommand::PageDown))
        );
    }

    // ── Editing keys ──

    #[test]
    fn enter_inserts_newline() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::Enter), no_mods()),
            Some(AppCommand::Edit(EditCommand::InsertNewline))
        );
    }

    #[test]
    fn tab_key() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::Tab), no_mods()),
            Some(AppCommand::Edit(EditCommand::Tab))
        );
    }

    #[test]
    fn backspace() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::Backspace), no_mods()),
            Some(AppCommand::Edit(EditCommand::Backspace))
        );
    }

    #[test]
    fn delete_forward() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::Delete), no_mods()),
            Some(AppCommand::Edit(EditCommand::DeleteForward))
        );
    }

    #[test]
    fn escape() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::Escape), no_mods()),
            Some(AppCommand::EditorShortcut(EditorShortcut::Escape))
        );
    }

    // ── Character input ──

    #[test]
    fn char_input_basic() {
        let key = Key::Character("a".into());
        assert_eq!(
            key_to_app_command(&key, no_mods()),
            Some(AppCommand::Edit(EditCommand::InsertChar("a".into())))
        );
    }

    #[test]
    fn char_input_cjk() {
        let key = Key::Character("世".into());
        assert_eq!(
            key_to_app_command(&key, no_mods()),
            Some(AppCommand::Edit(EditCommand::InsertChar("世".into())))
        );
    }

    #[test]
    fn char_input_emoji() {
        let key = Key::Character("🌏".into());
        assert_eq!(
            key_to_app_command(&key, no_mods()),
            Some(AppCommand::Edit(EditCommand::InsertChar("🌏".into())))
        );
    }

    #[test]
    fn named_space_inserts_space() {
        let key = Key::Named(NamedKey::Space);
        assert_eq!(
            key_to_app_command(&key, no_mods()),
            Some(AppCommand::Edit(EditCommand::InsertChar(" ".into())))
        );
    }

    // ── Cmd shortcuts (macOS) ──

    #[test]
    fn cmd_z_undo() {
        let key = Key::Character("z".into());
        assert_eq!(key_to_app_command(&key, cmd()), Some(AppCommand::Edit(EditCommand::Undo)));
    }

    #[test]
    fn cmd_shift_z_redo() {
        let key = Key::Character("z".into());
        assert_eq!(
            key_to_app_command(&key, cmd_shift()),
            Some(AppCommand::Edit(EditCommand::Redo))
        );
    }

    #[test]
    fn uppercase_logical_keys_preserve_existing_shortcut_commands() {
        let shortcuts = [
            ("S", cmd_shift(), AppCommand::Edit(EditCommand::SaveAs)),
            ("Z", cmd_shift(), AppCommand::Edit(EditCommand::Redo)),
            ("F", cmd_shift(), AppCommand::EditorShortcut(EditorShortcut::FindReplace)),
            ("T", cmd_shift(), AppCommand::EditorShortcut(EditorShortcut::ToggleToc)),
            ("S", cmd(), AppCommand::Edit(EditCommand::Save)),
            ("Z", cmd(), AppCommand::Edit(EditCommand::Undo)),
            ("A", cmd(), AppCommand::Edit(EditCommand::SelectAll)),
            ("C", cmd(), AppCommand::Edit(EditCommand::Copy)),
            ("X", cmd(), AppCommand::Edit(EditCommand::Cut)),
            ("F", cmd(), AppCommand::EditorShortcut(EditorShortcut::Find)),
            ("O", cmd(), AppCommand::EditorShortcut(EditorShortcut::OpenFile)),
            ("T", cmd(), AppCommand::EditorShortcut(EditorShortcut::NewTab)),
            ("W", cmd(), AppCommand::EditorShortcut(EditorShortcut::CloseTab)),
            ("B", cmd(), AppCommand::EditorShortcut(EditorShortcut::ToggleSidebarPin)),
            ("Z", ctrl(), AppCommand::Edit(EditCommand::Undo)),
            ("Y", ctrl(), AppCommand::Edit(EditCommand::Redo)),
            ("A", ctrl(), AppCommand::Edit(EditCommand::MoveToLineStart)),
            ("E", ctrl(), AppCommand::Edit(EditCommand::MoveToLineEnd)),
            ("H", ctrl(), AppCommand::Edit(EditCommand::Backspace)),
            ("D", ctrl(), AppCommand::Edit(EditCommand::DeleteForward)),
        ];

        for (character, modifiers, expected_command) in shortcuts {
            let key = Key::Character(character.into());
            assert_eq!(key_to_app_command(&key, modifiers), Some(expected_command), "{key:?}");
        }
    }

    #[test]
    fn uppercase_logical_keys_preserve_inserted_text_without_command_modifiers() {
        for character in ["Z", "S", "F", "T", "A", "É", "世"] {
            let key = Key::Character(character.into());
            for modifiers in [no_mods(), ModifiersState::SHIFT] {
                assert_eq!(
                    key_to_app_command(&key, modifiers),
                    Some(AppCommand::Edit(EditCommand::InsertChar(character.into())))
                );
            }
        }
    }

    #[test]
    fn cmd_a_select_all() {
        let key = Key::Character("a".into());
        assert_eq!(key_to_app_command(&key, cmd()), Some(AppCommand::Edit(EditCommand::SelectAll)));
    }

    #[test]
    fn cmd_c_copy() {
        let key = Key::Character("c".into());
        assert_eq!(key_to_app_command(&key, cmd()), Some(AppCommand::Edit(EditCommand::Copy)));
    }

    #[test]
    fn cmd_x_cut() {
        let key = Key::Character("x".into());
        assert_eq!(key_to_app_command(&key, cmd()), Some(AppCommand::Edit(EditCommand::Cut)));
    }

    #[test]
    fn cmd_v_paste() {
        let key = Key::Character("v".into());
        assert_eq!(key_to_app_command(&key, cmd()), Some(AppCommand::Edit(EditCommand::Paste)));
    }

    #[test]
    fn cmd_uppercase_v_without_shift_pastes() {
        let key = Key::Character("V".into());
        assert_eq!(key_to_app_command(&key, cmd()), Some(AppCommand::Edit(EditCommand::Paste)));
    }

    #[test]
    fn cmd_shift_v_pastes_plain_text() {
        let key = Key::Character("v".into());
        assert_eq!(
            key_to_app_command(&key, cmd_shift()),
            Some(AppCommand::Edit(EditCommand::PastePlainText))
        );
    }

    #[test]
    fn cmd_shift_uppercase_v_pastes_plain_text() {
        let key = Key::Character("V".into());
        assert_eq!(
            key_to_app_command(&key, cmd_shift()),
            Some(AppCommand::Edit(EditCommand::PastePlainText))
        );
    }

    #[test]
    fn ctrl_v_pastes() {
        let key = Key::Character("v".into());
        assert_eq!(key_to_app_command(&key, ctrl()), Some(AppCommand::Edit(EditCommand::Paste)));
    }

    #[test]
    fn ctrl_uppercase_v_without_shift_pastes() {
        let key = Key::Character("V".into());
        assert_eq!(key_to_app_command(&key, ctrl()), Some(AppCommand::Edit(EditCommand::Paste)));
    }

    #[test]
    fn ctrl_shift_v_pastes_plain_text() {
        let key = Key::Character("v".into());
        assert_eq!(
            key_to_app_command(&key, ctrl_shift()),
            Some(AppCommand::Edit(EditCommand::PastePlainText))
        );
    }

    #[test]
    fn ctrl_shift_uppercase_v_pastes_plain_text() {
        let key = Key::Character("V".into());
        assert_eq!(
            key_to_app_command(&key, ctrl_shift()),
            Some(AppCommand::Edit(EditCommand::PastePlainText))
        );
    }

    #[test]
    fn unmodified_v_preserves_character_case() {
        for text in ["v", "V"] {
            let key = Key::Character(text.into());
            assert_eq!(
                key_to_app_command(&key, no_mods()),
                Some(AppCommand::Edit(EditCommand::InsertChar(text.into())))
            );
        }
    }

    #[test]
    fn cmd_unknown_returns_none() {
        let key = Key::Character("q".into());
        assert_eq!(key_to_app_command(&key, cmd()), None);
    }

    // ── Ctrl shortcuts (terminal-style) ──

    #[test]
    fn ctrl_z_undo() {
        let key = Key::Character("z".into());
        assert_eq!(key_to_app_command(&key, ctrl()), Some(AppCommand::Edit(EditCommand::Undo)));
    }

    #[test]
    fn ctrl_y_redo() {
        let key = Key::Character("y".into());
        assert_eq!(key_to_app_command(&key, ctrl()), Some(AppCommand::Edit(EditCommand::Redo)));
    }

    #[test]
    fn ctrl_a_line_start() {
        let key = Key::Character("a".into());
        assert_eq!(
            key_to_app_command(&key, ctrl()),
            Some(AppCommand::Edit(EditCommand::MoveToLineStart))
        );
    }

    #[test]
    fn ctrl_e_line_end() {
        let key = Key::Character("e".into());
        assert_eq!(
            key_to_app_command(&key, ctrl()),
            Some(AppCommand::Edit(EditCommand::MoveToLineEnd))
        );
    }

    #[test]
    fn ctrl_h_backspace() {
        let key = Key::Character("h".into());
        assert_eq!(
            key_to_app_command(&key, ctrl()),
            Some(AppCommand::Edit(EditCommand::Backspace))
        );
    }

    #[test]
    fn ctrl_d_delete() {
        let key = Key::Character("d".into());
        assert_eq!(
            key_to_app_command(&key, ctrl()),
            Some(AppCommand::Edit(EditCommand::DeleteForward))
        );
    }

    // ── Unknown keys ──

    #[test]
    fn function_key_returns_none() {
        assert_eq!(key_to_app_command(&Key::Named(NamedKey::F1), no_mods()), None);
    }

    #[test]
    fn shift_alone_with_char_is_insert() {
        let key = Key::Character("A".into());
        assert_eq!(
            key_to_app_command(&key, shift()),
            Some(AppCommand::Edit(EditCommand::InsertChar("A".into())))
        );
    }

    // ── Shift+Arrow selection extension ──

    #[test]
    fn shift_arrow_left() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowLeft), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendLeft))
        );
    }

    #[test]
    fn shift_arrow_right() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowRight), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendRight))
        );
    }

    #[test]
    fn shift_arrow_up() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowUp), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendUp))
        );
    }

    #[test]
    fn shift_arrow_down() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowDown), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendDown))
        );
    }

    #[test]
    fn shift_alt_arrow_left() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowLeft), alt() | shift()),
            Some(AppCommand::Edit(EditCommand::ExtendWordLeft))
        );
    }

    #[test]
    fn shift_alt_arrow_right() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowRight), alt() | shift()),
            Some(AppCommand::Edit(EditCommand::ExtendWordRight))
        );
    }

    #[test]
    fn shift_cmd_arrow_left() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowLeft), cmd() | shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToLineStart))
        );
    }

    #[test]
    fn shift_cmd_arrow_right() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowRight), cmd() | shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToLineEnd))
        );
    }

    #[test]
    fn shift_cmd_arrow_up() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowUp), cmd() | shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToDocStart))
        );
    }

    #[test]
    fn shift_cmd_arrow_down() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::ArrowDown), cmd() | shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToDocEnd))
        );
    }

    // ── Shift+Home/End ──

    #[test]
    fn shift_home_extends_to_line_start() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::Home), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToLineStart))
        );
    }

    #[test]
    fn shift_end_extends_to_line_end() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::End), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToLineEnd))
        );
    }

    // ── Shift+Page Up/Down ──

    #[test]
    fn shift_page_up_extends_to_doc_start() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::PageUp), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToDocStart))
        );
    }

    #[test]
    fn shift_page_down_extends_to_doc_end() {
        assert_eq!(
            key_to_app_command(&Key::Named(NamedKey::PageDown), shift()),
            Some(AppCommand::Edit(EditCommand::ExtendToDocEnd))
        );
    }

    // ── Navigation history ──

    #[test]
    fn cmd_bracket_left_is_navigate_back() {
        assert_eq!(
            key_to_app_command(&Key::Character("[".into()), cmd()),
            Some(AppCommand::EditorShortcut(EditorShortcut::NavigateBack))
        );
    }

    #[test]
    fn cmd_bracket_right_is_navigate_forward() {
        assert_eq!(
            key_to_app_command(&Key::Character("]".into()), cmd()),
            Some(AppCommand::EditorShortcut(EditorShortcut::NavigateForward))
        );
    }

    #[test]
    fn cmd_shift_bracket_left_is_prev_tab() {
        assert_eq!(
            key_to_app_command(&Key::Character("[".into()), cmd_shift()),
            Some(AppCommand::EditorShortcut(EditorShortcut::PrevTab))
        );
    }

    #[test]
    fn cmd_shift_bracket_right_is_next_tab() {
        assert_eq!(
            key_to_app_command(&Key::Character("]".into()), cmd_shift()),
            Some(AppCommand::EditorShortcut(EditorShortcut::NextTab))
        );
    }
}
