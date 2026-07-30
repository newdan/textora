# Markdown WYSIWYG Cursor Span Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 `MarkdownEditorView` 点击光标卡顿、光标位置错误、进入 inline span 后不切换到源码展开态的问题。

**Architecture:** 保持现有分层：app 层继续拥有输入、编辑命令、undo/redo 和 `DocumentView`，markdown 插件只负责 Markdown 布局、像素命中、源码 byte 映射和 WYSIWYG 光标绘制。实现上先补可复现测试，再引入行级 materialized 映射，最后把点击、输入、导航和 dirty 粒度收敛到同一条同步链路。

**Tech Stack:** Rust workspace；`edit-plus-markdown` 负责 Markdown parser/builder/layout/render/view；`edit-plus-ui` 提供 `ViewPlugin`、`PluginMessage`、`PluginQuery`；`edit-plus-app` 负责鼠标、键盘、渲染调度；验证使用 `cargo test`、`cargo check`、`cargo fmt`、`./scripts/verify.sh`。

---

## Scope And Constraints

- 全程遵守 `AGENTS.md`：中文沟通、先复现测试再修、超过 3 个文件的修改必须拆分阶段、每次提交前确保编译通过。
- 绝对不能让 `crates/ui` 依赖 `crates/app`；新增 WYSIWYG 数据结构必须在 `crates/markdown` 或 `crates/ui` 的纯数据边界内。
- app 层不能直接访问 `MarkdownDoc`、`LazyLayout`、`StyleSpan` 等 markdown 内部结构；只能通过 `PluginQuery` / `PluginMessage` 交互。
- 优先修正文档、span、光标、点击的主链路；表格、图片、复杂嵌套 span 不纳入本计划。
- 光标移动不得触发整篇 Markdown 全量 parse/build/shape；源码变化可以先保留全量 parse/build，块级 diff 不纳入本计划。

## Current Evidence From Code

- `crates/markdown/src/edit.rs:38` 的 `materialize_text()` 只在单元测试中被调用，当前布局路径没有消费它。
- `crates/markdown/src/view.rs:573` 的 `handle_set_cursor_byte()` 每次光标变更都会 `mark_dirty()`。
- `crates/markdown/src/view.rs:1517` 的 `MarkdownEditorView::render()` 以 `full_layout=true` 调用引擎，触发 `crates/markdown/src/view.rs:257` 的 `ensure_all_blocks()`。
- `crates/app/src/dispatch/mouse.rs:115` 的点击流程先用旧布局 `HitTestByte`，再发送 `SetCursorByte`。
- `crates/markdown/src/builder.rs:598` 中 `StyleSpan.start/len` 以 byte 计数；`crates/markdown/src/layout/context.rs:37` 的 `char_at_x()` 和 `char_x()` 以 char index 计数。
- `crates/app/src/dispatch/wysiwyg.rs` 已有 WYSIWYG 导航和 augment 方法，但 `crates/app/src/dispatch/editor.rs` 的命令入口没有接入这些方法。

## File Structure

- Modify: `crates/markdown/src/edit.rs`
  - 扩展 `EditContext` 和 materialized 行映射纯函数。
  - 保持该文件不依赖 app，不做布局或渲染。
- Modify: `crates/markdown/src/layout/context.rs`
  - 给 `LayoutCtx` 增加可选编辑上下文和 source 文本引用。
  - 将 char/byte 命名拆清楚，避免坐标混用。
- Modify: `crates/markdown/src/layout/block.rs`
  - 文本块和列表项布局时调用 materialized 行映射。
  - 输出调整后的 `LaidOutLine.text` 和 `StyleSpan`。
- Modify: `crates/markdown/src/layout/types.rs`
  - 让 `LazyLayout` 保存 materialized 行到 source byte 的映射。
  - 提供 O(1) 或局部 O(line) 的 byte <-> visual char 查询方法。
- Modify: `crates/markdown/src/view.rs`
  - `PreviewEngine` 使用 materialized 映射实现 `HitTestByte`、`CursorScreenPos`、`VisualMoveWysiwyg`。
  - 引入 cursor dirty 粒度，避免光标移动全量重排。
  - 增加 WYSIWYG 回归测试。
- Modify: `crates/app/src/dispatch/mouse.rs`
  - 点击 WYSIWYG 时统一同步 source 和 cursor，并处理首次进入 span 的二阶段命中。
- Modify: `crates/app/src/dispatch/editor.rs`
  - 在命令入口接入 `dispatch_wysiwyg_navigation()`、`dispatch_wysiwyg_augmented_enter()`、`dispatch_wysiwyg_augmented_backspace()`。
  - 标准编辑执行后同步 `SetCursorByte`。
- Modify: `crates/app/src/dispatch/wysiwyg.rs`
  - 补齐当前已有方法的测试入口和必要辅助函数。
- Modify: `crates/app/src/app_renderer.rs`
  - 渲染前统一把文档 source/cursor 同步给 WYSIWYG 插件。
  - 维持 plugin rendering 路径，不把 markdown 内部结构泄漏给 app。
- Modify: `crates/ui/src/plugin.rs`
  - 如需新增纯数据查询/响应，只在这里定义跨层协议。

---

### Task 1: Failing Regression Tests For Span Unfold And Cursor Mapping

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/edit.rs`

**Interfaces:**
- Consumes: existing `MarkdownEditorView`, `PreviewEngine::hit_test_byte()`, `PreviewEngine::cursor_screen_pos()`, `PluginMessage::SetCursorByte`
- Produces: failing tests that prove span 不展开、CJK 光标漂移、点击 roundtrip 不可靠

- [ ] **Step 1: Add a reusable WYSIWYG editor test helper**

Add this helper inside `crates/markdown/src/view.rs` under existing `#[cfg(test)] mod wysiwyg_tests`:

