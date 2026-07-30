//! 持久化 B-tree，专为 DisplayLineMap 服务。
//!
//! 单维度（DisplayRow 累加），叶子最大 32 项，Arc 包装实现 O(1) clone（snapshot 共享）。

use std::ops::Range;
use std::sync::Arc;

use smallvec::SmallVec;

const TREE_BASE: usize = 16;
const LEAF_MAX: usize = 2 * TREE_BASE;

/// 单个 doc line 的视觉换行信息，存进 SnapTree。
#[derive(Clone, Debug)]
pub struct DisplayLineEntry {
    pub visual_line_count: u16,
    pub visual_breaks: SmallVec<[VisualBreak; 1]>,
    pub byte_offset: usize,
    pub byte_length: u32,
    pub content_hash: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualBreak {
    pub byte_start: u32,
    pub byte_end: u32,
    pub pixel_width: f32,
}

impl DisplayLineEntry {
    /// 占位：未 wrap 的单行，visual_line_count = 1。
    pub fn placeholder(
        byte_offset: usize,
        byte_length: u32,
        content_hash: u64,
        est_vl: u16,
    ) -> Self {
        let mut breaks = SmallVec::new();
        breaks.push(VisualBreak { byte_start: 0, byte_end: byte_length, pixel_width: 0.0 });
        Self {
            visual_line_count: est_vl.max(1),
            visual_breaks: breaks,
            byte_offset,
            byte_length,
            content_hash,
        }
    }
}

#[derive(Debug)]
enum Node {
    Leaf { entries: Vec<DisplayLineEntry>, total_rows: usize },
    Inner { children: Vec<Arc<Node>>, total_rows: usize, line_count: usize },
}

impl Node {
    fn line_count(&self) -> usize {
        match self {
            Node::Leaf { entries, .. } => entries.len(),
            Node::Inner { line_count, .. } => *line_count,
        }
    }
    fn total_rows(&self) -> usize {
        match self {
            Node::Leaf { total_rows, .. } => *total_rows,
            Node::Inner { total_rows, .. } => *total_rows,
        }
    }
}

/// 持久化 B-tree。Arc 包装 Node 实现 O(1) snapshot clone。
#[derive(Clone, Debug)]
pub struct SnapTree {
    root: Arc<Node>,
    line_count: usize,
}

/// Splice 操作结果。
#[derive(Debug)]
pub struct SpliceResult {
    pub rows_added: isize,
    pub lines_added: isize,
    pub rows_before_range: usize,
}

#[derive(Debug, Clone)]
pub struct RowLookup<'a> {
    pub doc_line: usize,
    pub visual_idx_in_doc: usize,
    pub entry: &'a DisplayLineEntry,
}

impl Default for SnapTree {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapTree {
    pub fn new() -> Self {
        Self { root: Arc::new(Node::Leaf { entries: Vec::new(), total_rows: 0 }), line_count: 0 }
    }

    pub fn from_entries(it: impl IntoIterator<Item = DisplayLineEntry>) -> Self {
        let entries: Vec<DisplayLineEntry> = it.into_iter().collect();
        let line_count = entries.len();
        let total_rows: usize = entries.iter().map(|e| e.visual_line_count as usize).sum();
        // 如果条目数少，直接一个叶子
        if entries.len() <= LEAF_MAX {
            return Self { root: Arc::new(Node::Leaf { entries, total_rows }), line_count };
        }
        // 否则建树
        let root = build_tree_from_entries(entries);
        Self { root, line_count }
    }

    pub fn line_count(&self) -> usize {
        self.line_count
    }

    pub fn total_rows(&self) -> usize {
        self.root.total_rows()
    }

    /// 按 visual row 索引查找对应的 doc line 和 intra-line visual index。
    pub fn find_by_row(&self, row: usize) -> Option<RowLookup<'_>> {
        row_lookup_at(&self.root, row)
    }

    /// doc_line → visual row offset (by positional index, not byte_offset)
    pub fn line_to_row(&self, doc_line: usize) -> usize {
        if self.line_count == 0 {
            return 0;
        }
        let doc_line = doc_line.min(self.line_count);
        line_to_row_inner(&self.root, doc_line)
    }

    /// O(1) access to a doc line entry by index (uses underlying Vec storage).
    /// Returns None if doc_line is out of bounds.
    pub fn get_entry(&self, doc_line: usize) -> Option<&DisplayLineEntry> {
        get_entry_inner(&self.root, doc_line)
    }

