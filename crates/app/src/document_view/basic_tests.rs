#![allow(deprecated)]
use super::*;
use core::types::ByteIndex;

use std::io::Write;

const TEST_LINE_HEIGHT: f32 = 24.27;

fn make_lines(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("line {i}")).collect()
}

#[test]
fn new_empty() {
    let dv = DocumentView::new(vec![], 10, 10.0);
    // Empty doc has one empty line
    assert_eq!(dv.line_count(), 1);
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT).len(), 1);
}

#[test]
fn new_with_lines() {
    let dv = DocumentView::new(make_lines(100), 10, 10.0);
    assert_eq!(dv.line_count(), 100);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 10);
    assert_eq!(vis[0], "line 0");
    assert_eq!(vis[9], "line 9");
}

#[test]
fn scroll_changes_visible_lines() {
    let mut dv = DocumentView::new(make_lines(100), 10, 10.0);
    dv.presentation.display.viewport.scroll_top = 50.0;
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 10);
    assert_eq!(vis[0], "line 50");
    assert_eq!(vis[9], "line 59");
}

#[test]
fn scroll_up_returns_to_top() {
    let mut dv = DocumentView::new(make_lines(100), 10, 10.0);
    dv.presentation.display.viewport.scroll_top = 50.0;
    dv.presentation.display.viewport.scroll_anchor = ui::viewport::ScrollAnchor::new(0, 0.0);
    dv.presentation.display.viewport.derive_scroll_top(&dv.presentation.display.display_map, 14.0);
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "line 0");
}

#[test]
fn scroll_clamps_at_bottom() {
    let mut dv = DocumentView::new(make_lines(20), 10, 10.0);
    dv.presentation.display.viewport.scroll_top = 10.0;
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 10);
    assert_eq!(vis[0], "line 10");
    assert_eq!(vis[9], "line 19");
}

#[test]
fn resize_updates_visible_rows() {
    let mut dv = DocumentView::new(make_lines(100), 10, 10.0);
    dv.resize(20, 20.0);
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT).len(), 20);
}

#[test]
fn from_file_loads_content() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    writeln!(tmp, "hello").unwrap();
    writeln!(tmp, "world").unwrap();
    writeln!(tmp, "foo").unwrap();

    let dv = DocumentView::from_file(tmp.path(), 10, 10.0).unwrap();
    // writeln! adds trailing newline -> TextBuffer counts 4 logical lines
    assert_eq!(dv.line_count(), 4);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "hello");
    assert_eq!(vis[1], "world");
    assert_eq!(vis[2], "foo");
    assert_eq!(vis[3], "");
    assert_eq!(dv.file_path, Some(tmp.path().to_path_buf()));
}

#[test]
fn from_file_nonexistent() {
    let result = DocumentView::from_file(Path::new("/no/such/file"), 10, 10.0);
    assert!(result.is_err());
}

#[test]
fn from_file_empty() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let dv = DocumentView::from_file(tmp.path(), 10, 10.0).unwrap();
    // Empty file has one empty line
    assert_eq!(dv.line_count(), 1);
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT).len(), 1);
}

#[test]
fn save_rejects_external_change_since_load() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"original").unwrap();

    let mut document = DocumentView::from_file(tmp.path(), 10, 10.0).unwrap();
    document.dirty = true;
    std::fs::write(tmp.path(), b"external change").unwrap();

    let error = document.save().expect_err("save must reject an external disk change");

    assert!(matches!(error, DocumentSaveError::ConcurrentModification));
    assert_eq!(std::fs::read_to_string(tmp.path()).unwrap(), "external change");
}

#[test]
fn document_save_revision_tracks_edits_and_successful_save() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"original").unwrap();

    let mut document = DocumentView::from_file(tmp.path(), 10, 10.0).unwrap();
    let initial_content_revision = document.content_revision();
    document.insert_at_cursor(b"local ");

    assert!(document.content_revision() > initial_content_revision);
    assert!(document.dirty);
    document.save().expect("unchanged disk revision should save");

    assert!(!document.dirty);
    assert!(document.disk_revision.is_some());
}

#[test]
fn document_save_uses_typed_errors_and_preserves_dirty_after_concurrent_change() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"original").unwrap();

    let mut document = DocumentView::from_file(tmp.path(), 10, 10.0).unwrap();
    document.insert_at_cursor(b"local ");
    std::fs::write(tmp.path(), b"external").unwrap();

    let error = document.save().expect_err("external disk change must reject save");

    assert!(matches!(error, DocumentSaveError::ConcurrentModification));
    assert!(document.dirty);
    assert_eq!(document.file_path, Some(tmp.path().to_path_buf()));
}

