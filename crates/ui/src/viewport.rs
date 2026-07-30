//! Viewport: tracks scroll position and visible line range.
//!
//! The viewport maps between document lines and screen rows,
//! handling scrolling, clamping, and resize events.
//!
//! # Coordinate Spaces
//!
//! - **DisplayRow**: virtual row after word-wrap. `scroll_top` is in this space.
//! - **DocLine**: document line number (before word-wrap). Use `WrapIndex::display_to_doc()` to convert from DisplayRow.
//!
//! The rendering loop iterates doc lines but produces DisplayRows.
//! Autoscroll and hit-testing operate entirely in DisplayRow space.

use std::fmt;
use std::ops::{Add, AddAssign, Range, Sub, SubAssign};

/// Trait abstracting line-mapping operations needed by Viewport.
/// Implemented by app-layer types (e.g. DisplayLineMap).
pub trait LineMap {
    fn map_line_count(&self) -> usize;
    fn map_total_rows(&self) -> usize;
    fn map_display_to_doc(&self, display_row: usize) -> usize;
    fn map_doc_to_display(&self, doc_line: usize) -> usize;
    /// O(1) 获取某个 doc_line 的 visual_line_count（折合多少 display_row）。
    fn visual_line_count(&self, doc_line: usize) -> u16;
}

// ── DisplayRow type ─────────────────────────────────────────────────

/// A virtual row number after word-wrap.
/// 0 = topmost visible row. Each document line may span 1..N DisplayRows.
#[derive(Clone, Copy, Default, Eq, Ord, PartialOrd, PartialEq, Hash)]
pub struct DisplayRow(pub u32);

impl DisplayRow {
    pub const ZERO: Self = DisplayRow(0);

    pub fn as_f64(self) -> f64 {
        self.0 as f64
    }
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Saturating subtraction: returns ZERO if rhs > self.
    pub fn saturating_sub(self, rhs: u32) -> Self {
        DisplayRow(self.0.saturating_sub(rhs))
    }

    /// Saturating addition: returns MAX on overflow.
    pub fn saturating_add(self, rhs: u32) -> Self {
        DisplayRow(self.0.saturating_add(rhs))
    }

    /// Checked addition: returns None on overflow.
    pub fn checked_add(self, rhs: u32) -> Option<Self> {
        self.0.checked_add(rhs).map(DisplayRow)
    }

    /// Checked subtraction: returns None on underflow.
    pub fn checked_sub(self, rhs: u32) -> Option<Self> {
        self.0.checked_sub(rhs).map(DisplayRow)
    }

    /// Next row (saturates at u32::MAX).
    pub fn next(self) -> Self {
        DisplayRow(self.0.saturating_add(1))
    }
}

impl fmt::Debug for DisplayRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DisplayRow({})", self.0)
    }
}

impl From<u32> for DisplayRow {
    fn from(v: u32) -> Self {
        DisplayRow(v)
    }
}

impl From<usize> for DisplayRow {
    /// Truncates to u32 — values > u32::MAX silently wrap.
    fn from(v: usize) -> Self {
        DisplayRow(v as u32)
    }
}

impl Add<u32> for DisplayRow {
    /// Panics in debug on overflow; wraps in release. Use `saturating_add` for safe bounds.
    type Output = Self;
    fn add(self, rhs: u32) -> Self {
        DisplayRow(self.0 + rhs)
    }
}

impl Sub<u32> for DisplayRow {
    /// Panics in debug on underflow; wraps in release. Use `saturating_sub` for safe bounds.
    type Output = Self;
    fn sub(self, rhs: u32) -> Self {
        DisplayRow(self.0 - rhs)
    }
}

impl AddAssign<u32> for DisplayRow {
    fn add_assign(&mut self, rhs: u32) {
        self.0 = self.0.saturating_add(rhs);
    }
}

impl SubAssign<u32> for DisplayRow {
    fn sub_assign(&mut self, rhs: u32) {
        self.0 = self.0.saturating_sub(rhs);
    }
}

// ── ScrollAnchor ────────────────────────────────────────────────────

/// 内容锚定的滚动位置：编辑后视口内容不漂移。
#[derive(Debug, Clone, Copy)]
pub struct ScrollAnchor {
    /// 锚定的文档行索引。
    pub doc_line: usize,
    /// 锚定行内的像素偏移（正 = 向下）。
    pub pixel_offset: f32,
}

impl ScrollAnchor {
    pub fn new(doc_line: usize, pixel_offset: f32) -> Self {
        Self { doc_line, pixel_offset: pixel_offset.max(0.0) }
    }

    pub fn top() -> Self {
        Self { doc_line: 0, pixel_offset: 0.0 }
    }
}

// ── Viewport ────────────────────────────────────────────────────────

/// Viewport state for a text document view.
#[derive(Debug, Clone)]
pub struct Viewport {
    /// Scroll position: first visible DisplayRow (fractional for sub-line pixel scrolling).
    /// Stage 5: 这是纯派生量，由 `derive_scroll_top` 从 `scroll_anchor` 计算。
    /// 用户路径不应直接写此字段。SOT 是 `scroll_anchor`。
    #[doc(hidden)]
    pub scroll_top: f64,
    /// Number of visible rows on screen (in DisplayRow units).
    pub visible_rows: usize,
    /// Exact height of the viewport in lines (can be fractional).
    pub viewport_height: f64,
    /// Content-anchored scroll position (for edit stability).
    pub scroll_anchor: ScrollAnchor,
}

