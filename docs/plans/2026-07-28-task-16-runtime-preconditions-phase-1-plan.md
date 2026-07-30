# Task 16 Runtime 前置解耦第一阶段实施计划

> **For Codex:** REQUIRED SUB-SKILL: Use `subagent-driven-development` to implement this plan task-by-task, and use `verification-before-completion` before reporting the phase complete.

**目标：** 提前完成原 Task 17 的持久化边界，并把不含 textora 产品语义的叶子运行时类型迁入 `appkit-shell`，为后续迁移 `UiShell`、拆分 `Workspace` 和创建真实 `ShellRuntime` 消除反向依赖。

**架构：** `appkit-core` 接管稳定的 workspace 持久化 DTO 与路径注入式 store；`appkit-shell` 接管路由、通用编辑器插件、tab 会话状态、鼠标状态和 UI 容器等通用运行时类型；`textora-app` 通过临时语义重导出保持现有调用路径，后续阶段统一删除兼容层。每个实现任务最多修改 3 个文件，并在提交前完成格式化、相关测试、app 编译和架构检查。

**技术栈：** Rust、Cargo workspace、serde/TOML、winit、textora `core`/`ui`/`appkit-core`/`appkit-shell` crates。

**依据：**

- 总计划：`docs/plans/2026-07-26-appkit-architecture-split-plan.md`
- 前置设计：`docs/specs/2026-07-28-task-16-runtime-facade-preconditions-design.md`

---

## Task 1：迁移 workspace 持久化 DTO

**文件：**

- 修改：`crates/appkit-core/src/workspace/types.rs`
- 修改：`crates/app/src/workspace.rs`

### Step 1：记录迁移前兼容性基线

运行：

```bash
cargo test -p textora-app --lib workspace::tests::persisted_workspace
```

预期：现有 sidebar、snapshot filename 和旧格式兼容测试全部通过。

### Step 2：把 DTO 与格式测试移入 core

在 `appkit-core::workspace::types` 中新增公开的：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTab { /* 保留原字段顺序和全部 serde 属性 */ }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedWorkspace { /* 保留原字段顺序和 tabs rename */ }
```

其中 `original_disk_revision` 使用：

```rust
Option<crate::snapshot::PersistedDiskRevision>
```

逐字保留下列兼容约束：

- `suggested_file_name`、scroll、snapshot、legacy、plugin、preview 字段的 `#[serde(default)]`；
- `original_disk_revision` 与 `clean_untitled_content` 的 `skip_serializing_if = "Option::is_none"`；
- `PersistedWorkspace::entries` 的 `#[serde(rename = "tabs")]`；
- 所有字段保持公开，供 app 组装和恢复快照。

把 `persisted_workspace_roundtrip_with_sidebar_fields`、
`persisted_workspace_missing_sidebar_fields_default`、
`persisted_workspace_snapshot_filename_roundtrip` 和
`persisted_workspace_backward_compat_no_snapshot_fields` 移到 core。

再增加 `persisted_workspace_golden_toml_roundtrip_preserves_schema`，用固定 TOML
覆盖：

```toml
version = 1
active_index = 0
sidebar_pinned = true
sidebar_width = 280.0

[[tabs]]
file_path = "/tmp/test.txt"
cursor_offset = 5
selection_anchor = 2
dirty = true
snapshot_filename = "abc123.dirty"
unsaved_lines = ["legacy line"]
active_plugin = "editor"
```

断言反序列化字段值，重新序列化后仍使用 `[[tabs]]`，并可再次反序列化为相同
关键字段。

### Step 3：让 app 使用 core DTO

从 `workspace.rs` 删除 DTO 定义和仅为它们存在的 serde import，改为：

```rust
use appkit_core::workspace::types::{PersistedTab, PersistedWorkspace};
```

保留 workspace 的产品组装、保存和恢复行为测试。

