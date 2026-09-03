# Markdown Structural Gap Hit Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让水平分割线、其上下内间距和相邻结构间距始终映射到合法源码边界，同时保持现有多空行视觉高度。

**Architecture:** 布局层从 `BlockNode::block_range` 提取不含行尾 CR/LF 的原子块源码范围并保存在 `FlatLine` 上，不伪造文字 grapheme 投影。视图层通过三个单一职责查询统一解析“行内点击、行首边界、行尾边界”，文本行继续走 `SourceProjectionIndex`，原子块直接使用显式源码范围。

**Tech Stack:** Rust、textora-markdown、现有 `LazyLayout` / `SourceProjectionIndex` / `ViewPlugin` 测试工具。

## Global Constraints

- 不改变 `HiddenBlockSeparator`、`EditableEmpty`、`line_height + paragraph_spacing` 的现有语义。
- 不让 UI 层依赖 app 层状态；修改仅限 `textora-markdown`。
- 生产代码与测试合计只修改 `layout/types.rs`、`selection.rs`、`view.rs` 三个文件。
- 禁止用无语义布尔组合表达新的互斥状态；原子块使用明确的 `atomic_source_range`。
- 禁止 `.unwrap()`；测试夹具使用带原因的 `.expect(...)`。
- 每次代码提交前运行 `cargo fmt --check` 和对应定向测试。

---

### Task 1: 保留原子块源码范围

**Files:**
- Modify: `crates/markdown/src/layout/types.rs:250-320, 680-825, 965-985`
- Modify: `crates/markdown/src/selection.rs:320-405`
- Test: `crates/markdown/src/layout/types.rs`

**Interfaces:**
- Consumes: `BlockNode::block_range: Range<usize>`、当前源码与现有 `FlatLine` 扁平化流程。
- Produces: `FlatLine::atomic_source_range: Option<Range<usize>>`，文本行恒为 `None`，水平分割线为其不含行尾换行的真实标记范围。

- [ ] **Step 1: 写失败测试，证明水平分割线丢失源码范围**

在 `layout/types.rs` 测试模块增加：

```rust
#[test]
fn horizontal_rule_flat_line_retains_atomic_source_range() {
    let source = "before\n\n---\n\nafter";
    let horizontal_rule_start = source.find("---").expect("fixture contains a rule");
    let layout = build_editing_layout(source, 0, 1);
    let horizontal_rule = layout
        .flat_lines
        .iter()
        .find(|line| line.text.is_empty() && line.source_projection.is_none())
        .expect("fixture lays out a horizontal rule");

    assert_eq!(
        horizontal_rule.atomic_source_range,
        Some(horizontal_rule_start..horizontal_rule_start + 3)
    );
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown --lib horizontal_rule_flat_line_retains_atomic_source_range -- --exact
```

Expected: 编译失败，提示 `FlatLine` 尚无 `atomic_source_range` 字段。

- [ ] **Step 3: 给 `FlatLine` 增加原子源码范围并贯穿递归扁平化**

在 `FlatLine` 增加：

```rust
/// Source range owned by a non-text visual block such as a horizontal rule.
pub(crate) atomic_source_range: Option<Range<usize>>,
```

文本行构造器设置 `None`。将 `flatten_block_into` 的最后一个参数改为：

```rust
doc_block: Option<&crate::builder::BlockNode>,
```

函数内部通过 `doc_block.map(|block| block.children.as_slice())` 读取子块；递归调用传入对应 `child`。水平分割线构造器从 `block_range` 裁掉末尾 `CR/LF` 后设置：

```rust
atomic_source_range: doc_block.and_then(|block| {
    Self::source_range_without_line_ending(&block.block_range, source_text)
}),
```

顶层调用直接传 `doc_block`。更新 `selection.rs` 中测试专用 `FlatLine` 字面量，全部设置 `atomic_source_range: None`。

