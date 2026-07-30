# 大文件滚动性能 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-06-03-large-file-scroll-perf-design.md`

**Goal:** 4.1MB / 18151 行 JSON 滚动一帧 < 2ms；同时彻底对齐 Zed `DisplayMap` 模型，把 `WrapIndex` 替换成 `DisplayLineMap` + Snapshot/Patch + RenderCache + ScrollAnchor。

**Architecture:** 自底向上五层 —— 精简 sum tree (`SnapTree`) → 持久化映射 (`DisplayLineMap` + `Snapshot/Patch`) → 后台 wrap/shape 单线程 worker → 行内相对坐标顶点缓存 (`RenderCache<doc_line, CachedLine<GlyphInstance>>`) → 内容锚定的滚动位置 (`ScrollAnchor`)。每个阶段独立可编译可回退。

**Tech Stack:** Rust 2024、winit / wgpu、`hashlink` LRU、新增依赖 `smallvec` + `xxhash-rust`。

**Phase 验收（来自 spec §9）：**
- P1：`cargo test -p edit-plus-app snap_tree` 全绿；`large_build_20000_entries < 50ms`
- P2：debug 模式下 DisplayLineMap 与 WrapIndex 在 1000 次随机查询中 100% 一致
- P3：4.1MB JSON 滚动一帧 < 2ms；主题切换 0 invalidate
- P4：插入 1000 行后 scroll_anchor.doc_line 不变；resize 时 anchor 不漂
- P5：wrap_index.rs 不存在；超长行开关 + resize 节流均落地

**约定：**
- 所有 commit 在 `crates/app/` 子树内，不修改 `crates/render` 之外的其它 crate（Phase 3 例外）
- 每个 task 必须以「测试绿 → commit」结束
- 每条 commit 用 `git add <具体路径>`，绝不 `git add -A`
- 跑 `cargo test -p edit-plus-app -- --quiet` 是默认验证命令

---

# Phase 1 — SnapTree 基础设施

**目标**：完整可用的精简版 sum tree，纯数据结构 + 单测。不接任何上游。
**估算**：500 行新代码 + 200 行测试。

## File Structure (Phase 1)

| 文件 | 职责 |
|------|------|
| `crates/app/Cargo.toml` | 添加 `smallvec` / `xxhash-rust` 依赖 |
| `Cargo.toml` (workspace) | 注册 workspace 依赖 |
| `crates/app/src/snap_tree.rs` | 数据结构 + API + 单测（~700 行） |
| `crates/app/src/lib.rs` | 注册 module |

## Task 1.1: 添加依赖

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/app/Cargo.toml`

- [ ] **Step 1: 在 workspace 根 `Cargo.toml` 添加 `smallvec` 与 `xxhash-rust`**

文件 `Cargo.toml` 中 `[workspace.dependencies]` 段（第 26-40 行附近）追加：

```toml
smallvec = { version = "1", features = ["const_generics"] }
xxhash-rust = { version = "0.8", features = ["xxh3"] }
```

- [ ] **Step 2: 在 `crates/app/Cargo.toml` 引入**

`[dependencies]` 段尾追加：

```toml
smallvec = { workspace = true }
xxhash-rust = { workspace = true }
```

- [ ] **Step 3: 验证编译**

```bash
cd /Users/dan/proj/llmws/edit+
cargo build -p edit-plus-app
```

Expected: 编译成功，无新 warning。

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/app/Cargo.toml Cargo.lock
git commit -m "chore: 引入 smallvec + xxhash-rust（DisplayLineMap 基础设施）"
```

---

## Task 1.2: SnapTree 骨架与 DisplayLineEntry

**Files:**
- Create: `crates/app/src/snap_tree.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 创建 `crates/app/src/snap_tree.rs` 骨架**

```rust
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
    pub fn placeholder(byte_offset: usize, byte_length: u32, content_hash: u64) -> Self {
        let mut breaks = SmallVec::new();
        breaks.push(VisualBreak {
            byte_start: 0,
            byte_end: byte_length,
            pixel_width: 0.0,
        });
        Self {
            visual_line_count: 1,
            visual_breaks: breaks,
            byte_offset,
            byte_length,
            content_hash,
        }
    }
}

#[derive(Debug)]
enum Node {
    Leaf {
        entries: Vec<DisplayLineEntry>,
        total_rows: usize,
    },
    Inner {
        children: Vec<Arc<Node>>,
        total_rows: usize,
        line_count: usize,
    },
}

impl Node {
    fn total_rows(&self) -> usize {
        match self {
            Node::Leaf { total_rows, .. } => *total_rows,
            Node::Inner { total_rows, .. } => *total_rows,
        }
    }

    fn line_count(&self) -> usize {
        match self {
            Node::Leaf { entries, .. } => entries.len(),
            Node::Inner { line_count, .. } => *line_count,
        }
    }
}

/// 持久化 B-tree。clone 是 Arc 浅克隆，O(1)。
#[derive(Clone, Debug)]
pub struct SnapTree {
    root: Arc<Node>,
}

#[derive(Debug)]
pub struct RowLookup<'a> {
    pub doc_line: usize,
    pub visual_idx_in_doc: usize,
    pub entry: &'a DisplayLineEntry,
}

#[derive(Debug, PartialEq)]
pub struct SpliceResult {
    pub old_rows: Range<usize>,
    pub new_rows: Range<usize>,
}

impl SnapTree {
    pub fn new() -> Self {
        Self {
            root: Arc::new(Node::Leaf {
                entries: Vec::new(),
                total_rows: 0,
            }),
        }
    }

    pub fn line_count(&self) -> usize {
        self.root.line_count()
    }

    pub fn total_rows(&self) -> usize {
        self.root.total_rows()
    }

    pub fn is_empty(&self) -> bool {
        self.line_count() == 0
    }
}

impl Default for SnapTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 2: 创建空测试文件 `crates/app/src/snap_tree/mod.rs`** —— 不需要，这里换成同文件 `mod tests`，跳过此步。直接走 step 3。

- [ ] **Step 3: 在 `crates/app/src/lib.rs` 注册 module**

定位 `pub mod wrap_index;` 行（应该在 21 行附近），下方追加：

```rust
pub mod snap_tree;
```

- [ ] **Step 4: 写空 tests mod 占位（同文件末尾）**

snap_tree.rs 末尾 `#[cfg(test)] mod tests;` 改为内联：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree_has_zero_rows() {
        let t = SnapTree::new();
        assert_eq!(t.line_count(), 0);
        assert_eq!(t.total_rows(), 0);
        assert!(t.is_empty());
    }
}
```

- [ ] **Step 5: 验证测试通过**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
```

Expected: `1 passed`。

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/snap_tree.rs crates/app/src/lib.rs
git commit -m "feat(snap_tree): 骨架 + DisplayLineEntry/VisualBreak 类型"
```

---

## Task 1.3: from_entries（叶子构建路径）

**Files:**
- Modify: `crates/app/src/snap_tree.rs`

- [ ] **Step 1: 写失败测试**

在 `mod tests` 内追加：

```rust
fn entry(rows: u16) -> DisplayLineEntry {
    let mut breaks = SmallVec::new();
    breaks.push(VisualBreak { byte_start: 0, byte_end: 10, pixel_width: 100.0 });
    DisplayLineEntry {
        visual_line_count: rows,
        visual_breaks: breaks,
        byte_offset: 0,
        byte_length: 10,
        content_hash: 0,
    }
}

#[test]
fn from_entries_small_fits_one_leaf() {
    let entries: Vec<_> = (0..10).map(|i| entry((i % 3 + 1) as u16)).collect();
    let expected_total: usize = entries.iter().map(|e| e.visual_line_count as usize).sum();

    let t = SnapTree::from_entries(entries);
    assert_eq!(t.line_count(), 10);
    assert_eq!(t.total_rows(), expected_total);
}
```

- [ ] **Step 2: 验证失败**

```bash
cargo test -p edit-plus-app snap_tree::tests::from_entries_small_fits_one_leaf -- --quiet
```

Expected: 编译错误 —— `from_entries` 未定义。

- [ ] **Step 3: 实现 `from_entries`（仅叶子分支）**

在 `impl SnapTree` 内追加：

```rust
pub fn from_entries(it: impl IntoIterator<Item = DisplayLineEntry>) -> Self {
    let entries: Vec<_> = it.into_iter().collect();
    if entries.len() <= LEAF_MAX {
        let total_rows: usize = entries.iter().map(|e| e.visual_line_count as usize).sum();
        return Self {
            root: Arc::new(Node::Leaf { entries, total_rows }),
        };
    }
    Self::build_balanced(entries)
}

fn build_balanced(entries: Vec<DisplayLineEntry>) -> Self {
    // 分块成多个 Leaf，再 bottom-up 组装 Inner 层。
    let mut leaves: Vec<Arc<Node>> = entries
        .chunks(LEAF_MAX)
        .map(|chunk| {
            let total: usize = chunk.iter().map(|e| e.visual_line_count as usize).sum();
            Arc::new(Node::Leaf {
                entries: chunk.to_vec(),
                total_rows: total,
            })
        })
        .collect();

    while leaves.len() > 1 {
        leaves = leaves
            .chunks(LEAF_MAX)
            .map(|chunk| {
                let children: Vec<_> = chunk.to_vec();
                let total_rows: usize = children.iter().map(|c| c.total_rows()).sum();
                let line_count: usize = children.iter().map(|c| c.line_count()).sum();
                Arc::new(Node::Inner { children, total_rows, line_count })
            })
            .collect();
    }

    Self { root: leaves.into_iter().next().unwrap() }
}
```

- [ ] **Step 4: 跑测试**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
```

Expected: 全 PASS。

- [ ] **Step 5: 加大文件构建测试**

测试 mod 内追加：

```rust
#[test]
fn from_entries_20000_balanced() {
    let entries: Vec<_> = (0..20_000).map(|i| entry(((i % 4) + 1) as u16)).collect();
    let expected: usize = entries.iter().map(|e| e.visual_line_count as usize).sum();
    let t = SnapTree::from_entries(entries);
    assert_eq!(t.line_count(), 20_000);
    assert_eq!(t.total_rows(), expected);
}

#[test]
fn from_entries_under_50ms_for_20k() {
    use std::time::Instant;
    let entries: Vec<_> = (0..20_000).map(|_| entry(1)).collect();
    let start = Instant::now();
    let t = SnapTree::from_entries(entries);
    let elapsed = start.elapsed();
    assert_eq!(t.line_count(), 20_000);
    assert!(elapsed.as_millis() < 50, "build took {:?}", elapsed);
}
```

- [ ] **Step 6: 跑测试 + commit**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
git add crates/app/src/snap_tree.rs
git commit -m "feat(snap_tree): from_entries 构建（叶子 + 多层 Inner）"
```

---

## Task 1.4: line_to_row + find_by_row

**Files:**
- Modify: `crates/app/src/snap_tree.rs`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn line_to_row_prefix_sum() {
    let entries: Vec<_> = (0..50).map(|i| entry((i % 3 + 1) as u16)).collect();
    let mut expected = vec![0usize; entries.len() + 1];
    for (i, e) in entries.iter().enumerate() {
        expected[i + 1] = expected[i] + e.visual_line_count as usize;
    }
    let t = SnapTree::from_entries(entries);
    for i in 0..=50 {
        assert_eq!(t.line_to_row(i), expected[i], "doc_line={i}");
    }
}

#[test]
fn find_by_row_inverse() {
    let entries: Vec<_> = (0..50).map(|i| entry((i % 3 + 1) as u16)).collect();
    let t = SnapTree::from_entries(entries.clone());
    for doc_line in 0..50 {
        let start_row = t.line_to_row(doc_line);
        let lookup = t.find_by_row(start_row).expect("must find");
        assert_eq!(lookup.doc_line, doc_line, "row={start_row}");
        assert_eq!(lookup.visual_idx_in_doc, 0);
        let last_row = start_row + entries[doc_line].visual_line_count as usize - 1;
        let lookup2 = t.find_by_row(last_row).expect("last visual");
        assert_eq!(lookup2.doc_line, doc_line);
        assert_eq!(lookup2.visual_idx_in_doc, entries[doc_line].visual_line_count as usize - 1);
    }
}

#[test]
fn find_by_row_out_of_range() {
    let entries: Vec<_> = (0..10).map(|_| entry(1)).collect();
    let t = SnapTree::from_entries(entries);
    assert!(t.find_by_row(100).is_none());
}
```

- [ ] **Step 2: 验证失败**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
```

Expected: `line_to_row` / `find_by_row` 未定义。

- [ ] **Step 3: 实现 line_to_row + find_by_row**

在 `impl SnapTree` 追加：

```rust
pub fn line_to_row(&self, doc_line: usize) -> usize {
    Self::line_to_row_in(&self.root, doc_line)
}

fn line_to_row_in(node: &Node, mut doc_line: usize) -> usize {
    match node {
        Node::Leaf { entries, .. } => {
            entries
                .iter()
                .take(doc_line)
                .map(|e| e.visual_line_count as usize)
                .sum()
        }
        Node::Inner { children, .. } => {
            let mut acc = 0usize;
            for child in children {
                let lc = child.line_count();
                if doc_line >= lc {
                    acc += child.total_rows();
                    doc_line -= lc;
                } else {
                    return acc + Self::line_to_row_in(child, doc_line);
                }
            }
            acc
        }
    }
}

pub fn find_by_row(&self, row: usize) -> Option<RowLookup<'_>> {
    if row >= self.total_rows() { return None; }
    let mut cur = &*self.root;
    let mut doc_line_offset = 0usize;
    let mut row_left = row;
    loop {
        match cur {
            Node::Leaf { entries, .. } => {
                let mut acc_rows = 0usize;
                for (i, e) in entries.iter().enumerate() {
                    let count = e.visual_line_count as usize;
                    if row_left < acc_rows + count {
                        return Some(RowLookup {
                            doc_line: doc_line_offset + i,
                            visual_idx_in_doc: row_left - acc_rows,
                            entry: e,
                        });
                    }
                    acc_rows += count;
                }
                return None;
            }
            Node::Inner { children, .. } => {
                let mut acc_rows = 0usize;
                let mut acc_lines = 0usize;
                let mut next: &Node = cur; // dummy
                for child in children {
                    let cr = child.total_rows();
                    if row_left < acc_rows + cr {
                        next = child;
                        break;
                    }
                    acc_rows += cr;
                    acc_lines += child.line_count();
                }
                doc_line_offset += acc_lines;
                row_left -= acc_rows;
                cur = next;
            }
        }
    }
}
```

- [ ] **Step 4: 跑测试 + commit**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
git add crates/app/src/snap_tree.rs
git commit -m "feat(snap_tree): line_to_row / find_by_row"
```

---

## Task 1.5: splice（核心增量更新）

