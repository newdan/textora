 Findings summary (deduplicated across 3 agents)

  Tier A — quick wins (low risk, ~20 lines total)
  1. settings.rs:82-86 — status_bar_height() getter duplicates the public field. Delete one.
  2. viewport.rs:174-186 — set_total_visual_lines / total_visual_lines field is set but never computed in production; total_visual_lines() always
  falls through. Either delete or wire up.
  3. viewport.rs — reset_visual_state block (scroll_visual_offset = 0; scroll_line_visual_count = 0; clamp()) duplicated in set_total_lines,
  scroll_to, scroll_up, scroll_down, resize. Extract once.
  4. app.rs:818-824 — first_line_visual_lines.clone() / last_line_visual_lines.clone() cloned per visible line per frame but only the last write is
  read. Hoist to a single assignment after the loop using a saved index.
  5. app.rs — narrative comments (// 4a:, // 4b:, // 3a:, // B1: …) reference plan sections, not intent. Delete.
  6. scroll_bench.rs:30 — stray double blank line.
  
  Tier B — code-reuse fixes (medium risk, behavior preserved, fixes correctness too)
  7. document_view.rs:122-142 word_select_at — allocates the entire document into Vec<u8> on every double-click. TextBuffer already implements
  ReadableDocument; call core::buffer::word_select(&self.tb, offset) directly.
  8. document_view.rs:380-432 extract_selected_text + copy/cut — duplicates TextBuffer::extract_user_selection(false/true) which is already
  chunk-aware. Replace.
  9. document_view.rs:454-498 extend_selection_* family (10 fns) — reimplements TextBuffer::selection_update_delta/offset/logical. Currently
  bypasses tb's selection state, so the new history_preserves_selection_state test path is actually fragile. Replace.
  10. document_view.rs:608-611 delete_selection — delete(Grapheme, 1) is a workaround relying on undocumented behavior. Use
  extract_user_selection(true) (atomic delete with history).
  11. document_view.rs:850-869 normalize_paste_text — first 3 lines duplicate core::file::strip_bom. Reuse it (need to make pub).
  12. app.rs:340-491 — three near-identical "find closest cluster to sticky_x" loops in move_cursor_visual (4a / 4b / 4c). Extract one helper.

  Tier C — bigger efficiency wins (higher impact, more delicate)
  13. document_view.rs:359-370 cursor_line() is O(N) linear scan, called ≥19×/frame. Replace with
  line_offsets.partition_point(...).saturating_sub(1) → O(log N).
  14. document_view.rs:686-690 sync_after_edit_incremental walks all N line offsets for every keystroke. Use partition_point to find the start,
  iterate suffix only.
  15. document_view.rs:728-804 rescan_lines_from rescans to EOF on every newline-containing edit (e.g. pressing Enter at top of 100k-line file).
  Stop early once newline-count delta is satisfied; batch-shift the suffix.
  16. app.rs:1051 / 1362-1374 — render runs full shape pass every frame; about_to_wait always requests redraw → permanent 60Hz idle loop. Add an
  early-out when !needs_redraw && !blink_changed.
  17. app.rs:761 shape cache key (offset << 32) | length — every keystroke shifts offsets and invalidates every line below. Re-key on (line_idx, 
  extract_user_selection(true) (atomic delete with history).
  11. document_view.rs:850-869 normalize_paste_text — first 3 lines duplicate core::file::strip_bom. Reuse it (need to make pub).
  12. app.rs:340-491 — three near-identical "find closest cluster to sticky_x" loops in move_cursor_visual (4a / 4b / 4c). Extract one helper.

  Tier C — bigger efficiency wins (higher impact, more delicate)
  13. document_view.rs:359-370 cursor_line() is O(N) linear scan, called ≥19×/frame. Replace with
  line_offsets.partition_point(...).saturating_sub(1) → O(log N).
  14. document_view.rs:686-690 sync_after_edit_incremental walks all N line offsets for every keystroke. Use partition_point to find the start,
  iterate suffix only.
  15. document_view.rs:728-804 rescan_lines_from rescans to EOF on every newline-containing edit (e.g. pressing Enter at top of 100k-line file).
  Stop early once newline-count delta is satisfied; batch-shift the suffix.
  16. app.rs:1051 / 1362-1374 — render runs full shape pass every frame; about_to_wait always requests redraw → permanent 60Hz idle loop. Add an
  early-out when !needs_redraw && !blink_changed.
  17. app.rs:761 shape cache key (offset << 32) | length — every keystroke shifts offsets and invalidates every line below. Re-key on (line_idx,
  content_hash) or content hash alone.

  Tier D — larger refactors (defer / discuss)
  18. app.rs:608-700 status_bar_text_vertices — ~120 lines reproducing the shape→atlas→vertex pipeline. Factor emit_text_run.
  19. app.rs:86-96 — six new App-level cache fields (first_line_visual_lines, etc.) are derivable from advance_cache. Restructure or move into a
  single struct.
  20. input.rs:75-119 — 16 hand-mapped (arrow × shift × alt × super) arms with identical structure. Reduce with a small movement-table.
  21. Viewport leaky abstraction — app.rs writes dv.viewport.scroll_visual_offset = X directly in several places instead of using viewport methods.

  Recommended approach

  Do Tier A + Tier B in this pass — they are mostly mechanical, fix real bugs (#9 selection-undo, #10 fragile delete contract), and remove ~200
  lines without changing the design. Each tier compiles independently.

  Hold Tier C for a separate pass — those are the high-impact perf fixes but they touch the line-index machinery and need test verification. Tier D
  are design-level changes that deserve their own discussion.
