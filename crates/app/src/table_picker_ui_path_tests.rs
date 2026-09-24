use super::*;
use ui::core::geom::Rect;
use ui::plugin::{EditHitTarget, TableColumnAlignment, TableStructureCommand};
use winit::event::ElementState;

const HIT_TEST_STEP_PX: usize = 4;
const PICKER_CELL_SIZE_PX: f32 = 24.0;
const PICKER_GRID_PADDING_PX: f32 = 12.0;
const PICKER_HEADER_HEIGHT_PX: f32 = 34.0;
const PICKER_INPUT_HEIGHT_PX: f32 = 30.0;
const PICKER_PANEL_PADDING_PX: f32 = 12.0;

fn render_markdown_plugin(app: &mut App) {
    app.sync_plugin_state();
    let bounds = app.plugin_render_bounds();
    let theme = app.current_theme.clone();
    let dpi = app.ui_metrics().dpi;
    let font_size = app.ui_metrics().font_size;
    let mut shaper = app
        .editor_runtime
        .new_shaper(font_size, "")
        .unwrap_or_else(|| shaping::Shaper::new().expect("test shaper should initialize"));
    let mut tab = app.active_tab_session_mut().expect("Markdown tab should be active");
    let _ = tab.render_plugin(bounds, &theme, &mut shaper, dpi);
}

fn text_caret_hit_point(app: &mut App, target: std::ops::Range<usize>) -> (f32, f32, usize) {
    let bounds = app.plugin_render_bounds();
    let width = bounds.w.ceil() as usize;
    let height = bounds.h.ceil() as usize;
    let mut observed_offsets = std::collections::BTreeSet::new();
    for y in (0..=height).step_by(HIT_TEST_STEP_PX) {
        for x in (0..=width).step_by(HIT_TEST_STEP_PX) {
            let px = bounds.x + x as f32;
            let py = bounds.y + y as f32;
            let tab = app.active_tab_session().expect("Markdown tab should be active");
            if let Some(Some(EditHitTarget::TextCaret { byte_offset, .. })) =
                tab.hit_test_edit_target(px, py, bounds.x, bounds.y)
            {
                observed_offsets.insert(byte_offset);
                if target.contains(&byte_offset) {
                    return (px, py, byte_offset);
                }
            }
        }
    }
    panic!(
        "rendered Markdown cell should expose a caret hit point in {target:?}; bounds={bounds:?}, observed {observed_offsets:?}"
    );
}

fn apply_event_actions(app: &mut App, actions: Vec<crate::actions::AppAction>) {
    for action in actions {
        let effect = app.reduce_action(action, None);
        app.apply_effect(effect);
    }
}

fn open_structure_menu_at(app: &mut App, byte_offset: usize) -> ui::core::geom::Rect {
    render_markdown_plugin(app);
    let (px, py, hit_byte) = text_caret_hit_point(app, byte_offset..byte_offset + 1);
    let actions = crate::events::handle_mouse_input_right(app, ElementState::Pressed, px, py);
    assert!(
        actions.iter().any(|action| matches!(action, crate::actions::AppAction::OpenPopupMenu(_))),
        "a real right-click on a rendered table cell should open its structure popup"
    );
    apply_event_actions(app, actions);
    let popup = app
        .ui_shell
        .active_overlay_widget_ref::<ui::popup_menu::PopupMenuWidget>()
        .expect("right-click should install the popup widget");
    assert!(popup.menu().items.iter().any(|item| matches!(
        item.action,
        ui::popup_menu::PopupMenuAction::TableStructure { cursor_byte, .. }
            if cursor_byte == hit_byte
    )));
    app.ui_shell.active_overlay_layout_rect().expect("popup should have a screen rectangle")
}

fn open_markdown_app(source: &str) -> (App, tempfile::TempDir) {
    let directory = tempfile::tempdir().expect("table UI test directory should exist");
    let path = directory.path().join("table-ui-path.md");
    std::fs::write(&path, source).expect("Markdown fixture should be writable");
    let mut app = App::new(None);
    app.open_file(&path).expect("Markdown fixture should open");
    (app, directory)
}