#[test]
fn document_save_as_new_path_starts_a_new_disk_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.md");
    let target = directory.path().join("saved-copy.md");
    std::fs::write(&source_path, b"source").unwrap();

    let mut document = DocumentView::from_file(&source_path, 10, 10.0).unwrap();
    document.save_as(&target).expect("save as should create a new file");

    assert_eq!(document.file_path, Some(target.clone()));
    assert!(document.disk_revision.as_ref().is_some_and(|revision| revision.path == target));
    assert!(!document.dirty);
}

#[test]
fn external_content_restores_edit_position() {
    let path = Path::new("notes.md");
    let mut document = DocumentView::from_external_content(path, "hello\nworld", 10, 10.0);
    let anchor = ui::viewport::ScrollAnchor::new(1, 2.5);

    document.restore_edit_position(8, Some(6), anchor);

    assert_eq!(document.cursor_offset(), ByteIndex(8));
    assert_eq!(document.cursor().selection_anchor, Some(6));
    assert_eq!(document.presentation.display.viewport.scroll_anchor.doc_line, anchor.doc_line);
    assert_eq!(
        document.presentation.display.viewport.scroll_anchor.pixel_offset,
        anchor.pixel_offset
    );
}

#[test]
fn viewport_only_shapes_visible_lines() {
    // plans.md §5: 100k lines, viewport of 50 — shape call ≤ viewport × 2.
    // At the DocumentView level, we verify visible_lines() returns at most
    // viewport rows, and visible_line() returns None for out-of-range indices.
    let lines: Vec<String> = (0..100_000).map(|i| format!("line {i}")).collect();
    let dv = DocumentView::new(lines, 50, 10.0);

    assert_eq!(dv.line_count(), 100_000);

    // Only 50 lines visible
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 50, "should return exactly viewport rows");
    assert_eq!(vis[0], "line 0");
    assert_eq!(vis[49], "line 49");

    // visible_line() returns correct data for visible range
    let first = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("first visible line");
    assert_eq!(first, &b"line 0"[..]);
    let last = dv.visible_line_with_line_height(49, TEST_LINE_HEIGHT).expect("last visible line");
    assert_eq!(last, &b"line 49"[..]);

    // Out-of-viewport indices return None (no hidden iteration)
    assert!(
        dv.visible_line_with_line_height(50, TEST_LINE_HEIGHT).is_none(),
        "index 50 should be out of range"
    );
    assert!(
        dv.visible_line_with_line_height(100_000, TEST_LINE_HEIGHT).is_none(),
        "far out of range"
    );

    // After scrolling, visible range shifts
    let mut dv = dv;
    dv.presentation.display.viewport.scroll_by(1000.0);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 50);
    assert_eq!(vis[0], "line 1000");
    assert_eq!(dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap(), &b"line 1000"[..]);
}

#[test]
fn viewport_scrolling_changes_visible_lines() {
    let lines: Vec<String> = (0..100_000).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 50, 10.0);

    // Scroll to middle (Stage 5: set anchor directly)
    dv.presentation.display.viewport.scroll_top = 50000.0;
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 50);
    assert_eq!(vis[0], "line 50000");
    assert_eq!(vis[49], "line 50049");
}

#[test]
fn viewport_visible_lines_never_exceeds_total() {
    // File smaller than viewport
    let lines: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
    let dv = DocumentView::new(lines, 50, 10.0);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 5, "should not exceed total lines");
}

#[test]
fn cursor_move_up_basic() {
    let mut dv = DocumentView::new(vec!["line0".to_string(), "line1".to_string()], 10, 10.0);
    dv.cursor_move_to_offset(6); // start of line1
    dv.cursor_move_up();
    assert!(dv.cursor().offset < ByteIndex(6), "should be on line0");
}

#[test]
fn cursor_move_down_basic() {
    let mut dv = DocumentView::new(vec!["line0".to_string(), "line1".to_string()], 10, 10.0);
    dv.cursor_move_down();
    assert!(dv.cursor().offset >= ByteIndex(6), "should be on line1");
}

#[test]
fn cursor_move_to_line_start_basic() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    dv.cursor_move_to_offset(5);
    dv.cursor_move_to_line_start();
    assert_eq!(dv.cursor().offset, ByteIndex(0));
}

#[test]
fn cursor_move_to_line_end_basic() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    dv.cursor_move_to_line_end();
    assert_eq!(dv.cursor().offset, ByteIndex(11));
}