```rust
    struct StubDoc {
        text: String,
    }

    impl StubDoc {
        fn new(text: &str) -> Self {
            Self { text: text.to_string() }
        }
    }

    impl core::document::DocView for StubDoc {
        fn line_count(&self) -> usize {
            self.text.lines().count().max(1)
        }

        fn doc_line_text(&self, line: usize) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Owned(self.text.lines().nth(line).unwrap_or("").to_string())
        }

        fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> std::borrow::Cow<'_, str> {
            let start = range.start.min(self.text.len());
            let end = range.end.min(self.text.len());
            std::borrow::Cow::Owned(self.text[start..end].to_string())
        }

        fn line_byte_offset(&self, line: usize) -> usize {
            let mut byte_offset = 0usize;
            for (idx, segment) in self.text.split_inclusive('\n').enumerate() {
                if idx == line {
                    return byte_offset;
                }
                byte_offset += segment.len();
            }
            self.text.len()
        }

        fn line_byte_length(&self, line: usize) -> usize {
            self.text.lines().nth(line).map(|s| s.len()).unwrap_or(0)
        }

        fn scroll_y(&self) -> f32 {
            0.0
        }

        fn viewport_height(&self) -> f32 {
            600.0
        }
    }

    impl core::document::DocViewMut for StubDoc {
        fn set_scroll_y(&mut self, _y: f32) {}

        fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
            self.text.replace_range(range, text);
        }
    }

    fn render_editor_once(view: &mut MarkdownEditorView, doc: &StubDoc) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ = <MarkdownEditorView as ViewPlugin>::render(
            view,
            doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );
    }
```

- [ ] **Step 2: Add failing test for span source unfolding**

Add this test in the same module:

```rust
    #[test]
    fn editor_render_expands_cursor_span_source_markers() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut doc = StubDoc::new("hello **world** here");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };

        let joined = lines.into_iter().map(|line| line.text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("hello **world** here"),
            "cursor inside bold span must materialize markdown markers, got {joined:?}"
        );
    }
```

Run:

```bash
cargo test -p edit-plus-markdown --lib -- editor_render_expands_cursor_span_source_markers
```

Expected: FAIL because `materialize_text()` is not connected to layout and flat lines still contain `hello world here`.

- [ ] **Step 3: Add failing test for CJK byte roundtrip**

Add:

```rust
    #[test]
    fn hit_test_byte_roundtrip_inside_cjk_bold_span() {
        let mut view = make_view("前缀 **世界** 后缀");
        view.engine_mut().handle_set_cursor_byte("前缀 **世".len());

        let (cursor_x, cursor_y, _cursor_w, cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor should resolve");
        let result = view.engine().hit_test_byte(cursor_x, cursor_y + cursor_h * 0.5, 0.0, 0.0);

        assert_eq!(
            result,
            Some("前缀 **世".len()),
            "CJK cursor screen position must hit-test back to the same source byte"
        );
    }
```

Run:

```bash
cargo test -p edit-plus-markdown --lib -- hit_test_byte_roundtrip_inside_cjk_bold_span
```

Expected: FAIL or expose an off-by-byte result because current code mixes byte span offsets with char positions.

- [ ] **Step 4: Add focused tests for `materialize_text()` byte mapping output**

In `crates/markdown/src/edit.rs`, add pure tests that will pass after Task 2 introduces `MaterializedLine`:

```rust
    #[test]
    fn materialized_line_maps_expanded_bold_markers_to_source_bytes() {
        let source = "hello **world** here";
        let line_text = "hello world here";
        let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
        let ctx = EditContext { cursor_byte: 10 };

        let line = materialize_line(line_text, &spans, source, Some(&ctx));

        assert_eq!(line.text, "hello **world** here");
        assert_eq!(line.visual_char_to_source_byte(6), Some(6));
        assert_eq!(line.visual_char_to_source_byte(8), Some(8));
        assert_eq!(line.source_byte_to_visual_char(10), Some(10));
    }

    #[test]
    fn materialized_line_keeps_folded_text_when_cursor_outside_span() {
        let source = "hello **world** here";
        let line_text = "hello world here";
        let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
        let ctx = EditContext { cursor_byte: 2 };

        let line = materialize_line(line_text, &spans, source, Some(&ctx));

        assert_eq!(line.text, "hello world here");
        assert_eq!(line.visual_char_to_source_byte(6), Some(8));
        assert_eq!(line.source_byte_to_visual_char(10), Some(8));
    }
```

Run:

```bash
cargo test -p edit-plus-markdown --lib -- materialized_line_
```

Expected: FAIL with unresolved `materialize_line`, `visual_char_to_source_byte`, and `source_byte_to_visual_char`.

- [ ] **Step 5: Commit regression tests**

After the tests fail for the expected reasons, commit:

```bash
git add crates/markdown/src/view.rs crates/markdown/src/edit.rs
git commit -m "test(markdown): capture wysiwyg cursor span regressions"
```

---

### Task 2: Introduce MaterializedLine As The Single Mapping Model

**Files:**
- Modify: `crates/markdown/src/edit.rs`

**Interfaces:**
- Produces: `MaterializedLine`, `MaterializedSpan`, `materialize_line()`
- Consumes: `StyleSpan`, `InlineStyle`, `EditContext`

- [ ] **Step 1: Implement materialized mapping data types**

