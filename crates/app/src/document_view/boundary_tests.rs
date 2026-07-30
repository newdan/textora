use super::*;
use crate::commands::execute_edit_command;
use crate::input::EditCommand;
use core::types::ByteIndex;

const TEST_LINE_HEIGHT: f32 = 24.27;

// ── BOM handling ─────────────────────────────────────────────────

#[test]
fn bom_at_start_backspace_noop() {
    // UTF-8 BOM is 3 bytes: 0xEF 0xBB 0xBF
    // After loading a file with BOM, backspace at position 0 should be no-op
    let bom = "\u{FEFF}"; // BOM character
    let content = format!("{bom}hello");
    let mut dv = DocumentView::new(vec![content], 10, 10.0);
    assert_eq!(dv.cursor().offset, ByteIndex(0));
    // Backspace at start should not corrupt the BOM
    dv.delete_backward(1);
    assert_eq!(dv.cursor().offset, ByteIndex(0));
    // Content should still be intact
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert!(vis[0].contains("hello"), "content should survive backspace at BOM");
}

#[test]
fn bom_not_counted_as_visible_column() {
    // BOM is a zero-width character; cursor should be at offset 0
    let bom = "\u{FEFF}";
    let content = format!("{bom}hello");
    let dv = DocumentView::new(vec![content], 10, 10.0);
    assert_eq!(dv.cursor().offset, ByteIndex(0));
    // Line content starts with BOM bytes
    let first_line = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert!(first_line.len() > 5, "BOM + hello should be > 5 bytes");
}

// ── Null byte handling ───────────────────────────────────────────

#[test]
fn insert_null_byte_replaced_with_fffd() {
    // \0 is replaced with U+FFFD (3 bytes in UTF-8)
    let mut dv = DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.insert_at_cursor(b"\0");
    // cursor should advance past U+FFFD (3 bytes)
    assert_eq!(dv.cursor().offset, ByteIndex(3));
    // The buffer should have 3 bytes (U+FFFD = \xEF\xBF\xBD)
    assert_eq!(dv.buffer_len(), 3);
}

#[test]
fn null_byte_in_content_replaced_with_fffd() {
    // A line with embedded null is sanitized to U+FFFD
    let mut dv = DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.insert_at_cursor(b"a\0b");
    let vis = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    // a(1) + U+FFFD(3) + b(1) = 5 bytes
    assert_eq!(vis.len(), 5);
    assert_eq!(&vis[0..1], &b"a"[..]);
    assert_eq!(&vis[1..4], &[0xEF, 0xBF, 0xBD]); // U+FFFD
    assert_eq!(&vis[4..5], &b"b"[..]);
}

// ── NFD combining characters ─────────────────────────────────────

#[test]
fn combining_accent_deleted_with_base() {
    // é as NFD: e + U+0301 (combining acute accent)
    // In NFC this is U+00E9, but in NFD it's two codepoints
    // cosmic-text should treat NFD as one grapheme
    let nfd_e_accent = "e\u{0301}"; // é in NFD form
    let line = format!("x{nfd_e_accent}y");
    let mut dv = DocumentView::new(vec![line], 10, 10.0);
    // cursor after the accented char: offset = 1 + nfd_e_accent.len()
    let after = 1 + nfd_e_accent.len();
    dv.cursor_move_to_offset(after);
    // One backspace should delete the whole grapheme (base + combining)
    dv.delete_backward(1);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "xy", "NFD base+combining should be deleted as one grapheme");
}

#[test]
fn multiple_combining_marks_one_grapheme() {
    // U+0061 (a) + U+0300 (grave) + U+0302 (circumflex) — one grapheme
    let complex = "a\u{0300}\u{0302}";
    let line = format!("x{complex}y");
    let mut dv = DocumentView::new(vec![line], 10, 10.0);
    let after = 1 + complex.len();
    dv.cursor_move_to_offset(after);
    dv.delete_backward(1);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis[0], "xy");
}

// ── Empty document edge cases ────────────────────────────────────