    pub fn splice(
        &mut self,
        range: Range<usize>,
        replacements: Vec<DisplayLineEntry>,
    ) -> SpliceResult {
        // 简化实现：全重建 + 替换
        let mut entries: Vec<DisplayLineEntry> =
            self.iter_lines(0..self.line_count).cloned().collect();
        let old_len = entries.len();
        let rows_before_range: usize =
            entries[..range.start].iter().map(|e| e.visual_line_count as usize).sum();
        let removed_rows: usize = entries[range.start..range.end.min(old_len)]
            .iter()
            .map(|e| e.visual_line_count as usize)
            .sum();
        let added_rows: usize = replacements.iter().map(|e| e.visual_line_count as usize).sum();

        entries.splice(range.start..range.end.min(old_len), replacements);
        let new_line_count = entries.len();

        let rows_added = added_rows as isize - removed_rows as isize;
        let lines_added = new_line_count as isize - old_len as isize;

        *self = Self::from_entries(entries);

        SpliceResult { rows_added, lines_added, rows_before_range }
    }

    pub fn iter_lines(&self, range: Range<usize>) -> LineIter<'_> {
        LineIter { tree: &self.root, idx: range.start, end: range.end.min(self.line_count) }
    }

    pub fn iter_rows(&self, rows: Range<usize>) -> RowIter<'_> {
        RowIter {
            tree: &self.root,
            current_row: rows.start,
            end_row: rows.end.min(self.root.total_rows()),
        }
    }
}

/// O(log n) prefix sum: compute display row offset for doc_line.
fn line_to_row_inner(node: &Arc<Node>, doc_line: usize) -> usize {
    match node.as_ref() {
        Node::Leaf { entries, .. } => {
            entries[..doc_line].iter().map(|e| e.visual_line_count as usize).sum()
        }
        Node::Inner { children, .. } => {
            let mut row = 0usize;
            let mut remaining = doc_line;
            for child in children {
                let child_lines = child.line_count();
                if remaining < child_lines {
                    return row + line_to_row_inner(child, remaining);
                }
                remaining -= child_lines;
                row += child.total_rows();
            }
            row
        }
    }
}

/// O(log n) entry lookup by doc line index.
fn get_entry_inner(node: &Arc<Node>, doc_line: usize) -> Option<&DisplayLineEntry> {
    match node.as_ref() {
        Node::Leaf { entries, .. } => entries.get(doc_line),
        Node::Inner { children, .. } => {
            let mut remaining = doc_line;
            for child in children {
                let child_lines = child.line_count();
                if remaining < child_lines {
                    return get_entry_inner(child, remaining);
                }
                remaining -= child_lines;
            }
            None
        }
    }
}

fn build_tree_from_entries(mut entries: Vec<DisplayLineEntry>) -> Arc<Node> {
    if entries.is_empty() {
        return Arc::new(Node::Leaf { entries: Vec::new(), total_rows: 0 });
    }
    // Build leaf nodes by draining chunks from the END of entries.
    // Draining from the end avoids O(n^2) shifting (Vec::drain from front
    // shifts all remaining elements each call).
    let mut leaves: Vec<Arc<Node>> = Vec::new();
    while !entries.is_empty() {
        let start = entries.len().saturating_sub(LEAF_MAX);
        let chunk: Vec<DisplayLineEntry> = entries.drain(start..).collect();
        let total_rows: usize = chunk.iter().map(|e| e.visual_line_count as usize).sum();
        leaves.push(Arc::new(Node::Leaf { entries: chunk, total_rows }));
    }
    leaves.reverse(); // restore original order (we drained from the end)
    // Build inner nodes bottom-up until only one root remains
    while leaves.len() > 1 {
        let mut next_level: Vec<Arc<Node>> = Vec::new();
        for chunk in leaves.chunks(LEAF_MAX) {
            let children = chunk.to_vec();
            let total_rows: usize = children.iter().map(|c| c.total_rows()).sum();
            let line_count: usize = children.iter().map(|c| c.line_count()).sum();
            next_level.push(Arc::new(Node::Inner { children, total_rows, line_count }));
        }
        leaves = next_level;
    }
    leaves
        .into_iter()
        .next()
        .unwrap_or_else(|| Arc::new(Node::Leaf { entries: Vec::new(), total_rows: 0 }))
}

pub struct LineIter<'a> {
    tree: &'a Arc<Node>,
    idx: usize,
    end: usize,
}

impl<'a> Iterator for LineIter<'a> {
    type Item = &'a DisplayLineEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.end {
            return None;
        }
        // 简化：flat 查找
        let result = nth_entry(self.tree, self.idx);
        self.idx += 1;
        result
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.idx;
        (remaining, Some(remaining))
    }
}

fn nth_entry(node: &Arc<Node>, idx: usize) -> Option<&DisplayLineEntry> {
    match node.as_ref() {
        Node::Leaf { entries, .. } => entries.get(idx),
        Node::Inner { children, .. } => {
            let mut offset = 0usize;
            for child in children {
                let lc = child.line_count();
                if idx < offset + lc {
                    return nth_entry(child, idx - offset);
                }
                offset += lc;
            }
            None
        }
    }
}

