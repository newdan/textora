use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use unicode_segmentation::UnicodeSegmentation;

use super::{EchoMode, MASKED_ECHO_GLYPH, TextBox, TextBoxChrome, TextBoxIme};
use crate::core::widget::{
    ControlAction, Event, EventCtx, TextPayload, Widget, WidgetAction, WidgetId,
};
use crate::core::{Clipboard, KeyCode, LayoutCtx, Modifiers, MouseButton, PaintCtx, Rect};

#[test]
fn password_mode_owns_the_input_method_policy() {
    let mut input = TextBox::new();
    assert!(input.ime_allowed());
    input.set_password_mode(true);
    assert!(!input.ime_allowed());
    input.set_password_mode(false);
    assert!(input.ime_allowed());
    input.set_echo_mode(EchoMode::Masked);
    assert!(!input.ime_allowed());
}

#[test]
fn matching_text_box_rebuild_preserves_selection_and_preedit() {
    let theme = crate::theme::test_theme();
    let mut context = EventCtx::new(&theme, 1.0);
    let mut previous = TextBox::with_id(WidgetId(42));
    previous.set_text("abcd");
    previous.set_focus(true);
    let _ = previous.on_event(
        &Event::KeyDown(KeyCode::Left, Modifiers { shift: true, ..Modifiers::NONE }),
        &mut context,
    );
    let mut replacement = TextBox::with_id(WidgetId(42));
    replacement.set_text("abcd");
    replacement.preserve_editing_state_from(&previous);
    assert_eq!(replacement.cursor_byte, previous.cursor_byte);
    assert_eq!(replacement.selection, previous.selection);

    let _ = previous.on_event(
        &Event::ImePreedit { text: "候选".to_owned(), cursor: Some((6, 6)) },
        &mut context,
    );
    replacement.preserve_editing_state_from(&previous);
    assert_eq!(replacement.preedit.as_str(), "候选");
    assert_eq!(replacement.preedit_cursor, previous.preedit_cursor);
}

#[test]
fn component_cursor_blinks_resets_on_typing_and_stops_when_unfocused() {
    let mut input = TextBox::new();
    assert_eq!(input.next_cursor_blink_at(), None);
    input.set_focus(true);
    let deadline = input.next_cursor_blink_at().expect("focused input should schedule blinking");
    assert!(!input.advance_cursor_blink(deadline - Duration::from_millis(1)));
    assert!(input.blink_on);
    assert!(input.advance_cursor_blink(deadline));
    assert!(!input.blink_on);
    let theme = crate::theme::test_theme();
    let mut context = EventCtx::new(&theme, 1.0);
    input.on_event(&Event::KeyDown(KeyCode::Char('a'), Modifiers::NONE), &mut context);
    assert!(input.blink_on);
    assert!(input.next_cursor_blink_at().is_some());
    input.select_all();
    assert_eq!(input.next_cursor_blink_at(), None);
    input.set_focus(false);
    assert_eq!(input.next_cursor_blink_at(), None);
}

struct TestClipboard {
    text: Rc<RefCell<Option<String>>>,
}

impl Clipboard for TestClipboard {
    fn read_text(&mut self) -> Option<String> {
        self.text.borrow().clone()
    }

    fn write_text(&mut self, text: &str) -> bool {
        self.text.replace(Some(text.to_owned()));
        true
    }
}

struct RejectingClipboard;

impl Clipboard for RejectingClipboard {
    fn read_text(&mut self) -> Option<String> {
        None
    }

    fn write_text(&mut self, _text: &str) -> bool {
        false
    }
}

fn paint_laid_out(tb: &mut TextBox) -> crate::core::paint::DrawList {
    use crate::core::measure::NoopMeasure;

    let theme = crate::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut layout_ctx =
        LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
    tb.layout(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_ctx);

    let mut draw_list = crate::core::paint::DrawList::new();
    let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
    let mut paint_ctx = PaintCtx {
        global_alpha: 1.0,
        list: &mut draw_list,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        shaper: Some(&mut shaper),
    };
    tb.paint(&mut paint_ctx);
    draw_list
}

fn laid_out_widget(mut text_box: TextBox) -> TextBox {
    use crate::core::measure::NoopMeasure;

    let theme = crate::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut layout_ctx =
        LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
    text_box.set_rect(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_ctx);
    text_box
}

fn key(text_box: &mut TextBox, key_code: KeyCode) -> Option<WidgetAction> {
    key_with_modifiers(text_box, key_code, Modifiers::NONE)
}

fn key_with_modifiers(
    text_box: &mut TextBox,
    key_code: KeyCode,
    modifiers: Modifiers,
) -> Option<WidgetAction> {
    let theme = crate::theme::test_theme();
    let mut event_ctx = EventCtx::new(&theme, 1.0);
    text_box.set_focus(true);
    text_box.on_event(&Event::KeyDown(key_code, modifiers), &mut event_ctx)
}

#[test]
fn unfocused_text_box_rejects_keyboard_and_ime_input() {
    let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(29)));
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::new(&theme, 1.0);

    assert_eq!(
        text_box
            .on_event(&Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE), &mut event_context,),
        None
    );
    assert_eq!(
        text_box.on_event(
            &Event::ImePreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut event_context
        ),
        None
    );
    assert_eq!(text_box.on_event(&Event::ImeCommit("你好".into()), &mut event_context), None);
    text_box.on_ime(&TextBoxIme::Commit("仍不应写入".into()));
    assert_eq!(text_box.text(), "");
    assert!(!text_box.has_preedit());
}

