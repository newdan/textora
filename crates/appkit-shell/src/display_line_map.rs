//! DisplayLineMap: 对齐 Zed DisplayMap 的持久化 DocLine → VisualRow 映射。
//!
//! 基于 SnapTree 构建，Snapshot/Patch 模型实现渲染层无锁读取。
//! 与 WrapIndex 并行运行，Phase 2 阶段通过 parallel-assert 验证一致性。

use std::ops::Range;
use std::sync::Arc;

use crate::snap_tree::{DisplayLineEntry, SnapTree};

/// DisplayLineMap 的不可变快照（O(1) clone）。
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub tree: Arc<SnapTree>,
    pub generation: u64,
    pub viewport_width: f32,
    pub font_size: f32,
}

/// 编辑后产生的差异信息。
#[derive(Clone, Debug, Default)]
pub struct DisplayPatch {
    /// 受影响的 doc_line 范围（用于 invalidate RenderCache）。
    pub affected_lines: Range<usize>,
    /// 总的 visual row 变化量。
    pub rows_delta: isize,
    /// 受影响范围之前的 visual row 偏移。
    pub rows_before: usize,
}

/// 持久化映射。
#[derive(Debug, Clone)]
pub struct DisplayLineMap {
    tree: SnapTree,
    /// O(1) parallel array for doc_line → entry access.
    entries: Vec<DisplayLineEntry>,
    generation: u64,
    viewport_width: f32,
    font_size: f32,
    line_count: usize,
}

impl Default for DisplayLineMap {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayLineMap {
    pub fn new() -> Self {
        Self {
            tree: SnapTree::new(),
            entries: Vec::new(),
            generation: 1,
            viewport_width: 800.0,
            font_size: 14.0,
            line_count: 0,
        }
    }

    /// 创建不可变快照。
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tree: Arc::new(self.tree.clone()),
            generation: self.generation,
            viewport_width: self.viewport_width,
            font_size: self.font_size,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn line_count(&self) -> usize {
        self.line_count
    }

    pub fn total_rows(&self) -> usize {
        self.tree.total_rows()
    }

    pub fn viewport_width(&self) -> f32 {
        self.viewport_width
    }

    /// 设置视口宽度或字号，变化时触发重新 wrap。
    pub fn set_viewport_size(&mut self, width: f32, font_size: f32) {
        let width_changed = (self.viewport_width - width).abs() > 1.0;
        let font_changed = (self.font_size - font_size).abs() > 0.1;

        // Always sync viewport_width and font_size so callers (init_display_map,
        // snapshots, etc.) see the current values even when the delta is small.
        self.viewport_width = width;
        self.font_size = font_size;

        if width_changed || font_changed {
            // Clear visual_breaks to force re-wrap with new dimensions,
            // but preserve visual_line_count to prevent scrollbar oscillation
            // when long lines expand from placeholder VL=1 to shaped VL=N.
            // The tree is NOT rebuilt here — async reshape will update entries
            // and rebuild_tree will be called from drain_reshape_results.
            for entry in &mut self.entries {
                entry.visual_breaks.clear();
            }
            self.generation += 1;
        }
    }

    /// 批量设置初始条目（用于从 TextBuffer 冷启动）。
    pub fn set_entries(&mut self, new_entries: Vec<DisplayLineEntry>) {
        self.line_count = new_entries.len();
        // Store entries first, then build tree (SnapTree clones internally).
        self.entries = new_entries;
        self.tree = SnapTree::from_entries(self.entries.iter().cloned());
        self.generation += 1;
    }

    /// 应用编辑，返回新 Snapshot 和差异信息。
    pub fn sync(
        &mut self,
        edit_range: Range<usize>,
        replacements: Vec<DisplayLineEntry>,
    ) -> (Snapshot, DisplayPatch) {
        // Bounds check: clamp to valid range
        let start = edit_range.start.min(self.entries.len());
        let end = edit_range.end.min(self.entries.len());
        if start > end || start > self.entries.len() {
            return (
                self.snapshot(),
                DisplayPatch { affected_lines: 0..self.line_count, rows_delta: 0, rows_before: 0 },
            );
        }
        let splice_result = self.tree.splice(start..end, replacements.clone());
        self.line_count = self.tree.line_count();
        self.generation += 1;

        // Update parallel entries Vec
        self.entries.splice(start..end, replacements);

        let patch = DisplayPatch {
            affected_lines: 0..self.line_count, // 简化：mark all affected
            rows_delta: splice_result.rows_added,
            rows_before: splice_result.rows_before_range,
        };

        (self.snapshot(), patch)
    }