- [ ] **Step 4: 运行格式化与定向测试并确认 GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown --lib horizontal_rule_flat_line_retains_atomic_source_range -- --exact
cargo test -p textora-markdown --lib selection::tests
```

Expected: 全部通过，且无警告。

- [ ] **Step 5: 提交布局元数据阶段**

```bash
git add crates/markdown/src/layout/types.rs crates/markdown/src/selection.rs
git commit -m "fix(markdown): retain atomic block source ranges"
```

---

### Task 2: 统一水平分割线与结构间距命中

**Files:**
- Modify: `crates/markdown/src/view.rs:1648-1765, 5768-5875`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `FlatLine::atomic_source_range`、`byte_from_flat_line_and_visual_grapheme`、现有空源码行命中查询。
- Produces: `source_byte_at_flat_line_hit`、`flat_line_start_source_byte`、`flat_line_end_source_byte` 三个私有查询；`hit_test_byte` 不再对原子块返回 `None`。

- [ ] **Step 1: 写失败测试覆盖分割线块及上下间距**

在 `wysiwyg_tests` 中增加测试。夹具为：

```rust
let source = "国内用增,\n\n---\n\n后续";
```

测试找到正文、水平分割线和后续段落三个 `FlatLine`，并断言：

```rust
assert_eq!(
    view.engine().hit_test_byte(x, rule.rect.y + rule.rect.h * 0.25, 0.0, 0.0),
    Some(rule_start)
);
assert_eq!(
    view.engine().hit_test_byte(x, rule.rect.y + rule.rect.h * 0.75, 0.0, 0.0),
    Some(rule_end)
);
assert_eq!(
    view.engine().hit_test_byte(x, rule.rect.y - 1.0, 0.0, 0.0),
    Some(rule_start)
);
assert_eq!(
    view.engine().hit_test_byte(x, rule.rect.y + rule.rect.h + 1.0, 0.0, 0.0),
    Some(rule_end)
);
```

再把返回的 `rule_start` 通过 `handle_set_cursor_byte` 设置并重渲染，断言存在文本为 `"---"` 的活动行。

- [ ] **Step 2: 写失败测试覆盖额外空行与 DPI 2×**

增加两个行为测试：

```rust
#[test]
fn horizontal_rule_hit_test_preserves_editable_blank_line() {
    let source = "before\n\n\n---\n\nafter";
    let editable_blank_byte = source.find("\n\n\n").expect("fixture contains blank run") + 2;
    let doc = StubDoc::new(source);
    let mut view = MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    render_editor_once(&mut view, &doc);
    view.engine.handle_set_cursor_byte(editable_blank_byte);

    let (blank_x, blank_y, _width, blank_height) = view
        .engine()
        .cursor_screen_pos()
        .expect("editable blank line has cursor geometry");
    assert_eq!(
        view.engine().hit_test_byte(
            blank_x,
            blank_y + blank_height * 0.5,
            0.0,
            0.0,
        ),
        Some(editable_blank_byte)
    );
}

#[test]
fn horizontal_rule_hit_test_uses_physical_coordinates_at_two_x_dpi() {
    let source = "before\n\n---\n\nafter";
    let doc = StubDoc::new(source);
    let mut view = MarkdownEditorView::new();
    view.set_source(source.to_owned(), 1);
    render_editor_viewport_with_dpi(&mut view, &doc, 800.0, 600.0, 2.0);

    let rule = view
        .engine()
        .flat_lines()
        .iter()
        .find(|line| line.atomic_source_range.is_some())
        .expect("fixture lays out a horizontal rule");
    let range = rule.atomic_source_range.clone().expect("rule owns source range");
    let x = rule.rect.x + 1.0;

    assert_eq!(
        view.engine().hit_test_byte(x, rule.rect.y + rule.rect.h * 0.25, 0.0, 0.0),
        Some(range.start)
    );
    assert_eq!(
        view.engine().hit_test_byte(x, rule.rect.y + rule.rect.h * 0.75, 0.0, 0.0),
        Some(range.end)
    );
}
```

第一项断言额外空行中心仍返回第二个空行的源码字节，靠近规则处返回 `rule_start`；第二项在物理像素下断言规则上下半部返回 `rule_start/rule_end`。

- [ ] **Step 3: 运行测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown --lib horizontal_rule_hit_test -- --nocapture
```

Expected: 规则块与靠近规则的间距断言得到 `None`，测试失败原因与根因一致。

- [ ] **Step 4: 实现三个边界查询并替换直接反查**

在 `PreviewEngine` 中增加：

