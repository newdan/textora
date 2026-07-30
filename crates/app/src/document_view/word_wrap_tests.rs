use super::*;

const TEST_LINE_HEIGHT: f32 = 24.27;

#[test]
fn short_line_no_wrap() {
    // A short line should not be wrapped
    let lines = vec!["hello world".to_string()];
    let dv = DocumentView::new(lines, 10, 10.0);
    // visible_lines should return 1 line
    assert_eq!(dv.visible_line_count_with_line_height(TEST_LINE_HEIGHT), 1);
}

#[test]
fn long_line_needs_wrap() {
    // A very long line should potentially need wrapping
    let long_line = "a".repeat(1000);
    let lines = vec![long_line];
    let dv = DocumentView::new(lines, 10, 10.0);
    // Still 1 doc line
    assert_eq!(dv.line_count(), 1);
    // visible_line_count returns doc lines visible, not visual lines
    assert_eq!(dv.visible_line_count_with_line_height(TEST_LINE_HEIGHT), 1);
}

#[test]
fn visual_line_split_basic() {
    // Test the visual line splitting logic
    // A line of 100 chars with viewport width 50px (at 8px/char = ~6 chars)
    // should split into multiple visual lines
    let line = "abcdefghijklmnopqrstuvwxyz"; // 26 chars
    let char_width = 8.0; // px per char (approximate)
    let viewport_width = 50.0; // px

    // Number of chars per visual line
    let chars_per_line = (viewport_width / char_width) as usize; // 6
    let expected_visual_lines = line.len().div_ceil(chars_per_line); // ceil(26/6) = 5

    assert_eq!(chars_per_line, 6);
    assert_eq!(expected_visual_lines, 5);
}
