use super::*;
use ui::core::measure::NoopMeasure;
use ui::core::{KeyCode, Modifiers};

fn layout(view: &mut NotoraSettingsView, width: f32, height: f32, dpi: f32) {
    let theme = ui::theme::test_theme();
    let mut measure = NoopMeasure;
    view.set_rect(
        Rect::new(0.0, 0.0, width * dpi, height * dpi),
        &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi },
    );
}

fn editor_view(width: f32, height: f32) -> NotoraSettingsView {
    let mut view = NotoraSettingsView::new(SettingsOverlayInput::default());
    view.active_category = NotoraSettingsCategory::Editor;
    layout(&mut view, width, height, 1.0);
    view.focused_id = Some(FONT_FAMILY_ID);
    view.form.set_keyboard_focus(view.focused_id);
    view
}

fn enter_font_draft(view: &mut NotoraSettingsView) {
    let theme = ui::theme::test_theme();
    let mut context = EventCtx::new(&theme, 1.0);
    view.route_event(
        &Event::KeyDown(KeyCode::Char('a'), Modifiers { cmd: true, ..Modifiers::NONE }),
        &mut context,
    );
    view.route_event(&Event::ImeCommit("Draft Mono".to_owned()), &mut context);
}

fn commit(view: &mut NotoraSettingsView) -> Option<SettingsOverlayAction> {
    let theme = ui::theme::test_theme();
    view.route_event(
        &Event::KeyDown(KeyCode::Enter, Modifiers::NONE),
        &mut EventCtx::new(&theme, 1.0),
    )
}

#[test]
fn persistence_feedback_keeps_uncommitted_text() {
    let mut view = editor_view(720.0, 560.0);
    enter_font_draft(&mut view);
    let mut input = view.input.clone();
    input.persistence =
        NotoraSettingsPersistenceView::SaveFailed { message: "保存失败".to_owned() };
    view.set_input(input);
    layout(&mut view, 720.0, 560.0, 1.0);
    assert_eq!(
        commit(&mut view),
        Some(SettingsOverlayAction::Update(ProductSettingsUpdate::FontFamily(
            "Draft Mono".to_owned()
        )))
    );
}

#[test]
fn updating_another_setting_keeps_uncommitted_text() {
    let mut view = editor_view(720.0, 560.0);
    enter_font_draft(&mut view);
    let mut input = view.input.clone();
    input.product_settings.editor.word_wrap = !input.product_settings.editor.word_wrap;
    view.set_input(input);
    layout(&mut view, 720.0, 560.0, 1.0);
    assert_eq!(
        commit(&mut view),
        Some(SettingsOverlayAction::Update(ProductSettingsUpdate::FontFamily(
            "Draft Mono".to_owned()
        )))
    );
}

#[test]
fn persistence_feedback_keeps_validation_errors() {
    let mut view = editor_view(720.0, 560.0);
    assert_eq!(view.map_text_commit(FONT_SIZE_ID, "100"), None);
    let mut input = view.input.clone();
    input.persistence =
        NotoraSettingsPersistenceView::SaveFailed { message: "保存失败".to_owned() };
    view.set_input(input);
    assert!(view.validation.is_some());
}

#[test]
fn product_update_keeps_scroll_position() {
    let mut view = editor_view(720.0, 300.0);
    let theme = ui::theme::test_theme();
    view.route_event(
        &Event::Wheel {
            dx: 0.0,
            dy: -100.0,
            px: view.form_rect.x + 10.0,
            py: view.form_rect.y + 10.0,
        },
        &mut EventCtx::new(&theme, 1.0),
    );
    let previous_scroll = view.form.scroll_offset();
    assert!(previous_scroll > 0.0);
    let mut input = view.input.clone();
    input.product_settings.editor.tab_width = 8;
    view.set_input(input);
    layout(&mut view, 720.0, 300.0, 1.0);
    assert_eq!(view.form.scroll_offset(), previous_scroll);
}

#[test]
fn compact_theme_controls_fit_inside_the_form_at_each_scale() {
    for dpi in [1.0, 2.0] {
        let mut view = NotoraSettingsView::new(SettingsOverlayInput::default());
        layout(&mut view, 452.0, 352.0, dpi);
        let theme = ui::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut row = view.theme_mode_row();
        row.set_rect(
            Rect::new(0.0, 0.0, view.form_rect.w, 120.0 * dpi),
            &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi },
        );
        assert!(row.control_rect().w >= SEGMENT_WIDTH_LOGICAL * 3.0 * dpi);
        assert!(row.control_rect().y >= row.label_rect().bottom());
    }
}