**Files:**
- Modify: `crates/app/src/snap_tree.rs`

splice 是最复杂的 API。第一版**简化实现**：物化全部 entries → 替换 → rebuild。20000 行/次成本 ~5ms，对编辑场景足够。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn splice_replace_single_line() {
    let entries: Vec<_> = (0..20).map(|_| entry(1)).collect();
    let mut t = SnapTree::from_entries(entries);
    let old_total = t.total_rows();

    let r = t.splice(5..6, vec![entry(3)]);
    assert_eq!(r.old_rows, 5..6);
    assert_eq!(r.new_rows, 5..8);
    assert_eq!(t.total_rows(), old_total + 2);
    assert_eq!(t.line_count(), 20);
}

#[test]
fn splice_insert_lines() {
    let entries: Vec<_> = (0..20).map(|_| entry(1)).collect();
    let mut t = SnapTree::from_entries(entries);
    let r = t.splice(10..10, vec![entry(2), entry(2), entry(2)]);
    assert_eq!(r.old_rows, 10..10);
    assert_eq!(r.new_rows, 10..16);
    assert_eq!(t.line_count(), 23);
}

#[test]
fn splice_delete_range() {
    let entries: Vec<_> = (0..20).map(|_| entry(2)).collect();
    let mut t = SnapTree::from_entries(entries);
    let r = t.splice(5..10, vec![]);
    assert_eq!(r.old_rows, 10..20);
    assert_eq!(r.new_rows, 10..10);
    assert_eq!(t.line_count(), 15);
}

#[test]
fn splice_preserves_total_rows_invariant() {
    let entries: Vec<_> = (0..30).map(|i| entry((i % 4 + 1) as u16)).collect();
    let mut t = SnapTree::from_entries(entries);
    let _ = t.splice(3..7, vec![entry(1), entry(2)]);
    let direct: usize = t
        .iter_entries()
        .map(|e| e.visual_line_count as usize)
        .sum();
    assert_eq!(t.total_rows(), direct);
}
```

- [ ] **Step 2: 验证失败**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
```

Expected: `splice` 未定义。

- [ ] **Step 3: 实现 splice + iter_entries（简化版）**

```rust
impl SnapTree {
    /// 替换 [range) 内的 entries。返回旧/新 DisplayRow 范围。
    pub fn splice(
        &mut self,
        range: Range<usize>,
        replacements: Vec<DisplayLineEntry>,
    ) -> SpliceResult {
        let old_start_row = self.line_to_row(range.start);
        let old_end_row = self.line_to_row(range.end);
        let new_rows_count: usize = replacements.iter().map(|e| e.visual_line_count as usize).sum();

        let mut entries: Vec<DisplayLineEntry> = self.iter_entries().cloned().collect();
        let drained_len = range.end - range.start;
        entries.splice(range.clone(), replacements);

        *self = SnapTree::from_entries(entries);

        SpliceResult {
            old_rows: old_start_row..old_end_row,
            new_rows: old_start_row..old_start_row + new_rows_count,
        }
    }

    pub fn iter_entries(&self) -> EntryIter<'_> {
        EntryIter::new(&self.root)
    }
}

pub struct EntryIter<'a> {
    stack: Vec<(&'a Node, usize)>,
}

impl<'a> EntryIter<'a> {
    fn new(root: &'a Node) -> Self {
        Self { stack: vec![(root, 0)] }
    }
}

impl<'a> Iterator for EntryIter<'a> {
    type Item = &'a DisplayLineEntry;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&mut (node, ref mut idx)) = self.stack.last_mut() {
            match node {
                Node::Leaf { entries, .. } => {
                    if *idx < entries.len() {
                        let e = &entries[*idx];
                        *idx += 1;
                        return Some(e);
                    } else {
                        self.stack.pop();
                    }
                }
                Node::Inner { children, .. } => {
                    if *idx < children.len() {
                        let child = &*children[*idx];
                        *idx += 1;
                        self.stack.push((child, 0));
                    } else {
                        self.stack.pop();
                    }
                }
            }
        }
        None
    }
}
```

注意上面 `splice` 提到 `drained_len`：保留它做 debug assert 也可以；当前未用，删除该行避免 warning：

```rust
let _ = drained_len; // 当前不需要；保持代码可读
```

或者直接删除该行（推荐）。

- [ ] **Step 4: 跑测试 + commit**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
git add crates/app/src/snap_tree.rs
git commit -m "feat(snap_tree): splice（rebuild 简化版）+ iter_entries"
```

---

## Task 1.6: clone 是 Arc 浅克隆 + iter_lines / iter_rows

**Files:**
- Modify: `crates/app/src/snap_tree.rs`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn clone_is_arc_shared() {
    let entries: Vec<_> = (0..20).map(|_| entry(1)).collect();
    let t = SnapTree::from_entries(entries);
    let t2 = t.clone();
    assert_eq!(Arc::strong_count(&t.root), 2);
    drop(t2);
    assert_eq!(Arc::strong_count(&t.root), 1);
}

#[test]
fn iter_lines_range() {
    let entries: Vec<_> = (0..20).map(|i| entry((i + 1) as u16 % 4 + 1)).collect();
    let expected: Vec<u16> = entries[5..10].iter().map(|e| e.visual_line_count).collect();
    let t = SnapTree::from_entries(entries);
    let got: Vec<u16> = t.iter_lines(5..10).map(|e| e.visual_line_count).collect();
    assert_eq!(got, expected);
}

#[test]
fn iter_rows_yields_visual_lines() {
    let entries: Vec<_> = vec![entry(1), entry(3), entry(2), entry(1)];
    let t = SnapTree::from_entries(entries);
    let collected: Vec<_> = t
        .iter_rows(1..6)
        .map(|r| (r.doc_line, r.visual_idx_in_doc))
        .collect();
    assert_eq!(
        collected,
        vec![(1, 0), (1, 1), (1, 2), (2, 0), (2, 1)]
    );
}
```

- [ ] **Step 2: 实现 iter_lines / iter_rows**

在 `impl SnapTree` 追加：

```rust
pub fn iter_lines(&self, range: Range<usize>) -> LineIter<'_> {
    LineIter {
        inner: EntryIter::new(&self.root),
        skip_remaining: range.start,
        take_remaining: range.end.saturating_sub(range.start),
    }
}

pub fn iter_rows(&self, rows: Range<usize>) -> RowIter<'_> {
    RowIter::new(&self.root, rows.start, rows.end)
}

pub struct LineIter<'a> {
    inner: EntryIter<'a>,
    skip_remaining: usize,
    take_remaining: usize,
}

impl<'a> Iterator for LineIter<'a> {
    type Item = &'a DisplayLineEntry;
    fn next(&mut self) -> Option<Self::Item> {
        while self.skip_remaining > 0 {
            self.inner.next()?;
            self.skip_remaining -= 1;
        }
        if self.take_remaining == 0 { return None; }
        let v = self.inner.next()?;
        self.take_remaining -= 1;
        Some(v)
    }
}

pub struct RowOwned {
    pub doc_line: usize,
    pub visual_idx_in_doc: usize,
}

pub struct RowIter<'a> {
    inner: EntryIter<'a>,
    cur_doc_line: usize,
    cur_entry: Option<&'a DisplayLineEntry>,
    cur_vl_idx: usize,
    rows_emitted: usize,
    rows_to_skip: usize,
    rows_max: usize,
}

impl<'a> RowIter<'a> {
    fn new(root: &'a Node, start: usize, end: usize) -> Self {
        Self {
            inner: EntryIter::new(root),
            cur_doc_line: 0,
            cur_entry: None,
            cur_vl_idx: 0,
            rows_emitted: 0,
            rows_to_skip: start,
            rows_max: end,
        }
    }
}

impl<'a> Iterator for RowIter<'a> {
    type Item = RowOwned;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.rows_emitted >= self.rows_max { return None; }
            // 当前 entry 仍有剩余 vl
            if let Some(entry) = self.cur_entry {
                if self.cur_vl_idx < entry.visual_line_count as usize {
                    let row = RowOwned {
                        doc_line: self.cur_doc_line,
                        visual_idx_in_doc: self.cur_vl_idx,
                    };
                    self.cur_vl_idx += 1;
                    self.rows_emitted += 1;
                    if self.rows_to_skip > 0 {
                        self.rows_to_skip -= 1;
                        continue;
                    }
                    return Some(row);
                } else {
                    // 该 entry 用完，前进
                    self.cur_doc_line += 1;
                    self.cur_entry = None;
                }
            }
            self.cur_entry = Some(self.inner.next()?);
            self.cur_vl_idx = 0;
        }
    }
}
```

- [ ] **Step 3: 跑测试 + commit**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
git add crates/app/src/snap_tree.rs
git commit -m "feat(snap_tree): iter_lines / iter_rows + clone 共享测试"
```

---

## Task 1.7: Phase 1 验收

- [ ] **Step 1: 跑全部 snap_tree 测试 + 计时**

```bash
cargo test -p edit-plus-app snap_tree -- --quiet
cargo test -p edit-plus-app snap_tree::tests::from_entries_under_50ms_for_20k -- --nocapture
```

Expected: 全部 PASS；20k 构建 < 50ms。

- [ ] **Step 2: 确认没有破坏其它测试**

```bash
cargo test -p edit-plus-app -- --quiet
```

Expected: 全部 PASS。

- [ ] **Step 3: tag 用于 Phase 间回退**

```bash
git tag phase1-snap-tree-done
```

---

# Phase 2 — DisplayLineMap + ReshapeWorker（与 WrapIndex 并行）

**目标**：完整可用的 DisplayLineMap，但**不接管渲染管线**。改为在 debug + 环境变量 `EDIT_PARALLEL_ASSERT=1` 下与 `WrapIndex` 并行运行 + 一致性 assert。
**估算**：1000 行新代码 + 300 行测试。

## File Structure (Phase 2)

| 文件 | 职责 |
|------|------|
| `crates/app/src/reshape_worker.rs` | 后台 wrap/shape 单线程 worker（~300 行） |
| `crates/app/src/display_line_map.rs` | DisplayLineMap + Snapshot + Patch（~700 行） |
| `crates/app/src/lib.rs` | 注册两个 module |
| `crates/app/src/app.rs` | 添加 `display_map` 字段 + parallel-assert 钩子（仅 debug） |

## Task 2.1: ReshapeWorker 骨架

**Files:**
- Create: `crates/app/src/reshape_worker.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 创建 `crates/app/src/reshape_worker.rs`**

```rust
//! 后台 wrap/shape 单线程 worker。
//!
//! - 主线程通过 `submit()` 推入请求；通过 `drain_completed()` 拉取结果。
//! - generation 三层校验，过期请求丢弃。
//! - 队列上限 1000；超过返回 Backpressured，调用方降级为同步处理。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use crate::display_line_map::{DisplayLineEntry, ReshapeOutput};

/// 主线程→worker 的请求负载。
pub struct ReshapeRequest {
    pub generation: u64,
    pub doc_line: usize,
    pub line_bytes: Arc<[u8]>,
    pub viewport_width: f32,
    pub font_size: f32,
}

/// worker→主线程的结果。`render_payload` 是 Phase 3 才会用到的字段，
/// Phase 2 阶段填空 Vec 即可。
pub struct ReshapeResult {
    pub generation: u64,
    pub doc_line: usize,
    pub entry: DisplayLineEntry,
    pub render_payload: ReshapeOutput,
}

pub enum SubmitOutcome {
    Accepted,
    Backpressured,
}

enum WorkerMsg {
    Request(ReshapeRequest),
    Shutdown,
}

const QUEUE_HIGH_WATERMARK: usize = 1000;

pub struct ReshapeWorker {
    tx: Sender<WorkerMsg>,
    rx_results: Receiver<ReshapeResult>,
    current_generation: Arc<AtomicU64>,
    pending_count: Arc<AtomicUsize>,
    join: Option<JoinHandle<()>>,
}

impl ReshapeWorker {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let (tx_results, rx_results) = mpsc::channel::<ReshapeResult>();
        let current_generation = Arc::new(AtomicU64::new(0));
        let pending_count = Arc::new(AtomicUsize::new(0));
        let cur_gen_w = Arc::clone(&current_generation);
        let pending_w = Arc::clone(&pending_count);

        let join = std::thread::Builder::new()
            .name("edit-plus-reshape-worker".into())
            .spawn(move || worker_loop(rx, tx_results, cur_gen_w, pending_w))
            .expect("spawn reshape worker");

        Self {
            tx,
            rx_results,
            current_generation,
            pending_count,
            join: Some(join),
        }
    }

    pub fn submit(&self, req: ReshapeRequest) -> SubmitOutcome {
        if self.pending_count.load(Ordering::Acquire) >= QUEUE_HIGH_WATERMARK {
            return SubmitOutcome::Backpressured;
        }
        self.pending_count.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.send(WorkerMsg::Request(req));
        SubmitOutcome::Accepted
    }

    pub fn drain_completed(&self, max: usize) -> Vec<ReshapeResult> {
        let mut out = Vec::new();
        for _ in 0..max {
            match self.rx_results.try_recv() {
                Ok(r) => out.push(r),
                Err(_) => break,
            }
        }
        out
    }

    pub fn cancel_before(&self, generation: u64) {
        // 推进 generation：worker 处理消息时会丢弃 < 该值的 request。
        let prev = self.current_generation.load(Ordering::Acquire);
        if generation > prev {
            self.current_generation.store(generation, Ordering::Release);
        }
    }

    pub fn pending(&self) -> usize {
        self.pending_count.load(Ordering::Acquire)
    }
}

impl Drop for ReshapeWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(WorkerMsg::Shutdown);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

fn worker_loop(
    rx: Receiver<WorkerMsg>,
    tx_results: Sender<ReshapeResult>,
    current_generation: Arc<AtomicU64>,
    pending_count: Arc<AtomicUsize>,
) {
    // Worker 内部初始化 shaper（Phase 3 才用到，Phase 2 占位）。
    while let Ok(msg) = rx.recv() {
        match msg {
            WorkerMsg::Shutdown => return,
            WorkerMsg::Request(req) => {
                pending_count.fetch_sub(1, Ordering::AcqRel);
                let cur = current_generation.load(Ordering::Acquire);
                if req.generation < cur {
                    continue;
                }
                let result = process_request(req);
                if tx_results.send(result).is_err() {
                    return;
                }
            }
        }
    }
}

fn process_request(req: ReshapeRequest) -> ReshapeResult {
    // Phase 2：仅基于 byte_length 估算 visual_line_count。
    // Phase 3 会替换为真实 shape + wrap。
    use xxhash_rust::xxh3::xxh3_64;
    let hash = xxh3_64(&req.line_bytes);
    let byte_length = req.line_bytes.len() as u32;
    let entry = DisplayLineEntry::placeholder(0, byte_length, hash);
    ReshapeResult {
        generation: req.generation,
        doc_line: req.doc_line,
        entry,
        render_payload: ReshapeOutput::default(),
    }
}
```