    /// display_row → doc_line
    pub fn display_to_doc(&self, display_row: usize) -> usize {
        self.tree
            .find_by_row(display_row)
            .map(|r| r.doc_line)
            .unwrap_or(self.line_count.saturating_sub(1))
    }

    /// display_row → (doc_line, visual_idx_in_doc)
    pub fn display_to_doc_with_intra(&self, display_row: usize) -> (usize, usize) {
        self.tree
            .find_by_row(display_row)
            .map(|r| (r.doc_line, r.visual_idx_in_doc))
            .unwrap_or((self.line_count.saturating_sub(1), 0))
    }

    /// doc_line → display_row（visual row 偏移量）
    pub fn doc_to_display(&self, doc_line: usize) -> usize {
        if doc_line >= self.line_count {
            return self.tree.total_rows();
        }
        self.tree.line_to_row(doc_line)
    }
    /// O(1) access to a DisplayLineEntry by doc_line index.
    pub fn get_entry(&self, doc_line: usize) -> Option<&DisplayLineEntry> {
        self.entries.get(doc_line)
    }

    /// Return total number of entries for O(1) access iteration.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Fast in-place update: only modifies the Vec, does NOT rebuild the tree.
    /// Use this in hot paths. Call `rebuild_tree()` later when tree queries are needed.
    pub fn update_entry_in_place(&mut self, doc_line: usize, entry: DisplayLineEntry) {
        if doc_line < self.entries.len() {
            self.entries[doc_line] = entry;
        }
    }

    /// Rebuild the SnapTree from the entries Vec (call after batch updates).
    pub fn rebuild_tree(&mut self) {
        self.tree = SnapTree::from_entries(self.entries.clone());
        self.line_count = self.entries.len();
    }
}

// ── ui::viewport::LineMap impl ────────────────────────────────────

impl ui::viewport::LineMap for DisplayLineMap {
    fn map_line_count(&self) -> usize {
        self.line_count()
    }
    fn map_total_rows(&self) -> usize {
        self.total_rows()
    }
    fn map_display_to_doc(&self, display_row: usize) -> usize {
        self.display_to_doc(display_row)
    }
    fn map_doc_to_display(&self, doc_line: usize) -> usize {
        self.doc_to_display(doc_line)
    }
    fn visual_line_count(&self, doc_line: usize) -> u16 {
        self.entries.get(doc_line).map(|e| e.visual_line_count).unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snap_tree::{DisplayLineEntry, VisualBreak};

    fn entry(vl: u16) -> DisplayLineEntry {
        let mut e = DisplayLineEntry::placeholder(0, 80, 0, 1);
        e.visual_line_count = vl;
        e
    }

    #[test]
    fn new_is_empty() {
        let m = DisplayLineMap::new();
        assert_eq!(m.line_count(), 0);
        assert_eq!(m.total_rows(), 0);
    }

    #[test]
    fn set_entries_basic() {
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(1), entry(3), entry(2)]);
        assert_eq!(m.line_count(), 3);
        assert_eq!(m.total_rows(), 6);
    }