#[test]
fn crlf_line_count_correct() {
    // Verify CRLF files have correct line count (not doubled)
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    use std::io::Write;
    // Write CRLF content
    tmp.write_all(
        b"line1
line2
line3
",
    )
    .unwrap();
    tmp.flush().unwrap();

    let dv = DocumentView::from_file(tmp.path(), 10, 10.0).unwrap();
    // TextBuffer counts trailing empty line after final newline
    assert_eq!(dv.line_count(), 4);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "line1");
    assert_eq!(vis[1], "line2");
    assert_eq!(vis[2], "line3");
    assert_eq!(vis[3], "");
}

// ========== TDD tests: TextBuffer delegation (S1) ==========

#[test]
fn insert_at_cursor_delegates_to_textbuffer() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    // cursor at 0, insert "X" before "hello"
    dv.insert_at_cursor(b"X");
    // cursor should advance by 1
    assert_eq!(dv.cursor().offset, ByteIndex(1));
    // visible line should now start with X
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "Xhello");
}

#[test]
fn delete_backward_grapheme_aware_zwj_emoji() {
    // ZWJ family emoji: 👨‍👩‍👧 is multiple codepoints but one grapheme cluster
    // U+1F468 U+200D U+1F469 U+200D U+1F467
    let emoji = "👨‍👩‍👧";
    let line = format!("before{emoji}after");
    let mut dv = DocumentView::new(vec![line], 10, 10.0);

    // Move cursor to just after the emoji (offset = 6 + emoji.len())
    let emoji_byte_len = emoji.len();
    let target_offset = 6 + emoji_byte_len;
    dv.cursor_move_to_offset(target_offset);
    assert_eq!(dv.cursor().offset, ByteIndex(target_offset));

    // One delete_backward should delete the entire grapheme cluster
    dv.delete_backward(1);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "beforeafter");
    assert_eq!(dv.cursor().offset, ByteIndex(6));
}

#[test]
fn cursor_move_left_crosses_line_boundary() {
    let mut dv = DocumentView::new(vec!["line1".to_string(), "line2".to_string()], 10, 10.0);
    // cursor at offset 6 (start of "line2")
    dv.cursor_move_to_offset(6);
    assert_eq!(dv.cursor().offset, ByteIndex(6));

    // move left should go to end of line1 (offset 5)
    dv.cursor_move_left();
    assert_eq!(dv.cursor().offset, ByteIndex(5));
}

#[test]
fn cursor_move_right_crosses_line_boundary() {
    let mut dv = DocumentView::new(vec!["line1".to_string(), "line2".to_string()], 10, 10.0);
    // cursor at offset 5 (end of "line1", before newline)
    dv.cursor_move_to_offset(5);
    assert_eq!(dv.cursor().offset, ByteIndex(5));

    // move right should go to start of line2 (offset 6)
    dv.cursor_move_right();
    assert_eq!(dv.cursor().offset, ByteIndex(6));
}

#[test]
fn cursor_move_word_left_skips_word() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // cursor at end (offset 11)
    dv.cursor_move_to_offset(11);
    assert_eq!(dv.cursor().offset, ByteIndex(11));

    // move word left should jump to start of "world" (offset 6)
    dv.cursor_move_word_left();
    assert_eq!(dv.cursor().offset, ByteIndex(6));

    // move word left again should jump to start of "hello" (offset 0)
    dv.cursor_move_word_left();
    assert_eq!(dv.cursor().offset, ByteIndex(0));
}

#[test]
fn cursor_move_word_right_skips_word() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // cursor at start (offset 0)
    assert_eq!(dv.cursor().offset, ByteIndex(0));

    // move word right should jump to end of "hello" (offset 5)
    dv.cursor_move_word_right();
    assert_eq!(dv.cursor().offset, ByteIndex(5));

    // move word right again should jump to end of "world" (offset 11)
    dv.cursor_move_word_right();
    assert_eq!(dv.cursor().offset, ByteIndex(11));
}

#[test]
fn undo_redo_after_insert() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert_eq!(dv.cursor().offset, ByteIndex(0));

    // insert "X"
    dv.insert_at_cursor(b"X");
    assert_eq!(dv.cursor().offset, ByteIndex(1));
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "Xhello");

    // undo
    dv.undo();
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "hello");
    assert_eq!(dv.cursor().offset, ByteIndex(0));

    // redo
    dv.redo();
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "Xhello");
}

#[test]
fn cursor_offset_correct_after_cjk_insert() {
    let mut dv = DocumentView::new(vec!["".to_string()], 10, 10.0);
    // Insert CJK character 世 (3 bytes UTF-8)
    dv.insert_at_cursor("世".as_bytes());
    assert_eq!(dv.cursor().offset, ByteIndex(3));

    // Insert another CJK character 界 (3 bytes)
    dv.insert_at_cursor("界".as_bytes());
    assert_eq!(dv.cursor().offset, ByteIndex(6));

    // Content should be "世界"
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "世界");
}

