//! Cursor movement helpers for visual-line-aware up/down navigation.
//!
//! Extracts the complex `move_cursor_visual` logic from App,
//! using a `CursorContext` struct to bundle all required state.

use appkit_core::document::DocumentModel;
use ui::render_geom::AdvanceCacheEntry;
use ui::viewport::DisplayRow;

use core::types::{ByteIndex, UniCharOffset};
use std::time::Instant;

/// Contains transient cursor rendering and caching state that is updated during layout and drawing.
pub struct CursorRenderState {
    pub cursor_pixel_x: f32,
    pub cursor_visual_line: Option<usize>,
    pub cursor_visual_line_in_doc: usize,
    pub cursor_blink_instant: Instant,
    pub sticky_x: f32,
    pub sticky_x_dirty: bool,
    pub last_cursor_offset: ByteIndex,
    /// (unichar_offset, vis_line) from last mouse click — bypasses ambiguous VL boundary disambiguation.
    /// Cleared on keyboard movement and viewport scroll.
    pub click_hint: Option<(UniCharOffset, usize)>,
}

impl CursorRenderState {
    pub fn new() -> Self {
        Self {
            cursor_pixel_x: 0.0,
            cursor_visual_line: None,
            cursor_visual_line_in_doc: 0,
            cursor_blink_instant: Instant::now(),
            sticky_x: 0.0,
            sticky_x_dirty: false,
            last_cursor_offset: ByteIndex::ZERO,
            click_hint: None,
        }
    }
}

impl Default for CursorRenderState {
    fn default() -> Self {
        Self::new()
    }
}

pub use crate::frame_cache::LineCache;

/// Bundles all state needed by `move_cursor_visual`.
pub struct CursorContext<'a> {
    pub cursor_visual_line: Option<usize>,
    pub advance_cache: &'a [AdvanceCacheEntry],
    pub first_line: &'a LineCache,
    pub last_line: &'a LineCache,
    pub display_map: &'a crate::display_line_map::DisplayLineMap,
    pub first_visible_row: DisplayRow,
    pub scroll_top: f64,
    pub sticky_x: f32,
    pub visible_rows: usize,
    pub dpi_scale: f32,
}

/// Result of a cursor movement attempt.
pub(crate) enum CursorMoveResult {
    /// Cursor was moved to this byte offset.
    Moved(ByteIndex),
    /// Cursor could not be moved (already at boundary).
    NotMoved,
}

/// Find byte offset on a visual line closest to sticky_x.
///
/// `vl_start`/`vl_end`: cluster range indices.
/// `clusters`: `(byte_start, byte_end, advance)` per cluster.
/// `line_doc_offset`: byte offset of the doc line in the document.
/// 在可视行边界列表中查找 offset 所属的可视行索引。
///
/// `bounds`: &[(byte_start, byte_end)]，每个可视行的字节范围（相对偏移）。
/// `offset`: 光标在该文档行内的字节偏移。
/// 在可视行边界列表中查找 offset 所属的可视行索引。
///
/// `bounds`: &[(byte_start, byte_end)]，每个可视行的字节范围（相对偏移）。
/// `offset`: 光标在该文档行内的字节偏移。
///
/// 规则：非最后可视行使用 [start, end)，最后可视行使用 [start, end]。
/// 这与 selection highlight（advance_cache 绝对范围不含上界）保持一致。
/// 注意：折行边界字节（如 byte=5 既是 VL0 末也是 VL1 首）仍有归属歧义——
/// 鼠标点击路径应通过 click_hint 绕过此函数；此函数用于键盘移动等 fallback。
pub fn find_visual_line_index(bounds: &[(usize, usize)], offset: usize) -> usize {
    let len = bounds.len();
    for (i, &(start, end)) in bounds.iter().enumerate() {
        let matches = if i + 1 < len { offset < end } else { offset <= end };
        if offset >= start && matches {
            return i;
        }
    }
    // fallback: 最后一行
    len.saturating_sub(1)
}