fn push_table_picker_overlay(app: &mut App) -> (Rect, Rect) {
    let mut picker = ui::table_picker::TablePickerWidget::new();
    picker.set_input(ui::table_picker::TablePickerInput { open: true });
    let mut frame = ui::modal_frame::ModalFrame::new("插入表格", Box::new(picker));
    let modal_size = Rect::new(0.0, 0.0, 300.0, 270.0);
    let mut measure = ui::core::NoopMeasure;
    let mut context = ui::core::LayoutCtx {
        ui_measure: None,
        measure: &mut measure,
        theme: &app.current_theme,
        dpi: 1.0,
    };
    ui::core::Widget::set_rect(&mut frame, modal_size, &mut context);
    let content_rect = frame.content_rect();
    let overlay_rect = Rect::new(100.0, 100.0, 300.0, 270.0);
    app.ui_shell.push_overlay(Box::new(frame), overlay_rect);
    (overlay_rect, content_rect)
}

fn picker_grid_point(overlay: Rect, content: Rect, column: f32, row: f32) -> (f32, f32) {
    (
        overlay.x + content.x + PICKER_GRID_PADDING_PX + column * PICKER_CELL_SIZE_PX,
        overlay.y + content.y + PICKER_HEADER_HEIGHT_PX + row * PICKER_CELL_SIZE_PX,
    )
}

fn send_picker_key(app: &mut App, key: ui::KeyCode) {
    let widget_action = app
        .ui_shell
        .forward_key(key, ui::Modifiers::NONE, &app.current_theme, app.ui_metrics().dpi)
        .expect("keyboard event should reach the focused table picker");
    let mut actions = Vec::new();
    crate::events::translate_widget_action(&widget_action, app, &mut actions);
    apply_event_actions(app, actions);
}

fn click_popup_item(app: &mut App, item_index: usize, popup_rect: Rect) {
    let click_point = {
        let popup = app
            .ui_shell
            .active_overlay_widget_ref::<ui::popup_menu::PopupMenuWidget>()
            .expect("popup widget should remain open");
        let item =
            popup.menu().item_rects.get(item_index).expect("requested popup item should exist");
        (popup_rect.x + item.x + item.w * 0.5, popup_rect.y + item.y + item.h * 0.5)
    };
    let press = crate::events::handle_mouse_input_left(
        app,
        ElementState::Pressed,
        click_point.0,
        click_point.1,
    );
    apply_event_actions(app, press);
    let release = crate::events::handle_mouse_input_left(
        app,
        ElementState::Released,
        click_point.0,
        click_point.1,
    );
    apply_event_actions(app, release);
}

#[test]
fn right_click_on_rendered_table_disables_header_row_and_last_column_deletion() {
    let source = "| only |\n| --- |\n| cell |";
    let mut app = open_markdown_app(source).0;
    let header_cell = source.find("only").expect("fixture has a header cell");
    let _popup_rect = open_structure_menu_at(&mut app, header_cell);
    let popup = app
        .ui_shell
        .active_overlay_widget_ref::<ui::popup_menu::PopupMenuWidget>()
        .expect("popup should remain open");
    assert!(!popup.menu().items[2].enabled, "table header rows cannot be deleted");
    assert!(!popup.menu().items[5].enabled, "the last table column cannot be deleted");
}

#[test]
fn right_click_and_choose_alignment_updates_the_clicked_second_column_once() {
    let source = "| first | second |\n| --- | --- |\n| left | right |";
    let mut app = open_markdown_app(source).0;
    let second_cell = source.find("right").expect("fixture has a second-column body cell");
    let popup_rect = open_structure_menu_at(&mut app, second_cell);
    let popup = app
        .ui_shell
        .active_overlay_widget_ref::<ui::popup_menu::PopupMenuWidget>()
        .expect("popup should remain open");
    assert!(matches!(
        popup.menu().items[9].action,
        ui::popup_menu::PopupMenuAction::TableStructure {
            command: TableStructureCommand::SetColumnAlignment(TableColumnAlignment::Right),
            ..
        }
    ));
    let generation_before =
        app.active_tab_session().expect("Markdown tab should be active").document.generation();

    click_popup_item(&mut app, 9, popup_rect);

    let active = app.active_tab_session().expect("Markdown tab should remain active");
    let updated_source = active.document.full_text();
    assert!(
        updated_source.contains("| --- | ---: |"),
        "second delimiter cell should align right: {updated_source:?}"
    );
    assert_eq!(
        updated_source.lines().nth(1),
        Some("| --- | ---: |"),
        "only the clicked second-column delimiter cell should change"
    );
    assert_eq!(
        active.document.generation(),
        generation_before + 1,
        "one popup click should create one transaction"
    );
    assert!(!app.ui_shell.active_overlay_is_modal());
}