#[test]
fn insert_newline_splits_line() {
    let mut dv = DocumentView::new(vec!["helloworld".to_string()], 10, 10.0);
    // Move cursor to offset 5 (between hello and world)
    dv.cursor_move_to_offset(5);
    assert_eq!(dv.cursor().offset, ByteIndex(5));

    // Insert newline
    dv.insert_at_cursor(
        b"
",
    );
    assert_eq!(dv.line_count(), 2);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "hello");
    assert_eq!(vis[1], "world");
}

#[test]
fn dirty_flag_set_after_edit() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert!(!dv.dirty);

    dv.insert_at_cursor(b"X");
    assert!(dv.dirty);
}

#[test]
fn delete_forward_at_eof_noop() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    // cursor at end
    dv.cursor_move_to_offset(5);
    assert_eq!(dv.cursor().offset, ByteIndex(5));

    // delete forward at EOF should be a no-op
    dv.delete_forward(1);
    assert_eq!(dv.cursor().offset, ByteIndex(5));
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "hello");
}

#[test]
fn multiple_edits_preserve_content() {
    let mut dv = DocumentView::new(vec!["abc".to_string()], 10, 10.0);
    // insert X at start
    dv.insert_at_cursor(b"X");
    // move to end
    dv.cursor_move_to_offset(4);
    // insert Y at end
    dv.insert_at_cursor(b"Y");
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "XabcY");
    assert_eq!(dv.cursor().offset, ByteIndex(5));
}

#[test]
fn visible_line_with_spaces_returns_correct_bytes() {
    // Bug: whitespace rendered as visible glyph (stale atlas data at 0,0)
    let dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("line should exist");
    assert_eq!(&line[..], b"hello world", "visible_line should preserve spaces as bytes");
}

#[test]
fn visible_line_with_multiple_spaces() {
    let dv = DocumentView::new(vec!["a  b   c".to_string()], 10, 10.0);
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("line should exist");
    assert_eq!(line, &b"a  b   c"[..]);
}

#[test]
fn visible_line_with_tab() {
    let dv = DocumentView::new(vec!["hello\tworld".to_string()], 10, 10.0);
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("line should exist");
    // The literal string has backslash-t, not a tab
    assert_eq!(line, &b"hello\tworld"[..]);
}

#[test]
fn line_count_mixed_cjk_ascii() {
    // Bug: CJK characters displayed wrong ("购买" → "够6") due to atlas collision
    // At the DocumentView level, verify content is preserved correctly
    let dv = DocumentView::new(vec!["购买".to_string()], 10, 10.0);
    assert_eq!(dv.line_count(), 1);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "购买");
}

#[test]
fn visible_line_preserves_cjk_content() {
    let dv = DocumentView::new(vec!["购买商品".to_string()], 10, 10.0);
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("line should exist");
    assert_eq!(line, "购买商品".as_bytes());
}

#[test]
fn line_count_digits_not_truncated() {
    // Bug: "62220099" displayed as "622200了了"
    let dv = DocumentView::new(vec!["62220099".to_string()], 10, 10.0);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "62220099", "digits should not be truncated or corrupted");
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("line should exist");
    assert_eq!(line, &b"62220099"[..]);
}

#[test]
fn visible_line_backslash_quote() {
    // Verify JSON-like content with backslash-quote is preserved
    let input = r#"{"key": "value"}"#;
    let dv = DocumentView::new(vec![input.to_string()], 10, 10.0);
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("line should exist");
    assert_eq!(line, input.as_bytes(), "JSON content should be preserved byte-for-byte");
}

#[test]
fn visible_line_empty_string() {
    // Empty vec creates an empty document with one empty line
    let dv = DocumentView::new(vec![], 10, 10.0);
    let line = dv
        .visible_line_with_line_height(0, TEST_LINE_HEIGHT)
        .expect("empty doc has one empty line");
    assert_eq!(&line[..], b"", "empty line should return empty bytes");

    // A document with a blank line (joined with newline creates content)
    let dv2 = DocumentView::new(vec!["".to_string(), "hello".to_string()], 10, 10.0);
    let line =
        dv2.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("blank line should exist");
    assert_eq!(&line[..], b"", "blank line should return empty bytes");
}

#[test]
fn visible_line_mixed_whitespace_and_content() {
    let dv = DocumentView::new(vec!["  hello  world  ".to_string()], 10, 10.0);
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).expect("line should exist");
    assert_eq!(line, &b"  hello  world  "[..]);
}

