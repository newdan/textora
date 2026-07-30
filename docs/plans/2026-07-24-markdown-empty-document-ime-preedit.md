# Markdown Empty Document IME Preedit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用回归测试保证新建空 Markdown WYSIWYG 文档无需 `UpdateSource` 也能显示 IME preedit 文字及组合光标。

**Architecture:** 测试直接驱动 `MarkdownEditorView` 的公开插件消息接口，复现应用层的 `SetCursorByte(0) → SetPreedit → render` 顺序。空文档光标修复已经建立 `edit_source = Some("")` 的初始化不变量，因此本计划只固化行为；若测试不通过，则停止并重新定位独立根因。

**Tech Stack:** Rust、textora-markdown、`ui::plugin::ViewPlugin`、`ui::core::paint::DrawList`

## Global Constraints

- 全程使用中文沟通。
- 遵守 `crates/ui` 与 `crates/app` 的跨层解耦红线。
- 不修改或覆盖 `.superpowers/sdd/task-3-report.md` 的现有用户改动。
- 提交前运行 `cargo fmt --check` 与 `cargo check -p textora-markdown`。

---

### Task 1: 固化新建空文档首次 IME 组合输入

**Files:**
- Modify/Test: `crates/markdown/src/view.rs` 的 `wysiwyg_tests` 模块

**Interfaces:**
- Consumes: `MarkdownEditorView::new()`、`ViewPlugin::handle_message()`、`ViewPlugin::render()`、`ViewPlugin::query()`
- Produces: 回归测试 `new_empty_editor_renders_preedit_without_source_update`

- [ ] **Step 1: 添加精确回归测试**

在 `new_empty_editor_draws_cursor_without_source_update` 后添加：

```rust
#[test]
fn new_empty_editor_renders_preedit_without_source_update() {
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

    let draw_list = render_editor_draw_list(&mut view, &document);
    let (preedit_x, preedit_width) = draw_list
        .cmds
        .iter()
        .find_map(|command| match command {
            ui::core::paint::DrawCmd::TextLayout { layout, x, .. }
                if layout.text == "拼音" =>
            {
                Some((*x, layout.shaped.width))
            }
            _ => None,
        })
        .expect("new empty editor should emit the IME preedit text layout");
    let cursor_x = match view.query(PluginQuery::CursorScreenPos(0), &document) {
        PluginResponse::CursorScreenRect(Some((x, _, _, _))) => x,
        other => panic!("expected preedit cursor rect, got {other:?}"),
    };

    assert!(
        (cursor_x - (preedit_x + preedit_width)).abs() < 0.01,
        "preedit cursor must follow shaped text: cursor={cursor_x}, text_end={}",
        preedit_x + preedit_width
    );
}
```

- [ ] **Step 2: 运行定向测试**

Run:

```bash
cargo test -p textora-markdown --lib new_empty_editor_renders_preedit_without_source_update
```

Expected: PASS。该行为由已提交的空文档初始化修复提供；若失败，保留失败输出并停止实施，重新执行系统化根因分析。

- [ ] **Step 3: 格式化并运行 Markdown 完整测试**

Run:

```bash
cargo fmt --all --check
cargo test -p textora-markdown --lib
cargo check -p textora-markdown
```

Expected:

- `cargo fmt --all --check` 退出码为 0。
- `cargo test -p textora-markdown --lib` 全部通过。
- `cargo check -p textora-markdown` 退出码为 0。

- [ ] **Step 4: 检查改动范围**

Run:

```bash
git diff --check
git status --short
```

Expected: 仅 `crates/markdown/src/view.rs` 以及原有
`.superpowers/sdd/task-3-report.md` 用户改动出现在状态中；没有空白错误。

- [ ] **Step 5: 提交回归测试**

Run:

```bash
git add crates/markdown/src/view.rs docs/plans/2026-07-24-markdown-empty-document-ime-preedit.md
git commit -m "test(markdown): cover empty document ime preedit"
```

Expected: 提交成功，且 `.superpowers/sdd/task-3-report.md` 不在提交中。