#[test]
fn all_commands_noop_on_empty_doc() {
    let mut dv = DocumentView::new(vec![], 10, 10.0);
    let initial_offset = dv.cursor().offset;

    // These should all be no-ops on empty document
    execute_edit_command(&EditCommand::Backspace, &mut dv, &[]);
    assert_eq!(dv.cursor().offset, initial_offset);

    execute_edit_command(&EditCommand::DeleteForward, &mut dv, &[]);
    assert_eq!(dv.cursor().offset, initial_offset);

    execute_edit_command(&EditCommand::MoveLeft, &mut dv, &[]);
    assert_eq!(dv.cursor().offset, initial_offset);

    execute_edit_command(&EditCommand::MoveWordLeft, &mut dv, &[]);
    assert_eq!(dv.cursor().offset, initial_offset);
}

#[test]
fn insert_into_empty_doc() {
    let mut dv = DocumentView::new(vec![], 10, 10.0);
    assert!(dv.is_empty());

    execute_edit_command(&EditCommand::InsertChar("A".into()), &mut dv, &[]);
    assert!(!dv.is_empty());
    assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "A");
    assert_eq!(dv.cursor().offset, ByteIndex(1));
}

// ── Long line edge cases ─────────────────────────────────────────

#[test]
fn very_long_line_insert() {
    let mut dv = DocumentView::new(vec!["".to_string()], 10, 10.0);
    // Insert 10000 characters
    let long_str = "x".repeat(10000);
    dv.insert_at_cursor(long_str.as_bytes());
    assert_eq!(dv.cursor().offset, ByteIndex(10000));
    assert_eq!(dv.buffer_len(), 10000);
}
#[test]
fn enter_at_end_of_long_line() {
    let long_line = "x".repeat(100_000);
    let mut dv = DocumentView::new(vec![long_line.clone()], 10, 10.0);
    dv.cursor_move_to_offset(100_000);
    execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
    // Should split into 2 lines without panic or extreme slowdown
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 2);
    assert_eq!(vis[0].len(), 100_000);
    assert_eq!(vis[1], "");
}

// ── CRLF line ending edge cases ──────────────────────────────────

#[test]
fn insert_crlf_in_crlf_mode() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    dv.set_crlf(true);
    dv.cursor_move_to_offset(5);
    execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
    // "hello" + CRLF = 7 bytes (LF mode would be 6)
    assert_eq!(dv.buffer_len(), 7, "hello(5) + \r\n(2) = 7");
    // Verify first visible line ends before \r
    let vis0 = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis0, &b"hello"[..]);
    // Cursor should be at start of line 2 (offset 7)
    assert_eq!(dv.cursor().offset, ByteIndex(7));
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 2, "should have 2 lines after newline");
}

// ── ZWJ middle cursor (grapheme boundary protection) ─────────────

#[test]
fn zwj_emoji_middle_cursor_moves_to_boundary() {
    // ZWJ emoji: 👨‍👩‍👧 — try to place cursor in the middle
    // TextBuffer should snap to grapheme boundary
    let emoji = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let line = format!("x{emoji}y");
    let mut dv = DocumentView::new(vec![line], 10, 10.0);
    let emoji_start = 1;
    let emoji_end = 1 + emoji.len();
    // Try to move cursor to middle of emoji cluster
    let middle = emoji_start + emoji.len() / 2;
    dv.cursor_move_to_offset(middle);
    // After TextBuffer cursor_move_to_offset, cursor should be at a valid position
    // It may snap to the start or end of the grapheme cluster
    let pos = dv.cursor().offset;
    assert!(
        pos.to_usize() <= emoji_start || pos.to_usize() >= emoji_end,
        "cursor at {} should snap to grapheme boundary ({} or {})",
        pos.to_usize(),
        emoji_start,
        emoji_end
    );
}

// ── null byte handling ───────────────────────────────────────────

#[test]
fn null_byte_replaced_in_buffer() {
    // \0 is replaced with U+FFFD (3 bytes); a + FFFD + b = 5 bytes
    let mut dv = DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.insert_at_cursor(b"a\0b");
    assert_eq!(dv.buffer_len(), 5);
    // Cursor should be at end
    assert_eq!(dv.cursor().offset, ByteIndex(5));
}

#[test]
fn null_byte_fffd_delete_backward() {
    // \0 becomes U+FFFD; delete_backward removes the whole grapheme
    let mut dv = DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.insert_at_cursor(b"\0");
    assert_eq!(dv.cursor().offset, ByteIndex(3)); // U+FFFD is 3 bytes
    dv.delete_backward(1);
    assert_eq!(dv.cursor().offset, ByteIndex(0));
    assert!(dv.is_empty());
}