### Step 4：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-core workspace::types
cargo test -p textora-app --lib workspace::tests::persisted_workspace
cargo check -p textora-app
bash scripts/check_architecture.sh
```

预期：全部通过，TOML 字段名与默认值不变。

提交：

```bash
git add crates/appkit-core/src/workspace/types.rs crates/app/src/workspace.rs
git commit -m "refactor(workspace): move persistence schema to core"
```

---

## Task 2：迁移路径注入式 WorkspaceStore

**文件：**

- 移动：`crates/app/src/workspace_store.rs` → `crates/appkit-core/src/workspace/store.rs`
- 修改：`crates/appkit-core/src/workspace/mod.rs`
- 修改：`crates/app/src/lib.rs`

### Step 1：记录迁移前 store 行为

运行：

```bash
cargo test -p textora-app --lib workspace_store::tests
```

预期：孤儿快照只清理注入目录的测试通过。

### Step 2：移动 store 并闭合 core 依赖

执行：

```bash
git mv crates/app/src/workspace_store.rs crates/appkit-core/src/workspace/store.rs
```

在新模块中：

- 使用 `crate::workspace::types::{PersistedTab, PersistedWorkspace}`；
- 使用 `crate::persistence::atomic_write`；
- 使用 `crate::snapshot::cleanup_orphans`；
- 将 `WorkspaceStore`、构造器和 app 已使用的方法设为 `pub`；
- 保留三个路径构造参数，不在 core 内推导 `~/.edit+`；
- 保留并迁移现有临时目录测试。

在 `workspace/mod.rs` 增加：

```rust
pub mod store;
```

在 app `lib.rs` 用临时语义重导出保持现有路径：

```rust
pub(crate) use appkit_core::workspace::store as workspace_store;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-core workspace::store
cargo check -p textora-app
bash scripts/check_architecture.sh
```

预期：store 测试在 core 通过，app 调用点无需批量改名。

提交：

```bash
git add crates/app/src/workspace_store.rs crates/appkit-core/src/workspace/store.rs crates/appkit-core/src/workspace/mod.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-core): move workspace store"
```

---

## Task 3：迁移 ViewRouteTable

**文件：**

- 移动：`crates/app/src/view_route.rs` → `crates/appkit-shell/src/view_route.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/lib.rs`

### Step 1：记录迁移前路由行为

运行：

```bash
cargo test -p textora-app --lib view_route::tests
```

预期：优先级、扩展名匹配、重复优先级和未知插件测试全部通过。

### Step 2：移动模块并公开 shell API

执行：

```bash
git mv crates/app/src/view_route.rs crates/appkit-shell/src/view_route.rs
```

把 `ViewPathMatcher`、`ViewRouteRule`、`ViewRouteError`、`ViewRouteTable` 及 app
使用的字段/方法从 `pub(crate)` 改为 `pub`。不改变匹配和排序算法。

在 shell `lib.rs` 增加：

```rust
pub mod view_route;
```

在 app `lib.rs` 增加临时语义重导出：

```rust
pub(crate) use appkit_shell::view_route;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell view_route::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/app/src/view_route.rs crates/appkit-shell/src/view_route.rs crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-shell): move view route table"
```

---

## Task 4：迁移通用 EditorPlugin

**文件：**

- 移动：`crates/app/src/plugins/editor.rs` → `crates/appkit-shell/src/editor_plugin.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/plugins/mod.rs`

### Step 1：记录迁移前插件行为

运行：

```bash
cargo test -p textora-app --lib plugins::editor::tests
```

预期：fallback editor factory 不抢占路径专用插件。

### Step 2：移动插件并保留 app 语义路径

执行：

```bash
git mv crates/app/src/plugins/editor.rs crates/appkit-shell/src/editor_plugin.rs
```

将 `EditorPlugin`、`EditorPluginFactory`、构造器和 app 使用字段设为 `pub`；
不改变 fallback factory、渲染占位和 render settings 消息行为。

在 shell `lib.rs` 增加：

```rust
pub mod editor_plugin;
```

在 `plugins/mod.rs` 用：

```rust
pub(crate) use appkit_shell::editor_plugin as editor;
```

替换原本的 `pub(crate) mod editor;`。

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell editor_plugin::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/app/src/plugins/editor.rs crates/appkit-shell/src/editor_plugin.rs crates/appkit-shell/src/lib.rs crates/app/src/plugins/mod.rs
git commit -m "refactor(appkit-shell): move fallback editor plugin"
```

---

## Task 5：迁移 MindmapStylePanelSession

**文件：**

- 移动：`crates/app/src/tab.rs` → `crates/appkit-shell/src/mindmap_style_panel.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/lib.rs`

### Step 1：记录迁移前状态机行为

运行：

```bash
cargo test -p textora-app --lib tab::tests
```

### Step 2：移动状态机并保留旧语义路径

执行：

```bash
git mv crates/app/src/tab.rs crates/appkit-shell/src/mindmap_style_panel.rs
```

将 enum 和现有方法设为 `pub`，不改变 `Closed` /
`Open { presets_expanded }` 互斥状态。

在 shell `lib.rs` 增加：

```rust
pub mod mindmap_style_panel;
```

在 app `lib.rs` 增加：