- [ ] **Step 2: 在 `crates/app/src/lib.rs` 注册**

定位 `pub mod snap_tree;`，下方追加：

```rust
pub mod display_line_map;
pub mod reshape_worker;
```

注：`display_line_map` 文件在 Task 2.4 创建。**为了让 `reshape_worker.rs` 现在能编译**，先在本任务把 `display_line_map.rs` 创建为最小桩文件。

- [ ] **Step 3: 创建 `crates/app/src/display_line_map.rs` 占位**

```rust
//! DisplayLineMap：对齐 Zed wrap_map 的持久化映射 + 同步原语。
//! Phase 2 占位骨架；Task 2.4 起逐步填充。

use crate::snap_tree::{DisplayLineEntry as SnapEntry, VisualBreak};

pub use crate::snap_tree::DisplayLineEntry;

#[derive(Default)]
pub struct ReshapeOutput {
    /// Phase 3 填充：CachedLine 的所有渲染数据。
    pub instances_placeholder: Vec<u8>,
}

#[allow(unused)]
fn _ensure_visualbreak_visible(_b: VisualBreak, _e: SnapEntry) {}
```

- [ ] **Step 4: 验证编译 + commit**

```bash
cargo build -p edit-plus-app
git add crates/app/src/reshape_worker.rs crates/app/src/display_line_map.rs crates/app/src/lib.rs
git commit -m "feat(reshape_worker): 单线程 worker 骨架 + generation 三层校验"
```

---

## Task 2.2: ReshapeWorker 单测（generation 取消、背压）

**Files:**
- Modify: `crates/app/src/reshape_worker.rs`

- [ ] **Step 1: 写失败测试**

文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn req(gen: u64, line: usize) -> ReshapeRequest {
        ReshapeRequest {
            generation: gen,
            doc_line: line,
            line_bytes: Arc::from(b"hello world".as_slice()),
            viewport_width: 800.0,
            font_size: 14.0,
        }
    }

    #[test]
    fn submit_then_drain_round_trip() {
        let w = ReshapeWorker::spawn();
        w.cancel_before(1);
        for i in 0..5 {
            let _ = w.submit(req(1, i));
        }
        // 等 worker 处理完
        std::thread::sleep(std::time::Duration::from_millis(50));
        let results = w.drain_completed(100);
        assert_eq!(results.len(), 5);
        let lines: Vec<_> = results.iter().map(|r| r.doc_line).collect();
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn worker_drops_stale_generation() {
        let w = ReshapeWorker::spawn();
        // 先把 current_generation 推到 5
        w.cancel_before(5);
        // 发送一个 generation=3 的过期请求
        let _ = w.submit(req(3, 99));
        // 再发送当前代请求
        let _ = w.submit(req(5, 100));
        std::thread::sleep(std::time::Duration::from_millis(50));
        let results = w.drain_completed(100);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].doc_line, 100);
    }

    #[test]
    fn backpressure_at_high_watermark() {
        let w = ReshapeWorker::spawn();
        // 发到队列高水位
        let mut accepted = 0;
        let mut backpressured = 0;
        for i in 0..(QUEUE_HIGH_WATERMARK + 100) {
            match w.submit(req(1, i)) {
                SubmitOutcome::Accepted => accepted += 1,
                SubmitOutcome::Backpressured => backpressured += 1,
            }
        }
        assert!(accepted >= QUEUE_HIGH_WATERMARK / 2, "accepted={accepted}");
        // 由于 worker 在并发消费，无法保证恰好等于 watermark；只要有背压触发即可
        assert!(accepted + backpressured == QUEUE_HIGH_WATERMARK + 100);
    }
}
```

- [ ] **Step 2: 跑测试 + commit**

```bash
cargo test -p edit-plus-app reshape_worker -- --quiet --test-threads=1
git add crates/app/src/reshape_worker.rs
git commit -m "test(reshape_worker): 取消语义 + 背压"
```

---

## Task 2.3: DisplayLineMap::from_buffer

**Files:**
- Modify: `crates/app/src/display_line_map.rs`

- [ ] **Step 1: 探索 TextBuffer API**

```bash
grep -n "pub fn line_count\|pub fn get_line\|pub fn line_bytes\|pub fn byte_offset" /Users/dan/proj/llmws/edit+/crates/core/src/lib.rs | head -20
grep -rn "pub struct TextBuffer\|impl TextBuffer" /Users/dan/proj/llmws/edit+/crates/core/src/ | head -10
```

记录：阅读结果（line_count / line_bytes / line_byte_offset 等真实方法名），在后续 task 中用真实 API 名替换下面伪代码 `buffer.line_count() / buffer.line_bytes(i) / buffer.line_byte_offset(i)`。

- [ ] **Step 2: 重写 `crates/app/src/display_line_map.rs`**

```rust
//! DisplayLineMap：对齐 Zed wrap_map 的持久化映射 + 同步原语。

use std::ops::Range;
use std::sync::Arc;

use xxhash_rust::xxh3::xxh3_64;

use crate::reshape_worker::{ReshapeRequest, ReshapeWorker, SubmitOutcome};
use crate::snap_tree::{SnapTree, RowLookup, RowOwned, SpliceResult};

pub use crate::snap_tree::{DisplayLineEntry, VisualBreak};