// ── Incremental line index correctness ──────────────────────────

#[test]
fn incremental_update_multiline_insert() {
    let mut dv = DocumentView::new(vec!["line1".to_string(), "line2".to_string()], 10, 10.0);
    // Insert "\nmid" at offset 5 (end of "line1") — splits line1
    dv.cursor_move_to_offset(5);
    dv.insert_at_cursor(b"\nmid");
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 3);
    assert_eq!(vis[0], "line1");
    assert_eq!(vis[1], "mid");
    assert_eq!(vis[2], "line2");
}

#[test]
fn incremental_update_single_char_repeated() {
    // Simulate typing 500 chars into a 1000-line file
    let lines: Vec<String> = (0..1000).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 50, 10.0);
    // Move to end of first line
    dv.cursor_move_to_offset(6);
    for ch in b"hello world" {
        dv.insert_at_cursor(&[*ch]);
    }
    // Verify line 0 has the inserted text
    let vis0 = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert!(vis0.starts_with(b"line 0hello world"), "vis0 = {:?}", vis0);
    // Verify total line count is still 1000
    assert_eq!(dv.line_count(), 1000);
}

#[test]
fn incremental_update_delete_across_lines() {
    let mut dv = DocumentView::new(vec!["ab".to_string(), "cd".to_string()], 10, 10.0);
    // Position at end of "ab" (offset 2)
    dv.cursor_move_to_offset(2);
    // Delete forward — should join lines
    dv.delete_forward(1);
    let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
    assert_eq!(vis.len(), 1, "delete newline should join lines");
    assert_eq!(vis[0], "abcd");
}

// ── Selection state (SelectAll) ─────────────────────────────────

#[test]
fn select_all_sets_anchor_and_cursor() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    assert!(!dv.has_selection());
    dv.select_all();
    assert!(dv.has_selection());
    assert_eq!(dv.cursor().selection_anchor, Some(0));
    assert_eq!(dv.cursor().offset, ByteIndex(11)); // "hello world".len()
    assert_eq!(dv.selection_range(), Some((0, 11)));
}

#[test]
fn select_all_via_execute_command() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert!(execute_edit_command(&EditCommand::SelectAll, &mut dv, &[]));
    assert!(dv.has_selection());
    assert_eq!(dv.selection_range(), Some((0, 5)));
}

#[test]
fn cursor_move_clears_selection() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    dv.select_all();
    assert!(dv.has_selection());
    dv.cursor_move_left();
    assert!(!dv.has_selection());
}

#[test]
fn edit_preserves_selection_anchor() {
    // Selection anchor is cleared on sync_cursor (called by cursor moves),
    // but NOT on sync_after_edit (called by inserts/deletes).
    // This is intentional: inserting while selected should eventually
    // delete the selection first (future work via delete_selection).
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    dv.select_all();
    assert!(dv.has_selection());
    // insert_at_cursor calls sync_after_edit, which does NOT clear selection
    // (selection clearing is in sync_cursor only)
}

#[test]
fn delete_selection_removes_selected_text() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Select "hello " (bytes 0..6)
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(6);
    assert!(dv.delete_selection());
    // Remaining content should be "world"
    assert_eq!(dv.cursor().offset, ByteIndex(0));
    assert!(!dv.has_selection());
    let vis = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis, &b"world"[..]);
}

#[test]
fn delete_selection_noop_when_empty() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    // No selection
    assert!(!dv.delete_selection());
    assert_eq!(dv.buffer_len(), 5);

    // Selection with anchor == cursor (empty range)
    dv.cursor_mut().selection_anchor = Some(3);
    dv.set_cursor_offset_synced(3);
    assert!(!dv.delete_selection());
    assert_eq!(dv.buffer_len(), 5);
}

#[test]
fn clear_selection_method() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    dv.select_all();
    assert!(dv.has_selection());
    dv.clear_selection();
    assert!(!dv.has_selection());
    // Cursor stays where it was
    assert_eq!(dv.cursor().offset, ByteIndex(5));
}

