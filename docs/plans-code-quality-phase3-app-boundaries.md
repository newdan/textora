# Application Boundary Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Workspace 成为 active view/doc 与 tab 状态的唯一访问边界，并把 app action 的伴随副作用收敛到统一 effect 应用点。

**Architecture:** 先补齐只读/可变访问器并用编译器驱动机械迁移，再私有化字段；之后定义可组合 `AppEffect`，handler 只修改领域状态并返回 effect，顶层分发器统一完成 redraw、reshape、title 和 persistence。最后按动作域移动 handler，不同时修改行为。

**Tech Stack:** Rust、现有 `AppAction`/`WorkspaceEffect`、app 单元测试。

---

### Task 1: 固化 Workspace active access 契约

**Files:**
- Modify: `crates/app/src/workspace.rs:83-145`

- [ ] **Step 1: 增加空 workspace、普通 editor、Markdown view 的 accessor 测试**

```rust
#[test]
fn active_accessors_follow_active_index() {
    let mut ws = Workspace::new();
    assert!(ws.active_view().is_none());
    assert!(ws.active_doc().is_none());

    ws.new_empty_tab(600.0);
    ws.new_empty_tab(600.0);
    ws.switch_to(0);
    ws.active_doc_mut().unwrap().insert_at_cursor(b"a");
    assert_eq!(ws.active_doc().unwrap().buffer_len(), 1);
    ws.switch_to(1);
    assert_eq!(ws.active_index(), 1);
    assert_eq!(ws.active_doc().unwrap().buffer_len(), 0);
}
```

- [ ] **Step 2: 完整定义访问接口**

```rust
pub(crate) fn active_index(&self) -> usize;
pub(crate) fn active_view(&self) -> Option<&View>;
pub(crate) fn active_view_mut(&mut self) -> Option<&mut View>;
pub(crate) fn active_doc(&self) -> Option<&DocumentView>;
pub(crate) fn active_doc_mut(&mut self) -> Option<&mut DocumentView>;
pub(crate) fn view(&self, index: usize) -> Option<&View>;
pub(crate) fn view_mut(&mut self, index: usize) -> Option<&mut View>;
pub(crate) fn views(&self) -> &[View];
pub(crate) fn pinned_indices(&self) -> &HashSet<usize>;
```

实现均使用 `get/get_mut`；删除这些方法上的 `#[allow(dead_code)]`。

- [ ] **Step 3: 验证并提交接口**

```bash
cargo test -p edit-plus-app --lib workspace::tests::active_accessors_follow_active_index -- --exact
git add crates/app/src/workspace.rs
git commit -m "refactor(app): define workspace access boundary"
```

### Task 2: 分批消除 active doc 穿透访问

**Files:**
- Modify in batch 1: `crates/app/src/app_dispatch.rs`, `crates/app/src/app_renderer.rs`, `crates/app/src/app_window.rs`
- Modify in batch 2: `crates/app/src/app_tab.rs`, `crates/app/src/app_scroll.rs`, `crates/app/src/app_reshape.rs`
- Modify in batch 3: `crates/app/src/app_search.rs`, `crates/app/src/app_lifecycle.rs`, `crates/app/src/events.rs`
- Modify in batch 4: `crates/app/src/app_init.rs`, `crates/app/src/app_tests.rs`

- [ ] **Step 1: 机械替换三类模式**

```rust
self.workspace.views.get(self.workspace.active_index).map(|view| view.doc())
// becomes
self.workspace.active_doc()

self.workspace.views.get_mut(self.workspace.active_index).map(|view| view.doc_mut())
// becomes
self.workspace.active_doc_mut()

self.workspace.active_index
// becomes, outside workspace.rs
self.workspace.active_index()
```

索引非 active 的访问改用 `view(index)`/`view_mut(index)`；遍历改用 `views()`。不得改变 `Option` 分支、borrow 生命周期或动作顺序。

- [ ] **Step 2: 每批运行 app 测试并提交**

对上述四批依次执行以下明确提交：

```bash
cargo check -p edit-plus-app --all-targets
cargo test -p edit-plus-app --lib
git add crates/app/src/app_dispatch.rs crates/app/src/app_renderer.rs crates/app/src/app_window.rs
git commit -m "refactor(app): route active document access through workspace"
git add crates/app/src/app_tab.rs crates/app/src/app_scroll.rs crates/app/src/app_reshape.rs
git commit -m "refactor(app): route tab and scroll access through workspace"
git add crates/app/src/app_search.rs crates/app/src/app_lifecycle.rs crates/app/src/events.rs
git commit -m "refactor(app): route search and lifecycle access through workspace"
git add crates/app/src/app_init.rs crates/app/src/app_tests.rs
git commit -m "refactor(app): migrate workspace setup and tests"
```