#[test]
fn accessibility_exposes_plain_value_and_set_value_reuses_text_edited_action() {
    let id = WidgetId(80);
    let mut text_box = laid_out_widget(TextBox::with_id(id));
    text_box.set_accessibility_label(Some("字体名称".into()));
    text_box.set_text("Menlo");
    text_box.set_keyboard_focus(Some(id));
    let context = crate::core::AccessibilityContext::new(10.0, 20.0);
    let node = text_box.accessibility_node(&context).expect("identified textbox exposes semantics");

    assert_eq!(node.role, crate::core::AccessibilityRole::TextField);
    assert_eq!(node.name.as_deref(), Some("字体名称"));
    assert_eq!(node.value.as_deref(), Some("Menlo"));
    assert_eq!(node.bounds, Rect::new(10.0, 20.0, 200.0, 28.0));
    assert!(node.state.focused);
    assert!(!node.state.sensitive);
    assert!(node.actions.contains(&crate::core::AccessibilityAction::SetValue));

    assert_eq!(
        text_box.on_accessibility_action(&crate::core::AccessibilityActionRequest::set_value(
            node.id,
            TextPayload::Plain("Monaco".into()),
        )),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            id,
            value: TextPayload::Plain("Monaco".into()),
        }))
    );
    assert_eq!(text_box.text(), "Monaco");
}

#[test]
fn accessibility_never_exposes_masked_text_value() {
    let id = WidgetId(81);
    let mut text_box = laid_out_widget(TextBox::with_id(id));
    text_box.set_accessibility_label(Some("访问令牌".into()));
    text_box.set_password_mode(true);
    text_box.set_text("secret-token");

    let node = text_box
        .accessibility_node(&crate::core::AccessibilityContext::default())
        .expect("masked textbox remains discoverable");

    assert_eq!(node.value, None);
    assert!(node.state.sensitive);
    assert_eq!(crate::core::AccessibilityTree::new(node, None).validate(), Ok(()));
}

#[test]
fn pointer_leave_preserves_text_drag_and_cancel_clears_drag_and_preedit() {
    let mut text_box = laid_out_widget(TextBox::new());
    text_box.set_text("hello");
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::new(&theme, 1.0);

    assert!(
        text_box
            .on_event(
                &Event::MouseDown { px: 10.0, py: 14.0, button: MouseButton::Left },
                &mut event_context,
            )
            .is_some()
    );
    text_box.set_focus(true);
    assert_eq!(
        text_box.on_event(
            &Event::ImePreedit { text: "未完成".to_owned(), cursor: Some((0, 6)) },
            &mut event_context,
        ),
        Some(WidgetAction::Consumed)
    );

    assert_eq!(text_box.on_event(&Event::PointerLeave, &mut event_context), None);
    assert!(text_box.is_capturing());
    assert!(text_box.has_preedit());

    assert_eq!(
        text_box.on_event(&Event::InteractionCancel, &mut event_context),
        Some(WidgetAction::Consumed)
    );
    assert!(!text_box.is_capturing());
    assert!(!text_box.has_preedit());
    assert_eq!(text_box.on_event(&Event::InteractionCancel, &mut event_context), None);
    assert_eq!(
        text_box.on_event(
            &Event::MouseUp { px: 10.0, py: 14.0, button: MouseButton::Left },
            &mut event_context,
        ),
        None
    );
}

#[test]
fn tiny_pointer_move_during_click_keeps_caret_visible_after_mouse_up() {
    use crate::core::paint::DrawCmd;

    let mut text_box = laid_out_widget(TextBox::new());
    text_box.set_text("hello");
    text_box.set_focus(true);
    text_box.set_blink(true);

    assert!(text_box.on_mouse_down(20.0, 14.0));
    text_box.on_mouse_drag(20.25, 14.0);
    text_box.on_mouse_up();

    assert_eq!(text_box.selection, None);
    let draw_list = paint_laid_out(&mut text_box);
    assert!(draw_list.cmds.iter().any(|command| {
        matches!(
            command,
            DrawCmd::FillRect { rect, radius, .. }
                if *radius == 0.0 && rect.w == 2.0
        )
    }));
}

#[test]
fn fixed_size_text_box_is_left_aligned_and_vertically_centered() {
    let theme = crate::theme::test_theme();
    let mut measure = crate::core::measure::NoopMeasure;
    let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    let mut text_box = TextBox::with_id(WidgetId(90));
    text_box.set_fixed_size_logical(200.0, 32.0);

    text_box.set_rect(Rect::new(0.0, 0.0, 240.0, 56.0), &mut layout);

    assert_eq!(text_box.rect(), Rect::new(0.0, 12.0, 200.0, 32.0));
}

#[test]
fn leading_content_inset_reserves_room_for_an_input_icon() {
    let mut text_box = TextBox::with_id(WidgetId(91));
    text_box.set_placeholder("Search...");
    text_box.set_leading_content_inset_logical(28.0);

    let draw_list = paint_laid_out(&mut text_box);

    assert_eq!(text_box.text_pad, 28.0);
    assert!(draw_list.cmds.iter().any(|command| {
        matches!(command, crate::core::paint::DrawCmd::TextLayout { x, .. } if *x == 28.0)
    }));
}

#[test]
fn textbox_widget_emits_plain_edit_and_commit_actions() {
    let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(30)));
    assert_eq!(
        key(&mut box_, KeyCode::Char('x')),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            id: WidgetId(30),
            value: TextPayload::Plain("x".into()),
        }))
    );
    assert!(matches!(
        key(&mut box_, KeyCode::Enter),
        Some(WidgetAction::Control(ControlAction::TextCommitted {
            id: WidgetId(30),
            value: TextPayload::Plain(text),
        })) if text == "x"
    ));
}

#[test]
fn no_change_key_path_cmd_x_without_selection_consumes_without_edit_action() {
    let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(31)));
    box_.set_text("hello");

    let cmd = Modifiers { cmd: true, ..Modifiers::NONE };
    assert_eq!(
        key_with_modifiers(&mut box_, KeyCode::Char('x'), cmd),
        Some(WidgetAction::Consumed)
    );
    assert_eq!(box_.text(), "hello");
}

#[test]
fn no_change_key_path_backspace_at_start_consumes_without_edit_action() {
    let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(32)));
    box_.set_text("a");
    box_.cursor_byte = 0;

    assert_eq!(key(&mut box_, KeyCode::Backspace), Some(WidgetAction::Consumed));
    assert_eq!(box_.text(), "a");
    assert_eq!(box_.cursor_byte(), 0);
}