#[test]
fn picker_keyboard_events_reach_the_modal_widget_and_confirm_one_table_transaction() {
    let (mut app, _directory) = open_markdown_app("existing paragraph");
    let generation_before =
        app.active_tab_session().expect("Markdown tab should be active").document.generation();
    push_table_picker_overlay(&mut app);
    send_picker_key(&mut app, ui::KeyCode::Right);
    send_picker_key(&mut app, ui::KeyCode::Down);
    send_picker_key(&mut app, ui::KeyCode::Enter);

    let active = app.active_tab_session().expect("Markdown tab should remain active");
    assert_eq!(active.document.generation(), generation_before + 1);
    let parsed = textora_markdown::parser::parse_markdown(&active.document.full_text());
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
        "three selected rows should include two body rows in addition to the header"
    );
    assert!(!app.ui_shell.active_overlay_is_modal());
}

#[test]
fn picker_drag_events_confirm_only_after_release_and_create_one_transaction() {
    let (mut app, _directory) = open_markdown_app("existing paragraph");
    let generation_before =
        app.active_tab_session().expect("Markdown tab should be active").document.generation();
    let (overlay, content) = push_table_picker_overlay(&mut app);
    let start = picker_grid_point(overlay, content, 0.5, 0.5);
    let target = picker_grid_point(overlay, content, 2.5, 2.5);

    let hover = crate::events::handle_cursor_moved(&mut app, target.0, target.1);
    apply_event_actions(&mut app, hover);
    assert_eq!(
        app.active_tab_session().expect("Markdown tab should remain active").document.generation(),
        generation_before,
        "hover preview must not edit the document"
    );
    let press =
        crate::events::handle_mouse_input_left(&mut app, ElementState::Pressed, start.0, start.1);
    apply_event_actions(&mut app, press);
    let drag = crate::events::handle_cursor_moved(&mut app, target.0, target.1);
    apply_event_actions(&mut app, drag);
    assert_eq!(
        app.active_tab_session().expect("Markdown tab should remain active").document.generation(),
        generation_before,
        "dragging should update only the preview"
    );
    let release = crate::events::handle_mouse_input_left(
        &mut app,
        ElementState::Released,
        target.0,
        target.1,
    );
    apply_event_actions(&mut app, release);

    let active = app.active_tab_session().expect("Markdown tab should remain active");
    assert_eq!(active.document.generation(), generation_before + 1);
    let parsed = textora_markdown::parser::parse_markdown(&active.document.full_text());
    assert!(parsed.events.iter().any(|event| matches!(
        event,
        textora_markdown::parser::MarkdownEvent::Start(
            textora_markdown::parser::MarkdownTag::Table(alignments)
        ) if alignments.len() == 3
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
        3,
        "four selected rows should create three body rows"
    );
}

#[test]
fn picker_numeric_input_and_enter_confirm_the_selected_dimensions_once() {
    let (mut app, _directory) = open_markdown_app("existing paragraph");
    let generation_before =
        app.active_tab_session().expect("Markdown tab should be active").document.generation();
    let (overlay, content) = push_table_picker_overlay(&mut app);
    let initial_hover = picker_grid_point(overlay, content, 1.5, 1.5);
    let hover = crate::events::handle_cursor_moved(&mut app, initial_hover.0, initial_hover.1);
    apply_event_actions(&mut app, hover);
    let row_input = (
        overlay.x + content.x + content.w * 0.75,
        overlay.y + content.y + content.h
            - (PICKER_INPUT_HEIGHT_PX + PICKER_PANEL_PADDING_PX * 0.5),
    );
    let press = crate::events::handle_mouse_input_left(
        &mut app,
        ElementState::Pressed,
        row_input.0,
        row_input.1,
    );
    apply_event_actions(&mut app, press);
    let release = crate::events::handle_mouse_input_left(
        &mut app,
        ElementState::Released,
        row_input.0,
        row_input.1,
    );
    apply_event_actions(&mut app, release);
    send_picker_key(&mut app, ui::KeyCode::Char('4'));
    send_picker_key(&mut app, ui::KeyCode::Enter);

    let active = app.active_tab_session().expect("Markdown tab should remain active");
    assert_eq!(active.document.generation(), generation_before + 1);
    let parsed = textora_markdown::parser::parse_markdown(&active.document.full_text());
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
        3,
        "numeric row entry should override the hover preview and create three body rows"
    );
}