impl Viewport {
    /// Create a new viewport.
    pub fn new(visible_rows: usize) -> Self {
        Self {
            scroll_top: 0.0,
            visible_rows: visible_rows.max(1),
            viewport_height: visible_rows.max(1) as f64,
            scroll_anchor: ScrollAnchor::top(),
        }
    }

    /// Handle a window resize.
    /// Caller should clamp scroll_top after calling this (via clamp_scroll_top or clamp_scroll_top_no_wrap).
    pub fn resize(&mut self, visible_rows: usize, viewport_height: f64) {
        self.visible_rows = visible_rows.max(1);
        self.viewport_height = viewport_height.max(1.0);
    }

    // ── DisplayRow-based API ──────────────────────────────────────

    /// First visible DisplayRow (integer part of scroll_top).
    pub fn first_visible_row(&self) -> DisplayRow {
        DisplayRow(self.scroll_top.floor() as u32)
    }

    /// Sub-line pixel offset (negative: top line shifted up by this amount).
    pub fn sub_line_pixel_offset(&self, line_height: f32) -> f32 {
        -(self.scroll_top.fract() as f32 * line_height)
    }

    /// Visible DisplayRow range: `[first_visible_row, first_visible_row + visible_rows)`.
    pub fn visible_display_range(&self) -> Range<DisplayRow> {
        let start = self.first_visible_row();
        let end = DisplayRow((self.scroll_top + self.visible_rows as f64).ceil() as u32);
        start..end
    }

    /// Visible document line range, computed exactly via WrapIndex.
    pub fn visible_doc_line_range(&self, map: &impl LineMap) -> Range<usize> {
        let total_lines = map.map_line_count();
        if total_lines == 0 {
            return 0..0;
        }
        let start = map.map_display_to_doc(self.scroll_top.floor() as usize);
        let end_display = (self.scroll_top + self.visible_rows as f64).ceil() as usize;
        // end_display >= 1 guaranteed (visible_rows >= 1, scroll_top >= 0).
        let end = map.map_display_to_doc(end_display.saturating_sub(1)) + 1;
        let end = end.min(total_lines);
        start..end
    }

    /// Approximate visible document line range (no WrapIndex needed).
    /// Treats each DisplayRow as ~1 doc line. Good enough for non-wrapping scenarios.
    pub fn visible_doc_line_range_approx(&self, total_lines: usize) -> Range<usize> {
        if total_lines == 0 {
            return 0..0;
        }
        let max_start = total_lines.saturating_sub(1);
        let start = (self.scroll_top.floor() as usize).min(max_start);
        let end = ((self.scroll_top + self.visible_rows as f64).ceil() as usize).min(total_lines);
        start..end
    }

    /// Scroll by delta DisplayRows. Positive = down, negative = up.
    /// Caller should clamp scroll_top after calling this.
    #[deprecated(note = "Stage 5: 使用 scroll_doc_lines 或 scroll_pixels 替代。")]
    pub fn scroll_by(&mut self, delta: f64) {
        self.scroll_top = (self.scroll_top + delta).max(0.0);
    }

    /// Scroll to a specific DisplayRow position (fractional).
    /// Caller should clamp scroll_top after calling this.
    #[deprecated(note = "Stage 5: 使用 scroll_doc_lines 替代。")]
    pub fn scroll_to_row(&mut self, row: f64) {
        self.scroll_top = row.max(0.0);
    }

    /// Clamp scroll_top so it doesn't exceed the content bottom, then sync anchor.
    #[deprecated(note = "Stage 5: 使用 clamp_anchor 替代。")]
    pub fn clamp_scroll_top(&mut self, map: &impl LineMap, _line_height: f32) {
        let total_visual = map.map_total_rows();
        let max_visual = (total_visual as f64 - self.viewport_height).max(0.0);
        if self.scroll_top > max_visual {
            self.scroll_top = max_visual;
        }
    }

    /// Clamp scroll_top without WrapIndex (fallback for initialization).
    pub fn clamp_scroll_top_no_wrap(&mut self, total_lines: usize) {
        let max_visual = (total_lines as f64 - self.viewport_height).max(0.0);
        if self.scroll_top > max_visual {
            self.scroll_top = max_visual;
        }
    }

    /// Whether the viewport is scrolled to the top.
    pub fn is_at_top(&self) -> bool {
        self.scroll_top < 1.0
    }

    // ── ScrollAnchor methods ─────────────────────────────────────