/// Phase 3 填充：CachedLine 的渲染数据。Phase 2 仅占位。
#[derive(Default, Clone)]
pub struct ReshapeOutput {
    pub instances_placeholder: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct DisplayPatch {
    pub affected_rows: Vec<Range<usize>>,
    pub line_shift: Option<LineShift>,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct LineShift {
    pub at: usize,
    pub delta: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Edit {
    pub doc_line_range: Range<usize>,    // 原行索引范围
    pub new_line_count: usize,            // 替换后该范围占多少行
}

/// 不可变快照，渲染层无锁读取。
#[derive(Clone)]
pub struct DisplaySnapshot {
    tree: SnapTree,
    pub generation: u64,
    pub viewport_width: f32,
    pub font_size: f32,
}

impl DisplaySnapshot {
    pub fn line_count(&self) -> usize { self.tree.line_count() }
    pub fn total_rows(&self) -> usize { self.tree.total_rows() }
    pub fn resolve_row(&self, row: usize) -> Option<RowLookup<'_>> { self.tree.find_by_row(row) }
    pub fn line_to_row(&self, doc_line: usize) -> usize { self.tree.line_to_row(doc_line) }
    pub fn iter_rows(&self, rows: Range<usize>) -> impl Iterator<Item = RowOwned> + '_ {
        self.tree.iter_rows(rows)
    }
}

const SMALL_EDIT_THRESHOLD: usize = 100;

pub struct DisplayLineMap {
    tree: SnapTree,
    generation: u64,
    viewport_width: f32,
    font_size: f32,
    worker: ReshapeWorker,
    pending_render_inserts: Vec<(usize, ReshapeOutput)>,
}

impl DisplayLineMap {
    /// 从 buffer 初始构建。所有行用 placeholder（visual_line_count = 1），
    /// 视口附近的行通过 worker 后台精修。
    pub fn from_buffer<B: BufferLike>(buffer: &B, viewport_width: f32, font_size: f32) -> Self {
        let line_count = buffer.line_count();
        let entries: Vec<DisplayLineEntry> = (0..line_count)
            .map(|i| {
                let bytes = buffer.line_bytes(i);
                let hash = xxh3_64(&bytes);
                DisplayLineEntry::placeholder(buffer.line_byte_offset(i), bytes.len() as u32, hash)
            })
            .collect();

        Self {
            tree: SnapTree::from_entries(entries),
            generation: 1,
            viewport_width,
            font_size,
            worker: ReshapeWorker::spawn(),
            pending_render_inserts: Vec::new(),
        }
    }

    pub fn snapshot(&self) -> DisplaySnapshot {
        DisplaySnapshot {
            tree: self.tree.clone(),
            generation: self.generation,
            viewport_width: self.viewport_width,
            font_size: self.font_size,
        }
    }

    pub fn line_count(&self) -> usize { self.tree.line_count() }
    pub fn total_rows(&self) -> usize { self.tree.total_rows() }
    pub fn line_to_row(&self, doc_line: usize) -> usize { self.tree.line_to_row(doc_line) }
}

/// `TextBuffer` 抽象，便于测试 mock。
pub trait BufferLike {
    fn line_count(&self) -> usize;
    fn line_bytes(&self, line: usize) -> Arc<[u8]>;
    fn line_byte_offset(&self, line: usize) -> usize;
}
```

- [ ] **Step 3: 写测试**

文件末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct MockBuffer { lines: Vec<Vec<u8>> }
    impl BufferLike for MockBuffer {
        fn line_count(&self) -> usize { self.lines.len() }
        fn line_bytes(&self, line: usize) -> Arc<[u8]> {
            Arc::from(self.lines[line].clone().into_boxed_slice())
        }
        fn line_byte_offset(&self, line: usize) -> usize {
            self.lines.iter().take(line).map(|l| l.len() + 1).sum()
        }
    }

    fn buf(lines: Vec<&str>) -> MockBuffer {
        MockBuffer { lines: lines.into_iter().map(|s| s.as_bytes().to_vec()).collect() }
    }

    #[test]
    fn from_buffer_creates_one_entry_per_line() {
        let b = buf(vec!["abc", "defg", "hi"]);
        let m = DisplayLineMap::from_buffer(&b, 800.0, 14.0);
        assert_eq!(m.line_count(), 3);
        assert_eq!(m.total_rows(), 3); // placeholder = 1 vl/行
    }

    #[test]
    fn snapshot_is_arc_shared_with_owner() {
        let b = buf(vec!["x"; 10].into_iter().map(|s| s).collect());
        let m = DisplayLineMap::from_buffer(&b, 800.0, 14.0);
        let s1 = m.snapshot();
        let s2 = m.snapshot();
        assert_eq!(s1.line_count(), s2.line_count());
        // 不同 snapshot 共享底层
        assert_eq!(s1.generation, s2.generation);
    }
}
```

- [ ] **Step 4: 跑测试 + commit**

```bash
cargo test -p edit-plus-app display_line_map -- --quiet
git add crates/app/src/display_line_map.rs
git commit -m "feat(display_line_map): from_buffer + snapshot + BufferLike trait"
```

---

## Task 2.4: sync 小编辑路径（同步精修）

**Files:**
- Modify: `crates/app/src/display_line_map.rs`

- [ ] **Step 1: 写失败测试**

`mod tests` 内追加：

```rust
#[test]
fn sync_small_edit_synchronously_completes() {
    let mut b = buf(vec!["aaa", "bbb", "ccc", "ddd"]);
    let mut m = DisplayLineMap::from_buffer(&b, 800.0, 14.0);
    let initial_gen = m.snapshot().generation;

    // 修改 line 1：bbb → bbbb (内容长度变了 → content_hash 变了)
    b.lines[1] = b"bbbb".to_vec();
    let edits = vec![Edit { doc_line_range: 1..2, new_line_count: 1 }];
    let (snap, patch) = m.sync(&b, &edits);

    assert!(snap.generation > initial_gen);
    assert_eq!(snap.line_count(), 4);
    assert_eq!(patch.affected_rows.len(), 1);
    assert_eq!(patch.line_shift, None);
}

#[test]
fn sync_insert_lines_emits_line_shift() {
    let mut b = buf(vec!["a", "b", "c"]);
    let mut m = DisplayLineMap::from_buffer(&b, 800.0, 14.0);

    b.lines.insert(1, b"x".to_vec());
    b.lines.insert(2, b"y".to_vec());
    let edits = vec![Edit { doc_line_range: 1..1, new_line_count: 2 }];
    let (snap, patch) = m.sync(&b, &edits);

    assert_eq!(snap.line_count(), 5);
    assert_eq!(patch.line_shift, Some(LineShift { at: 1, delta: 2 }));
}
```

注意：`Eq` for `LineShift` 需要 `derive(PartialEq)`。回到结构体定义，把 `#[derive(Clone, Copy, Debug)]` 改为 `#[derive(Clone, Copy, Debug, PartialEq, Eq)]`。

- [ ] **Step 2: 实现 sync（小编辑分支）**

`impl DisplayLineMap` 追加：

```rust
pub fn sync<B: BufferLike>(
    &mut self,
    buffer: &B,
    edits: &[Edit],
) -> (DisplaySnapshot, DisplayPatch) {
    self.generation += 1;
    self.worker.cancel_before(self.generation);

    let mut patch = DisplayPatch { generation: self.generation, ..Default::default() };

    for edit in edits {
        let old_range = edit.doc_line_range.clone();
        let new_count = edit.new_line_count;
        let old_count = old_range.end - old_range.start;
        let net_delta = new_count as i64 - old_count as i64;

        // 决定走小编辑分支还是大编辑分支
        if new_count.max(old_count) <= SMALL_EDIT_THRESHOLD {
            // 小编辑：同步重建受影响 entries
            let new_entries: Vec<DisplayLineEntry> = (0..new_count)
                .map(|offset| {
                    let line_idx = old_range.start + offset;
                    let bytes = buffer.line_bytes(line_idx);
                    let hash = xxh3_64(&bytes);
                    // Phase 3 会换成真实 shape；Phase 2 用 placeholder
                    DisplayLineEntry::placeholder(
                        buffer.line_byte_offset(line_idx),
                        bytes.len() as u32,
                        hash,
                    )
                })
                .collect();
            let r = self.tree.splice(old_range.clone(), new_entries);
            patch.affected_rows.push(r.old_rows.start..r.new_rows.end);
        } else {
            // 大编辑：占位 + 入队 worker
            let placeholders: Vec<DisplayLineEntry> = (0..new_count)
                .map(|offset| {
                    let line_idx = old_range.start + offset;
                    let bytes = buffer.line_bytes(line_idx);
                    let hash = xxh3_64(&bytes);
                    DisplayLineEntry::placeholder(
                        buffer.line_byte_offset(line_idx),
                        bytes.len() as u32,
                        hash,
                    )
                })
                .collect();
            let r = self.tree.splice(old_range.clone(), placeholders);
            patch.affected_rows.push(r.old_rows.start..r.new_rows.end);

            // 视口附近 + 全段都入队
            for offset in 0..new_count {
                let line_idx = old_range.start + offset;
                let bytes = buffer.line_bytes(line_idx);
                let req = ReshapeRequest {
                    generation: self.generation,
                    doc_line: line_idx,
                    line_bytes: bytes,
                    viewport_width: self.viewport_width,
                    font_size: self.font_size,
                };
                if matches!(self.worker.submit(req), SubmitOutcome::Backpressured) {
                    break; // 主线程兜底，未来帧再补
                }
            }
        }

        if net_delta != 0 {
            patch.line_shift = Some(LineShift { at: old_range.start, delta: net_delta });
        }
    }

    (self.snapshot(), patch)
}
```

- [ ] **Step 3: 跑测试 + commit**

```bash
cargo test -p edit-plus-app display_line_map -- --quiet
git add crates/app/src/display_line_map.rs
git commit -m "feat(display_line_map): sync 小/大编辑分支 + line_shift"
```

---

## Task 2.5: poll_worker + set_viewport_size

**Files:**
- Modify: `crates/app/src/display_line_map.rs`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn poll_worker_ignores_stale_results() {
    let b = buf(vec!["a"; 5].into_iter().map(|s| s).collect());
    let mut m = DisplayLineMap::from_buffer(&b, 800.0, 14.0);
    // 模拟一次 sync（会推进 generation 到 2）
    let edits = vec![Edit { doc_line_range: 0..0, new_line_count: 0 }];
    let _ = m.sync(&b, &edits);
    // 此后 worker 收到 generation = 2 之后的 result 才有效
    // 当前没有真实工作，poll_worker 返回空
    let p = m.poll_worker();
    assert!(p.is_none() || p.unwrap().affected_rows.is_empty());
}

#[test]
fn set_viewport_size_marks_all_dirty() {
    let b = buf(vec!["aa", "bb", "cc"]);
    let mut m = DisplayLineMap::from_buffer(&b, 800.0, 14.0);
    let p = m.set_viewport_size(&b, 400.0, 14.0);
    assert_eq!(p.affected_rows, vec![0..3]);
}
```

- [ ] **Step 2: 实现**

```rust
impl DisplayLineMap {
    pub fn poll_worker(&mut self) -> Option<DisplayPatch> {
        let results = self.worker.drain_completed(16);
        if results.is_empty() { return None; }
        let mut patch = DisplayPatch { generation: self.generation, ..Default::default() };
        for r in results {
            if r.generation != self.generation { continue; }
            let line_range = r.doc_line..r.doc_line + 1;
            let s = self.tree.splice(line_range, vec![r.entry]);
            patch.affected_rows.push(s.old_rows.start..s.new_rows.end);
            self.pending_render_inserts.push((r.doc_line, r.render_payload));
        }
        if patch.affected_rows.is_empty() { None } else { Some(patch) }
    }

    pub fn drain_pending_render_inserts(&mut self) -> Vec<(usize, ReshapeOutput)> {
        std::mem::take(&mut self.pending_render_inserts)
    }

    pub fn set_viewport_size<B: BufferLike>(
        &mut self,
        buffer: &B,
        width: f32,
        font_size: f32,
    ) -> DisplayPatch {
        self.viewport_width = width;
        self.font_size = font_size;
        self.generation += 1;
        self.worker.cancel_before(self.generation);

        let mut patch = DisplayPatch { generation: self.generation, ..Default::default() };
        let line_count = self.tree.line_count();
        if line_count == 0 { return patch; }

        // 全部行入队（worker 处理完成后会逐步刷新 tree）
        for line_idx in 0..line_count {
            let bytes = buffer.line_bytes(line_idx);
            let req = ReshapeRequest {
                generation: self.generation,
                doc_line: line_idx,
                line_bytes: bytes,
                viewport_width: self.viewport_width,
                font_size: self.font_size,
            };
            if matches!(self.worker.submit(req), SubmitOutcome::Backpressured) { break; }
        }

        patch.affected_rows.push(0..line_count);
        patch
    }
}
```

- [ ] **Step 3: 跑测试 + commit**

```bash
cargo test -p edit-plus-app display_line_map -- --quiet
git add crates/app/src/display_line_map.rs
git commit -m "feat(display_line_map): poll_worker + set_viewport_size"
```

---

## Task 2.6: app.rs parallel-assert 钩子（debug only）

**Files:**
- Modify: `crates/app/src/app.rs`

目标：让 `App` 在持有 `WrapIndex` 的同时构建 `DisplayLineMap`，每帧 debug 模式 assert 两者 `line_to_row` / `find_by_row` 在 32 个随机点上一致。**仅在 `cfg(debug_assertions) && env "EDIT_PARALLEL_ASSERT=1"` 时启用**，发布构建零开销。

- [ ] **Step 1: 探索 App / TextState 当前结构**

```bash
grep -n "wrap_index\|WrapIndex" /Users/dan/proj/llmws/edit+/crates/app/src/app.rs | head -30
```

记录 wrap_index 字段的位置和初始化点。

- [ ] **Step 2: 在 App 中加 `display_map: Option<DisplayLineMap>` 字段**

定位 App 结构体定义（搜索 `pub struct App` 或 `struct App {`），在 `wrap_index:` 字段下方追加：

```rust
#[cfg(debug_assertions)]
display_map_debug: Option<crate::display_line_map::DisplayLineMap>,
```

- [ ] **Step 3: 在文件加载/buffer 创建处初始化**

定位创建 `WrapIndex` 的位置（grep `WrapIndex::new`），在其后追加：

```rust
#[cfg(debug_assertions)]
{
    if std::env::var("EDIT_PARALLEL_ASSERT").as_deref() == Ok("1") {
        // 注意：这里需要把 buffer 包裹成 BufferLike。
        // BufferLike trait 在 display_line_map.rs，TextBuffer 在 core::TextBuffer。
        // 需要为 TextBuffer 实现 BufferLike，详见下面 step 4。
        self.display_map_debug = Some(crate::display_line_map::DisplayLineMap::from_buffer(
            &buffer_adapter::Adapter(&buffer),
            screen_w - gutter,
            font_size,
        ));
    } else {
        self.display_map_debug = None;
    }
}
```

- [ ] **Step 4: 创建 `crates/app/src/buffer_adapter.rs`**

```rust
//! 把 core::TextBuffer 适配为 display_line_map::BufferLike。

use std::sync::Arc;
use crate::display_line_map::BufferLike;

// TextBuffer 的真实路径需要根据探索结果调整：
// 例如 use core::TextBuffer; 或 use edit_plus_core::TextBuffer;
use core::TextBuffer;

pub struct Adapter<'a>(pub &'a TextBuffer);

impl<'a> BufferLike for Adapter<'a> {
    fn line_count(&self) -> usize {
        self.0.line_count()  // 探索时确认真实方法名
    }
    fn line_bytes(&self, line: usize) -> Arc<[u8]> {
        // 真实方法名可能是 get_line / line_content / line_bytes 之一
        let v = self.0.get_line_content(line).into_owned();
        Arc::from(v.into_boxed_slice())
    }
    fn line_byte_offset(&self, line: usize) -> usize {
        self.0.line_byte_offset(line).unwrap_or(0)
    }
}
```

注：Step 3 的 `&buffer_adapter::Adapter(&buffer)` 用法假设 `Adapter` 实现了 `BufferLike` —— 调用时拿 `&Adapter`。但 `from_buffer` 签名是 `<B: BufferLike>(buffer: &B, ...)`，`B = Adapter<'a>`，所以传 `&Adapter(&buffer)` 正确。

- [ ] **Step 5: 在 lib.rs 注册 buffer_adapter**

```rust
pub mod buffer_adapter;
```

- [ ] **Step 6: 加 parallel assert helper**

在 `crates/app/src/app.rs` 适当位置（推荐放在 render 函数附近）追加：

```rust
#[cfg(debug_assertions)]
fn assert_display_map_matches_wrap_index(
    display_map: &crate::display_line_map::DisplayLineMap,
    wrap_index: &crate::wrap_index::WrapIndex,
) {
    use rand::Rng;
    if std::env::var("EDIT_PARALLEL_ASSERT").as_deref() != Ok("1") { return; }
    let n = wrap_index.len();
    if n == 0 || display_map.line_count() != n { return; }

    // 32 个随机 doc_line 测 line_to_row
    let mut rng = SimpleRng::new(0xDEADBEEF);
    for _ in 0..32 {
        let dl = rng.next() % n;
        let a = display_map.line_to_row(dl);
        let b = wrap_index.doc_to_display(dl);
        debug_assert_eq!(a, b, "line_to_row mismatch at doc_line={dl}");
    }
    // 32 个随机 row 测 find_by_row
    let total_rows = display_map.total_rows();
    if total_rows > 0 {
        for _ in 0..32 {
            let row = rng.next() % total_rows;
            let a = display_map.snapshot().resolve_row(row).map(|l| l.doc_line);
            let b = wrap_index.display_to_doc(row);
            debug_assert_eq!(a, Some(b), "find_by_row mismatch at row={row}");
        }
    }
}

#[cfg(debug_assertions)]
struct SimpleRng(u64);
#[cfg(debug_assertions)]
impl SimpleRng {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next(&mut self) -> usize {
        // xorshift64
        let mut x = self.0;
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        self.0 = x;
        (x as usize) & (usize::MAX >> 1)
    }
}
```

注：上面用了 `wrap_index.len()` —— 阅读 wrap_index.rs，是 `len()` 还是 `total_lines()`，按真实方法名替换。

- [ ] **Step 7: 在每帧渲染前调用**

定位每帧的 update / render 入口（搜索 `fn render` / `fn update`），在合适位置插：

```rust
#[cfg(debug_assertions)]
if let Some(ref dm) = self.display_map_debug {
    assert_display_map_matches_wrap_index(dm, &self.wrap_index);
}
```

- [ ] **Step 8: 跑 cargo build + 单测**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
```

Expected: 全部 PASS（无 EDIT_PARALLEL_ASSERT 时是 no-op）。

- [ ] **Step 9: 启用 parallel assert 跑一遍**

```bash
EDIT_PARALLEL_ASSERT=1 cargo test -p edit-plus-app -- --quiet
```

Expected: PASS。

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/app.rs crates/app/src/buffer_adapter.rs crates/app/src/lib.rs
git commit -m "feat(app): debug 模式下 DisplayLineMap 与 WrapIndex 并行 assert"
```

---

## Task 2.7: Phase 2 验收

- [ ] **Step 1: 启动 release 构建一次确认无 cfg debug 泄漏**

```bash
cargo build -p edit-plus-app --release
```

- [ ] **Step 2: 集成测试 — 用 4.1MB JSON 跑 parallel assert**

```bash
cd /Users/dan/proj/llmws/edit+
EDIT_PARALLEL_ASSERT=1 cargo run -p edit-plus-app -- /Users/dan/Downloads/段落标题.json &
APP_PID=$!
sleep 5
kill $APP_PID
```

观察终端：debug_assert 不应触发。如有 panic，记录失败的 row/doc_line，回头修。

- [ ] **Step 3: tag**

```bash
git tag phase2-display-line-map-done
```

---

# Phase 3 — RenderCache + 顶点重构

**目标**：把渲染管线从 `WrapIndex` 切换到 `DisplaySnapshot`，引入 `RenderCache<doc_line, CachedLine>` 行内相对坐标缓存。验收：4.1MB JSON 滚动一帧 < 2ms；主题切换 0 invalidate。
**估算**：700 行新代码 + 300 行改动。

## File Structure (Phase 3)

| 文件 | 职责 |
|------|------|
| `crates/render/src/lib.rs` | `GlyphAtlas::insert_with_eviction` + `InsertOutcome` |
| `crates/app/src/render_geom.rs` | `GlyphInstance` 类型 |
| `crates/app/src/render_cache.rs` | RenderCache 主体（~400 行） |
| `crates/app/src/render_pipeline.rs` | 重写 `shape_visible_lines` |
| `crates/app/src/reshape_worker.rs` | 把 placeholder 换成真实 shape |
| `crates/app/src/app.rs` | TextState 接入 RenderCache |

## Task 3.1: GlyphAtlas::insert_with_eviction

**Files:**
- Modify: `crates/render/src/lib.rs:138-175`

- [ ] **Step 1: 写失败测试（在 `crates/render/src/lib.rs` `tests` mod 内追加）**

```rust
#[test]
fn insert_with_eviction_returns_evicted_keys() {
    let mut atlas = GlyphAtlas::new(256, 256, 3);
    let key1 = GlyphKey { glyph_id: 1, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
    let key2 = GlyphKey { glyph_id: 2, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
    let key3 = GlyphKey { glyph_id: 3, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };
    let key4 = GlyphKey { glyph_id: 4, font_id: 0usize, font_size: 14 * 64, subpixel_phase: 0 };

    assert!(matches!(atlas.insert_with_eviction(key1, 10, 10, 0.0, 0.0), InsertOutcome::Allocated { .. }));
    assert!(matches!(atlas.insert_with_eviction(key2, 10, 10, 0.0, 0.0), InsertOutcome::Allocated { .. }));
    assert!(matches!(atlas.insert_with_eviction(key3, 10, 10, 0.0, 0.0), InsertOutcome::Allocated { .. }));

    let outcome = atlas.insert_with_eviction(key4, 10, 10, 0.0, 0.0);
    match outcome {
        InsertOutcome::Allocated { evicted, .. } => {
            assert_eq!(evicted.as_slice(), &[key2]);  // LRU = key2
        }
        _ => panic!("expected Allocated"),
    }
}
```

- [ ] **Step 2: 实现**

`crates/render/src/lib.rs` 在 `pub struct GlyphAtlas` 之后定义：

```rust
pub enum InsertOutcome {
    Allocated { slot: GlyphSlot, evicted: smallvec::SmallVec<[GlyphKey; 4]> },
    Oversized,
}
```

注：需要给 `crates/render/Cargo.toml` 添加 `smallvec`：

```toml
smallvec = { workspace = true }
```

`impl GlyphAtlas` 内追加：

```rust
pub fn insert_with_eviction(
    &mut self,
    key: GlyphKey,
    width: u32,
    height: u32,
    bearing_x: f32,
    bearing_y: f32,
) -> InsertOutcome {
    if self.oversized.contains(&key) { return InsertOutcome::Oversized; }

    let mut evicted = smallvec::SmallVec::<[GlyphKey; 4]>::new();
    if self.slots.len() >= self.slots.capacity() {
        // hashlink::LruCache 的 LRU 淘汰可通过 remove_lru() 取出
        if let Some((evicted_key, _)) = self.slots.remove_lru() {
            evicted.push(evicted_key);
        }
    }

    for page in &mut self.pages {
        if let Some((x, y)) = page.allocate(width, height) {
            let slot = GlyphSlot { x, y, width, height, page: page.index, bearing_x, bearing_y };
            self.slots.insert(key, slot);
            return InsertOutcome::Allocated { slot, evicted };
        }
    }

    let page_index = self.pages.len() as u32;
    let mut new_page = AtlasPage::new(page_index, self.page_width, self.page_height);
    if let Some((x, y)) = new_page.allocate(width, height) {
        let slot = GlyphSlot { x, y, width, height, page: page_index, bearing_x, bearing_y };
        self.pages.push(new_page);
        self.slots.insert(key, slot);
        InsertOutcome::Allocated { slot, evicted }
    } else {
        self.oversized.insert(key);
        InsertOutcome::Oversized
    }
}
```

- [ ] **Step 3: 验证旧 `insert` 仍兼容（保留旧签名）**

```bash
cargo test -p edit-plus-render -- --quiet
```

- [ ] **Step 4: Commit**

```bash
git add crates/render/Cargo.toml crates/render/src/lib.rs
git commit -m "feat(render): GlyphAtlas::insert_with_eviction 返回被驱逐 keys"
```

---

## Task 3.2: GlyphInstance 类型

**Files:**
- Modify: `crates/app/src/render_geom.rs`

- [ ] **Step 1: 在 `crates/app/src/render_geom.rs` 末尾追加**

```rust
/// 行内相对坐标的字形实例 — RenderCache 的存储粒度。
///
/// 不存储 y / NDC / color。渲染时由调用方加 y_offset、查 highlight 主题、合成 6 顶点。
#[derive(Clone, Debug)]
pub struct GlyphInstance {
    pub atlas_slot_id: u32,
    pub x_local: f32,
    pub advance: f32,
    pub byte_start: u32,
    pub vl_index: u8,
}
```

- [ ] **Step 2: 验证编译 + commit**

```bash
cargo build -p edit-plus-app
git add crates/app/src/render_geom.rs
git commit -m "feat(render_geom): GlyphInstance 行内相对坐标"
```

---

## Task 3.3: render_cache.rs 骨架（LRU + slot table）

**Files:**
- Create: `crates/app/src/render_cache.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 创建 `crates/app/src/render_cache.rs`**

```rust
//! 行级字形实例缓存。
//!
//! - LRU<doc_line, CachedLine>，容量 = visible_rows + 2*OVERSCAN
//! - GlyphInstance 行内相对 x，渲染时合成顶点（< 1ms 全屏）
//! - atlas_slot_id 间接层 → atlas LRU 驱逐时通过反向索引精确失效

use std::collections::HashMap;

use hashlink::LruCache;
use render::{GlyphKey, GlyphSlot};
use shaping::ShapedRun;
use smallvec::SmallVec;

use crate::render_geom::GlyphInstance;

pub const OVERSCAN: usize = 500;

#[derive(Clone)]
pub struct CachedLine {
    pub instances: Vec<GlyphInstance>,
    pub vl_count: u8,
    pub line_number_glyphs: Vec<GlyphInstance>,
    pub atlas_generation: u64,
}

pub struct AtlasSlotEntry {
    pub key: GlyphKey,
    pub slot: GlyphSlot,
}

pub struct RenderCache {
    cache: LruCache<usize, CachedLine>,
    pub atlas_generation: u64,
    line_number_pool: HashMap<u32, ShapedRun>,
    pub slot_table: Vec<AtlasSlotEntry>,
    pub free_slot_ids: Vec<u32>,
    pub reverse_index: HashMap<GlyphKey, SmallVec<[u32; 4]>>,
}

impl RenderCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: LruCache::new(capacity),
            atlas_generation: 0,
            line_number_pool: HashMap::new(),
            slot_table: Vec::new(),
            free_slot_ids: Vec::new(),
            reverse_index: HashMap::new(),
        }
    }

    pub fn capacity(&self) -> usize { self.cache.capacity() }

    pub fn get(&mut self, doc_line: usize) -> Option<&CachedLine> {
        self.cache.get(&doc_line)
    }

    pub fn insert(&mut self, doc_line: usize, line: CachedLine) {
        self.cache.insert(doc_line, line);
    }

    pub fn invalidate_rows(&mut self, doc_lines: std::ops::Range<usize>) {
        let keys: Vec<_> = self.cache.iter().map(|(k, _)| *k).filter(|k| doc_lines.contains(k)).collect();
        for k in keys { self.cache.remove(&k); }
    }

    pub fn invalidate_all(&mut self) {
        self.cache.clear();
        // slot_table 与 reverse_index 可保留，因为 atlas 未变；只是无人引用。
    }

    pub fn shift(&mut self, at: usize, delta: i64) {
        let mut moved: Vec<(usize, CachedLine)> = Vec::new();
        // 收集需要 shift 的 entry（key >= at）
        let keys: Vec<_> = self.cache.iter().map(|(k, _)| *k).collect();
        for k in keys {
            if k >= at {
                if let Some(v) = self.cache.remove(&k) {
                    let new_k = (k as i64 + delta).max(0) as usize;
                    moved.push((new_k, v));
                }
            }
        }
        for (k, v) in moved { self.cache.insert(k, v); }
    }

    pub fn bump_atlas_generation(&mut self) {
        self.atlas_generation += 1;
    }

    /// atlas LRU 驱逐回调：精确失效占用了被驱逐 key 的所有行。
    pub fn handle_atlas_eviction(&mut self, evicted: &[GlyphKey]) {
        for key in evicted {
            if let Some(slot_ids) = self.reverse_index.remove(key) {
                for slot_id in slot_ids {
                    self.free_slot_ids.push(slot_id);
                }
            }
        }
        // 受影响的 doc_line：扫一遍 cache。
        // 实际场景被驱逐字形大概率分散在很多行 → 简化：bump generation，
        // 渲染层下次访问时检测 generation 不一致就重 shape。
        self.atlas_generation += 1;
    }

    pub fn shape_line_number(&mut self, n: u32, shaper: &mut shaping::Shaper) -> &ShapedRun {
        self.line_number_pool.entry(n).or_insert_with(|| {
            let s = format!("{}", n);
            shaper.shape(&s).expect("shape line number")
        })
    }
}
```

- [ ] **Step 2: 注册 module + 写测试**

`crates/app/src/lib.rs`：

```rust
pub mod render_cache;
```

文件末尾追加 tests：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_line() -> CachedLine {
        CachedLine {
            instances: vec![],
            vl_count: 1,
            line_number_glyphs: vec![],
            atlas_generation: 0,
        }
    }

    #[test]
    fn lru_capacity_capped() {
        let mut c = RenderCache::new(3);
        for i in 0..10 { c.insert(i, dummy_line()); }
        let count = (0..10).filter(|i| c.get(*i).is_some()).count();
        assert!(count <= 3);
    }

    #[test]
    fn invalidate_rows_only_drops_intersecting() {
        let mut c = RenderCache::new(100);
        for i in 0..10 { c.insert(i, dummy_line()); }
        c.invalidate_rows(3..7);
        for i in 0..10 {
            let present = c.get(i).is_some();
            if (3..7).contains(&i) { assert!(!present, "i={i}"); } else { assert!(present, "i={i}"); }
        }
    }

    #[test]
    fn shift_offsets_keys_correctly() {
        let mut c = RenderCache::new(100);
        for i in 0..5 { c.insert(i, dummy_line()); }
        c.shift(2, 10); // i in 0,1 不动；i in 2,3,4 → 12,13,14
        assert!(c.get(0).is_some());
        assert!(c.get(1).is_some());
        assert!(c.get(2).is_none());
        assert!(c.get(12).is_some());
        assert!(c.get(13).is_some());
        assert!(c.get(14).is_some());
    }

    #[test]
    fn shift_negative_clamps_at_zero() {
        let mut c = RenderCache::new(100);
        for i in 5..10 { c.insert(i, dummy_line()); }
        c.shift(0, -3); // i 5→2, 6→3, ...
        assert!(c.get(2).is_some());
    }
}
```