fn find_closest_offset(
    clusters: &[(usize, usize, f32)],
    vl_start: usize,
    vl_end: usize,
    sticky_x: f32,
    dpi_scale: f32,
    line_doc_offset: usize,
) -> ByteIndex {
    let mut best_rel = 0usize;
    let mut best_dist = f32::MAX;
    let mut cum_x = 32.0 * dpi_scale;

    for ci in vl_start..vl_end {
        if ci < clusters.len() {
            let (bs, _be, adv) = clusters[ci];
            let dist = (cum_x - sticky_x).abs();
            if dist < best_dist {
                best_dist = dist;
                best_rel = bs;
            }
            cum_x += adv;
        }
    }

    // Also check end-of-line position
    let end_dist = (cum_x - sticky_x).abs();
    if end_dist < best_dist && vl_end > 0 && vl_end <= clusters.len() {
        best_rel = clusters[vl_end - 1].1; // byte_range.end of last cluster
    }

    ByteIndex(line_doc_offset + best_rel)
}

/// 4a: Move within visible advance_cache.
/// 4b: Move up past visible area.
fn move_up_past_visible(ctx: &CursorContext, dv: &DocumentModel) -> CursorMoveResult {
    let first_doc_line = ctx.advance_cache[0].doc_line;
    let scroll_offset = {
        let first_vl_of_line = ctx.display_map.doc_to_display(first_doc_line);
        ctx.first_visible_row.saturating_sub(first_vl_of_line as u32)
    };
    let scroll_line = {
        let sl = ctx.display_map.display_to_doc(ctx.scroll_top.floor() as usize);
        sl.min(ctx.display_map.line_count().saturating_sub(1))
    };

    if first_doc_line == scroll_line
        && scroll_offset > DisplayRow::ZERO
        && !ctx.first_line.visual_lines.is_empty()
    {
        // On first visible doc line with visual offset — scroll up one visual line
        let new_offset = scroll_offset - 1;
        if new_offset.as_usize() < ctx.first_line.visual_lines.len() {
            let (vl_start, vl_end, _) = ctx.first_line.visual_lines[new_offset.as_usize()];
            let target = find_closest_offset(
                &ctx.first_line.clusters,
                vl_start,
                vl_end,
                ctx.sticky_x,
                ctx.dpi_scale,
                ctx.first_line.doc_offset,
            );
            CursorMoveResult::Moved(target)
        } else {
            CursorMoveResult::NotMoved
        }
    } else if first_doc_line > 0 {
        // Move cursor to end of previous doc line
        let prev_line = first_doc_line - 1;
        let line_start = dv.line_byte_offset(prev_line).unwrap_or(0);
        let line_len = dv.line_byte_length(prev_line).unwrap_or(0);
        let end_offset = if line_len > 0 && line_start + line_len > line_start {
            line_start + line_len - 1
        } else {
            line_start
        };
        CursorMoveResult::Moved(ByteIndex(end_offset))
    } else {
        CursorMoveResult::NotMoved
    }
}

