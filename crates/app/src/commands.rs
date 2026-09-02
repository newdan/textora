//! Command dispatch for editor operations.
//!
//! Extracts the `execute_edit_command` function from DocumentView,
//! which is a pure command dispatcher that doesn't depend on DocumentView's internal state.

use crate::cursor_motion::find_visual_line_index;
use crate::document_view::DocumentView;
use crate::edit_transaction::DocumentModelMut;
use crate::input::EditCommand;
use appkit_core::document::DocumentModel;
use appkit_shell::SystemClipboard;
use ui::core::Clipboard;
use ui::render_geom::AdvanceCacheEntry;

/// Execute an editor command on the document view.
/// Returns true if the command was handled and the document was modified.
/// Returns (visual_line_abs_start, visual_line_abs_end) for the cursor's current
/// visual line, using advance_cache. Falls back to doc-line bounds when the cursor
/// is not in the visible area.
fn cursor_visual_line_bounds(
    dv: &DocumentModel,
    advance_cache: &[AdvanceCacheEntry],
) -> (usize, usize) {
    let cursor_line = dv.cursor_line();
    let line_abs_start = dv.line_byte_offset(cursor_line).unwrap_or(0);
    let local_offset = dv.cursor().offset.to_usize().saturating_sub(line_abs_start);
    // Build bounds list for the current doc line
    let mut bounds: Vec<(usize, usize)> = Vec::new();
    let mut entry_indices: Vec<usize> = Vec::new();
    for (i, entry) in advance_cache.iter().enumerate() {
        if entry.doc_line == cursor_line && !entry.clusters.is_empty() {
            // clusters store vl-local end; add vl_byte_start for line-local absolute end
            let end = entry.vl_byte_start + entry.clusters.last().map(|&(e, _, _)| e).unwrap_or(0);
            bounds.push((entry.vl_byte_start, end));
            entry_indices.push(i);
        }
    }
    if bounds.is_empty() {
        // Fallback: not in visible area -> use document line bounds.
        let line_len = dv.line_index.lengths.get(cursor_line).copied().unwrap_or(0);
        return (line_abs_start, line_abs_start + line_len);
    }
    let vl_idx = find_visual_line_index(&bounds, local_offset);
    let entry = &advance_cache[entry_indices[vl_idx]];
    let vl_start = entry.vl_byte_start;
    let vl_end = vl_start + entry.clusters.last().map(|&(end, _, _)| end).unwrap_or(0);
    (line_abs_start + vl_start, line_abs_start + vl_end)
}

/// Like `cursor_visual_line_bounds`, but when the cursor is exactly at the
/// boundary between two visual lines (vl_end of N == vl_start of N+1),
/// returns the bounds of the *next* visual line (N+1).  This is what Home
/// expects: the cursor at the start of a visual line should stay on that line.
fn home_visual_line_bounds(
    dv: &DocumentModel,
    advance_cache: &[AdvanceCacheEntry],
) -> (usize, usize) {
    let (vl_start, vl_end) = cursor_visual_line_bounds(dv, advance_cache);
    if dv.cursor().offset.to_usize() != vl_end {
        return (vl_start, vl_end);
    }
    // Cursor at boundary — find the visual line that *starts* here.
    let cursor_line = dv.cursor_line();
    let line_abs_start = dv.line_byte_offset(cursor_line).unwrap_or(0);
    let local_offset = dv.cursor().offset.to_usize().saturating_sub(line_abs_start);
    for entry in advance_cache {
        if entry.doc_line == cursor_line
            && entry.vl_byte_start == local_offset
            && !entry.clusters.is_empty()
        {
            let end = entry.vl_byte_start + entry.clusters.last().map(|&(e, _, _)| e).unwrap_or(0);
            return (line_abs_start + entry.vl_byte_start, line_abs_start + end);
        }
    }
    (vl_start, vl_end)
}