#[test]
fn visible_range_with_line_height_uses_explicit_value() {
    let mut dv = DocumentView::new(make_lines(20), 4, 4.0);
    dv.presentation.display.display_map.set_entries(
        (0..20)
            .map(|i| {
                let visual_lines = if i == 3 { 3 } else { 1 };
                crate::snap_tree::DisplayLineEntry::placeholder(i * 8, 8, 0, visual_lines)
            })
            .collect(),
    );
    dv.presentation.display.viewport.scroll_anchor = ui::viewport::ScrollAnchor::new(3, 50.0);
    let range = dv.visible_doc_range_with_line_height(36.0);
    assert_eq!(range, 3..7);
}

#[test]
fn visible_line_with_line_height_returns_content() {
    let mut dv = DocumentView::new(make_lines(5), 4, 4.0);
    dv.presentation.display.display_map.set_entries(
        (0..5).map(|i| crate::snap_tree::DisplayLineEntry::placeholder(i * 8, 8, 0, 1)).collect(),
    );
    let line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT);
    assert!(line.is_some());
    let bytes = line.unwrap();
    assert!(!bytes.is_empty());
}

#[test]
fn public_viewport_scroll_helpers_update_anchor_without_display_map() {
    let mut dv = DocumentView::new(make_lines(20), 4, 40.0);

    assert_eq!(dv.viewport_anchor_doc_line(), 0);
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "line 0");

    dv.scroll_doc_lines_for_viewport(3, TEST_LINE_HEIGHT);
    assert_eq!(dv.viewport_anchor_doc_line(), 3);
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "line 3");

    dv.scroll_to_doc_line_for_viewport(0, TEST_LINE_HEIGHT);
    assert_eq!(dv.viewport_anchor_doc_line(), 0);
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "line 0");
}

#[test]
fn gbk_file_marks_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gbk.txt");
    // "你好\n" in GB18030 encoding (GBK superset)
    std::fs::write(&path, [0xC4, 0xE3, 0xBA, 0xC3, 0x0A]).unwrap();

    let dv = DocumentView::from_file(&path, 10, 100.0).unwrap();
    assert!(dv.dirty, "GBK transcoded file should be marked dirty");
    assert!(matches!(dv.original_encoding, Some("GB18030") | Some("GBK")));
}

#[test]
fn document_view_owns_rebuildable_presentation_aggregate() {
    let mut view = DocumentView::new(vec!["hello".into(), "world".into()], 8, 96.0);
    view.presentation.search_state.panel_visible = true;
    view.presentation.search_state.query = "hello".to_owned();
    let original_cursor = view.cursor.offset;
    let original_line_count = view.line_index.line_count();

    view.presentation = crate::document_presentation::DocumentPresentation::new(16, 192.0);

    assert_eq!(view.cursor.offset, original_cursor);
    assert_eq!(view.line_index.line_count(), original_line_count);
    assert!(!view.presentation.search_state.panel_visible);
    assert!(view.presentation.search_state.query.is_empty());
}

#[test]
fn presentation_can_be_taken_and_restored_without_touching_model() {
    let mut view = DocumentView::new(vec!["hello".into(), "world".into()], 8, 96.0);
    view.presentation.search_state.query = "needle".to_owned();
    let original_cursor = view.cursor.offset;
    let original_line_count = view.line_index.line_count();

    let mut detached = view.take_presentation();
    detached.search_state.panel_visible = true;
    detached.search_state.query.push('!');

    assert!(view.presentation.search_state.query.is_empty());
    assert_eq!(view.cursor.offset, original_cursor);
    assert_eq!(view.line_index.line_count(), original_line_count);

    view.restore_presentation(detached);

    assert!(view.presentation.search_state.panel_visible);
    assert_eq!(view.presentation.search_state.query, "needle!");
    assert_eq!(view.cursor.offset, original_cursor);
    assert_eq!(view.line_index.line_count(), original_line_count);
}

#[test]
fn document_view_roundtrips_through_explicit_model_and_presentation_parts() {
    let mut view = DocumentView::new(vec!["hello".into(), "world".into()], 8, 96.0);
    view.cursor_move_to_offset(3);
    view.presentation.search_state.query = "needle".to_owned();

    let (model, presentation) = view.into_parts();

    assert_eq!(model.cursor.offset, ByteIndex(3));
    assert_eq!(presentation.search_state.query, "needle");

    let restored = DocumentView::from_parts(model, presentation);
    assert_eq!(restored.full_text(), "hello\nworld");
    assert_eq!(restored.cursor.offset, ByteIndex(3));
    assert_eq!(restored.presentation.search_state.query, "needle");
}