In `crates/markdown/src/edit.rs`, add these types below `span_marker_len()`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedSpan {
    pub start: usize,
    pub len: usize,
    pub style: InlineStyle,
    pub source_range: std::ops::Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedLine {
    pub text: String,
    pub spans: Vec<MaterializedSpan>,
    visual_char_to_source_byte: Vec<usize>,
}

impl MaterializedLine {
    pub fn visual_char_to_source_byte(&self, visual_char: usize) -> Option<usize> {
        self.visual_char_to_source_byte.get(visual_char).copied()
    }

    pub fn source_byte_to_visual_char(&self, source_byte: usize) -> Option<usize> {
        self.visual_char_to_source_byte
            .iter()
            .enumerate()
            .min_by_key(|(_, mapped_byte)| mapped_byte.abs_diff(source_byte))
            .map(|(visual_char, _)| visual_char)
    }
}
```

- [ ] **Step 2: Implement `push_chars_with_source_bytes()` helper**

Add this private helper in `edit.rs`:

```rust
fn push_chars_with_source_bytes(
    output: &mut String,
    visual_to_source: &mut Vec<usize>,
    text: &str,
    source_start: usize,
) {
    for (relative_byte, ch) in text.char_indices() {
        output.push(ch);
        visual_to_source.push(source_start + relative_byte);
    }
}
```

- [ ] **Step 3: Implement `materialize_line()`**

Add this public function in `edit.rs` and keep existing `materialize_text()` as a wrapper for compatibility:

```rust
pub fn materialize_line(
    line_text: &str,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
) -> MaterializedLine {
    let cursor_span = edit_ctx.and_then(|ctx| spans.iter().find(|span| cursor_in_span(span, ctx.cursor_byte)));
    let mut text = String::with_capacity(line_text.len() + cursor_span.map_or(0, |span| {
        span.source_range.len().saturating_sub(span.len)
    }));
    let mut visual_to_source = Vec::with_capacity(line_text.chars().count() + 1);
    let mut materialized_spans = Vec::with_capacity(spans.len());

    let mut folded_byte = 0usize;
    let mut source_byte = spans
        .first()
        .map(|span| span.source_range.start.saturating_sub(span.start))
        .unwrap_or(0);

    for span in spans {
        if span.start > folded_byte {
            let plain = &line_text[folded_byte..span.start];
            push_chars_with_source_bytes(&mut text, &mut visual_to_source, plain, source_byte);
            source_byte += plain.len();
            folded_byte = span.start;
        }

        let materialized_start = text.len();
        if Some(span.source_range.clone()) == cursor_span.map(|cursor| cursor.source_range.clone()) {
            let expanded = &source[span.source_range.clone()];
            push_chars_with_source_bytes(
                &mut text,
                &mut visual_to_source,
                expanded,
                span.source_range.start,
            );
            materialized_spans.push(MaterializedSpan {
                start: materialized_start,
                len: expanded.len(),
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            });
        } else {
            let folded = &line_text[span.start..span.start + span.len];
            let marker_prefix_len = span_marker_len(&span.style).0;
            push_chars_with_source_bytes(
                &mut text,
                &mut visual_to_source,
                folded,
                span.source_range.start + marker_prefix_len,
            );
            materialized_spans.push(MaterializedSpan {
                start: materialized_start,
                len: folded.len(),
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            });
        }

        source_byte = span.source_range.end;
        folded_byte = span.start + span.len;
    }

    if folded_byte < line_text.len() {
        let trailing = &line_text[folded_byte..];
        push_chars_with_source_bytes(&mut text, &mut visual_to_source, trailing, source_byte);
        source_byte += trailing.len();
    }

    visual_to_source.push(source_byte);

    MaterializedLine { text, spans: materialized_spans, visual_char_to_source_byte: visual_to_source }
}

pub fn materialize_text(
    line_text: &str,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
) -> String {
    materialize_line(line_text, spans, source, edit_ctx).text
}
```

- [ ] **Step 4: Run pure edit tests**

Run:

```bash
cargo test -p edit-plus-markdown --lib -- materialize
```

Expected: All `edit.rs` materialize tests pass.

- [ ] **Step 5: Commit materialized mapping model**

```bash
git add crates/markdown/src/edit.rs
git commit -m "feat(markdown): add materialized line source mapping"
```

---

### Task 3: Thread EditContext Through Markdown Layout

**Files:**
- Modify: `crates/markdown/src/layout/context.rs`
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/layout/block.rs`
- Modify: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `MaterializedLine`, `EditContext`
- Produces: layout output whose `FlatLine.text` contains expanded source markers when cursor is inside a span

- [ ] **Step 1: Add edit context to `LayoutCtx`**

In `crates/markdown/src/layout/context.rs`, add fields to `LayoutCtx`:

```rust
    pub(crate) source_text: Option<&'a str>,
    pub(crate) edit_ctx: Option<&'a crate::edit::EditContext>,
```

Update `LayoutCtx::new()` signature:

```rust
    pub fn new(
        doc: &'a dyn core::document::DocView,
        style: &'a crate::style::MarkdownStyle,
        viewport_w: f32,
        mut shaper: Option<&'a mut Shaper>,
        highlighter: Option<&'a dyn crate::builder::CodeHighlighter>,
        source_text: Option<&'a str>,
        edit_ctx: Option<&'a crate::edit::EditContext>,
    ) -> Self {
```

Initialize the new fields in `Self { ... }`:

```rust
            source_text,
            edit_ctx,
```

- [ ] **Step 2: Update all `LayoutCtx::new()` call sites**

In `crates/markdown/src/layout/types.rs`, add fields to `LazyLayout`:

```rust
    source_text: Option<String>,
    edit_ctx: Option<crate::edit::EditContext>,
```

Add setters:

```rust
    pub fn set_edit_source(&mut self, source_text: Option<String>) {
        self.source_text = source_text;
    }

    pub fn set_edit_ctx(&mut self, edit_ctx: Option<crate::edit::EditContext>) {
        self.edit_ctx = edit_ctx;
    }
```

When passing to `LayoutCtx`, use:

```rust
self.source_text.as_deref()
self.edit_ctx.as_ref()
```

For example, the `ensure_visible()` call site should become:

```rust
let mut ctx = super::context::LayoutCtx::new(
    doc,
    style,
    viewport_w,
    Some(shaper),
    highlighter,
    self.source_text.as_deref(),
    self.edit_ctx.as_ref(),
);
```

- [ ] **Step 3: Feed source and edit context from `PreviewEngine::rebuild_layout()`**

In `crates/markdown/src/view.rs`, after creating `LazyLayout`:

```rust
let mut lazy = LazyLayout::new(doc, style, viewport_w, doc_view);
lazy.set_edit_source(self.edit_source.clone());
lazy.set_edit_ctx(self.edit_ctx.clone());
```

Add a field to `PreviewEngine`:

```rust
    edit_source: Option<String>,
```

Initialize it in `PreviewEngine::new()`:

```rust
            edit_source: None,
```

In `MarkdownEditorView::set_source()`, after updating `self.source`, set:

```rust
self.engine.set_edit_source(Some(self.source.clone()));
```

Implement `PreviewEngine::set_edit_source()`:

```rust
    pub fn set_edit_source(&mut self, source: Option<String>) {
        self.edit_source = source;
    }
```

- [ ] **Step 4: Materialize raw lines before wrapping**

In `crates/markdown/src/layout/block.rs`, inside text block layout before `ctx.wrap_text(raw, font_size)`, replace raw line selection with:

```rust
let line_styles = raw_styles.get(line_idx).map(|s| s.as_slice()).unwrap_or(&[]);
let materialized = if let Some(source_text) = ctx.source_text {
    crate::edit::materialize_line(raw, line_styles, source_text, ctx.edit_ctx)
} else {
    crate::edit::materialize_line(raw, line_styles, "", None)
};
let wrapped = ctx.wrap_text(&materialized.text, font_size);
```

When calling `layout_line_with_styles()`, pass converted spans:

```rust
let materialized_styles: Vec<StyleSpan> = materialized
    .spans
    .iter()
    .map(|span| StyleSpan {
        start: span.start,
        len: span.len,
        style: span.style.clone(),
        source_range: span.source_range.clone(),
    })
    .collect();
```

Use `materialized_styles.as_slice()` instead of the original `line_styles`.

- [ ] **Step 5: Apply the same materialization to list item own text**

In the `BlockKind::ListItem` branch in `crates/markdown/src/layout/block.rs`, before `ctx.wrap_text(raw, font_size)`, use the same materialization block from Step 4 and pass `materialized_styles` to `layout_line_with_styles()`.

- [ ] **Step 6: Run span unfold regression**

Run:

```bash
cargo test -p edit-plus-markdown --lib -- editor_render_expands_cursor_span_source_markers
```

Expected: PASS.

- [ ] **Step 7: Run full markdown tests**

Run:

```bash
cargo test -p edit-plus-markdown --lib
```

Expected: PASS.

- [ ] **Step 8: Commit layout integration**

```bash
git add crates/markdown/src/layout/context.rs crates/markdown/src/layout/types.rs crates/markdown/src/layout/block.rs crates/markdown/src/view.rs
git commit -m "feat(markdown): thread edit context into wysiwyg layout"
```

---

### Task 4: Replace Ad Hoc Cursor Mapping With Materialized Line Mapping

**Files:**
- Modify: `crates/markdown/src/layout/types.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/layout/context.rs`

**Interfaces:**
- Consumes: `MaterializedLine`
- Produces: stable byte <-> visual char <-> x mapping for ASCII, CJK, and expanded spans

- [ ] **Step 1: Add flat line source mapping storage**

In `crates/markdown/src/layout/types.rs`, add:

```rust
#[derive(Clone, Debug, Default)]
pub struct FlatLineSourceMap {
    pub flat_idx: usize,
    pub source_bytes_by_visual_char: Vec<usize>,
}
```

Add field to `LazyLayout`:

```rust
    pub flat_line_source_maps: Vec<FlatLineSourceMap>,
```

Initialize it in `LazyLayout::new()`:

```rust
            flat_line_source_maps: Vec::new(),
```

- [ ] **Step 2: Populate source maps in `build_flat_lines()`**

In `build_flat_lines()`, create a `source_maps` vector next to `lines` and `map`.

When flattening a `LaidOutLine`, compute map entries from text if exact materialized map is not yet threaded:

```rust
source_maps.push(FlatLineSourceMap {
    flat_idx,
    source_bytes_by_visual_char: (0..=flat_line.text.chars().count())
        .map(|visual_char| {
            flat_line
                .text
                .char_indices()
                .nth(visual_char)
                .map(|(byte, _)| line_source_start + byte)
                .unwrap_or(line_source_start + flat_line.text.len())
        })
        .collect(),
});
```

After Task 3, replace this fallback with the materialized map attached to each laid-out line. Keep the fallback path for non-WYSIWYG preview.

- [ ] **Step 3: Rename char-based helper variables to visual-char terminology**

In `crates/markdown/src/layout/context.rs`:

```rust
pub(crate) fn char_at_x(flat_line: &FlatLine, rel_x: f32) -> usize
```

Rename local variables only:

```rust
let mut visual_char = 0usize;
```

In `crates/markdown/src/view.rs`, rename parameters:

```rust
fn byte_from_flat_line_and_visual_char(
    &self,
    flat_line_idx: usize,
    visual_char: usize,
) -> Option<usize>
```

This keeps behavior reviewable while making byte/char boundaries explicit.

- [ ] **Step 4: Reimplement `byte_from_flat_line_and_char()` using `flat_line_source_maps`**

In `crates/markdown/src/view.rs`, replace the span-specific block in `byte_from_flat_line_and_char()` with:

```rust
let source_map = lazy.flat_line_source_maps.get(flat_line_idx)?;
let visual_char = self.adjusted_char_offset(lazy, flat_line_idx, char_offset);
source_map.source_bytes_by_visual_char.get(visual_char).copied()
```

Remove the special expanded span branch that manually adds `prefix_len` and `suffix_len`.

- [ ] **Step 5: Reimplement `char_offset_from_byte()` using nearest mapped byte**

In `crates/markdown/src/view.rs`:

```rust
fn char_offset_from_byte(
    &self,
    _block: &crate::builder::BlockNode,
    _line_idx: usize,
    byte: usize,
) -> Option<usize> {
    let lazy = self.lazy.as_ref()?;
    let (block_line_base, line_idx) = lazy.find_block_line_at_byte(byte)?;

    for (flat_idx, &(map_base, map_line)) in lazy.block_line_map.iter().enumerate() {
        if map_base != block_line_base || map_line != line_idx {
            continue;
        }
        let source_map = lazy.flat_line_source_maps.get(flat_idx)?;
        return source_map
            .source_bytes_by_visual_char
            .iter()
            .enumerate()
            .min_by_key(|(_, mapped_byte)| mapped_byte.abs_diff(byte))
            .map(|(visual_char, _)| visual_char);
    }
    None
}
```

- [ ] **Step 6: Run mapping tests**

Run:

```bash
cargo test -p edit-plus-markdown --lib -- hit_test_byte_roundtrip
cargo test -p edit-plus-markdown --lib -- hit_test_byte_roundtrip_inside_cjk_bold_span
cargo test -p edit-plus-markdown --lib -- visual_move_within_single_line_expanded_span
```

Expected: PASS.

- [ ] **Step 7: Commit mapping rewrite**

```bash
git add crates/markdown/src/layout/types.rs crates/markdown/src/view.rs crates/markdown/src/layout/context.rs
git commit -m "fix(markdown): use materialized source maps for wysiwyg cursor"
```

---

### Task 5: Reduce Cursor Dirty Work From Full Layout To Affected Lines

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/layout/types.rs`

**Interfaces:**
- Produces: cursor movement no longer marks whole engine dirty
- Consumes: existing `LazyLayout`, `EditContext`

- [ ] **Step 1: Introduce dirty reason enum**

In `crates/markdown/src/view.rs`, add near `PreviewEngine`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
enum EngineDirty {
    Clean,
    SourceChanged,
    StyleChanged,
    ViewportChanged,
    CursorMoved { old_byte: Option<usize>, new_byte: usize },
}
```

Replace `dirty: bool` field with:

```rust
    dirty: EngineDirty,
```

Initialize:

```rust
            dirty: EngineDirty::SourceChanged,
```

- [ ] **Step 2: Update dirty helpers**

Replace `mark_dirty()` with:

```rust
    pub fn mark_source_dirty(&mut self) {
        self.dirty = EngineDirty::SourceChanged;
        self.sel.clear();
        self.cached_dl = None;
        self.cached_vertices = None;
    }

    fn mark_cursor_moved(&mut self, new_byte: usize) {
        let old_byte = self.edit_ctx.as_ref().map(|ctx| ctx.cursor_byte);
        self.edit_ctx = Some(crate::edit::EditContext { cursor_byte: new_byte });
        self.dirty = EngineDirty::CursorMoved { old_byte, new_byte };
        self.cached_dl = None;
        self.cached_vertices = None;
    }
```

Update source callers from `mark_dirty()` to `mark_source_dirty()`.

- [ ] **Step 3: Stop full rebuild for cursor-only changes**

Change `needs_rebuild()`:

```rust
fn needs_rebuild(&self, style_hash: u64, viewport_w: f32) -> bool {
    matches!(
        self.dirty,
        EngineDirty::SourceChanged | EngineDirty::StyleChanged | EngineDirty::ViewportChanged
    ) || self.lazy.is_none()
        || style_hash != self.cached_style_hash
        || viewport_w != self.cached_viewport_w
}
```

- [ ] **Step 4: Add lazy relayout method for cursor spans**

In `crates/markdown/src/layout/types.rs`, add:

```rust
pub fn invalidate_lines_for_source_bytes(&mut self, bytes: impl IntoIterator<Item = usize>) {
    for byte in bytes {
        let Some((block_line_base, _line_idx)) = self.find_block_line_at_byte(byte) else {
            continue;
        };
        for (laid_idx, doc_idx) in self.laid_to_doc.iter().enumerate() {
            let before = Self::count_block_lines_before(self.source.blocks(), &self.source.blocks()[*doc_idx]);
            if before == block_line_base {
                if let Some(precise) = self.precise.get_mut(laid_idx) {
                    *precise = false;
                }
                if let Some(slot) = self.laid_out.get_mut(laid_idx) {
                    *slot = None;
                }
            }
        }
    }
}
```

If `count_block_lines_before()` is private and not usable here, move the logic into a private helper on `LazyLayout` and keep the public API above.

- [ ] **Step 5: Apply cursor dirty during render**

In `PreviewEngine::render()`, after the rebuild branch and before cache reuse:

```rust
if let EngineDirty::CursorMoved { old_byte, new_byte } = self.dirty.clone() {
    if let Some(lazy) = self.lazy.as_mut() {
        lazy.set_edit_ctx(self.edit_ctx.clone());
        lazy.invalidate_lines_for_source_bytes(old_byte.into_iter().chain(std::iter::once(new_byte)));
        lazy.ensure_visible(
            self.scroll_y,
            viewport_h,
            style,
            viewport_w,
            shaper.as_deref_mut().expect("WYSIWYG cursor relayout requires shaper"),
            Some(&highlighter),
            doc_view,
        );
        lazy.build_flat_lines(doc_view);
    }
    self.dirty = EngineDirty::Clean;
}
```

If `shaper` is `None`, fall back to `mark_source_dirty()` and rebuild through the existing no-shaper branch.

- [ ] **Step 6: Add a test that cursor move preserves parsed source**

In `crates/markdown/src/view.rs`, add:

```rust
    #[test]
    fn cursor_move_does_not_require_source_generation_change() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut doc = StubDoc::new("hello **world** here");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 7);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::NeedsSourceUpdate(7), &doc);
        assert!(matches!(response, PluginResponse::Bool(false)));
    }
```

Run:

```bash
cargo test -p edit-plus-markdown --lib -- cursor_move_does_not_require_source_generation_change
```

Expected: PASS.

- [ ] **Step 7: Commit cursor dirty reduction**

```bash
git add crates/markdown/src/view.rs crates/markdown/src/layout/types.rs
git commit -m "perf(markdown): avoid full layout on wysiwyg cursor moves"
```

---

### Task 6: Synchronize Source And Cursor In App Dispatch

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/dispatch/mouse.rs`
- Modify: `crates/app/src/dispatch/editor.rs`

**Interfaces:**
- Consumes: `PluginMessage::UpdateSource`, `PluginMessage::SetCursorByte`
- Produces: app-level single source of truth for WYSIWYG cursor sync

- [ ] **Step 1: Add app helper to push source and cursor**

In `crates/app/src/app_renderer.rs` or a new app impl section in `crates/app/src/dispatch/editor.rs`, add:

```rust
impl App {
    pub(crate) fn sync_wysiwyg_plugin_state(&mut self) {
        let Some(tab) = self.workspace.active_entry_mut() else {
            return;
        };
        if !tab.plugin.is_wysiwyg() {
            return;
        }

        let generation = tab.doc.tb().gap_buffer().generation();
        let needs_update = matches!(
            tab.plugin.query(ui::plugin::PluginQuery::NeedsSourceUpdate(generation), &tab.doc),
            ui::plugin::PluginResponse::Bool(true)
        );

        if needs_update {
            let gb = tab.doc.tb().gap_buffer();
            let c1 = gb.read_forward(0);
            let c2 = gb.read_forward(c1.len());
            let mut text = String::with_capacity(c1.len() + c2.len());
            text.push_str(&String::from_utf8_lossy(c1));
            text.push_str(&String::from_utf8_lossy(c2));
            tab.plugin.handle_message(
                ui::plugin::PluginMessage::UpdateSource { text, generation },
                &mut tab.doc,
            );
        }

        let cursor_byte = tab.doc.cursor_offset().to_usize();
        tab.plugin.handle_message(ui::plugin::PluginMessage::SetCursorByte(cursor_byte), &mut tab.doc);
    }
}
```

- [ ] **Step 2: Use helper before WYSIWYG hit testing**

In `crates/app/src/dispatch/mouse.rs`, before `HitTestByte` in the WYSIWYG branch:

```rust
self.sync_wysiwyg_plugin_state();
```

Then after `tab.doc.cursor_move_to_offset(byte)`, replace direct `SetCursorByte` with:

```rust
tab.plugin.handle_message(ui::plugin::PluginMessage::SetCursorByte(byte), &mut tab.doc);
```

Keep the explicit message here because click target byte is known and should not wait for renderer.

- [ ] **Step 3: Use helper after successful text edits**

In `crates/app/src/dispatch/editor.rs`, after `execute_edit_command_v2()` reports `outcome.executed` and before returning:

```rust
let is_wysiwyg = self.workspace.active_entry().is_some_and(|tab| tab.plugin.is_wysiwyg());
if is_wysiwyg {
    self.sync_wysiwyg_plugin_state();
}
```

Place this after `reset_cursor_after_edit(&mut dv.cursor_render_state);` and after releasing the mutable `dv` borrow.

- [ ] **Step 4: Avoid duplicate source update code in renderer**

In `crates/app/src/app_renderer.rs`, keep existing source update path for preview and novel plugins. For WYSIWYG plugins, call:

```rust
if self.workspace.active_entry().is_some_and(|tab| tab.plugin.is_wysiwyg()) {
    self.sync_wysiwyg_plugin_state();
} else {
    // existing NeedsSourceUpdate / UpdateSource block
}
```

- [ ] **Step 5: Add app-level test for cursor sync helper**

In `crates/app/src/app_tests.rs`, add a recording WYSIWYG plugin and a test for `sync_wysiwyg_plugin_state()`:

```rust
#[derive(Default)]
struct RecordingWysiwygState {
    source_text: String,
    generation: u32,
    cursor_byte: Option<usize>,
}

struct RecordingWysiwygPlugin {
    state: std::rc::Rc<std::cell::RefCell<RecordingWysiwygState>>,
}

impl RecordingWysiwygPlugin {
    fn new(state: std::rc::Rc<std::cell::RefCell<RecordingWysiwygState>>) -> Self {
        Self { state }
    }
}

impl ui::plugin::ViewPlugin for RecordingWysiwygPlugin {
    fn name(&self) -> &str {
        "recording_wysiwyg"
    }

    fn render(
        &mut self,
        _doc: &dyn core::document::DocView,
        _bounds: ui::core::geom::Rect,
        _theme: &ui::theme::Theme,
        _shaper: &mut shaping::Shaper,
        _dpi_scale: f32,
    ) -> ui::core::paint::DrawList {
        ui::core::paint::DrawList::new()
    }

    fn allows_editing(&self) -> bool {
        true
    }

    fn handles_own_rendering(&self) -> bool {
        true
    }

    fn is_wysiwyg(&self) -> bool {
        true
    }

    fn query(
        &self,
        query: ui::plugin::PluginQuery,
        _doc: &dyn core::document::DocView,
    ) -> ui::plugin::PluginResponse {
        match query {
            ui::plugin::PluginQuery::NeedsSourceUpdate(generation) => {
                ui::plugin::PluginResponse::Bool(generation != self.state.borrow().generation)
            }
            _ => ui::plugin::PluginResponse::None,
        }
    }

    fn handle_message(
        &mut self,
        msg: ui::plugin::PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        match msg {
            ui::plugin::PluginMessage::UpdateSource { text, generation } => {
                let mut state = self.state.borrow_mut();
                state.source_text = text;
                state.generation = generation;
                true
            }
            ui::plugin::PluginMessage::SetCursorByte(byte) => {
                self.state.borrow_mut().cursor_byte = Some(byte);
                true
            }
            _ => false,
        }
    }
}

#[test]
fn sync_wysiwyg_plugin_state_pushes_source_and_cursor() {
    let state = std::rc::Rc::new(std::cell::RefCell::new(RecordingWysiwygState::default()));
    let mut app = App::new(None);
    let mut doc = DocumentView::new(vec!["hello **world**".to_string()], 80, 10.0);
    doc.cursor_move_to_offset("hello **world**".len());
    app.workspace.push_entry_for_test(DocItem::new(
        doc,
        Box::new(RecordingWysiwygPlugin::new(state.clone())),
    ));
    app.workspace.switch_to(0);

    app.sync_wysiwyg_plugin_state();

    let recorded = state.borrow();
    assert_eq!(recorded.source_text, "hello **world**");
    assert_eq!(recorded.cursor_byte, Some("hello **world**".len()));
}
```

- [ ] **Step 6: Run app checks**

Run:

```bash
cargo test -p edit-plus-app --lib -- wysiwyg
cargo check -p edit-plus-app
```

Expected: PASS.

- [ ] **Step 7: Commit app sync**

```bash
git add crates/app/src/app_renderer.rs crates/app/src/dispatch/mouse.rs crates/app/src/dispatch/editor.rs crates/app/src/app_tests.rs
git commit -m "fix(app): synchronize wysiwyg source and cursor state"
```

---

### Task 7: Wire WYSIWYG Keyboard Navigation And Augmentation

**Files:**
- Modify: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/dispatch/wysiwyg.rs`

**Interfaces:**
- Consumes: existing `dispatch_wysiwyg_navigation()`, `dispatch_wysiwyg_augmented_enter()`, `dispatch_wysiwyg_augmented_backspace()`
- Produces: WYSIWYG mode uses plugin visual navigation instead of standard editor display-map navigation

- [ ] **Step 1: Add pure route classifier**

In `crates/app/src/dispatch/editor.rs`, add this enum above `impl App`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WysiwygCommandRoute {
    Navigation,
    AugmentedEnter,
    AugmentedBackspace,
}
```

Add this helper:

```rust
fn wysiwyg_route_for_command(cmd: &EditCommand) -> Option<WysiwygCommandRoute> {
    match cmd {
        EditCommand::MoveLeft
        | EditCommand::MoveRight
        | EditCommand::MoveUp
        | EditCommand::MoveDown
        | EditCommand::MoveToLineStart
        | EditCommand::MoveToLineEnd
        | EditCommand::MoveToDocStart
        | EditCommand::MoveToDocEnd
        | EditCommand::PageUp
        | EditCommand::PageDown => Some(WysiwygCommandRoute::Navigation),
        EditCommand::InsertNewline => Some(WysiwygCommandRoute::AugmentedEnter),
        EditCommand::Backspace => Some(WysiwygCommandRoute::AugmentedBackspace),
        EditCommand::MoveWordLeft | EditCommand::MoveWordRight => None,
        _ => None,
    }
}
```

- [ ] **Step 2: Add WYSIWYG command gate near top of `dispatch_edit_command()`**

In `crates/app/src/dispatch/editor.rs`, after preview-mode blocking and before sidebar mode:

```rust
        if self.workspace.active_entry().is_some_and(|tab| tab.plugin.is_wysiwyg())
            && !self.wysiwyg_recursing
        {
            match wysiwyg_route_for_command(&cmd) {
                Some(WysiwygCommandRoute::Navigation) => {
                    return self.dispatch_wysiwyg_navigation(&cmd, event_loop);
                }
                Some(WysiwygCommandRoute::AugmentedEnter) => {
                    return self.dispatch_wysiwyg_augmented_enter(event_loop);
                }
                Some(WysiwygCommandRoute::AugmentedBackspace) => {
                    return self.dispatch_wysiwyg_augmented_backspace(event_loop);
                }
                None => {}
            }
        }
```

- [ ] **Step 3: Keep word navigation on standard byte path for now**

Do not map `MoveWordLeft` or `MoveWordRight` to one-character WYSIWYG movement. Let those commands fall through to the existing editor command path until word-aware WYSIWYG mapping is implemented.

- [ ] **Step 4: Fix `dispatch_wysiwyg_augmented_enter()` cursor result**

In `crates/app/src/dispatch/wysiwyg.rs`, after recursive dispatch, sync plugin state:

```rust
self.sync_wysiwyg_plugin_state();
```

Place it after `self.wysiwyg_recursing = false;`.

- [ ] **Step 5: Simplify Backspace hook**

In `dispatch_wysiwyg_augmented_backspace()`, remove the unused augmentation query and keep:

```rust
    pub(crate) fn dispatch_wysiwyg_augmented_backspace(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect {
        self.wysiwyg_recursing = true;
        let result = self.dispatch_edit_command(EditCommand::Backspace, event_loop);
        self.wysiwyg_recursing = false;
        self.sync_wysiwyg_plugin_state();
        result
}
```

- [ ] **Step 6: Add route classifier tests**

In `crates/app/src/dispatch/editor.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wysiwyg_route_maps_arrow_keys_to_navigation() {
        assert_eq!(
            wysiwyg_route_for_command(&EditCommand::MoveRight),
            Some(WysiwygCommandRoute::Navigation)
        );
        assert_eq!(
            wysiwyg_route_for_command(&EditCommand::MoveDown),
            Some(WysiwygCommandRoute::Navigation)
        );
    }

    #[test]
    fn wysiwyg_route_maps_edit_hooks() {
        assert_eq!(
            wysiwyg_route_for_command(&EditCommand::InsertNewline),
            Some(WysiwygCommandRoute::AugmentedEnter)
        );
        assert_eq!(
            wysiwyg_route_for_command(&EditCommand::Backspace),
            Some(WysiwygCommandRoute::AugmentedBackspace)
        );
    }

    #[test]
    fn wysiwyg_route_leaves_word_navigation_on_standard_path() {
        assert_eq!(wysiwyg_route_for_command(&EditCommand::MoveWordLeft), None);
        assert_eq!(wysiwyg_route_for_command(&EditCommand::MoveWordRight), None);
    }
}
```

- [ ] **Step 7: Run app dispatch tests**

Run:

```bash
cargo test -p edit-plus-app --lib -- wysiwyg_route
cargo check -p edit-plus-app
```

Expected: PASS.

- [ ] **Step 8: Commit keyboard wiring**

```bash
git add crates/app/src/dispatch/editor.rs crates/app/src/dispatch/wysiwyg.rs
git commit -m "fix(app): route keyboard commands through wysiwyg dispatch"
```

---

### Task 8: Cursor Rendering Ownership And Blink Correctness

**Files:**
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app_window.rs`

**Interfaces:**
- Consumes: `ViewPlugin::shows_cursor()`, plugin-rendered cursor rect
- Produces: one visible WYSIWYG cursor with app-controlled blink wakeup

- [ ] **Step 1: Add explicit cursor rendering capability**

In `crates/ui/src/plugin.rs`, add a default trait method:

```rust
    fn needs_cursor_blink_wakeup(&self) -> bool {
        self.shows_cursor()
    }
```

- [ ] **Step 2: Override WYSIWYG blink capability**

In `MarkdownEditorView` implementation:

```rust
    fn needs_cursor_blink_wakeup(&self) -> bool {
        true
    }
```

Keep `shows_cursor()` true if app lifecycle uses it only for wakeup after Step 3. If `shows_cursor()` is also used by standard editor drawing, set:

```rust
    fn shows_cursor(&self) -> bool {
        false
    }
```

and update app wakeup sites to use `needs_cursor_blink_wakeup()`.

- [ ] **Step 3: Update app wakeup checks**

In `crates/app/src/app_lifecycle.rs` and `crates/app/src/app_window.rs`, replace:

```rust
t.plugin.shows_cursor()
```

with:

```rust
t.plugin.needs_cursor_blink_wakeup()
```

- [ ] **Step 4: Make plugin cursor obey blink phase**

In `crates/ui/src/plugin.rs`, add:

```rust
    SetCursorVisible(bool),
```

to `PluginMessage`.

In `PreviewEngine`, add:

```rust
    cursor_visible: bool,
```

Initialize to `true`, handle message:

```rust
PluginMessage::SetCursorVisible(visible) => {
    self.cursor_visible = *visible;
    Some(true)
}
```

In `MarkdownEditorView::render()`, wrap cursor draw:

```rust
if self.engine.cursor_visible {
    if let Some((cx, cy, cw, ch)) = self.engine.cursor_screen_pos() {
        let cursor_rect = ui::core::geom::Rect::new(bounds.x + cx, bounds.y + cy, cw, ch);
        dl.fill(cursor_rect, theme.editor.cursor);
    }
}
```

- [ ] **Step 5: Send blink state before rendering**

In `crates/app/src/app_renderer.rs`, before plugin render:

```rust
if tab.plugin.is_wysiwyg() {
    let visible = tab
        .doc
        .cursor_render_state
        .cursor_blink_instant
        .elapsed()
        .as_millis()
        % 1000
        < 500;
    tab.plugin.handle_message(ui::plugin::PluginMessage::SetCursorVisible(visible), &mut tab.doc);
}
```

If a helper already computes cursor phase, use the existing helper instead of duplicating the modulo logic.

- [ ] **Step 6: Update cursor render test**

In `crates/markdown/src/view.rs`, add:

```rust
    #[test]
    fn editor_view_hides_cursor_when_blink_phase_hidden() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, ViewPlugin};

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut doc = StubDoc::new("hello world");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        view.handle_message(PluginMessage::SetCursorVisible(false), &mut doc);

        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let dl = <MarkdownEditorView as ViewPlugin>::render(
            &mut view,
            &doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        let has_cursor_fill = dl.cmds.iter().any(
            |cmd| matches!(cmd, DrawCmd::FillRect { color, .. } if *color == theme.editor.cursor),
        );
        assert!(!has_cursor_fill, "hidden blink phase must not draw WYSIWYG cursor");
    }
```

- [ ] **Step 7: Run cursor tests**

Run:

```bash
cargo test -p edit-plus-markdown --lib -- editor_view_
cargo check -p edit-plus-app
```

Expected: PASS.

- [ ] **Step 8: Commit cursor ownership**

```bash
git add crates/ui/src/plugin.rs crates/markdown/src/view.rs crates/app/src/app_lifecycle.rs crates/app/src/app_window.rs crates/app/src/app_renderer.rs
git commit -m "fix(markdown): align wysiwyg cursor blink ownership"
```

---

### Task 9: Final Verification And Documentation Update

**Files:**
- Modify: `docs/plans/2026-06-23-wysiwyg-crash-fix-and-cursor-render-review.md`
- Modify: `docs/superpowers/specs/2026-06-23-markdown-wysiwyg-editor-design.md`

**Interfaces:**
- Produces: docs that record the final source mapping and dirty strategy

- [ ] **Step 1: Update review doc with resolved items**

In `docs/plans/2026-06-23-wysiwyg-crash-fix-and-cursor-render-review.md`, add a section:

```markdown
## 7. 2026-06-25 Follow-up Resolution

- `materialize_text()` is now wired into layout through `LayoutCtx`, so cursor span expansion affects actual `FlatLine` text.
- WYSIWYG cursor mapping uses materialized source maps instead of ad hoc byte arithmetic.
- Cursor movement no longer marks the full Markdown engine as source-dirty.
- WYSIWYG keyboard movement is routed through `dispatch_wysiwyg_navigation()`.
- Cursor blink ownership is explicit through plugin wakeup and cursor visibility messages.
```

- [ ] **Step 2: Update design spec implementation notes**

In `docs/superpowers/specs/2026-06-23-markdown-wysiwyg-editor-design.md`, update Section 9 with:

```markdown
Implementation note: cursor screen resolution is based on `MaterializedLine` source maps. The map is produced before wrapping, sliced with wrapped lines, and then used by `HitTestByte`, `CursorScreenPos`, and visual movement. Inline marker expansion therefore has one source of truth.
```

- [ ] **Step 3: Run formatting and targeted tests**

Run:

```bash
cargo fmt
cargo test -p edit-plus-markdown --lib -- wysiwyg
cargo test -p edit-plus-app --lib -- wysiwyg
cargo check -p edit-plus-app
```

Expected: all commands pass.

- [ ] **Step 4: Run full verification**

Run:

```bash
./scripts/verify.sh
```

Expected: script exits with status 0.

- [ ] **Step 5: Commit docs and final verification**

```bash
git add docs/plans/2026-06-23-wysiwyg-crash-fix-and-cursor-render-review.md docs/superpowers/specs/2026-06-23-markdown-wysiwyg-editor-design.md
git commit -m "docs(markdown): record wysiwyg cursor mapping resolution"
```

---

## Self-Review

**Spec coverage:** The plan covers span 展开、点击命中、光标位置、CJK byte/char 映射、点击卡顿、输入后同步、键盘 WYSIWYG 接线、光标闪烁职责和最终验证。

**Placeholder scan:** No unresolved placeholder sections are intentionally left in this plan. Each task includes exact files, test snippets, expected command outcomes, and commit commands.

**Type consistency:** The plan consistently uses `EditContext`, `MaterializedLine`, `MaterializedSpan`, `PreviewEngine`, `LazyLayout`, `PluginMessage`, `PluginQuery`, `PluginResponse`, and `MarkdownEditorView` with names matching existing project conventions or names introduced in earlier tasks.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-25-markdown-wysiwyg-cursor-span-optimization.md`. Two execution options:

1. Subagent-Driven (recommended) - dispatch a fresh subagent per task, review between tasks, fast iteration.
2. Inline Execution - execute tasks in this session using executing-plans, batch execution with checkpoints.