/// Compute the indent offset (first non-whitespace byte) within a visual line range.
fn visual_line_indent_offset(dv: &DocumentModel, vl_start: usize, vl_end: usize) -> usize {
    let mut off = vl_start;
    while off < vl_end {
        let chunk = dv.tb.read_forward(off);
        if chunk.is_empty() {
            break;
        }
        for &b in chunk.iter() {
            if off >= vl_end {
                break;
            }
            if b != b' ' && b != b'\t' {
                return off;
            }
            off += 1;
        }
    }
    vl_start
}

/// 编辑命令的副作用范围。
#[derive(Default, Debug, Clone)]
pub struct EditOutcome {
    /// 命令是否真实执行。
    pub executed: bool,
    /// 受影响的 doc line 区间 [start, end_exclusive)。
    pub dirty_lines: Option<std::ops::Range<usize>>,
    /// 命令执行前的 line_count。
    pub old_line_count: usize,
    /// 命令执行后的 line_count。
    pub new_line_count: usize,
}

impl EditOutcome {
    pub(crate) fn invalidates_all_render_cache(&self) -> bool {
        self.dirty_lines.is_some() && self.new_line_count != self.old_line_count
    }

    pub(crate) fn render_cache_invalidation_range(&self) -> Option<std::ops::Range<usize>> {
        if self.invalidates_all_render_cache() {
            return None;
        }
        let dirty_lines = self.dirty_lines.as_ref()?;
        Some(dirty_lines.clone())
    }
}

/// 包装 `execute_edit_command`，返回受影响行范围。
pub(crate) fn execute_edit_command_v2(
    cmd: &EditCommand,
    dv: &mut DocumentView,
    advance_cache: &[AdvanceCacheEntry],
) -> EditOutcome {
    let mut presentation = dv.take_presentation();
    let page_step_rows = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
    let outcome = execute_edit_command_v2_with_presentation(
        cmd,
        dv,
        advance_cache,
        &mut presentation.cursor_render_state,
        page_step_rows,
    );
    dv.restore_presentation(presentation);
    outcome
}

pub(crate) fn execute_edit_command_v2_with_presentation(
    cmd: &EditCommand,
    dv: &mut impl DocumentModelMut,
    advance_cache: &[AdvanceCacheEntry],
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
    page_step_rows: usize,
) -> EditOutcome {
    let mut clipboard = SystemClipboard;
    execute_edit_command_v2_with_presentation_and_clipboard(
        cmd,
        dv,
        advance_cache,
        cursor_render_state,
        page_step_rows,
        &mut clipboard,
    )
}

fn execute_edit_command_v2_with_presentation_and_clipboard(
    cmd: &EditCommand,
    dv: &mut impl DocumentModelMut,
    advance_cache: &[AdvanceCacheEntry],
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
    page_step_rows: usize,
    clipboard: &mut dyn Clipboard,
) -> EditOutcome {
    let dv = dv.document_model_mut();
    let old_line_count = dv.line_count();
    let _cursor_line_before = dv.cursor_line();

    let mut min_line = dv.cursor_line();
    let mut max_line = min_line;
    if let Some((start, end)) = dv.selection_range() {
        min_line = match dv.line_index.offsets.binary_search(&start) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        max_line = match dv.line_index.offsets.binary_search(&end) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
    }

    let executed = execute_edit_command_with_presentation(
        cmd,
        dv,
        advance_cache,
        cursor_render_state,
        page_step_rows,
        clipboard,
    );

    let new_line_count = dv.line_count();
    let cursor_line_after = dv.cursor_line();

    let dirty_lines = if !executed {
        None
    } else {
        let start = min_line.min(cursor_line_after);
        match cmd {
            EditCommand::Paste
            | EditCommand::PastePlainText
            | EditCommand::Undo
            | EditCommand::Redo => Some(0..old_line_count),
            EditCommand::MoveLeft
            | EditCommand::MoveRight
            | EditCommand::MoveUp
            | EditCommand::MoveDown
            | EditCommand::MoveToLineStart
            | EditCommand::MoveToLineEnd
            | EditCommand::PageUp
            | EditCommand::PageDown
            | EditCommand::ExtendToLineStart
            | EditCommand::ExtendToLineEnd
            | EditCommand::Copy => None,
            _ => {
                let lines_deleted = old_line_count.saturating_sub(new_line_count);
                let end = max_line + 1 + lines_deleted;
                Some(start..end.min(old_line_count))
            }
        }
    };
    EditOutcome { executed, dirty_lines, old_line_count, new_line_count }
}

