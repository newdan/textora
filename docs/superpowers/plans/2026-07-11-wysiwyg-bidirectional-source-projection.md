# WYSIWYG 双向源码—视觉投影层 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 `textora-markdown` 建立唯一、可验证、支持 cursor affinity 的双向源码—视觉投影层，彻底替代 WYSIWYG 可编辑文本的 source byte 猜测。

**Architecture:** Parser/Builder 在 Markdown 折叠发生时保存精确源码关系，`ProjectedText` 在 wrapping 前统一承载直接、折叠和虚拟文本投影；layout 只切片投影，`SourceProjectionIndex` 负责双向查询。Cursor rect、hit-test、导航、selection、IME、列表、表格和空行逐阶段迁移，最后删除 legacy fallback。

**Tech Stack:** Rust、pulldown-cmark offset events、textora-markdown、Unicode extended grapheme、Shaper、cargo test。

## Global Constraints

- 产品名是 textora，Markdown crate 包名是 `textora-markdown`。
- 源码持久坐标始终是 UTF-8 byte boundary；视觉停靠单位始终是 Unicode extended grapheme cluster boundary。
- 像素 advance 只负责 grapheme 与 x 的转换，不参与源码 byte 推导。
- `ProjectionSpan::visual_range` 使用 `ProjectedText.text` 内部 UTF-8 byte range，仅用于文本变换和 wrapping 切片；所有可停靠位置仍只使用 grapheme-indexed `boundaries`。
- 每个可编辑 `LaidOutLine` 必须携带完整投影；缺失投影不得静默回退到文本长度计算。
- 单个 visual line 的 source byte 单调不减；合法重复边界必须带 `Upstream`/`Downstream` affinity。
- 投影绑定 `source_generation: u32` 和 `layout_revision: u64`；不得混用不同 generation/revision。
- `crates/ui` 不得依赖 `DocumentView`、Workspace、Commands 或 Events；投影实现只位于 `textora-markdown`。
- 不改变 Enter、Backspace、Indent、Outdent 等结构编辑策略，不改变 Markdown 渲染样式。
- 不新增 fuzz 或 Unicode 第三方依赖；复用 `core::unicode` 与现有 grapheme 工具。
- 每个任务最多修改三个生产文件；每次提交前必须通过相关测试、`cargo fmt --all -- --check` 和编译。
- Rust 禁止无说明 `.unwrap()`；确定不失败时使用带具体理由的 `.expect(...)`。

## File Structure

- Create `crates/markdown/src/projection.rs`：投影类型、构建器、验证器、wrapping 切片、双向索引和纯单元测试。
- Modify `crates/markdown/src/lib.rs`：声明内部 `projection` 模块。
- Modify `crates/markdown/src/builder.rs`：在 parser event range 尚完整时构造每个逻辑文本行的 `ProjectedText`。
- Modify `crates/markdown/src/edit.rs`：把 marker 展开和 IME preedit 实现为投影变换。
- Modify `crates/markdown/src/layout/block.rs`：所有 Markdown 文本块用 `ProjectedText` wrapping，并为每个 `LaidOutLine` 切出投影。
- Modify `crates/markdown/src/layout/types.rs`：保存视觉行投影、构造 `SourceProjectionIndex`，最终删除 legacy fallback。
- Modify `crates/markdown/src/layout/source_line_map.rs`：把可编辑空行和隐藏间隔转换为投影语义。
- Modify `crates/markdown/src/view.rs`：所有 WYSIWYG cursor 消费端改用统一索引。
- Modify `crates/markdown/src/selection.rs`：selection 文本与高亮使用投影边界，不再直接读取旧 map。
- Modify `crates/ui/src/plugin.rs`：向 App 暴露纯数据 visual line 几何，用于跨层真实坐标回归，不暴露 markdown 投影类型。
- Modify `crates/app/src/dispatch/mouse.rs`：投影 query 失败时保持现有 cursor/selection。
- Modify `crates/app/src/app_tests.rs`：添加非自指向的真实交互回归测试。

---

### Task 1: 建立投影核心类型和不变量验证器

**Files:**

- Create: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/lib.rs`
- Test: `crates/markdown/src/projection.rs`

**Interfaces:**

- Consumes: `crate::grapheme_map::{grapheme_count, grapheme_index_at_byte}`。
- Produces: `CursorAffinity`、`SourceAnchor`、`ProjectionSpanKind`、`ProjectionSpan`、`ProjectedText`、`ProjectionError`、`ProjectedText::validate()`。

- [ ] **Step 1: 写投影不变量失败测试**

在新文件底部先写以下测试，测试引用的类型尚不存在，编译应失败：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_ascii_projection_has_one_boundary_per_grapheme_plus_sentinel() {
        let projected = ProjectedText::direct("abc", 5);
        assert_eq!(projected.text, "abc");
        assert_eq!(
            projected.boundaries.iter().map(|anchor| anchor.byte).collect::<Vec<_>>(),
            vec![5, 6, 7, 8]
        );
        assert_eq!(projected.validate(".....abc"), Ok(()));
    }

    #[test]
    fn validation_rejects_non_monotonic_source_boundaries() {
        let projected = ProjectedText {
            text: "ab".to_string(),
            spans: Vec::new(),
            boundaries: vec![
                SourceAnchor::downstream(4),
                SourceAnchor::downstream(3),
                SourceAnchor::downstream(5),
            ],
        };
        assert_eq!(
            projected.validate("....."),
            Err(ProjectionError::NonMonotonicSourceOrder { previous: 4, current: 3 })
        );
    }

    #[test]
    fn validation_rejects_boundary_count_mismatch() {
        let projected = ProjectedText {
            text: "👨\u{200d}👩".to_string(),
            spans: Vec::new(),
            boundaries: vec![SourceAnchor::downstream(0)],
        };
        assert!(matches!(
            projected.validate("👨\u{200d}👩"),
            Err(ProjectionError::BoundaryCountMismatch { .. })
        ));
    }
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib projection::tests -- --nocapture`

Expected: FAIL，错误包含 `unresolved import` 或 `cannot find type ProjectedText`。

- [ ] **Step 3: 实现核心类型与验证器**

在 `lib.rs` 增加：

```rust
pub(crate) mod projection;
```

在 `projection.rs` 实现以下完整公开面；错误类型字段必须保持不变，后续任务直接匹配：

```rust
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CursorAffinity {
    Upstream,
    Downstream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceAnchor {
    pub byte: usize,
    pub affinity: CursorAffinity,
}

impl SourceAnchor {
    pub(crate) const fn upstream(byte: usize) -> Self {
        Self { byte, affinity: CursorAffinity::Upstream }
    }

    pub(crate) const fn downstream(byte: usize) -> Self {
        Self { byte, affinity: CursorAffinity::Downstream }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionSpanKind {
    Direct,
    Collapsed,
    Virtual { anchor_byte: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionSpan {
    pub source_range: Range<usize>,
    pub visual_range: Range<usize>,
    pub kind: ProjectionSpanKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectedText {
    pub text: String,
    pub spans: Vec<ProjectionSpan>,
    pub boundaries: Vec<SourceAnchor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionError {
    BoundaryCountMismatch { expected: usize, actual: usize },
    InvalidSourceBoundary { byte: usize },
    NonMonotonicSourceOrder { previous: usize, current: usize },
    UnclassifiedDuplicateBoundary { byte: usize },
    StaleGeneration { expected: u32, actual: u32 },
    MissingEditableProjection { flat_line_idx: usize },
}

impl ProjectedText {
    pub(crate) fn direct(text: &str, source_start: usize) -> Self {
        let mut boundaries = text
            .char_indices()
            .map(|(relative_byte, _)| SourceAnchor::downstream(source_start + relative_byte))
            .collect::<Vec<_>>();
        boundaries.push(SourceAnchor::downstream(source_start + text.len()));
        let boundaries = grapheme_boundary_anchors(text, &boundaries);
        Self {
            text: text.to_string(),
            spans: vec![ProjectionSpan {
                source_range: source_start..source_start + text.len(),
                visual_range: 0..text.len(),
                kind: ProjectionSpanKind::Direct,
            }],
            boundaries,
        }
    }

    pub(crate) fn grapheme_count(&self) -> usize {
        crate::grapheme_map::grapheme_count(&self.text)
    }

    pub(crate) fn validate(&self, source: &str) -> Result<(), ProjectionError> {
        let expected = self.grapheme_count() + 1;
        if self.boundaries.len() != expected {
            return Err(ProjectionError::BoundaryCountMismatch {
                expected,
                actual: self.boundaries.len(),
            });
        }
        let mut previous = None;
        for anchor in &self.boundaries {
            if anchor.byte > source.len() || !source.is_char_boundary(anchor.byte) {
                return Err(ProjectionError::InvalidSourceBoundary { byte: anchor.byte });
            }
            if let Some(previous_byte) = previous
                && anchor.byte < previous_byte
            {
                return Err(ProjectionError::NonMonotonicSourceOrder {
                    previous: previous_byte,
                    current: anchor.byte,
                });
            }
            previous = Some(anchor.byte);
        }
        Ok(())
    }
}
```