#[test]
fn resizing_while_changing_dpi_keeps_full_control_height() {
    let mut view = editor_view(720.0, 560.0);
    layout(&mut view, 452.0, 352.0, 2.0);
    let caret = view.form.focused_ime_cursor_rect().expect("font field should remain focused");
    let mut fresh = editor_view(452.0, 352.0);
    layout(&mut fresh, 452.0, 352.0, 2.0);
    let expected = fresh.form.focused_ime_cursor_rect().expect("font field should have a caret");
    assert_eq!(caret, expected);
}

#[test]
fn responsive_layout_keeps_the_text_selection() {
    let mut view = editor_view(720.0, 560.0);
    let theme = ui::theme::test_theme();
    view.route_event(
        &Event::KeyDown(KeyCode::Char('a'), Modifiers { cmd: true, ..Modifiers::NONE }),
        &mut EventCtx::new(&theme, 1.0),
    );
    layout(&mut view, 660.0, 560.0, 1.0);
    view.route_event(&Event::ImeCommit("X".to_owned()), &mut EventCtx::new(&theme, 1.0));
    assert_eq!(
        commit(&mut view),
        Some(SettingsOverlayAction::Update(ProductSettingsUpdate::FontFamily("X".to_owned())))
    );
}

#[test]
fn invalid_number_identifies_the_field_and_allowed_range() {
    let mut view = editor_view(720.0, 560.0);
    assert_eq!(view.map_text_commit(FONT_SIZE_ID, "NaN"), None);
    let message = view.message_text().expect("invalid input should explain the correction");
    assert!(message.contains("字号"));
    assert!(message.contains("6–72"));
}

#[test]
fn displayed_float_preserves_the_stored_precision() {
    let ratio = 1.234_567_f32;
    assert_eq!(format_float(ratio).parse::<f32>().expect("displayed ratio should parse"), ratio);
}

#[test]
fn persistence_message_is_clipped_before_the_retry_button() {
    let mut view = editor_view(452.0, 352.0);
    let mut input = view.input.clone();
    input.persistence = NotoraSettingsPersistenceView::SaveFailed {
        message: "无法写入设置文件：".repeat(30),
    };
    view.set_input(input);
    layout(&mut view, 452.0, 352.0, 1.0);
    let theme = ui::theme::test_theme();
    let mut draw_list = ui::core::paint::DrawList::new();
    view.paint_message(&mut PaintCtx {
        list: &mut draw_list,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        global_alpha: 1.0,
        shaper: None,
    });
    let clip = draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            ui::core::paint::DrawCmd::PushClip(rect) => Some(*rect),
            _ => None,
        })
        .expect("save errors must be clipped to their own text area");
    assert!(clip.right() < view.retry_button_rect().x);
    assert!(clip.x >= view.form_rect.x);
}

#[test]
fn long_validation_message_wraps_without_hiding_the_allowed_range() {
    let mut view = editor_view(412.0, 352.0);
    let mut input = view.input.clone();
    input.persistence =
        NotoraSettingsPersistenceView::SaveFailed { message: "保存失败".to_owned() };
    view.set_input(input);
    view.map_text_commit(AUTO_SAVE_DELAY_ID, "0");
    layout(&mut view, 412.0, 352.0, 1.0);
    let theme = ui::theme::test_theme();
    let mut draw_list = ui::core::paint::DrawList::new();
    let mut shaper = shaping::Shaper::new().expect("message test shaper should exist");
    view.paint_message(&mut PaintCtx {
        list: &mut draw_list,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        global_alpha: 1.0,
        shaper: Some(&mut shaper),
    });
    let lines: Vec<_> = draw_list
        .cmds
        .iter()
        .filter_map(|command| match command {
            ui::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text != "重试" => {
                Some(layout.text.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.concat().contains("100–60000"));
}

#[test]
fn resizing_preserves_scroll_until_the_final_viewport_is_known() {
    let mut view = editor_view(452.0, 560.0);
    let theme = ui::theme::test_theme();
    view.route_event(
        &Event::Wheel {
            dx: 0.0,
            dy: -1_000.0,
            px: view.form_rect.x + 10.0,
            py: view.form_rect.y + 10.0,
        },
        &mut EventCtx::new(&theme, 1.0),
    );
    assert!(view.form.scroll_offset() > 0.0);
    layout(&mut view, 720.0, 300.0, 1.0);
    assert!(view.form.scroll_offset() > 0.0);
}
