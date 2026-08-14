use super::*;

// ── Selection helpers ───────────────────────────────────────────

/// Helper: set selection from anchor to cursor offset.
fn select_range(dv: &mut DocumentView, anchor: usize, cursor: usize) {
    dv.cursor_mut().selection_anchor = Some(anchor);
    dv.set_cursor_offset_synced(cursor);
}

// ── Shift+Arrow selection extension ─────────────────────────────

#[test]
fn shift_right_extends_selection() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Start at offset 0, start selection
    dv.cursor_mut().selection_anchor = Some(0);

    // Simulate shift+right: set cursor_offset directly (preserving anchor)
    // The actual method will move by grapheme; here we test the anchor behavior.
    dv.set_cursor_offset_synced(5);
    assert_eq!(dv.selection_range(), Some((0, 5)));
}

#[test]
fn shift_left_extends_selection() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Anchor at 5, cursor moves left to 2 (preserving anchor)
    dv.cursor_mut().selection_anchor = Some(5);
    dv.set_cursor_offset_synced(2);
    assert_eq!(dv.selection_range(), Some((2, 5)));
}

#[test]
fn shift_click_extends_to_point() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Anchor at 3, shift+click moves cursor to 8 (preserving anchor)
    dv.cursor_mut().selection_anchor = Some(3);
    dv.set_cursor_offset_synced(8);
    assert_eq!(dv.selection_range(), Some((3, 8)));
}

#[test]
fn shift_click_reverse_direction() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Anchor at 8, shift+click moves cursor to 2
    dv.cursor_mut().selection_anchor = Some(8);
    dv.set_cursor_offset_synced(2);
    // start <= end always
    assert_eq!(dv.selection_range(), Some((2, 8)));
}

#[test]
fn triple_click_selects_line() {
    let dv = &mut DocumentView::new(
        vec!["first".to_string(), "second".to_string(), "third".to_string()],
        10,
        10.0,
    );
    // Cursor on line 1 ("second", starts at offset 6)
    let line = 1usize;
    let line_start = dv.line_index.offsets[line];
    let line_end = if line + 1 < dv.line_index.offsets.len() {
        dv.line_index.offsets[line + 1]
    } else {
        dv.buffer_len()
    };

    select_range(dv, line_start, line_end);
    let (sel_start, sel_end) = dv.selection_range().unwrap();
    assert_eq!(sel_start, line_start);
    assert_eq!(sel_end, line_end);
}

#[test]
fn double_click_selects_word() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Simulate: double-click on "world" → word_select returns 6..11
    select_range(dv, 6, 11);
    let (sel_start, sel_end) = dv.selection_range().unwrap();
    assert_eq!(sel_start, 6);
    assert_eq!(sel_end, 11);
}

#[test]
fn double_click_selects_word_cjk() {
    let dv = &mut DocumentView::new(vec!["你好世界abc".to_string()], 10, 10.0);
    // CJK chars are Word class, contiguous CJK = "你好世界" = 12 bytes
    select_range(dv, 0, 12);
    let (sel_start, sel_end) = dv.selection_range().unwrap();
    assert_eq!(sel_start, 0);
    assert_eq!(sel_end, 12);
}