```rust
fn source_byte_at_flat_line_hit(&self, line: &FlatLine, doc_y: f32, line_x: f32) -> Option<usize>;
fn flat_line_start_source_byte(&self, line: &FlatLine) -> Option<usize>;
fn flat_line_end_source_byte(&self, line: &FlatLine) -> Option<usize>;
```

规则如下：

- `atomic_source_range` 存在时，行内点击按 `rect` 垂直中点返回 `start/end`。
- 文本行行内点击继续使用 `grapheme_at_x` 与现有投影索引。
- 行首查询：原子块返回 `range.start`，文本行查询 grapheme `0`。
- 行尾查询：原子块返回 `range.end`，文本行查询最后一个 grapheme。

将 `hit_test_byte` 的同一行分支、上下间距分支、文档首尾吸附分支全部改为调用这些查询。保留 `visible_empty_source_line_byte_at_doc_y` 在块外间距决策之前，确保额外可编辑空行优先于邻块吸附。

- [ ] **Step 5: 增加间距不变量与增量更新测试**

增加：

```rust
#[test]
fn horizontal_rule_spacing_counts_only_additional_blank_lines() {
    fn rule_y(source: &str) -> (f32, f32, f32) {
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_once(&mut view, &doc);
        let y = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("fixture lays out a horizontal rule")
            .rect
            .y;
        (y, view.engine().base_line_height, view.engine().paragraph_spacing)
    }

    let (single_blank_y, line_height, paragraph_spacing) = rule_y("before\n\n---");
    let (double_blank_y, _, _) = rule_y("before\n\n\n---");
    assert_eq!(double_blank_y - single_blank_y, line_height + paragraph_spacing);
}

#[test]
fn removing_extra_blank_line_clears_horizontal_rule_y_delta() {
    fn current_rule_y(view: &MarkdownEditorView) -> f32 {
        view.engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("fixture lays out a horizontal rule")
            .rect
            .y
    }

    let mut reused = MarkdownEditorView::new();
    let double_blank = "before\n\n\n---";
    reused.set_source(double_blank.to_owned(), 1);
    render_editor_once(&mut reused, &StubDoc::new(double_blank));

    let single_blank = "before\n\n---";
    reused.set_source(single_blank.to_owned(), 2);
    render_editor_once(&mut reused, &StubDoc::new(single_blank));

    let mut fresh = MarkdownEditorView::new();
    fresh.set_source(single_blank.to_owned(), 2);
    render_editor_once(&mut fresh, &StubDoc::new(single_blank));

    assert_eq!(current_rule_y(&reused), current_rule_y(&fresh));
}
```

后一测试必须使用同一个 `MarkdownEditorView` 连续 `set_source`，比较更新后的规则坐标与全新单空行视图坐标相等。

- [ ] **Step 6: 运行定向测试、Markdown 全库测试和格式检查**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown --lib horizontal_rule_hit_test -- --nocapture
cargo test -p textora-markdown --lib horizontal_rule_spacing -- --nocapture
cargo test -p textora-markdown --lib removing_extra_blank_line_clears_horizontal_rule_y_delta -- --exact
cargo test -p textora-markdown --lib
```

Expected: 全部通过，无失败和编译警告。

- [ ] **Step 7: 提交命中修复**

```bash
git add crates/markdown/src/view.rs
git commit -m "fix(markdown): hit test horizontal rule boundaries"
```

---

### Task 3: 项目级验证

**Files:**
- No source changes expected.

**Interfaces:**
- Consumes: Task 1 与 Task 2 的两个提交。
- Produces: 项目级编译与验证证据。

- [ ] **Step 1: 运行 Markdown 与应用层编译**

```bash
cargo check -p textora-markdown
cargo check -p textora-app
```

Expected: 两个包均编译通过。

- [ ] **Step 2: 运行项目全面验证**

```bash
./scripts/verify.sh
```

Expected: 脚本退出码为 `0`；若存在与本次改动无关的既有失败，记录精确命令和错误，不改动无关代码。

- [ ] **Step 3: 检查工作树与提交边界**

```bash
git status --short
git log -4 --oneline
```

Expected: 工作树干净；文档、布局元数据、命中逻辑各自形成清晰提交。