#[test]
fn no_change_key_path_delete_at_end_consumes_without_edit_action() {
    let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(33)));
    box_.set_text("a");
    box_.cursor_byte = box_.text.len();

    assert_eq!(key(&mut box_, KeyCode::Delete), Some(WidgetAction::Consumed));
    assert_eq!(box_.text(), "a");
    assert_eq!(box_.cursor_byte(), 1);
}

#[test]
fn new_textbox_is_empty() {
    let tb = TextBox::new();
    assert_eq!(tb.text(), "");
    assert_eq!(tb.cursor_byte(), 0);
    assert!(tb.selection.is_none());
}

#[test]
fn insert_char() {
    let mut tb = TextBox::new();
    tb.on_key(KeyCode::Char('a'), Modifiers::NONE);
    assert_eq!(tb.text(), "a");
    assert_eq!(tb.cursor_byte(), 1);
    tb.on_key(KeyCode::Char('b'), Modifiers::NONE);
    assert_eq!(tb.text(), "ab");
    assert_eq!(tb.cursor_byte(), 2);
}

#[test]
fn backspace() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.on_key(KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(tb.text(), "hell");
    assert_eq!(tb.cursor_byte(), 4);
}

#[test]
fn backspace_at_start_does_nothing() {
    let mut tb = TextBox::new();
    tb.set_text("a");
    tb.cursor_byte = 0;
    tb.on_key(KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(tb.text(), "a");
    assert_eq!(tb.cursor_byte(), 0);
}

#[test]
fn backspace_utf8_char() {
    let mut tb = TextBox::new();
    tb.set_text("ab中d");
    // cursor is at end (byte 6 for "ab中d")
    tb.on_key(KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(tb.text(), "ab中");
    assert_eq!(tb.cursor_byte(), 5); // "中" is 3 bytes in UTF-8
}

#[test]
fn cursor_and_backspace_treat_combining_sequence_as_one_grapheme() {
    let mut tb = TextBox::new();
    tb.set_text("e\u{301}");

    tb.on_key(KeyCode::Left, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 0);

    tb.on_key(KeyCode::End, Modifiers::NONE);
    tb.on_key(KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(tb.text(), "");
    assert_eq!(tb.cursor_byte(), 0);
}

#[test]
fn cursor_and_delete_treat_zwj_emoji_as_one_grapheme() {
    let family = "👨‍👩‍👧‍👦";
    let mut tb = TextBox::new();
    tb.set_text(family);

    tb.on_key(KeyCode::Left, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 0);

    tb.on_key(KeyCode::Delete, Modifiers::NONE);
    assert_eq!(tb.text(), "");
}

#[test]
fn layout_exposes_only_grapheme_boundaries_for_pointer_hit_testing() {
    let combining_sequence = "e\u{301}";
    let family = "👨‍👩‍👧‍👦";
    let text = format!("{combining_sequence}{family}");
    let mut tb = TextBox::new();
    tb.set_text(&text);

    paint_laid_out(&mut tb);

    let boundaries: Vec<_> =
        tb.grapheme_offsets.iter().map(|(byte_offset, _)| *byte_offset).collect();
    assert_eq!(boundaries, vec![0, combining_sequence.len(), text.len()]);
}

#[test]
fn mouse_drag_selects_complete_graphemes() {
    struct GraphemeMeasure;

    impl crate::core::measure::TextMeasure for GraphemeMeasure {
        fn measure(&mut self, text: &str, _font_size: f32) -> f32 {
            text.graphemes(true).count() as f32 * 10.0
        }
    }

    let mut text_box = TextBox::new();
    text_box.set_text("ae\u{301}中b");
    let theme = crate::theme::test_theme();
    let mut measure = GraphemeMeasure;
    let mut layout_context =
        LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
    text_box.layout(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_context);
    let text_origin_x = text_box.rect.x + text_box.text_pad;

    assert!(text_box.on_mouse_down(text_origin_x + 10.0, 14.0));
    text_box.on_mouse_drag(text_origin_x + 30.0, 14.0);

    assert_eq!(text_box.selection_text(), Some("e\u{301}中"));
    assert_eq!(text_box.cursor_byte(), "ae\u{301}中".len());
}

#[test]
fn double_click_selects_the_word_at_the_pointer() {
    struct FixedAdvanceMeasure;

    impl crate::core::measure::TextMeasure for FixedAdvanceMeasure {
        fn measure(&mut self, text: &str, _font_size: f32) -> f32 {
            text.graphemes(true).count() as f32 * 10.0
        }
    }

    let mut text_box = TextBox::new();
    text_box.set_text("hello world");
    let theme = crate::theme::test_theme();
    let mut measure = FixedAdvanceMeasure;
    let mut layout_context =
        LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
    text_box.layout(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_context);
    let word_x = text_box.rect.x + text_box.text_pad + 80.0;

    assert!(text_box.on_mouse_down(word_x, 14.0));
    text_box.on_mouse_up();
    assert!(text_box.on_mouse_down(word_x, 14.0));

    assert_eq!(text_box.selection_text(), Some("world"));
}

#[test]
fn triple_click_selects_all_single_line_text() {
    let mut text_box = laid_out_widget(TextBox::new());
    text_box.set_text("hello world");
    let pointer_x = text_box.rect.x + text_box.text_pad;

    for _ in 0..3 {
        assert!(text_box.on_mouse_down(pointer_x, 14.0));
        text_box.on_mouse_up();
    }

    assert_eq!(text_box.selection_text(), Some("hello world"));
}

#[test]
fn set_text_resets_cursor_and_selection() {
    let mut tb = TextBox::new();
    tb.selection = Some((0, 3));
    tb.set_text("new");
    assert_eq!(tb.text(), "new");
    assert_eq!(tb.cursor_byte(), 3);
    assert!(tb.selection.is_none());
}

#[test]
fn cursor_rendering_is_centered() {
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::{LayoutCtx, PaintCtx};

    let mut tb = TextBox::new();
    tb.set_text("a");

    struct DummyMeasure;
    impl crate::core::measure::TextMeasure for DummyMeasure {
        fn measure(&mut self, text: &str, _size: f32) -> f32 {
            text.len() as f32 * 10.0
        }
    }
    let mut measure = DummyMeasure;
    let theme = crate::theme::test_theme();

    let mut lctx = LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
    // text_pad will be 4.0 * dpi = 4.0
    // text_x will be 0.0 + 4.0 = 4.0
    tb.layout(Rect::new(0.0, 0.0, 100.0, 30.0), &mut lctx);

    tb.set_focus(true);
    tb.set_blink(true);

    let mut dl = DrawList::new();
    let mut pctx = PaintCtx::new(&mut dl, &theme, 1.0);
    tb.paint(&mut pctx);

    // Filter FillRect with 0 radius (cursor). Background has corner_radius 3.0, selection is not drawn.
    let cursor_cmds: Vec<_> = dl
        .cmds
        .iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::FillRect { rect, radius, .. } if *radius == 0.0 => Some(rect),
            _ => None,
        })
        .collect();

    assert_eq!(cursor_cmds.len(), 1, "Should emit one FillRect for cursor");

    let cursor_rect = cursor_cmds[0];

    // Logical cursor_x = 10.0 (text length 1 * 10.0)
    // rect.x = 0.0, text_x = 4.0
    // logical drawing center = text_x + cursor_x = 14.0
    // cursor_w = 2.0 * dpi = 2.0
    // centered left bound = 14.0 - 1.0 = 13.0
    assert_eq!(cursor_rect.x, 13.0);
    assert_eq!(cursor_rect.w, 2.0);

    // IME candidate window follows the painted cursor, including the text inset.
    let ime_rect = tb.ime_cursor_rect();
    assert_eq!(ime_rect.x, 14.0);
}

#[test]
fn selection_text_returns_correct_slice() {
    let mut tb = TextBox::new();
    tb.set_text("hello world");
    tb.selection = Some((0, 5));
    assert_eq!(tb.selection_text(), Some("hello"));
    // Order doesn't matter
    tb.selection = Some((11, 6));
    assert_eq!(tb.selection_text(), Some("world"));
}

#[test]
fn delete_selection_replaces_on_insert() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((0, 5));
    tb.on_key(KeyCode::Char('x'), Modifiers::NONE);
    assert_eq!(tb.text(), "x");
    assert_eq!(tb.cursor_byte(), 1);
    assert!(tb.selection.is_none());
}

