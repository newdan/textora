# Markdown Empty Line Before Atomic Block Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让段落末尾 Enter 新增的空行光标稳定显示在水平分割线上方，并在输入文字前后保持同一垂直槽位。

**Architecture:** `LazyLayout` 构建源码行布局输入时，把文字行的 `source_projection.source_extent` 与原子块的 `atomic_source_range` 统一转换为 `RenderedLineLayout`。`SourceLineMap` 因而能把水平分割线作为真实邻接块计算空行几何，无需在光标绘制层增加特判。

**Tech Stack:** Rust、textora-markdown、`LazyLayout`、`SourceLineMap`、现有 `MarkdownEditorView` 测试工具。

## Global Constraints

- 不改变 Enter 的源码编辑策略。
- 不改变水平分割线的视觉高度、间距和点击边界。
- 不为原子块伪造文字或 grapheme 投影。
- 单个块间空行仍为 `HiddenBlockSeparator`，额外空行仍占 `line_height + paragraph_spacing`。
- 修改限于 `crates/markdown/src/layout/types.rs` 与 `crates/markdown/src/view.rs`。
- 先确认回归测试 RED，再修改生产代码。
- 禁止 `.unwrap()`；测试夹具使用包含原因的 `.expect(...)`。

---

### Task 1: 将原子块纳入源码行垂直布局

**Files:**
- Modify: `crates/markdown/src/layout/types.rs:315-345, 2160-2210`
- Modify: `crates/markdown/src/view.rs:4070-4145`
- Test: `crates/markdown/src/layout/types.rs`
- Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `FlatLine::source_projection: Option<VisualLineProjection>`、`FlatLine::atomic_source_range: Option<Range<usize>>`。
- Produces: 交给 `SourceLineMap::attach_layout` 的 `RenderedLineLayout` 同时覆盖文字行和原子块；不新增公共 API。

- [ ] **Step 1: 写布局层失败测试**

在 `layout/types.rs` 的测试模块增加：

```rust
#[test]
fn atomic_block_bounds_the_projected_empty_line_before_it() {
    let source = "图片需讨论\n\n\n---\n\n移动";
    let editable_empty_line_byte = "图片需讨论\n\n".len();
    let layout = build_editing_layout(source, editable_empty_line_byte, 1);
    let horizontal_rule = layout
        .flat_lines
        .iter()
        .find(|line| line.atomic_source_range.is_some())
        .expect("fixture lays out a horizontal rule");
    let editable_empty_line = layout
        .projected_empty_lines
        .iter()
        .find(|line| line.source_byte == editable_empty_line_byte)
        .expect("Enter-created empty line must be projected");

    assert!(
        editable_empty_line.y_top + editable_empty_line.height <= horizontal_rule.rect.y + 0.01,
        "empty line must end before the horizontal rule: empty={editable_empty_line:?}, rule={:?}",
        horizontal_rule.rect
    );
}
```

- [ ] **Step 2: 写编辑器级失败测试**

在 `view.rs` 的 `wysiwyg_tests` 中增加测试。测试必须调用真实 `augment_edit`、应用返回的源码替换并重新渲染：

