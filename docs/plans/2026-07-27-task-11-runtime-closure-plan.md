# Task 11 Runtime Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 Task 11 的所有权收口，使 `WorkspaceModel` 只保存 `DocumentModel`，`TabRuntimeStore` 成为 `ViewPlugin`、`DocumentPresentation`、画布和 per-tab UI 会话的唯一所有者。

**Architecture:** 先引入借用型 `TabSession` / `TabSessionMut`，用 `TabId` 将 workspace model 与 runtime 临时组合，避免共享所有权或双写。随后分两阶段迁移：先迁插件 runtime，再迁 `DocumentPresentation`；最后删除 `DocItem`、store fallback 和所有按 index 跨调用关联。

**Tech Stack:** Rust 2024、`WorkspaceModel<DocumentModel>`、`HashMap<TabId, TabRuntime>`、现有 `ViewPlugin` 和 `DocumentPresentation`。

## Global Constraints

- 每个实现子任务最多修改 3 个文件；超出时继续拆分。
- 行为变化必须先写失败测试并观察预期失败。
- `WorkspaceModel` 与 `TabRuntimeStore` 仅通过 `TabId` 关联。
- 禁止 `Rc<RefCell<_>>`、全局 runtime、裸指针或 workspace/runtime 双写。
- `ui` 不得依赖 app 状态类型；本任务不改变用户可见行为或持久化格式。
- 每次提交前运行 `cargo fmt --all -- --check` 和 `cargo check -p textora-app`。
- 最终运行 `TEXTORA_ARCHITECTURE_MIGRATION=1 ./scripts/verify.sh`。

---

### Task 1: Define borrowed tab-session boundaries

**Files:**
- Create: `crates/app/src/tab_session.rs`
- Modify: `crates/app/src/tab_runtime.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Produces: `TabSession<'a>` and `TabSessionMut<'a>`.
- Consumes: `TabId`, `DocumentView`, `TabRuntime`.

- [ ] Add tests proving a session preserves its `TabId` and that mutable plugin changes affect the borrowed runtime only.
- [ ] Run `cargo test -p textora-app --lib tab_session` and verify failure before the API exists.
- [ ] Implement:

```rust
pub(crate) struct TabSession<'a> {
    pub(crate) id: TabId,
    pub(crate) document: &'a DocumentView,
    pub(crate) runtime: &'a TabRuntime,
}

pub(crate) struct TabSessionMut<'a> {
    pub(crate) id: TabId,
    pub(crate) document: &'a mut DocumentView,
    pub(crate) runtime: &'a mut TabRuntime,
}
```

- [ ] Keep constructors private; only App accessors may combine workspace and runtime.
- [ ] Run `cargo test -p textora-app --lib tab_session` and `cargo check -p textora-app`.
- [ ] Commit with `refactor(tabs): define borrowed tab sessions`.

### Task 2: Route App access through borrowed sessions

**Files:**
- Modify: `crates/app/src/app_tab.rs`
- Modify: `crates/app/src/app_tests.rs`
- Modify: `crates/app/src/tab_runtime.rs`

**Interfaces:**
- Produces: `App::tab_session(TabId)`, `tab_session_mut(TabId)`, `active_tab_session()` and `active_tab_session_mut()`.
- Preserves temporarily: store-first workspace fallback in `App::tab_runtime()` and `tab_runtime_mut()` until Task 6.

- [ ] Add a test proving `tab_session(id)` combines the document and the store-preferred runtime for exactly that ID.
- [ ] Run the focused test and verify it fails because session composition does not exist.
- [ ] Implement session composition from disjoint `workspace` / `tab_runtime_store` fields; keep the existing fallback only as a migration source.
- [ ] Run `cargo test -p textora-app --lib tab_runtime` and `cargo test -p textora-app --lib app_tab`.
- [ ] Commit with `refactor(tabs): make runtime store authoritative`.

### Task 3: Migrate plugin consumers to `TabSession`

Each row is an independent subtask and commit. Replace direct `workspace.*entry*.runtime` reads with `tab_session*` access while preserving the existing `DocItem` document/presentation helpers.

| Subtask | Files | Test command | Commit |
|---|---|---|---|
| 3A | `app_scroll.rs`, `dispatch/viewport.rs`, `dispatch/commands.rs` | `cargo test -p textora-app --lib app_scroll` | `refactor(tabs): route scrolling through tab sessions` |
| 3B | `app_dispatch.rs`, `dispatch/editor.rs`, `dispatch/wysiwyg.rs` | `cargo test -p textora-app --lib app_dispatch` | `refactor(tabs): route editing through tab sessions` |
| 3C | `app_renderer.rs`, `render_pipeline.rs`, `app_reshape.rs` | `cargo test -p textora-app --lib app_renderer` | `refactor(tabs): route rendering through tab sessions` |
| 3D | `dispatch/mouse.rs`, `events.rs`, `app_lifecycle.rs` | `cargo test -p textora-app --lib dispatch::mouse` | `refactor(tabs): route input through tab sessions` |
| 3E | `app_window.rs`, `commands.rs`, `app_tests.rs` | `cargo test -p textora-app --lib app_window` | `refactor(tabs): route window state through tab sessions` |

For every row:

- [ ] Add or adapt one test that inserts two runtimes and proves the operation touches only the active `TabId`.
- [ ] Run the row's test command and verify the test fails before the call-site migration.
- [ ] Migrate only the listed files.
- [ ] Run the row's test command and `cargo check -p textora-app`.
- [ ] Commit with the exact message in the table.

### Task 4: Make workspace runtime operations explicit

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/tab_session.rs`