```rust
pub(crate) use appkit_shell::mindmap_style_panel as tab;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell mindmap_style_panel::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/app/src/tab.rs crates/appkit-shell/src/mindmap_style_panel.rs crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-shell): move mindmap panel session"
```

---

## Task 6：迁移 SmoothScroll

**文件：**

- 移动：`crates/app/src/smooth_scroll.rs` → `crates/appkit-shell/src/smooth_scroll.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/lib.rs`

### Step 1：补充并运行迁移特征测试

在原模块增加两个测试：

- `new_scroll_starts_at_rest`：断言 current/target 都是 `0.0` 且未动画；
- `tick_converges_and_snaps_to_target`：设置 target，循环 tick（设明确最大迭代
  常量防止死循环），断言最终 current 等于 target 且不再动画。

运行：

```bash
cargo test -p textora-app --lib smooth_scroll::tests
```

### Step 2：移动模块

执行：

```bash
git mv crates/app/src/smooth_scroll.rs crates/appkit-shell/src/smooth_scroll.rs
```

将 `SmoothScroll` 及其现有方法设为 `pub`。在 shell `lib.rs` 增加：

```rust
pub mod smooth_scroll;
```

在 app `lib.rs` 增加：

```rust
pub(crate) use appkit_shell::smooth_scroll;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell smooth_scroll::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/app/src/smooth_scroll.rs crates/appkit-shell/src/smooth_scroll.rs crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-shell): move smooth scroll state"
```

---

## Task 7：抽取 MouseState 与拖拽会话状态

**文件：**

- 新增：`crates/appkit-shell/src/mouse_state.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/mouse.rs`

### Step 1：记录迁移前状态行为

运行：

```bash
cargo test -p textora-app --lib mouse::tests::overlay_hover_needs_redraw
```

预期：首次、阈值内、越过阈值三个测试通过。

### Step 2：只抽取状态，不迁移产品命中测试算法

把以下类型和实现移到 `mouse_state.rs`：

- `CanvasDragEligibility`
- `CanvasDragSession`
- `MouseState`
- `MouseState::new`
- `MouseState::overlay_hover_needs_redraw`

将 hover redraw 阈值提取为语义化模块常量：

```rust
const HOVER_REDRAW_THRESHOLD_PX_SQUARED: f32 = 4.0;
```

类型和 app 使用的字段/方法设为 `pub`。把三个 hover redraw 测试移入 shell；
`mouse.rs` 保留文档 hit-test、selection 和输入处理算法，并通过：

```rust
pub(crate) use appkit_shell::mouse_state::{
    CanvasDragEligibility, CanvasDragSession, MouseState,
};
```

保持现有 `crate::mouse::*` 调用路径。

在 shell `lib.rs` 增加：

```rust
pub mod mouse_state;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell mouse_state::tests
cargo test -p textora-app --lib mouse::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/appkit-shell/src/mouse_state.rs crates/appkit-shell/src/lib.rs crates/app/src/mouse.rs
git commit -m "refactor(appkit-shell): extract mouse runtime state"
```

---

## Task 8：迁移 EditorHostWidget

**文件：**

- 移动：`crates/app/src/editor_host.rs` → `crates/appkit-shell/src/editor_host.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/lib.rs`

### Step 1：记录迁移前 widget 行为

运行：

```bash
cargo test -p textora-app --lib editor_host::tests
```

### Step 2：移动模块并保留 app 路径

执行：

```bash
git mv crates/app/src/editor_host.rs crates/appkit-shell/src/editor_host.rs
```

保持 `EditorHostWidget` 的 rect、paint、hit 和 event 行为不变。在 shell
`lib.rs` 增加：

```rust
pub mod editor_host;
```

在 app `lib.rs` 增加：

```rust
pub(crate) use appkit_shell::editor_host;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell editor_host::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/app/src/editor_host.rs crates/appkit-shell/src/editor_host.rs crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-shell): move editor host widget"
```

---

## Task 9：迁移 TabRuntime 与 TabRuntimeStore

**文件：**

- 移动：`crates/app/src/tab_runtime.rs` → `crates/appkit-shell/src/tab_runtime.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/lib.rs`

### Step 1：让特征测试脱离 DocumentView

在移动前把
`runtime_presentation_can_be_rebuilt_without_changing_document_model` 的文档构造
改为 `appkit_core::document::DocumentModel` + `core::buffer::TextBuffer`，仍断言：

- 文档全文保持 `"hello"`；
- 替换 presentation 后 visible rows 为 20；
- viewport height 为 240。

测试插件改用已经迁入 shell 的 `crate::editor_plugin::EditorPlugin`。

