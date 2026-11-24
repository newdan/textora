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

#[test]
fn settings_text_inputs_align_with_the_right_edge_of_wide_rows() {
    let theme = ui::theme::test_theme();
    for dpi in [1.0, 1.5, 2.0] {
        let view = editor_view(720.0, 560.0);
        let mut row = view.text_row(
            FONT_FAMILY_ID,
            "正文字体",
            "输入字体名称；代码块仍使用等宽字体。",
            "Menlo",
            "字体名称",
        );
        let mut measure = NoopMeasure;
        let width = 540.0 * dpi;
        row.set_rect(
            Rect::new(0.0, 0.0, width, ROW_HEIGHT_LOGICAL * dpi),
            &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi },
        );
        let node = row
            .accessibility_node(&ui::core::AccessibilityContext::new(0.0, 0.0))
            .expect("font row must expose its field");
        let field = node
            .children
            .iter()
            .find(|node| node.id == FONT_FAMILY_ID.into())
            .expect("font field must expose its actual bounds");
        assert_eq!(field.bounds.w, TEXT_BOX_WIDTH_LOGICAL * dpi);
        assert_eq!(field.bounds.right(), width - ROW_HORIZONTAL_INSET_LOGICAL * dpi);
        assert!(
            row.description_rect().expect("font description").right()
                + ROW_COLUMN_GAP_LOGICAL * dpi
                <= field.bounds.x
        );
    }
}

#[test]
fn all_settings_categories_keep_text_out_of_controls_at_each_width_and_scale() {
    let theme = ui::theme::test_theme();
    let mut shaper = shaping::Shaper::new().expect("settings regression needs system fonts");
    for category in [
        NotoraSettingsCategory::Appearance,
        NotoraSettingsCategory::Editor,
        NotoraSettingsCategory::Interface,
        NotoraSettingsCategory::Workspace,
    ] {
        for width in [412.0, 660.0, 720.0, 1024.0] {
            for dpi in [1.0, 1.5, 2.0] {
                let mut view = NotoraSettingsView::new(SettingsOverlayInput::default());
                view.active_category = category;
                layout(&mut view, width, 2400.0, dpi);
                let mut nodes = Vec::new();
                view.form.collect_accessibility_nodes(
                    &ui::core::AccessibilityContext::new(view.form_rect.x, view.form_rect.y),
                    &mut nodes,
                );
                let mut drawing = ui::core::DrawList::new();
                paint_widget_at(
                    &view.form,
                    view.form_rect,
                    &mut PaintCtx {
                        list: &mut drawing,
                        theme: &theme,
                        dpi,
                        global_alpha: 1.0,
                        shaper: Some(&mut shaper),
                    },
                );
                assert_settings_labels_are_clipped(&drawing, &nodes, view.form_rect);
            }
        }
    }
}

fn assert_settings_labels_are_clipped(
    drawing: &ui::core::DrawList,
    nodes: &[ui::core::AccessibilityNode],
    form_bounds: Rect,
) {
    use ui::core::{AccessibilityRole, DrawCmd};

    assert_settings_labels_do_not_overlap_controls(nodes);
    let labels: Vec<_> = nodes
        .iter()
        .flat_map(|node| &node.children)
        .filter(|node| node.role == AccessibilityRole::StaticText)
        .collect();
    let mut clips = Vec::new();
    let mut checked_labels = 0;
    for command in &drawing.cmds {
        match command {
            DrawCmd::PushClip(bounds) => clips.push(*bounds),
            DrawCmd::PopClip => {
                clips.pop().expect("clip commands must be balanced");
            }
            DrawCmd::TextLayout { layout, x, .. } => {
                if let Some(label) =
                    labels.iter().find(|label| label.name.as_deref() == Some(layout.text.as_str()))
                {
                    assert_eq!(
                        clips.last(),
                        Some(&label.bounds),
                        "text '{}' must stay in its own area",
                        layout.text
                    );
                    assert!(
                        layout.shaped.width <= label.bounds.w + f32::EPSILON,
                        "full description '{}' needs {}px but only {}px is available",
                        layout.text,
                        layout.shaped.width,
                        label.bounds.w
                    );
                    checked_labels += 1;
                } else if layout.text == "主题" {
                    let clip = clips.last().expect("theme row title must be clipped");
                    assert_eq!(clip.x, *x);
                    assert!(clip.w < form_bounds.w);
                    checked_labels += 1;
                }
            }
            _ => {}
        }
    }
    assert!(clips.is_empty());
    assert_eq!(
        checked_labels,
        labels.len().max(1),
        "all row titles and descriptions must be checked"
    );
}

fn assert_settings_labels_do_not_overlap_controls(nodes: &[ui::core::AccessibilityNode]) {
    use ui::core::AccessibilityRole;

    for row in nodes {
        for label in row.children.iter().filter(|node| node.role == AccessibilityRole::StaticText) {
            for control in
                row.children.iter().filter(|node| node.role != AccessibilityRole::StaticText)
            {
                assert!(
                    label.bounds.right() <= control.bounds.x
                        || label.bounds.bottom() <= control.bounds.y,
                    "label {:?} must not overlap control {:?}",
                    label.name,
                    control.role
                );
            }
        }
    }
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
fn interface_font_commits_as_a_separate_update() {
    let mut view = NotoraSettingsView::new(SettingsOverlayInput::default());
    view.active_category = NotoraSettingsCategory::Interface;
    layout(&mut view, 720.0, 560.0, 1.0);

    assert_eq!(
        view.map_text_commit(UI_FONT_FAMILY_ID, "Segoe UI"),
        Some(SettingsOverlayAction::Update(ProductSettingsUpdate::UiFontFamily(
            "Segoe UI".to_owned()
        )))
    );
    assert_eq!(view.map_text_commit(UI_FONT_FAMILY_ID, "   "), None);
    assert_eq!(view.validation.map(|validation| validation.id), Some(UI_FONT_FAMILY_ID));
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