```rust
#[test]
fn paragraph_enter_before_horizontal_rule_keeps_empty_and_typed_line_in_place_at_each_dpi() {
    let source = "图片需讨论\n\n---\n\n移动";
    let paragraph_end = "图片需讨论".len();

    for dpi_scale in [1.0, 2.0] {
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_viewport_with_dpi(&mut view, &document, 800.0, 600.0, dpi_scale);
        view.engine.handle_set_cursor_byte(paragraph_end);

        let augmentation = view
            .engine
            .augment_edit(paragraph_end, AugmentKind::Enter)
            .expect("paragraph-end Enter creates an editable empty line");
        let replace_range = augmentation
            .replace_range
            .clone()
            .unwrap_or(paragraph_end..paragraph_end);
        document.text.replace_range(
            replace_range,
            augmentation.insert_text.as_deref().expect("Enter inserts a source newline"),
        );
        view.set_source(document.text.clone(), 2);
        view.engine.handle_set_cursor_byte(augmentation.cursor_byte_after);
        render_editor_viewport_with_dpi(&mut view, &document, 800.0, 600.0, dpi_scale);

        let horizontal_rule = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("fixture lays out an inactive horizontal rule");
        let (_, empty_cursor_y, _, empty_cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("Enter-created empty line has a cursor rect");
        let empty_line_top = empty_cursor_y
            - empty_cursor_height * (1.0 - WYSIWYG_CURSOR_ASCENT_RATIO);

        assert!(
            empty_cursor_y + empty_cursor_height <= horizontal_rule.rect.y + 0.01,
            "empty cursor must stay above the rule at {dpi_scale}x DPI"
        );

        let insertion = view
            .engine
            .augment_edit(
                augmentation.cursor_byte_after,
                AugmentKind::InsertText(String::from("新")),
            )
            .expect("typing materializes the editable empty line");
        let insertion_range = insertion
            .replace_range
            .clone()
            .unwrap_or(augmentation.cursor_byte_after..augmentation.cursor_byte_after);
        document.text.replace_range(
            insertion_range,
            insertion.insert_text.as_deref().expect("typing inserts source text"),
        );
        view.set_source(document.text.clone(), 3);
        view.engine.handle_set_cursor_byte(insertion.cursor_byte_after);
        render_editor_viewport_with_dpi(&mut view, &document, 800.0, 600.0, dpi_scale);

        let typed_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains('新'))
            .expect("typed paragraph is laid out");
        let updated_rule = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.atomic_source_range.is_some())
            .expect("horizontal rule remains inactive after typing");

        assert!((typed_line.rect.y - empty_line_top).abs() < 1.0);
        assert!(typed_line.rect.y + typed_line.rect.h <= updated_rule.rect.y + 0.01);
    }
}
```

- [ ] **Step 3: 运行定向测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown --lib atomic_block_bounds_the_projected_empty_line_before_it
cargo test -p textora-markdown --lib paragraph_enter_before_horizontal_rule_keeps_empty_and_typed_line_in_place_at_each_dpi
```

Expected: 两个测试至少一个因空行或光标位于水平分割线下方而失败；不得因编译错误或夹具错误失败。

- [ ] **Step 4: 让原子块参与 `RenderedLineLayout` 构建**

将 `collect_source_only_empty_line_projections` 中只接受文字投影的 `filter_map` 改为统一读取源码范围：

```rust
let rendered_lines = self
    .flat_lines
    .iter()
    .filter_map(|line| {
        let source_range = line
            .source_projection
            .as_ref()
            .map(|projection| projection.source_extent.clone())
            .or_else(|| line.atomic_source_range.clone())?;
        Some(RenderedLineLayout {
            source_range,
            y_top: line.rect.y,
            height: line.rect.h,
        })
    })
    .collect::<Vec<_>>();
```

不得给原子块增加 `VisualLineProjection`，也不得在 `cursor_screen_pos_for_byte` 中增加水平分割线分支。

- [ ] **Step 5: 运行定向测试并确认 GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown --lib atomic_block_bounds_the_projected_empty_line_before_it
cargo test -p textora-markdown --lib paragraph_enter_before_horizontal_rule_keeps_empty_and_typed_line_in_place_at_each_dpi
cargo test -p textora-markdown --lib horizontal_rule
```

Expected: 全部通过，且无警告。

- [ ] **Step 6: 运行 Markdown 整包回归并提交**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-markdown --lib
```

Expected: 全部通过，0 failed。

Commit:

```bash
git add crates/markdown/src/layout/types.rs crates/markdown/src/view.rs
git commit -m "fix(markdown): keep empty cursor above atomic blocks"
```

- [ ] **Step 7: 运行项目完整验证**

Run:

```bash
./scripts/verify.sh
```

Expected: 架构检查、格式、Clippy、全部 workspace 测试与 doctest 通过。