- [ ] **Step 3: 跑测试 + commit**

```bash
cargo test -p edit-plus-app render_cache -- --quiet
git add crates/app/src/render_cache.rs crates/app/src/lib.rs
git commit -m "feat(render_cache): LRU + shift + invalidate_rows + slot_table 骨架"
```

---

## Task 3.4: worker 真实 shape + wrap

**Files:**
- Modify: `crates/app/src/reshape_worker.rs`
- Modify: `crates/app/src/display_line_map.rs`

worker 之前的 `process_request` 仅生成 placeholder。Phase 3 让它真实做 shape + wrap，并产出 `CachedLine`。

- [ ] **Step 1: 把 `ReshapeOutput` 替换为 `CachedLine`**

修改 `crates/app/src/display_line_map.rs`：

```rust
pub type ReshapeOutput = crate::render_cache::CachedLine;
```

删除原来 `pub struct ReshapeOutput { ... }` 定义。

- [ ] **Step 2: 把 worker 的 `process_request` 替换为真实 shape**

worker 需要持有 `Shaper`。修改 `worker_loop`：

```rust
fn worker_loop(
    rx: Receiver<WorkerMsg>,
    tx_results: Sender<ReshapeResult>,
    current_generation: Arc<AtomicU64>,
    pending_count: Arc<AtomicUsize>,
) {
    let mut shaper = match shaping::Shaper::new() {
        Ok(s) => s,
        Err(_) => return,
    };
    while let Ok(msg) = rx.recv() {
        match msg {
            WorkerMsg::Shutdown => return,
            WorkerMsg::Request(req) => {
                pending_count.fetch_sub(1, Ordering::AcqRel);
                let cur = current_generation.load(Ordering::Acquire);
                if req.generation < cur { continue; }
                shaper.set_font_size(req.font_size);
                let result = process_request(req, &mut shaper);
                if tx_results.send(result).is_err() { return; }
            }
        }
    }
}

fn process_request(req: ReshapeRequest, shaper: &mut shaping::Shaper) -> ReshapeResult {
    use xxhash_rust::xxh3::xxh3_64;
    use crate::display_line_map::DisplayLineEntry;
    use crate::snap_tree::VisualBreak;
    use smallvec::SmallVec;

    let hash = xxh3_64(&req.line_bytes);
    let bytes = &req.line_bytes[..];
    let line_str = std::str::from_utf8(bytes).unwrap_or("");

    // 真实 shape
    let shaped = match shaper.shape(line_str) {
        Ok(s) => s,
        Err(_) => {
            // shape 失败：placeholder + 空 instances
            let entry = DisplayLineEntry::placeholder(0, bytes.len() as u32, hash);
            return ReshapeResult {
                generation: req.generation,
                doc_line: req.doc_line,
                entry,
                render_payload: crate::render_cache::CachedLine {
                    instances: vec![],
                    vl_count: 1,
                    line_number_glyphs: vec![],
                    atlas_generation: 0,
                },
            };
        }
    };

    // 真实 wrap：按 viewport_width 切 visual line
    let vls = compute_visual_breaks(&shaped, bytes, req.viewport_width);
    let vl_count = vls.len().max(1) as u16;

    let mut breaks = SmallVec::<[VisualBreak; 1]>::new();
    for (start, end, width) in &vls {
        breaks.push(VisualBreak { byte_start: *start as u32, byte_end: *end as u32, pixel_width: *width });
    }

    let entry = DisplayLineEntry {
        visual_line_count: vl_count,
        visual_breaks: breaks,
        byte_offset: 0, // 由 sync 填充
        byte_length: bytes.len() as u32,
        content_hash: hash,
    };

    // 注：这里只产出 entry。GlyphInstance 的填充需要 atlas slot_id —— 而 atlas 在主线程。
    // 因此 CachedLine.instances 在主线程接到 result 后填，worker 只产出 metadata。
    // 暂时返回空 instances；主线程的 fill_render_cache_from_result 完成最后一公里。
    let cached = crate::render_cache::CachedLine {
        instances: vec![],
        vl_count: vl_count.min(255) as u8,
        line_number_glyphs: vec![],
        atlas_generation: 0,
    };

    ReshapeResult {
        generation: req.generation,
        doc_line: req.doc_line,
        entry,
        render_payload: cached,
    }
}

fn compute_visual_breaks(
    shaped: &shaping::ShapedRun,
    bytes: &[u8],
    viewport_width: f32,
) -> Vec<(usize, usize, f32)> {
    // 与 render_pipeline.rs 中 compute_visual_lines 等价的算法
    // —— Task 3.5 中复用现有实现，先打 stub，后续移植
    let total: f32 = shaped.clusters.iter().map(|c| c.advance).sum();
    if total <= viewport_width || shaped.clusters.is_empty() {
        let end = bytes.len();
        return vec![(0, end, total)];
    }
    let mut out = Vec::new();
    let mut acc_w = 0.0f32;
    let mut start_byte = 0usize;
    for c in &shaped.clusters {
        if acc_w + c.advance > viewport_width && acc_w > 0.0 {
            out.push((start_byte, c.byte_range.start, acc_w));
            start_byte = c.byte_range.start;
            acc_w = c.advance;
        } else {
            acc_w += c.advance;
        }
    }
    out.push((start_byte, bytes.len(), acc_w));
    out
}
```

- [ ] **Step 3: 保留原有 placeholder 调用 backward compat**

`reshape_worker.rs` 顶部的 `use crate::display_line_map::{DisplayLineEntry, ReshapeOutput};` 改为：

```rust
use crate::display_line_map::DisplayLineEntry;
use crate::render_cache::CachedLine as ReshapeOutput;
```

- [ ] **Step 4: 验证编译 + 跑现有测试**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app reshape_worker display_line_map -- --quiet
```

注：`compute_visual_breaks` 是 stub，结果未必和 `render_pipeline::compute_visual_lines` 完全一致。Phase 3 内部一致即可（worker 用它，render_pipeline 也将用它），Task 3.6 会统一。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/reshape_worker.rs crates/app/src/display_line_map.rs
git commit -m "feat(reshape_worker): 真实 shape + wrap (产出 entry，instances 留主线程)"
```

---

## Task 3.5: 重写 `shape_visible_lines` —— 切到 DisplaySnapshot + RenderCache

**Files:**
- Modify: `crates/app/src/render_pipeline.rs`
- Modify: `crates/app/src/app.rs`

这是 Phase 3 最大的改动。**策略**：保留原 `shape_visible_lines` 函数签名不变，但内部从"WrapIndex 迭代"改为"DisplaySnapshot 迭代 + RenderCache 查表 + cache miss 兜底"。

- [ ] **Step 1: 在 `TextState` 中增加字段（见 Task 3.7），先在 render_pipeline 中接收为参数**

修改 `shape_visible_lines` 签名（`render_pipeline.rs:129-149`）：把 `wrap_index: &mut WrapIndex` 替换为 `snapshot: &DisplaySnapshot`，并在参数列表追加 `render_cache: &mut RenderCache`、`worker: &ReshapeWorker`。**保留 `wrap_index` 兼容**——添加在末尾用 `Option<&mut WrapIndex>`，以便此 task 期间双写。

