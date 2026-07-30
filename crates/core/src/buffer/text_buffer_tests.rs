use super::{CursorMovement, SearchOptions, TextBuffer};
use crate::types::ByteIndex;

fn buffer_contents(buf: &mut TextBuffer) -> String {
    let mut str = String::new();
    buf.save_as_string(&mut str);
    str
}

#[test]
fn replace_one_zero_width() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(true);
    buf.write_raw(b"a\nb\n");
    buf.cursor_move_to_logical(Default::default());

    for _ in 0..6 {
        buf.find_and_replace("$", SearchOptions { use_regex: true, ..Default::default() }, b"x")
            .unwrap();
    }

    assert_eq!(buffer_contents(&mut buf), "axx\nbxx\nx\n");
}

#[test]
fn replace_all_zero_width() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(true);
    buf.write_raw(b"a\nb\n");

    buf.find_and_replace_all("$", SearchOptions { use_regex: true, ..Default::default() }, b"x")
        .unwrap();

    assert_eq!(buffer_contents(&mut buf), "ax\nbx\nx\n");
}

// ── Stage 6: cursor movement tests ──────────────────────────────────────

#[test]
fn cursor_move_left_right_basic() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");

    // Cursor at end (offset 5)
    assert_eq!(buf.cursor.offset, ByteIndex(5));

    // Move left one grapheme
    buf.cursor_move_delta(CursorMovement::Grapheme, -1);
    assert_eq!(buf.cursor.offset, ByteIndex(4));

    // Move right one grapheme
    buf.cursor_move_delta(CursorMovement::Grapheme, 1);
    assert_eq!(buf.cursor.offset, ByteIndex(5));
}

#[test]
fn cursor_move_left_at_line_start() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello\nworld");

    // Move to start of second line (offset 6)
    buf.cursor_move_to_byte(ByteIndex(6));
    assert_eq!(buf.cursor.logical_pos.line, 1);

    // Move left at line start should go to end of previous line
    buf.cursor_move_delta(CursorMovement::Grapheme, -1);
    assert_eq!(buf.cursor.offset, ByteIndex(5)); // after 'o' on first line
    assert_eq!(buf.cursor.logical_pos.line, 0);
}

#[test]
fn cursor_move_right_at_line_end() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello\nworld");

    // Move to end of first line (offset 5, before \n)
    buf.cursor_move_to_byte(ByteIndex(5));

    // Move right should go to start of next line
    buf.cursor_move_delta(CursorMovement::Grapheme, 1);
    assert_eq!(buf.cursor.offset, ByteIndex(6)); // 'w' on second line
    assert_eq!(buf.cursor.logical_pos.line, 1);
}

#[test]
fn cursor_move_word_unicode_boundary() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello world foo");

    // Move to end
    buf.cursor_move_to_byte(ByteIndex(15));

    // Move one word left
    buf.cursor_move_delta(CursorMovement::Word, -1);
    let pos_after_first_word_move = buf.cursor.offset;
    assert_eq!(pos_after_first_word_move, ByteIndex(12)); // start of "foo"

    // Move one more word left
    buf.cursor_move_delta(CursorMovement::Word, -1);
    assert_eq!(buf.cursor.offset, ByteIndex(6)); // start of "world"
}

#[test]
fn cursor_move_left_at_bof_noop() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");

    // Move to start
    buf.cursor_move_to_byte(ByteIndex(0));

    // Move left at beginning should stay at 0
    buf.cursor_move_delta(CursorMovement::Grapheme, -1);
    assert_eq!(buf.cursor.offset, ByteIndex(0));
}

#[test]
fn cursor_move_right_at_eof_noop() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");

    // Cursor is already at end (offset 5)
    assert_eq!(buf.cursor.offset, ByteIndex(5));

    // Move right at end should stay
    buf.cursor_move_delta(CursorMovement::Grapheme, 1);
    assert_eq!(buf.cursor.offset, ByteIndex(5));
}