提交前确认每批最多 3 个文件；第 4 批仅 2 个文件。

- [ ] **Step 3: 私有化字段并用 rg 验收**

在 `Workspace` 中把 `views`、`active_index`、`pinned_indices` 改为私有字段。Run:

```bash
rg -n "workspace\.(views|active_index|pinned_indices)" crates/app/src -g '*.rs' -g '!workspace.rs'
cargo check -p edit-plus-app --all-targets
```

Expected: `rg` 无输出；check PASS。

- [ ] **Step 4: 提交字段私有化**

```bash
git add crates/app/src/workspace.rs
git commit -m "refactor(app): make workspace storage private"
```

### Task 3: 定义统一可组合 AppEffect

**Files:**
- Create: `crates/app/src/app_effect.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 1: 先写合并语义测试**

```rust
#[test]
fn merge_preserves_all_requested_side_effects() {
    let effect = AppEffect::REDRAW
        .merge(AppEffect::RESHAPE)
        .merge(AppEffect::UPDATE_TITLE);
    assert!(effect.redraw);
    assert!(effect.reshape);
    assert!(effect.update_title);
    assert!(!effect.persist_workspace);
}
```

- [ ] **Step 2: 实现 effect 数据结构**

```rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppEffect {
    pub(crate) redraw: bool,
    pub(crate) reshape: bool,
    pub(crate) update_title: bool,
    pub(crate) persist_workspace: bool,
}

impl AppEffect {
    pub(crate) const NONE: Self = Self { redraw: false, reshape: false, update_title: false, persist_workspace: false };
    pub(crate) const REDRAW: Self = Self { redraw: true, ..Self::NONE };
    pub(crate) const RESHAPE: Self = Self { redraw: true, reshape: true, ..Self::NONE };
    pub(crate) const UPDATE_TITLE: Self = Self { redraw: true, update_title: true, ..Self::NONE };