    #[test]
    fn snapshot_captures_state() {
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(2), entry(1)]);
        let snap = m.snapshot();
        assert_eq!(snap.tree.total_rows(), 3);
        assert_eq!(snap.generation, m.generation());
    }

    #[test]
    fn sync_returns_patch() {
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(1), entry(1), entry(1)]);
        let (snap, patch) = m.sync(1..2, vec![entry(3)]);
        assert_eq!(snap.tree.total_rows(), 5);
        assert_eq!(patch.rows_delta, 2); // added 3, removed 1
    }

    #[test]
    fn sync_out_of_bounds_range() {
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(1), entry(1)]);

        // This simulates the out-of-bounds text delete range where edit_range extends past line_count
        let (snap, patch) = m.sync(1..5, vec![entry(2)]);

        // Should have safely clamped to 1..2 and succeeded
        assert_eq!(m.line_count(), 2);
        assert_eq!(snap.tree.total_rows(), 3);
        assert_eq!(patch.rows_delta, 1);

        // Completely out of bounds should be clamped to the end (appending)
        let (_snap2, patch2) = m.sync(10..20, vec![entry(3)]);
        assert_eq!(m.line_count(), 3, "Clamped out of bounds splice adds to the end");
        assert_eq!(patch2.rows_delta, 3, "Added one entry of visual_line_count 3");
    }

    #[test]
    fn display_to_doc_mapping() {
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(2), entry(1), entry(3)]);
        assert_eq!(m.display_to_doc(0), 0);
        assert_eq!(m.display_to_doc(1), 0);
        assert_eq!(m.display_to_doc(2), 1);
        assert_eq!(m.display_to_doc(3), 2);
    }

    #[test]
    fn doc_to_display_mapping() {
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(2), entry(1), entry(3)]);
        assert_eq!(m.doc_to_display(0), 0);
        assert_eq!(m.doc_to_display(1), 2);
        assert_eq!(m.doc_to_display(2), 3);
    }

    #[test]
    fn parallel_consistency_1000_random_queries() {
        // 生成随机条目，验证 display_to_doc 和 doc_to_display 一致性
        let mut m = DisplayLineMap::new();
        let entries: Vec<_> = (0..100)
            .map(|i| {
                let vl = ((i % 5) + 1) as u16;
                let mut e = DisplayLineEntry::placeholder(i * 80, 80, 0, 1);
                e.visual_line_count = vl;
                e
            })
            .collect();
        m.set_entries(entries);

        // 验证 display_to_doc → doc_to_display 往返
        for row in 0..m.total_rows() {
            let doc = m.display_to_doc(row);
            let disp = m.doc_to_display(doc);
            assert!(disp <= row, "row={row} doc={doc} disp={disp}");
        }
    }

    #[test]
    fn update_entry_in_place_does_not_affect_total_rows() {
        // Core invariant for ghost-line fix: update_entry_in_place changes
        // the entry but does NOT rebuild the tree. total_rows stays stale
        // until rebuild_tree() is called.
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(1), entry(2), entry(3)]); // total = 6
        assert_eq!(m.total_rows(), 6);

        // Change line 1 from 2 visual lines to 5
        let mut e = entry(5);
        e.byte_offset = 80;
        m.update_entry_in_place(1, e);

        // Entry updated but tree NOT rebuilt — total_rows still 6
        assert_eq!(m.get_entry(1).unwrap().visual_line_count, 5);
        assert_eq!(m.total_rows(), 6, "total_rows should be stale after update_entry_in_place");

        // After rebuild_tree, total_rows reflects new count: 1+5+3=9
        m.rebuild_tree();
        assert_eq!(m.total_rows(), 9);
    }

    #[test]
    fn rebuild_tree_after_multiple_in_place_updates() {
        // Multiple update_entry_in_place calls, then one rebuild_tree.
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(1), entry(1), entry(1), entry(1)]); // total = 4
        assert_eq!(m.total_rows(), 4);

        m.update_entry_in_place(0, entry(3));
        m.update_entry_in_place(2, entry(2));

        // Tree still has old values
        assert_eq!(m.total_rows(), 4);
        assert_eq!(m.get_entry(0).unwrap().visual_line_count, 3);
        assert_eq!(m.get_entry(2).unwrap().visual_line_count, 2);

        m.rebuild_tree();
        assert_eq!(m.total_rows(), 7); // 3+1+2+1=7
    }

    #[test]
    fn doc_to_display_uses_rebuilt_tree() {
        // After rebuild_tree, doc_to_display must reflect new counts.
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(2), entry(2), entry(2)]); // total=6
        assert_eq!(m.doc_to_display(0), 0);
        assert_eq!(m.doc_to_display(1), 2);
        assert_eq!(m.doc_to_display(2), 4);

        // Change line 1 from 2→5 visual lines in-place
        let mut e = entry(5);
        e.byte_offset = 80;
        m.update_entry_in_place(1, e);
        m.rebuild_tree();

        assert_eq!(m.total_rows(), 9); // 2+5+2=9
        assert_eq!(m.doc_to_display(0), 0);
        assert_eq!(m.doc_to_display(1), 2);
        assert_eq!(m.doc_to_display(2), 7); // shifted by extra 3 rows from line 1
    }

    #[test]
    fn visual_line_count_zero_after_update_entry() {
        // Edge case: reduce visual_line_count to 0? Shouldn't happen in practice
        // but verify it doesn't panic.
        let mut m = DisplayLineMap::new();
        m.set_entries(vec![entry(1), entry(1)]);
        assert_eq!(m.total_rows(), 2);

        let mut e = entry(0);
        e.byte_offset = 0;
        m.update_entry_in_place(0, e);
        m.rebuild_tree();
        assert_eq!(m.total_rows(), 1); // 0+1=1
    }

    #[test]
    fn set_viewport_size_preserves_visual_line_count() {
        // When viewport width changes, set_viewport_size should clear
        // visual_breaks but preserve visual_line_count to prevent
        // scrollbar oscillation when long lines have VL >> 1.
        let mut m = DisplayLineMap::new();
        // Set entries with VL=3 for line 1 (simulating a wrapped long line)
        m.set_entries(vec![entry(1), entry(3), entry(1)]);
        assert_eq!(m.total_rows(), 5);
        // Populate visual_breaks (simulating shaped state)
        for entry in &mut m.entries {
            entry.visual_breaks.push(VisualBreak {
                byte_start: 0,
                byte_end: 80,
                pixel_width: 100.0,
            });
        }
        // Trigger width change
        m.set_viewport_size(900.0, 14.0);
        // visual_breaks should be cleared (force re-wrap)
        for entry in &m.entries {
            assert!(entry.visual_breaks.is_empty(), "visual_breaks should be cleared");
        }
        // visual_line_count should be preserved (not reset to 1)
        assert_eq!(m.entries[1].visual_line_count, 3, "VL count should be preserved");
        // total_rows from tree should still reflect preserved VL counts
        assert_eq!(m.total_rows(), 5, "total_rows should be preserved");
        // Subsequent call with same width should be a no-op
        m.set_viewport_size(900.0, 14.0);
        // No change — everything should stay the same
        assert_eq!(m.total_rows(), 5);
    }

    /// Placeholder VL estimates can diverge from actual VL.
    /// When entries with overestimated VL are replaced with correct VL,
    /// total_rows changes. This reproduces the half-screen root cause:
    /// visible_doc_range_from_anchor uses placeholder VL to determine
    /// how many doc lines to render, and overestimation leads to too few.
    #[test]
    fn placeholder_vl_diverges_from_actual() {
        let mut m = DisplayLineMap::new();
        // Simulate placeholder entries with overestimated VL (est_vl=3 for each 120-byte Latin line)
        let placeholder_entries: Vec<_> = (0..50)
            .map(|i| {
                let mut e = DisplayLineEntry::placeholder(i * 120, 120, 0, 3);
                e.visual_line_count = 3; // placeholder overestimates
                e
            })
            .collect();
        m.set_entries(placeholder_entries);
        let placeholder_total = m.total_rows();
        assert_eq!(placeholder_total, 150, "50 lines * VL=3 = 150 rows");

        // Replace with actual VL (VL=2 for each line — twice as many visual rows as placeholder expected)
        let actual_entries: Vec<_> = (0..50)
            .map(|i| {
                let mut e = DisplayLineEntry::placeholder(i * 120, 120, 0, 2);
                e.visual_line_count = 2; // actual is lower
                e
            })
            .collect();
        m.set_entries(actual_entries);
        let actual_total = m.total_rows();
        assert_eq!(actual_total, 100, "50 lines * VL=2 = 100 rows");

        assert!(
            placeholder_total > actual_total,
            "placeholder overestimation (VL=3→{}) > actual (VL=2→{})",
            placeholder_total,
            actual_total
        );
    }

    /// Verify that VL=1 (most conservative estimate) always yields the widest
    /// visible doc range. Used as the implicit safe bound in the render loop.
    #[test]
    fn conservative_vl_one_maximizes_range() {
        let mut m = DisplayLineMap::new();
        // Entries with VL=1
        let vl1: Vec<_> = (0..100)
            .map(|i| {
                let mut e = DisplayLineEntry::placeholder(i * 80, 80, 0, 1);
                e.visual_line_count = 1;
                e
            })
            .collect();
        m.set_entries(vl1);
        let rows_vl1 = m.total_rows();

        // Same entries with VL=3 (overestimated)
        let vl3: Vec<_> = (0..100)
            .map(|i| {
                let mut e = DisplayLineEntry::placeholder(i * 80, 80, 0, 1);
                e.visual_line_count = 3;
                e
            })
            .collect();
        m.set_entries(vl3);
        let rows_vl3 = m.total_rows();

        // VL=1 gives the lowest total_rows → widest doc range in visible_doc_range_from_anchor
        assert!(
            rows_vl1 < rows_vl3,
            "VL=1 total_rows ({}) < VL=3 total_rows ({}) → VL=1 includes most doc lines per viewport",
            rows_vl1,
            rows_vl3
        );
    }
}
