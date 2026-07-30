# Markdown Empty Document DPI Typography Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让高 DPI 环境中的 Markdown WYSIWYG 初始空文档光标、IME preedit 和 `CursorScreenPos` 使用正确的物理字号。

**Architecture:** `PreviewEngine` 保留逻辑编辑器设置，同时记录最近一次 `MarkdownStyle` 中的物理正文排版指标。只有找不到实际或相邻 `FlatLine` 的空文档兜底路径使用这组指标，已有内容的排版与光标几何保持不变。

**Tech Stack:** Rust、textora-markdown、`MarkdownStyle`、`ui::plugin::ViewPlugin`

## Global Constraints

- 全程使用中文沟通。
- Bug 修复必须先看到回归测试按预期失败，再修改生产代码。
- 不修改 `crates/ui` 与 `crates/app` 的依赖层次或插件协议。
- 不覆盖 `.superpowers/sdd/task-3-report.md` 的现有用户改动。
- 提交前运行 `cargo fmt --all --check` 和 `cargo check -p textora-markdown`。

---

### Task 1: 使用物理排版指标渲染空文档输入状态

**Files:**
- Modify/Test: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `MarkdownStyle::body_font_size`、`MarkdownStyle::line_height`、`MarkdownEditorView::render`
- Produces: `PreviewEngine.rendered_body_font_size: f32`、`PreviewEngine.rendered_line_height: f32`

- [ ] **Step 1: 添加高 DPI 失败回归测试**

在 `new_empty_editor_renders_preedit_without_source_update` 后添加：

```rust
#[test]
fn new_empty_editor_uses_physical_typography_at_high_dpi() {
    use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

    let mut document = StubDoc::new("");
    let mut view = MarkdownEditorView::new();
    view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
    view.handle_message(
        PluginMessage::SetPreedit {
            text: "拼音".into(),
            cursor: Some((6, 6)),
        },
        &mut document,
    );

    let dpi_scale = 2.0;
    let expected_font_size = view.engine().base_font_size * dpi_scale;
    let draw_list =
        render_editor_draw_list_with_dpi(&mut view, &document, dpi_scale);
    let cursor_rect = editor_cursor_rect_from_draw_list(&draw_list);
    let preedit_font_size = draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            ui::core::paint::DrawCmd::TextLayout { layout, .. }
                if layout.text == "拼音" =>
            {
                Some(layout.font_size)
            }
            _ => None,
        })
        .expect("new empty editor should emit the IME preedit text layout");
    let queried_cursor_height = match view.query(PluginQuery::CursorScreenPos(0), &document) {
        PluginResponse::CursorScreenRect(Some((_, _, _, height))) => height,
        other => panic!("expected preedit cursor rect, got {other:?}"),
    };

    assert!((cursor_rect.h - expected_font_size).abs() < 0.01);
    assert!((preedit_font_size - expected_font_size).abs() < 0.01);
    assert!((queried_cursor_height - expected_font_size).abs() < 0.01);
}
```

- [ ] **Step 2: 运行测试并确认按根因失败**

Run:

```bash
cargo test -p textora-markdown --lib new_empty_editor_uses_physical_typography_at_high_dpi
```

Expected: FAIL；`cursor_rect.h` 实际为 15，期望为 30。失败必须来自空文档兜底仍使用逻辑字号，而不是测试编译错误。

- [ ] **Step 3: 在 PreviewEngine 中记录本帧物理指标**

在 `PreviewEngine` 的 `base_font_size`、`base_line_height` 后增加：

```rust
rendered_body_font_size: f32,
rendered_line_height: f32,
```

在 `PreviewEngine::new()` 中初始化：

```rust
rendered_body_font_size: 15.0,
rendered_line_height: 24.0,
```

在 `PreviewEngine::render()` 计算 `style_hash` 前更新：

```rust
self.rendered_body_font_size = style.body_font_size;
self.rendered_line_height = style.line_height;
```

将 `empty_source_line_metrics()` 最后的无相邻行回退改为：

```rust
(
    0.0,
    self.rendered_line_height * source_line.index as f32,
    self.rendered_body_font_size,
    self.rendered_line_height,
)
```

将 `empty_source_line_typography()` 最后的无相邻行回退改为：

```rust
(0.0, self.rendered_body_font_size, self.rendered_line_height)
```

- [ ] **Step 4: 重新运行定向测试**

Run:

```bash
cargo test -p textora-markdown --lib new_empty_editor_uses_physical_typography_at_high_dpi
```

Expected: PASS；空文档光标、preedit 和查询矩形高度均为 30px。

- [ ] **Step 5: 运行完整验证**

Run:

```bash
cargo fmt --all
cargo fmt --all --check
cargo test -p textora-markdown --lib
cargo check -p textora-markdown
git diff --check
git status --short
```

Expected:

- 格式检查通过。
- Markdown 全部单元测试通过。
- 编译检查退出码为 0。
- 无空白错误。
- 状态只包含 `crates/markdown/src/view.rs`，以及原有的 `.superpowers/sdd/task-3-report.md` 用户改动。

- [ ] **Step 6: 提交修复**

Run:

```bash
git add crates/markdown/src/view.rs docs/plans/2026-07-24-markdown-empty-document-dpi-typography.md
git commit -m "fix(markdown): scale empty document typography for dpi"
```

Expected: 提交成功，且 `.superpowers/sdd/task-3-report.md` 不在提交中。