    pub(crate) const fn merge(self, other: Self) -> Self {
        Self {
            redraw: self.redraw || other.redraw,
            reshape: self.reshape || other.reshape,
            update_title: self.update_title || other.update_title,
            persist_workspace: self.persist_workspace || other.persist_workspace,
        }
    }
}
```

- [ ] **Step 3: 在 App 实现唯一 apply_effect**

```rust
pub(crate) fn apply_effect(&mut self, effect: AppEffect) {
    if effect.reshape { self.invalidate_reshape(); }
    if effect.update_title { self.update_window_title(); }
    if effect.persist_workspace { self.persist_workspace_state(); }
    if effect.redraw {
        self.needs_redraw = true;
        if let Some(window) = &self.window { window.request_redraw(); }
    }
}
```

`update_window_title` 复用 `app_tab.rs` 的现有方法；将现有 `save_workspace_snapshot` 重命名为 `persist_workspace_state`，保留原方法体和调用顺序后由 `apply_effect` 调用。

- [ ] **Step 4: 验证并提交**

```bash
cargo test -p edit-plus-app --lib app_effect::tests::
git add crates/app/src/app_effect.rs crates/app/src/lib.rs crates/app/src/app.rs
git commit -m "refactor(app): centralize application side effects"
```

### Task 4: 按动作域拆分 dispatch handler

**Files:**
- Create: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 第一批只移动 AppCommand 分支**

`lib.rs` 注册：

```rust
pub(crate) mod dispatch {
    pub(crate) mod commands;
}
```

`commands.rs` 实现：

```rust
impl App {
    pub(crate) fn dispatch_app_command(
        &mut self,
        command: AppCommand,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect;
}
```

从 `app_dispatch.rs::execute_commands` 移动原 `AppCommand` match arm 的领域修改；把 `needs_redraw`/reshape/title/persist 写入改成返回 `AppEffect`。顶层循环合并 effect 后只调用一次 `apply_effect`。

- [ ] **Step 2: 验证行为不变并提交新模块**

```bash
cargo test -p edit-plus-app --lib app_dispatch
cargo test -p edit-plus-app --lib commands
git add crates/app/src/dispatch/commands.rs crates/app/src/app_dispatch.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract command dispatch domain"
```

### Task 5: 提取 editor dispatch

**Files:**
- Create: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 移动 EditCommand 分支**

```rust
impl App {
    pub(crate) fn dispatch_edit_command(
        &mut self,
        command: EditCommand,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect;
}
```

保留原编辑顺序和 `reset_after_edit` 调用；编辑成功返回 `RESHAPE.merge(UPDATE_TITLE)`，无动作返回 `NONE`。`lib.rs::dispatch` 增加 `pub(crate) mod editor;`。

- [ ] **Step 2: 验证并提交**

```bash
cargo test -p edit-plus-app --lib commands
cargo test -p edit-plus-app --lib document_view
git add crates/app/src/dispatch/editor.rs crates/app/src/app_dispatch.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract editor dispatch domain"
```

### Task 6: 提取 tab dispatch

**Files:**
- Create: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 移动 SwitchTab/CloseTab/NewEmptyTab/TogglePin/context-menu 分支**

定义领域动作并使用统一入口：

```rust
pub(crate) enum TabDispatchAction {
    Switch(usize),
    Close(usize),
    NewEmpty,
    TogglePin,
    Context(ContextMenuAction, usize),
}

pub(crate) fn dispatch_tab_action(&mut self, action: TabDispatchAction) -> AppEffect;
```

顶层 `AppAction` match 将 tab variants 映射到该 enum。把 `WorkspaceEffect` 映射为 `AppEffect`：`ActiveTabChanged` → reshape/title/persist/redraw，`LayoutChanged` → persist/redraw，`None` → none。

- [ ] **Step 2: 验证并提交**

```bash
cargo test -p edit-plus-app --lib workspace::tests::
cargo test -p edit-plus-app --lib app_tab
git add crates/app/src/dispatch/tabs.rs crates/app/src/app_dispatch.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract tab dispatch domain"
```

### Task 7: 提取 search dispatch

**Files:**
- Create: `crates/app/src/dispatch/search.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 移动 ToggleFind/SearchBarAction 与 replace 分支**

```rust
pub(crate) enum SearchDispatchAction {
    ToggleFind,
    Widget(SearchBarAction),
}

pub(crate) fn dispatch_search_action(&mut self, action: SearchDispatchAction) -> AppEffect;
```

搜索状态 mutation 只通过 `workspace.active_doc_mut()`；替换 backend error 进入现有可见错误文本并返回 REDRAW，不能只写窗口标题或静默无动作。

- [ ] **Step 2: 验证并提交**

```bash
cargo test -p edit-plus-app --lib app_search
cargo test -p edit-plus-core --lib buffer::text_buffer::tests::regex_replace
git add crates/app/src/dispatch/search.rs crates/app/src/app_dispatch.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract search dispatch domain"
```

### Task 8: 提取 pointer/scroll dispatch

**Files:**
- Create: `crates/app/src/dispatch/pointer.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 移动 mouse、scrollbar 与 viewport 分支**

```rust
pub(crate) enum PointerDispatchAction {
    UpdateMousePos(f64, f64),
    Scroll(MouseScrollDelta),
    EditorInput { state: ElementState, px: f32, py: f32, hit: Option<(usize, usize, usize)> },
    EditorMoved { px: f32, py: f32, hit: Option<(usize, usize, usize)> },
    Scrollbar(ScrollbarAction),
    UpdateScrollTop(f64),
    ScrollViewportBy(f64),
}

pub(crate) fn dispatch_pointer_action(&mut self, action: PointerDispatchAction) -> AppEffect;
```

处理 `UpdateMousePos/HandleScroll/EditorMouseInput/EditorCursorMoved/ScrollbarAction/UpdateScrollTop/ScrollViewportBy`，保留 hit-test 结果，不在 handler 内直接 `request_redraw`。

- [ ] **Step 2: 验证顶层分发器只做路由与 effect 应用**

```bash
rg -n "needs_redraw = true|request_redraw\(|invalidate_reshape\(" crates/app/src/dispatch
cargo test -p edit-plus-app --lib mouse
cargo test -p edit-plus-app --lib app_scroll
```

Expected: `rg` 无输出；测试 PASS。

- [ ] **Step 3: 提交**

```bash
git add crates/app/src/dispatch/pointer.rs crates/app/src/app_dispatch.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract pointer dispatch domain"
```

### Task 9: 缩小 app crate 公共 API

**Files:**
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/main.rs`

- [ ] **Step 1: 将非外部 API module 改为 pub(crate)**

保留公开项仅为：

```rust
pub use app::App;
pub use app_event::AppEvent;
pub use gpu::{GpuError, headless_init};
```

bench 确需的 `document_view`/`display_line_map` 暂时保持 `pub`，并在注释注明仅为 benchmark API；其他 `pub mod` 改为 `pub(crate) mod`。binary 使用 re-export，不依赖内部 module 路径。

- [ ] **Step 2: 验证 lib、bin、tests、benches**

```bash
cargo check -p edit-plus-app --all-targets
cargo test -p edit-plus-app --lib
```

Expected: PASS；没有通过重新扩大 module 可见性来绕过编译错误。

- [ ] **Step 3: 提交**

```bash
git add crates/app/src/lib.rs crates/app/src/main.rs
git commit -m "refactor(app): narrow crate public API"
```