**Interfaces:**
- Produces: runtime-aware snapshot, restore, view-toggle and plugin-creation operations keyed by `TabId`.
- Removes: workspace methods that discover plugin state through `DocItem.runtime`.

- [ ] Add tests proving snapshot and toggle operate on an explicitly supplied runtime for the same `TabId`.
- [ ] Verify failure while workspace still reads `DocItem.runtime`.
- [ ] Move plugin query/toggle logic to `TabSession` and pass runtime references explicitly at workspace composition boundaries.
- [ ] Run `cargo test -p textora-app --lib workspace` and `tab_session`.
- [ ] Commit with `refactor(workspace): make plugin runtime inputs explicit`.

### Task 5: Detach plugin runtime during tab creation

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`

**Interfaces:**
- Produces: `OpenedTab { effect: WorkspaceEffect, runtime: TabRuntime }`.
- Consumes: injected registry/routes from Task 11C.

- [ ] Add tests proving new, typed-new, file-open and external-content paths each return exactly one runtime with the same `TabId`.
- [ ] Verify the tests fail while runtime remains embedded in `DocItem`.
- [ ] Change creation APIs to return `OpenedTab`; insert its runtime in App before applying its `WorkspaceEffect`.
- [ ] Make `DocItem.runtime` an `Option<TabRuntime>` only as a transfer slot, and immediately `take()` it at the composition boundary; all consumers were migrated in Tasks 3–4.
- [ ] Run `cargo test -p textora-app --lib workspace` and `cargo test -p textora-app --lib dispatch::tabs`.
- [ ] Commit with `refactor(workspace): detach runtime when tabs open`.

### Task 6: Remove plugin runtime from `DocItem`

**Files:**
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/tab_runtime.rs`

**Interfaces:**
- Removes: `DocItem.runtime`, plugin-aware methods on `DocItem`.
- Produces: equivalent plugin-aware methods on `TabSession` / `TabSessionMut`.

- [ ] Add a source-boundary test asserting `DocItem` has no `runtime` field and workspace code contains no `.runtime`.
- [ ] Run it and verify failure.
- [ ] Move plugin query/message/render/toggle/canvas helpers from `DocItem` to borrowed sessions.
- [ ] Make snapshot/restore/switch-plugin accept the runtime store explicitly and use `TabId`.
- [ ] Run `cargo test -p textora-app --lib tab`, `workspace`, and `tab_runtime`.
- [ ] Commit with `refactor(workspace): remove plugin runtime from document entries`.

### Task 7: Move presentation into `TabRuntime`

**Files:**
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/src/tab_runtime.rs`
- Modify: `crates/app/src/tab_session.rs`

**Interfaces:**
- `TabRuntime::new(plugin, presentation)`.
- `DocumentView::into_parts() -> (DocumentModel, DocumentPresentation)`.
- `DocumentView::from_parts(DocumentModel, DocumentPresentation) -> DocumentView`.

- [ ] Add a test proving dropping/rebuilding a runtime presentation does not alter `DocumentModel`.
- [ ] Verify failure before `TabRuntime` owns presentation.
- [ ] Add explicit split/join helpers and move presentation ownership to runtime.
- [ ] Implement presentation/display/search helpers on borrowed sessions.
- [ ] Run `cargo test -p textora-app --lib document_view` and `tab_session`.
- [ ] Commit with `refactor(tabs): move presentation into tab runtime`.

### Task 8: Store `DocumentModel` directly and delete `DocItem`

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/app_tab.rs`

**Interfaces:**
- `Workspace { model: WorkspaceModel<DocumentModel>, ... }`.
- `WorkspaceEntry.suggested_file_name` is the only suggested-name owner.
- Removes: `DocItem`, runtime fallback, duplicate suggested-name storage.

- [ ] Add a boundary test asserting `WorkspaceModel<DocItem>` and `struct DocItem` are absent.
- [ ] Run it and verify failure.
- [ ] Move document title/path/dirty helpers to `WorkspaceEntry<DocumentModel>`-aware workspace methods.
- [ ] Delete `DocItem` and update App session construction.
- [ ] Run `cargo test -p textora-app --lib workspace`, `app_tab`, and `tab`.
- [ ] Commit with `refactor(workspace): store document models directly`.

### Task 9: Prove lifecycle bijection and remove migration compatibility

**Files:**
- Modify: `crates/app/src/app_tests.rs`
- Modify: `crates/app/src/tab_runtime.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`

**Interfaces:**
- Produces: `TabRuntimeStore::ids()` for invariant checks.
- Removes: obsolete transitional tests and fallback wording.

- [ ] Add tests covering new/open/restore/reorder/switch/single-close/batch-close/close-last and assert workspace IDs equal runtime-store IDs after every operation.
- [ ] Verify at least the restore test fails before final wiring.
- [ ] Add one invariant helper used only at lifecycle boundaries and tests.
- [ ] Run `cargo test -p textora-app --lib tab_runtime`, `workspace`, `app_tab`, and `app_dispatch`.
- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `TEXTORA_ARCHITECTURE_MIGRATION=1 ./scripts/verify.sh`.
- [ ] Commit with `refactor(workspace): enforce model runtime lifecycle bijection`.