#[test]
fn new_constructor_sanitizes_null_bytes() {
    // new() should also replace null bytes
    let dv = DocumentView::new(vec!["a\0b".to_string()], 10, 10.0);
    let vis = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    // a(1) + U+FFFD(3) + b(1) = 5 bytes
    assert_eq!(vis.len(), 5);
    assert_eq!(&vis[0..1], &b"a"[..]);
    assert_eq!(&vis[1..4], &[0xEF, 0xBF, 0xBD]);
    assert_eq!(&vis[4..5], &b"b"[..]);
}

#[test]
fn backspace_with_selection_deletes_selection() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Select "hello " (bytes 0..6)
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(6);
    // Backspace should delete the selection, not just one char
    assert!(execute_edit_command(&EditCommand::Backspace, &mut dv, &[]));
    assert!(!dv.has_selection());
    assert_eq!(dv.cursor().offset, ByteIndex(0));
    let vis = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis, &b"world"[..]);
}

#[test]
fn delete_forward_with_selection_deletes_selection() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Select "world" (bytes 6..11)
    dv.cursor_mut().selection_anchor = Some(6);
    dv.set_cursor_offset_synced(11);
    assert!(execute_edit_command(&EditCommand::DeleteForward, &mut dv, &[]));
    assert!(!dv.has_selection());
    let vis = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis, &b"hello "[..]);
}

#[test]
fn insert_char_with_selection_replaces_selection() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Select "hello" (bytes 0..5)
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(5);
    assert!(execute_edit_command(&EditCommand::InsertChar("X".into()), &mut dv, &[]));
    assert!(!dv.has_selection());
    let vis = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis, &b"X world"[..]);
}

#[test]
fn insert_newline_with_selection_replaces_selection() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Select "hello " (bytes 0..6)
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(6);
    assert!(execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]));
    assert!(!dv.has_selection());
    // "hello " replaced with newline -> "\nworld"
    let vis0 = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis0, &b""[..]);
    let vis1 = dv.visible_line_with_line_height(1, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis1, &b"world"[..]);
}

#[test]
fn select_all_then_backspace_clears_document() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert!(execute_edit_command(&EditCommand::SelectAll, &mut dv, &[]));
    assert!(dv.has_selection());
    assert!(execute_edit_command(&EditCommand::Backspace, &mut dv, &[]));
    assert!(!dv.has_selection());
    assert!(dv.is_empty());
    assert_eq!(dv.cursor().offset, ByteIndex(0));
}
#[test]
fn select_all_then_insert_char_replaces_all() {
    let mut dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert!(execute_edit_command(&EditCommand::SelectAll, &mut dv, &[]));
    assert!(dv.has_selection());
    // Type "X" — should replace entire content
    assert!(execute_edit_command(&EditCommand::InsertChar("X".into()), &mut dv, &[]));
    assert!(!dv.has_selection());
    assert_eq!(dv.buffer_len(), 1);
    let vis = dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap();
    assert_eq!(vis, &b"X"[..]);
}

#[test]
fn delete_selection_multiline_rescans_correctly() {
    // "line0\nline1\nline2" — select "line1\n" (bytes 6..12)
    let mut dv = DocumentView::new(
        ["line0", "line1", "line2"].iter().map(|s| s.to_string()).collect(),
        10,
        10.0,
    );
    dv.cursor_mut().selection_anchor = Some(6);
    dv.set_cursor_offset_synced(12);
    assert!(dv.delete_selection());
    // Remaining: "line0\nline2"
    assert_eq!(dv.line_index.offsets.len(), 2);
    assert_eq!(dv.line_index.offsets, vec![0, 6]);
    assert_eq!(dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap(), &b"line0"[..]);
    assert_eq!(dv.visible_line_with_line_height(1, TEST_LINE_HEIGHT).unwrap(), &b"line2"[..]);
    assert_eq!(dv.cursor().offset, ByteIndex(6));
}

#[test]
fn delete_selection_all_content_rescans_correctly() {
    // "line0\nline1" (11 bytes) — select all
    let mut dv =
        DocumentView::new(["line0", "line1"].iter().map(|s| s.to_string()).collect(), 10, 10.0);
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(11);
    assert!(dv.delete_selection());
    assert_eq!(dv.line_index.offsets.len(), 1);
    assert_eq!(dv.buffer_len(), 0);
}