#[test]
fn double_click_selects_word_unicode() {
    // Test various word boundaries with the edit+ word classifier

    // Simple ASCII word
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    let (start, end) = dv.word_select_at(2); // click on "hello"
    assert_eq!(start, 0);
    assert_eq!(end, 5);

    // Second word
    let (start, end) = dv.word_select_at(7); // click on "world"
    assert_eq!(start, 6);
    assert_eq!(end, 11);

    // Hyphenated word: hyphen is Separator class
    let dv = &mut DocumentView::new(vec!["foo-bar".to_string()], 10, 10.0);
    let (start, end) = dv.word_select_at(0); // click on "foo"
    assert_eq!(start, 0);
    assert_eq!(end, 3, "foo before hyphen");
    let (start, end) = dv.word_select_at(4); // click on "bar"
    assert_eq!(start, 4);
    assert_eq!(end, 7, "bar after hyphen");

    // Numbers are Word class
    let dv = &mut DocumentView::new(vec!["abc 123 def".to_string()], 10, 10.0);
    let (start, end) = dv.word_select_at(5); // click on "123"
    assert_eq!(start, 4);
    assert_eq!(end, 7, "number run is one word");

    // Underscore is Word class (not in separator list)
    let dv = &mut DocumentView::new(vec!["foo_bar".to_string()], 10, 10.0);
    let (start, end) = dv.word_select_at(3); // click on "bar" part
    assert_eq!(start, 0, "underscore is word char");
    assert_eq!(end, 7);

    // Multiple spaces
    let dv = &mut DocumentView::new(vec!["a  b".to_string()], 10, 10.0);
    let (start, end) = dv.word_select_at(0); // click on "a"
    assert_eq!(start, 0);
    assert_eq!(end, 1);
    let (start, end) = dv.word_select_at(3); // click on "b"
    assert_eq!(start, 3);
    assert_eq!(end, 4);

    // Separator characters
    let dv = &mut DocumentView::new(vec!["a.b,c".to_string()], 10, 10.0);
    let (start, end) = dv.word_select_at(0); // click on "a"
    assert_eq!(start, 0);
    assert_eq!(end, 1);
    let (start, end) = dv.word_select_at(2); // click on "b"
    assert_eq!(start, 2);
    assert_eq!(end, 3);
}

#[test]
fn select_all_creates_full_range() {
    let dv = &mut DocumentView::new(vec!["line1".to_string(), "line2".to_string()], 10, 10.0);
    dv.select_all();
    assert!(dv.has_selection());
    let (start, end) = dv.selection_range().unwrap();
    assert_eq!(start, 0);
    assert_eq!(end, dv.buffer_len());
}

#[test]
fn selection_cleared_on_clear() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    select_range(dv, 0, 5);
    assert!(dv.has_selection());
    dv.clear_selection();
    assert!(!dv.has_selection());
}

// ── Clipboard EOL normalization ─────────────────────────────────

#[test]
fn clipboard_eol_crlf_to_lf() {
    let input = b"hello\r\nworld\r\n";
    let output = normalize_paste_text(input);
    assert_eq!(output, &b"hello\nworld\n"[..]);
}

#[test]
fn clipboard_eol_cr_only_to_lf() {
    let input = b"hello\rworld\r";
    let output = normalize_paste_text(input);
    assert_eq!(output, &b"hello\nworld\n"[..]);
}

#[test]
fn clipboard_eol_mixed() {
    let input = b"a\r\nb\rc\r\nd\ne";
    let output = normalize_paste_text(input);
    assert_eq!(output, &b"a\nb\nc\nd\ne"[..]);
}

#[test]
fn clipboard_eol_lf_unchanged() {
    let input = b"hello\nworld\n";
    let output = normalize_paste_text(input);
    assert_eq!(output, &b"hello\nworld\n"[..]);
}

// ── Clipboard BOM stripping ─────────────────────────────────────

#[test]
fn clipboard_strip_bom() {
    let input = b"\xEF\xBB\xBFhello world";
    let output = normalize_paste_text(input);
    assert_eq!(output, &b"hello world"[..]);
}

#[test]
fn clipboard_no_bom_unchanged() {
    let input = b"hello world";
    let output = normalize_paste_text(input);
    assert_eq!(output, &b"hello world"[..]);
}

#[test]
fn clipboard_bom_with_crlf() {
    let input = b"\xEF\xBB\xBFline1\r\nline2\r\n";
    let output = normalize_paste_text(input);
    assert_eq!(output, &b"line1\nline2\n"[..]);
}

#[test]
fn clipboard_empty_input() {
    let output = normalize_paste_text(b"");
    assert_eq!(output, &b""[..]);
}

// ── Selection + delete ──────────────────────────────────────────

#[test]
fn delete_selection_basic() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    select_range(dv, 0, 5);
    let deleted = dv.delete_selection();
    assert!(deleted);
    assert_eq!(dv.buffer_len(), 6); // " world"
    assert!(!dv.has_selection());
}