pub(crate) fn execute_edit_command(
    cmd: &EditCommand,
    dv: &mut DocumentView,
    advance_cache: &[AdvanceCacheEntry],
) -> bool {
    let mut clipboard = SystemClipboard;
    let mut presentation = dv.take_presentation();
    let page_step_rows = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
    let executed = execute_edit_command_with_presentation(
        cmd,
        dv,
        advance_cache,
        &mut presentation.cursor_render_state,
        page_step_rows,
        &mut clipboard,
    );
    dv.restore_presentation(presentation);
    executed
}

fn execute_edit_command_with_presentation(
    cmd: &EditCommand,
    dv: &mut DocumentModel,
    advance_cache: &[AdvanceCacheEntry],
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
    page_step_rows: usize,
    clipboard: &mut dyn Clipboard,
) -> bool {
    // Save and reset the Home toggle state before handling the command.
    let was_home = dv.cursor().last_command_was_home;
    dv.cursor_mut().last_command_was_home = false;
    dv.cursor_mut().last_command_was_end = false;
    cursor_render_state.click_hint = None;

    match cmd {
        EditCommand::ToggleSidebarPin => false,
        EditCommand::ToggleView => false,
        EditCommand::ToggleToc => false,
        EditCommand::NextChapter => false,
        EditCommand::PrevChapter => false,
        EditCommand::MoveLeft => {
            dv.cursor_move_left();
            true
        }
        EditCommand::MoveRight => {
            let before = dv.cursor().offset;
            dv.cursor_move_right();
            // Prevent cursor from resting on a trailing empty line at EOF.
            let line = dv.cursor_line();
            let len = dv.line_byte_length(line).unwrap_or(0);
            if len == 0 && line + 1 == dv.line_count() && dv.cursor().offset > before {
                // Moved to trailing empty line — revert.
                dv.cursor_move_to_offset(before.to_usize());
                return false;
            }
            if dv.cursor().offset == before {
                return false;
            }
            true
        }
        EditCommand::MoveUp => {
            dv.cursor_move_up();
            true
        }
        EditCommand::MoveDown => {
            dv.cursor_move_down();
            true
        }
        EditCommand::MoveToLineStart => {
            if !advance_cache.is_empty() {
                let (vl_start, vl_end) = home_visual_line_bounds(dv, advance_cache);
                let indent_offset = visual_line_indent_offset(dv, vl_start, vl_end);
                let doc_line_start = dv.line_byte_offset(dv.cursor_line()).unwrap_or(0);
                let target = if was_home && dv.cursor().offset.to_usize() != indent_offset {
                    vl_start
                } else if dv.cursor().offset.to_usize() == indent_offset {
                    // Already at indent (or no indent).  If we're also at the
                    // visual line start and it's not the doc line start, jump
                    // back to doc line start so repeated Home reaches column 0.
                    if was_home
                        && dv.cursor().offset.to_usize() == vl_start
                        && vl_start != doc_line_start
                    {
                        doc_line_start
                    } else {
                        vl_start
                    }
                } else {
                    indent_offset
                };
                dv.cursor_move_to_offset(target);
            } else {
                let indent_offset = dv.indent_column_offset();
                if was_home && dv.cursor().offset.to_usize() != indent_offset {
                    dv.cursor_move_to_line_start();
                } else if dv.cursor().offset.to_usize() == indent_offset {
                    dv.cursor_move_to_line_start();
                } else {
                    dv.cursor_move_to_offset(indent_offset);
                }
            }
            dv.cursor_mut().last_command_was_home = true;
            true
        }
        EditCommand::MoveToLineEnd => {
            if !advance_cache.is_empty() {
                let (_vl_start, vl_end) = cursor_visual_line_bounds(dv, advance_cache);
                // Determine if this is the last visual line of the doc line
                let cursor_line = dv.cursor_line();
                let line_abs_start = dv.line_byte_offset(cursor_line).unwrap_or(0);
                let line_len = dv.line_index.lengths.get(cursor_line).copied().unwrap_or(0);
                let is_last_vl_of_doc = vl_end >= line_abs_start + line_len;
                let target = if vl_end > 0 {
                    if is_last_vl_of_doc {
                        // Last visual line of doc: use newline check (existing logic)
                        let last_byte =
                            dv.tb.read_forward(vl_end - 1).first().copied().unwrap_or(0);
                        if last_byte == b'\n' || last_byte == b'\r' { vl_end - 1 } else { vl_end }
                    } else {
                        // Non-last VL: go to vl_end (after last char).
                        // Rendering pipeline uses last_command_was_end to prefer
                        // this VL at boundaries instead of the next VL.
                        dv.cursor_mut().last_command_was_end = true;
                        vl_end
                    }
                } else {
                    vl_end
                };
                dv.cursor_move_to_offset(target);
            } else {
                dv.cursor_move_to_line_end();
            }
            true
        }
        EditCommand::MoveToDocStart => {
            dv.cursor_move_to_offset(0);
            true
        }
        EditCommand::MoveToDocEnd => {
            dv.cursor_move_to_offset(dv.buffer_len());
            true
        }
        EditCommand::MoveWordLeft => {
            dv.cursor_move_word_left();
            true
        }
        EditCommand::MoveWordRight => {
            dv.cursor_move_word_right();
            true
        }
        EditCommand::Backspace => {
            if !dv.delete_selection() {
                dv.delete_backward(1);
            }
            true
        }
        EditCommand::DeleteForward => {
            if !dv.delete_selection() {
                dv.delete_forward(1);
            }
            true
        }
        EditCommand::DeleteRange(range) => {
            let start = range.start.min(dv.buffer_len());
            let end = range.end.min(dv.buffer_len());
            if start < end {
                dv.cursor_move_to_offset(end);
                dv.cursor_mut().selection_anchor = Some(start);
                dv.delete_selection();
            }
            true
        }
        EditCommand::ReplaceRange { range, text } => {
            crate::edit_transaction::execute_text_replacement(
                &ui::plugin::TextReplacement { range: range.clone(), text: text.clone() },
                range.start + text.len(),
                dv,
            )
        }
        EditCommand::InsertNewline => {
            dv.delete_selection();
            // Always insert \n; TextBuffer normalizes to CRLF when newlines_are_crlf is set.
            dv.insert_at_cursor(b"\n");
            true
        }
        EditCommand::InsertChar(s) => {
            dv.delete_selection();
            dv.insert_at_cursor(s.as_bytes());
            true
        }
        EditCommand::InsertText(s) => {
            dv.delete_selection();
            dv.insert_at_cursor(s.as_bytes());
            true
        }
        EditCommand::Undo => {
            dv.undo();
            true
        }
        EditCommand::Redo => {
            dv.redo();
            true
        }
        EditCommand::SelectAll => {
            dv.select_all();
            true
        }
        EditCommand::PageUp => {
            let vpos = dv.tb.cursor_visual_pos();
            let new_row = (vpos.row as isize - page_step_rows as isize).max(0) as usize;
            dv.tb.cursor_move_to_visual(core::types::VisualPoint {
                column: vpos.column,
                row: new_row,
            });
            dv.sync_cursor();
            true
        }
        EditCommand::PageDown => {
            let vpos = dv.tb.cursor_visual_pos();
            let new_row = (vpos.row as isize + page_step_rows as isize).max(0) as usize;
            dv.tb.cursor_move_to_visual(core::types::VisualPoint {
                column: vpos.column,
                row: new_row,
            });
            dv.sync_cursor();
            true
        }
        EditCommand::Tab => {
            if dv.has_selection() {
                dv.delete_selection();
            }
            let tab_size = dv.tb.tab_size() as usize;
            if dv.tb.indent_with_tabs() {
                dv.insert_at_cursor(b"\t");
            } else {
                // tab_size is clamped to 1..=8 by TextBuffer
                const SPACES: &[u8] = b"        ";
                dv.insert_at_cursor(&SPACES[..tab_size]);
            }
            true
        }
        // ── Selection extension (Shift+Arrow) ──
        EditCommand::ExtendLeft => {
            dv.extend_selection_left();
            true
        }
        EditCommand::ExtendRight => {
            dv.extend_selection_right();
            true
        }
        EditCommand::ExtendUp => {
            dv.extend_selection_up();
            true
        }
        EditCommand::ExtendDown => {
            dv.extend_selection_down();
            true
        }
        EditCommand::ExtendWordLeft => {
            dv.extend_selection_word_left();
            true
        }
        EditCommand::ExtendWordRight => {
            dv.extend_selection_word_right();
            true
        }
        EditCommand::ExtendToLineStart => {
            if !advance_cache.is_empty() {
                let (vl_start, vl_end) = home_visual_line_bounds(dv, advance_cache);
                let indent_offset = visual_line_indent_offset(dv, vl_start, vl_end);
                let doc_line_start = dv.line_byte_offset(dv.cursor_line()).unwrap_or(0);
                let target = if was_home && dv.cursor().offset.to_usize() != indent_offset {
                    vl_start
                } else if dv.cursor().offset.to_usize() == indent_offset {
                    if was_home
                        && dv.cursor().offset.to_usize() == vl_start
                        && vl_start != doc_line_start
                    {
                        doc_line_start
                    } else {
                        vl_start
                    }
                } else {
                    indent_offset
                };
                dv.ensure_selection_active();
                dv.set_cursor_offset_synced(target);
            } else {
                let indent_offset = dv.indent_column_offset();
                if was_home && dv.cursor().offset.to_usize() != indent_offset {
                    dv.extend_selection_to_line_start();
                } else if dv.cursor().offset.to_usize() == indent_offset {
                    dv.extend_selection_to_line_start();
                } else {
                    dv.ensure_selection_active();
                    dv.set_cursor_offset_synced(indent_offset);
                }
            }
            dv.cursor_mut().last_command_was_home = true;
            true
        }
        EditCommand::ExtendToLineEnd => {
            if !advance_cache.is_empty() {
                dv.ensure_selection_active();
                let (_vl_start, vl_end) = cursor_visual_line_bounds(dv, advance_cache);
                // Determine if this is the last visual line of the doc line
                let cursor_line = dv.cursor_line();
                let line_abs_start = dv.line_byte_offset(cursor_line).unwrap_or(0);
                let line_len = dv.line_index.lengths.get(cursor_line).copied().unwrap_or(0);
                let is_last_vl_of_doc = vl_end >= line_abs_start + line_len;
                let target = if vl_end > 0 {
                    if is_last_vl_of_doc {
                        let last_byte =
                            dv.tb.read_forward(vl_end - 1).first().copied().unwrap_or(0);
                        if last_byte == b'\n' || last_byte == b'\r' { vl_end - 1 } else { vl_end }
                    } else {
                        // Non-last VL: go to vl_end. Rendering uses
                        // last_command_was_end for boundary affinity.
                        dv.cursor_mut().last_command_was_end = true;
                        vl_end
                    }
                } else {
                    vl_end
                };
                dv.set_cursor_offset_synced(target);
            } else {
                dv.extend_selection_to_line_end();
            }
            true
        }
        EditCommand::ExtendToDocStart => {
            dv.extend_selection_to_doc_start();
            true
        }
        EditCommand::ExtendToDocEnd => {
            dv.extend_selection_to_doc_end();
            true
        }

        EditCommand::Copy => copy_selection_to_clipboard(dv, clipboard),
        EditCommand::Cut => cut_selection_to_clipboard(dv, clipboard),
        EditCommand::Paste | EditCommand::PastePlainText => paste_from_clipboard(dv, clipboard),
        // Commands that need external state (event_loop)
        // ── Save (handled at App level, not in execute_edit_command) ──
        EditCommand::Save | EditCommand::SaveAs => false,

        EditCommand::Escape => false,

        // ── Search (handled at App level) ──
        EditCommand::Find
        | EditCommand::FindReplace
        | EditCommand::FindNext
        | EditCommand::FindPrev => false,

        EditCommand::OpenFile | EditCommand::OpenFolder => false,
        EditCommand::NewTab | EditCommand::CloseTab | EditCommand::ReopenTab => false,
        EditCommand::NextTab | EditCommand::PrevTab => false,
        EditCommand::NavigateBack | EditCommand::NavigateForward => false,
        EditCommand::SwitchTab(_) => false,
    }
}