pub struct RowIter<'a> {
    tree: &'a Arc<Node>,
    current_row: usize,
    end_row: usize,
}

impl<'a> Iterator for RowIter<'a> {
    type Item = RowLookup<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_row >= self.end_row {
            return None;
        }
        let result = row_lookup_at(self.tree, self.current_row);
        if let Some(ref _rl) = result {
            self.current_row += 1;
        }
        result
    }
}

fn row_lookup_at<'a>(node: &'a Arc<Node>, row: usize) -> Option<RowLookup<'a>> {
    fn inner<'a>(node: &'a Arc<Node>, row: usize, doc_line_offset: usize) -> Option<RowLookup<'a>> {
        match node.as_ref() {
            Node::Leaf { entries, .. } => {
                let mut prefix = 0usize;
                for (i, entry) in entries.iter().enumerate() {
                    let vr = entry.visual_line_count as usize;
                    if row < prefix + vr {
                        return Some(RowLookup {
                            doc_line: doc_line_offset + i,
                            visual_idx_in_doc: row - prefix,
                            entry,
                        });
                    }
                    prefix += vr;
                }
                None
            }
            Node::Inner { children, .. } => {
                let mut offset = 0usize;
                let mut doc_prefix = doc_line_offset;
                for child in children {
                    let tr = child.total_rows();
                    if row < offset + tr {
                        return inner(child, row - offset, doc_prefix);
                    }
                    offset += tr;
                    doc_prefix += child.line_count();
                }
                None
            }
        }
    }
    inner(node, row, 0)
}

#[cfg(test)]
mod tests {

    use super::*;

    fn make_entry(visual_line_count: u16) -> DisplayLineEntry {
        DisplayLineEntry {
            visual_line_count,
            visual_breaks: {
                let mut b = SmallVec::new();
                b.push(VisualBreak { byte_start: 0, byte_end: 80, pixel_width: 800.0 });
                b
            },
            byte_offset: 0,
            byte_length: 80,
            content_hash: 42,
        }
    }

    #[test]
    fn new_is_empty() {
        let t = SnapTree::new();
        assert_eq!(t.line_count(), 0);
        assert_eq!(t.total_rows(), 0);
    }

    #[test]
    fn from_entries_basic() {
        let entries: Vec<_> = (0..10).map(|_| make_entry(1)).collect();
        let t = SnapTree::from_entries(entries);
        assert_eq!(t.line_count(), 10);
        assert_eq!(t.total_rows(), 10);
    }

    #[test]
    fn find_by_row_single() {
        let entries = vec![make_entry(3), make_entry(2), make_entry(1)];
        let t = SnapTree::from_entries(entries);
        // row 0-2 → entry 0
        let r = t.find_by_row(0).unwrap();
        assert_eq!(r.visual_idx_in_doc, 0);
        let r = t.find_by_row(2).unwrap();
        assert_eq!(r.visual_idx_in_doc, 2);
        // row 3-4 → entry 1
        let r = t.find_by_row(3).unwrap();
        assert_eq!(r.visual_idx_in_doc, 0);
        assert_eq!(r.entry.visual_line_count, 2);
        let r = t.find_by_row(4).unwrap();
        assert_eq!(r.visual_idx_in_doc, 1);
        // row 5 → entry 2
        let r = t.find_by_row(5).unwrap();
        assert_eq!(r.visual_idx_in_doc, 0);
    }

    #[test]
    fn find_by_row_out_of_bounds() {
        let entries = vec![make_entry(1)];
        let t = SnapTree::from_entries(entries);
        assert!(t.find_by_row(1).is_none());
        assert!(t.find_by_row(100).is_none());
    }

    #[test]
    fn line_to_row() {
        let entries = vec![
            make_entry(2), // doc_line 0 → row 0
            make_entry(1), // doc_line 1 → row 2
            make_entry(3), // doc_line 2 → row 3
        ];
        let t = SnapTree::from_entries(entries);
        assert_eq!(t.line_to_row(0), 0);
        assert_eq!(t.line_to_row(1), 2);
        assert_eq!(t.line_to_row(2), 3);
    }

    #[test]
    fn splice_insert() {
        let entries: Vec<_> = (0..5)
            .map(|i| {
                let mut e = make_entry(1);
                e.byte_offset = i * 80;
                e
            })
            .collect();
        let mut t = SnapTree::from_entries(entries);
        let result = t.splice(2..2, vec![make_entry(3)]);
        assert_eq!(result.lines_added, 1);
        assert_eq!(result.rows_added, 3); // added 3, removed 0
        assert_eq!(t.line_count(), 6);
    }

    #[test]
    fn splice_remove() {
        let entries: Vec<_> = (0..5).map(|_| make_entry(1)).collect();
        let mut t = SnapTree::from_entries(entries);
        let result = t.splice(1..4, vec![]);
        assert_eq!(result.lines_added, -3);
        assert_eq!(result.rows_added, -3);
        assert_eq!(t.line_count(), 2);
    }