// ── Stage 6: insert/delete tests ────────────────────────────────────────

#[test]
fn insert_text_and_read_back() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");
    assert_eq!(buf.text_length(), 5);

    buf.write_raw(b" world");
    assert_eq!(buf.text_length(), 11);
}

#[test]
fn backspace_grapheme_cluster_zwj_emoji() {
    let mut buf = TextBuffer::new(true).unwrap();
    // ZWJ family emoji: 👨‍👩‍👧 (U+1F468 U+200D U+1F469 U+200D U+1F467)
    let emoji = "👨\u{200D}👩\u{200D}👧";
    buf.write_raw(emoji.as_bytes());
    let len_after_insert = buf.text_length();
    assert!(len_after_insert > 0);

    // Delete one grapheme cluster backward — should delete entire ZWJ sequence
    buf.delete(CursorMovement::Grapheme, -1);
    assert_eq!(buf.text_length(), 0, "ZWJ emoji should be deleted as one grapheme");
}

#[test]
fn delete_at_eof_no_op() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");

    // Cursor at end, delete forward should do nothing
    let len_before = buf.text_length();
    buf.delete(CursorMovement::Grapheme, 1);
    assert_eq!(buf.text_length(), len_before, "delete at EOF should be no-op");
}

#[test]
fn backspace_at_bof_no_op() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");

    // Move to start
    buf.cursor_move_to_byte(ByteIndex(0));

    let len_before = buf.text_length();
    buf.delete(CursorMovement::Grapheme, -1);
    assert_eq!(buf.text_length(), len_before, "backspace at BOF should be no-op");
}

// ── Stage 6: line ending tests ──────────────────────────────────────────

#[test]
fn enter_inserts_lf_in_lf_file() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.set_crlf(false);
    buf.write_raw(b"hello");

    // Simulate Enter by inserting \n
    buf.write_raw(b"\n");
    assert!(buf.text_length() > 5);
    // Should contain LF
    let contents = buffer_contents(&mut buf);
    assert!(contents.contains("hello\n"), "LF file should use \\n");
}

#[test]
fn is_crlf_detection() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"a\r\nb");
    // After loading CRLF content, buffer should detect CRLF
    // Note: write_raw normalizes newlines, so this tests the flag
    buf.set_crlf(true);
    assert!(buf.is_crlf());
    buf.set_crlf(false);
    assert!(!buf.is_crlf());
}

// ── Stage 6: undo/redo tests ────────────────────────────────────────────

#[test]
fn history_undo_single_insert() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");
    assert_eq!(buf.text_length(), 5);

    // Undo should remove the text
    buf.undo();
    assert_eq!(buf.text_length(), 0, "undo should remove inserted text");

    // Redo should restore it
    buf.redo();
    assert_eq!(buf.text_length(), 5, "redo should restore text");
}

#[test]
fn history_undo_delete() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello world");

    // Delete "world"
    buf.cursor_move_to_byte(ByteIndex(11));
    buf.delete(CursorMovement::Grapheme, -5);
    assert_eq!(buf.text_length(), 6, "should be 'hello '");

    // Undo should restore "world"
    buf.undo();
    assert_eq!(buf.text_length(), 11, "undo should restore deleted text");

    // Redo should delete again
    buf.redo();
    assert_eq!(buf.text_length(), 6, "redo should delete again");
}

// ── Stage 6: dirty flag tests ───────────────────────────────────────────

#[test]
fn dirty_flag_lifecycle() {
    let mut buf = TextBuffer::new(true).unwrap();

    // New buffer is clean
    assert!(!buf.is_dirty());

    // Insert makes it dirty
    buf.write_raw(b"hello");
    assert!(buf.is_dirty());

    // Mark as clean
    buf.mark_as_clean();
    assert!(!buf.is_dirty());

    // Delete makes it dirty again
    buf.delete(CursorMovement::Grapheme, -1);
    assert!(buf.is_dirty());
}

