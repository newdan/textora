# 关闭最后一个文件后的默认文档实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 关闭最后一个文件后自动创建一个干净、无路径且可编辑的默认未命名文档，使 sidebar 不再保留已关闭文件的旧状态。

**Architecture:** `Workspace` 继续只负责条目增删并允许短暂为空；`App::handle_workspace_effect` 在处理 `NavEffect::ActiveChanged` 时恢复应用级“至少一个文档”不变量。默认文档复用 `Workspace::new_untitled`，随后沿用现有活动文档初始化、布局、重绘和持久化流程。

**Tech Stack:** Rust、textora-app、现有 Workspace/AppEffect 导航状态流、Cargo test/check/fmt。

## Global Constraints

- 默认文档必须内容为空、无文件路径、初始干净并可编辑。
- 固定文件、取消保存、关闭非最后一个文件的既有行为不变。
- 单项关闭和批量关闭最终都通过同一个应用层不变量恢复逻辑。
- `ui` 不得依赖 `app` 状态结构体；本修复仅位于 `crates/app`。
- 不新增依赖，不做无关重构，不使用 `.unwrap()`。

## 文件结构

- Modify/Test: `crates/app/src/dispatch/tabs.rs`：应用工作区效果处理和对应的关闭回归测试均位于此文件，修复不跨越其他职责边界。

---

### Task 1: 在活动文档关闭后恢复默认文档不变量

**Files:**
- Modify/Test: `crates/app/src/dispatch/tabs.rs:25-57,460-477`

**Interfaces:**
- Consumes: `Workspace::is_empty() -> bool`、`Workspace::new_untitled(ViewportDimensions) -> NavEffect`、`App::viewport_dimensions(f32) -> ViewportDimensions`。
- Produces: `App::handle_workspace_effect(NavEffect) -> AppEffect` 在 `ActiveChanged` 导致工作区为空时创建并初始化一个默认文档。

- [x] **Step 1: 将现有回归测试改为目标行为**

把 `closing_the_last_tab_handles_active_changed_without_an_active_document` 替换为：

```rust
#[test]
fn closing_the_last_tab_creates_an_editable_default_document() {
    let mut app = App::new(None);
    app.workspace.new_untitled(test_viewport());

    let workspace_effect =
        app.workspace.close_entry(0).expect("the only unpinned tab should close");
    let app_effect = app.handle_workspace_effect(workspace_effect);

    assert!(app_effect.redraw);
    assert_eq!(app.workspace.len(), 1);
    assert_eq!(app.workspace.active_index(), 0);

    let default_entry = app.workspace.active_entry().expect("a default document should remain");
    assert_eq!(default_entry.doc_title(), "untitled");
    assert_eq!(default_entry.doc.buffer_len(), 0);
    assert!(default_entry.doc.file_path.is_none());
    assert!(!default_entry.doc.dirty);

    app.workspace
        .active_doc_mut()
        .expect("the default document should be editable")
        .insert_at_cursor(b"x");
    let edited_document = app.workspace.active_doc().expect("default document exists");
    assert_eq!(edited_document.buffer_len(), 1);
    assert!(edited_document.dirty);
}
```

- [x] **Step 2: 运行精确测试并确认 RED**

Run:

```bash
cargo test -p textora-app --lib dispatch::tabs::tests::closing_the_last_tab_creates_an_editable_default_document -- --exact
```

Expected: FAIL，因为当前 `handle_workspace_effect` 处理后 `app.workspace.len()` 为 `0`，而测试期望 `1`。

- [x] **Step 3: 在应用层恢复默认文档**

在 `handle_workspace_effect` 开头、进入 `match` 前加入：

```rust
let effect = if matches!(effect, crate::navigator::NavEffect::ActiveChanged)
    && self.workspace.is_empty()
{
    let viewport = self.viewport_dimensions(self.screen_height());
    effect.merge(self.workspace.new_untitled(viewport))
} else {
    effect
};
```

保持后续 `match effect` 不变，让新文档继续走既有 `ActiveChanged` 初始化和 AppEffect 合并逻辑。

- [x] **Step 4: 运行精确测试并确认 GREEN**

Run:

```bash
cargo test -p textora-app --lib dispatch::tabs::tests::closing_the_last_tab_creates_an_editable_default_document -- --exact
```

Expected: PASS。

- [x] **Step 5: 格式化并运行 app 库测试**

Run:

```bash
cargo fmt --all
cargo test -p textora-app --lib
```

Expected: 全部测试 PASS，无编译错误。

- [x] **Step 6: 运行编译与格式检查**

Run:

```bash
cargo check -p textora-app
cargo fmt --all -- --check
git diff --check
```

Expected: 所有命令退出码为 `0`。

- [x] **Step 7: 提交修复**

```bash
git add crates/app/src/dispatch/tabs.rs docs/superpowers/plans/2026-07-18-last-file-default-document.md
git commit -m "fix(app): keep editable document after closing last file"
```
