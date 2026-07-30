# Markdown WYSIWYG 空文档输入光标修复实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建空 Markdown 文档首次渲染时，在字节位置 0 显示 WYSIWYG 输入光标。

**Architecture:** 保持 Markdown 插件自行渲染光标的现有边界。新建插件时让 `PreviewEngine` 的编辑源与 `MarkdownEditorView::source` 的初始空字符串保持一致，从而在 app 因 generation 同为 0 而跳过首次 `UpdateSource` 时，空文档光标仍能计算几何。

**Tech Stack:** Rust、`textora-markdown`、现有 `ViewPlugin`/`DrawList` 单元测试。

## Global Constraints

- 不在 app 层增加空文档光标特判。
- 不改变非空文档、Markdown 预览模式、光标颜色、宽度或闪烁周期。
- 先观察失败测试，再写生产代码；只修改 `crates/markdown/src/view.rs`。
- 提交前运行 `cargo fmt --check`、目标测试和 `cargo check -p textora-markdown`。

---

### Task 1: 初始化空编辑源并验证首次光标渲染

**Files:**
- Modify/Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `MarkdownEditorView::new()`, `PluginMessage::SetCursorByte(usize)`, `ViewPlugin::render(...)`, `editor_cursor_rect_from_draw_list(&DrawList)`。
- Produces: `MarkdownEditorView::new()` 返回一个 `engine.edit_source == Some("")` 的编辑视图；公共 API 不变。

- [ ] **Step 1: 写入精确复现新空文档首次渲染的失败测试**

在 `wysiwyg_tests` 中加入测试。不要调用 `set_source` 或发送 `UpdateSource`，以复现 app 发现文档 generation 与插件缓存 generation 都是 0 时的真实路径：

```rust
#[test]
fn new_empty_editor_draws_cursor_without_source_update() {
    use ui::plugin::{PluginMessage, ViewPlugin};

    let mut document = StubDoc::new("");
    let mut view = MarkdownEditorView::new();
    view.handle_message(PluginMessage::SetCursorByte(0), &mut document);

    let draw_list = render_editor_draw_list(&mut view, &document);
    let cursor_rect = editor_cursor_rect_from_draw_list(&draw_list);

    assert!(cursor_rect.x + cursor_rect.w > 0.0, "empty document cursor must overlap editor bounds");
    assert!(cursor_rect.y + cursor_rect.h > 0.0, "empty document cursor must overlap editor bounds");
}
```

- [ ] **Step 2: 运行目标测试并确认 RED**

Run:

```bash
cargo test -p textora-markdown --lib view::wysiwyg_tests::new_empty_editor_draws_cursor_without_source_update -- --exact
```

Expected: FAIL，`editor_cursor_rect_from_draw_list` 报告缺少光标填充矩形；失败原因必须是空编辑源未初始化，而不是编译错误或测试夹具错误。

- [ ] **Step 3: 在构造函数中同步初始化空编辑源**

将 `MarkdownEditorView::new()` 改为先初始化引擎的编辑源，再构造视图：

```rust
pub fn new() -> Self {
    let mut engine = PreviewEngine::new();
    engine.set_edit_source(Some(String::new()));
    Self { engine, source: String::new(), cached_source_hash: 0, cached_generation: 0 }
}
```

该修改只修复内部初始状态不一致：`source` 已经是空字符串，因此 `edit_source` 也应表示已知的空字符串，而不是未知来源。

- [ ] **Step 4: 运行目标测试并确认 GREEN**

Run:

```bash
cargo test -p textora-markdown --lib view::wysiwyg_tests::new_empty_editor_draws_cursor_without_source_update -- --exact
```

Expected: PASS，绘制列表包含空文档光标矩形。

- [ ] **Step 5: 运行相关回归测试**

Run:

```bash
cargo test -p textora-markdown --lib -- editor_view_render_draws_cursor_rect_when_cursor_set
cargo test -p textora-markdown --lib -- editor_view_hides_cursor_when_blink_phase_hidden
cargo test -p textora-markdown --lib -- empty_document_flat_lines_is_empty
```

Expected: 三组测试全部 PASS，证明非空光标、闪烁隐藏和空布局语义保持不变。

- [ ] **Step 6: 格式化并编译检查**

Run:

```bash
cargo fmt --check
cargo check -p textora-markdown
```

Expected: 两个命令退出码均为 0，无格式差异或编译错误。

- [ ] **Step 7: 检查变更并提交代码**

Run:

```bash
git diff --check
git diff -- crates/markdown/src/view.rs
git add crates/markdown/src/view.rs
git commit -m "fix(markdown): show caret in new empty document"
```

Expected: 仅包含目标测试和构造函数的最小修复；提交成功。