运行：

```bash
cargo test -p textora-app --lib tab_runtime::tests
```

### Step 2：移动 runtime store

执行：

```bash
git mv crates/app/src/tab_runtime.rs crates/appkit-shell/src/tab_runtime.rs
```

把本地依赖改为 shell 模块：

```rust
use crate::canvas_viewport::CanvasViewportSession;
use crate::document_presentation::DocumentPresentation;
use crate::mindmap_style_panel::MindmapStylePanelSession;
```

将 `TabRuntime`、`TabRuntimeStore`、app 使用字段和方法设为 `pub`。在 shell
`lib.rs` 增加：

```rust
pub mod tab_runtime;
```

在 app `lib.rs` 增加：

```rust
pub(crate) use appkit_shell::tab_runtime;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell tab_runtime::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/app/src/tab_runtime.rs crates/appkit-shell/src/tab_runtime.rs crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-shell): move tab runtime store"
```

---

## Task 10：迁移 TabSession 与 TabSessionMut

**文件：**

- 移动：`crates/app/src/tab_session.rs` → `crates/appkit-shell/src/tab_session.rs`
- 修改：`crates/appkit-shell/src/lib.rs`
- 修改：`crates/app/src/lib.rs`

### Step 1：让会话测试只使用 core model 和 shell runtime

在移动前给测试模块增加最小文档 helper：

```rust
fn document(text: &str) -> DocumentModel {
    let mut buffer =
        TextBuffer::new(false).expect("tab session test buffer must be constructible");
    buffer.write_raw(text.as_bytes());
    DocumentModel::new(buffer)
}
```

把 `DocumentView` 替换成该 helper，把 `EditorPlugin` 替换为
`crate::editor_plugin::EditorPlugin`。删除测试对旧 `DocumentView.presentation`
占位状态的断言，只保留 runtime presentation 是插件查询、advance cache 和
style panel 的唯一来源这一行为断言。

运行：

```bash
cargo test -p textora-app --lib tab_session::tests
```

### Step 2：移动会话模块

执行：

```bash
git mv crates/app/src/tab_session.rs crates/appkit-shell/src/tab_session.rs
```

将生产依赖改为 shell 内模块路径：

- `crate::cursor_motion`
- `crate::document_presentation`
- `crate::display_state`
- `crate::tab_runtime`
- `crate::canvas_viewport`
- `crate::display_line_map`
- `crate::snap_tree`
- `crate::mindmap_style_panel`

将 `TabSession`、`TabSessionMut`、app 使用字段和方法设为 `pub`；内部适配器
`PresentedDocument` / `PresentedDocumentMut` 与文本转换 helper 保持私有。

在 shell `lib.rs` 增加：

```rust
pub mod tab_session;
```

在 app `lib.rs` 增加：

```rust
pub(crate) use appkit_shell::tab_session;
```

### Step 3：验证并提交

运行：

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell tab_session::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

提交：

```bash
git add crates/app/src/tab_session.rs crates/appkit-shell/src/tab_session.rs crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-shell): move tab session facade"
```

---

## Task 11：第一阶段总体验证与下一阶段入口

**文件：** 无实现文件修改。

### Step 1：检查产品语义没有进入 shell

运行：

```bash
rg -n "textora_sync|TextoraSettings|NativeMenu|textora_markdown" crates/appkit-shell
```

预期：无匹配。

### Step 2：运行阶段验证

运行：

```bash
cargo fmt --all -- --check
bash scripts/check_architecture.sh
cargo check --workspace
cargo test -p textora-appkit-core
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
```

预期：全部通过。

### Step 3：审计兼容层和所有权

确认：

- `PersistedTab`、`PersistedWorkspace`、`WorkspaceStore` 只在 core 定义一次；
- 本计划列出的 shell 类型只在 shell 定义一次；
- app 只保留明确标记的临时语义重导出，没有复制实现；
- `appkit-core` 和 `appkit-shell` 的依赖树没有新增禁止依赖；
- worktree 除忽略的进度账本外保持干净。

### Step 4：生成第二阶段计划

基于实际迁移结果另写计划，范围只包括：

1. 清除 `UiShell` 的产品 downcast/action 并迁移 `UiShell`；
2. 设计 `PreparedTab` 边界，拆分 shell Workspace controller 与 app adapter；
3. 不在该计划中提前创建空壳 `ShellRuntime`。

第二阶段计划经审阅后再实施。原 Task 16 只有在
`ShellRuntime` 真正持有并驱动 runtime 状态后才可标记完成。