在 `projection.rs` 增加以下私有 helper。它让现有 grapheme map 对唯一 char ordinal 分组，再取回完整 anchor，因此不复制 Unicode 分段算法：

```rust
fn grapheme_boundary_anchors(
    text: &str,
    char_anchors: &[SourceAnchor],
) -> Vec<SourceAnchor> {
    let char_count = text.chars().count();
    assert_eq!(
        char_anchors.len(),
        char_count + 1,
        "char anchors must include one sentinel"
    );
    let char_ordinals = (0..=char_count).collect::<Vec<_>>();
    crate::grapheme_map::build_visual_grapheme_map(text, &char_ordinals)
        .as_slice()
        .iter()
        .map(|&ordinal| char_anchors[ordinal])
        .collect()
}
```

不得修改 `grapheme_map.rs`，也不得复制第二份 grapheme 实现。

- [ ] **Step 4: 运行 GREEN 和格式检查**

Run: `cargo test -p textora-markdown --lib projection::tests -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: 全部 PASS。

- [ ] **Step 5: 提交投影核心**

```bash
git add crates/markdown/src/lib.rs crates/markdown/src/projection.rs
git commit -m "feat(markdown): add source projection primitives"
```

---

### Task 2: 在 Builder 阶段保留 Text 与 SoftBreak 的源码关系

**Files:**

- Modify: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/builder.rs`
- Modify: `crates/markdown/src/edit.rs`（仅更新 `BlockNode` 测试夹具的新增字段）
- Test: `crates/markdown/src/builder.rs`

**Interfaces:**

- Consumes: Task 1 的 `ProjectedText`、`ProjectionSpan`、`SourceAnchor`。
- Produces: `TextProjectionBuilder::{push_direct,push_soft_break,finish}`；`BlockNode::projected_lines: Vec<ProjectedText>`。

- [ ] **Step 1: 写连续引用和普通 softbreak 的失败测试**

在 `builder.rs` 测试模块加入：

```rust
#[test]
fn builder_preserves_blockquote_softbreak_source_jump() {
    let source = "> first\n> second";
    let parsed = crate::parser::parse_markdown(source);
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parsed, &style);
    let paragraph = &doc.blocks[0].children[0];
    let projected = &paragraph.projected_lines[0];
    let space = projected.text.find(' ').expect("softbreak must render as a space");
    let grapheme = crate::grapheme_map::grapheme_index_at_byte(&projected.text, space);
    assert_eq!(projected.boundaries[grapheme].byte, "> first".len());
    assert_eq!(
        projected.boundaries[grapheme + 1].byte,
        source.find("second").expect("fixture must contain second")
    );
}

#[test]
fn builder_plain_softbreak_maps_newline_to_one_visual_space() {
    let source = "first\nsecond";
    let parsed = crate::parser::parse_markdown(source);
    let style = crate::test_utils::default_style();
    let doc = MarkdownDoc::build(&parsed, &style);
    let projected = &doc.blocks[0].projected_lines[0];
    assert_eq!(projected.text, "first second");
    assert_eq!(projected.boundaries[6].byte, 6);
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib builder::tests::builder_preserves_blockquote_softbreak_source_jump -- --exact`

Expected: FAIL，`BlockNode` 尚无 `projected_lines`。

- [ ] **Step 3: 实现 TextProjectionBuilder**

在 `projection.rs` 增加：

```rust
#[derive(Default)]
pub(crate) struct TextProjectionBuilder {
    text: String,
    spans: Vec<ProjectionSpan>,
    char_anchors: Vec<SourceAnchor>,
    pending_gap_start: Option<usize>,
}

impl TextProjectionBuilder {
    pub(crate) fn push_direct(&mut self, text: &str, source_range: Range<usize>) {
        if let Some(gap_start) = self.pending_gap_start.take() {
            let visual_start = self.text.len();
            self.text.push(' ');
            self.char_anchors.push(SourceAnchor::upstream(gap_start));
            self.spans.push(ProjectionSpan {
                source_range: gap_start..source_range.start,
                visual_range: visual_start..self.text.len(),
                kind: ProjectionSpanKind::Collapsed,
            });
        }
        let visual_start = self.text.len();
        self.text.push_str(text);
        self.char_anchors.extend(
            text.char_indices()
                .map(|(offset, _)| SourceAnchor::downstream(source_range.start + offset)),
        );
        self.spans.push(ProjectionSpan {
            source_range,
            visual_range: visual_start..self.text.len(),
            kind: ProjectionSpanKind::Direct,
        });
    }

    pub(crate) fn push_soft_break(&mut self, event_range: Range<usize>) {
        self.pending_gap_start = Some(event_range.start);
    }

    pub(crate) fn finish(mut self, source_end: usize) -> ProjectedText {
        self.char_anchors.push(SourceAnchor::downstream(source_end));
        let boundaries = grapheme_boundary_anchors(&self.text, &self.char_anchors);
        ProjectedText { text: self.text, spans: self.spans, boundaries }
    }
}
```

修改 `PendingLine`，让文本、styles 和 projection builder 同生命周期；`MarkdownEvent::Text` 调用 `push_text_with_source(text, current_event_range.clone())`，`MarkdownEvent::SoftBreak` 只调用 `push_soft_break(current_event_range.clone())`，不再直接 `push_text(" ")`。`flush_line_to_vec()` 同时返回 `ProjectedText`，写入新字段：

```rust
pub struct BlockNode {
    // existing fields unchanged
    pub projected_lines: Vec<crate::projection::ProjectedText>,
}
```

所有 `BlockNode` 构造处显式初始化 `projected_lines: Vec::new()`；Novel zero-copy 构造路径保持为空，表示它不参与 Markdown WYSIWYG 投影。

- [ ] **Step 4: 运行 Builder 全测**

Run: `cargo test -p textora-markdown --lib builder::tests -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；现有 parser/builder 结构测试无回归。

- [ ] **Step 5: 提交 Builder 来源保留**

```bash
git add crates/markdown/src/projection.rs crates/markdown/src/builder.rs crates/markdown/src/edit.rs
git commit -m "feat(markdown): preserve source projection during parsing"
```

---

### Task 3: 把 marker 展开与 IME preedit 变成投影变换

**Files:**

- Modify: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/edit.rs`
- Test: `crates/markdown/src/edit.rs`

**Interfaces:**

- Consumes: `ProjectedText`、`ActiveBlockMarker`、`EditContext`。
- Produces: `materialize_projected_line(base, spans, source, edit_ctx, source_line)`；`ProjectedText::{prepend_direct,insert_virtual}`。