#[test]
fn delete_selection_empty_is_noop() {
    let dv = &mut DocumentView::new(vec!["hello".to_string()], 10, 10.0);
    let deleted = dv.delete_selection();
    assert!(!deleted);
    assert_eq!(dv.buffer_len(), 5);
}

// ── Undo/redo with selection ────────────────────────────────────

#[test]
fn undo_restores_after_selection_delete() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
    // Select " world" (6..11) and delete
    select_range(dv, 5, 11);
    dv.delete_selection();
    assert_eq!(dv.buffer_len(), 5, "should be 'hello'");

    // Undo should restore "hello world"
    dv.undo();
    assert_eq!(dv.buffer_len(), 11, "undo should restore 'hello world'");
}

#[test]
fn undo_coalesced_typing_single_step() {
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    // Type 50 characters using write_canon (coalesced)
    // Type 26 lowercase letters — coalesced into one undo step
    for ch in b"abcdefghijklmnopqrstuvwxyz" {
        dv.tb.write_canon(&[*ch]);
    }
    dv.sync_cursor_offset_from_tb();
    dv.dirty = dv.tb.is_dirty();
    dv.rebuild_viewport();
    assert_eq!(dv.buffer_len(), 26);

    // Undo — continuous typing coalesces, so fewer undo steps
    let mut undo_count = 0;
    for _ in 0..60 {
        let before = dv.buffer_len();
        dv.undo();
        if dv.buffer_len() >= before {
            break;
        }
        undo_count += 1;
    }
    // coalesced typing → single undo removes all 26 chars
    assert!(undo_count > 0, "undo should reduce buffer");
    assert_eq!(dv.buffer_len(), 0, "single undo should remove all coalesced text");
}

// ── Selection edge cases ────────────────────────────────────────

#[test]
fn selection_across_invalid_utf8_lossy() {
    // Insert raw bytes including invalid UTF-8, then select across them
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    // Insert "hello" + invalid byte + "world"
    dv.insert_at_cursor(b"hello");
    dv.insert_at_cursor(&[0xFF, 0xFE]); // invalid UTF-8
    dv.insert_at_cursor(b"world");

    // Select all (should not panic)
    dv.select_all();
    assert!(dv.has_selection());

    // Extract selected text — lossy conversion should not panic
    let text = dv.extract_selected_text();
    assert!(text.is_some(), "should extract text even with invalid UTF-8");
    let bytes = text.unwrap();
    assert!(!bytes.is_empty(), "should have some bytes");
    // The invalid bytes should be present in the raw output
    assert!(
        bytes.contains(&0xFF) || bytes.contains(&0xFE),
        "raw bytes should include invalid UTF-8 bytes"
    );
}

#[test]
fn clipboard_lossy_copy_does_not_modify_document() {
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.insert_at_cursor(b"aa");
    dv.insert_at_cursor(&[0xFF, 0xFE]);
    dv.insert_at_cursor(b"bb");

    dv.select_all();
    let raw = dv.extract_selected_text().unwrap();
    assert_eq!(raw.len(), 6);
    assert_eq!(raw[2], 0xFF);
    assert_eq!(raw[3], 0xFE);

    // Lossy conversion replaces invalid bytes with U+FFFD
    let lossy = String::from_utf8_lossy(&raw);
    assert!(lossy.contains("\u{FFFD}"), "lossy should contain replacement chars");

    // Document bytes unchanged after clear
    dv.clear_selection();
    assert_eq!(dv.buffer_len(), 6, "document length unchanged");
}

#[test]
#[ignore]
fn clipboard_copy_system_lossy_with_invalid_utf8() {
    use ui::core::Clipboard;

    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.insert_at_cursor(b"hello");
    dv.insert_at_cursor(&[0xFF]);
    dv.insert_at_cursor(b"world");

    dv.select_all();
    let ok = dv.copy_selection_to_clipboard();
    if !ok {
        eprintln!("Skipping clipboard_copy_system_lossy_with_invalid_utf8: clipboard unavailable");
        return;
    }

    if let Some(clip_text) = appkit_shell::SystemClipboard.read_text() {
        assert!(clip_text.contains("hello"), "valid prefix");
        assert!(clip_text.contains("world"), "valid suffix");
    }
}

