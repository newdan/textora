use super::*;
use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext};
use ui::core::widget::KeyCode;

const ACTIVE_EDITOR_CONTEXT: EditorInputContext =
    EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };

fn set_markdown_cursor(app: &mut App, cursor_byte: usize) {
    let Some(mut tab) = app.active_tab_session_mut() else {
        panic!("Markdown editor should be active");
    };
    tab.document.cursor_move_to_offset(cursor_byte);
    tab.send_message(ui::plugin::PluginMessage::SetCursorByte(cursor_byte));
}

fn active_markdown_source(app: &App) -> String {
    app.active_tab_session().expect("Markdown editor should remain active").document.full_text()
}

fn active_markdown_cursor(app: &App) -> usize {
    app.active_tab_session()
        .expect("Markdown editor should remain active")
        .document
        .cursor_offset()
        .0
}

#[test]
fn markdown_table_tabs_navigate_cells_respect_ime_and_preserve_enter_navigation() {
    let directory = tempfile::tempdir().expect("table navigation test directory should exist");
    let path = directory.path().join("table-navigation.md");
    let source = "| head | other |\n| --- | --- |\n| one | two |\n| three | four |\n\noutside";
    std::fs::write(&path, source).expect("table navigation fixture should be writable");
    let mut app = App::new(None);
    app.open_file(&path).expect("Markdown fixture should open");

    let first_cell = source.find("one").expect("first body cell should exist");
    let next_cell = source.find("two").expect("next body cell should exist");
    let previous_row = source.find("three").expect("next table row should exist");
    set_markdown_cursor(&mut app, first_cell);
    app.editor_runtime.handle_key_input(
        ACTIVE_EDITOR_CONTEXT,
        KeyCode::Tab,
        ui::core::Modifiers::NONE,
    );
    assert_eq!(active_markdown_cursor(&app), next_cell, "Tab should visit the next table cell");
    assert_eq!(active_markdown_source(&app), source, "table navigation must not edit source");

    app.editor_runtime.handle_key_input(
        ACTIVE_EDITOR_CONTEXT,
        KeyCode::Tab,
        ui::core::Modifiers { shift: true, ..ui::core::Modifiers::NONE },
    );
    assert_eq!(
        active_markdown_cursor(&app),
        first_cell,
        "Shift+Tab should visit the previous cell"
    );
    assert_eq!(active_markdown_source(&app), source, "reverse navigation must not edit source");

    app.editor_runtime.update_preedit(ACTIVE_EDITOR_CONTEXT, "拼".to_owned(), Some((0, 1)));
    app.editor_runtime.handle_key_input(
        ACTIVE_EDITOR_CONTEXT,
        KeyCode::Tab,
        ui::core::Modifiers::NONE,
    );
    app.editor_runtime.handle_key_input(
        ACTIVE_EDITOR_CONTEXT,
        KeyCode::Tab,
        ui::core::Modifiers { shift: true, ..ui::core::Modifiers::NONE },
    );
    assert_eq!(
        active_markdown_cursor(&app),
        first_cell,
        "IME composition should block both Tab directions"
    );
    assert_eq!(active_markdown_source(&app), source, "IME composition must not mutate the table");
    app.editor_runtime.update_preedit(ACTIVE_EDITOR_CONTEXT, String::new(), None);

    app.editor_runtime.handle_key_input(
        ACTIVE_EDITOR_CONTEXT,
        KeyCode::Enter,
        ui::core::Modifiers::NONE,
    );
    assert_eq!(
        active_markdown_cursor(&app),
        previous_row,
        "Enter should keep moving to the same column below"
    );
    assert_eq!(active_markdown_source(&app), source, "Enter navigation must not edit source");

    let outside_cell = source.find("outside").expect("outside paragraph should exist");
    set_markdown_cursor(&mut app, outside_cell);
    app.editor_runtime.handle_key_input(
        ACTIVE_EDITOR_CONTEXT,
        KeyCode::Tab,
        ui::core::Modifiers::NONE,
    );
    let indented_source = active_markdown_source(&app);
    assert_ne!(indented_source, source, "Tab outside a table should preserve indentation behavior");
    assert!(
        indented_source.ends_with("    outside") || indented_source.ends_with("\toutside"),
        "outside Tab should indent the paragraph: {indented_source:?}"
    );
}