/// 4c: Move down past visible area.
fn move_down_past_visible(ctx: &CursorContext, dv: &DocumentModel) -> CursorMoveResult {
    let Some(last_entry) = ctx.advance_cache.last() else {
        return CursorMoveResult::NotMoved;
    };
    let last_doc_line = last_entry.doc_line;
    let last_clusters = &last_entry.clusters;
    let next_byte_in_line =
        last_entry.vl_byte_start + last_clusters.last().map(|c| c.0).unwrap_or(0);

    let line_start = dv.line_byte_offset(last_doc_line).unwrap_or(0);
    let line_len = dv.line_byte_length(last_doc_line).unwrap_or(0);
    let total_lines = dv.line_count();

    let target = line_start + next_byte_in_line;
    let line_end = line_start + line_len;

    let scroll_line = {
        let sl = ctx.display_map.display_to_doc(ctx.scroll_top.floor() as usize);
        sl.min(ctx.display_map.line_count().saturating_sub(1))
    };
    let scroll_vl_count =
        ctx.display_map.get_entry(scroll_line).map(|e| e.visual_line_count).unwrap_or(1) as usize;

    let is_long_first_line = last_doc_line == scroll_line && scroll_vl_count > ctx.visible_rows;

    if is_long_first_line && target < line_end {
        // Long first line: use cached visual lines to find sticky_x position on next visual line
        let ll = &ctx.last_line.visual_lines;
        let clusters = &ctx.last_line.clusters;
        let current_vl = ll
            .iter()
            .position(|&(s, e, _)| {
                s < clusters.len()
                    && e <= clusters.len()
                    && clusters[s].0 <= (next_byte_in_line - line_start)
                    && (next_byte_in_line - line_start) <= clusters[e.saturating_sub(1)].1
            })
            .unwrap_or(0);
        let next_vl = current_vl + 1;
        if next_vl < ll.len() {
            let (vl_start, vl_end, _) = ll[next_vl];
            let best_target = find_closest_offset(
                clusters,
                vl_start,
                vl_end,
                ctx.sticky_x,
                ctx.dpi_scale,
                line_start,
            );
            CursorMoveResult::Moved(best_target)
        } else {
            // Next visual line not in cache — fallback to target byte
            CursorMoveResult::Moved(ByteIndex(target))
        }
    } else if target < line_end {
        // Next visual line in same doc line (word wrap)
        CursorMoveResult::Moved(ByteIndex(target))
    } else if last_doc_line + 1 < total_lines {
        // At end of doc line — move to start of next non-empty doc line
        let target_line = last_doc_line + 1;
        if target_line < total_lines
            && let Some(offset) = dv.line_byte_offset(target_line)
        {
            return CursorMoveResult::Moved(ByteIndex(offset));
        }
        CursorMoveResult::NotMoved
    } else {
        CursorMoveResult::NotMoved
    }
}