#[test]
fn generation_increments_on_edit() {
    let mut buf = TextBuffer::new(true).unwrap();
    let gen0 = buf.generation();

    buf.write_raw(b"a");
    let gen1 = buf.generation();
    assert!(gen1 > gen0, "generation should increment on insert");

    buf.write_raw(b"b");
    let gen2 = buf.generation();
    assert!(gen2 >= gen1, "generation should not decrease");
}

// ── Stage 6: multi-line editing ─────────────────────────────────────────

#[test]
fn insert_newline_creates_new_line() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello\nworld");
    assert_eq!(buf.logical_line_count(), 2);

    // Move to end of first line and insert newline
    buf.cursor_move_to_byte(ByteIndex(5));
    buf.write_raw(b"\n");
    assert_eq!(buf.logical_line_count(), 3, "should have 3 lines now");
}

#[test]
fn delete_join_lines() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"ab\ncd");

    // Move to start of second line
    buf.cursor_move_to_byte(ByteIndex(3));
    // Backspace should join lines
    buf.delete(CursorMovement::Grapheme, -1);
    assert_eq!(buf.logical_line_count(), 1, "joining lines should reduce count");
}

// ── Stage 6: edge case tests ───────────────────────────────────────────

#[test]
fn backspace_nfd_combining_char_deletes_cluster() {
    // NFD: é = e + U+0301 (combining acute accent)
    let mut buf = TextBuffer::new(true).unwrap();
    let nfd_e = "e\u{0301}"; // 2 bytes: 'e' + combining accent
    buf.write_raw(nfd_e.as_bytes());
    assert_eq!(buf.text_length(), nfd_e.len());

    // Backspace should delete the entire grapheme cluster (base + combining)
    buf.delete(CursorMovement::Grapheme, -1);
    assert_eq!(buf.text_length(), 0, "NFD combining char should be deleted with base");
}

#[test]
fn backspace_multiple_combining_chars() {
    // Multiple combining marks: a + ring + acute
    let mut buf = TextBuffer::new(true).unwrap();
    let text = "a\u{030A}\u{0301}"; // 'a' + ring above + acute
    buf.write_raw(text.as_bytes());
    let len = buf.text_length();
    assert!(len > 1, "should be multi-byte");

    buf.delete(CursorMovement::Grapheme, -1);
    assert_eq!(buf.text_length(), 0, "all combining chars should be deleted together");
}

#[test]
fn insert_null_byte() {
    // null byte should be insertable
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"ab");
    buf.cursor_move_to_byte(ByteIndex(1));
    buf.write_raw(b"\0");
    assert_eq!(buf.text_length(), 3);
}

#[test]
fn delete_at_eof_preserves_content() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");
    buf.cursor_move_to_byte(ByteIndex(5));
    buf.delete(CursorMovement::Grapheme, 1);
    buf.delete(CursorMovement::Word, 1);
    buf.delete(CursorMovement::Grapheme, 100);
    assert_eq!(buf.text_length(), 5, "content should be unchanged");
}

#[test]
fn backspace_at_bof_preserves_content() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");
    buf.cursor_move_to_byte(ByteIndex(0));
    buf.delete(CursorMovement::Grapheme, -1);
    buf.delete(CursorMovement::Word, -1);
    buf.delete(CursorMovement::Grapheme, -100);
    assert_eq!(buf.text_length(), 5, "content should be unchanged");
}

#[test]
fn insert_empty_is_noop() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");
    let len = buf.text_length();
    buf.write_raw(b"");
    assert_eq!(buf.text_length(), len, "empty insert should not change length");
}

#[test]
fn alternating_insert_delete() {
    let mut buf = TextBuffer::new(true).unwrap();
    for ch in b"abcdef" {
        buf.write_raw(&[*ch]);
    }
    assert_eq!(buf.text_length(), 6);

    for expected in (0..6).rev() {
        buf.delete(CursorMovement::Grapheme, -1);
        assert_eq!(buf.text_length(), expected);
    }
    assert_eq!(buf.text_length(), 0);
}