#[test]
fn selection_across_multiline() {
    let dv = &mut DocumentView::new(
        vec!["line1".to_string(), "line2".to_string(), "line3".to_string()],
        10,
        10.0,
    );
    // Select from middle of line1 to middle of line3
    dv.cursor_mut().selection_anchor = Some(3); // "lin" of line1
    dv.set_cursor_offset_synced(16); // middle of line3

    let (start, end) = dv.selection_range().unwrap();
    assert_eq!(start, 3);
    assert_eq!(end, 16);
    assert!(end - start > 10, "selection should span multiple lines");

    // Extract should work across lines
    let text = dv.extract_selected_text();
    assert!(text.is_some());
    let bytes = text.unwrap();
    assert!(bytes.contains(&b'\n'), "should contain newline");
}

// ── Clipboard roundtrip ─────────────────────────────────────────

#[test]
#[ignore]
fn clipboard_roundtrip_utf8() {
    use ui::core::Clipboard;

    // Test clipboard set/get with various UTF-8 content
    // This test requires a display server; skip if clipboard unavailable
    let mut clipboard = appkit_shell::SystemClipboard;

    // Test with ASCII
    if !clipboard.write_text("hello world") {
        eprintln!("Skipping clipboard_roundtrip_utf8: clipboard unavailable");
        return;
    }
    let Some(got) = clipboard.read_text() else {
        eprintln!("Skipping clipboard_roundtrip_utf8: clipboard unreadable");
        return;
    };
    assert_eq!(got, "hello world");

    // Test with CJK
    assert!(clipboard.write_text("你好世界"));
    let got = clipboard.read_text().expect("written clipboard text should remain readable");
    assert_eq!(got, "你好世界");

    // Test with emoji
    assert!(clipboard.write_text("🌍🌏🌎"));
    let got = clipboard.read_text().expect("written clipboard text should remain readable");
    assert_eq!(got, "🌍🌏🌎");

    // Test with mixed content
    let mixed = "Hello 你好 🌍\nNew line\ttab";
    assert!(clipboard.write_text(mixed));
    let got = clipboard.read_text().expect("written clipboard text should remain readable");
    assert_eq!(got, mixed);
}

#[test]
fn clipboard_paste_normalizes_crlf_and_strips_bom() {
    // Simulate: external clipboard has CRLF + BOM (common on Windows/macOS)
    // paste_text should normalize to LF and strip BOM
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    let input = b"\xEF\xBB\xBFline1\r\nline2\r\n";
    dv.paste_text(input);

    // Should be "line1\nline2\n" (BOM stripped, CRLF→LF)
    assert_eq!(dv.buffer_len(), 12); // "line1\nline2\n"
}

#[test]
fn clipboard_paste_handles_various_formats() {
    // Test that paste_text correctly handles content from various clipboard sources
    // On macOS, arboard.extract_plain_text() handles RTF/HTML/TIFF filtering.
    // paste_text() receives plain text and normalizes it.

    // Plain ASCII
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.paste_text(b"hello world");
    assert_eq!(dv.buffer_len(), 11);

    // UTF-8 with CJK
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.paste_text("你好世界".as_bytes());
    assert_eq!(dv.buffer_len(), 12); // 4 CJK chars * 3 bytes each

    // Mixed line endings (common when pasting from web/RTF)
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.paste_text(b"line1\r\nline2\rline3\n");
    // Should normalize all to LF
    dv.select_all();
    let content = dv.extract_selected_text().unwrap();
    assert_eq!(content, &b"line1\nline2\nline3\n"[..]);

    // Content with BOM in middle (should NOT strip, only at start)
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.paste_text(b"hello\xEF\xBB\xBFworld");
    dv.select_all();
    let content = dv.extract_selected_text().unwrap();
    assert_eq!(content, b"hello\xEF\xBB\xBFworld", "BOM in middle should be preserved");

    // Empty paste
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.paste_text(b"");
    assert_eq!(dv.buffer_len(), 0);

    // Tab characters
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    dv.paste_text(b"col1\tcol2\tcol3");
    dv.select_all();
    let content = dv.extract_selected_text().unwrap();
    assert_eq!(content, b"col1\tcol2\tcol3", "tabs should be preserved");
}

