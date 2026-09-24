use super::*;
use crate::actions::AppAction;
use ui::plugin::SemanticEditCommand;

#[test]
fn inserted_empty_table_undo_redo_and_save_reopen_preserve_source_cursor_and_structure() {
    let directory = tempfile::tempdir().expect("table creation test directory should exist");
    let path = directory.path().join("created-table.md");
    let original_source = "existing paragraph";
    std::fs::write(&path, original_source).expect("table creation fixture should be writable");

    let mut app = App::new(None);
    app.open_file(&path).expect("Markdown fixture should open");
    let cursor_before =
        app.active_tab_session().expect("Markdown tab should be active").document.cursor_offset().0;
    let generation_before =
        app.active_tab_session().expect("Markdown tab should be active").document.generation();
    let mut picker = ui::table_picker::TablePickerWidget::new();
    picker.set_input(ui::table_picker::TablePickerInput { open: true });
    app.ui_shell.push_overlay_with_policy(
        Box::new(ui::modal_frame::ModalFrame::new("插入表格", Box::new(picker))),
        ui::OverlayLayout::Centered {
            preferred_size: (300.0, 270.0),
            min_margin: 16.0,
            max_width_ratio: 0.92,
            max_height_ratio: 0.90,
        },
        ui::OverlayInputPolicy::Modal,
        ui::DismissPolicy::EscapeOrExplicit,
    );

    let effect = app.reduce_action(
        AppAction::TablePicker(ui::table_picker::TablePickerAction::Confirmed(
            ui::table_picker::TableSize { columns: 2, rows: 3 },
        )),
        None,
    );
    assert!(effect.redraw);
    let inserted_source =
        app.active_tab_session().expect("Markdown tab should remain active").document.full_text();
    assert_eq!(
        app.active_tab_session().expect("Markdown tab should remain active").document.generation(),
        generation_before + 1,
        "one picker confirmation should produce one document generation"
    );
    let inserted_cursor = app
        .active_tab_session()
        .expect("Markdown tab should remain active")
        .document
        .cursor_offset()
        .0;
    assert!(inserted_source.contains("| --- | --- |"));
    assert_eq!(inserted_source.matches("| | |").count(), 3);

    app.dispatch_semantic_edit(SemanticEditCommand::Undo);
    let active = app.active_tab_session().expect("Markdown tab should remain active");
    assert_eq!(active.document.full_text(), original_source);
    assert_eq!(active.document.cursor_offset().0, cursor_before);

    app.dispatch_semantic_edit(SemanticEditCommand::Redo);
    let active = app.active_tab_session().expect("Markdown tab should remain active");
    assert_eq!(active.document.full_text(), inserted_source);
    assert_eq!(active.document.cursor_offset().0, inserted_cursor);

    let tab_id = app.editor_tab_id_at(0).expect("saved Markdown tab should have an id");
    let save =
        app.editor_runtime.prepare_save(tab_id).expect("redo state should be available for saving");
    std::fs::write(&save.path, save.serialized_contents)
        .expect("prepared table source should be saved");

    let mut reopened_app = App::new(None);
    reopened_app.open_file(&path).expect("saved Markdown file should reopen");
    let reopened_source = reopened_app
        .active_tab_session()
        .expect("reopened Markdown tab should be active")
        .document
        .full_text();
    assert_eq!(reopened_source, inserted_source);
    let parsed = textora_markdown::parser::parse_markdown(&reopened_source);
    assert!(parsed.events.iter().any(|event| matches!(
        event,
        textora_markdown::parser::MarkdownEvent::Start(
            textora_markdown::parser::MarkdownTag::Table(alignments)
        ) if alignments.len() == 2
    )));
    assert_eq!(
        parsed
            .events
            .iter()
            .filter(|event| matches!(
                event,
                textora_markdown::parser::MarkdownEvent::Start(
                    textora_markdown::parser::MarkdownTag::TableRow
                )
            ))
            .count(),
        2,
        "two empty body rows should survive file reopen"
    );
}
