//! Product tests for the shared source-editor commands.

use crate::document_fixture::DocumentFixture;
use appkit_core::edit_command::EditCommand;
use ui::core::Clipboard;
use ui::render_geom::AdvanceCacheEntry;

pub(crate) use appkit_shell::edit_transaction::EditOutcome;
use appkit_shell::editor_commands::execute_edit_command_with_clipboard as execute_edit_command_v2_with_presentation_and_clipboard;
pub(crate) use appkit_shell::editor_commands::execute_edit_command_with_system_clipboard as execute_edit_command_v2_with_presentation;

pub(crate) fn execute_edit_command_v2(
    cmd: &EditCommand,
    dv: &mut DocumentFixture,
    advance_cache: &[AdvanceCacheEntry],
) -> EditOutcome {
    let presentation = &mut dv.presentation;
    let page_step_rows = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
    execute_edit_command_v2_with_presentation(
        cmd,
        &mut dv.model,
        advance_cache,
        &mut presentation.cursor_render_state,
        page_step_rows,
    )
}

pub(crate) fn execute_edit_command(
    command: &EditCommand,
    document: &mut DocumentFixture,
    advance_cache: &[AdvanceCacheEntry],
) -> bool {
    execute_edit_command_v2(command, document, advance_cache).executed
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::line_index::LineIndex;
    use textora_core::types::ByteIndex;

    struct TestClipboard {
        text: Option<String>,
        writes_succeed: bool,
    }

    impl Clipboard for TestClipboard {
        fn read_text(&mut self) -> Option<String> {
            self.text.clone()
        }

        fn write_text(&mut self, text: &str) -> bool {
            if !self.writes_succeed {
                return false;
            }
            self.text = Some(text.to_owned());
            true
        }
    }

    fn execute_with_clipboard(
        command: &EditCommand,
        document: &mut DocumentFixture,
        clipboard: &mut dyn Clipboard,
    ) -> EditOutcome {
        let presentation = &mut document.presentation;
        let page_step_rows = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
        execute_edit_command_v2_with_presentation_and_clipboard(
            command,
            &mut document.model,
            &[],
            &mut presentation.cursor_render_state,
            page_step_rows,
            clipboard,
        )
    }

    fn make_dv(content: &str) -> DocumentFixture {
        let lines: Vec<String> = if content.is_empty() {
            vec![]
        } else {
            content.split('\n').map(String::from).collect()
        };
        DocumentFixture::new(lines, 20, 10.0)
    }

    // ── Cursor movement ──────────────────────────────────────────────

    #[test]
    fn move_left_basic() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(3);
        assert!(execute_edit_command(&EditCommand::MoveLeft, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(2));
    }

    #[test]
    fn move_right_basic() {
        let mut dv = make_dv("hello");
        assert!(execute_edit_command(&EditCommand::MoveRight, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(1));
    }

    #[test]
    fn move_to_doc_start() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::MoveToDocStart, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(0));
    }

    #[test]
    fn move_to_doc_end() {
        let mut dv = make_dv("hello");
        assert!(execute_edit_command(&EditCommand::MoveToDocEnd, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(5));
    }

    #[test]
    fn move_word_left() {
        let mut dv = make_dv("hello world");
        dv.cursor_move_to_offset(11);
        assert!(execute_edit_command(&EditCommand::MoveWordLeft, &mut dv, &[]));
        // word_backward stops at word boundary (after space)
        assert_eq!(dv.cursor().offset, ByteIndex(6));
    }

    #[test]
    fn move_word_right() {
        let mut dv = make_dv("hello world");
        assert!(execute_edit_command(&EditCommand::MoveWordRight, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(5));
    }

    // ── Editing ──────────────────────────────────────────────────────

    #[test]
    fn backspace_deletes_grapheme() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(1);
        assert!(execute_edit_command(&EditCommand::Backspace, &mut dv, &[]));
        assert_eq!(dv.document_lines()[0], "ello");
        assert_eq!(dv.cursor().offset, ByteIndex(0));
    }

    #[test]
    fn delete_forward_deletes_grapheme() {
        let mut dv = make_dv("hello");
        assert!(execute_edit_command(&EditCommand::DeleteForward, &mut dv, &[]));
        assert_eq!(dv.document_lines()[0], "ello");
    }

    #[test]
    fn insert_char() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::InsertChar("X".into()), &mut dv, &[]));
        assert_eq!(dv.document_lines()[0], "helloX");
    }

    #[test]
    fn insert_newline() {
        let mut dv = make_dv("helloworld");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]));
        let vis = dv.document_lines();
        assert_eq!(vis[0], "hello");
        assert_eq!(vis[1], "world");
    }

    // ── Undo/Redo ────────────────────────────────────────────────────

    #[test]
    fn undo_redo() {
        let mut dv = make_dv("hello");
        execute_edit_command(&EditCommand::InsertChar("X".into()), &mut dv, &[]);
        assert_eq!(dv.document_lines()[0], "Xhello");

        execute_edit_command(&EditCommand::Undo, &mut dv, &[]);
        assert_eq!(dv.document_lines()[0], "hello");

        execute_edit_command(&EditCommand::Redo, &mut dv, &[]);
        assert_eq!(dv.document_lines()[0], "Xhello");
    }

    // ── PageUp/PageDown ──────────────────────────────────────────────

    /// Helper: create 100-line doc with visible_rows=10.
    /// Each line "line N" (N=0..99).  Lines 0-9 are 6 bytes each → 7 bytes/line with \n.
    fn make_100_line_dv() -> DocumentFixture {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        DocumentFixture::new(lines, 10, 10.0)
    }

    #[test]
    fn page_down_moves_by_visible_rows_minus_one() {
        let mut dv = make_100_line_dv();
        // visible_rows=10 → page_size=9
        assert!(execute_edit_command(&EditCommand::PageDown, &mut dv, &[]));
        // cursor should be at line 9, column 0 → offset 63
        assert_eq!(dv.cursor_line(), 9, "PageDown should move by visible_rows-1 lines");
        assert_eq!(dv.cursor().offset, ByteIndex(63));
    }

    #[test]
    fn page_up_moves_by_visible_rows_minus_one() {
        let mut dv = make_100_line_dv();
        // Move to line 18 first (two page-downs)
        execute_edit_command(&EditCommand::PageDown, &mut dv, &[]);
        execute_edit_command(&EditCommand::PageDown, &mut dv, &[]);
        assert_eq!(dv.cursor_line(), 18);
        // PageUp should move back by 9 lines → line 9
        execute_edit_command(&EditCommand::PageUp, &mut dv, &[]);
        assert_eq!(dv.cursor_line(), 9, "PageUp should move by visible_rows-1 lines");
    }

    #[test]
    fn page_down_then_up_returns_to_start() {
        let mut dv = make_100_line_dv();
        let start_offset = dv.cursor().offset;
        execute_edit_command(&EditCommand::PageDown, &mut dv, &[]);
        execute_edit_command(&EditCommand::PageUp, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, start_offset, "PageDown then PageUp should return to start");
    }

    #[test]
    fn page_down_at_bottom_clamps() {
        // 12-line doc, visible_rows=10 → page_size=9
        let lines: Vec<String> = (0..12).map(|i| format!("line {i}")).collect();
        let mut dv = DocumentFixture::new(lines, 10, 10.0);
        // First PageDown: line 0 → line 9
        execute_edit_command(&EditCommand::PageDown, &mut dv, &[]);
        assert_eq!(dv.cursor_line(), 9);
        // Second PageDown: would go to line 18, but only 12 lines → clamped to line 11
        execute_edit_command(&EditCommand::PageDown, &mut dv, &[]);
        assert_eq!(dv.cursor_line(), 11, "PageDown at bottom should clamp to last line");
    }

    #[test]
    fn page_up_at_top_stays() {
        let mut dv = make_100_line_dv();
        // Already at top — PageUp should not crash, cursor stays at 0
        assert!(execute_edit_command(&EditCommand::PageUp, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(0), "PageUp at top should stay at offset 0");
    }

    #[test]
    fn page_down_preserves_column() {
        let mut dv = make_100_line_dv();
        // Move to column 3 of line 0
        dv.cursor_move_to_offset(3);
        assert_eq!(dv.cursor_column(), 3);
        execute_edit_command(&EditCommand::PageDown, &mut dv, &[]);
        // Should still be at column 3 on line 9
        assert_eq!(dv.cursor_line(), 9);
        assert_eq!(dv.cursor_column(), 3, "PageDown should preserve column position");
    }

    #[test]
    fn move_up_basic() {
        let mut dv = DocumentFixture::new(vec!["line0".to_string(), "line1".to_string()], 10, 10.0);
        // cursor at start of line1 (offset 6)
        dv.cursor_move_to_offset(6);
        assert!(execute_edit_command(&EditCommand::MoveUp, &mut dv, &[]));
        // should be on line0 now
        assert!(dv.cursor().offset < ByteIndex(6));
    }

    #[test]
    fn move_down_basic() {
        let mut dv = DocumentFixture::new(vec!["line0".to_string(), "line1".to_string()], 10, 10.0);
        assert_eq!(dv.cursor().offset, ByteIndex(0));
        assert!(execute_edit_command(&EditCommand::MoveDown, &mut dv, &[]));
        // should be on line1 now
        assert!(dv.cursor().offset >= ByteIndex(6));
    }

    // ── Home (MoveToLineStart) — indent-aware ────────────────────────

    #[test]
    fn home_first_press_goes_to_indent() {
        let mut dv = make_dv("    hello world");
        dv.cursor_move_to_offset(10); // somewhere in "world"
        assert!(execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]));
        // First press → indent start (column 4, offset 4)
        assert_eq!(dv.cursor().offset, ByteIndex(4), "First Home should go to indent start");
    }

    #[test]
    fn home_second_press_goes_to_line_start() {
        let mut dv = make_dv("    hello world");
        dv.cursor_move_to_offset(10);
        // First press → indent
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(4));
        // Second press → line start (column 0)
        assert!(execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(0), "Second Home should go to column 0");
    }

    #[test]
    fn home_resets_on_other_command() {
        let mut dv = make_dv("    hello world");
        dv.cursor_move_to_offset(10);
        // First Home → indent
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(4));
        // Move right resets the toggle
        execute_edit_command(&EditCommand::MoveRight, &mut dv, &[]);
        // Next Home should go to indent again, not line start
        assert!(execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]));
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(4),
            "Home after other command should go to indent again"
        );
    }

    #[test]
    fn home_no_indent_goes_to_line_start() {
        let mut dv = make_dv("hello world");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]));
        // No leading whitespace → goes directly to column 0
        assert_eq!(dv.cursor().offset, ByteIndex(0));
    }

    #[test]
    fn home_at_indent_goes_to_line_start() {
        let mut dv = make_dv("    hello");
        dv.cursor_move_to_offset(4); // already at indent start
        assert!(execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]));
        // At indent → first press should go to line start
        assert_eq!(dv.cursor().offset, ByteIndex(0), "Home at indent should go to line start");
    }

    // ── End (MoveToLineEnd) ──────────────────────────────────────────

    #[test]
    fn end_goes_to_line_end() {
        let mut dv = make_dv("hello world");
        assert!(execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(11), "End should go to end of line");
    }

    #[test]
    fn end_from_middle() {
        let mut dv = make_dv("hello world");
        dv.cursor_move_to_offset(3);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(11));
    }

    #[test]
    fn end_with_word_wrap_goes_to_visual_line_end() {
        // "abcdefghij" = 10 chars, simulated wrap at byte 5 → "abcde" + "fghij"
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["abcdefghij".to_string()], 10, 10.0);
        dv.cursor_move_to_offset(2);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
        ];
        assert!(execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac));
        // Non-last VL: cursor at vl_end (byte 5). End affinity in rendering
        // keeps caret on this visual line.
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "End with word wrap should go to visual line last char"
        );
    }

    // ── Shift+Home (ExtendToLineStart) — indent-aware ───────────────

    #[test]
    fn extend_to_line_start_first_press_goes_to_indent() {
        let mut dv = make_dv("    hello world");
        dv.cursor_move_to_offset(10); // in "world"
        assert!(execute_edit_command(&EditCommand::ExtendToLineStart, &mut dv, &[]));
        // First press → indent start (offset 4), selection active
        assert_eq!(dv.cursor().offset, ByteIndex(4), "Shift+Home should go to indent start");
        assert!(dv.cursor().selection_anchor.is_some(), "Selection should be active");
    }

    #[test]
    fn extend_to_line_start_second_press_goes_to_line_start() {
        let mut dv = make_dv("    hello world");
        dv.cursor_move_to_offset(10);
        execute_edit_command(&EditCommand::ExtendToLineStart, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(4));
        // Second press → line start (offset 0)
        assert!(execute_edit_command(&EditCommand::ExtendToLineStart, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(0), "Second Shift+Home should go to line start");
    }

    // ── Shift+End (ExtendToLineEnd) — soft-wrap aware ────────────────

    #[test]
    fn extend_to_line_end_with_word_wrap() {
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["abcdefghij".to_string()], 10, 10.0);
        dv.cursor_move_to_offset(2);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
        ];
        assert!(execute_edit_command(&EditCommand::ExtendToLineEnd, &mut dv, &ac));
        // Non-last VL: cursor at vl_end (byte 5). End affinity in rendering.
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "Shift+End with wrap should go to visual line last char"
        );
    }

    // ── Edge cases ─────────────────────────────────────────────────

    #[test]
    fn home_on_empty_line() {
        let mut dv = DocumentFixture::new(vec!["".to_string(), "hello".to_string()], 10, 10.0);
        // Cursor on empty line 0
        assert!(execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(0), "Home on empty line should stay at 0");
    }

    #[test]
    fn home_on_whitespace_only_line() {
        let mut dv = make_dv("     ");
        dv.cursor_move_to_offset(3);
        assert!(execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]));
        // All whitespace → goes to line start (offset 0)
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(0),
            "Home on all-whitespace line should go to line start"
        );
    }

    #[test]
    fn page_down_with_word_wrap_moves_by_display_rows() {
        // Line "abcdefghij" (10 chars) wraps at 5 → 2 display rows
        // With visible_rows=4, page_size=3
        let mut dv = DocumentFixture::new(vec!["abcde".to_string(); 20], 4, 10.0);
        // PageDown should move by 3 lines (visible_rows-1)
        assert!(execute_edit_command(&EditCommand::PageDown, &mut dv, &[]));
        assert_eq!(dv.cursor_line(), 3, "PageDown should move by visible_rows-1 lines");
    }

    #[test]
    fn home_toggle_independent_of_extend() {
        // Home and Shift+Home share the same toggle state
        let mut dv = make_dv("    hello world");
        dv.cursor_move_to_offset(10);
        // Regular Home → indent
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(4));
        // Shift+Home → should go to line start (second press)
        assert!(execute_edit_command(&EditCommand::ExtendToLineStart, &mut dv, &[]));
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(0),
            "Shift+Home after Home should go to line start"
        );
    }

    // ── Tab ──────────────────────────────────────────────────────────

    #[test]
    fn tab_inserts_spaces() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::Tab, &mut dv, &[]));
        assert_eq!(dv.document_lines()[0], "hello    ");
    }

    #[test]
    fn cut_without_a_selection_is_not_reported_as_executed() {
        let mut dv = make_dv("hello");
        assert!(!execute_edit_command(&EditCommand::Cut, &mut dv, &[]));
    }

    #[test]
    fn copy_without_a_selection_is_not_reported_as_executed() {
        let mut dv = make_dv("hello");
        assert!(!execute_edit_command(&EditCommand::Copy, &mut dv, &[]));
    }

    #[test]
    fn paste_without_clipboard_text_is_not_reported_as_executed() {
        let mut dv = make_dv("hello");
        let mut clipboard = TestClipboard { text: None, writes_succeed: true };

        let outcome = execute_with_clipboard(&EditCommand::Paste, &mut dv, &mut clipboard);

        assert!(!outcome.executed);
        assert_eq!(outcome.dirty_lines, None);
        assert_eq!(dv.buffer_len(), 5);
    }

    #[test]
    fn paste_plain_text_uses_the_plain_clipboard_compatibility_path() {
        let mut dv = make_dv("styled");
        dv.select_all();
        let mut clipboard = TestClipboard { text: Some("plain".to_owned()), writes_succeed: true };

        let outcome = execute_with_clipboard(&EditCommand::PastePlainText, &mut dv, &mut clipboard);

        assert!(outcome.executed);
        assert_eq!(dv.document_lines(), vec!["plain"]);
    }

    #[test]
    fn successful_copy_is_executed_without_dirty_document_lines() {
        let mut dv = make_dv("hello");
        dv.select_all();
        let mut clipboard = TestClipboard { text: None, writes_succeed: true };

        let outcome = execute_with_clipboard(&EditCommand::Copy, &mut dv, &mut clipboard);

        assert!(outcome.executed);
        assert_eq!(outcome.dirty_lines, None);
        assert_eq!(clipboard.text.as_deref(), Some("hello"));
        assert_eq!(dv.buffer_len(), 5);
    }

    #[test]
    fn failed_cut_preserves_the_document_and_is_not_reported_as_executed() {
        let mut dv = make_dv("hello");
        dv.select_all();
        let mut clipboard = TestClipboard { text: None, writes_succeed: false };

        let outcome = execute_with_clipboard(&EditCommand::Cut, &mut dv, &mut clipboard);

        assert!(!outcome.executed);
        assert_eq!(outcome.dirty_lines, None);
        assert_eq!(dv.extract_selected_text().as_deref(), Some(b"hello".as_slice()));
    }

    // ── Edge cases ───────────────────────────────────────────────────

    #[test]
    fn backspace_at_start_noop() {
        let mut dv = make_dv("hello");
        assert_eq!(dv.cursor().offset, ByteIndex(0));
        assert!(execute_edit_command(&EditCommand::Backspace, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(0));
        assert_eq!(dv.document_lines()[0], "hello");
    }

    #[test]
    fn backspace_with_empty_selection_works() {
        let mut dv = make_dv("hello world");
        dv.cursor_move_to_offset(5);
        dv.cursor_mut().selection_anchor = Some(5); // Empty selection

        // Execute Backspace
        execute_edit_command(&EditCommand::Backspace, &mut dv, &[]);

        // Should delete backward instead of being blocked
        assert_eq!(dv.cursor().offset, ByteIndex(4), "Cursor should move backward after deletion");
        assert_eq!(dv.cursor().selection_anchor, None, "Anchor should be cleared");
        assert_eq!(
            dv.document_lines()[0],
            "hell world",
            "Character before cursor should be deleted"
        );
    }

    #[test]
    fn delete_at_end_noop() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::DeleteForward, &mut dv, &[]));
        assert_eq!(dv.cursor().offset, ByteIndex(5));
    }
    #[test]
    fn delete_forward_at_eof_command_noop() {
        let mut dv = DocumentFixture::new(vec!["hi".to_string()], 10, 10.0);
        dv.cursor_move_to_offset(2); // at EOF
        let before_len = dv.buffer_len();
        assert!(execute_edit_command(&EditCommand::DeleteForward, &mut dv, &[]));
        assert_eq!(dv.buffer_len(), before_len, "delete at EOF should be noop");
        assert_eq!(dv.cursor().offset, ByteIndex(2));
    }

    #[test]
    fn backspace_zwj_emoji_one_key() {
        // ZWJ emoji: 👨‍👩‍👧 — one grapheme, multiple codepoints
        let emoji = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let line = format!("x{emoji}y");
        let mut dv = make_dv(&line);
        // cursor at byte offset after emoji: 1 + emoji.len()
        let after_emoji = 1 + emoji.len();
        dv.cursor_move_to_offset(after_emoji);
        assert!(execute_edit_command(&EditCommand::Backspace, &mut dv, &[]));
        let vis = dv.document_lines();
        assert_eq!(vis[0], "xy");
    }

    #[test]
    fn insert_char_cjk() {
        let mut dv = make_dv("");
        assert!(execute_edit_command(&EditCommand::InsertChar("世".into()), &mut dv, &[]));
        assert!(execute_edit_command(&EditCommand::InsertChar("界".into()), &mut dv, &[]));
        assert_eq!(dv.document_lines()[0], "世界");
        assert_eq!(dv.cursor().offset, ByteIndex(6));
    }

    #[test]
    fn move_left_at_start_noop() {
        let mut dv = make_dv("hello");
        assert_eq!(dv.cursor().offset, ByteIndex(0));
        execute_edit_command(&EditCommand::MoveLeft, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(0));
    }

    #[test]
    fn move_right_at_end_noop() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(5);
        execute_edit_command(&EditCommand::MoveRight, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(5));
    }

    #[test]
    fn move_right_at_end_of_last_line_with_trailing_newline() {
        // "line1\nline2\nline3\n" — 3 non-empty lines + trailing empty line.
        // Right arrow at end of "line3" should NOT wrap to the empty trailing line.
        let mut dv = make_dv("line1\nline2\nline3\n");
        // Move to end of "line3" (offset = 17)
        let line3_start = dv.line_byte_offset(2).unwrap();
        let line3_len = dv.line_byte_length(2).unwrap();
        dv.cursor_move_to_offset(line3_start + line3_len);
        let offset_before = dv.cursor().offset;
        assert_eq!(dv.cursor_line(), 2); // line 3 (0-indexed = 2)
        let result = execute_edit_command(&EditCommand::MoveRight, &mut dv, &[]);
        assert!(!result, "MoveRight at end of last non-empty line should return false.");
        assert_eq!(
            dv.cursor().offset,
            offset_before,
            "cursor should not move past end of last line."
        );
    }

    #[test]
    fn move_right_at_end_of_single_line_no_trailing_newline() {
        // Single line without trailing newline — cursor should stay at end.
        let mut dv = make_dv("abc");
        dv.cursor_move_to_offset(3);
        let result = execute_edit_command(&EditCommand::MoveRight, &mut dv, &[]);
        assert!(!result, "MoveRight at end of single-line doc should return false.");
        assert_eq!(dv.cursor().offset, ByteIndex(3));
    }

    #[test]
    fn move_right_middle_of_multi_line_still_works() {
        // Moving right in the middle of a multi-line doc should still work.
        let mut dv = make_dv("line1\nline2\nline3\n");
        let line1_start = dv.line_byte_offset(0).unwrap();
        dv.cursor_move_to_offset(line1_start + 2); // middle of "line1"
        let result = execute_edit_command(&EditCommand::MoveRight, &mut dv, &[]);
        assert!(result, "MoveRight in middle of line should return true.");
        assert_eq!(dv.cursor().offset, ByteIndex(line1_start + 3));
    }

    // ── A2: Visual-line Home/End with advance_cache ────────────────

    #[test]
    fn home_end_multisegment_visual_line() {
        // Simulate a long doc line wrapped into 3 visual segments at bytes 0..5, 5..10, 10..15
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 10,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
        ];
        // Cursor in segment 2 (byte 7)
        dv.cursor_move_to_offset(7);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // Non-last VL: cursor at vl_end (byte 10). End affinity keeps it on seg2.
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(10),
            "End in segment 2 should go to vl_end (byte 10)"
        );
        // Now Home from segment 2
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        // Cursor at boundary (10) → home_visual_line_bounds finds VL that starts at 10
        assert_eq!(dv.cursor().offset, ByteIndex(10), "Home at boundary → VL start = byte 10");
    }

    #[test]
    fn home_first_segment_goes_to_doc_start() {
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["hello world".to_string()], 10, 10.0);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(6, 72.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 6,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
        ];
        dv.cursor_move_to_offset(3); // in segment 1 ("hello ")
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(0),
            "Home in segment 1 should go to doc line start"
        );
    }

    #[test]
    fn end_last_segment_goes_to_doc_end() {
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["hello world".to_string()], 10, 10.0);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(6, 72.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 6,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
        ];
        dv.cursor_move_to_offset(8); // in segment 2 ("world")
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(11),
            "End in last segment should go to doc line end"
        );
    }

    #[test]
    fn shift_home_end_multisegment() {
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 10,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
        ];
        dv.cursor_move_to_offset(7); // segment 2
        execute_edit_command(&EditCommand::ExtendToLineEnd, &mut dv, &ac);
        // Non-last VL: cursor at vl_end (byte 10). End affinity keeps on seg2.
        assert_eq!(dv.cursor().offset, ByteIndex(10), "Shift+End in seg2 → byte 9");
        execute_edit_command(&EditCommand::ExtendToLineStart, &mut dv, &ac);
        // Cursor at boundary (10) → home_visual_line_bounds finds VL3 start=10
        assert_eq!(dv.cursor().offset, ByteIndex(10), "Shift+Home at boundary → byte 10");
    }

    #[test]
    fn home_end_fallback_when_cache_empty() {
        // advance_cache empty → falls back to doc-line behavior
        let mut dv = DocumentFixture::new(vec!["hello world".to_string()], 10, 10.0);
        dv.cursor_move_to_offset(3);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(11), "End with empty cache → doc line end");
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &[]);
        assert_eq!(dv.cursor().offset, ByteIndex(0), "Home with empty cache → doc line start");
    }

    #[test]
    fn home_at_visual_line_boundary_stays_on_current_line() {
        // Regression: pressing Home at the start of a visual line (which is
        // also the end of the previous visual line) used to jump to the
        // previous line because cursor_visual_line_bounds matched the
        // previous entry (local_offset <= vl_end with <=).
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
        // 3 visual segments: bytes 0..5, 5..10, 10..16
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 10,
                vl_grapheme_start: 0,
                clusters: vec![(6, 72.0, 0)],
            },
        ];
        // Move cursor to byte 5 (start of segment 2, end of segment 1).
        dv.cursor_move_to_offset(5);
        // First Home -> indent or visual line start (byte 5, no indent).
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "First Home at seg2 start should stay at byte 5"
        );
        // Second Home -> visual line start (byte 5 again since no indent).
        // Before fix this would jump to byte 0 (seg1 start).
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(0),
            "Second Home from seg2 start should go to doc start"
        );
    }

    #[test]
    fn home_third_press_goes_to_doc_start() {
        // Verify that pressing Home enough times still reaches doc start
        // (the toggle between indent/start should still work).
        use ui::render_geom::AdvanceCacheEntry;
        let mut dv = DocumentFixture::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 10,
                vl_grapheme_start: 0,
                clusters: vec![(6, 72.0, 0)],
            },
        ];
        // Start from middle of segment 2
        dv.cursor_move_to_offset(7);
        // First Home -> seg2 start (byte 5)
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        assert_eq!(dv.cursor().offset, ByteIndex(5));
        // Second Home -> doc start (byte 0), since at seg2 start already
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        assert_eq!(dv.cursor().offset, ByteIndex(0), "Second Home from seg start -> doc start");
    }

    #[test]
    fn indent_home_in_visual_line() {
        use ui::render_geom::AdvanceCacheEntry;
        // "    hello world" — indented. Visual line 1: "    hello ", VL2: "world"
        let mut dv = DocumentFixture::new(vec!["    hello world".to_string()], 10, 10.0);
        let ac = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(10, 120.0, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 10,
                vl_grapheme_start: 0,
                clusters: vec![(5, 60.0, 0)],
            },
        ];
        dv.cursor_move_to_offset(7); // in "hello" part of segment 1
        // First Home → indent start (byte 4)
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        assert_eq!(dv.cursor().offset, ByteIndex(4), "First Home → indent start at byte 4");
        // Second Home → visual line start (byte 0)
        execute_edit_command(&EditCommand::MoveToLineStart, &mut dv, &ac);
        assert_eq!(dv.cursor().offset, ByteIndex(0), "Second Home → visual line start at byte 0");
    }

    // ── EditOutcome tests ──────────────────────────────────────────────

    #[test]
    fn edit_outcome_insert_char_marks_current_line() {
        let mut dv = make_dv("hello\nworld");
        let out = execute_edit_command_v2(&EditCommand::InsertChar("X".into()), &mut dv, &[]);
        assert!(out.executed);
        assert_eq!(out.old_line_count, 2);
        assert_eq!(out.new_line_count, 2);
        assert_eq!(out.dirty_lines, Some(0..1));
    }

    #[test]
    fn edit_outcome_insert_newline_marks_split_lines() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(2); // "he|llo"
        let out = execute_edit_command_v2(&EditCommand::InsertNewline, &mut dv, &[]);
        assert!(out.executed);
        assert_eq!(out.old_line_count, 1);
        assert_eq!(out.new_line_count, 2);
        assert_eq!(out.dirty_lines, Some(0..1));
    }

    #[test]
    fn edit_outcome_backspace_at_line_start_merges() {
        let mut dv = make_dv("hello\nworld");
        dv.cursor_move_to_offset(6); // line 1 start
        let out = execute_edit_command_v2(&EditCommand::Backspace, &mut dv, &[]);
        assert_eq!(out.old_line_count, 2);
        assert_eq!(out.new_line_count, 1);
        assert_eq!(out.dirty_lines, Some(0..2));
    }

    #[test]
    fn edit_outcome_movement_no_dirty() {
        let mut dv = make_dv("hello");
        let out = execute_edit_command_v2(&EditCommand::MoveRight, &mut dv, &[]);
        assert!(out.executed);
        assert_eq!(out.dirty_lines, None);
    }

    // ── End boundary regression tests (real shaper-style multi-cluster) ───

    /// Build a realistic advance_cache entry with per-character (byte)
    /// clusters, matching real shaper output.
    fn make_realistic_entry(
        doc_line: usize,
        vl_byte_start: usize,
        byte_count: usize,
        char_advance: f32,
    ) -> AdvanceCacheEntry {
        let margin = 8.0f32;
        let mut clusters = Vec::new();
        let mut x = margin;
        for b in 1..=byte_count {
            x += char_advance;
            clusters.push((b, x, 0)); // vl-local byte offset
        }
        AdvanceCacheEntry { doc_line, vl_byte_start, vl_grapheme_start: 0, clusters }
    }

    #[test]
    fn end_multi_cluster_nonlast_vl() {
        // Simulate real shaper: each VL has 5 per-char clusters
        let mut dv = DocumentFixture::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
        let ac = vec![
            make_realistic_entry(0, 0, 5, 7.8),
            make_realistic_entry(0, 5, 5, 7.8),
            make_realistic_entry(0, 10, 6, 7.8),
        ];
        // Cursor in VL0 at byte 3
        dv.cursor_move_to_offset(3);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // Non-last VL: cursor at vl_end (5). End affinity in rendering.
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "End in VL0 (multi-cluster) should go to vl_end byte 5"
        );
    }

    #[test]
    fn end_at_vl_boundary_in_previous_vl() {
        // Half-open: byte 5 belongs to VL1 (start of wrapped portion).
        // End goes to VL1's end (byte 10). For mouse clicks, click_hint
        // ensures the correct VL; for keyboard, End follows half-open semantics.
        let mut dv = DocumentFixture::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
        let ac = vec![
            make_realistic_entry(0, 0, 5, 7.8),
            make_realistic_entry(0, 5, 5, 7.8),
            make_realistic_entry(0, 10, 6, 7.8),
        ];
        // Cursor at byte 5 = start of VL1 (half-open semantics)
        dv.cursor_move_to_offset(5);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // End on VL1 → goes to VL1's end at byte 10
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(10),
            "End at VL1 start should go to VL1 end (byte 10)"
        );
    }

    #[test]
    fn end_last_vl_with_newline_like_boundary() {
        // Simulate where last VL vl_end might be before line_len
        // (e.g., trailing newline excluded by shaper).
        let mut dv = DocumentFixture::new(vec!["abcde".to_string()], 10, 10.0);
        let ac = vec![make_realistic_entry(0, 0, 5, 7.8)];
        dv.cursor_move_to_offset(2);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // Last VL, no newline: vl_end=5, last_byte_at_4='e', not newline -> target=5
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "End on last VL without newline should go to doc line end"
        );
    }

    #[test]
    fn end_last_vl_multi_line_doc_newline_boundary() {
        // Two lines: "hello" + newline + "world"
        // Line 0: "hello" (5 bytes content), line_len=5 (newline NOT included)
        let mut dv = DocumentFixture::new(vec!["hello".to_string(), "world".to_string()], 20, 20.0);
        let ac = vec![make_realistic_entry(0, 0, 5, 7.8), make_realistic_entry(1, 0, 5, 7.8)];
        dv.cursor_move_to_offset(2); // line 0
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // line_len for line 0 = 5 (newline excluded from lengths)
        // vl_end = 5, is_last_vl_of_doc: 5 >= 5? Yes
        // last_byte at 4 = 'o', not newline -> target = 5
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(5),
            "End on line 0 (before newline) should go to byte 5"
        );
    }

    #[test]
    fn extend_to_line_end_nonlast_vl_should_stay_in_vl() {
        // Bug: ExtendToLineEnd does NOT have is_last_vl_of_doc check.
        // For non-last VLs, it always does newline check -> target = vl_end.
        // This puts cursor at exclusive end = start of next VL.
        let mut dv = DocumentFixture::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
        let ac = vec![
            make_realistic_entry(0, 0, 5, 7.8),
            make_realistic_entry(0, 5, 5, 7.8),
            make_realistic_entry(0, 10, 6, 7.8),
        ];
        dv.cursor_move_to_offset(3); // VL0
        execute_edit_command(&EditCommand::ExtendToLineEnd, &mut dv, &ac);
        // ExtendToLineEnd should also stay within VL for non-last VLs
        // Cursor at vl_end (byte 5). End affinity keeps it on this VL in rendering.
        assert_eq!(dv.cursor().offset, ByteIndex(5), "Shift+End in non-last VL goes to vl_end");
    }

    // ── CJK (multi-byte) End regression tests ───────────────────────

    /// Build a CJK advance_cache entry with 3-byte-per-char clusters.
    fn make_cjk_entry(
        doc_line: usize,
        vl_byte_start: usize,
        char_count: usize,
        char_advance: f32,
    ) -> AdvanceCacheEntry {
        let margin = 8.0f32;
        let mut clusters = Vec::new();
        let mut x = margin;
        let mut byte_off = vl_byte_start;
        for _ in 0..char_count {
            x += char_advance;
            byte_off += 3; // CJK: 3 bytes per char
            clusters.push((byte_off, x, 0));
        }
        AdvanceCacheEntry { doc_line, vl_byte_start, vl_grapheme_start: 0, clusters }
    }

    #[test]
    fn end_cjk_nonlast_vl() {
        // "关于广东省" = 5 chars, 15 bytes
        // VL0: 3 chars [0,9), VL1: 2 chars [9,15)
        let mut dv = DocumentFixture::new(vec!["关于广东省".to_string()], 10, 10.0);
        let ac = vec![make_cjk_entry(0, 0, 3, 20.0), make_cjk_entry(0, 9, 2, 20.0)];
        dv.cursor_move_to_offset(3);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // vl_end=9. End affinity in rendering keeps on VL0.
        assert_eq!(dv.cursor().offset, ByteIndex(9), "End in CJK VL0 should go to byte 9 (vl_end)");
    }

    #[test]
    fn end_cjk_second_vl_is_last() {
        // "关于广东省珠海市" = 8 chars, 24 bytes
        // VL0: 4 chars [0,12), VL1: 4 chars [12,24)
        let mut dv = DocumentFixture::new(vec!["关于广东省珠海市".to_string()], 10, 10.0);
        let ac = vec![make_cjk_entry(0, 0, 4, 18.0), make_cjk_entry(0, 12, 4, 18.0)];
        dv.cursor_move_to_offset(15);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        assert_eq!(
            dv.cursor().offset,
            ByteIndex(24),
            "End in CJK last VL should go to doc end byte 24"
        );
    }

    #[test]
    fn end_cjk_boundary_between_vls() {
        // Half-open: byte 12 belongs to VL1 (start of wrapped CJK portion).
        // End goes to VL1's end (byte 24). Previous inclusive-end rule kept
        // cursor at VL0's end (byte 12); half-open correctly places it on VL1.
        let mut dv = DocumentFixture::new(vec!["关于广东省珠海市".to_string()], 10, 10.0);
        let ac = vec![make_cjk_entry(0, 0, 4, 18.0), make_cjk_entry(0, 12, 4, 18.0)];
        dv.cursor_move_to_offset(12);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // Half-open: byte 12 → VL1 → End → VL1's end at 24
        assert_eq!(dv.cursor().offset, ByteIndex(24), "End in CJK VL1 should go to VL1 end (24)");
    }

    #[test]
    fn end_cjk_mid_char_byte_offset() {
        let mut dv = DocumentFixture::new(vec!["关于广东省".to_string()], 10, 10.0);
        let ac = vec![make_cjk_entry(0, 0, 3, 20.0), make_cjk_entry(0, 9, 2, 20.0)];
        dv.cursor_move_to_offset(4);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        assert_eq!(dv.cursor().offset, ByteIndex(9), "End should go to vl_end (9)");
    }

    // ── Enter / Backspace / rebuild: consecutive-newline scanning ──
    //
    // These cover both rescan_from (incremental) and rebuild_from (full)
    // paths that were broken when two \n bytes appeared consecutively.

    /// Enter on a single-space line with a slash-line right after.
    /// Covers both cursor-at-end-of-space and cursor-at-start-of-space.
    #[test]
    fn enter_on_space_line_preserves_next() {
        // --- cursor at end of space-line (after the space, at the LF) ---
        let mut dv = DocumentFixture::new(
            [
                "line1",
                "line2",
                "line3",
                "line4",
                "long line five",
                " ",
                " /Users/foo/bar  some text",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            60,
            10.0,
        );
        let body_end: usize = [6, 6, 6, 6, 15].iter().sum(); // bytes before " \n"
        dv.cursor_move_to_offset(body_end + 1);
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        let vis = dv.document_lines();
        assert_eq!(vis.len(), 8);
        assert_eq!(vis[5], " ");
        assert_eq!(vis[6], "");
        assert!(vis[7].starts_with(" /Users/"), "slash eaten: {:?}", vis[7]);

        // --- cursor at start of space-line (before the space) ---
        let mut dv = DocumentFixture::new(
            ["line1", " ", " /Users/foo"].iter().map(|s| s.to_string()).collect(),
            60,
            10.0,
        );
        dv.cursor_move_to_offset(6); // after "line1\n"
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        let vis = dv.document_lines();
        assert_eq!(vis[0], "line1");
        assert_eq!(vis[1], "");
        assert_eq!(vis[2], " ");
        assert!(vis[3].starts_with(" /Users/"), "slash eaten: {:?}", vis[3]);
    }

    /// Enter in middle of a line splits it correctly.
    #[test]
    fn enter_splits_line() {
        let mut dv = make_dv("hello world");
        dv.cursor_move_to_offset(5);
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        let vis = dv.document_lines();
        assert_eq!(vis[0], "hello");
        assert_eq!(vis[1], " world");
    }

    /// Consecutive Enter presses must each produce an empty line.
    #[test]
    fn consecutive_enter_empty_lines() {
        // Double Enter between two lines
        let mut dv = make_dv(
            "a
b",
        );
        dv.cursor_move_to_offset(2);
        for _ in 0..2 {
            execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        }
        let vis = dv.document_lines();
        assert_eq!(vis, &["a", "", "", "b"]);

        // Triple Enter at end of a single-space line
        let mut dv =
            DocumentFixture::new(["x", " ", "y"].iter().map(|s| s.to_string()).collect(), 10, 10.0);
        dv.cursor_move_to_offset(3); // after "x\n "
        for _ in 0..3 {
            execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        }
        let vis = dv.document_lines();
        assert_eq!(vis, &["x", " ", "", "", "", "y"]);

        // Enter at offset 0
        let mut dv = make_dv(
            "first
second",
        );
        dv.cursor_move_to_offset(0);
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        assert_eq!(dv.document_lines(), &["", "first", "second"]);
    }

    /// Backspace across line boundaries must preserve content.
    #[test]
    fn backspace_across_lines_preserves_content() {
        // Backspace at line start merges with previous line
        let mut dv = make_dv(
            "hello
world",
        );
        dv.cursor_move_to_offset(6);
        execute_edit_command(&EditCommand::Backspace, &mut dv, &[]);
        assert_eq!(dv.document_lines(), &["helloworld"]);

        // Backspace across consecutive empty lines
        let mut dv = DocumentFixture::new(
            ["a", "", "", "b"].iter().map(|s| s.to_string()).collect(),
            10,
            10.0,
        );
        dv.cursor_move_to_offset(4); // "a\n\n\n|b"
        execute_edit_command(&EditCommand::Backspace, &mut dv, &[]);
        assert_eq!(dv.document_lines(), &["a", "", "b"]);
    }

    /// Full rebuild (rebuild_from) preserves consecutive empty lines.
    #[test]
    fn rebuild_preserves_empty_lines() {
        let dv = DocumentFixture::new(
            ["a", "", "", "", "b"].iter().map(|s| s.to_string()).collect(),
            10,
            10.0,
        );
        let rebuilt = LineIndex::rebuild_from(&dv.tb);
        assert_eq!(rebuilt.line_count(), 5);
        assert_eq!(dv.document_lines(), &["a", "", "", "", "b"]);
    }

    /// Enter at end of a non-empty line that has an empty line right after.
    /// Must insert a new line, not just move cursor to the next line.
    #[test]
    fn enter_at_end_of_line_before_empty_line() {
        // Simulates: line 8 "/Users/...", line 9 "", line 10 " "
        let mut dv = DocumentFixture::new(
            vec!["/Users/foo/bar".to_string(), "".to_string(), " ".to_string()],
            10,
            10.0,
        );
        // Cursor at end of line 0: after "/Users/foo/bar\n"
        dv.cursor_move_to_offset(15); // "/Users/foo/bar\n" = 15 bytes
        let old = dv.line_count();
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        let vis = dv.document_lines();
        // Should now have 4 lines: "/Users/foo/bar", "", "", " "
        assert_eq!(dv.line_count(), old + 1, "line count should increase by 1");
        assert_eq!(vis[0], "/Users/foo/bar");
        assert_eq!(vis[1], "");
        assert_eq!(vis[2], "");
        assert_eq!(vis[3], " ");
    }
}
