//! Keyboard input → EditCommand mapping.
//!
//! Translates winit `KeyEvent` into editor commands.

use winit::keyboard::{Key, ModifiersState, NamedKey};

pub use appkit_core::edit_command::EditCommand;

/// Map a key + modifiers to an `EditCommand`.
///

/// Translate a winit key event into an EditCommand.
///
/// IMPORTANT: Callers MUST apply IME guard before using the result.
/// During IME composition (preedit_text is non-empty, or key is NamedKey::Process),
/// InsertChar commands from this function should be suppressed — see
/// `events::handle_keyboard` for the canonical IME guard implementation.
/// Ime::Commit is the authoritative insertion path during composition.
/// Returns `None` if the key combination is not bound to any command.
pub fn key_to_command(key: &Key, mods: ModifiersState) -> Option<EditCommand> {
    eprintln!(
        "[key_to_command] key={key:?} ctrl={} super={} alt={} shift={}",
        mods.control_key(),
        mods.super_key(),
        mods.alt_key(),
        mods.shift_key()
    );
    let ctrl = mods.control_key();
    let super_ = mods.super_key();
    let alt = mods.alt_key();
    let shift = mods.shift_key();

    match key {
        // ── Escape ──
        Key::Named(NamedKey::Escape) => Some(EditCommand::Escape),

        // ── F3: Find next/prev ──
        Key::Named(NamedKey::F3) => {
            if shift {
                Some(EditCommand::FindPrev)
            } else {
                Some(EditCommand::FindNext)
            }
        }

        // ── Arrow keys ──
        Key::Named(NamedKey::ArrowLeft) => {
            if super_ {
                if shift {
                    Some(EditCommand::ExtendToLineStart)
                } else {
                    Some(EditCommand::MoveToLineStart)
                }
            } else if alt {
                if shift {
                    Some(EditCommand::ExtendWordLeft)
                } else {
                    Some(EditCommand::MoveWordLeft)
                }
            } else if shift {
                Some(EditCommand::ExtendLeft)
            } else {
                Some(EditCommand::MoveLeft)
            }
        }
        Key::Named(NamedKey::ArrowRight) => {
            if super_ {
                if shift {
                    Some(EditCommand::ExtendToLineEnd)
                } else {
                    Some(EditCommand::MoveToLineEnd)
                }
            } else if alt {
                if shift {
                    Some(EditCommand::ExtendWordRight)
                } else {
                    Some(EditCommand::MoveWordRight)
                }
            } else if shift {
                Some(EditCommand::ExtendRight)
            } else {
                Some(EditCommand::MoveRight)
            }
        }
        Key::Named(NamedKey::ArrowUp) => {
            if super_ {
                if shift {
                    Some(EditCommand::ExtendToDocStart)
                } else {
                    Some(EditCommand::MoveToDocStart)
                }
            } else if alt {
                Some(EditCommand::PrevChapter)
            } else if shift {
                Some(EditCommand::ExtendUp)
            } else {
                Some(EditCommand::MoveUp)
            }
        }
        Key::Named(NamedKey::ArrowDown) => {
            if super_ {
                if shift {
                    Some(EditCommand::ExtendToDocEnd)
                } else {
                    Some(EditCommand::MoveToDocEnd)
                }
            } else if alt {
                Some(EditCommand::NextChapter)
            } else if shift {
                Some(EditCommand::ExtendDown)
            } else {
                Some(EditCommand::MoveDown)
            }
        }

        // ── Home/End ──
        Key::Named(NamedKey::Home) => {
            if shift {
                Some(EditCommand::ExtendToLineStart)
            } else {
                Some(EditCommand::MoveToLineStart)
            }
        }
        Key::Named(NamedKey::End) => {
            if shift {
                Some(EditCommand::ExtendToLineEnd)
            } else {
                Some(EditCommand::MoveToLineEnd)
            }
        }

        // ── Page Up/Down ──
        Key::Named(NamedKey::PageUp) => {
            if shift {
                Some(EditCommand::ExtendToDocStart)
            } else {
                Some(EditCommand::PageUp)
            }
        }
        Key::Named(NamedKey::PageDown) => {
            if shift {
                Some(EditCommand::ExtendToDocEnd)
            } else {
                Some(EditCommand::PageDown)
            }
        }

        // ── Enter ──
        Key::Named(NamedKey::Enter) => Some(EditCommand::InsertNewline),

        // ── Tab ──
        Key::Named(NamedKey::Tab) => Some(EditCommand::Tab),

        // ── Backspace/Delete ──
        Key::Named(NamedKey::Backspace) => Some(EditCommand::Backspace),
        Key::Named(NamedKey::Delete) => Some(EditCommand::DeleteForward),

        // ── Space ──
        Key::Named(NamedKey::Space) => Some(EditCommand::InsertChar(" ".to_string())),

        // ── Character input ──
        Key::Character(c) => {
            if super_ {
                // Cmd+key shortcuts
                match c.as_str() {
                    "s" if shift => Some(EditCommand::SaveAs),
                    "s" => Some(EditCommand::Save),
                    "z" if shift => Some(EditCommand::Redo),
                    "z" => Some(EditCommand::Undo),
                    "a" => Some(EditCommand::SelectAll),
                    "c" => Some(EditCommand::Copy),
                    "x" => Some(EditCommand::Cut),
                    "v" | "V" if shift => Some(EditCommand::PastePlainText),
                    "v" => Some(EditCommand::Paste),
                    "f" if shift || alt => Some(EditCommand::FindReplace),
                    "f" => Some(EditCommand::Find),
                    "o" => Some(EditCommand::OpenFile),
                    "t" if shift => Some(EditCommand::ToggleToc),
                    "t" => Some(EditCommand::NewTab),
                    "w" => Some(EditCommand::CloseTab),
                    "m" | "M" if shift => Some(EditCommand::ToggleView),
                    "b" => {
                        if super_ && !ctrl && !alt {
                            Some(EditCommand::ToggleSidebarPin)
                        } else {
                            None
                        }
                    }
                    "[" | "{" if shift => Some(EditCommand::PrevTab),
                    "]" | "}" if shift => Some(EditCommand::NextTab),
                    "[" => Some(EditCommand::NavigateBack),
                    "]" => Some(EditCommand::NavigateForward),
                    "1" => Some(EditCommand::SwitchTab(0)),
                    "2" => Some(EditCommand::SwitchTab(1)),
                    "3" => Some(EditCommand::SwitchTab(2)),
                    "4" => Some(EditCommand::SwitchTab(3)),
                    "5" => Some(EditCommand::SwitchTab(4)),
                    "6" => Some(EditCommand::SwitchTab(5)),
                    "7" => Some(EditCommand::SwitchTab(6)),
                    "8" => Some(EditCommand::SwitchTab(7)),
                    "9" => Some(EditCommand::SwitchTab(8)),
                    _ => None,
                }
            } else if ctrl {
                // Ctrl+key (terminal-style, mostly unused on macOS)
                match c.as_str() {
                    "v" | "V" if shift => Some(EditCommand::PastePlainText),
                    "v" => Some(EditCommand::Paste),
                    "z" => Some(EditCommand::Undo),
                    "y" => Some(EditCommand::Redo),
                    "a" => Some(EditCommand::MoveToLineStart),
                    "e" => Some(EditCommand::MoveToLineEnd),
                    "h" => Some(EditCommand::Backspace),
                    "d" => Some(EditCommand::DeleteForward),
                    "k" => None, // kill line — future
                    _ => None,
                }
            } else {
                Some(EditCommand::InsertChar(c.to_string()))
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
            key_to_command(&Key::Named(NamedKey::ArrowLeft), no_mods()),
            Some(EditCommand::MoveLeft)
        );
    }

    #[test]
    fn arrow_right_basic() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowRight), no_mods()),
            Some(EditCommand::MoveRight)
        );
    }

    #[test]
    fn arrow_up_basic() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowUp), no_mods()),
            Some(EditCommand::MoveUp)
        );
    }

    #[test]
    fn arrow_down_basic() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowDown), no_mods()),
            Some(EditCommand::MoveDown)
        );
    }

    #[test]
    fn cmd_arrow_left_line_start() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowLeft), cmd()),
            Some(EditCommand::MoveToLineStart)
        );
    }

    #[test]
    fn cmd_arrow_right_line_end() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowRight), cmd()),
            Some(EditCommand::MoveToLineEnd)
        );
    }

    #[test]
    fn alt_arrow_left_word() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowLeft), alt()),
            Some(EditCommand::MoveWordLeft)
        );
    }

    #[test]
    fn alt_arrow_right_word() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowRight), alt()),
            Some(EditCommand::MoveWordRight)
        );
    }

    #[test]
    fn cmd_arrow_up_doc_start() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowUp), cmd()),
            Some(EditCommand::MoveToDocStart)
        );
    }

    #[test]
    fn cmd_arrow_down_doc_end() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowDown), cmd()),
            Some(EditCommand::MoveToDocEnd)
        );
    }

    // ── Home/End ──

    #[test]
    fn home_key() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::Home), no_mods()),
            Some(EditCommand::MoveToLineStart)
        );
    }

    #[test]
    fn end_key() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::End), no_mods()),
            Some(EditCommand::MoveToLineEnd)
        );
    }

    // ── Page Up/Down ──

    #[test]
    fn page_up() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::PageUp), no_mods()),
            Some(EditCommand::PageUp)
        );
    }

    #[test]
    fn page_down() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::PageDown), no_mods()),
            Some(EditCommand::PageDown)
        );
    }

    // ── Editing keys ──

    #[test]
    fn enter_inserts_newline() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::Enter), no_mods()),
            Some(EditCommand::InsertNewline)
        );
    }

    #[test]
    fn tab_key() {
        assert_eq!(key_to_command(&Key::Named(NamedKey::Tab), no_mods()), Some(EditCommand::Tab));
    }

    #[test]
    fn backspace() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::Backspace), no_mods()),
            Some(EditCommand::Backspace)
        );
    }

    #[test]
    fn delete_forward() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::Delete), no_mods()),
            Some(EditCommand::DeleteForward)
        );
    }

    #[test]
    fn escape() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::Escape), no_mods()),
            Some(EditCommand::Escape)
        );
    }

    // ── Character input ──

    #[test]
    fn char_input_basic() {
        let key = Key::Character("a".into());
        assert_eq!(key_to_command(&key, no_mods()), Some(EditCommand::InsertChar("a".into())));
    }

    #[test]
    fn char_input_cjk() {
        let key = Key::Character("世".into());
        assert_eq!(key_to_command(&key, no_mods()), Some(EditCommand::InsertChar("世".into())));
    }

    #[test]
    fn char_input_emoji() {
        let key = Key::Character("🌏".into());
        assert_eq!(key_to_command(&key, no_mods()), Some(EditCommand::InsertChar("🌏".into())));
    }

    #[test]
    fn named_space_inserts_space() {
        let key = Key::Named(NamedKey::Space);
        assert_eq!(key_to_command(&key, no_mods()), Some(EditCommand::InsertChar(" ".into())));
    }

    // ── Cmd shortcuts (macOS) ──

    #[test]
    fn cmd_z_undo() {
        let key = Key::Character("z".into());
        assert_eq!(key_to_command(&key, cmd()), Some(EditCommand::Undo));
    }

    #[test]
    fn cmd_shift_z_redo() {
        let key = Key::Character("z".into());
        assert_eq!(key_to_command(&key, cmd_shift()), Some(EditCommand::Redo));
    }

    #[test]
    fn cmd_a_select_all() {
        let key = Key::Character("a".into());
        assert_eq!(key_to_command(&key, cmd()), Some(EditCommand::SelectAll));
    }

    #[test]
    fn cmd_c_copy() {
        let key = Key::Character("c".into());
        assert_eq!(key_to_command(&key, cmd()), Some(EditCommand::Copy));
    }

    #[test]
    fn cmd_x_cut() {
        let key = Key::Character("x".into());
        assert_eq!(key_to_command(&key, cmd()), Some(EditCommand::Cut));
    }

    #[test]
    fn cmd_v_paste() {
        let key = Key::Character("v".into());
        assert_eq!(key_to_command(&key, cmd()), Some(EditCommand::Paste));
    }

    #[test]
    fn cmd_shift_v_pastes_plain_text() {
        let key = Key::Character("v".into());
        assert_eq!(key_to_command(&key, cmd_shift()), Some(EditCommand::PastePlainText));
    }

    #[test]
    fn cmd_shift_uppercase_v_pastes_plain_text() {
        let key = Key::Character("V".into());
        assert_eq!(key_to_command(&key, cmd_shift()), Some(EditCommand::PastePlainText));
    }

    #[test]
    fn ctrl_v_pastes() {
        let key = Key::Character("v".into());
        assert_eq!(key_to_command(&key, ctrl()), Some(EditCommand::Paste));
    }

    #[test]
    fn ctrl_shift_v_pastes_plain_text() {
        let key = Key::Character("v".into());
        assert_eq!(key_to_command(&key, ctrl_shift()), Some(EditCommand::PastePlainText));
    }

    #[test]
    fn ctrl_shift_uppercase_v_pastes_plain_text() {
        let key = Key::Character("V".into());
        assert_eq!(key_to_command(&key, ctrl_shift()), Some(EditCommand::PastePlainText));
    }

    #[test]
    fn unmodified_v_preserves_character_case() {
        for text in ["v", "V"] {
            let key = Key::Character(text.into());
            assert_eq!(key_to_command(&key, no_mods()), Some(EditCommand::InsertChar(text.into())));
        }
    }

    #[test]
    fn cmd_unknown_returns_none() {
        let key = Key::Character("q".into());
        assert_eq!(key_to_command(&key, cmd()), None);
    }

    // ── Ctrl shortcuts (terminal-style) ──

    #[test]
    fn ctrl_z_undo() {
        let key = Key::Character("z".into());
        assert_eq!(key_to_command(&key, ctrl()), Some(EditCommand::Undo));
    }

    #[test]
    fn ctrl_y_redo() {
        let key = Key::Character("y".into());
        assert_eq!(key_to_command(&key, ctrl()), Some(EditCommand::Redo));
    }

    #[test]
    fn ctrl_a_line_start() {
        let key = Key::Character("a".into());
        assert_eq!(key_to_command(&key, ctrl()), Some(EditCommand::MoveToLineStart));
    }

    #[test]
    fn ctrl_e_line_end() {
        let key = Key::Character("e".into());
        assert_eq!(key_to_command(&key, ctrl()), Some(EditCommand::MoveToLineEnd));
    }

    #[test]
    fn ctrl_h_backspace() {
        let key = Key::Character("h".into());
        assert_eq!(key_to_command(&key, ctrl()), Some(EditCommand::Backspace));
    }

    #[test]
    fn ctrl_d_delete() {
        let key = Key::Character("d".into());
        assert_eq!(key_to_command(&key, ctrl()), Some(EditCommand::DeleteForward));
    }

    // ── Unknown keys ──

    #[test]
    fn function_key_returns_none() {
        assert_eq!(key_to_command(&Key::Named(NamedKey::F1), no_mods()), None);
    }

    #[test]
    fn shift_alone_with_char_is_insert() {
        let key = Key::Character("A".into());
        assert_eq!(key_to_command(&key, shift()), Some(EditCommand::InsertChar("A".into())));
    }

    // ── Shift+Arrow selection extension ──

    #[test]
    fn shift_arrow_left() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowLeft), shift()),
            Some(EditCommand::ExtendLeft)
        );
    }

    #[test]
    fn shift_arrow_right() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowRight), shift()),
            Some(EditCommand::ExtendRight)
        );
    }

    #[test]
    fn shift_arrow_up() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowUp), shift()),
            Some(EditCommand::ExtendUp)
        );
    }

    #[test]
    fn shift_arrow_down() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowDown), shift()),
            Some(EditCommand::ExtendDown)
        );
    }

    #[test]
    fn shift_alt_arrow_left() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowLeft), alt() | shift()),
            Some(EditCommand::ExtendWordLeft)
        );
    }

    #[test]
    fn shift_alt_arrow_right() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowRight), alt() | shift()),
            Some(EditCommand::ExtendWordRight)
        );
    }

    #[test]
    fn shift_cmd_arrow_left() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowLeft), cmd() | shift()),
            Some(EditCommand::ExtendToLineStart)
        );
    }

    #[test]
    fn shift_cmd_arrow_right() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowRight), cmd() | shift()),
            Some(EditCommand::ExtendToLineEnd)
        );
    }

    #[test]
    fn shift_cmd_arrow_up() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowUp), cmd() | shift()),
            Some(EditCommand::ExtendToDocStart)
        );
    }

    #[test]
    fn shift_cmd_arrow_down() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::ArrowDown), cmd() | shift()),
            Some(EditCommand::ExtendToDocEnd)
        );
    }

    // ── Shift+Home/End ──

    #[test]
    fn shift_home_extends_to_line_start() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::Home), shift()),
            Some(EditCommand::ExtendToLineStart)
        );
    }

    #[test]
    fn shift_end_extends_to_line_end() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::End), shift()),
            Some(EditCommand::ExtendToLineEnd)
        );
    }

    // ── Shift+Page Up/Down ──

    #[test]
    fn shift_page_up_extends_to_doc_start() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::PageUp), shift()),
            Some(EditCommand::ExtendToDocStart)
        );
    }

    #[test]
    fn shift_page_down_extends_to_doc_end() {
        assert_eq!(
            key_to_command(&Key::Named(NamedKey::PageDown), shift()),
            Some(EditCommand::ExtendToDocEnd)
        );
    }

    // ── Navigation history ──

    #[test]
    fn cmd_bracket_left_is_navigate_back() {
        assert_eq!(
            key_to_command(&Key::Character("[".into()), cmd()),
            Some(EditCommand::NavigateBack)
        );
    }

    #[test]
    fn cmd_bracket_right_is_navigate_forward() {
        assert_eq!(
            key_to_command(&Key::Character("]".into()), cmd()),
            Some(EditCommand::NavigateForward)
        );
    }

    #[test]
    fn cmd_shift_bracket_left_is_prev_tab() {
        assert_eq!(
            key_to_command(&Key::Character("[".into()), cmd_shift()),
            Some(EditCommand::PrevTab)
        );
    }

    #[test]
    fn cmd_shift_bracket_right_is_next_tab() {
        assert_eq!(
            key_to_command(&Key::Character("]".into()), cmd_shift()),
            Some(EditCommand::NextTab)
        );
    }
}