    #[test]
    fn splice_replace() {
        let entries: Vec<_> = (0..5).map(|_| make_entry(1)).collect();
        let mut t = SnapTree::from_entries(entries);
        let result = t.splice(1..3, vec![make_entry(5), make_entry(2)]);
        assert_eq!(result.lines_added, 0);
        assert_eq!(result.rows_added, 5); // added 7, removed 2
        assert_eq!(t.line_count(), 5);
    }

    #[test]
    fn large_build_20000_entries() {
        let entries: Vec<_> = (0..20000).map(|_| make_entry(1)).collect();
        let t = SnapTree::from_entries(entries);
        assert_eq!(t.line_count(), 20000);
        assert_eq!(t.total_rows(), 20000);
    }

    #[test]
    fn multi_row_entries_total_rows() {
        let entries = vec![make_entry(3), make_entry(1), make_entry(5), make_entry(2)];
        let t = SnapTree::from_entries(entries);
        assert_eq!(t.total_rows(), 11);
    }

    #[test]
    fn row_iter_yields_correct_doc_lines() {
        let entries: Vec<_> = (0..10)
            .map(|i| {
                let mut e = make_entry(2);
                e.byte_offset = i * 80;
                e
            })
            .collect();
        let t = SnapTree::from_entries(entries);
        let results: Vec<_> = t.iter_rows(0..t.total_rows()).collect();
        assert_eq!(results.len(), 20); // 10 entries, each 2 rows = 20 visual rows total
        // first two RowLookups should be for doc_line 0
        assert_eq!(results[0].doc_line, 0);
        assert_eq!(results[0].visual_idx_in_doc, 0);
        assert_eq!(results[1].doc_line, 0);
        assert_eq!(results[1].visual_idx_in_doc, 1);
        assert_eq!(results[2].doc_line, 1);
    }

    #[test]
    fn line_iter_size_hint() {
        let entries: Vec<_> = (0..5).map(|_| make_entry(1)).collect();
        let t = SnapTree::from_entries(entries);
        let iter = t.iter_lines(0..t.line_count());
        assert_eq!(iter.size_hint(), (5, Some(5)));
    }

    #[test]
    fn clone_is_cheap() {
        let entries: Vec<_> = (0..100).map(|_| make_entry(1)).collect();
        let t1 = SnapTree::from_entries(entries);
        let t2 = t1.clone();
        assert_eq!(t2.line_count(), 100);
        // Arc clone: same root pointer
        assert!(std::sync::Arc::ptr_eq(&t1.root, &t2.root));
    }

    #[test]
    fn find_by_row_empty_tree() {
        let t = SnapTree::new();
        assert!(t.find_by_row(0).is_none());
    }

    #[test]
    fn splice_at_end() {
        let entries: Vec<_> = (0..3).map(|_| make_entry(1)).collect();
        let mut t = SnapTree::from_entries(entries);
        let result = t.splice(3..3, vec![make_entry(2)]);
        assert_eq!(result.lines_added, 1);
        assert_eq!(t.line_count(), 4);
    }

    #[test]
    fn splice_beyond_end_clamped() {
        let entries: Vec<_> = (0..3).map(|_| make_entry(1)).collect();
        let mut t = SnapTree::from_entries(entries);
        let result = t.splice(2..10, vec![]);
        assert_eq!(t.line_count(), 2);
        assert_eq!(result.lines_added, -1);
    }

    #[test]
    fn iter_lines_empty_range() {
        let entries: Vec<_> = (0..5).map(|_| make_entry(1)).collect();
        let t = SnapTree::from_entries(entries);
        let result: Vec<_> = t.iter_lines(2..2).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn large_build_performance() {
        use std::time::Instant;
        let entries: Vec<_> = (0..20000).map(|_| make_entry(1)).collect();
        let start = Instant::now();
        let t = SnapTree::from_entries(entries);
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 50,
            "large_build took {}ms, expected < 50ms",
            elapsed.as_millis()
        );
        assert_eq!(t.line_count(), 20000);
    }

    #[test]
    fn find_by_row_mixed_visual_lines() {
        let entries = vec![make_entry(3), make_entry(1), make_entry(2)];
        let t = SnapTree::from_entries(entries);
        assert_eq!(t.find_by_row(0).unwrap().doc_line, 0);
        assert_eq!(t.find_by_row(2).unwrap().doc_line, 0);
        assert_eq!(t.find_by_row(3).unwrap().doc_line, 1);
        assert_eq!(t.find_by_row(4).unwrap().doc_line, 2);
        assert_eq!(t.find_by_row(5).unwrap().doc_line, 2);
    }
}
// force rebuild marker