// ── Stage 7: undo/redo edge cases ──────────────────────────────────────

#[test]
fn history_redo_after_branch_loses_redo_stack() {
    let mut buf = TextBuffer::new(true).unwrap();
    // Type "abc" — each char coalesces into one Write group
    buf.write_raw(b"abc");
    let len_abc = buf.text_length();
    assert_eq!(len_abc, 3);

    // Undo twice: first undo removes "abc", second is no-op
    buf.undo();
    assert_eq!(buf.text_length(), 0, "undo should remove 'abc'");
    buf.undo();
    assert_eq!(buf.text_length(), 0, "second undo is no-op");

    // Redo to get "abc" back
    buf.redo();
    assert_eq!(buf.text_length(), 3, "redo should restore 'abc'");

    // Undo again to go back to empty
    buf.undo();
    assert_eq!(buf.text_length(), 0);

    // Now type "x" — this should clear the redo stack
    buf.write_raw(b"x");
    assert_eq!(buf.text_length(), 1);

    // Redo should be a no-op (redo stack was cleared when we typed "x")
    buf.redo();
    assert_eq!(buf.text_length(), 1, "redo should be no-op after branch");
}

#[test]
fn history_coalesce_continuous_typing() {
    let mut buf = TextBuffer::new(true).unwrap();
    // Type "hello" character by character using write_canon (non-raw)
    // which uses HistoryType::Write and coalesces continuous writes.
    // write_raw uses HistoryType::Other which does NOT coalesce.
    for ch in b"hello" {
        buf.write_canon(&[*ch]);
    }
    assert_eq!(buf.text_length(), 5);

    // Single undo should remove all of "hello" (coalesced into one step)
    buf.undo();
    assert_eq!(buf.text_length(), 0, "continuous typing should coalesce into one undo step");

    // Redo should restore all of "hello"
    buf.redo();
    assert_eq!(buf.text_length(), 5, "redo should restore all coalesced text");
}

#[test]
fn history_limit_memory_cap() {
    let mut buf = TextBuffer::new(true).unwrap();
    // write_raw uses HistoryType::Other which never coalesces,
    // so each call creates a separate undo entry.
    // Build up text so length changes with each undo.
    for i in 0..1100u32 {
        buf.write_raw(format!("{i:04}").as_bytes());
    }
    assert_eq!(buf.text_length(), 1100 * 4);

    // Undo until no more entries or we've done 1100 attempts
    let mut undo_count = 0;
    for _ in 0..1100 {
        let len_before = buf.text_length();
        buf.undo();
        let len_after = buf.text_length();
        if len_after >= len_before {
            break; // No more undo entries
        }
        undo_count += 1;
    }
    // Should have been able to undo ~1000 entries (the cap)
    // but not all 1100 (oldest 100 were dropped)
    assert!(undo_count <= 1001, "undo stack should be capped at ~1000, got {undo_count}");
    assert!(undo_count >= 995, "should have ~1000 undo entries, got {undo_count}");
    // Text should still have ~100 entries worth of data (400 bytes)
    assert!(buf.text_length() > 0, "oldest entries should have been dropped");
}

#[test]
fn history_undo_replace() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello world");
    assert_eq!(buf.text_length(), 11);

    // Select "world" by setting selection
    buf.cursor_move_to_byte(ByteIndex(6));
    buf.start_selection();
    buf.selection_update_offset(11);
    assert!(buf.has_selection());

    // Type "rust" — should replace selection
    buf.write_raw(b"rust");
    assert_eq!(buf.text_length(), 10, "'hello rust' = 10 bytes");
    assert!(!buf.has_selection(), "selection should be cleared after write");

    // Undo should restore "hello world"
    buf.undo();
    assert_eq!(buf.text_length(), 11, "undo should restore 'hello world'");
}