#[test]
fn backspace_deletes_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((0, 3));
    tb.on_key(KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(tb.text(), "lo");
    assert_eq!(tb.cursor_byte(), 0);
    assert!(tb.selection.is_none());
}

#[test]
fn cursor_left_right() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 0;
    tb.on_key(KeyCode::Right, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 1);
    tb.on_key(KeyCode::Right, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 2);
    tb.on_key(KeyCode::Left, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 1);
}

#[test]
fn cursor_home_end() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 1;
    tb.on_key(KeyCode::Home, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 0);
    tb.on_key(KeyCode::End, Modifiers::NONE);
    assert_eq!(tb.cursor_byte(), 3);
}

#[test]
fn shift_right_creates_selection() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 0;
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    tb.on_key(KeyCode::Right, shift);
    assert!(tb.selection.is_some());
    let (anchor, cursor) = tb.selection.unwrap();
    assert_eq!(anchor, 0);
    assert_eq!(cursor, 1);
    assert_eq!(tb.cursor_byte(), 1);
}

#[test]
fn shift_left_creates_selection() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 3;
    let shift = Modifiers { shift: true, ..Modifiers::NONE };
    tb.on_key(KeyCode::Left, shift);
    assert!(tb.selection.is_some());
}

#[test]
fn left_collapses_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((0, 5));
    tb.on_key(KeyCode::Left, Modifiers::NONE);
    assert!(tb.selection.is_none());
    assert_eq!(tb.cursor_byte(), 0);
}

#[test]
fn right_collapses_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((0, 5));
    tb.on_key(KeyCode::Right, Modifiers::NONE);
    assert!(tb.selection.is_none());
    assert_eq!(tb.cursor_byte(), 5);
}

#[test]
fn select_all_works() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.select_all();
    assert_eq!(tb.selection, Some((0, 5)));
    assert_eq!(tb.selection_text(), Some("hello"));
}

#[test]
fn selection_text_utf8() {
    let mut tb = TextBox::new();
    tb.set_text("a中b");
    tb.selection = Some((1, 4)); // "中" in UTF-8: a=byte0, 中=bytes 1-3, b=byte4
    assert_eq!(tb.selection_text(), Some("中"));
}

#[test]
fn set_focus_clears_selection() {
    let mut tb = TextBox::new();
    tb.set_focus(true);
    tb.set_text("hello");
    tb.selection = Some((0, 3));
    tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
    tb.set_focus(false);
    assert!(tb.selection.is_none());
    assert!(!tb.has_preedit());
}

#[test]
fn ime_preedit_updates_state() {
    let mut tb = TextBox::new();
    tb.set_focus(true);
    tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
    assert!(tb.has_preedit());
    assert_eq!(tb.preedit, "ni");
}

#[test]
fn ime_commit_inserts_text() {
    let mut tb = TextBox::new();
    tb.set_focus(true);
    tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
    tb.on_ime(&TextBoxIme::Commit("你好".into()));
    assert!(!tb.has_preedit());
    assert_eq!(tb.text(), "你好");
    assert_eq!(tb.cursor_byte(), 6);
}

#[test]
fn text_box_ime_debug_never_exposes_text() {
    fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

    const SECRET: &str = "text-box-ime-secret";
    let preedit = TextBoxIme::Preedit { text: SECRET.to_owned(), cursor: Some((0, 4)) };
    let commit = TextBoxIme::Commit(SECRET.to_owned());

    assert_zeroize_on_drop::<TextBoxIme>();
    assert!(!format!("{preedit:?}").contains(SECRET));
    assert!(!format!("{commit:?}").contains(SECRET));
}