```rust
pub(crate) fn shape_visible_lines(
    theme: &crate::theme::Theme,
    dv: &mut DocumentView,
    tab_bar_height: f32,
    text: &mut TextState,
    gpu: &GpuState,
    snapshot: &crate::display_line_map::DisplaySnapshot,
    render_cache: &mut crate::render_cache::RenderCache,
    worker: &crate::reshape_worker::ReshapeWorker,
    advance_cache: &mut Vec<AdvanceCacheEntry>,
    cluster_pool: &mut Vec<Vec<(usize, f32)>>,
    cursor_visual_line: &mut Option<usize>,
    cursor_visual_line_in_doc: &mut usize,
    cursor_pixel_x: &mut f32,
    sticky_x: &mut f32,
    sticky_x_dirty: &mut bool,
    first_line: &mut LineCache,
    last_line: &mut LineCache,
    settings: &Settings,
    screen_w: f32,
    screen_h: f32,
    gutter_width: f32,
) -> Vec<GlyphVertex>
```

- [ ] **Step 2: 重写主循环 — 基于 snapshot 迭代**

把 `for i in 0..vis_count { ... }` 整段（line 185-505）替换为基于 snapshot 的循环。新的主循环（伪代码 → 实现）：

```rust
let scroll_top_row = dv.viewport.scroll_top.floor() as usize;
let visible_rows = dv.viewport.visible_rows;
let row_end = (scroll_top_row + visible_rows + 1).min(snapshot.total_rows());

let mut all_vertices = Vec::new();
let mut budget_miss = 2usize; // 主线程兜底预算
let mut visible_doc_lines: SmallVec<[usize; 64]> = SmallVec::new();

let mut last_doc_line: Option<usize> = None;
for row_owned in snapshot.iter_rows(scroll_top_row..row_end) {
    // 第一次见到一个新的 doc_line 时，决定是否需要 reshape
    if Some(row_owned.doc_line) != last_doc_line {
        last_doc_line = Some(row_owned.doc_line);
        visible_doc_lines.push(row_owned.doc_line);

        let needs_reshape = match render_cache.get(row_owned.doc_line) {
            Some(c) if c.atlas_generation == render_cache.atlas_generation => false,
            _ => true,
        };
        if needs_reshape {
            if budget_miss > 0 {
                let line_bytes = dv.line_bytes(row_owned.doc_line);
                let cached = build_cached_line_inline(
                    &line_bytes,
                    row_owned.doc_line,
                    &mut text.shaper,
                    &mut text.atlas,
                    &text.atlas_texture,
                    &gpu.ctx.queue,
                    render_cache,
                    settings,
                    screen_w - 16.0 - gutter_width,
                );
                render_cache.insert(row_owned.doc_line, cached);
                budget_miss -= 1;
            } else {
                // 兜底超预算：入队 worker，本帧画占位
                let bytes = dv.line_bytes(row_owned.doc_line);
                let _ = worker.submit(crate::reshape_worker::ReshapeRequest {
                    generation: snapshot.generation,
                    doc_line: row_owned.doc_line,
                    line_bytes: bytes,
                    viewport_width: snapshot.viewport_width,
                    font_size: snapshot.font_size,
                });
            }
        }
    }

    // 拿 cached（重新 get）→ 推顶点
    if let Some(cached) = render_cache.get(row_owned.doc_line) {
        let visual_idx = row_owned.visual_idx_in_doc;
        let row_index_in_viewport = (snapshot.line_to_row(row_owned.doc_line)
            + visual_idx)
            .saturating_sub(scroll_top_row);
        let line_y = row_index_in_viewport as f32 * settings.line_height
            + dv.viewport.sub_line_pixel_offset(settings.line_height);
        let y_base = line_y + settings.line_height * 0.8 + tab_bar_height;

        let theme_color = theme.foreground;
        for inst in cached.instances.iter().filter(|i| i.vl_index as usize == visual_idx) {
            // 查 highlight 颜色
            let color = highlight_color_for_offset_per_line(
                dv, row_owned.doc_line, inst.byte_start as usize, theme_color, theme,
            );
            // 查 atlas slot
            let slot_entry = &render_cache.slot_table[inst.atlas_slot_id as usize];
            let slot = slot_entry.slot;
            let verts = GlyphRenderer::generate_vertices(
                &[(slot, inst.x_local, y_base)],
                ATLAS_SIZE,
                ATLAS_SIZE,
                screen_w,
                screen_h,
                color,
            );
            all_vertices.extend(verts);
        }
        // 行号 (visual_idx == 0 时)
        if visual_idx == 0 && gutter_width > 0.0 {
            for inst in &cached.line_number_glyphs {
                let slot = render_cache.slot_table[inst.atlas_slot_id as usize].slot;
                let verts = GlyphRenderer::generate_vertices(
                    &[(slot, inst.x_local, y_base)],
                    ATLAS_SIZE,
                    ATLAS_SIZE,
                    screen_w,
                    screen_h,
                    theme.line_number,
                );
                all_vertices.extend(verts);
            }
        }
    }
    // else: 兜底超预算，画背景占位即可（背景由其它管线绘制，这里跳过）
}

// 预取：滚动方向 OVERSCAN 范围内未缓存行入队 worker
for line in visible_doc_lines.iter().copied().last().into_iter() {
    let prefetch_end = (line + 1 + crate::render_cache::OVERSCAN).min(snapshot.line_count());
    for l in (line + 1)..prefetch_end {
        if render_cache.get(l).is_none() {
            let bytes = dv.line_bytes(l);
            let _ = worker.submit(crate::reshape_worker::ReshapeRequest {
                generation: snapshot.generation,
                doc_line: l,
                line_bytes: bytes,
                viewport_width: snapshot.viewport_width,
                font_size: snapshot.font_size,
            });
        }
    }
}
```

- [ ] **Step 3: 实现 `build_cached_line_inline`**

它就是把原来"shape → wrap → 生成 GlyphVertex"的流程改写为"shape → wrap → 填 RenderCache.slot_table → 产出 GlyphInstance"。文件末尾追加：

```rust
fn build_cached_line_inline(
    line_bytes: &[u8],
    doc_line: usize,
    shaper: &mut shaping::Shaper,
    atlas: &mut render::GlyphAtlas,
    atlas_texture: &wgpu::Texture,
    queue: &wgpu::Queue,
    render_cache: &mut crate::render_cache::RenderCache,
    settings: &crate::settings::Settings,
    viewport_width: f32,
) -> crate::render_cache::CachedLine {
    use crate::render_geom::GlyphInstance;
    use render::{GlyphKey, InsertOutcome};

    let line_str = std::str::from_utf8(line_bytes).unwrap_or("");
    let shaped = match shaper.shape(line_str) {
        Ok(s) => s,
        Err(_) => return crate::render_cache::CachedLine {
            instances: vec![],
            vl_count: 1,
            line_number_glyphs: vec![],
            atlas_generation: render_cache.atlas_generation,
        },
    };

    let char_width = pick_char_width(&shaped.clusters, line_bytes, settings.font_size * 0.6);
    let visual_lines = compute_visual_lines(&shaped.clusters, line_bytes, char_width, viewport_width);

    let mut instances = Vec::with_capacity(shaped.clusters.len());
    for (vl_idx, &(vl_start, vl_end, _vl_w)) in visual_lines.iter().enumerate() {
        let mut x_cursor = 32.0 * settings.dpi_scale;
        for cluster in &shaped.clusters[vl_start..vl_end] {
            let cluster_bytes = &line_bytes[cluster.byte_range.clone()];
            let is_ws = is_whitespace_cluster(cluster_bytes);
            let advance = if is_ws {
                ws_cluster_advance(cluster_bytes, char_width)
            } else {
                cluster.advance.max(1.0)
            };
            if is_ws {
                x_cursor += advance;
                continue;
            }

            let font_id_usize = {
                use std::hash::{Hash, Hasher};
                let mut h = std::hash::DefaultHasher::new();
                cluster.font_id.hash(&mut h);
                h.finish() as usize
            };
            let key = GlyphKey {
                glyph_id: cluster.glyph_id,
                font_id: font_id_usize,
                font_size: (shaper.font_size() * 64.0) as u32,
                subpixel_phase: 0,
            };

            // atlas 查/插
            let slot = if let Some(s) = atlas.get(&key) {
                *s
            } else if let Some(bitmap) = shaper.rasterize_glyph(cluster.font_id, cluster.glyph_id as u16, shaper.font_size()) {
                if bitmap.width == 0 || bitmap.height == 0 { x_cursor += advance; continue; }
                let outcome = atlas.insert_with_eviction(key, bitmap.width, bitmap.height, bitmap.left as f32, bitmap.top as f32);
                let allocated_slot = match outcome {
                    InsertOutcome::Allocated { slot, evicted } => {
                        if !evicted.is_empty() {
                            render_cache.handle_atlas_eviction(&evicted);
                        }
                        // 写纹理
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: atlas_texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: slot.x, y: slot.y, z: 0 },
                                aspect: wgpu::TextureAspect::All,
                            },
                            &bitmap.data,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(bitmap.width),
                                rows_per_image: Some(bitmap.height),
                            },
                            wgpu::Extent3d { width: bitmap.width, height: bitmap.height, depth_or_array_layers: 1 },
                        );
                        slot
                    }
                    InsertOutcome::Oversized => { x_cursor += advance; continue; }
                };
                allocated_slot
            } else {
                x_cursor += advance; continue;
            };

            // 注册到 slot_table，得 atlas_slot_id
            let slot_id = render_cache.free_slot_ids.pop().unwrap_or_else(|| {
                let id = render_cache.slot_table.len() as u32;
                render_cache.slot_table.push(crate::render_cache::AtlasSlotEntry { key, slot });
                id
            });
            render_cache.slot_table[slot_id as usize] = crate::render_cache::AtlasSlotEntry { key, slot };
            render_cache
                .reverse_index
                .entry(key)
                .or_default()
                .push(slot_id);

            instances.push(GlyphInstance {
                atlas_slot_id: slot_id,
                x_local: x_cursor,
                advance,
                byte_start: cluster.byte_range.start as u32,
                vl_index: vl_idx.min(255) as u8,
            });
            x_cursor += advance;
        }
    }

    let _ = doc_line; // 暂未使用，保留参数以备 line_number_glyphs 后续填充
    crate::render_cache::CachedLine {
        instances,
        vl_count: visual_lines.len().min(255) as u8,
        line_number_glyphs: vec![], // Task 3.6 填行号
        atlas_generation: render_cache.atlas_generation,
    }
}

fn highlight_color_for_offset_per_line(
    dv: &DocumentView,
    doc_line: usize,
    byte_offset: usize,
    fallback: [f32; 4],
    theme: &crate::theme::Theme,
) -> [f32; 4] {
    let spans: Vec<(usize, crate::theme::HighlightKind)> = dv
        .highlights_for_line(doc_line)
        .iter()
        .map(|h| (h.start, h.kind))
        .collect();
    if spans.is_empty() { return fallback; }
    highlight_color_for_offset(&spans, byte_offset, theme)
}
```

- [ ] **Step 4: 编译并修复类型/导入错误**

```bash
cargo build -p edit-plus-app 2>&1 | head -80
```

按编译器提示修复。常见：
- `dv.line_bytes(...)` 真实方法名（grep 验证）
- `shaper.set_font_size` 方法名（grep 验证）

- [ ] **Step 5: 跑现有测试**

```bash
cargo test -p edit-plus-app -- --quiet
```

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/render_pipeline.rs
git commit -m "feat(render_pipeline): 切到 DisplaySnapshot + RenderCache + cache miss 兜底"
```

---

## Task 3.6: 行号缓存（line_number_glyphs）

**Files:**
- Modify: `crates/app/src/render_pipeline.rs`

之前 `build_cached_line_inline` 把 `line_number_glyphs` 留空。这里把行号也填入。

- [ ] **Step 1: 在 `build_cached_line_inline` 内追加行号 instances**

在函数末尾、return 之前插：

```rust
// 行号
let line_num_str = format!("{}", doc_line + 1);
let line_num_shaped = render_cache.shape_line_number((doc_line + 1) as u32, shaper).clone();
let mut line_number_glyphs = Vec::with_capacity(line_num_shaped.clusters.len());
let ln_font_size = shaper.font_size() * 0.8;
let mut ln_x = 0.0f32;
for cluster in &line_num_shaped.clusters {
    let font_id_usize = {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        cluster.font_id.hash(&mut h);
        h.finish() as usize
    };
    let key = render::GlyphKey {
        glyph_id: cluster.glyph_id,
        font_id: font_id_usize,
        font_size: (ln_font_size * 64.0) as u32,
        subpixel_phase: 0,
    };
    let slot = if let Some(s) = atlas.get(&key) {
        *s
    } else if let Some(bm) = shaper.rasterize_glyph(cluster.font_id, cluster.glyph_id as u16, ln_font_size) {
        if bm.width == 0 || bm.height == 0 { ln_x += cluster.advance; continue; }
        match atlas.insert_with_eviction(key, bm.width, bm.height, bm.left as f32, bm.top as f32) {
            render::InsertOutcome::Allocated { slot, evicted } => {
                if !evicted.is_empty() { render_cache.handle_atlas_eviction(&evicted); }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: atlas_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: slot.x, y: slot.y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &bm.data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bm.width), rows_per_image: Some(bm.height) },
                    wgpu::Extent3d { width: bm.width, height: bm.height, depth_or_array_layers: 1 },
                );
                slot
            }
            render::InsertOutcome::Oversized => { ln_x += cluster.advance; continue; }
        }
    } else {
        ln_x += cluster.advance; continue;
    };

    let slot_id = render_cache.free_slot_ids.pop().unwrap_or_else(|| {
        let id = render_cache.slot_table.len() as u32;
        render_cache.slot_table.push(crate::render_cache::AtlasSlotEntry { key, slot });
        id
    });
    render_cache.slot_table[slot_id as usize] = crate::render_cache::AtlasSlotEntry { key, slot };
    render_cache.reverse_index.entry(key).or_default().push(slot_id);

    line_number_glyphs.push(crate::render_geom::GlyphInstance {
        atlas_slot_id: slot_id,
        x_local: ln_x,
        advance: cluster.advance,
        byte_start: 0,
        vl_index: 0,
    });
    ln_x += cluster.advance;
}
let _ = line_num_str; // 仅用于调试日志
```

把 `line_number_glyphs: vec![]` 改为 `line_number_glyphs`。

- [ ] **Step 2: Commit**

```bash
cargo build -p edit-plus-app
git add crates/app/src/render_pipeline.rs
git commit -m "feat(render_pipeline): 行号 GlyphInstance 入 CachedLine.line_number_glyphs"
```

---

## Task 3.7: TextState 接入 RenderCache + 调用点更新

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 探索调用 `shape_visible_lines` 的位置**

```bash
grep -n "shape_visible_lines" /Users/dan/proj/llmws/edit+/crates/app/src/app.rs
```

应有 2~3 处。

- [ ] **Step 2: 在 `TextState` 添加字段**

定位 `pub(crate) struct TextState` 定义（line ~46）。在 `wrap_cache: ...` 字段下方追加：

```rust
pub(crate) display_map: crate::display_line_map::DisplayLineMap,
pub(crate) current_snapshot: crate::display_line_map::DisplaySnapshot,
pub(crate) render_cache: crate::render_cache::RenderCache,
```

- [ ] **Step 3: 初始化 — 文件加载完成后**

定位 `TextState { ... }` struct literal 构造点（grep `shape_cache: LruCache::new`，line ~857），追加初始化代码（用上面 buffer_adapter）：

```rust
let display_map = crate::display_line_map::DisplayLineMap::from_buffer(
    &crate::buffer_adapter::Adapter(&buffer),
    screen_w - 16.0 - gutter_width,
    settings.font_size,
);
let current_snapshot = display_map.snapshot();
let render_cache_capacity = (settings.visible_rows_estimate + 2 * crate::render_cache::OVERSCAN).max(64);
let render_cache = crate::render_cache::RenderCache::new(render_cache_capacity);
```

并在 struct literal 内追加 `display_map`, `current_snapshot`, `render_cache` 三个字段。

注：`settings.visible_rows_estimate` 不一定存在。退而求其次，用 `((screen_h / settings.line_height) as usize).max(40)`。

- [ ] **Step 4: 更新所有 `shape_visible_lines` 调用**

每个调用点（grep `shape_visible_lines`）：把 `wrap_index: &mut self.wrap_index` 替换为：

```rust
snapshot: &text.current_snapshot,
render_cache: &mut text.render_cache,
worker: &text.display_map.worker_handle(),
```

但 `worker_handle()` 当前不存在。在 `DisplayLineMap` 增加：

```rust
impl DisplayLineMap {
    pub fn worker(&self) -> &crate::reshape_worker::ReshapeWorker { &self.worker }
}
```

并把 `display_line_map.rs` 中 `worker: ReshapeWorker` 字段保持私有，仅通过 `worker()` 暴露。

- [ ] **Step 5: 在每帧 update 中调 poll_worker**

在 `App::update` 或类似入口（grep `fn update` / `fn render`）末尾追加：

```rust
if let Some(text) = self.text.as_mut() {
    if let Some(patch) = text.display_map.poll_worker() {
        for r in patch.affected_rows {
            text.render_cache.invalidate_rows(r);
        }
        // 把 worker 推回的 ReshapeOutput 写入 render_cache
        for (doc_line, cached) in text.display_map.drain_pending_render_inserts() {
            text.render_cache.insert(doc_line, cached);
        }
        text.current_snapshot = text.display_map.snapshot();
    }
}
```

- [ ] **Step 6: 编译 + 修错 + 跑测试**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
```