    /// 从 scroll_top 同步 scroll_anchor（通过 WrapIndex 映射）。
    #[deprecated(
        note = "Stage 5: anchor 是 SOT，不再需要从 scroll_top 反推。使用 scroll_doc_lines / scroll_pixels 直接设置 anchor。"
    )]
    pub fn sync_anchor_from_scroll(&mut self, map: &impl LineMap, line_height: f32) {
        let display_row = self.scroll_top.floor() as usize;
        let doc_line = map.map_display_to_doc(display_row);
        let first_row_of_line = map.map_doc_to_display(doc_line);
        // row_offset: 折行内第几行（0 = 该文档行的第一个显示行）
        let row_offset = display_row.saturating_sub(first_row_of_line);
        let sub_row = self.scroll_top.fract();
        // pixel_offset = (折行偏移 + 亚行像素) × line_height
        let total_offset = (row_offset as f32 + sub_row as f32) * line_height;
        self.scroll_anchor = ScrollAnchor::new(doc_line, total_offset);
    }

    /// 从 scroll_anchor 恢复 scroll_top（通过 WrapIndex 映射）。
    #[deprecated(note = "Stage 5: 使用 derive_scroll_top 替代。")]
    pub fn restore_scroll_from_anchor(&mut self, map: &impl LineMap, line_height: f32) {
        let display_row = map.map_doc_to_display(self.scroll_anchor.doc_line) as f64;
        let lh = line_height.max(1.0) as f64;
        self.scroll_top = display_row + self.scroll_anchor.pixel_offset as f64 / lh;
    }

    // ── Anchor-based API (Stage 5) ──────────────────────────────

    /// 可见的 doc_line 范围（直接从 anchor 计算）。
    ///
    /// 从 `anchor.doc_line` 开始向下遍历，累加每行的 visual line 像素高度，
    /// 直到填满 visible_rows。返回的 Range 是 doc_line 空间的 exclusive 上界。
    pub fn visible_doc_range_from_anchor(
        &self,
        map: &impl LineMap,
        line_height: f32,
    ) -> Range<usize> {
        let total_lines = map.map_line_count();
        if total_lines == 0 {
            return 0..0;
        }
        let start = self.scroll_anchor.doc_line.min(total_lines.saturating_sub(1));
        let viewport_pixels = self.visible_rows as f32 * line_height;
        // When pixel_offset > 0, the anchor line is partially scrolled out of view.
        // Only the remaining portion of the anchor line is visible in the viewport.
        let anchor_vl = map.visual_line_count(start).max(1) as f32 * line_height;
        let anchor_visible_height = if self.scroll_anchor.pixel_offset > 0.0 {
            (anchor_vl - self.scroll_anchor.pixel_offset).clamp(0.0, viewport_pixels)
        } else {
            anchor_vl.min(viewport_pixels)
        };
        let mut remaining = viewport_pixels - anchor_visible_height;
        let mut end = start + 1; // Start from the line after anchor
        // Use a small epsilon to avoid f32 accumulation errors near zero.
        const EPSILON: f32 = 0.01;
        while end < total_lines && remaining > EPSILON {
            let vl = map.visual_line_count(end).max(1);
            remaining -= vl as f32 * line_height;
            end += 1;
        }
        start..end.min(total_lines)
    }

    /// 按文档行步进滚动（替代 `scroll_by` display_row 方式）。
    ///
    /// 正 = 向下，负 = 向上。pixel_offset 归零。
    pub fn scroll_doc_lines(&mut self, delta: isize, map: &impl LineMap) {
        let total = map.map_line_count();
        if total == 0 {
            return;
        }
        let max_line = total.saturating_sub(1);
        let new_doc = self.scroll_anchor.doc_line.saturating_add_signed(delta).min(max_line);
        self.scroll_anchor = ScrollAnchor::new(new_doc, 0.0);
    }

    /// 按像素精确滚动（鼠标滚轮 PixelDelta 使用）。
    ///
    /// 在行内累积 pixel_offset，满一行时步进 doc_line。
    /// 使用迭代实现，避免大滚动时栈溢出。
    pub fn scroll_pixels(&mut self, dy: f32, map: &impl LineMap, line_height: f32) {
        let total = map.map_line_count();
        if total == 0 {
            return;
        }
        let max_line = total.saturating_sub(1);
        let mut remaining = dy;

        if remaining > 0.0 {
            // 向下滚动
            while remaining > 0.0 && self.scroll_anchor.doc_line < max_line {
                let current_vl =
                    map.visual_line_count(self.scroll_anchor.doc_line) as f32 * line_height;
                let space_in_line = current_vl - self.scroll_anchor.pixel_offset;
                if remaining < space_in_line {
                    self.scroll_anchor.pixel_offset += remaining;
                    remaining = 0.0;
                } else {
                    remaining -= space_in_line;
                    self.scroll_anchor.doc_line += 1;
                    self.scroll_anchor.pixel_offset = 0.0;
                }
            }
            // 在最后一行内继续累积 pixel_offset
            if remaining > 0.0 && self.scroll_anchor.doc_line >= max_line {
                let last_vl = map.visual_line_count(max_line) as f32 * line_height;
                self.scroll_anchor.pixel_offset =
                    (self.scroll_anchor.pixel_offset + remaining).min(last_vl);
            }
        } else if remaining < 0.0 {
            // 向上滚动
            while remaining < 0.0 && self.scroll_anchor.doc_line > 0 {
                let space_up = self.scroll_anchor.pixel_offset;
                if -remaining <= space_up {
                    self.scroll_anchor.pixel_offset += remaining;
                    remaining = 0.0;
                } else {
                    remaining += space_up;
                    self.scroll_anchor.doc_line -= 1;
                    let prev_vl =
                        map.visual_line_count(self.scroll_anchor.doc_line) as f32 * line_height;
                    self.scroll_anchor.pixel_offset = prev_vl;
                }
            }
            // 在第一行内继续累积（向 0 clamp）
            if remaining < 0.0 && self.scroll_anchor.doc_line == 0 {
                self.scroll_anchor.pixel_offset =
                    (self.scroll_anchor.pixel_offset + remaining).max(0.0);
            }
        }
    }

    /// 派生 scroll_top（仅供滚动条/外部系统使用）。
    ///
    /// 从 anchor.doc_line 计算 display_row，加上 pixel_offset 的行内偏移。
    pub fn derive_scroll_top(&mut self, map: &impl LineMap, line_height: f32) {
        let display_row = map.map_doc_to_display(self.scroll_anchor.doc_line) as f64;
        let lh = line_height.max(1.0) as f64;
        self.scroll_top = display_row + self.scroll_anchor.pixel_offset as f64 / lh;
    }

    /// Clamp anchor.doc_line 不超过文档末尾。
    pub fn clamp_anchor(&mut self, map: &impl LineMap, line_height: f32) {
        let total = map.map_line_count();
        if total == 0 {
            self.scroll_anchor = ScrollAnchor::top();
            return;
        }
        let max_line = total.saturating_sub(1);
        if self.scroll_anchor.doc_line > max_line {
            self.scroll_anchor.doc_line = max_line;
            self.scroll_anchor.pixel_offset = 0.0;
        }
        // 确保 pixel_offset 不超过当前行的高度
        let vl = map.visual_line_count(self.scroll_anchor.doc_line) as f32 * line_height;
        if self.scroll_anchor.pixel_offset > vl {
            self.scroll_anchor.pixel_offset = vl;
        }

        // DisplayRow-space clamp: prevent scrolling beyond content
        // (viewport_height rows of blank space below last line).
        let lh = line_height.max(1.0);
        let display_row = map.map_doc_to_display(self.scroll_anchor.doc_line) as f64;
        let raw_scroll = display_row + self.scroll_anchor.pixel_offset as f64 / lh as f64;
        let max_scroll = (map.map_total_rows() as f64 - self.viewport_height).max(0.0);
        if raw_scroll > max_scroll {
            // Clamp scroll position, then back-derive consistent anchor.
            let dr = max_scroll.floor() as usize;
            let doc = map.map_display_to_doc(dr);
            let first = map.map_doc_to_display(doc);
            let row_off = dr.saturating_sub(first);
            let sub = max_scroll.fract() as f32;
            self.scroll_anchor = ScrollAnchor::new(doc, (row_off as f32 + sub) * lh);
        }
    }

    /// Set scroll position from a DisplayRow-space value (e.g. from scrollbar drag).
    /// Converts DisplayRow to (doc_line, pixel_offset) and clamps to valid range.
    pub fn set_scroll_top(&mut self, target: f64, map: &impl LineMap, line_height: f32) {
        let max_display = (map.map_total_rows() as f64 - self.viewport_height).max(0.0);
        let clamped = target.clamp(0.0, max_display);
        let dr = clamped.floor() as usize;
        let doc = map.map_display_to_doc(dr);
        let first = map.map_doc_to_display(doc);
        let row_off = dr.saturating_sub(first);
        let sub = clamped.fract() as f32;
        let lh = line_height.max(1.0);
        self.scroll_anchor = ScrollAnchor::new(doc, (row_off as f32 + sub) * lh);
    }

    // ── Doc-line convenience methods ──────────────────────────────

    /// Scroll to a specific document line (approximate).
    #[allow(deprecated)]
    pub fn scroll_to_doc_line(&mut self, line: usize) {
        self.scroll_to_row(line as f64);
    }

    /// Scroll to a specific document line using WrapIndex (exact).
    #[allow(deprecated)]
    pub fn scroll_to_doc_line_wrap(&mut self, line: usize, map: &impl LineMap) {
        let display_row = map.map_doc_to_display(line);
        self.scroll_to_row(display_row as f64);
    }
}
// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_row_basic() {
        let r = DisplayRow(5);
        assert_eq!(r.as_f64(), 5.0);
        assert_eq!(r.as_usize(), 5);
        assert_eq!(r.next(), DisplayRow(6));
    }

    #[test]
    fn display_row_saturating_sub() {
        assert_eq!(DisplayRow(3).saturating_sub(5), DisplayRow(0));
        assert_eq!(DisplayRow(10).saturating_sub(3), DisplayRow(7));
    }

    #[test]
    fn display_row_from_conversions() {
        assert_eq!(DisplayRow::from(42u32), DisplayRow(42));
        assert_eq!(DisplayRow::from(42usize), DisplayRow(42));
    }

    #[test]
    fn display_row_ordering() {
        assert!(DisplayRow(1) < DisplayRow(2));
        assert!(DisplayRow(5) > DisplayRow(3));
        assert_eq!(DisplayRow(0), DisplayRow::ZERO);
    }

    #[test]
    fn sub_line_pixel_offset_negative_for_positive_scroll() {
        let mut v = Viewport::new(30);
        v.scroll_top = 3.7;
        let offset = v.sub_line_pixel_offset(14.0);
        assert!(offset <= 0.0);
    }

    #[test]
    fn visible_doc_range_partial_overflow_line() {
        // pixel_offset=7, viewport=2 rows (28px). Anchor at line 5.
        // anchor_visible = 14-7 = 7px. remaining = 28-7 = 21px.
        // Subsequent line 6 (14px) fits, remaining = 7px.
        // Subsequent line 7 (14px) overflows (7px visible).
        // Range: 5..8 (anchor + 2 subsequent lines).
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(2);
        v.scroll_anchor = ScrollAnchor::new(5, 7.0);
        let range = v.visible_doc_range_from_anchor(&map, 14.0);
        assert_eq!(range, 5..8, "anchor (7px visible) + 2 subsequent lines");
    }

    #[test]
    fn visible_doc_range_exact_fit() {
        // 30 rows (420px), anchor=5, offset=0. 30 lines = 420px exactly.
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(5, 0.0);
        let range = v.visible_doc_range_from_anchor(&map, 14.0);
        assert_eq!(range, 5..35, "30 rows = 30 single-line rows");
    }

    #[test]
    fn visible_doc_range_partial_first_line() {
        // pixel_offset=10, viewport=30 rows (420px).
        // anchor_visible = 14-10 = 4px. remaining = 420-4 = 416px.
        // 29 subsequent lines (406px) fit, remaining = 10px.
        // 30th line (14px) overflows (10px visible).
        // Range: 5..36 (anchor + 30 subsequent lines).
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(5, 10.0);
        let range = v.visible_doc_range_from_anchor(&map, 14.0);
        assert_eq!(range, 5..36, "anchor (4px visible) + 30 subsequent lines");
    }

    // ── Anchor round-trip tests ──────────────────────────────

    /// Mock LineMap: doc_line → display_row = doc_line * vl (simulate wrap).
    struct MockLineMap {
        vl: usize,
    }
    impl LineMap for MockLineMap {
        fn map_line_count(&self) -> usize {
            1000
        }
        fn map_total_rows(&self) -> usize {
            1000 * self.vl
        }
        fn map_doc_to_display(&self, doc_line: usize) -> usize {
            doc_line * self.vl
        }
        fn map_display_to_doc(&self, display_row: usize) -> usize {
            if self.vl == 0 {
                0
            } else {
                display_row.saturating_sub(display_row % self.vl) / self.vl
            }
        }
        fn visual_line_count(&self, _doc_line: usize) -> u16 {
            self.vl as u16
        }
    }

    #[test]
    fn anchor_roundtrip_1_to_1() {
        // Each doc_line = 1 display_row
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_top = 42.5;
        v.sync_anchor_from_scroll(&map, 14.0);
        let saved_anchor = v.scroll_anchor;
        v.restore_scroll_from_anchor(&map, 14.0);
        assert!(
            (v.scroll_top - 42.5).abs() < 0.01,
            "round-trip should be identity, got {:.2}",
            v.scroll_top
        );
        assert_eq!(
            saved_anchor.doc_line, v.scroll_anchor.doc_line,
            "anchor should not change during restore"
        );
    }

    #[test]
    fn anchor_roundtrip_with_wrap() {
        // Each doc_line = 3 display rows (word-wrap)
        let map = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        // scroll_top = 25.0 → display_row 25 → doc_line 8 (row 25 / VL 3 = 8)
        // row_offset in doc_line: 25 - 8*3 = 1
        v.scroll_top = 25.0;
        v.sync_anchor_from_scroll(&map, 14.0);
        // anchor: doc_line = 8, pixel_offset = 1*14 = 14
        assert_eq!(v.scroll_anchor.doc_line, 8);
        assert!((v.scroll_anchor.pixel_offset - 14.0).abs() < 0.1);
        v.restore_scroll_from_anchor(&map, 14.0);
        // restore: display_row = 8*3 = 24, + 14/14 = 1 → 25.0
        assert!(
            (v.scroll_top - 25.0).abs() < 0.01,
            "round-trip with wrap should be identity, got {:.2}",
            v.scroll_top
        );
    }

    #[test]
    fn anchor_roundtrip_fractional() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_top = 100.3;
        v.sync_anchor_from_scroll(&map, 10.0);
        // doc_line = 100, pixel_offset = 0.3 * 10 = 3
        assert_eq!(v.scroll_anchor.doc_line, 100);
        assert!((v.scroll_anchor.pixel_offset - 3.0).abs() < 0.1);
        v.restore_scroll_from_anchor(&map, 10.0);
        // display_row = 100, + 3/10 = 0.3 → 100.3
        assert!((v.scroll_top - 100.3).abs() < 0.01);
    }

    #[test]
    fn clamp_scroll_top_does_not_write_anchor() {
        // clamp should NOT call sync_anchor; anchor stays unchanged
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_top = 500.0;
        v.sync_anchor_from_scroll(&map, 14.0);
        let anchor_before = v.scroll_anchor;
        v.clamp_scroll_top(&map, 14.0);
        assert_eq!(
            v.scroll_anchor.doc_line, anchor_before.doc_line,
            "clamp_scroll_top must not write anchor"
        );
        assert!(
            (v.scroll_anchor.pixel_offset - anchor_before.pixel_offset).abs() < 0.1,
            "clamp_scroll_top must not write anchor"
        );
    }

    #[test]
    fn restore_does_not_write_anchor() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_top = 100.0;
        v.sync_anchor_from_scroll(&map, 14.0);
        let anchor_before = v.scroll_anchor;
        v.scroll_top = 200.0; // move scroll_top away
        v.restore_scroll_from_anchor(&map, 14.0);
        assert_eq!(
            v.scroll_anchor.doc_line, anchor_before.doc_line,
            "restore_scroll_from_anchor must not write anchor"
        );
    }

    // ── Stage 5: Anchor-based API tests ─────────────────────────

    #[test]
    fn visible_doc_range_from_anchor_single_line() {
        // 30 visible rows, each line = 1 visual line, 14px line_height
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(10, 0.0);
        let range = v.visible_doc_range_from_anchor(&map, 14.0);
        assert_eq!(range, 10..40, "30 single-line rows visible from line 10");
    }

    #[test]
    fn visible_doc_range_from_anchor_with_wrap() {
        // Each doc_line = 3 visual lines (3 * 14 = 42px)
        // viewport = 30 rows → 30 * 14 = 420px
        // 420px / 42px per doc_line = 10 doc_lines
        let map = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(0, 0.0);
        let range = v.visible_doc_range_from_anchor(&map, 14.0);
        assert_eq!(range, 0..10, "30 rows with 3-wrap = 10 doc lines");
    }

    #[test]
    fn visible_doc_range_from_anchor_with_pixel_offset() {
        // pixel_offset = 7px, viewport = 30 rows * 14px = 420px
        // anchor_visible = 14-7 = 7px. remaining = 420-7 = 413px
        // 29 subsequent lines (406px) fit, remaining = 7px.
        // 30th line (14px) overflows (7px visible).
        // Range: 5..36 (anchor + 30 subsequent lines).
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(5, 7.0);
        let range = v.visible_doc_range_from_anchor(&map, 14.0);
        assert_eq!(range, 5..36, "anchor (7px visible) + 30 subsequent lines");
    }

    #[test]
    fn visible_doc_range_from_anchor_at_end() {
        // Start near the end, should clamp to total_lines
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(995, 0.0);
        let range = v.visible_doc_range_from_anchor(&map, 14.0);
        assert_eq!(range, 995..1000, "clamped to total_lines=1000");
    }

    #[test]
    fn visible_doc_range_tall_line_with_pixel_offset() {
        // Regression: when anchor is a tall line (e.g. 30 VL) and pixel_offset
        // scrolls into the middle, subsequent lines must still be included.
        // Viewport: 10 rows = 140px. Anchor line: 30 VL = 420px.
        // pixel_offset = 200 → anchor visible = 420-200 = 220px, but viewport only 140px
        // So anchor fills entire viewport, range = just anchor line.
        // pixel_offset = 350 → anchor visible = 420-350 = 70px, remaining = 140-70 = 70px
        // Next line (1 VL = 14px) fits, so range should include it.
        struct TallLineMap;
        impl LineMap for TallLineMap {
            fn map_line_count(&self) -> usize {
                5
            }
            fn map_total_rows(&self) -> usize {
                33
            } // 30 + 1 + 1 + 1
            fn map_display_to_doc(&self, _: usize) -> usize {
                0
            }
            fn map_doc_to_display(&self, _: usize) -> usize {
                0
            }
            fn visual_line_count(&self, doc_line: usize) -> u16 {
                if doc_line == 0 { 30 } else { 1 }
            }
        }
        let mut v = Viewport::new(10); // 10 rows = 140px
        // Case 1: pixel_offset = 350, anchor visible = 70px, remaining = 70px
        // Should include at least line 0 and line 1
        v.scroll_anchor = ScrollAnchor::new(0, 350.0);
        let range = v.visible_doc_range_from_anchor(&TallLineMap, 14.0);
        assert!(range.end > 1, "must include lines after tall anchor, got {:?}", range);

        // Case 2: pixel_offset = 410, anchor visible = 10px, remaining = 130px
        // Should include line 0 + multiple subsequent lines
        v.scroll_anchor = ScrollAnchor::new(0, 410.0);
        let range = v.visible_doc_range_from_anchor(&TallLineMap, 14.0);
        assert!(range.end >= 4, "must include most subsequent lines, got {:?}", range);
    }

    #[test]
    fn visible_doc_range_nonempty_when_pixel_offset_exceeds_viewport() {
        // Regression: when pixel_offset >= viewport_pixels, the while loop
        // never executes and end == start → empty range. The fix adds end += 1
        // guard. Test with very tall line (3 VL = 126px) and small viewport (1 row = 14px).
        struct TallLineMap;
        impl LineMap for TallLineMap {
            fn map_line_count(&self) -> usize {
                1
            }
            fn map_total_rows(&self) -> usize {
                3
            }
            fn map_display_to_doc(&self, _: usize) -> usize {
                0
            }
            fn map_doc_to_display(&self, _: usize) -> usize {
                0
            }
            fn visual_line_count(&self, _: usize) -> u16 {
                3
            }
        }
        let mut v = Viewport::new(1); // viewport_height = 1, viewport_pixels = 14
        v.scroll_anchor = ScrollAnchor::new(0, 14.0); // pixel_offset = 14 >= 14
        let range = v.visible_doc_range_from_anchor(&TallLineMap, 14.0);
        assert_eq!(range, 0..1, "single tall line must produce non-empty range");
    }

    #[test]
    fn set_scroll_top_no_wrap() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.set_scroll_top(50.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 50);
        assert!((v.scroll_anchor.pixel_offset - 0.0).abs() < 0.01, "exact line boundary");
    }

    #[test]
    fn set_scroll_top_with_wrap() {
        let map = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        // display_row 7 = doc_line 2 * 3 + 1 = doc_line 2, offset 1 VL (14 px into line)
        v.set_scroll_top(7.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 2);
        assert!((v.scroll_anchor.pixel_offset - 14.0).abs() < 0.01, "1 VL offset into doc_line 2");
    }

    #[test]
    fn set_scroll_top_clamps_to_max() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        // line_count=1000, total_rows=1000, max_scroll=970
        v.set_scroll_top(1500.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 970);
        assert!((v.scroll_anchor.pixel_offset - 0.0).abs() < 0.01);
    }

    #[test]
    fn clamp_anchor_with_wrap_clamps_display_row() {
        // map: 1000 lines, each VL=3. total_display_rows=3000. viewport=30.
        // If anchor is at doc_line=999, display_row=2997, max_scroll=2970.
        // Clamp should bring anchor back to doc_line=990 (display_row=2970).
        let map = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(999, 0.0);
        v.clamp_anchor(&map, 14.0);
        // display_row(999)=2997 > max=2970 → back-derive: dr=2970, doc=2970/3=990
        assert_eq!(v.scroll_anchor.doc_line, 990);
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0);
    }

    #[test]
    fn visible_doc_range_from_anchor_empty() {
        struct EmptyMap;
        impl LineMap for EmptyMap {
            fn map_line_count(&self) -> usize {
                0
            }
            fn map_total_rows(&self) -> usize {
                0
            }
            fn map_display_to_doc(&self, _: usize) -> usize {
                0
            }
            fn map_doc_to_display(&self, _: usize) -> usize {
                0
            }
            fn visual_line_count(&self, _: usize) -> u16 {
                0
            }
        }
        let v = Viewport::new(30);
        let range = v.visible_doc_range_from_anchor(&EmptyMap, 14.0);
        assert_eq!(range, 0..0, "empty map returns empty range");
    }

    #[test]
    fn scroll_doc_lines_basic() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(10, 5.0);
        v.scroll_doc_lines(5, &map);
        assert_eq!(v.scroll_anchor.doc_line, 15);
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0, "pixel_offset resets on doc_line scroll");
    }

    #[test]
    fn scroll_doc_lines_negative() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(10, 0.0);
        v.scroll_doc_lines(-3, &map);
        assert_eq!(v.scroll_anchor.doc_line, 7);
    }

    #[test]
    fn scroll_doc_lines_clamp_zero() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(2, 0.0);
        v.scroll_doc_lines(-10, &map);
        assert_eq!(v.scroll_anchor.doc_line, 0, "clamped to 0");
    }

    #[test]
    fn scroll_doc_lines_clamp_max() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(998, 0.0);
        v.scroll_doc_lines(100, &map);
        assert_eq!(v.scroll_anchor.doc_line, 999, "clamped to last line");
    }

    #[test]
    fn scroll_doc_lines_empty_map() {
        struct EmptyMap;
        impl LineMap for EmptyMap {
            fn map_line_count(&self) -> usize {
                0
            }
            fn map_total_rows(&self) -> usize {
                0
            }
            fn map_display_to_doc(&self, _: usize) -> usize {
                0
            }
            fn map_doc_to_display(&self, _: usize) -> usize {
                0
            }
            fn visual_line_count(&self, _: usize) -> u16 {
                0
            }
        }
        let mut v = Viewport::new(30);
        v.scroll_doc_lines(5, &EmptyMap);
        assert_eq!(v.scroll_anchor.doc_line, 0, "empty map no-op");
    }

    #[test]
    fn scroll_pixels_down_basic() {
        // 14px line height, scroll down 28px = 2 lines
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(0, 0.0);
        v.scroll_pixels(28.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 2);
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0);
    }

    #[test]
    fn scroll_pixels_down_fractional() {
        // Scroll 21px with 14px lines = 1 line + 7px
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(0, 0.0);
        v.scroll_pixels(21.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 1);
        assert!((v.scroll_anchor.pixel_offset - 7.0).abs() < 0.01);
    }

    #[test]
    fn scroll_pixels_up_basic() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(5, 7.0);
        v.scroll_pixels(-7.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 5);
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0);
    }

    #[test]
    fn scroll_pixels_up_cross_line() {
        // At line 5, offset 7. Scroll up -14px → line 4, offset 7.
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(5, 7.0);
        v.scroll_pixels(-14.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 4);
        assert!((v.scroll_anchor.pixel_offset - 7.0).abs() < 0.01);
    }

    #[test]
    fn scroll_pixels_up_clamp_zero() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(0, 3.0);
        v.scroll_pixels(-10.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 0);
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0, "clamped to 0");
    }

    #[test]
    fn scroll_pixels_down_clamp_max() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(999, 0.0);
        v.scroll_pixels(1000.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 999);
        assert!((v.scroll_anchor.pixel_offset - 14.0).abs() < 0.01, "clamped at last line height");
    }

    #[test]
    fn scroll_pixels_with_wrap() {
        // 3 VL per line, 14px per VL = 42px per doc_line
        let map = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(0, 0.0);
        v.scroll_pixels(42.0, &map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 1, "42px = exactly 1 wrapped line");
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0);
    }

    #[test]
    fn scroll_pixels_empty_map() {
        struct EmptyMap;
        impl LineMap for EmptyMap {
            fn map_line_count(&self) -> usize {
                0
            }
            fn map_total_rows(&self) -> usize {
                0
            }
            fn map_display_to_doc(&self, _: usize) -> usize {
                0
            }
            fn map_doc_to_display(&self, _: usize) -> usize {
                0
            }
            fn visual_line_count(&self, _: usize) -> u16 {
                0
            }
        }
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(10, 5.0);
        v.scroll_pixels(100.0, &EmptyMap, 14.0);
        // Empty map → early return, anchor unchanged (same as scroll_doc_lines)
        assert_eq!(v.scroll_anchor.doc_line, 10, "empty map no-op");
        assert_eq!(v.scroll_anchor.pixel_offset, 5.0);
    }

    #[test]
    fn scroll_pixels_large_delta_stress() {
        // Stress test: scroll 10000 lines at once (iterative, not recursive)
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(0, 0.0);
        v.scroll_pixels(10000.0 * 14.0, &map, 14.0);
        // Should clamp to last line (999) with max pixel_offset
        assert_eq!(v.scroll_anchor.doc_line, 999, "clamped to last line");
        assert!((v.scroll_anchor.pixel_offset - 14.0).abs() < 0.01, "clamped at last line height");
    }

    #[test]
    fn derive_scroll_top_matches_sync_anchor_roundtrip() {
        // With vl=3, doc_line=8, pixel_offset=14 → should match old sync/restore
        let map = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(8, 14.0);
        v.derive_scroll_top(&map, 14.0);
        // display_row = 8 * 3 = 24, + 14/14 = 1 → 25.0
        assert!(
            (v.scroll_top - 25.0).abs() < 0.01,
            "derive_scroll_top should match old restore, got {:.2}",
            v.scroll_top
        );
    }

    #[test]
    fn derive_scroll_top_empty_map() {
        struct EmptyMap;
        impl LineMap for EmptyMap {
            fn map_line_count(&self) -> usize {
                0
            }
            fn map_total_rows(&self) -> usize {
                0
            }
            fn map_display_to_doc(&self, _: usize) -> usize {
                0
            }
            fn map_doc_to_display(&self, _: usize) -> usize {
                0
            }
            fn visual_line_count(&self, _: usize) -> u16 {
                0
            }
        }
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(10, 5.0);
        v.derive_scroll_top(&EmptyMap, 14.0);
        // Should not panic; scroll_top stays 0.0 for empty maps
        assert!(v.scroll_top >= 0.0, "empty map derive shouldn't produce negative");
    }

    #[test]
    fn clamp_anchor_normal() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(500, 0.0);
        v.clamp_anchor(&map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 500, "within range, no change");
    }

    #[test]
    fn clamp_anchor_beyond_end() {
        let map = MockLineMap { vl: 1 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(1500, 5.0);
        v.clamp_anchor(&map, 14.0);
        // After DisplayRow clamp: max_scroll = 1000 - 30 = 970
        assert_eq!(v.scroll_anchor.doc_line, 970);
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0);
    }

    #[test]
    fn clamp_anchor_pixel_offset_exceeds_line() {
        let map = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(5, 100.0); // vl=3, max=42px
        v.clamp_anchor(&map, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 5);
        assert!((v.scroll_anchor.pixel_offset - 42.0).abs() < 0.01, "clamped to line height");
    }

    #[test]
    fn clamp_anchor_empty_map() {
        struct EmptyMap;
        impl LineMap for EmptyMap {
            fn map_line_count(&self) -> usize {
                0
            }
            fn map_total_rows(&self) -> usize {
                0
            }
            fn map_display_to_doc(&self, _: usize) -> usize {
                0
            }
            fn map_doc_to_display(&self, _: usize) -> usize {
                0
            }
            fn visual_line_count(&self, _: usize) -> u16 {
                0
            }
        }
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(10, 5.0);
        v.clamp_anchor(&EmptyMap, 14.0);
        assert_eq!(v.scroll_anchor.doc_line, 0);
        assert_eq!(v.scroll_anchor.pixel_offset, 0.0);
    }

    /// When VL is overestimated (e.g. placeholder est_vl), visible_doc_range_from_anchor
    /// returns a narrower doc-line range. This confirms the root cause of the
    /// "half-screen" bug: fewer doc lines are included than needed to fill the viewport.
    #[test]
    fn visible_range_overestimated_vl_narrows_range() {
        // 30-row viewport, 14px line height → 420 viewport pixels
        // VL=1 (correct): needs ~30 doc lines to fill viewport
        // VL=3 (overestimated by placeholder): needs ~10 doc lines
        let map_correct = MockLineMap { vl: 1 };
        let map_over = MockLineMap { vl: 3 };
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(0, 0.0);

        let range_correct = v.visible_doc_range_from_anchor(&map_correct, 14.0);
        let range_over = v.visible_doc_range_from_anchor(&map_over, 14.0);

        assert_eq!(
            range_correct.end - range_correct.start,
            30,
            "correct VL=1 should need ~30 doc lines for 30-row viewport"
        );
        assert_eq!(
            range_over.end - range_over.start,
            10,
            "overestimated VL=3 should need ~10 doc lines for 30-row viewport"
        );
        assert!(
            range_over.end - range_over.start < range_correct.end - range_correct.start,
            "overestimated VL produces narrower range than correct VL"
        );
    }

    /// The conservative bound (total_lines - start_doc) always exceeds or equals
    /// any range computed from VL estimates, ensuring the render loop never starves.
    #[test]
    fn conservative_bound_exceeds_estimated_range() {
        let map = MockLineMap { vl: 3 }; // overestimated VL
        let total_lines: usize = 1000;
        let mut v = Viewport::new(30);
        v.scroll_anchor = ScrollAnchor::new(500, 0.0);

        let estimated = v.visible_doc_range_from_anchor(&map, 14.0);
        let estimated_count = estimated.end - estimated.start;
        // Conservative bound: all lines from anchor to end of doc
        let conservative_bound = total_lines.saturating_sub(estimated.start);

        assert_eq!(conservative_bound, 500, "conservative bound = remaining doc lines");
        assert!(
            conservative_bound > estimated_count,
            "conservative bound ({}) > estimated range ({})",
            conservative_bound,
            estimated_count
        );
    }
}