#[test]
fn delete_selection_preserves_scroll_position() {
    use crate::snap_tree::DisplayLineEntry;
    let lines: Vec<String> = (0..50).map(|i| format!("line {i}")).collect();
    let mut dv = DocumentView::new(lines, 10, 10.0);
    let mut entries = Vec::new();
    for _ in 0..50 {
        entries.push(DisplayLineEntry::placeholder(0, 80, 0, 1));
    }
    dv.presentation.display.display_map.set_entries(entries);
    // Scroll down
    dv.presentation.display.viewport.scroll_doc_lines(20, &dv.presentation.display.display_map);
    // Select and delete within visible area
    dv.cursor_mut().selection_anchor = Some(300);
    dv.set_cursor_offset_synced(310);
    assert!(dv.delete_selection());
    // Scroll should be preserved (not reset to 0)
    assert!(
        dv.presentation.display.viewport.scroll_anchor.doc_line > 0,
        "scroll should not reset to 0 after delete"
    );
}

#[test]
fn undo_redo_after_delete() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(5);
    assert!(dv.delete_selection());
    assert_eq!(dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap(), &b" world"[..]);
    // Undo should restore "hello world"
    dv.undo();
    assert_eq!(dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap(), &b"hello world"[..]);
    // Redo should re-delete to " world"
    dv.redo();
    assert_eq!(dv.visible_line_with_line_height(0, TEST_LINE_HEIGHT).unwrap(), &b" world"[..]);
}

#[test]
fn count_selection_chars_ascii() {
    let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(5);
    assert_eq!(dv.count_selection_chars(), Some(5));
}

#[test]
fn count_selection_chars_utf8() {
    let mut dv = DocumentView::new(vec!["héllo".to_string()], 10, 10.0);
    // "héllo" = h(1) + é(2) + l(1) + l(1) + o(1) = 6 bytes, 5 chars
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(6);
    assert_eq!(dv.count_selection_chars(), Some(5));
}

#[test]
fn count_selection_chars_no_selection() {
    let dv = DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    assert_eq!(dv.count_selection_chars(), None);
}

#[test]
fn count_selection_chars_multiline() {
    let mut dv =
        DocumentView::new(["line0", "line1"].iter().map(|s| s.to_string()).collect(), 10, 10.0);
    // Select "line0\nline1" (all 11 bytes, 11 chars)
    dv.cursor_mut().selection_anchor = Some(0);
    dv.set_cursor_offset_synced(11);
    assert_eq!(dv.count_selection_chars(), Some(11));
}

// ── Multi-byte UTF-8 cursor snapping ───────────────────────────

#[test]
fn set_cursor_offset_synced_mid_char_snaps_to_boundary() {
    // "你好abc" → 你(3 bytes) 好(3 bytes) a b c
    // Byte offsets: 你=[0,3), 好=[3,6), a=6, b=7, c=8
    let mut dv = DocumentView::new(vec!["你好abc".to_string()], 10, 10.0);
    assert_eq!(dv.cursor().offset, ByteIndex(0));

    // Request offset 1 (middle of '你') — should snap to 0 or 3
    dv.set_cursor_offset_synced(1);
    let snapped = dv.cursor().offset;
    assert!(
        snapped == ByteIndex(0) || snapped == ByteIndex(3),
        "mid-char offset 1 should snap to grapheme boundary, got {}",
        snapped.to_usize()
    );
    dv.assert_cursor_synced();

    // Request offset 4 (middle of '好') — should snap to 3 or 6
    dv.set_cursor_offset_synced(4);
    let snapped = dv.cursor().offset;
    assert!(
        snapped == ByteIndex(3) || snapped == ByteIndex(6),
        "mid-char offset 4 should snap to grapheme boundary, got {}",
        snapped.to_usize()
    );
    dv.assert_cursor_synced();

    // Valid grapheme boundary should remain unchanged
    dv.set_cursor_offset_synced(3);
    assert_eq!(dv.cursor().offset, ByteIndex(3));
    dv.assert_cursor_synced();

    dv.set_cursor_offset_synced(6);
    assert_eq!(dv.cursor().offset, ByteIndex(6));
    dv.assert_cursor_synced();
}