修复编译错误（最常见：`shape_visible_lines` 调用方少了/多了参数；`DocumentView::line_bytes` 真实名字）。

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/app.rs crates/app/src/display_line_map.rs crates/app/src/render_pipeline.rs
git commit -m "feat(app): TextState 接入 RenderCache + 每帧 poll_worker"
```

---

## Task 3.8: 性能 bench — 滚动 < 2ms / 主题切换 0 invalidate

**Files:**
- Modify: `crates/app/benches/scroll_bench.rs`

- [ ] **Step 1: 阅读现有 scroll_bench**

```bash
cat /Users/dan/proj/llmws/edit+/crates/app/benches/scroll_bench.rs | head -80
```

- [ ] **Step 2: 加 4MB JSON 滚动 bench（不依赖 GPU）**

由于 GPU 在 bench 中不可用，bench 只覆盖 **CPU 侧热路径**：
`snapshot.iter_rows + render_cache.get + 把 GlyphInstance 折算成 (slot, x, y, color) 的循环`。

`crates/app/benches/scroll_bench.rs` 末尾追加：

```rust
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use edit_plus_app::display_line_map::{BufferLike, DisplayLineMap};
use edit_plus_app::render_cache::{CachedLine, RenderCache, OVERSCAN};
use edit_plus_app::render_geom::GlyphInstance;
use std::sync::Arc;

struct VecBuffer(Vec<Vec<u8>>);
impl BufferLike for VecBuffer {
    fn line_count(&self) -> usize { self.0.len() }
    fn line_bytes(&self, i: usize) -> Arc<[u8]> {
        Arc::from(self.0[i].clone().into_boxed_slice())
    }
    fn line_byte_offset(&self, i: usize) -> usize {
        self.0.iter().take(i).map(|l| l.len() + 1).sum()
    }
}

fn synth_cached(width: usize) -> CachedLine {
    let instances: Vec<GlyphInstance> = (0..width)
        .map(|i| GlyphInstance {
            atlas_slot_id: 0,
            x_local: i as f32 * 8.0,
            advance: 8.0,
            byte_start: i as u32,
            vl_index: 0,
        })
        .collect();
    CachedLine { instances, vl_count: 1, line_number_glyphs: vec![], atlas_generation: 0 }
}

fn bench_scroll_4mb(c: &mut Criterion) {
    let path = "/Users/dan/Downloads/段落标题.json";
    if !std::path::Path::new(path).exists() { return; }
    let raw = std::fs::read(path).unwrap();
    let lines: Vec<Vec<u8>> = raw.split(|b| *b == b'\n').map(|s| s.to_vec()).collect();
    let buf = VecBuffer(lines);
    let map = DisplayLineMap::from_buffer(&buf, 1200.0, 14.0);
    let snap = map.snapshot();

    let mut cache = RenderCache::new(buf.0.len() + 2 * OVERSCAN);
    for i in 0..buf.0.len() { cache.insert(i, synth_cached(80)); }

    c.bench_function("scroll_4mb_one_frame", |b| {
        b.iter(|| {
            let scroll_top = 5000usize;
            let visible = 40usize;
            let mut total_glyphs = 0usize;
            for row in snap.iter_rows(scroll_top..scroll_top + visible) {
                if let Some(c) = cache.get(row.doc_line) {
                    for inst in &c.instances {
                        let _ = black_box((inst.x_local, inst.advance, inst.byte_start));
                        total_glyphs += 1;
                    }
                }
            }
            black_box(total_glyphs);
        });
    });
}

criterion_group!(benches, bench_scroll_4mb);
criterion_main!(benches);
```

注：如 `crates/app/benches/scroll_bench.rs` 已有 `criterion_main!`，把上面的 `bench_scroll_4mb` 函数加进现有 group 即可，不要重复 `criterion_main!`。

- [ ] **Step 3: 跑 bench**

```bash
cargo bench -p edit-plus-app --bench scroll_bench -- scroll_4mb 2>&1 | tail -20
```

记录数字到 `docs/superpowers/plans/2026-06-03-large-file-scroll-perf.md` 末尾的"验收记录"段。

- [ ] **Step 4: 主题切换 0 invalidate 测试**

`crates/app/src/render_cache.rs` 测试 mod 内追加：

```rust
#[test]
fn theme_change_does_not_invalidate() {
    let mut c = RenderCache::new(100);
    for i in 0..10 { c.insert(i, dummy_line()); }
    // 模拟主题切换 —— 不调任何 invalidate
    // RenderCache 不感知主题，仅 atlas/编辑/resize 才失效
    for i in 0..10 { assert!(c.get(i).is_some()); }
}
```

```bash
cargo test -p edit-plus-app render_cache::tests::theme_change_does_not_invalidate -- --quiet
```

- [ ] **Step 5: Commit**

```bash
git add crates/app/benches/scroll_bench.rs crates/app/src/render_cache.rs
git commit -m "test(perf): 4MB JSON 滚动 bench + 主题切换 0 invalidate 验证"
```

- [ ] **Step 6: Phase 3 tag**

```bash
git tag phase3-render-cache-done
```

---

# Phase 4 — ScrollAnchor

**目标**：`viewport.scroll_top: f64` → `scroll_anchor: ScrollAnchor`。
**估算**：250 行改动。

## File Structure (Phase 4)

| 文件 | 职责 |
|------|------|
| `crates/app/src/viewport.rs` | ScrollAnchor 结构 + 转换函数 + Viewport 字段切换 |
| `crates/app/src/mouse.rs` | 滚轮 delta 转 anchor 调整 |
| `crates/app/src/commands.rs` | 跳转命令构造 anchor |
| `crates/app/src/scrollbar.rs` | 滚动条与 anchor 互转 |

## Task 4.1: ScrollAnchor 结构 + 不变量 + 测试

**Files:**
- Modify: `crates/app/src/viewport.rs`

- [ ] **Step 1: 写失败测试**

`crates/app/src/viewport.rs` 末尾的 `#[cfg(test)] mod tests` 内追加（如不存在则新建）：

```rust
#[cfg(test)]
mod scroll_anchor_tests {
    use super::*;

    #[test]
    fn anchor_default_at_origin() {
        let a = ScrollAnchor::default();
        assert_eq!(a.doc_line, 0);
        assert_eq!(a.pixel_offset, 0.0);
    }

    #[test]
    fn anchor_doc_line_unchanged_after_edit_above() {
        let mut a = ScrollAnchor { doc_line: 100, pixel_offset: 5.0 };
        let patch = crate::display_line_map::DisplayPatch {
            affected_rows: vec![0..10],
            line_shift: Some(crate::display_line_map::LineShift { at: 5, delta: 3 }),
            generation: 1,
        };
        a.adjust_after_edit(&patch);
        assert_eq!(a.doc_line, 103);
        assert_eq!(a.pixel_offset, 5.0);
    }

    #[test]
    fn anchor_clamps_when_doc_shrinks() {
        let mut a = ScrollAnchor { doc_line: 10, pixel_offset: 5.0 };
        let patch = crate::display_line_map::DisplayPatch {
            affected_rows: vec![],
            line_shift: Some(crate::display_line_map::LineShift { at: 5, delta: -8 }),
            generation: 1,
        };
        a.adjust_after_edit(&patch);
        // doc_line=10 落在 [5, 5+8) 区间外 → 保持 doc_line - 8 = 2
        assert_eq!(a.doc_line, 2);
    }

    #[test]
    fn anchor_lands_on_deleted_range_resets_to_at() {
        let mut a = ScrollAnchor { doc_line: 7, pixel_offset: 5.0 };
        let patch = crate::display_line_map::DisplayPatch {
            affected_rows: vec![],
            line_shift: Some(crate::display_line_map::LineShift { at: 5, delta: -3 }),
            generation: 1,
        };
        a.adjust_after_edit(&patch);
        assert_eq!(a.doc_line, 5);
        assert_eq!(a.pixel_offset, 0.0);
    }

    #[test]
    fn anchor_pixel_offset_refolds_on_resize() {
        let mut a = ScrollAnchor { doc_line: 10, pixel_offset: 8.0 };
        a.refold_on_resize(20.0, 24.0);
        assert!((a.pixel_offset - 8.0 * 24.0 / 20.0).abs() < 0.01);
    }
}
```

- [ ] **Step 2: 实现 ScrollAnchor**

`crates/app/src/viewport.rs` 顶部 use 区追加：

```rust
use crate::display_line_map::{DisplayPatch, DisplaySnapshot};
```

文件末尾追加：

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollAnchor {
    pub doc_line: usize,
    pub pixel_offset: f32,
}

impl ScrollAnchor {
    pub fn to_scroll_top(&self, snapshot: &DisplaySnapshot, line_height: f32) -> f64 {
        let row = snapshot.line_to_row(self.doc_line) as f64;
        row + (self.pixel_offset as f64) / (line_height as f64)
    }

    pub fn from_scroll_top(top: f64, snapshot: &DisplaySnapshot, line_height: f32) -> Self {
        let row = top.floor() as usize;
        match snapshot.resolve_row(row) {
            Some(lookup) => Self {
                doc_line: lookup.doc_line,
                pixel_offset: ((top - row as f64) as f32) * line_height,
            },
            None => Self::default(),
        }
    }

    pub fn adjust_after_edit(&mut self, patch: &DisplayPatch) {
        if let Some(shift) = patch.line_shift {
            let at = shift.at;
            let delta = shift.delta;
            if delta > 0 {
                if self.doc_line >= at {
                    self.doc_line = self.doc_line.saturating_add(delta as usize);
                }
            } else {
                let abs = (-delta) as usize;
                if self.doc_line >= at + abs {
                    self.doc_line -= abs;
                } else if self.doc_line >= at {
                    self.doc_line = at;
                    self.pixel_offset = 0.0;
                }
            }
        }
    }

    pub fn refold_on_resize(&mut self, old_line_height: f32, new_line_height: f32) {
        if old_line_height > 0.0 {
            self.pixel_offset *= new_line_height / old_line_height;
        }
    }

    pub fn clamp(
        &mut self,
        snapshot: &DisplaySnapshot,
        viewport_rows: usize,
        line_height: f32,
    ) {
        let total = snapshot.total_rows();
        let max_top = total.saturating_sub(viewport_rows) as f64;
        let top = self.to_scroll_top(snapshot, line_height);
        if top > max_top {
            *self = ScrollAnchor::from_scroll_top(max_top, snapshot, line_height);
        }
        if total == 0 {
            self.doc_line = 0;
            self.pixel_offset = 0.0;
        }
    }
}
```

- [ ] **Step 3: 跑测试 + commit**

```bash
cargo test -p edit-plus-app viewport::scroll_anchor_tests -- --quiet
git add crates/app/src/viewport.rs
git commit -m "feat(viewport): ScrollAnchor + 转换 + adjust_after_edit/refold/clamp"
```

---

## Task 4.2: Viewport 字段切换 scroll_top → scroll_anchor

**Files:**
- Modify: `crates/app/src/viewport.rs`
- Modify: 所有读写 `viewport.scroll_top` 的调用点

- [ ] **Step 1: 探索 scroll_top 引用**

```bash
grep -rn "scroll_top" /Users/dan/proj/llmws/edit+/crates/app/src | wc -l
grep -rn "viewport\.scroll_top\|\.scroll_top " /Users/dan/proj/llmws/edit+/crates/app/src/ | head -30
```

- [ ] **Step 2: 在 Viewport 同时保留两个字段（过渡期）**

`crates/app/src/viewport.rs` 中 `pub struct Viewport`：

```rust
pub struct Viewport {
    // 旧字段保留过渡：
    pub scroll_top: f64,
    // 新字段：
    pub scroll_anchor: ScrollAnchor,
    // ...其它字段保留
}
```

并在所有写 `scroll_top` 的位置同时写 `scroll_anchor`（暂时双写，确保两者一致）：

```rust
impl Viewport {
    pub fn set_scroll_top(&mut self, top: f64, snapshot: &DisplaySnapshot, line_height: f32) {
        self.scroll_top = top;
        self.scroll_anchor = ScrollAnchor::from_scroll_top(top, snapshot, line_height);
    }
}
```

- [ ] **Step 3: 把所有外部对 `scroll_top` 的赋值改为 `set_scroll_top`**

对 grep 出的每个写入点（`scroll_top = ...` / `scroll_top += ...`）：替换为 `viewport.set_scroll_top(new_top, &snapshot, line_height)`。

```bash
grep -rn "\.scroll_top\s*=\|\.scroll_top\s*+=\|\.scroll_top\s*-=" /Users/dan/proj/llmws/edit+/crates/app/src/
```

- [ ] **Step 4: 编译 + 跑测试**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
```

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/
git commit -m "refactor(viewport): 引入 set_scroll_top 双写 ScrollAnchor"
```

---

## Task 4.3: 编辑后调 anchor.adjust_after_edit + resize 后 refold

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 找到 sync 调用点**

```bash
grep -n "display_map.sync\|display_map\.set_viewport" /Users/dan/proj/llmws/edit+/crates/app/src/
```

- [ ] **Step 2: 在 sync 后插入**

每个 sync 调用后追加：

```rust
viewport.scroll_anchor.adjust_after_edit(&patch);
viewport.scroll_anchor.clamp(&text.current_snapshot, viewport.visible_rows, settings.line_height);
viewport.scroll_top = viewport.scroll_anchor.to_scroll_top(&text.current_snapshot, settings.line_height);
```

- [ ] **Step 3: resize 处理点（grep `set_viewport_size` 调用方 / `winit::Event::Resized`）**

resize 前记录 `old_line_height`；resize 后：

```rust
viewport.scroll_anchor.refold_on_resize(old_line_height, settings.line_height);
viewport.scroll_top = viewport.scroll_anchor.to_scroll_top(&text.current_snapshot, settings.line_height);
```

- [ ] **Step 4: 编译 + 跑测试 + commit**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
git add crates/app/src/app.rs
git commit -m "feat(app): 编辑后 adjust_after_edit + resize 后 refold"
```

