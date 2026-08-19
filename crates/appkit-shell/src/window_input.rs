use appkit_core::edit_command::EditCommand;
use ui::core::widget::{KeyCode, Modifiers};
use winit::event::MouseScrollDelta;
use winit::keyboard::{Key, ModifiersState, NamedKey};

const WHEEL_LINES_PER_EVENT: f32 = 3.0;

/// Returns whether the operating system assigned this key event to an IME.
pub fn is_ime_process_key(logical_key: &Key) -> bool {
    matches!(logical_key, Key::Named(NamedKey::Process))
}

/// Converts winit modifier state into the UI's modifier representation.
pub fn ui_modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        shift: state.shift_key(),
        cmd: state.super_key(),
        alt: state.alt_key(),
        ctrl: state.control_key(),
    }
}

/// Converts a winit logical key and event text into a UI key code.
///
/// Preserves named Tab as a semantic command. Other keys prefer event text for
/// keyboard-layout and Shift combinations, then fall back to the logical key.
pub fn winit_key_to_keycode(logical_key: &Key, text: Option<&str>) -> Option<KeyCode> {
    if matches!(logical_key, Key::Named(NamedKey::Tab)) {
        return Some(KeyCode::Tab);
    }

    if let Some(text) = text
        && !text.is_empty()
        && let Some(character) = text.chars().next()
        && !character.is_control()
    {
        return Some(KeyCode::Char(character));
    }

    if let Key::Character(characters) = logical_key
        && let Some(character) = characters.chars().next()
    {
        return Some(KeyCode::Char(character));
    }

    match logical_key {
        Key::Named(NamedKey::Escape) => Some(KeyCode::Escape),
        Key::Named(NamedKey::Enter) => Some(KeyCode::Enter),
        Key::Named(NamedKey::Backspace) => Some(KeyCode::Backspace),
        Key::Named(NamedKey::Delete) => Some(KeyCode::Delete),
        Key::Named(NamedKey::Tab) => Some(KeyCode::Tab),
        Key::Named(NamedKey::Space) => Some(KeyCode::Char(' ')),
        Key::Named(NamedKey::ArrowUp) => Some(KeyCode::Up),
        Key::Named(NamedKey::ArrowDown) => Some(KeyCode::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(KeyCode::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KeyCode::Right),
        Key::Named(NamedKey::Home) => Some(KeyCode::Home),
        Key::Named(NamedKey::End) => Some(KeyCode::End),
        Key::Named(NamedKey::PageUp) => Some(KeyCode::PageUp),
        Key::Named(NamedKey::PageDown) => Some(KeyCode::PageDown),
        _ => None,
    }
}

/// Converts winit scroll deltas into UI pixel deltas.
pub fn scroll_delta_pixels(delta: &MouseScrollDelta, line_height: f32) -> (f32, f32) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => {
            let pixels_per_line_event = line_height * WHEEL_LINES_PER_EVENT;
            (*x * pixels_per_line_event, *y * pixels_per_line_event)
        }
        MouseScrollDelta::PixelDelta(position) => (position.x as f32, position.y as f32),
    }
}

/// Allows navigation and non-editing commands while an IME preedit is active.
pub fn command_allowed_during_preedit(preedit_text: &str, command: &EditCommand) -> bool {
    preedit_text.is_empty() || !command_mutates_document(command)
}

fn command_mutates_document(command: &EditCommand) -> bool {
    matches!(
        command,
        EditCommand::InsertChar(_)
            | EditCommand::InsertText(_)
            | EditCommand::InsertNewline
            | EditCommand::Backspace
            | EditCommand::DeleteForward
            | EditCommand::DeleteRange(_)
            | EditCommand::ReplaceRange { .. }
            | EditCommand::Cut
            | EditCommand::Paste
            | EditCommand::Undo
            | EditCommand::Redo
            | EditCommand::Tab
    )
}

#[cfg(test)]
mod tests {
    use appkit_core::edit_command::EditCommand;
    use ui::core::widget::KeyCode;
    use winit::dpi::PhysicalPosition;
    use winit::event::MouseScrollDelta;
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    use super::{
        command_allowed_during_preedit, is_ime_process_key, scroll_delta_pixels, ui_modifiers,
        winit_key_to_keycode,
    };

    #[test]
    fn identifies_ime_process_key() {
        assert!(is_ime_process_key(&Key::Named(NamedKey::Process)));
        assert!(!is_ime_process_key(&Key::Named(NamedKey::Enter)));
    }

    #[test]
    fn maps_all_supported_ui_modifiers() {
        let state = ModifiersState::SHIFT
            | ModifiersState::SUPER
            | ModifiersState::ALT
            | ModifiersState::CONTROL;

        let modifiers = ui_modifiers(state);

        assert!(modifiers.shift);
        assert!(modifiers.cmd);
        assert!(modifiers.alt);
        assert!(modifiers.ctrl);
    }

    #[test]
    fn blocks_document_mutations_during_preedit() {
        let mutating_commands = [
            EditCommand::InsertChar("a".into()),
            EditCommand::InsertText("text".into()),
            EditCommand::InsertNewline,
            EditCommand::Backspace,
            EditCommand::DeleteForward,
            EditCommand::Tab,
            EditCommand::Cut,
            EditCommand::Paste,
            EditCommand::Undo,
            EditCommand::Redo,
        ];

        for command in mutating_commands {
            assert!(!command_allowed_during_preedit("拼", &command), "{command:?}");
        }

        assert!(command_allowed_during_preedit("拼", &EditCommand::MoveLeft));
        assert!(command_allowed_during_preedit("拼", &EditCommand::ExtendRight));
        assert!(command_allowed_during_preedit("", &EditCommand::InsertChar("a".into())));
    }

    #[test]
    fn keycode_prefers_event_text_for_character_input() {
        assert_eq!(
            winit_key_to_keycode(&Key::Character("1".into()), Some("!")),
            Some(KeyCode::Char('!'))
        );
    }

    #[test]
    fn keycode_falls_back_to_logical_character_after_control_text() {
        assert_eq!(
            winit_key_to_keycode(&Key::Character("a".into()), Some("\x01")),
            Some(KeyCode::Char('a'))
        );
    }

    #[test]
    fn keycode_maps_named_control_keys() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Escape), None),
            Some(KeyCode::Escape)
        );
    }

    #[test]
    fn keycode_maps_named_tab_to_semantic_tab_despite_event_text() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Tab), Some("\t")),
            Some(KeyCode::Tab)
        );
    }

    #[test]
    fn keycode_maps_named_space_without_event_text() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Space), None),
            Some(KeyCode::Char(' '))
        );
    }

    #[test]
    fn scroll_delta_converts_line_units_to_pixels() {
        assert_eq!(
            scroll_delta_pixels(&MouseScrollDelta::LineDelta(1.0, -2.0), 10.0),
            (30.0, -60.0),
        );
    }

    #[test]
    fn scroll_delta_preserves_pixel_units() {
        assert_eq!(
            scroll_delta_pixels(
                &MouseScrollDelta::PixelDelta(PhysicalPosition::new(4.5, -7.25)),
                10.0,
            ),
            (4.5, -7.25),
        );
    }
}