- [ ] **Step 1: 写 marker 和 preedit 失败测试**

```rust
#[test]
fn materialized_heading_marker_keeps_absolute_source_anchors() {
    let base = crate::projection::ProjectedText::direct("Title", 2);
    let marker = ActiveBlockMarker {
        marker_text: "# ".to_string(),
        marker_source_range: 0..2,
    };
    let projected = materialize_block_marker(base, &marker);
    assert_eq!(projected.text, "# Title");
    assert_eq!(
        projected.boundaries.iter().map(|anchor| anchor.byte).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4, 5, 6, 7]
    );
}

#[test]
fn preedit_visual_text_anchors_every_boundary_to_cursor_byte() {
    let base = crate::projection::ProjectedText::direct("ab", 10);
    let projected = base.insert_virtual(1, "中文", 11);
    assert_eq!(projected.text, "a中文b");
    assert_eq!(projected.boundaries[1].byte, 11);
    assert_eq!(projected.boundaries[2].byte, 11);
    assert_eq!(projected.boundaries[3].byte, 11);
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib edit::tests::materialized_heading_marker_keeps_absolute_source_anchors -- --exact`

Expected: FAIL，缺少 `materialize_block_marker`。

- [ ] **Step 3: 实现投影变换并保留旧 API 兼容层**

实现以下签名：

```rust
pub(crate) fn materialize_block_marker(
    base: crate::projection::ProjectedText,
    marker: &ActiveBlockMarker,
) -> crate::projection::ProjectedText;

pub(crate) fn materialize_projected_line(
    base: &crate::projection::ProjectedText,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
    source_line: Option<&SourceLineContext>,
) -> crate::projection::ProjectedText;
```

`prepend_direct()` 必须使用 marker 的绝对 range 构造边界并整体平移原 `visual_range`；`insert_virtual()` 必须在 grapheme 边界插入文本，插入的所有边界均使用 `Virtual { anchor_byte }`。旧 `materialize_line_with_source_context()` 暂时调用新函数后转换回 `MaterializedLine`，确保 Task 4 之前现有调用者仍能编译。

- [ ] **Step 4: 运行 edit 和 grapheme 回归**

Run: `cargo test -p textora-markdown --lib edit::tests -- --nocapture && cargo test -p textora-markdown --lib grapheme_map::tests -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS。

- [ ] **Step 5: 提交投影 materialization**

```bash
git add crates/markdown/src/projection.rs crates/markdown/src/edit.rs
git commit -m "refactor(markdown): materialize markers as source projections"
```

---

### Task 4: Wrapping 全量切片投影并覆盖 heading/blockquotes

**Files:**

- Modify: `crates/markdown/src/edit.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/selection.rs`（仅更新 `FlatLine` 测试夹具的新字段）
- Test: `crates/markdown/src/layout/types.rs`

**Interfaces:**

- Consumes: `BlockNode::projected_lines`、`materialize_projected_line()`。
- Produces: `LaidOutLine::source_projection: Option<VisualLineProjection>`；`ProjectedText::slice_visual_line()`。

- [ ] **Step 1: 写长 heading 和连续 blockquote 失败测试**

在 `layout/types.rs` 测试模块增加可控制宽度的精确布局 helper：

```rust
fn layout_with_cursor_and_width(
    source: &str,
    cursor_byte: usize,
    width: f32,
) -> LazyLayout<crate::builder::MarkdownDoc> {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = core::document::StringDocView::new(source);
    let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
    lazy.set_edit_source(Some(source.to_string()));
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    let mut shaper = shaping::Shaper::new().expect("projection test needs a shaper");
    lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
    lazy.build_flat_lines(&doc_view);
    lazy
}
```

随后加入：

```rust
#[test]
fn active_wrapped_heading_gives_every_segment_explicit_projection() {
    let source = "# a heading long enough to wrap across three visual rows";
    let lazy = layout_with_cursor_and_width(source, 4, 120.0);
    let lines = &lazy.flat_lines;
    assert!(lines.len() >= 3);
    assert!(lines.iter().all(|line| line.source_projection.is_some()));
}