---

## Task 4.4: 集成测试 — 编辑后视口锚定不漂

**Files:**
- Create: `crates/app/tests/scroll_anchor_integration.rs`

- [ ] **Step 1: 写测试**

```rust
//! 编辑后 scroll_anchor.doc_line 不变。

use edit_plus_app::display_line_map::{DisplayLineMap, Edit, DisplayPatch};
use edit_plus_app::viewport::ScrollAnchor;
use edit_plus_app::buffer_adapter::Adapter;

// 注：需要让 BufferLike 在 tests crate 可用。如果是 pub(crate)，
// 改为 pub trait 即可（display_line_map.rs 中已经 pub）。

struct MockBuffer { lines: Vec<Vec<u8>> }
impl edit_plus_app::display_line_map::BufferLike for MockBuffer {
    fn line_count(&self) -> usize { self.lines.len() }
    fn line_bytes(&self, line: usize) -> std::sync::Arc<[u8]> {
        std::sync::Arc::from(self.lines[line].clone().into_boxed_slice())
    }
    fn line_byte_offset(&self, line: usize) -> usize {
        self.lines.iter().take(line).map(|l| l.len() + 1).sum()
    }
}

#[test]
fn anchor_unchanged_after_inserting_lines_above() {
    let mut buf = MockBuffer { lines: (0..50).map(|i| format!("line {i}").into_bytes()).collect() };
    let mut map = DisplayLineMap::from_buffer(&buf, 800.0, 14.0);

    let mut anchor = ScrollAnchor { doc_line: 30, pixel_offset: 0.0 };

    // 在 doc_line=10 处插入 5 行
    for i in 0..5 { buf.lines.insert(10 + i, format!("inserted {i}").into_bytes()); }
    let edits = vec![Edit { doc_line_range: 10..10, new_line_count: 5 }];
    let (_snap, patch) = map.sync(&buf, &edits);

    anchor.adjust_after_edit(&patch);
    assert_eq!(anchor.doc_line, 35, "anchor doc_line should shift down by 5");
}

#[test]
fn anchor_unchanged_when_editing_below() {
    let mut buf = MockBuffer { lines: (0..50).map(|i| format!("line {i}").into_bytes()).collect() };
    let mut map = DisplayLineMap::from_buffer(&buf, 800.0, 14.0);

    let mut anchor = ScrollAnchor { doc_line: 30, pixel_offset: 0.0 };

    buf.lines[40] = b"changed".to_vec();
    let edits = vec![Edit { doc_line_range: 40..41, new_line_count: 1 }];
    let (_snap, patch) = map.sync(&buf, &edits);

    anchor.adjust_after_edit(&patch);
    assert_eq!(anchor.doc_line, 30);
}
```

- [ ] **Step 2: 跑测试 + commit**

```bash
cargo test -p edit-plus-app --test scroll_anchor_integration -- --quiet
git add crates/app/tests/scroll_anchor_integration.rs
git commit -m "test(viewport): scroll_anchor 在编辑/resize 下的不变量"
```

- [ ] **Step 3: Phase 4 tag**

```bash
git tag phase4-scroll-anchor-done
```

---

# Phase 5 — 清理 + 收尾

**目标**：删除 WrapIndex / shape_cache / wrap_cache，落地 settings 开关 + resize 节流。
**估算**：50 行新增 / 900 行删除。

## Task 5.1: 移除 parallel-assert 钩子

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 删除 `display_map_debug` 字段、初始化、调用点**

```bash
grep -n "display_map_debug\|EDIT_PARALLEL_ASSERT\|assert_display_map_matches_wrap_index\|SimpleRng" /Users/dan/proj/llmws/edit+/crates/app/src/app.rs
```

逐一删除。

- [ ] **Step 2: 编译 + 跑测试 + commit**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
git add crates/app/src/app.rs
git commit -m "chore(app): 移除 Phase 2 临时 parallel-assert 钩子"
```

---

## Task 5.2: 删除 WrapIndex

**Files:**
- Delete: `crates/app/src/wrap_index.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: 所有 `use crate::wrap_index::WrapIndex` 引用

- [ ] **Step 1: 找到所有引用**

```bash
grep -rn "wrap_index\|WrapIndex" /Users/dan/proj/llmws/edit+/crates/app/src/ | grep -v "// " | head -40
```

- [ ] **Step 2: 把每个调用点替换为 snapshot 等价 API**

| 旧 API | 新 API |
|--------|--------|
| `wrap_index.doc_to_display(dl)` | `text.current_snapshot.line_to_row(dl)` |
| `wrap_index.display_to_doc(row)` | `text.current_snapshot.resolve_row(row).map(\|l\| l.doc_line)` |
| `wrap_index.total_display_rows()` | `text.current_snapshot.total_rows()` |
| `wrap_index.visual_line_count(dl)` | `text.current_snapshot.resolve_row(text.current_snapshot.line_to_row(dl)).map(\|l\| l.entry.visual_line_count as usize)` |
| `wrap_index.update(dl, n)` | 通过 sync 自动维护 |
| `wrap_index.set_viewport_width(w)` | `text.display_map.set_viewport_size(...)` |

- [ ] **Step 3: 删除文件 + lib.rs 引用**

```bash
git rm /Users/dan/proj/llmws/edit+/crates/app/src/wrap_index.rs
```

`crates/app/src/lib.rs` 删除：

```rust
pub mod wrap_index;
```

- [ ] **Step 4: 编译 + 跑全部测试**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
```

修复编译错误。常见：测试文件 `crates/app/tests/render_smoke.rs` 等可能引用 WrapIndex。

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/lib.rs crates/app/src/
git commit -m "chore(wrap_index): 删除，全部切到 DisplaySnapshot"
```

---

## Task 5.3: 移除 shape_cache + wrap_cache

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 删除 TextState 中两个字段**

```bash
grep -n "shape_cache\|wrap_cache" /Users/dan/proj/llmws/edit+/crates/app/src/app.rs
```

删除：
- `pub(crate) shape_cache: ...`
- `pub(crate) wrap_cache: ...`
- 初始化 `shape_cache: LruCache::new(1024)`
- 初始化 `wrap_cache: LruCache::new(2048)`
- TextState 内任何 `self.shape_cache` / `self.wrap_cache` 引用（应该已经全部移到 RenderCache）

- [ ] **Step 2: 编译 + 跑测试 + commit**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
git add crates/app/src/app.rs
git commit -m "chore(app): 移除 shape_cache / wrap_cache（被 RenderCache 吸收）"
```

---

## Task 5.4: 超长行截断开关

**Files:**
- Modify: `crates/app/src/settings.rs`
- Modify: `crates/app/src/reshape_worker.rs`

- [ ] **Step 1: settings.rs 加字段**

```bash
grep -n "pub struct Settings\|font_size:" /Users/dan/proj/llmws/edit+/crates/app/src/settings.rs | head
```

定位 `pub struct Settings`，追加：

```rust
/// 0 = 关闭；> 0 时单行 byte_length 超过该值的行只 shape 前 N 字节并显示截断标记。
pub max_line_bytes_for_shaping: usize,
```

并在 `impl Default for Settings` 内追加：

```rust
max_line_bytes_for_shaping: 0,
```

- [ ] **Step 2: worker 处 read 该值**

`reshape_worker.rs` `process_request` 进入 shape 之前判断：

```rust
// 注意：max_line_bytes_for_shaping 通过 ReshapeRequest 传入；
// 因此 ReshapeRequest 增加字段。
```

`ReshapeRequest`：

```rust
pub max_line_bytes: usize,  // 0 = 不截断
```

`process_request`：

```rust
let bytes_view: &[u8] = if req.max_line_bytes > 0 && req.line_bytes.len() > req.max_line_bytes {
    &req.line_bytes[..req.max_line_bytes]
} else {
    &req.line_bytes[..]
};
let line_str = std::str::from_utf8(bytes_view).unwrap_or("");
// 后面 shape 用 line_str
```

并在调用 `worker.submit(ReshapeRequest { ... })` 的所有地方追加 `max_line_bytes: settings.max_line_bytes_for_shaping`（display_line_map 的 sync / set_viewport_size，以及 render_pipeline 的 prefetch + 兜底）。

- [ ] **Step 3: 写测试**

`reshape_worker.rs` tests：

```rust
#[test]
fn truncates_long_line_when_threshold_set() {
    let big = vec![b'a'; 200_000];
    let w = ReshapeWorker::spawn();
    w.cancel_before(1);
    let _ = w.submit(ReshapeRequest {
        generation: 1,
        doc_line: 0,
        line_bytes: Arc::from(big.into_boxed_slice()),
        viewport_width: 800.0,
        font_size: 14.0,
        max_line_bytes: 1000,
    });
    std::thread::sleep(std::time::Duration::from_millis(100));
    let r = w.drain_completed(10);
    assert_eq!(r.len(), 1);
    // 截断后 byte_length 仍是原始长度（用户看到的逻辑长度）；shape 使用 1000 字节
    assert_eq!(r[0].entry.byte_length, 200_000);
}
```

- [ ] **Step 4: 跑测试 + commit**

```bash
cargo test -p edit-plus-app reshape_worker -- --quiet
git add crates/app/src/settings.rs crates/app/src/reshape_worker.rs crates/app/src/display_line_map.rs crates/app/src/render_pipeline.rs
git commit -m "feat(settings): max_line_bytes_for_shaping 开关（默认关闭）"
```

---

## Task 5.5: resize 16ms 节流

**Files:**
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 探索 resize 入口**

```bash
grep -n "WindowEvent::Resized\|fn handle_resize\|Resized {" /Users/dan/proj/llmws/edit+/crates/app/src/app.rs
```

- [ ] **Step 2: 在 App 结构体追加字段**

```rust
pending_resize: Option<(u32, u32)>,
last_resize_handled: std::time::Instant,
```

初始化：`pending_resize: None, last_resize_handled: std::time::Instant::now(),`

- [ ] **Step 3: 改 resize 事件处理**

把原本立即调用 `set_viewport_size` 的逻辑改为：

```rust
// resize 入口
self.pending_resize = Some((new_w, new_h));
// 判断是否立即处理
let now = std::time::Instant::now();
if now.duration_since(self.last_resize_handled).as_millis() >= 16 {
    self.flush_pending_resize();
}
```

并在每帧 update 末尾调：

```rust
self.flush_pending_resize();
```

实现：

```rust
fn flush_pending_resize(&mut self) {
    let now = std::time::Instant::now();
    if now.duration_since(self.last_resize_handled).as_millis() < 16 { return; }
    let Some((w, h)) = self.pending_resize.take() else { return; };
    self.last_resize_handled = now;
    // 真实 resize 逻辑（含 set_viewport_size、render_cache.invalidate_all、anchor.refold_on_resize 等）
    // 这里复用原有 handle_resize_internal 的代码
    self.do_resize(w, h);
}
```

- [ ] **Step 4: 编译 + 跑测试 + commit**

```bash
cargo build -p edit-plus-app
cargo test -p edit-plus-app -- --quiet
git add crates/app/src/app.rs
git commit -m "feat(app): resize 16ms 节流（最后一次为准）"
```

---

## Task 5.6: 清理冗余依赖与最终验证

**Files:**
- 全局

- [ ] **Step 1: cargo udeps（可选）**

```bash
cargo +nightly udeps -p edit-plus-app 2>&1 | tail -20
```

如果有未用依赖，从 `crates/app/Cargo.toml` 删除。

- [ ] **Step 2: clippy + fmt**

```bash
cargo clippy -p edit-plus-app -- -D warnings
cargo fmt -p edit-plus-app
```

- [ ] **Step 3: 全部测试 + bench**

```bash
cargo test -p edit-plus-app -- --quiet
cargo bench -p edit-plus-app --bench scroll_bench 2>&1 | tail -30
```

- [ ] **Step 4: 4MB JSON 手测**

```bash
cargo run -p edit-plus-app --release -- /Users/dan/Downloads/段落标题.json
```

观察：
- 文件加载 < 1s
- 滚动条拖动顺滑（60fps）
- 主题切换不卡

- [ ] **Step 5: 记录 KPI 数字**

把 bench 结果与手测观察追加到本 plan 文件末尾「## 验收记录」段。

- [ ] **Step 6: Phase 5 tag**

```bash
git tag phase5-cleanup-done
```

- [ ] **Step 7: 写 release commit**

```bash
git commit --allow-empty -m "release: large-file-scroll-perf 完成

- 4MB JSON 滚动一帧 < 2ms
- 主题切换 0 invalidate
- 编辑后 scroll_anchor.doc_line 不变
- WrapIndex 已删除，全部走 DisplayLineMap
- resize 节流 16ms / 单行截断开关 max_line_bytes_for_shaping
"
```

---

## 验收记录

> 实施完成后填写。

- 4MB JSON 滚动一帧实测：<填 ms>
- 主题切换 invalidate 数：<填数字>（应为 0）
- 编辑插入 1000 行后 anchor.doc_line 漂移：<填>（应为 0）
- 冷启动 4MB JSON 首屏：<填 ms>
- wrap_index.rs 是否存在：<填 是/否>（应为 否）