#[test]
fn layout_computes_cursor_x() {
    use crate::core::measure::NoopMeasure;

    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 2;

    let theme = crate::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut ctx = LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
    let rect = Rect::new(10.0, 0.0, 200.0, 28.0);
    tb.layout(rect, &mut ctx);
    assert_eq!(tb.rect.x, 10.0);
    // cursor_x is 0.0 with NoopMeasure since it returns 0 for everything
    assert_eq!(tb.cursor_x, 0.0);
}

#[test]
fn paint_empty_shows_placeholder() {
    use crate::core::paint::{DrawCmd, DrawList};

    let mut tb = TextBox::new();
    tb.set_placeholder("Search...");
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
    tb.blink_on = true;
    tb.focused = true;

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut shaper = shaping::Shaper::new().unwrap();
    let mut pc = PaintCtx {
        global_alpha: 1.0,
        list: &mut dl,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        shaper: Some(&mut shaper),
    };
    tb.paint(&mut pc);

    // Should have: bg, border, placeholder text, cursor
    assert!(dl.cmds.len() >= 4);
    let texts: Vec<_> =
        dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).collect();
    assert_eq!(texts.len(), 1, "expected 1 text (placeholder)");
}

fn input_chrome_colors(theme: &crate::theme::Theme, focused: bool) -> ([f32; 4], [f32; 4]) {
    use crate::core::paint::{DrawCmd, DrawList};

    let mut input = TextBox::new();
    input.set_focus(focused);
    input.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
    let mut draw_list = DrawList::new();
    input.paint_chrome(&mut PaintCtx::new(&mut draw_list, theme, 1.0));
    let background = draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            DrawCmd::FillRect { color, .. } => Some(*color),
            _ => None,
        })
        .expect("input paints its actual background");
    let border = draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            DrawCmd::StrokeRect { color, .. } => Some(*color),
            _ => None,
        })
        .expect("input paints its actual border");
    (background, border)
}

#[test]
fn input_borders_are_visible_on_their_actual_builtin_surfaces() {
    const MINIMUM_CONTROL_CONTRAST: f32 = 3.0;
    let luminance = |color: [f32; 4]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
    for definition in [
        crate::theme::ThemeDefinition::default_light(),
        crate::theme::ThemeDefinition::default_dark(),
    ] {
        let theme = crate::theme::Theme::from_definition(&definition);
        for focused in [false, true] {
            let (background, border) = input_chrome_colors(&theme, focused);
            for surface in [
                theme.palette.bg_base,
                theme.palette.bg_surface,
                theme.palette.bg_elevated,
                background,
            ] {
                let outside = luminance(surface);
                let outline = luminance(border);
                let contrast = (outside.max(outline) + 0.05) / (outside.min(outline) + 0.05);
                assert!(
                    contrast >= MINIMUM_CONTROL_CONTRAST,
                    "{} focused={focused}: border contrast {contrast}",
                    theme.name
                );
            }
        }
    }
}

#[test]
fn placeholder_text_remains_readable_in_builtin_input_states() {
    use crate::core::paint::{DrawCmd, DrawList};

    const MINIMUM_TEXT_CONTRAST: f32 = 4.5;
    let luminance = |color: [f32; 4]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
    let mut shaper = shaping::Shaper::new().expect("placeholder test needs text shaping");
    for definition in [
        crate::theme::ThemeDefinition::default_light(),
        crate::theme::ThemeDefinition::default_dark(),
    ] {
        let theme = crate::theme::Theme::from_definition(&definition);
        for focused in [false, true] {
            let mut input = TextBox::new();
            input.set_placeholder("搜索笔记");
            input.set_focus(focused);
            input.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
            let mut draw_list = DrawList::new();
            let mut context = PaintCtx::new(&mut draw_list, &theme, 1.0);
            context.shaper = Some(&mut shaper);
            input.paint(&mut context);
            let label = draw_list
                .cmds
                .iter()
                .find_map(|command| match command {
                    DrawCmd::TextLayout { layout, color, .. } if layout.text == "搜索笔记" => {
                        Some(*color)
                    }
                    _ => None,
                })
                .expect("empty input paints its placeholder");
            let background = luminance(input_chrome_colors(&theme, focused).0);
            let foreground = luminance(label) * label[3] + background * (1.0 - label[3]);
            let contrast =
                (background.max(foreground) + 0.05) / (background.min(foreground) + 0.05);
            assert!(
                contrast >= MINIMUM_TEXT_CONTRAST,
                "{} focused={focused}: placeholder contrast {contrast}",
                theme.name
            );
        }
    }
}

#[test]
fn paint_shows_text_not_placeholder() {
    use crate::core::paint::{DrawCmd, DrawList};

    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.set_placeholder("Search...");
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut shaper = shaping::Shaper::new().unwrap();
    let mut pc = PaintCtx {
        global_alpha: 1.0,
        list: &mut dl,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        shaper: Some(&mut shaper),
    };
    tb.paint(&mut pc);

    let texts: Vec<_> =
        dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).collect();
    assert_eq!(texts.len(), 1);
    if let DrawCmd::TextLayout { layout, .. } = &texts[0] {
        assert_eq!(&layout.text, "hello");
    }
}

#[test]
fn seamless_chrome_removes_the_idle_input_frame() {
    use crate::core::paint::DrawCmd;

    let mut text_box = TextBox::new();
    text_box.set_text("沉浸式标题");
    text_box.set_chrome(TextBoxChrome::Seamless);

    let draw_list = paint_laid_out(&mut text_box);

    assert!(
        !draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { .. } | DrawCmd::StrokeRect { .. }
        ))
    );
    assert!(draw_list.cmds.iter().any(|command| matches!(command, DrawCmd::TextLayout { .. })));
}