/// Move cursor to an adjacent visual line, preserving horizontal position (sticky_x).
///
/// Returns the new byte offset if the cursor was moved.
pub fn move_cursor_visual(
    delta: isize,
    ctx: CursorContext,
    dv: &DocumentModel,
) -> Option<ByteIndex> {
    let Some(current_vis) = ctx.cursor_visual_line else { return None };
    let current_vis = current_vis as isize;
    let target_vis = current_vis + delta;

    let result = if target_vis >= 0 && (target_vis as usize) < ctx.advance_cache.len() {
        // 4a: Target is within visible advance_cache
        let entry = &ctx.advance_cache[target_vis as usize];
        let doc_line = entry.doc_line;
        let vl_byte_start = entry.vl_byte_start;
        let vl_clusters = &entry.clusters;

        let mut best_offset = vl_byte_start;
        let mut best_dist = f32::MAX;
        let mut prev_end = vl_byte_start;
        for &(cluster_end, cluster_x, _) in vl_clusters {
            let dist = (cluster_x - ctx.sticky_x).abs();
            if dist < best_dist {
                best_dist = dist;
                best_offset = prev_end;
            }
            prev_end = vl_byte_start + cluster_end;
        }
        let last_x = vl_clusters.last().map(|&(_, x, _)| x).unwrap_or(0.0);
        if (last_x - ctx.sticky_x).abs() < best_dist {
            best_offset = prev_end;
        }

        let line_start = dv.line_byte_offset(doc_line);
        if let Some(line_start) = line_start {
            CursorMoveResult::Moved(ByteIndex(line_start + best_offset))
        } else {
            CursorMoveResult::NotMoved
        }
    } else if target_vis < 0 && !ctx.advance_cache.is_empty() {
        move_up_past_visible(&ctx, dv)
    } else if target_vis as usize >= ctx.advance_cache.len() && !ctx.advance_cache.is_empty() {
        move_down_past_visible(&ctx, dv)
    } else {
        CursorMoveResult::NotMoved
    };

    match result {
        CursorMoveResult::Moved(offset) => Some(offset),
        CursorMoveResult::NotMoved => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_line_map::DisplayLineMap;
    use appkit_core::document::DocumentModel;
    use core::buffer::TextBuffer;
    use ui::render_geom::AdvanceCacheEntry;

    fn make_model(content: &str) -> DocumentModel {
        let mut text_buffer = TextBuffer::new(false)
            .expect("TextBuffer creation should not require presentation state");
        text_buffer.write_raw(content.as_bytes());
        DocumentModel::new(text_buffer)
    }

    fn make_line_cache(
        clusters: Vec<(usize, usize, f32)>,
        visual_lines: Vec<(usize, usize, f32)>,
    ) -> LineCache {
        LineCache { clusters, visual_lines, doc_offset: 0 }
    }

    #[test]
    fn default_cursor_render_state_matches_new_state() {
        let before_default = Instant::now();
        let default_state = CursorRenderState::default();
        let after_default = Instant::now();
        let before_new = Instant::now();
        let new_state = CursorRenderState::new();
        let after_new = Instant::now();

        assert_eq!(default_state.cursor_pixel_x, new_state.cursor_pixel_x);
        assert_eq!(default_state.cursor_visual_line, new_state.cursor_visual_line);
        assert_eq!(default_state.cursor_visual_line_in_doc, new_state.cursor_visual_line_in_doc);
        assert!(default_state.cursor_blink_instant >= before_default);
        assert!(default_state.cursor_blink_instant <= after_default);
        assert!(new_state.cursor_blink_instant >= before_new);
        assert!(new_state.cursor_blink_instant <= after_new);
        assert_eq!(default_state.sticky_x, new_state.sticky_x);
        assert_eq!(default_state.sticky_x_dirty, new_state.sticky_x_dirty);
        assert_eq!(default_state.last_cursor_offset, new_state.last_cursor_offset);
        assert_eq!(default_state.click_hint, new_state.click_hint);
    }

    #[test]
    fn move_cursor_visual_vl_local_offset() {
        // clusters 改为 vl-local 后，4a 路径中 best_offset/prev_end 应
        // 返回 line-local 偏移。用两 cluster 的 VL 可清晰复现：
        //   VL1: vl_byte_start=5, clusters=[(3, 110, 0), (5, 130, 0)]
        //   对应绝对 bytes [5,8) x=[100,110] 和 [8,10) x=[110,130]
        // sticky_x=125（第二 cluster 右半）→ 应选 byte 8（第二 cluster 起点）
        let advance_cache = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(3usize, 110.0f32, 0), (5usize, 130.0f32, 0)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 0,
                // vl-local: cluster 0 覆盖 3 bytes [5,8), cluster 1 覆盖 2 bytes [8,10)
                clusters: vec![(3usize, 110.0f32, 0), (5usize, 130.0f32, 0)],
            },
        ];

        let model = make_model("abcdefghij");
        let sticky_x = 125.0; // 第二 cluster 右半

        let display_map = DisplayLineMap::new();
        let first_line = make_line_cache(vec![], vec![]);
        let last_line = make_line_cache(vec![], vec![]);

        let ctx = CursorContext {
            cursor_visual_line: Some(0),
            advance_cache: &advance_cache,
            first_line: &first_line,
            last_line: &last_line,
            display_map: &display_map,
            first_visible_row: DisplayRow::ZERO,
            scroll_top: 0.0,
            sticky_x,
            visible_rows: 10,
            dpi_scale: 1.0,
        };

        // delta=1: VL0 → VL1 (4a path)
        let result = move_cursor_visual(1, ctx, &model);
        // VL1 第二 cluster 右半 → best_offset 应为 8 (line_local: 5 + 3)
        assert_eq!(
            result,
            Some(ByteIndex(8)),
            "4a 多 cluster: sticky_x=125 应返回 line-local offset=8 (5+3), 非 vl-local 的 3"
        );
    }

    #[test]
    fn move_down_past_visible_does_not_skip_empty_lines() {
        // Regression: move_down_past_visible skipped empty lines, causing
        // cursor to get stuck when next doc line was empty.
        // Fix: move directly to next doc line without skipping empty ones.
        let content = "hello\n\nworld";
        let model = make_model(content);

        let advance_cache = vec![AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 0,
            vl_grapheme_start: 0,
            clusters: vec![(5usize, 50.0f32, 0)],
        }];
        let display_map = DisplayLineMap::new();

        let first_line = make_line_cache(vec![], vec![]);
        let last_line = make_line_cache(vec![], vec![]);
        let ctx = CursorContext {
            cursor_visual_line: Some(0),
            advance_cache: &advance_cache,
            first_line: &first_line,
            last_line: &last_line,
            display_map: &display_map,
            first_visible_row: DisplayRow::ZERO,
            scroll_top: 0.0,
            sticky_x: 100.0,
            visible_rows: 1,
            dpi_scale: 1.0,
        };

        // delta=1 → target_vis=1 >= advance_cache.len()=1 → move_down_past_visible
        let result = move_cursor_visual(1, ctx, &model);
        let line1_offset = model.line_byte_offset(1);
        assert!(result.is_some(), "should move to empty line 1, not return None");
        assert_eq!(
            result,
            line1_offset.map(ByteIndex),
            "should move to start of empty line 1 (offset={:?}), not skip it",
            line1_offset
        );
    }

    #[test]
    fn move_down_past_visible_empty_line_at_eof_moves_to_it() {
        // File ending with an empty line: cursor should still be able to
        // move to it, not get stuck at the last non-empty line.
        // Use explicit lines so trailing empty line is preserved
        // (str::lines() drops trailing empty strings).
        let model = make_model("hello\n");

        let advance_cache = vec![AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 0,
            vl_grapheme_start: 0,
            clusters: vec![(5usize, 50.0f32, 0)],
        }];
        let display_map = DisplayLineMap::new();
        let first_line = make_line_cache(vec![], vec![]);
        let last_line = make_line_cache(vec![], vec![]);
        let ctx = CursorContext {
            cursor_visual_line: Some(0),
            advance_cache: &advance_cache,
            first_line: &first_line,
            last_line: &last_line,
            display_map: &display_map,
            first_visible_row: DisplayRow::ZERO,
            scroll_top: 0.0,
            sticky_x: 100.0,
            visible_rows: 1,
            dpi_scale: 1.0,
        };

        let result = move_cursor_visual(1, ctx, &model);
        let eof_line = model.line_count() - 1;
        let eof_offset = model.line_byte_offset(eof_line);
        assert!(result.is_some(), "should move to trailing empty line");
        assert_eq!(
            result,
            eof_offset.map(ByteIndex),
            "should move to trailing empty line, not get stuck"
        );
    }

    #[test]

    fn find_visual_line_boundary_uses_half_open() {
        // Half-open intervals: non-last VL uses [start, end).
        // byte 5 is the start of VL1 (wrapped portion), not the end of VL0.
        // This matches selection highlight behavior where VL0 abs range is [0,5).
        let bounds = vec![(0usize, 5usize), (5usize, 10usize)];
        assert_eq!(
            find_visual_line_index(&bounds, 5),
            1,
            "half-open: byte 5 starts VL1, not end of VL0"
        );
        assert_eq!(find_visual_line_index(&bounds, 0), 0);
        assert_eq!(find_visual_line_index(&bounds, 4), 0);
        assert_eq!(find_visual_line_index(&bounds, 6), 1);
        assert_eq!(find_visual_line_index(&bounds, 9), 1);
        assert_eq!(find_visual_line_index(&bounds, 10), 1, "last VL's end is inclusive");
    }
}