#[test]
fn consecutive_blockquote_projection_jumps_over_second_marker() {
    let source = "> first physical line\n> second physical line";
    let second = source.find("second").expect("fixture must contain second");
    let lazy = layout_with_cursor_and_width(source, second, 180.0);
    let second_line = lazy
        .flat_lines
        .iter()
        .find(|line| line.text.contains("second"))
        .expect("second text must be visible");
    let projection = second_line.source_projection.as_ref().expect("projection required");
    assert!(projection.boundaries.iter().any(|anchor| anchor.byte == second));
    assert!(!projection.boundaries.iter().any(|anchor| anchor.byte == 0));
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib layout::types::tests::consecutive_blockquote_projection_jumps_over_second_marker -- --exact`

Expected: FAIL，`FlatLine`/`LaidOutLine` 尚无 `source_projection`。

- [ ] **Step 3: 接入 wrapping 投影切片**

新增并统一使用：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollapsedBoundary {
    pub source_range: Range<usize>,
    pub upstream_grapheme: usize,
    pub downstream_grapheme: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisualLineProjection {
    pub flat_line_idx: usize,
    pub boundaries: Vec<SourceAnchor>,
    pub source_extent: Range<usize>,
    pub collapsed: Vec<CollapsedBoundary>,
}

impl ProjectedText {
    pub(crate) fn slice_visual_line(
        &self,
        flat_line_idx: usize,
        visual_byte_range: Range<usize>,
    ) -> Result<VisualLineProjection, ProjectionError>;
}
```

`layout_text_block()` 不再根据 `line_styles.is_empty()` 决定是否生成 map：每个 `raw` 都从对应 `projected_lines[line_idx]` 开始，先执行 materialization，再 wrapping，再按 `WrappedLine.byte_start..byte_end` 切片。活动 marker 在 wrapping 前加入，删除本路径对 `prepend_marker_to_line()` 的调用。

同一任务把 code block 的 `code_line_source_starts` 转成 `ProjectedText::direct(line_text, source_start)`，把 metadata 每个物理行转换成 direct projection，并让活动 horizontal rule 复用其 Builder projection。列表和表格仍由 Task 8/9 单独迁移。

`LaidOutLine` 和 `FlatLine` 增加 `source_projection`，`push_flat_line()` 原样复制；旧 `source_bytes_by_visual_grapheme` 暂时从 `projection.boundaries` 派生，供尚未迁移的消费者兼容。

- [ ] **Step 4: 运行布局与真实 promotion 定向测试**

Run: `cargo test -p textora-markdown --lib layout::types::tests -- --nocapture && cargo test -p textora-markdown --lib view::wysiwyg_tests::promotion_blockquote_click_roundtrip_and_vertical_navigation_reach_line_three -- --exact && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；每个可编辑 heading/paragraph/blockquote 分段都有显式投影。

- [ ] **Step 5: 提交 wrapping 切换**

```bash
git add crates/markdown/src/edit.rs crates/markdown/src/layout/block.rs crates/markdown/src/layout/types.rs crates/markdown/src/selection.rs
git commit -m "feat(markdown): carry projections through wrapped layout"
```

---

### Task 5: 构建 generation-safe 双向 SourceProjectionIndex

**Files:**

- Modify: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Test: `crates/markdown/src/projection.rs`

**Interfaces:**

- Consumes: `VisualLineProjection`。
- Produces: `VisualPosition`、`CollapsedSourceRange`、`SourceProjectionIndex::{build,source_anchor_at,visual_position_for_source,visual_lines}`。

- [ ] **Step 1: 写 affinity、stale generation 和有序遍历测试**

```rust
#[test]
fn reverse_index_uses_requested_affinity_at_shared_wrap_boundary() {
    let lines = shared_boundary_fixture();
    let index = SourceProjectionIndex::build(7, 3, lines).expect("fixture is valid");
    assert_eq!(
        index.visual_position_for_source(5, CursorAffinity::Upstream),
        Some(VisualPosition { flat_line_idx: 0, grapheme_pos: 5 })
    );
    assert_eq!(
        index.visual_position_for_source(5, CursorAffinity::Downstream),
        Some(VisualPosition { flat_line_idx: 1, grapheme_pos: 0 })
    );
}

#[test]
fn index_rejects_stale_generation_queries() {
    let index = SourceProjectionIndex::build(7, 3, shared_boundary_fixture())
        .expect("fixture is valid");
    assert_eq!(
        index.source_anchor_at(8, VisualPosition { flat_line_idx: 0, grapheme_pos: 0 }),
        Err(ProjectionError::StaleGeneration { expected: 7, actual: 8 })
    );
}

```

在 `layout/types.rs` 测试模块复用 Task 4 的 `layout_with_cursor_and_width()`，另加：

```rust
#[test]
fn evicting_shapes_does_not_remove_retained_projection_lines() {
    let source = "first paragraph\n\nsecond paragraph\n\nthird paragraph";
    let mut lazy = layout_with_cursor_and_width(source, 0, 240.0);
    let before = lazy.source_projection_index.as_ref()
        .expect("full fixture must have an index")
        .visual_lines().len();
    lazy.evict_outside(&(1..2));
    lazy.rebuild_source_projection_index().expect("retained projections are valid");
    assert_eq!(
        lazy.source_projection_index.as_ref()
            .expect("index must survive shape eviction")
            .visual_lines().len(),
        before
    );
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib projection::tests::reverse_index_uses_requested_affinity_at_shared_wrap_boundary -- --exact`

Expected: FAIL，`SourceProjectionIndex` 不存在。

- [ ] **Step 3: 实现索引并接入 LazyLayout**

保持以下确切 API：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VisualPosition {
    pub flat_line_idx: usize,
    pub grapheme_pos: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceVisualAnchor {
    pub source: SourceAnchor,
    pub visual: VisualPosition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollapsedSourceRange {
    pub source_range: Range<usize>,
    pub upstream: VisualPosition,
    pub downstream: VisualPosition,
}

pub(crate) struct SourceProjectionIndex {
    source_generation: u32,
    layout_revision: u64,
    visual_lines: Vec<VisualLineProjection>,
    reverse: Vec<SourceVisualAnchor>,
    collapsed: Vec<CollapsedSourceRange>,
}

impl SourceProjectionIndex {
    pub(crate) fn build(
        source_generation: u32,
        layout_revision: u64,
        visual_lines: Vec<VisualLineProjection>,
    ) -> Result<Self, ProjectionError>;

    pub(crate) fn source_anchor_at(
        &self,
        source_generation: u32,
        position: VisualPosition,
    ) -> Result<SourceAnchor, ProjectionError>;

    pub(crate) fn visual_position_for_source(
        &self,
        source_byte: usize,
        affinity: CursorAffinity,
    ) -> Option<VisualPosition>;

    pub(crate) fn visual_lines(&self) -> &[VisualLineProjection];
}
```

在 `LazyLayout` 新增 `retained_block_projections: Vec<Vec<VisualLineProjection>>`、`pub(crate) source_projection_index: Option<SourceProjectionIndex>`、`pub(crate) source_projection_error: Option<ProjectionError>`、`source_generation: u32` 和 `layout_revision: u64`，并增加 `set_source_generation(u32)`。每次 block 精确布局完成后更新对应 retained slot；`evict_outside()` 只逐出 `LaidOutBlock` 的 shape/绘制数据，不清空轻量投影。

保持 `build_flat_lines(doc_view)` 现有签名，内部读取 `self.source_generation`；`rebuild_source_projection_index()` 必须从所有 retained slots 构建全局索引，而不是只遍历 viewport。每次成功验证后递增 revision、清空旧 error 并原子替换索引。任何 `ProjectionError` 清空本次索引并保存到 `source_projection_error`，不能保留指向新 flat lines 的旧索引。Task 6 在每次 `MarkdownView::set_source(text, generation)` 时把 generation 同步给 `LazyLayout`。

- [ ] **Step 4: 运行索引与增量布局测试**

Run: `cargo test -p textora-markdown --lib projection::tests -- --nocapture && cargo test -p textora-markdown --lib layout::types::tests -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；反向查询为二分查找，重复边界由 affinity 唯一决定。

- [ ] **Step 5: 提交双向索引**

```bash
git add crates/markdown/src/projection.rs crates/markdown/src/layout/types.rs
git commit -m "feat(markdown): build bidirectional source projection index"
```

---

### Task 6: 切换 cursor rect 与 hit-test 到统一索引

**Files:**

- Modify: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**

- Consumes: `SourceProjectionIndex::{source_anchor_at,visual_position_for_source}`。
- Produces: `PreviewEngine::projection_index()` 内部访问器；cursor/hit-test 不再调用旧 map lookup。

- [ ] **Step 1: 把 promotion 测试改成非自指向 oracle 并确认失败**

新增 helper，通过真实显示文字找到 flat line，而不是从 source map 找坐标：

```rust
fn click_point_for_visible_text(engine: &PreviewEngine<MarkdownDoc>, needle: &str) -> (f32, f32) {
    let line = engine
        .flat_lines()
        .iter()
        .find(|line| line.text.contains(needle))
        .expect("needle must be rendered");
    let byte = line.text.find(needle).expect("needle must be in selected line");
    let grapheme = crate::grapheme_map::grapheme_index_at_byte(&line.text, byte);
    (
        line.rect.x + crate::layout::grapheme_x(line, grapheme),
        line.rect.y + line.rect.h * 0.5,
    )
}

#[test]
fn promotion_line_three_real_rect_hits_line_three_source_range() {
    let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches and campaigns.\n> Style anchor: Apple Keynote and exhibitions.\n";
    let line_three = source.find("Applicable").expect("fixture must contain line three");
    let line_four = source.find("\n> Style").expect("fixture must contain line four") + 1;
    let view = make_view(source);
    let (x, y) = click_point_for_visible_text(view.engine(), "Applicable");
    let hit = view.engine().hit_test_byte(x, y, 0.0, 0.0).expect("hit required");
    assert!((line_three..line_four).contains(&hit));
}

#[test]
fn promotion_line_three_cursor_position_matches_its_visible_text_line() {
    let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches and campaigns.\n> Style anchor: Apple Keynote and exhibitions.\n";
    let line_three = source.find("Applicable").expect("fixture must contain line three");
    let view = make_view(source);
    let visible_line = view
        .engine()
        .flat_lines()
        .iter()
        .position(|line| line.text.contains("Applicable"))
        .expect("line three text must be visible");
    assert_eq!(
        view.engine().cursor_visual_position_for_byte(
            line_three,
            CursorAffinity::Downstream,
        ),
        Some(VisualPosition { flat_line_idx: visible_line, grapheme_pos: 0 })
    );
}
```

- [ ] **Step 2: 运行 RED，确认旧实现落入错误源码行**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests::promotion_line_three_cursor_position_matches_its_visible_text_line -- --exact --nocapture`

Expected: FAIL，`cursor_visual_position_for_byte` 尚不存在。真实坐标 hit-test 测试同时保留，防止消费端再次走偏。

- [ ] **Step 3: 切换两个消费端**

在 `PreviewEngine` 增加以下内部入口，测试和 cursor rect 共用它：

```rust
#[cfg(test)]
pub(crate) fn projection_index(&self) -> &SourceProjectionIndex {
    self.lazy.as_ref()
        .and_then(|lazy| lazy.source_projection_index.as_ref())
        .expect("rendered WYSIWYG test view must publish a source projection index")
}

pub(crate) fn cursor_visual_position_for_byte(
    &self,
    source_byte: usize,
    affinity: CursorAffinity,
) -> Option<VisualPosition> {
    self.lazy.as_ref()?
        .source_projection_index.as_ref()?
        .visual_position_for_source(source_byte, affinity)
}
```

`byte_from_flat_line_and_visual_grapheme()` 改为 `source_projection_index.source_anchor_at(cached_generation, VisualPosition)`；`find_flat_and_grapheme_for_byte()` 改为 `visual_position_for_source(byte, Downstream)`。删除这两个函数内部的 binary-search duplicate heuristic，但暂不删除旧字段。

`cursor_screen_pos_for_byte()` 默认请求 `Downstream`；LineEnd 或显式左向边界调用方可在 Task 7 请求 `Upstream`。`hit_test_byte()` 从 visual position 直接取 anchor，不做 nearest-neighbor source-line fallback。

- [ ] **Step 4: 运行 hit-test/cursor 全套回归**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；promotion 非自指向测试稳定通过。

- [ ] **Step 5: 提交 cursor/hit-test 切换**

```bash
git add crates/markdown/src/projection.rs crates/markdown/src/view.rs
git commit -m "fix(markdown): route cursor and hit testing through projections"
```

---

### Task 7: 切换视觉导航、selection 与 IME

**Files:**

- Modify: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/selection.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**

- Consumes: Task 5/6 的索引查询。
- Produces: `SourceProjectionIndex::{move_horizontal,line_boundary}`；selection 和 IME 统一投影。

- [ ] **Step 1: 写无循环水平遍历、垂直导航和 preedit 测试**

```rust
#[test]
fn promotion_blockquote_left_right_traversal_is_ordered_and_terminates() {
    let source = "> first physical line\n> second physical line";
    let view = make_view(source);
    let mut byte = source.find("second").expect("fixture must contain second");
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..=source.len() * 2 {
        assert!(visited.insert(byte), "horizontal navigation must not loop at byte {byte}");
        let next = view.engine().visual_move(byte, MoveDirection::Left, None).expect("move left");
        if next == 0 {
            return;
        }
        byte = next;
    }
    panic!("left traversal did not reach document start");
}

#[test]
fn virtual_preedit_roundtrip_keeps_committed_source_byte() {
    let mut view = make_view("ab");
    view.engine_mut().handle_set_cursor_byte(1);
    view.engine_mut().set_preedit_text("中文".to_string(), Some((3, 3)));
    let rect = view.engine().cursor_screen_pos().expect("preedit cursor rect");
    let hit = view.engine().hit_test_byte(rect.0, rect.1 + rect.3 * 0.5, 0.0, 0.0);
    assert_eq!(hit, Some(1));
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests::promotion_blockquote_left_right_traversal_is_ordered_and_terminates -- --exact --nocapture`

Expected: FAIL，检测到重复 byte 或未到达文档起点。

- [ ] **Step 3: 切换导航和 selection**

实现确切方法：

```rust
pub(crate) enum HorizontalDirection { Previous, Next }
pub(crate) enum LineBoundary { Start, End }

impl SourceProjectionIndex {
    pub(crate) fn move_horizontal(
        &self,
        current_byte: usize,
        direction: HorizontalDirection,
    ) -> Option<SourceAnchor>;

    pub(crate) fn line_boundary(
        &self,
        current_byte: usize,
        boundary: LineBoundary,
    ) -> Option<SourceAnchor>;
}
```

`visual_move()` 的 Left/Right/LineStart/LineEnd 使用以上 API；Up/Down 继续按相邻 visual line 和 sticky x 选择 grapheme，再用 `source_anchor_at()` 返回 byte。`selection.rs` 的 grapheme 计数和 byte 查询只读取 `FlatLine::source_projection`；IME preedit 视觉位置读取 `Virtual` anchors，不再把 preedit byte 加到 source byte。

- [ ] **Step 4: 运行导航、selection、IME 回归**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests -- --nocapture && cargo test -p textora-markdown --lib selection::tests -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；水平遍历终止，selection 与 preedit 不产生伪 source byte。

- [ ] **Step 5: 提交剩余消费端切换**

```bash
git add crates/markdown/src/projection.rs crates/markdown/src/view.rs crates/markdown/src/selection.rs
git commit -m "fix(markdown): unify navigation selection and ime projection"
```

---

### Task 8: 统一多行列表与嵌套列表投影

**Files:**

- Modify: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/builder.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Test: `crates/markdown/src/layout/block.rs`

**Interfaces:**

- Consumes: `BlockNode::projected_lines`、`ProjectedText::slice_visual_line()`。
- Produces: list continuation marker 的 collapsed spans；list item 所有 wrapped lines 显式投影。

- [ ] **Step 1: 写多行/嵌套 list 失败测试**

在 `layout/block.rs` 测试模块增加递归查找 helper：

```rust
fn find_line_in_block<'a>(block: &'a LaidOutBlock, needle: &str) -> Option<&'a LaidOutLine> {
    match &block.kind {
        LaidOutBlockKind::Text { lines }
        | LaidOutBlockKind::MetadataBlock { lines }
        | LaidOutBlockKind::CodeBlock { lines, .. } => {
            lines.iter().find(|line| line.text.contains(needle))
        }
        LaidOutBlockKind::BlockQuote { blocks } => {
            blocks.iter().find_map(|child| find_line_in_block(child, needle))
        }
        LaidOutBlockKind::ListItem { lines, blocks, .. } => lines
            .iter()
            .find(|line| line.text.contains(needle))
            .or_else(|| blocks.iter().find_map(|child| find_line_in_block(child, needle))),
        LaidOutBlockKind::Table { header, rows, .. } => header
            .iter()
            .flatten()
            .chain(rows.iter().flatten().flatten())
            .find(|line| line.text.contains(needle)),
        LaidOutBlockKind::HorizontalRule => None,
    }
}

fn layout_with_cursor_and_width(
    source: &str,
    cursor_byte: usize,
    width: f32,
) -> LazyLayout<crate::builder::MarkdownDoc> {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = core::document::StringDocView::new(source);
    let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
    lazy.set_edit_source(Some(source.to_string()));
    lazy.set_edit_ctx(Some(crate::edit::EditContext {
        cursor_byte,
        preedit_text: None,
        preedit_cursor: None,
    }));
    let mut shaper = shaping::Shaper::new().expect("list projection test needs a shaper");
    lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
    lazy.build_flat_lines(&doc_view);
    lazy
}
```

随后加入：

```rust
#[test]
fn multiline_list_item_projection_skips_continuation_indent() {
    let source = "- first line\n  continuation line";
    let continuation = source.find("continuation").expect("fixture contains continuation");
    let lazy = layout_with_cursor_and_width(source, continuation, 400.0);
    let line = lazy
        .laid_out
        .iter()
        .flatten()
        .find_map(|block| find_line_in_block(block, "continuation"))
        .expect("continuation must have a laid-out line");
    assert!(line.source_projection.as_ref().expect("projection").boundaries
        .iter().any(|anchor| anchor.byte == continuation));
}

#[test]
fn nested_list_cursor_projection_belongs_to_inner_item() {
    let source = "- outer\n  - inner wrapped content wrapped content";
    let inner = source.find("inner").expect("fixture contains inner");
    let lazy = layout_with_cursor_and_width(source, inner, 140.0);
    assert!(lazy.flat_lines.iter().filter(|line| line.text.contains("inner"))
        .all(|line| line.source_projection.is_some()));
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib layout::block::tests::multiline_list_item_projection_skips_continuation_indent -- --exact`

Expected: FAIL，continuation 投影缺失或落在缩进 byte。

- [ ] **Step 3: 删除 list 专用线性 map 构造**

`split_list_item_source_lines()` 改为从 Builder 已保存的 `ProjectedText` 分割；continuation indent/marker 作为 `Collapsed` span 保留。`plain_source_maps` 和 `source_map_for_text_line()` 不再用于 list item。`layout_line_with_styles()` 始终接收 `&ProjectedText` 并切片。

- [ ] **Step 4: 运行列表、marker、编辑策略回归**

Run: `cargo test -p textora-markdown --lib layout::block::tests -- --nocapture && cargo test -p textora-markdown --lib view::wysiwyg_tests::hit_test_byte_roundtrip_inside_list_item_respects_indent -- --exact && cargo test -p textora-app --lib markdown_empty_list_enter_uses_structural_edit_policy -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；列表编辑行为不变。

- [ ] **Step 5: 提交列表迁移**

```bash
git add crates/markdown/src/projection.rs crates/markdown/src/builder.rs crates/markdown/src/layout/block.rs
git commit -m "fix(markdown): project multiline and nested list items"
```

---

### Task 9: 为表格单元格建立独立投影 owner

**Files:**

- Modify: `crates/markdown/src/projection.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Test: `crates/markdown/src/layout/block.rs`

**Interfaces:**

- Consumes: cell `BlockNode::projected_lines`。
- Produces: `ProjectionOwnerId`；每个 table cell/wrapped line 独立 source projection。

- [ ] **Step 1: 写多单元格唯一归属失败测试**

在 `layout/block.rs` 测试模块增加：

```rust
fn layout_doc_with_width(source: &str, width: f32) -> LaidOutDoc {
    let parsed = crate::parser::parse_markdown(source);
    let style = default_style();
    let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
    let doc_view = core::document::StringDocView::new(source);
    let mut shaper = shaping::Shaper::new().expect("table projection test needs a shaper");
    layout_doc_with_shaper(&doc.blocks, &style, width, Some(&mut shaper), None, &doc_view)
}
```

随后加入：

```rust
#[test]
fn wrapped_table_cells_keep_distinct_source_extents() {
    let source = "| left header | right header |\n| --- | --- |\n| left body wraps here | right body wraps here |";
    let laid = layout_doc_with_width(source, 220.0);
    let left = laid.blocks.iter().find_map(|block| find_line_in_block(block, "left body"))
        .expect("left body must be laid out");
    let right = laid.blocks.iter().find_map(|block| find_line_in_block(block, "right body"))
        .expect("right body must be laid out");
    let left_projection = left.source_projection.as_ref().expect("left projection");
    let right_projection = right.source_projection.as_ref().expect("right projection");
    assert_ne!(left_projection.owner, right_projection.owner);
    assert!(left_projection.source_extent.end <= right_projection.source_extent.start);
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib layout::block::tests::wrapped_table_cells_keep_distinct_source_extents -- --exact`

Expected: FAIL，缺少 owner 或 cell extent 重叠。

- [ ] **Step 3: 实现表格 owner 与 cell 投影**

增加确切类型：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ProjectionOwnerId {
    Block { block_start: usize, logical_line: usize },
    TableCell { table_start: usize, row: usize, column: usize, logical_line: usize },
    EmptyLine { source_byte: usize },
}
```

把 `owner` 加到 `VisualLineProjection`。Task 2 已让 `TableCell_` 保留自身 projected lines；`layout_table()` 在遍历 header/body 时传入 row/column，按 cell projection wrapping。flatten table 时不再给所有 cell 复用父 `block_line_base/doc_line_idx`；source index 直接读取 line projection。

- [ ] **Step 4: 运行表格布局和编辑回归**

Run: `cargo test -p textora-markdown --lib layout_table -- --nocapture && cargo test -p textora-markdown --lib table -- --nocapture && cargo test -p textora-app --lib markdown_table_enter_keeps_moved_cursor_visible -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；每个 cell 的 source extent 和 owner 唯一。

- [ ] **Step 5: 提交表格迁移**

```bash
git add crates/markdown/src/projection.rs crates/markdown/src/layout/block.rs crates/markdown/src/layout/types.rs
git commit -m "fix(markdown): assign projections to table cells"
```

---

### Task 10: 将可编辑空行纳入投影索引

**Files:**

- Modify: `crates/markdown/src/layout/source_line_map.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**

- Consumes: `SourceLineMap` 的 empty line role/geometry。
- Produces: `VisualLineProjection::empty()`；空行导航不再走非空行旁路。

- [ ] **Step 1: 写结构间空行双向导航失败测试**

```rust
#[test]
fn editable_empty_line_is_a_zero_grapheme_projection_line() {
    let source = "paragraph\n\n\nnext";
    let second_empty = "paragraph\n\n".len();
    let view = make_view(source);
    let position = view.engine().projection_index()
        .visual_position_for_source(second_empty, CursorAffinity::Downstream)
        .expect("editable empty line must be projected");
    let line = &view.engine().projection_index().visual_lines()[position.flat_line_idx];
    assert_eq!(line.owner, ProjectionOwnerId::EmptyLine { source_byte: second_empty });
    assert_eq!(line.boundaries, vec![SourceAnchor::downstream(second_empty)]);
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests::editable_empty_line_is_a_zero_grapheme_projection_line -- --exact`

Expected: FAIL，空行尚未进入 projection index。

- [ ] **Step 3: 发布空行 projection 并删除导航旁路**

在 `layout/types.rs` 为已有类型增加以下同 crate inherent impl，不新增第四个生产文件：

```rust
impl VisualLineProjection {
    pub(crate) fn empty(
        flat_line_idx: usize,
        source_byte: usize,
        owner: ProjectionOwnerId,
    ) -> Self {
        Self {
            flat_line_idx,
            boundaries: vec![SourceAnchor::downstream(source_byte)],
            source_extent: source_byte..source_byte,
            collapsed: Vec::new(),
            owner,
        }
    }
}
```

`SourceLineMap` 增加 `projected_empty_lines()`，只为 `EditableEmpty` 返回 owner、source byte、y 和 height；`HiddenBlockSeparator` 只进入 collapsed ranges。`build_flat_lines()` 将这些零 grapheme lines 与文本 lines 按 y 合并后构建 index。

`visual_move_left_from_empty_source_line()`、`visual_move_right_from_empty_source_line()`、`visual_move_from_empty_source_line()` 改为调用统一索引；确认测试通过后删除这些函数及 `previous_non_empty_source_line`/`next_non_empty_source_line` 导航用途。

- [ ] **Step 4: 运行所有空行测试**

Run: `cargo test -p textora-markdown --lib empty_line -- --nocapture && cargo test -p textora-markdown --lib layout::source_line_map::tests -- --nocapture && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS；隐藏 separator 不可点击，可编辑空行可到达且不会被跳过。

- [ ] **Step 5: 提交空行统一**

```bash
git add crates/markdown/src/layout/source_line_map.rs crates/markdown/src/layout/types.rs crates/markdown/src/view.rs
git commit -m "refactor(markdown): project editable empty lines"
```

---

### Task 11: 删除 legacy source-map fallback 和重复边界启发式

**Files:**

- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/view.rs`
- Test: `crates/markdown/src/layout/types.rs`

**Interfaces:**

- Consumes: 所有结构均已提供 `VisualLineProjection`。
- Produces: 可编辑路径只剩 `SourceProjectionIndex`；缺投影返回 `ProjectionError::MissingEditableProjection`。

- [ ] **Step 1: 写禁止 fallback 的失败测试**

```rust
#[test]
fn every_editable_flat_line_has_projection_after_full_layout() {
    let corpus = [
        "plain paragraph",
        "# wrapped heading wrapped heading wrapped heading",
        "> first\n> second",
        "- first\n  continuation",
        "| a | b |\n| --- | --- |\n| c | d |",
        "```rust\nlet value = 1;\n```",
    ];
    for source in corpus {
        let lazy = layout_with_cursor_and_width(source, 0, 160.0);
        lazy.validate_editable_projections()
            .unwrap_or_else(|error| panic!("missing projection for {source:?}: {error:?}"));
    }
}
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-markdown --lib layout::types::tests::every_editable_flat_line_has_projection_after_full_layout -- --exact --nocapture`

Expected: FAIL，`LazyLayout::validate_editable_projections` 尚不存在。

- [ ] **Step 3: 删除 legacy 字段和算法**

先实现：

```rust
pub(crate) fn validate_editable_projections(&self) -> Result<(), ProjectionError> {
    for line in &self.flat_lines {
        if line.source_projection.is_none() {
            return Err(ProjectionError::MissingEditableProjection {
                flat_line_idx: line.flat_idx,
            });
        }
    }
    Ok(())
}
```

随后删除：

- `LazyLayout::line_byte_offsets`
- `LazyLayout::block_line_map`
- `LazyLayout::flat_line_source_maps`
- `LazyLayout::source_byte_visual_positions`
- `fallback_line_byte_offsets`
- `collect_line_byte_offsets()`、`line_source_len()`、`marker_overhead_before()` 的映射用途
- `prepend_marker_to_line()` 中 relative/absolute map 启发式
- `find_flat_and_grapheme_for_byte()` 的重复 byte `grapheme_pos == 0` 规则和 nearest-neighbor 扫描

`FlatLine::source_bytes_by_visual_grapheme` 也删除，所有调用改读 `source_projection.boundaries`。对 preview/Novel 不可编辑行允许 `source_projection: None`，但 WYSIWYG query 遇到它必须返回 `MissingEditableProjection`，不得猜测 byte。

- [ ] **Step 4: 静态确认和全 crate 测试**

Run: `rg -n "fallback_line_byte_offsets|line_byte_offsets|flat_line_source_maps|source_byte_visual_positions|source_bytes_by_visual_grapheme" crates/markdown/src`

Expected: 无生产代码命中；若测试 fixture 仍使用旧字段，同步改成 `source_projection.boundaries`。

Run: `cargo test -p textora-markdown --lib && cargo fmt --all -- --check && cargo check -p textora-markdown`

Expected: PASS。

- [ ] **Step 5: 提交 legacy 清理**

```bash
git add crates/markdown/src/layout/block.rs crates/markdown/src/layout/types.rs crates/markdown/src/view.rs
git commit -m "refactor(markdown): remove legacy cursor mapping fallback"
```

---

### Task 12: 添加 App 层真实坐标回归

**Files:**

- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/app/src/app_tests.rs`
- Test: `crates/app/src/app_tests.rs`

**Interfaces:**

- Consumes: 现有 `PluginQuery::FlatLines`、`HitTestByte` 和 `VisualMove`。
- Produces: `ui::plugin::FlatLine::{rect,grapheme_x}` 纯数据字段；App 不依赖 markdown 内部投影类型。

- [ ] **Step 1: 写 App 层真实视觉行失败测试**

先修改现有 `wysiwyg_promotion_blockquote_line_three_click_and_up_navigation_reach_its_source_range` 的坐标准备部分，使它直接读取 visual line 几何；字段尚不存在，因此应编译失败：

```rust
let visual_lines = {
    let tab = app.workspace.active_entry_mut().expect("active entry");
    match tab.plugin.query(ui::plugin::PluginQuery::FlatLines, &tab.doc) {
        ui::plugin::PluginResponse::FlatLines(lines) => lines,
        response => panic!("expected FlatLines, got {response:?}"),
    }
};
let line_three = visual_lines.iter()
    .find(|line| line.text.contains("Applicable scenarios"))
    .expect("line three text must be visible");
let visible_offset = line_three.text.find("Applicable scenarios")
    .expect("needle must be in line three");
let bounds = app.plugin_render_bounds();
let click_x = bounds.x + line_three.rect.x + line_three.grapheme_x[visible_offset];
let click_y = bounds.y + line_three.rect.y + line_three.rect.h * 0.5;
```

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-app --lib wysiwyg_promotion_blockquote_line_three_click_and_up_navigation_reach_its_source_range -- --exact`

Expected: FAIL，`ui::plugin::FlatLine` 没有 `rect` 或 `grapheme_x` 字段。

- [ ] **Step 3: 扩展纯数据 FlatLine 并补水平遍历断言**

在 `ui::plugin::FlatLine` 增加：

```rust
pub rect: crate::core::geom::Rect,
/// X advance at every visual grapheme boundary, including the sentinel.
pub grapheme_x: Vec<f32>,
```

`MarkdownEditorView` 响应 `PluginQuery::FlatLines` 时从内部 flat line 复制 rect，并用 `(0..=grapheme_count).map(|g| crate::layout::grapheme_x(line, g))` 填充 advance。其他 plugin 的 `FlatLine` 构造处显式提供零 rect 和 `[0.0]`，不得新增 markdown 依赖。

在 App 测试增加以下 helper，验证 Left 能进入前一源码行且不循环：

```rust
fn move_left_reaches_range_without_cycle(
    app: &mut App,
    start_byte: usize,
    target: std::ops::Range<usize>,
) -> bool {
    app.workspace.active_entry_mut().expect("active entry")
        .doc.cursor_move_to_offset(start_byte);
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..=start_byte + 1 {
        let current = app.workspace.active_entry().expect("active entry")
            .doc.cursor_offset().to_usize();
        if target.contains(&current) {
            return true;
        }
        if !visited.insert(current) {
            return false;
        }
        let effect = app.dispatch_wysiwyg_navigation(&crate::input::EditCommand::MoveLeft);
        if !effect.redraw {
            return false;
        }
    }
    false
}
```

最终测试同时断言真实 line3 点击、从 line4 Up、从 line4 Left 都进入 `line_three_start..line_four_start`。

- [ ] **Step 4: 运行 UI/Markdown/App 定向测试**

Run: `cargo test -p textora-ui --lib plugin -- --nocapture && cargo test -p textora-markdown --lib view::wysiwyg_tests -- --nocapture && cargo test -p textora-app --lib wysiwyg_promotion_blockquote_line_three_click_and_up_navigation_reach_its_source_range -- --exact --nocapture && cargo fmt --all -- --check && cargo check --workspace`

Expected: PASS。

- [ ] **Step 5: 提交 App 真实交互回归**

```bash
git add crates/ui/src/plugin.rs crates/markdown/src/view.rs crates/app/src/app_tests.rs
git commit -m "test(app): verify WYSIWYG projection with real line geometry"
```

---

### Task 13: 投影错误时保持 App 光标和选区不变

**Files:**

- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `crates/app/src/app_tests.rs`
- Test: `crates/app/src/app_tests.rs`

**Interfaces:**

- Consumes: Markdown plugin 在缺失/非法投影时返回 `PluginResponse::BytePosition(None)`。
- Produces: `set_plugin_cursor_from_point() -> Option<usize>`；失败命中不再伪造 byte 0 或 buffer end。

- [ ] **Step 1: 写缺投影命中失败测试**

在 `app_tests.rs` 使用现有 `RecordingWysiwygPlugin` 增加：

```rust
#[test]
fn wysiwyg_missing_projection_keeps_cursor_and_selection_unchanged() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState {
        hit_test_byte: None,
        content_height: 500.0,
        ..RecordingWysiwygState::default()
    }));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["abcdef".to_string()], 80, 10.0);
    doc.cursor_move_to_offset(3);
    doc.cursor_mut().selection_anchor = Some(1);
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(RecordingWysiwygPlugin::new(state)),
    ));
    let _ = app.workspace.switch_to(0);
    let bounds = app.plugin_render_bounds();
    app.dispatch_editor_mouse_input(
        winit::event::ElementState::Pressed,
        bounds.x + 10.0,
        bounds.y + 10.0,
        None,
    );
    let entry = app.workspace.active_entry().expect("active entry");
    assert_eq!(entry.doc.cursor_offset().to_usize(), 3);
    assert_eq!(entry.doc.cursor().selection_anchor, Some(1));
}
```

在 `RecordingWysiwygState` 新增 `content_height: f32`，默认值为 `0.0`；让 `PluginQuery::ContentHeight` 返回 `PluginResponse::Float(state.content_height)`。

- [ ] **Step 2: 运行 RED**

Run: `cargo test -p textora-app --lib wysiwyg_missing_projection_keeps_cursor_and_selection_unchanged -- --exact --nocapture`

Expected: FAIL，当前点击路径把 cursor 移到 byte 0 或清空 selection。

- [ ] **Step 3: 让点击失败显式返回 None**

把签名改为：

```rust
fn set_plugin_cursor_from_point(&mut self, px: f32, py: f32) -> Option<usize>;
```

规则固定为：

- `HitTestByte(Some(byte))` 返回 `Some(byte)` 并继续两阶段命中。
- 点击确实低于 `ContentHeight` 时仍返回 `Some(buffer_len)`，保留既有“点击文档末尾”行为。
- 内容区域内的 `HitTestByte(None)` 返回 `None`，不得返回 0。
- 调用方用 `let Some(byte) = ... else { return AppEffect::NONE; };`，因此不修改 cursor/selection，也不发送 plugin selection 消息。

- [ ] **Step 4: 运行鼠标和 App 全测**

Run: `cargo test -p textora-app --lib wysiwyg_missing_projection_keeps_cursor_and_selection_unchanged -- --exact --nocapture && cargo test -p textora-app --lib dispatch::mouse::tests -- --nocapture && cargo test -p textora-app --lib`

Expected: PASS。

- [ ] **Step 5: 提交安全失败行为**

```bash
git add crates/app/src/dispatch/mouse.rs crates/app/src/app_tests.rs
git commit -m "fix(app): preserve cursor when WYSIWYG projection is unavailable"
```

---

### Task 14: 确定性 corpus、完整验证和设计归档

**Files:**

- Modify: `crates/markdown/src/view.rs`
- Modify: `docs/superpowers/specs/2026-07-11-wysiwyg-bidirectional-source-projection-design.md`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**

- Consumes: 完成迁移后的全部投影消费端。
- Produces: heading、nested blockquote、list、table、Unicode、空行和全量/增量一致性 corpus；最终验证记录。

- [ ] **Step 1: 增加确定性 corpus 遍历**

在 `view.rs` 测试模块加入以下确定性遍历；collapsed source range 内部位置以索引返回的 canonical downstream anchor 为 oracle，不能直接要求原 byte 不变：

```rust
fn source_grapheme_boundaries(source: &str) -> Vec<usize> {
    let count = crate::grapheme_map::grapheme_count(source);
    (0..=count)
        .map(|index| crate::grapheme_map::byte_at_grapheme_index(source, index))
        .collect()
}