#[test]
fn masked_textbox_paints_bullets_and_commits_sensitive_payload() {
    let mut box_ = TextBox::new();
    box_.set_echo_mode(EchoMode::Masked);
    box_.set_text("secret-value");

    let draw_list = paint_laid_out(&mut box_);
    assert!(!format!("{:?}", draw_list.cmds).contains("secret-value"));

    assert!(box_.on_key(KeyCode::Enter, Modifiers::NONE));
    assert!(matches!(
        box_.take_committed_payload(),
        Some(TextPayload::Sensitive(secret)) if secret.expose() == "secret-value"
    ));
}

#[test]
fn masked_text_uses_one_bullet_per_grapheme() {
    let mut text_box = TextBox::new();
    text_box.set_echo_mode(EchoMode::Masked);
    text_box.set_text("e\u{301}👨‍👩‍👧‍👦");

    assert_eq!(text_box.display_text(), "••");
}

#[test]
fn byte_limit_does_not_insert_partial_grapheme_from_ime_or_clipboard() {
    let mut text_box = TextBox::new();
    text_box.set_max_len_bytes(1);
    text_box.set_focus(true);

    text_box.on_ime(&TextBoxIme::Commit("e\u{301}".into()));
    assert_eq!(text_box.text(), "");

    let clipboard_text = Rc::new(RefCell::new(Some("e\u{301}".to_owned())));
    let mut clipboard = TestClipboard { text: clipboard_text };
    let command = Modifiers { cmd: true, ..Modifiers::NONE };
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);
    assert_eq!(text_box.text(), "");
}

#[test]
fn clipboard_paste_normalizes_line_breaks_for_single_line_input() {
    let clipboard_text = Rc::new(RefCell::new(Some("第一行\r\n第二行\n第三行\r第四行".to_owned())));
    let mut clipboard = TestClipboard { text: clipboard_text };
    let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(46)));
    text_box.set_focus(true);
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    let command = Modifiers { cmd: true, ..Modifiers::NONE };

    let action =
        text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);

    assert!(matches!(action, Some(WidgetAction::Control(ControlAction::TextEdited { .. }))));
    assert_eq!(text_box.text(), "第一行 第二行 第三行 第四行");
}

#[test]
fn rejected_grapheme_paste_preserves_the_replaced_selection() {
    let clipboard_text = Rc::new(RefCell::new(Some("e\u{301}".to_owned())));
    let mut clipboard = TestClipboard { text: clipboard_text };
    let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(47)));
    text_box.set_max_len_bytes(1);
    text_box.set_text("a");
    text_box.set_focus(true);
    text_box.select_all();
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    let command = Modifiers { cmd: true, ..Modifiers::NONE };

    let action =
        text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);

    assert_eq!(action, Some(WidgetAction::Consumed));
    assert_eq!(text_box.text(), "a");
    assert_eq!(text_box.selection_text(), Some("a"));
}

#[test]
fn widget_event_uses_context_clipboard_without_component_configuration() {
    let clipboard_text = Rc::new(RefCell::new(Some("粘贴内容".to_owned())));
    let mut clipboard = TestClipboard { text: Rc::clone(&clipboard_text) };
    let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(44)));
    text_box.set_text("原文");
    text_box.set_focus(true);
    text_box.select_all();
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    let command = Modifiers { cmd: true, ..Modifiers::NONE };

    let paste = text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);
    assert!(matches!(
        paste,
        Some(WidgetAction::Control(ControlAction::TextEdited {
            value: TextPayload::Plain(text),
            ..
        })) if text == "粘贴内容"
    ));

    text_box.select_all();
    let _ = text_box.on_event(&Event::KeyDown(KeyCode::Char('c'), command), &mut event_context);
    assert_eq!(clipboard_text.borrow().as_deref(), Some("粘贴内容"));

    text_box.select_all();
    let cut = text_box.on_event(&Event::KeyDown(KeyCode::Char('x'), command), &mut event_context);
    assert!(matches!(cut, Some(WidgetAction::Control(ControlAction::TextEdited { .. }))));
    assert_eq!(text_box.text(), "");
}

#[test]
fn cut_preserves_selection_when_clipboard_write_fails() {
    let mut clipboard = RejectingClipboard;
    let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(45)));
    text_box.set_text("不能丢失");
    text_box.set_focus(true);
    text_box.select_all();
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    let command = Modifiers { cmd: true, ..Modifiers::NONE };

    let action =
        text_box.on_event(&Event::KeyDown(KeyCode::Char('x'), command), &mut event_context);

    assert_eq!(action, Some(WidgetAction::Consumed));
    assert_eq!(text_box.text(), "不能丢失");
    assert_eq!(text_box.selection_text(), Some("不能丢失"));
}

#[test]
fn masked_text_and_preedit_storage_never_debug_plaintext() {
    let mut text_box = TextBox::new();
    text_box.set_echo_mode(EchoMode::Masked);
    text_box.set_text("stored-api-key");
    text_box.set_focus(true);
    text_box.on_ime(&TextBoxIme::Preedit { text: "preedit-secret".into(), cursor: Some((0, 14)) });

    let stored_text_debug = format!("{:?}", text_box.text);
    let preedit_debug = format!("{:?}", text_box.preedit);
    assert!(!stored_text_debug.contains("stored-api-key"));
    assert!(!preedit_debug.contains("preedit-secret"));
}

#[test]
fn masked_copy_and_cut_never_send_plaintext_to_clipboard() {
    let clipboard_text = Rc::new(RefCell::new(None));
    let mut clipboard = TestClipboard { text: Rc::clone(&clipboard_text) };
    let mut text_box = TextBox::new();
    text_box.set_echo_mode(EchoMode::Masked);
    text_box.set_text("clipboard-secret");
    text_box.set_focus(true);
    text_box.select_all();

    let command = Modifiers { cmd: true, ..Modifiers::NONE };
    let theme = crate::theme::test_theme();
    let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    assert!(
        text_box
            .on_event(&Event::KeyDown(KeyCode::Char('c'), command), &mut event_context)
            .is_some()
    );
    assert!(clipboard_text.borrow().is_none());

    assert!(
        text_box
            .on_event(&Event::KeyDown(KeyCode::Char('x'), command), &mut event_context)
            .is_some()
    );
    assert!(clipboard_text.borrow().is_none());
    assert_eq!(text_box.text(), "");
}

