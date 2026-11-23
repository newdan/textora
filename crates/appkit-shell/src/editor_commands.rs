//! Command dispatch for editor operations.
//!
//! Resolves source-editor commands against a document and its visual rows.

use crate::SystemClipboard;
use crate::cursor_motion::find_visual_line_index;
use appkit_core::document::DocumentModel;
use appkit_core::edit_command::EditCommand;
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

use crate::edit_transaction::EditOutcome;

pub fn execute_edit_command_with_system_clipboard(
    cmd: &EditCommand,
    dv: &mut DocumentModel,
    advance_cache: &[AdvanceCacheEntry],
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
    page_step_rows: usize,
) -> EditOutcome {
    let mut clipboard = SystemClipboard;
    execute_edit_command_with_clipboard(
        cmd,
        dv,
        advance_cache,
        cursor_render_state,
        page_step_rows,
        &mut clipboard,
    )
}

pub fn execute_edit_command_with_clipboard(
    cmd: &EditCommand,
    dv: &mut DocumentModel,
    advance_cache: &[AdvanceCacheEntry],
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
    page_step_rows: usize,
    clipboard: &mut dyn Clipboard,
) -> EditOutcome {
    let old_line_count = dv.line_count();

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
                if was_home || dv.cursor().offset.to_usize() == indent_offset {
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
            dv.tb.cursor_move_to_visual(textora_core::types::VisualPoint {
                column: vpos.column,
                row: new_row,
            });
            dv.sync_cursor();
            true
        }
        EditCommand::PageDown => {
            let vpos = dv.tb.cursor_visual_pos();
            let new_row = (vpos.row as isize + page_step_rows as isize).max(0) as usize;
            dv.tb.cursor_move_to_visual(textora_core::types::VisualPoint {
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
                if was_home || dv.cursor().offset.to_usize() == indent_offset {
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
    let normalized = crate::normalize_paste_text(text.as_bytes());
    if normalized.is_empty() {
        return false;
    }
    document.delete_selection();
    document.insert_at_cursor(&normalized);
    true
}

pub fn edit_requires_reshape(cmd: &EditCommand, outcome: &EditOutcome) -> bool {
    if !outcome.executed {
        return false;
    }

    if outcome.new_line_count != outcome.old_line_count {
        return true;
    }

    if matches!(
        cmd,
        EditCommand::Paste
            | EditCommand::PastePlainText
            | EditCommand::Undo
            | EditCommand::Redo
            | EditCommand::Cut
    ) {
        return true;
    }

    outcome.dirty_lines.as_ref().is_some_and(|range| range.len() > 1)
}