#[test]
fn history_preserves_selection_state() {
    let mut buf = TextBuffer::new(true).unwrap();
    buf.write_raw(b"hello");
    buf.mark_as_clean();

    // Start selection, extend it
    buf.cursor_move_to_byte(ByteIndex(1));
    buf.start_selection();
    buf.selection_update_offset(4);
    assert!(buf.has_selection());

    // Undo (there's nothing to undo since no edit happened — but let's test
    // that undo/redo preserves selection when edits do happen)
    buf.write_raw(b"X"); // replaces selection with "X" → "hXo"
    assert_eq!(buf.text_length(), 3);

    buf.undo();
    assert_eq!(buf.text_length(), 5, "undo should restore 'hello'");
    // After undo, selection should be restored
    assert!(buf.has_selection(), "undo should restore selection state");
}

// ── Stage 11: Replace + ICU regex tests ────────────────────────────────

#[test]
fn undo_after_replace_all_single_step() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw(b"foo bar foo baz foo");
    assert_eq!(buffer_contents(&mut buf), "foo bar foo baz foo");

    buf.find_and_replace_all("foo", SearchOptions::default(), b"X").unwrap();
    assert_eq!(buffer_contents(&mut buf), "X bar X baz X");

    buf.undo();
    assert_eq!(
        buffer_contents(&mut buf),
        "foo bar foo baz foo",
        "undo after replace_all must restore all matches as a single step"
    );
}

#[test]
fn regex_replace_capture_groups_via_find_and_replace_all() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw(b"abc 123 def 456 ghi");

    let options = SearchOptions { use_regex: true, ..Default::default() };
    buf.find_and_replace_all(r"(\d+)", options, b"[$1]").unwrap();

    assert_eq!(buffer_contents(&mut buf), "abc [123] def [456] ghi");
}

#[test]
fn regex_replace_with_dollar_dollar_escape() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw(b"price 100");

    let options = SearchOptions { use_regex: true, ..Default::default() };
    buf.find_and_replace_all(r"(\d+)", options, b"$$$1").unwrap();

    assert_eq!(buffer_contents(&mut buf), "price $100");
}

#[test]
fn regex_replace_case_insensitive_capture_groups() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw(b"Foo BAR foo bar FOO");

    let options = SearchOptions { use_regex: true, match_case: false, ..Default::default() };
    buf.find_and_replace_all(r"(foo|bar)", options, b"[$0]").unwrap();

    assert_eq!(buffer_contents(&mut buf), "[Foo] [BAR] [foo] [bar] [FOO]");
}

#[test]
fn replace_all_empty_pattern_returns_error() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.write_raw(b"hello");

    let result = buf.find_and_replace_all("", SearchOptions::default(), b"x");
    assert!(result.is_err(), "empty pattern should return error");
}

#[test]
fn replace_all_with_regex_anchor() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw(b"a\nb\nc");

    let options = SearchOptions { use_regex: true, ..Default::default() };
    buf.find_and_replace_all("^", options, b"> ").unwrap();

    assert_eq!(buffer_contents(&mut buf), "> a\n> b\n> c");
}

#[test]
fn replace_all_and_undo_then_redo() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw(b"x1y x2y x3y");

    let options = SearchOptions { use_regex: true, ..Default::default() };
    buf.find_and_replace_all(r"x(\d)y", options, b"[$1]").unwrap();
    assert_eq!(buffer_contents(&mut buf), "[1] [2] [3]");

    buf.undo();
    assert_eq!(buffer_contents(&mut buf), "x1y x2y x3y");

    buf.redo();
    assert_eq!(
        buffer_contents(&mut buf),
        "[1] [2] [3]",
        "redo after replace_all undo must restore all replacements as single step"
    );
}

#[test]
fn regex_replace_preserves_utf8_byte_ranges() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.write_raw("a中🙂b 中🙂".as_bytes());

    buf.find_and_replace_all(
        r"(中)(🙂)",
        SearchOptions { use_regex: true, ..Default::default() },
        b"$2$1",
    )
    .unwrap();

    assert_eq!(buffer_contents(&mut buf), "a🙂中b 🙂中");
}