#[test]
fn masked_text_edit_emits_only_sensitive_payload() {
    let mut masked = laid_out_widget(TextBox::with_id(WidgetId(41)));
    masked.set_echo_mode(EchoMode::Masked);
    masked.sync_text("never-print-m");
    let action = key(&mut masked, KeyCode::Char('e'));
    assert!(matches!(
        action,
        Some(WidgetAction::Control(ControlAction::TextEdited {
            id: WidgetId(41),
            value: TextPayload::Sensitive(_),
        }))
    ));
    assert!(!format!("{action:?}").contains("never-print-me"));

    masked.selection = Some((0, masked.text.len()));
    assert!(matches!(
        key(&mut masked, KeyCode::Delete),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            value: TextPayload::Sensitive(_),
            ..
        }))
    ));

    masked.set_text("ab");
    masked.cursor_byte = masked.text.len();
    assert!(matches!(
        key(&mut masked, KeyCode::Backspace),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            value: TextPayload::Sensitive(_),
            ..
        }))
    ));

    masked.set_text("ab");
    masked.cursor_byte = 0;
    assert!(matches!(
        key(&mut masked, KeyCode::Delete),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            value: TextPayload::Sensitive(_),
            ..
        }))
    ));

    masked.set_text("");
    masked.cursor_byte = 0;
    let paste_modifiers = Modifiers { cmd: true, ..Modifiers::NONE };
    let clipboard_text = Rc::new(RefCell::new(Some(String::from("paste"))));
    let mut clipboard = TestClipboard { text: clipboard_text };
    let theme = crate::theme::test_theme();
    let mut event_ctx = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
    assert!(matches!(
        masked.on_event(&Event::KeyDown(KeyCode::Char('v'), paste_modifiers), &mut event_ctx),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            value: TextPayload::Sensitive(_),
            ..
        }))
    ));

    masked.set_text("");
    masked.cursor_byte = 0;
    assert!(matches!(
        masked.on_event(&Event::ImeCommit(String::from("你好")), &mut event_ctx),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            value: TextPayload::Sensitive(_),
            ..
        }))
    ));

    let mut plain = laid_out_widget(TextBox::with_id(WidgetId(42)));
    assert_eq!(
        key(&mut plain, KeyCode::Char('p')),
        Some(WidgetAction::Control(ControlAction::TextEdited {
            id: WidgetId(42),
            value: TextPayload::Plain("p".into()),
        }))
    );
}

#[test]
fn masked_preedit_uses_bullets_for_layout_and_paint() {
    use crate::core::measure::TextMeasure;
    use crate::core::paint::{DrawCmd, DrawList};

    struct DistinguishingMeasure;

    impl TextMeasure for DistinguishingMeasure {
        fn measure(&mut self, text: &str, _size: f32) -> f32 {
            text.chars().map(|ch| if ch == MASKED_ECHO_GLYPH { 7.0 } else { 13.0 }).sum()
        }
    }

    let mut tb = TextBox::new();
    tb.set_echo_mode(EchoMode::Masked);
    tb.set_text("secret");
    tb.set_focus(true);
    tb.cursor_byte = tb.text.len();
    tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((0, 1)) });

    let theme = crate::theme::test_theme();
    let mut measure = DistinguishingMeasure;
    let mut layout_ctx =
        LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
    tb.layout(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_ctx);

    assert_eq!(tb.preedit_width, 14.0);
    assert_eq!(tb.preedit_cursor_x, 7.0);

    let mut draw_list = DrawList::new();
    let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
    let mut paint_ctx = PaintCtx {
        global_alpha: 1.0,
        list: &mut draw_list,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        shaper: Some(&mut shaper),
    };
    tb.paint(&mut paint_ctx);

    let debug_output = format!("{:?}", draw_list.cmds);
    assert!(!debug_output.contains("ni"));

    let text_layouts: Vec<_> =
        draw_list.cmds.iter().filter(|cmd| matches!(cmd, DrawCmd::TextLayout { .. })).collect();
    assert_eq!(text_layouts.len(), 2, "expected masked text plus masked preedit");
    if let DrawCmd::TextLayout { layout, .. } = &text_layouts[1] {
        assert_eq!(&layout.text, "••");
    }

    let underline_count = draw_list
        .cmds
        .iter()
        .filter(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, radius, .. }
                if *radius == 0.0 && rect.w == 14.0 && rect.h == 1.5
            )
        })
        .count();
    assert_eq!(underline_count, 1, "expected masked preedit underline to remain visible");
}

#[test]
fn paint_hides_cursor_when_blink_off() {
    use crate::core::paint::{DrawCmd, DrawList};

    let mut tb = TextBox::new();
    tb.set_text("x");
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
    tb.focused = true;
    tb.blink_on = false;

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut shaper = shaping::Shaper::new().unwrap();
    let mut pc = PaintCtx {
        global_alpha: 1.0,
        list: &mut dl,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        shaper: Some(&mut shaper),
    };
    tb.paint(&mut pc);

    let fills: Vec<_> = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).collect();
    // Border is now StrokeRect, so only bg fill remains (no cursor fill)
    assert_eq!(fills.len(), 1, "expected only bg fill, no cursor");
}