#[test]
fn projection_corpus_roundtrips_every_source_grapheme_boundary() {
    use ui::plugin::{PluginMessage, ViewPlugin};

    let corpus = [
        "plain paragraph",
        "# heading heading heading heading heading heading",
        "> outer\n> > **inner** — continuation",
        "- outer\n  - inner wrapped content wrapped content",
        "| left | middle | right |\n| --- | --- | --- |\n| 左侧内容 | middle content | 👨\u{200d}👩 |",
        "paragraph\n\n\nnext",
        "e\u{301} and 👨\u{200d}👩",
    ];

    for source in corpus {
        for width in [140.0, 320.0, 800.0] {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            for source_byte in source_grapheme_boundaries(source) {
                view.handle_message(PluginMessage::SetCursorByte(source_byte), &mut document);
                render_editor_narrow(&mut view, &document, width);
                let position = view
                    .engine()
                    .projection_index()
                    .visual_position_for_source(source_byte, CursorAffinity::Downstream)
                    .expect("every source grapheme boundary must have a canonical position");
                let expected = view
                    .engine()
                    .projection_index()
                    .source_anchor_at(1, position)
                    .expect("canonical position must map back to source")
                    .byte;
                let (x, y, _cursor_width, cursor_height) = view
                    .engine()
                    .cursor_screen_pos()
                    .expect("every canonical source position must have a cursor rect");
                let hit = view
                    .engine()
                    .hit_test_byte(x, y + cursor_height * 0.5, 0.0, 0.0)
                    .expect("cursor rect center must be hittable");
                assert_eq!(hit, expected, "source={source:?}, width={width}, byte={source_byte}");
            }
        }
    }
}
```

同一模块增加未受影响 block 的全量/增量一致性测试：

```rust
#[test]
fn cursor_only_refresh_keeps_unaffected_block_projection_identical() {
    use ui::plugin::{PluginMessage, ViewPlugin};

    let source = "# first heading\n\nmiddle paragraph stays stable\n\n> final quote";
    let first = source.find("first").expect("fixture contains first");
    let final_quote = source.find("final").expect("fixture contains final");
    let mut document = StubDoc::new(source);
    let mut view = MarkdownEditorView::new();
    view.set_source(document.text.clone(), 1);
    view.handle_message(PluginMessage::SetCursorByte(first), &mut document);
    render_editor_narrow(&mut view, &document, 320.0);
    let before = view.engine().flat_lines().iter()
        .find(|line| line.text.contains("middle paragraph"))
        .expect("middle paragraph must be visible")
        .source_projection
        .clone()
        .expect("middle paragraph needs projection");

    view.handle_message(PluginMessage::SetCursorByte(final_quote), &mut document);
    render_editor_narrow(&mut view, &document, 320.0);
    let after = view.engine().flat_lines().iter()
        .find(|line| line.text.contains("middle paragraph"))
        .expect("middle paragraph must stay visible")
        .source_projection
        .clone()
        .expect("middle paragraph needs projection");

    assert_eq!(after, before);
}
```

- [ ] **Step 2: 运行 corpus、crate 和工作区测试**

Run: `cargo test -p textora-markdown --lib view::wysiwyg_tests::projection_corpus_roundtrips_every_source_grapheme_boundary -- --exact --nocapture && cargo test -p textora-markdown --lib && cargo test -p textora-app --lib`

Expected: PASS。

- [ ] **Step 3: 执行项目级验证**

Run: `cargo fmt --all -- --check && cargo check --workspace`

Expected: PASS。

Run: `./scripts/verify.sh`

Expected: 所有格式、编译、Clippy 和测试阶段成功退出；记录首次布局与 cursor-only 更新未出现数量级退化。

- [ ] **Step 4: 更新设计状态**

把设计文档状态改为“Implemented”，追加实际验证命令和结果摘要，不增加未完成项。

- [ ] **Step 5: 提交最终验证**

```bash
git add crates/markdown/src/view.rs docs/superpowers/specs/2026-07-11-wysiwyg-bidirectional-source-projection-design.md
git commit -m "test(markdown): verify bidirectional source projection"
```

---

## Execution Order and Review Gates

严格按 Task 1 → 14 执行。Task 4、6、7、9、11、13 属于高风险切换点，每个任务完成后必须单独进行：

1. 需求符合性审查：接口与本计划一致，没有扩大结构编辑范围。
2. 代码质量审查：无 fallback、无宽泛命名、无未说明 magic value、无跨层依赖。
3. 定向手测：使用 `promotion.md` 当前窗口宽度验证鼠标和方向键；手测不得替代自动化测试。

任何任务连续两次修复仍无法通过其 RED/GREEN 测试，停止叠加补丁，回到投影不变量和 owner/affinity 设计重新审查。