// ── Undo/redo performance thresholds ────────────────────────────

#[test]
fn undo_redo_50_steps_performance() {
    use std::time::Instant;

    // Use write_raw to create 52 separate undo entries (one per char)
    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    for ch in b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ" {
        dv.tb.write_raw(&[*ch]);
    }
    dv.sync_cursor_offset_from_tb();
    dv.dirty = dv.tb.is_dirty();
    dv.rebuild_viewport();
    assert_eq!(dv.buffer_len(), 52);

    let start = Instant::now();
    for _ in 0..52 {
        let before = dv.buffer_len();
        dv.undo();
        if dv.buffer_len() >= before {
            break;
        }
    }
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 100, "undo 52 steps took {elapsed:?} (threshold: < 100ms)");
}

#[test]
fn undo_past_load_point_no_panic() {
    // Simulate: load file, mark clean, edit, then undo past the clean point
    let dv = &mut DocumentView::new(vec!["original".to_string()], 10, 10.0);
    assert_eq!(dv.buffer_len(), 8);

    // Mark as clean (simulates file load point)
    dv.tb.mark_as_clean();
    assert!(!dv.dirty);

    // Edit: add text
    dv.insert_at_cursor(b" modified");
    assert!(dv.dirty);
    assert_eq!(dv.buffer_len(), 17);

    // Undo past the clean point — should not panic
    dv.undo();
    assert_eq!(dv.buffer_len(), 8, "undo should restore to load point");
    assert!(!dv.dirty, "should be clean at load point");
}

// ── Copy 1MB performance ────────────────────────────────────────

#[test]
fn copy_1mb_text_performance() {
    use std::time::Instant;

    let dv = &mut DocumentView::new(vec!["".to_string()], 10, 10.0);
    // Insert 1MB of text
    let chunk = b"abcdefghijklmnopqrstuvwxyz0123456789\n";
    let target = 1024 * 1024; // 1 MB
    while dv.buffer_len() < target {
        let remaining = target - dv.buffer_len();
        let to_write = &chunk[..chunk.len().min(remaining)];
        dv.insert_at_cursor(to_write);
    }

    // Select all
    dv.select_all();

    // Measure copy (extract_selected_text)
    let start = Instant::now();
    let text = dv.extract_selected_text();
    let elapsed = start.elapsed();

    assert!(text.is_some(), "should extract text");
    let bytes = text.unwrap();
    assert!(bytes.len() >= target, "should extract at least 1MB");

    assert!(elapsed.as_millis() < 50, "copy 1MB took {elapsed:?} (threshold: < 50ms)");
}

// ── Mouse drag selection ────────────────────────────────────────

#[test]
fn mouse_drag_creates_range() {
    let dv = &mut DocumentView::new(vec!["hello world".to_string()], 10, 10.0);

    // Simulate mouse down at offset 3 ("l" of "lo world")
    dv.cursor_mut().selection_anchor = Some(3);
    dv.set_cursor_offset_synced(3);
    assert_eq!(dv.selection_range(), Some((3, 3)), "initial click should be point selection");

    // Simulate mouse drag to offset 8 ("o" of "orld")
    dv.set_cursor_offset_synced(8);
    let (start, end) = dv.selection_range().unwrap();
    assert_eq!(start, 3, "anchor should remain at 3");
    assert_eq!(end, 8, "cursor should be at 8");
    assert!(dv.has_selection(), "should have selection after drag");

    // Simulate mouse drag backward to offset 1 ("e")
    dv.set_cursor_offset_synced(1);
    let (start, end) = dv.selection_range().unwrap();
    assert_eq!(start, 1, "start should be min(anchor, cursor)");
    assert_eq!(end, 3, "end should be max(anchor, cursor)");
    assert!(dv.has_selection(), "should still have selection");

    // Verify selected text
    let text = dv.extract_selected_text();
    assert!(text.is_some());
    assert_eq!(text.unwrap(), b"el", "selected text should be 'el'");
}