#[test]
fn paint_preedit_draws_text_and_underline() {
    use crate::core::paint::{DrawCmd, DrawList};

    let mut tb = TextBox::new();
    tb.set_text("hello ");
    tb.set_focus(true);
    tb.cursor_byte = 6;
    tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
    tb.on_ime(&TextBoxIme::Preedit { text: "世界".into(), cursor: Some((0, 6)) });
    tb.preedit_width = 28.0; // simulate layout

    let theme = crate::theme::test_theme();
    let mut dl = DrawList::new();
    let mut shaper = shaping::Shaper::new().unwrap();
    let mut pc = PaintCtx {
        global_alpha: 1.0,
        list: &mut dl,
        theme: &theme,
        dpi: 1.0,
        offset: (0.0, 0.0),
        shaper: Some(&mut shaper),
    };
    tb.paint(&mut pc);

    let texts: Vec<_> =
        dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).collect();
    assert_eq!(texts.len(), 2, "expected main text + preedit text");
    if let DrawCmd::TextLayout { layout, .. } = &texts[1] {
        assert_eq!(&layout.text, "世界");
    }
}

#[test]
fn sync_text_overwrites_when_different() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.cursor_byte = 3;
    tb.sync_text("world");
    assert_eq!(tb.text(), "world");
    assert!(tb.selection.is_none());
}

#[test]
fn sync_text_noop_when_same() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.cursor_byte = 3;
    tb.sync_text("hello");
    assert_eq!(tb.text(), "hello");
    assert_eq!(tb.cursor_byte(), 3); // preserved!
}

#[test]
fn sync_text_clamps_cursor_when_shorter() {
    let mut tb = TextBox::new();
    tb.set_text("long text");
    tb.sync_text("short");
    assert_eq!(tb.cursor_byte(), 5); // clamped to "short".len()
}

#[test]
fn sync_text_clamps_cursor_to_utf8_char_boundary() {
    let mut tb = TextBox::new();
    tb.set_text("123456789");

    tb.sync_text("H2AI 战略");

    assert_eq!(tb.cursor_byte(), 8);
    assert!(tb.text().is_char_boundary(tb.cursor_byte()));
    paint_laid_out(&mut tb);
}

#[test]
fn sync_text_clamps_cursor_to_grapheme_boundary() {
    let mut tb = TextBox::new();
    tb.set_text("x");

    tb.sync_text("e\u{301}");

    assert_eq!(tb.cursor_byte(), 0);
    paint_laid_out(&mut tb);
}

#[test]
fn ime_cursor_rect_includes_preedit_width() {
    let mut tb = TextBox::new();
    tb.rect = Rect::new(10.0, 0.0, 200.0, 28.0);
    tb.cursor_x = 50.0;
    tb.preedit_cursor_x = 30.0;
    let r = tb.ime_cursor_rect();
    assert_eq!(r.x, 10.0 + 50.0 + 30.0);
}

#[test]
fn ime_cursor_rect_no_preedit() {
    let mut tb = TextBox::new();
    tb.rect = Rect::new(10.0, 0.0, 200.0, 28.0);
    tb.cursor_x = 50.0;
    tb.preedit_width = 0.0;
    let r = tb.ime_cursor_rect();
    assert_eq!(r.x, 10.0 + 50.0);
}

#[test]
fn enter_key_records_committed_payload_without_widget_binding() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    let result = tb.on_key(KeyCode::Enter, Modifiers::NONE);
    assert!(result);
    assert_eq!(tb.take_committed_payload(), Some(TextPayload::Plain("hello".into())));
}

#[test]
fn escape_key_is_consumed_without_action() {
    let mut tb = laid_out_widget(TextBox::with_id(WidgetId(43)));
    assert_eq!(key(&mut tb, KeyCode::Escape), Some(WidgetAction::Consumed));
}

#[test]
fn delete_removes_char_after_cursor() {
    let mut tb = TextBox::new();
    tb.set_text("abc");
    tb.cursor_byte = 1;
    tb.on_key(KeyCode::Delete, Modifiers::NONE);
    assert_eq!(tb.text(), "ac");
    assert_eq!(tb.cursor_byte(), 1);
}

#[test]
fn delete_removes_selection() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.selection = Some((1, 4));
    tb.on_key(KeyCode::Delete, Modifiers::NONE);
    assert_eq!(tb.text(), "ho");
    assert_eq!(tb.cursor_byte(), 1);
    assert!(tb.selection.is_none());
}

#[test]
fn alt_left_right_moves_by_word() {
    let mut tb = TextBox::new();
    tb.set_text("hello world!");
    tb.cursor_byte = 0;
    let alt = Modifiers { alt: true, ..Modifiers::NONE };

    tb.on_key(KeyCode::Right, alt);
    assert_eq!(tb.cursor_byte(), 6);

    tb.on_key(KeyCode::Right, alt);
    assert_eq!(tb.cursor_byte(), 12);

    tb.on_key(KeyCode::Left, alt);
    assert_eq!(tb.cursor_byte(), 6);

    tb.on_key(KeyCode::Left, alt);
    assert_eq!(tb.cursor_byte(), 0);
}

#[test]
fn cmd_left_right_moves_to_edges() {
    let mut tb = TextBox::new();
    tb.set_text("hello world!");
    tb.cursor_byte = 5;
    let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

    tb.on_key(KeyCode::Left, cmd);
    assert_eq!(tb.cursor_byte(), 0);

    tb.cursor_byte = 5;
    tb.on_key(KeyCode::Right, cmd);
    assert_eq!(tb.cursor_byte(), 12);
}

#[test]
fn up_down_are_consumed_without_mutating_text() {
    let mut tb = TextBox::new();
    tb.set_text("hello");
    tb.cursor_byte = 3;

    let res_up = tb.on_key(KeyCode::Up, Modifiers::NONE);
    assert!(res_up);
    assert_eq!(tb.text(), "hello");
    assert_eq!(tb.cursor_byte(), 3);

    let res_down = tb.on_key(KeyCode::Down, Modifiers::NONE);
    assert!(res_down);
    assert_eq!(tb.text(), "hello");
    assert_eq!(tb.cursor_byte(), 3);

    let res_pgup = tb.on_key(KeyCode::PageUp, Modifiers::NONE);
    assert!(res_pgup);

    let res_pgdn = tb.on_key(KeyCode::PageDown, Modifiers::NONE);
    assert!(res_pgdn);
}