fn copy_selection_to_clipboard(document: &DocumentModel, clipboard: &mut dyn Clipboard) -> bool {
    let Some(bytes) = document.extract_selected_text() else {
        return false;
    };
    if bytes.is_empty() {
        return false;
    }
    clipboard.write_text(&String::from_utf8_lossy(&bytes))
}

fn cut_selection_to_clipboard(document: &mut DocumentModel, clipboard: &mut dyn Clipboard) -> bool {
    if !copy_selection_to_clipboard(document, clipboard) {
        return false;
    }
    document.delete_selection()
}

fn paste_from_clipboard(document: &mut DocumentModel, clipboard: &mut dyn Clipboard) -> bool {
    let Some(text) = clipboard.read_text() else {
        return false;
    };
    let normalized = crate::document_view::normalize_paste_text(text.as_bytes());
    if normalized.is_empty() {
        return false;
    }
    document.delete_selection();
    document.insert_at_cursor(&normalized);
    true
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use crate::line_index::LineIndex;
    use core::types::ByteIndex;

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
        document: &mut DocumentView,
        clipboard: &mut dyn Clipboard,
    ) -> EditOutcome {
        let mut presentation = document.take_presentation();
        let page_step_rows = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
        let outcome = execute_edit_command_v2_with_presentation_and_clipboard(
            command,
            document,
            &[],
            &mut presentation.cursor_render_state,
            page_step_rows,
            clipboard,
        );
        document.restore_presentation(presentation);
        outcome
    }

    const TEST_LINE_HEIGHT: f32 = 24.27;

    fn make_dv(content: &str) -> DocumentView {
        let lines: Vec<String> = if content.is_empty() {
            vec![]
        } else {
            content.split('\n').map(String::from).collect()
        };
        DocumentView::new(lines, 20, 10.0)
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
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "ello");
        assert_eq!(dv.cursor().offset, ByteIndex(0));
    }

    #[test]
    fn delete_forward_deletes_grapheme() {
        let mut dv = make_dv("hello");
        assert!(execute_edit_command(&EditCommand::DeleteForward, &mut dv, &[]));
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "ello");
    }

    #[test]
    fn insert_char() {
        let mut dv = make_dv("hello");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::InsertChar("X".into()), &mut dv, &[]));
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "helloX");
    }

    #[test]
    fn insert_newline() {
        let mut dv = make_dv("helloworld");
        dv.cursor_move_to_offset(5);
        assert!(execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]));
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
        assert_eq!(vis[0], "hello");
        assert_eq!(vis[1], "world");
    }

    // ── Undo/Redo ────────────────────────────────────────────────────

    #[test]
    fn undo_redo() {
        let mut dv = make_dv("hello");
        execute_edit_command(&EditCommand::InsertChar("X".into()), &mut dv, &[]);
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "Xhello");

        execute_edit_command(&EditCommand::Undo, &mut dv, &[]);
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "hello");

        execute_edit_command(&EditCommand::Redo, &mut dv, &[]);
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "Xhello");
    }

    // ── PageUp/PageDown ──────────────────────────────────────────────

    /// Helper: create 100-line doc with visible_rows=10.
    /// Each line "line N" (N=0..99).  Lines 0-9 are 6 bytes each → 7 bytes/line with \n.
    fn make_100_line_dv() -> DocumentView {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        DocumentView::new(lines, 10, 10.0)
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
        let mut dv = DocumentView::new(lines, 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["line0".to_string(), "line1".to_string()], 10, 10.0);
        // cursor at start of line1 (offset 6)
        dv.cursor_move_to_offset(6);
        assert!(execute_edit_command(&EditCommand::MoveUp, &mut dv, &[]));
        // should be on line0 now
        assert!(dv.cursor().offset < ByteIndex(6));
    }

    #[test]
    fn move_down_basic() {
        let mut dv = DocumentView::new(vec!["line0".to_string(), "line1".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghij".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghij".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["".to_string(), "hello".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcde".to_string(); 20], 4, 10.0);
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
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "hello    ");
    }

    // ── Commands that don't modify DocumentView ──────────────────────

    #[test]
    fn escape_returns_false() {
        let mut dv = make_dv("hello");
        assert!(!execute_edit_command(&EditCommand::Escape, &mut dv, &[]));
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
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT), vec!["plain"]);
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
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "hello");
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
            dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0],
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
        let mut dv = DocumentView::new(vec!["hi".to_string()], 10, 10.0);
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
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
        assert_eq!(vis[0], "xy");
    }

    #[test]
    fn insert_char_cjk() {
        let mut dv = make_dv("");
        assert!(execute_edit_command(&EditCommand::InsertChar("世".into()), &mut dv, &[]));
        assert!(execute_edit_command(&EditCommand::InsertChar("界".into()), &mut dv, &[]));
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)[0], "世界");
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
        let mut dv = DocumentView::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["hello world".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["    hello world".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["abcde".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["hello".to_string(), "world".to_string()], 20, 20.0);
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
        let mut dv = DocumentView::new(vec!["abcdefghijklmnop".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["关于广东省".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["关于广东省珠海市".to_string()], 10, 10.0);
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
        let mut dv = DocumentView::new(vec!["关于广东省珠海市".to_string()], 10, 10.0);
        let ac = vec![make_cjk_entry(0, 0, 4, 18.0), make_cjk_entry(0, 12, 4, 18.0)];
        dv.cursor_move_to_offset(12);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        // Half-open: byte 12 → VL1 → End → VL1's end at 24
        assert_eq!(dv.cursor().offset, ByteIndex(24), "End in CJK VL1 should go to VL1 end (24)");
    }

    #[test]
    fn end_cjk_mid_char_byte_offset() {
        let mut dv = DocumentView::new(vec!["关于广东省".to_string()], 10, 10.0);
        let ac = vec![make_cjk_entry(0, 0, 3, 20.0), make_cjk_entry(0, 9, 2, 20.0)];
        dv.cursor_move_to_offset(4);
        execute_edit_command(&EditCommand::MoveToLineEnd, &mut dv, &ac);
        assert_eq!(dv.cursor().offset, ByteIndex(9), "End should go to vl_end (9)");
    }

    #[test]
    fn document_view_cursor_render_helpers_manage_click_hint_and_page_step() {
        let mut dv = make_dv("hello\nworld");
        dv.presentation.display.viewport.visible_rows = 5;

        dv.set_click_hint(core::types::UniCharOffset(3), 1);
        assert_eq!(dv.click_hint(), Some((core::types::UniCharOffset(3), 1)));

        dv.clear_click_hint();
        assert!(dv.click_hint().is_none());
        assert_eq!(dv.page_step_rows(), 4);
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
        let mut dv = DocumentView::new(
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
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
        assert_eq!(vis.len(), 8);
        assert_eq!(vis[5], " ");
        assert_eq!(vis[6], "");
        assert!(vis[7].starts_with(" /Users/"), "slash eaten: {:?}", vis[7]);

        // --- cursor at start of space-line (before the space) ---
        let mut dv = DocumentView::new(
            ["line1", " ", " /Users/foo"].iter().map(|s| s.to_string()).collect(),
            60,
            10.0,
        );
        dv.cursor_move_to_offset(6); // after "line1\n"
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
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
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
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
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
        assert_eq!(vis, &["a", "", "", "b"]);

        // Triple Enter at end of a single-space line
        let mut dv =
            DocumentView::new(["x", " ", "y"].iter().map(|s| s.to_string()).collect(), 10, 10.0);
        dv.cursor_move_to_offset(3); // after "x\n "
        for _ in 0..3 {
            execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        }
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
        assert_eq!(vis, &["x", " ", "", "", "", "y"]);

        // Enter at offset 0
        let mut dv = make_dv(
            "first
second",
        );
        dv.cursor_move_to_offset(0);
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT), &["", "first", "second"]);
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
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT), &["helloworld"]);

        // Backspace across consecutive empty lines
        let mut dv =
            DocumentView::new(["a", "", "", "b"].iter().map(|s| s.to_string()).collect(), 10, 10.0);
        dv.cursor_move_to_offset(4); // "a\n\n\n|b"
        execute_edit_command(&EditCommand::Backspace, &mut dv, &[]);
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT), &["a", "", "b"]);
    }

    /// Full rebuild (rebuild_from) preserves consecutive empty lines.
    #[test]
    fn rebuild_preserves_empty_lines() {
        let dv = DocumentView::new(
            ["a", "", "", "", "b"].iter().map(|s| s.to_string()).collect(),
            10,
            10.0,
        );
        let rebuilt = LineIndex::rebuild_from(&dv.tb);
        assert_eq!(rebuilt.line_count(), 5);
        assert_eq!(dv.visible_lines_with_line_height(TEST_LINE_HEIGHT), &["a", "", "", "", "b"]);
    }

    /// Enter at end of a non-empty line that has an empty line right after.
    /// Must insert a new line, not just move cursor to the next line.
    #[test]
    fn enter_at_end_of_line_before_empty_line() {
        // Simulates: line 8 "/Users/...", line 9 "", line 10 " "
        let mut dv = DocumentView::new(
            vec!["/Users/foo/bar".to_string(), "".to_string(), " ".to_string()],
            10,
            10.0,
        );
        // Cursor at end of line 0: after "/Users/foo/bar\n"
        dv.cursor_move_to_offset(15); // "/Users/foo/bar\n" = 15 bytes
        let old = dv.line_count();
        execute_edit_command(&EditCommand::InsertNewline, &mut dv, &[]);
        let vis = dv.visible_lines_with_line_height(TEST_LINE_HEIGHT);
        // Should now have 4 lines: "/Users/foo/bar", "", "", " "
        assert_eq!(dv.line_count(), old + 1, "line count should increase by 1");
        assert_eq!(vis[0], "/Users/foo/bar");
        assert_eq!(vis[1], "");
        assert_eq!(vis[2], "");
        assert_eq!(vis[3], " ");
    }
}