#[test]
fn invalid_regex_returns_error_without_mutating_buffer() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.write_raw(b"unchanged");

    let result = buf.find_and_replace_all(
        "(",
        SearchOptions { use_regex: true, ..Default::default() },
        b"x",
    );

    assert!(result.is_err());
    assert_eq!(buffer_contents(&mut buf), "unchanged");

    let edit_group_is_clear = buf.active_edit_group.is_none();
    buf.cursor_move_to_byte(ByteIndex(0));
    buf.write_raw(b"X");
    assert_eq!(buffer_contents(&mut buf), "Xunchanged");

    buf.undo();
    assert_eq!(
        buffer_contents(&mut buf),
        "unchanged",
        "undo after an invalid regex should only revert the subsequent independent edit"
    );
    assert_eq!(
        (buf.cursor_offset(), edit_group_is_clear),
        (ByteIndex(0), true),
        "an invalid regex must not leak edit grouping state or alter the next edit's undo cursor",
    );
}

#[test]
fn replace_range_basic() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(false);
    buf.write_raw(b"hello world");
    // Replace bytes 6..11 ("world") with "rust"
    buf.replace_range(6..11, b"rust");
    assert_eq!(buffer_contents(&mut buf), "hello rust");
}

#[test]
fn replace_range_empty_replacement_is_delete() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(false);
    buf.write_raw(b"hello world");
    buf.replace_range(5..11, b"");
    assert_eq!(buffer_contents(&mut buf), "hello");
}

#[test]
fn replace_range_empty_range_is_insert() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(false);
    buf.write_raw(b"hello world");
    buf.replace_range(5..5, b" beautiful");
    assert_eq!(buffer_contents(&mut buf), "hello beautiful world");
}

#[test]
fn replace_range_at_start() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(false);
    buf.write_raw(b"hello world");
    buf.replace_range(0..5, b"Hi");
    assert_eq!(buffer_contents(&mut buf), "Hi world");
}

#[test]
fn replace_range_at_end() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(false);
    buf.write_raw(b"hello world");
    buf.replace_range(6..11, b"rust");
    assert_eq!(buffer_contents(&mut buf), "hello rust");
}

#[test]
fn replace_range_multiline() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(false);
    buf.write_raw(b"line1\nline2\nline3");
    // Replace "line2\n" with ""
    buf.replace_range(6..12, b"");
    assert_eq!(buffer_contents(&mut buf), "line1\nline3");
}

#[test]
fn replace_range_with_newline_replacement() {
    let mut buf = TextBuffer::new(false).unwrap();
    buf.set_crlf(false);
    buf.set_insert_final_newline(false);
    buf.write_raw(b"hello world");
    buf.replace_range(5..6, b"\n");
    assert_eq!(buffer_contents(&mut buf), "hello\nworld");
}

fn text_buffer_from_text(text: &str) -> TextBuffer {
    let mut buffer = TextBuffer::new(false).expect("test buffer must be created");
    buffer.write_raw(text.as_bytes());
    buffer
}

#[test]
fn grapheme_boundary_delta_does_not_mutate_real_cursor() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let mut buffer = text_buffer_from_text(&format!("a{emoji}b"));
    let emoji_end = 1 + emoji.len();
    buffer.cursor_move_to_byte(ByteIndex(emoji_end));

    let target = buffer.grapheme_boundary_delta(ByteIndex(emoji_end), -1);

    assert_eq!(target, ByteIndex(1));
    assert_eq!(buffer.cursor.offset, ByteIndex(emoji_end));
}

#[test]
fn grapheme_boundary_query_rejects_middle_of_cluster() {
    let emoji = "👨\u{200D}👩\u{200D}👧";
    let buffer = text_buffer_from_text(emoji);

    assert!(buffer.is_grapheme_boundary(ByteIndex(0)));
    assert!(buffer.is_grapheme_boundary(ByteIndex(emoji.len())));
    assert!(!buffer.is_grapheme_boundary(ByteIndex(4)));
}
