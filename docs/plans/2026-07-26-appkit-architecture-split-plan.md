# textora appkit Architecture Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将现有 `textora-app` 拆成无窗口 `textora-appkit-core`、通用运行时 `textora-appkit-shell` 和精简后的 textora 产品层，同时保持唯一 `textora` binary 与全部现有行为不变。

**Architecture:** 先在原 crate 内建立 `ProductPaths`、`TabId`、模型/展示和产品/运行时边界，再移动已经解耦的模块。`appkit-core` 不接触 UI/窗口/渲染类型；`appkit-shell` 持有插件、展示状态与事件机制；`app` 注入插件、路由和 textora 产品服务。

**Tech Stack:** Rust 2024、Cargo workspace、winit 0.30、wgpu、现有 `textora-core` / `textora-ui` / `textora-markdown` / `textora-sync` crates。

## Global Constraints

- 全程保持只生成 `textora` 一个 binary；不创建笔记 App。
- `appkit-core` 禁止依赖 `ui`、`winit`、`wgpu`、`render`、`shaping`、`textora-markdown`、`textora-sync`。
- `appkit-shell` 禁止依赖 `textora-markdown`、`textora-sync`、`textora-app`。
- `ui` 中不得保留 `SyncSettingsAction`、sync 页面或 `textora_sync` 语义。
- 不改变 `~/.edit+` 下现有设置、workspace、history、pinned paths 和 dirty snapshot 的兼容格式。
- 每个实现子任务最多修改 3 个文件；超过 3 个文件必须继续拆分。
- 所有行为变更先写失败测试；纯路径搬迁前后运行同一组测试。
- 每次提交前必须 `cargo fmt --all -- --check` 且相关 crate 编译通过。
- P0–P4 每阶段结束运行相关测试；P5 必须运行 `./scripts/verify.sh`。
- 当前主工作区已有无关未提交修改；执行前必须使用 `using-git-worktrees` 创建隔离 worktree，并确保本设计与本计划已进入执行分支。

**Design spec:** `docs/specs/2026-07-26-note-app-architecture-design.md`

---

## File Structure

### 新 crate

```text
crates/appkit-core/
  Cargo.toml
  src/
    lib.rs
    content_hash.rs
    document/
      mod.rs
      cursor.rs
      model.rs
    edit.rs
    external_document_change.rs
    file_history.rs
    file_safety.rs
    line_index.rs
    navigator.rs
    persistence.rs
    snapshot.rs
    workspace/
      mod.rs
      store.rs
      types.rs

crates/appkit-shell/
  Cargo.toml
  src/
    lib.rs
    event.rs
    product_host.rs
    runtime.rs
    tab_runtime.rs
    view_route.rs
    document_presentation.rs
    input_mapper.rs
    ...现有窗口、渲染、reshape、dispatch 模块
```

### textora 产品层

```text
crates/app/src/
  product_paths.rs
  textora_product.rs
  sync_settings_page.rs
  sync_settings_types.rs
  main.rs
  native_menu.rs
  macos_open_documents.rs
  sync_*.rs
```

`crates/app/src/app.rs` 在迁移期保留为兼容 facade，P4 最后收敛为：

```rust
pub struct App {
    shell: appkit_shell::ShellRuntime,
    product: TextoraProduct,
}
```

---

### Task 1: Scaffold `textora-appkit-core`

**Files:**
- Create: `crates/appkit-core/Cargo.toml`
- Create: `crates/appkit-core/src/lib.rs`
- Modify: `crates/app/Cargo.toml`

**Interfaces:**
- Produces: crate `appkit_core`
- Consumes: `textora-core` and serialization/file dependencies only

- [ ] **Step 1: Add the app dependency first**

Add to `crates/app/Cargo.toml`:

```toml
appkit-core = { path = "../appkit-core", package = "textora-appkit-core" }
```

- [ ] **Step 2: Verify the missing crate fails**

Run:

```bash
cargo check -p textora-app
```

Expected: FAIL because `../appkit-core/Cargo.toml` does not exist.

- [ ] **Step 3: Create the minimal crate**

`crates/appkit-core/Cargo.toml`:

```toml
[package]
name = "textora-appkit-core"
version = "0.1.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[lib]
name = "appkit_core"
path = "src/lib.rs"

[dependencies]
core.workspace = true
serde.workspace = true
toml.workspace = true
similar = "2"
blake3 = "1"
dirs = "6"
notify = "8"

[dev-dependencies]
tempfile = "3"
```

`crates/appkit-core/src/lib.rs`:

```rust
//! Headless application model and persistence for textora-based products.

#![forbid(unsafe_code)]
```

- [ ] **Step 4: Verify the scaffold**

Run:

```bash
cargo check -p textora-appkit-core
cargo check -p textora-app
```

Expected: both commands PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/appkit-core crates/app/Cargo.toml
git commit -m "refactor(appkit): scaffold headless core crate"
```

---

### Task 2: Scaffold `textora-appkit-shell`

**Files:**
- Create: `crates/appkit-shell/Cargo.toml`
- Create: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/Cargo.toml`

**Interfaces:**
- Produces: crate `appkit_shell`
- Consumes: `appkit_core`, `ui`, rendering and window dependencies

- [ ] **Step 1: Add the app dependency and verify failure**

Add:

```toml
appkit-shell = { path = "../appkit-shell", package = "textora-appkit-shell" }
```

Run `cargo check -p textora-app`.

Expected: FAIL because the shell manifest does not exist.

- [ ] **Step 2: Create the shell manifest**

```toml
[package]
name = "textora-appkit-shell"
version = "0.1.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[lib]
name = "appkit_shell"
path = "src/lib.rs"

[dependencies]
appkit-core = { path = "../appkit-core", package = "textora-appkit-core" }
core.workspace = true
ui.workspace = true
winit.workspace = true
wgpu.workspace = true
render.workspace = true
shaping.workspace = true
bytemuck = { version = "1", features = ["derive"] }
pollster = "0.4"
hashlink.workspace = true
smallvec.workspace = true
unicode_categories.workspace = true
```

`crates/appkit-shell/src/lib.rs`:

```rust
//! Window, input, plugin-session, and rendering runtime.
```

- [ ] **Step 3: Verify**

Run:

```bash
cargo check -p textora-appkit-shell
cargo check -p textora-app
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/appkit-shell crates/app/Cargo.toml
git commit -m "refactor(appkit): scaffold shell crate"
```

---

### Task 3: Enforce dependency boundaries automatically

**Files:**
- Create: `scripts/check_architecture.sh`
- Modify: `scripts/verify.sh`

**Interfaces:**
- Produces: executable boundary check used by full verification

- [ ] **Step 1: Write the boundary script**

The script must run `cargo tree --prefix none` for each new crate and reject exact forbidden package names:

```bash
#!/usr/bin/env bash
set -euo pipefail

check_forbidden_dependency() {
  local package_name="$1"
  shift
  local dependency_tree
  dependency_tree="$(cargo tree -p "$package_name" --prefix none)"
  for forbidden_name in "$@"; do
    if printf '%s\n' "$dependency_tree" | grep -Eq "^${forbidden_name}( |$)"; then
      echo "${package_name} must not depend on ${forbidden_name}" >&2
      exit 1
    fi
  done
}

check_forbidden_dependency textora-appkit-core \
  textora-ui winit wgpu textora-render textora-shaping textora-markdown textora-sync
check_forbidden_dependency textora-appkit-shell \
  textora-markdown textora-sync textora-app

if rg -n '\\.edit\\+' crates/appkit-core crates/appkit-shell; then
  echo "shared crates must not hardcode .edit+" >&2
  exit 1
fi

if rg -n 'SyncSettings|textora_sync' crates/ui; then
  echo "ui must not contain textora sync product types" >&2
  exit 1
fi
```

- [ ] **Step 2: Verify it initially fails for the known UI sync coupling**

Run `bash scripts/check_architecture.sh`.

Expected: FAIL and print matches from `crates/ui/src/widgets/settings_view`.

- [ ] **Step 3: Add a transitional switch without weakening the final rule**

Until Task 11 removes the UI coupling, invoke the script from `verify.sh` only when:

```bash
if [[ -z "${TEXTORA_ARCHITECTURE_MIGRATION:-}" ]]; then
  bash scripts/check_architecture.sh
fi
```

During P0–P3 use `TEXTORA_ARCHITECTURE_MIGRATION=1 ./scripts/verify.sh`. Task 12 removes this conditional and makes the boundary check unconditional.

- [ ] **Step 4: Verify existing checks**

Run:

```bash
TEXTORA_ARCHITECTURE_MIGRATION=1 bash scripts/verify.sh
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/check_architecture.sh scripts/verify.sh
git commit -m "test(architecture): enforce appkit dependency boundaries"
```

---

### Task 4: Define product-owned persistence paths

**Files:**
- Create: `crates/app/src/product_paths.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Produces: `ProductPaths::textora(home_dir: &Path) -> ProductPaths`

- [ ] **Step 1: Write the path compatibility test**

Add inside `product_paths.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::ProductPaths;
    use std::path::Path;

    #[test]
    fn textora_paths_preserve_existing_layout() {
        let paths = ProductPaths::textora(Path::new("/home/user"));
        let root = Path::new("/home/user/.edit+");
        assert_eq!(paths.config_dir, root);
        assert_eq!(paths.settings_file, root.join("settings.toml"));
        assert_eq!(paths.workspace_file, root.join("workspace.toml"));
        assert_eq!(paths.pinned_paths_file, root.join("pinned_paths.json"));
        assert_eq!(paths.snapshots_dir, root.join("snapshots"));
        assert_eq!(paths.history_file, root.join("history.toml"));
    }
}
```

- [ ] **Step 2: Run the test and verify failure**

Run:

```bash
cargo test -p textora-app --lib product_paths::tests::textora_paths_preserve_existing_layout
```

Expected: FAIL because the module/type does not exist.

- [ ] **Step 3: Implement `ProductPaths`**

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductPaths {
    pub config_dir: PathBuf,
    pub theme_dir: PathBuf,
    pub workspace_file: PathBuf,
    pub pinned_paths_file: PathBuf,
    pub snapshots_dir: PathBuf,
    pub history_file: PathBuf,
    pub settings_file: PathBuf,
}

impl ProductPaths {
    pub fn textora(home_dir: &Path) -> Self {
        let config_dir = home_dir.join(".edit+");
        Self {
            theme_dir: config_dir.join("themes"),
            workspace_file: config_dir.join("workspace.toml"),
            pinned_paths_file: config_dir.join("pinned_paths.json"),
            snapshots_dir: config_dir.join("snapshots"),
            history_file: config_dir.join("history.toml"),
            settings_file: config_dir.join("settings.toml"),
            config_dir,
        }
    }
}
```

Register `mod product_paths;` in `lib.rs`.

- [ ] **Step 4: Verify and commit**

Run the targeted test and `cargo check -p textora-app`; both must PASS.

```bash
git add crates/app/src/product_paths.rs crates/app/src/lib.rs
git commit -m "refactor(app): centralize product paths"
```

---

### Task 5: Inject `ProductPaths` into application construction

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_init.rs`

**Interfaces:**
- Consumes: `ProductPaths`
- Produces: `App::paths: ProductPaths`

- [ ] **Step 1: Add a constructor test in `app_init.rs`**

Assert that a headless/default `App` constructed with a temporary home retains the supplied `ProductPaths` and passes its `config_dir` to `WorkspaceStore` and `FileHistory`.

- [ ] **Step 2: Run the targeted test**

Expected: FAIL because `App` has no `paths` field.

- [ ] **Step 3: Add the field and single construction point**

Add:

```rust
pub(crate) paths: crate::product_paths::ProductPaths,
```

Construct it once:

```rust
let home_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
let paths = ProductPaths::textora(&home_dir);
```

Replace the local `config_dir` construction with `paths.config_dir.clone()` and store `paths` in `App`.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test -p textora-app --lib app_init
cargo check -p textora-app
```

Expected: PASS.

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs
git commit -m "refactor(app): inject product paths"
```

---

### Task 6: Remove remaining shared-path reconstruction

This task is split into three independently compiled subtasks.

#### Task 6A: Workspace and snapshot paths

**Files:**
- Modify: `crates/app/src/workspace_store.rs`
- Modify: `crates/app/src/dirty_snapshot.rs`
- Modify: `crates/app/src/app_tab.rs`

- [ ] Change `WorkspaceStore::new` to consume explicit workspace, pinned, and snapshot paths.
- [ ] Change dirty snapshot APIs from `snapshots_dir()` to an explicit `&Path`.
- [ ] Pass `self.paths.snapshots_dir` from `App`; remove `App::config_dir()`.
- [ ] Add a test proving cleanup only touches the injected snapshot directory.
- [ ] Run `cargo test -p textora-app --lib workspace_store`, `cargo test -p textora-app --lib dirty_snapshot`, and `cargo test -p textora-app --lib app_tab`.
- [ ] Commit with `refactor(persistence): inject workspace snapshot paths`.

#### Task 6B: Settings and theme paths

**Files:**
- Modify: `crates/app/src/settings_io.rs`
- Modify: `crates/app/src/theme_loader.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`

- [ ] Make settings load/save functions accept `&Path`.
- [ ] Make custom theme loading accept the injected theme directory.
- [ ] Replace direct `~/.edit+/settings.toml` construction with `self.paths.settings_file`.
- [ ] Add compatibility tests using temporary directories.
- [ ] Run `cargo test -p textora-app --lib settings_io`, `cargo test -p textora-app --lib theme_loader`, and `cargo test -p textora-app --lib dispatch::tabs`.
- [ ] Commit with `refactor(settings): inject product file paths`.

#### Task 6C: History path

**Files:**
- Modify: `crates/app/src/file_history.rs`
- Modify: `crates/app/src/app_lifecycle.rs`

- [ ] Change `FileHistory::load/save` to consume the exact history file path.
- [ ] Use `self.paths.history_file` in lifecycle persistence.
- [ ] Preserve the current TOML format with a round-trip test.
- [ ] Run `cargo test -p textora-app --lib file_history` and `cargo test -p textora-app --lib app_lifecycle`.
- [ ] Commit with `refactor(history): use injected history path`.

---

### Task 7: Move headless leaf modules into `appkit-core`

Each row is a separate subtask and commit. A row modifies exactly the moved source file plus the two crate roots:

| Subtask | Move | Add to `appkit-core/src/lib.rs` | Compatibility re-export in `app/src/lib.rs` | Test command |
|---|---|---|---|---|
| 7A | `content_hash.rs` → `appkit-core/src/content_hash.rs` | `pub mod content_hash;` | `pub(crate) use appkit_core::content_hash;` | `cargo test -p textora-appkit-core content_hash` |
| 7B | `line_index.rs` → `appkit-core/src/line_index.rs` | `pub mod line_index;` | `pub(crate) use appkit_core::line_index;` | `cargo test -p textora-appkit-core line_index` |
| 7C | `navigator.rs` → `appkit-core/src/navigator.rs` | `pub mod navigator;` | `pub(crate) use appkit_core::navigator;` | `cargo test -p textora-appkit-core navigator` |
| 7D | `persistence.rs` → `appkit-core/src/persistence.rs` | `pub mod persistence;` | `pub(crate) use appkit_core::persistence;` | `cargo test -p textora-appkit-core persistence` |
| 7E | `external_document_change.rs` → `appkit-core/src/external_document_change.rs` | `pub mod external_document_change;` | matching re-export | `cargo test -p textora-appkit-core external_document_change` |

For every row:

- [ ] Run the test before the move and record the passing count.
- [ ] Use `git mv` for the file.
- [ ] Replace `pub(crate)` with `pub` only for symbols consumed across the crate boundary; keep helpers private.
- [ ] Run the listed core test, then `cargo check -p textora-app`.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Commit 7A–7E respectively as `refactor(appkit-core): move content hash`, `refactor(appkit-core): move line index`, `refactor(appkit-core): move navigator`, `refactor(appkit-core): move persistence`, and `refactor(appkit-core): move external change classification`.

After 7E, run:

```bash
cargo test -p textora-appkit-core
cargo test -p textora-app --lib
```

Expected: PASS with the same behavior tests as before migration.

---

### Task 8: Move the file-safety persistence cluster

#### Task 8A: Merge conflict-copy policy into file safety

**Files:**
- Modify: `crates/app/src/file_safety.rs`
- Delete: `crates/app/src/conflict_copy.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Move `create_conflict_copy` and its tests into `file_safety.rs`.
- [ ] Update internal calls to use the local function.
- [ ] Delete the old module registration.
- [ ] Run `cargo test -p textora-app --lib file_safety`.
- [ ] Commit with `refactor(file-safety): colocate conflict preservation`.

#### Task 8B: Move file safety

**Files:**
- Move: `crates/app/src/file_safety.rs` → `crates/appkit-core/src/file_safety.rs`
- Modify: `crates/appkit-core/src/lib.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Export `file_safety` from core and re-export it temporarily from app.
- [ ] Run `cargo test -p textora-appkit-core file_safety`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(appkit-core): move file safety`.

#### Task 8C: Move dirty snapshots

**Files:**
- Move: `crates/app/src/dirty_snapshot.rs` → `crates/appkit-core/src/snapshot.rs`
- Modify: `crates/appkit-core/src/lib.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Rename public entry points to the `snapshot` module path.
- [ ] Keep injected `snapshots_dir: &Path` parameters; no environment reads are allowed.
- [ ] Run `cargo test -p textora-appkit-core snapshot`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(appkit-core): move dirty snapshots`.

#### Task 8D: Move file history

**Files:**
- Move: `crates/app/src/file_history.rs` → `crates/appkit-core/src/file_history.rs`
- Modify: `crates/appkit-core/src/lib.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Export and re-export the module.
- [ ] Run `cargo test -p textora-appkit-core file_history`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(appkit-core): move file history`.

---

### Task 9: Introduce stable `TabId` and pure workspace DTOs

#### Task 9A: Define `TabId`

**Files:**
- Create: `crates/appkit-core/src/workspace/types.rs`
- Create: `crates/appkit-core/src/workspace/mod.rs`
- Modify: `crates/appkit-core/src/lib.rs`

**Produces:**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabId(u64);

#[derive(Debug, Default)]
pub struct TabIdAllocator {
    next_raw_id: u64,
}
```

Tests must prove IDs are non-zero, monotonically increasing, unequal after close/reopen, and survive tab reordering because the ID is stored on the tab rather than derived from index.

- [ ] Write the tests.
- [ ] Verify failure.
- [ ] Implement the types without a global atomic.
- [ ] Run `cargo test -p textora-appkit-core workspace::types`.
- [ ] Commit with `refactor(workspace): add stable tab identity`.

#### Task 9B: Attach IDs to document tabs

**Files:**
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/workspace.rs`

- [ ] Add `id: TabId` to `DocItem`.
- [ ] Make all constructors receive an allocated ID.
- [ ] Add `Workspace::tab_id_at(index)` and `Workspace::index_of(TabId)`.
- [ ] Add tests for close, close-others, reorder-equivalent restore, and navigation history.
- [ ] Run `cargo test -p textora-app --lib workspace`.
- [ ] Commit with `refactor(workspace): identify tabs independently of indices`.

#### Task 9C: Convert cross-call actions to `TabId`

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/dispatch/tabs.rs`

Change action payloads that can outlive a single hit-test call:

```rust
SwitchTab(TabId),
CloseTab(TabId),
ExecuteContextMenuAction(ContextMenuAction, TabId),
HoverTab(Option<TabId>),
```

Widget hit results remain indexes; `events.rs` converts index to `TabId` immediately before creating `AppAction`.

- [ ] Add a regression test that queues a close action, mutates tab order, then closes the originally targeted ID.
- [ ] Run the test and verify the old index action closes the wrong tab.
- [ ] Implement the ID conversion.
- [ ] Run `cargo test -p textora-app --lib dispatch::tabs` and `cargo test -p textora-app --lib events`.
- [ ] Commit with `refactor(tabs): route durable actions by tab id`.

#### Task 9D: Introduce the generic workspace model seam

**Files:**
- Create: `crates/appkit-core/src/workspace/model.rs`
- Modify: `crates/appkit-core/src/workspace/mod.rs`
- Modify: `crates/app/src/workspace.rs`

Define a headless container that does not know the concrete document/runtime type:

```rust
pub struct WorkspaceEntry<T> {
    pub id: TabId,
    pub value: T,
    pub suggested_file_name: Option<String>,
}

pub struct WorkspaceModel<T> {
    entries: Vec<WorkspaceEntry<T>>,
    active_id: Option<TabId>,
    pinned_ids: HashSet<TabId>,
    back_history: Vec<TabId>,
    forward_history: Vec<TabId>,
    id_allocator: TabIdAllocator,
}
```

- [ ] Port tab ordering, active selection, pinning and navigation-history tests to `appkit-core`.
- [ ] Verify the tests fail before the model exists.
- [ ] Make the existing app `Workspace` delegate those operations to `WorkspaceModel<DocItem>`.
- [ ] Keep file/plugin operations in the app facade until Tasks 10–11 change the entry value to `DocumentModel`.
- [ ] Run `cargo test -p textora-appkit-core workspace::model` and `cargo test -p textora-app --lib workspace`.
- [ ] Commit with `refactor(workspace): introduce headless workspace model`.

---

### Task 10: Separate `DocumentModel` from presentation state

#### Task 10A: Move pure cursor state

**Files:**
- Create: `crates/appkit-core/src/document/cursor.rs`
- Create: `crates/appkit-core/src/document/mod.rs`
- Modify: `crates/appkit-core/src/lib.rs`

Move only byte/selection state and model navigation data. Pixel positions, sticky X and cursor blink remain in shell.

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorState {
    pub byte_offset: usize,
    pub selection_anchor: Option<usize>,
}
```

- [ ] Port current cursor-state tests that do not construct `DisplayState`.
- [ ] Verify core tests pass.
- [ ] Commit with `refactor(document): extract pure cursor state`.

#### Task 10B: Define `DocumentModel`

**Files:**
- Create: `crates/appkit-core/src/document/model.rs`
- Modify: `crates/appkit-core/src/document/mod.rs`
- Modify: `crates/app/src/document_view/mod.rs`

`DocumentModel` owns:

```rust
pub struct DocumentModel {
    pub text_buffer: core::buffer::TextBuffer,
    pub line_index: crate::line_index::LineIndex,
    pub file_path: Option<PathBuf>,
    pub disk_revision: Option<crate::file_safety::DiskRevision>,
    pub content_revision: u64,
    pub cursor: CursorState,
    pub dirty: bool,
    pub dirty_snapshot_id: Option<String>,
    pub crlf: bool,
    pub had_bom: bool,
    pub original_encoding: Option<&'static str>,
    pub language: Option<&'static core::highlight::Language>,
}
```

`DocumentView` temporarily becomes:

```rust
pub struct DocumentView {
    pub model: appkit_core::document::DocumentModel,
    pub display: DisplayState,
    pub highlighter_cache: HighlighterCache,
    pub cursor_render_state: CursorRenderState,
    pub search_state: SearchState,
}
```

Implement temporary `Deref/DerefMut<Target = DocumentModel>` so existing field access continues compiling during migration. Mark the compatibility deref for removal in Task 16D.

- [ ] Write a model test proving editing metadata can be created without `ui::Settings` or viewport dimensions.
- [ ] Run it and verify failure.
- [ ] Move the fields and adapt constructors.
- [ ] Run `cargo test -p textora-app --lib document_view`.
- [ ] Run `cargo test -p textora-appkit-core document`.
- [ ] Commit with `refactor(document): separate model from presentation`.

#### Task 10C: Move pure search state

**Files:**
- Move: `crates/app/src/search_state.rs` → `crates/appkit-core/src/document/search.rs`
- Modify: `crates/appkit-core/src/document/mod.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Move existing search-state tests unchanged.
- [ ] Re-export temporarily from app.
- [ ] Run `cargo test -p textora-appkit-core document::search`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(document): move search model state`.

#### Task 10D: Name the presentation aggregate

**Files:**
- Create: `crates/app/src/document_presentation.rs`
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/src/lib.rs`

Define:

```rust
pub(crate) struct DocumentPresentation {
    pub display: DisplayState,
    pub highlighter_cache: HighlighterCache,
    pub cursor_render_state: CursorRenderState,
    pub search_state: appkit_core::document::SearchState,
}
```

- [ ] Add a test proving `DocumentPresentation` can be discarded and rebuilt without changing `DocumentModel`.
- [ ] Make `DocumentView` a temporary pair of `DocumentModel` and `DocumentPresentation`.
- [ ] Run `cargo test -p textora-app --lib document_view`.
- [ ] Commit with `refactor(document): isolate rebuildable presentation state`.

#### Task 10E: Extract pure text-edit transactions

**Files:**
- Create: `crates/appkit-core/src/edit.rs`
- Modify: `crates/appkit-core/src/lib.rs`
- Modify: `crates/app/src/edit_transaction.rs`

Define the headless edit boundary:

```rust
pub struct TextEdit {
    pub range: Range<usize>,
    pub replacement: String,
}

pub struct EditOutcome {
    pub executed: bool,
    pub dirty_line_start: usize,
    pub dirty_line_end: usize,
    pub line_count_changed: bool,
}

pub fn apply_text_edit(model: &mut DocumentModel, edit: TextEdit) -> EditOutcome;
```

- [ ] Port insert, delete, replace, undo grouping and invalid-range tests that do not need pixel metrics.
- [ ] Run the core tests and verify failure before the API exists.
- [ ] Implement `apply_text_edit` against `DocumentModel`.
- [ ] Make the shell-side plugin edit plan map to `TextEdit`; keep advance-cache and plugin policy handling in `edit_transaction.rs`.
- [ ] Run `cargo test -p textora-appkit-core edit` and `cargo test -p textora-app --lib edit_transaction`.
- [ ] Commit with `refactor(edit): extract headless text transactions`.

---

### Task 11: Separate plugin runtime from workspace model

#### Task 11A: Define `TabRuntimeStore`

**Files:**
- Create: `crates/app/src/tab_runtime.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/tab.rs`

```rust
pub(crate) struct TabRuntime {
    pub plugin: Box<dyn ui::plugin::ViewPlugin>,
    pub cached_toggle_plugin: Option<Box<dyn ui::plugin::ViewPlugin>>,
    pub toggle_source_scroll_y: f32,
    pub toc_visible: bool,
    pub presentation: DocumentPresentation,
    pub canvas_viewport: CanvasViewportSession,
}

#[derive(Default)]
pub(crate) struct TabRuntimeStore {
    entries: HashMap<TabId, TabRuntime>,
}
```

- [ ] Add tests for insert/get/remove and removal by exact `TabId`.
- [ ] Verify failure before implementation.
- [ ] Move the listed runtime fields out of `DocItem`.
- [ ] Run `cargo test -p textora-app --lib tab_runtime` and `cargo test -p textora-app --lib tab`.
- [ ] Commit with `refactor(workspace): isolate per-tab runtime state`.

#### Task 11B: Make workspace lifecycle return IDs

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_tab.rs`
- Modify: `crates/app/src/app_dispatch.rs`

Workspace close/switch effects must carry IDs:

```rust
pub enum WorkspaceEffect {
    None,
    Activated(TabId),
    Closed { closed: TabId, activated: Option<TabId> },
}
```

- [ ] Add a test that every close path removes exactly one matching runtime.
- [ ] Route effects through one `App::apply_workspace_effect`.
- [ ] After runtime fields leave `DocItem`, store `DocumentModel` directly in `WorkspaceModel` and keep `suggested_file_name` on `WorkspaceEntry`; delete the empty `DocItem` adapter.
- [ ] Run `cargo test -p textora-app --lib workspace`, `cargo test -p textora-app --lib app_tab`, and `cargo test -p textora-app --lib app_dispatch`.
- [ ] Commit with `refactor(workspace): synchronize model and runtime lifecycle`.

#### Task 11C: Add typed injected routes

**Files:**
- Create: `crates/app/src/view_route.rs`
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_init.rs`

Define:

```rust
pub enum ViewPathMatcher {
    FileNameSuffix(&'static str),
    Extension(&'static str),
}

pub struct ViewRouteRule {
    pub matcher: ViewPathMatcher,
    pub default_plugin: &'static str,
    pub toggle_target: Option<&'static str>,
    pub priority: u16,
}
```

- [ ] Add tests proving `.mmap.md` beats `.md`, `.txt` maps to editor/novel, duplicate priorities are rejected, and every plugin ID exists.
- [ ] Remove `Workspace::new()` plugin registration.
- [ ] Construct registry and routes in `app_init.rs`.
- [ ] Run `cargo test -p textora-app --lib view_route` and `cargo test -p textora-app --lib workspace`.
- [ ] Commit with `refactor(plugins): inject registry and view routes`.

---

### Task 12: Define the shell/product event boundary

#### Task 12A: Add shell effects and events

**Files:**
- Create: `crates/appkit-shell/src/event.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/app_event.rs`

Implement:

```rust
#[derive(Debug, Clone)]
pub enum ShellEvent {
    StartBackgroundServices,
    ReshapeResultsReady,
    FileSafetyResultsReady,
    ProductWake,
}
```

Move the existing boolean-union effect into shell as `ShellEffect`; re-export it temporarily from app so dispatch code continues to compile.

- [ ] Add effect union-law and fixed-order tests.
- [ ] Convert current `AppEvent` into a temporary type alias/re-export.
- [ ] Run `cargo test -p textora-appkit-shell event`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(shell): define runtime event and effect contract`.

#### Task 12B: Add `ProductHost`

**Files:**
- Create: `crates/appkit-shell/src/product_host.rs`
- Create: `crates/app/src/textora_product.rs`
- Modify: `crates/appkit-shell/src/lib.rs`

Use the exact associated-type boundary:

```rust
pub trait ProductHost {
    fn start_background_services(&mut self, wake: ProductWakeHandle);
    fn drain_product_events(&mut self) -> ShellEffect;
    fn shutdown(&mut self);
}
```

`ProductWakeHandle` wraps `EventLoopProxy<ShellEvent>` and exposes only `wake() -> Result<(), WakeError>`.
`ShellRuntime` 本身不泛型；需要启动、唤醒或关闭产品服务的生命周期方法临时借用 `&mut impl ProductHost`。textora 的 widget 组合和产品 action reducer 留在 `crates/app`。

- [ ] Add a fake host test proving shell only observes `ProductWake` and never sees product payload.
- [ ] Implement `TextoraProduct` lifecycle methods without `Any`, string action names, or global callbacks.
- [ ] Run `cargo test -p textora-appkit-shell product_host`.
- [ ] Commit with `refactor(shell): add typed product host port`.

#### Task 12C: Collapse product background events

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/sync_controller.rs`

- [ ] Add a regression test that a sync completion enqueues product data, sends one `ProductWake`, drains the controller, and requests redraw.
- [ ] Replace `SyncResultsReady` and `RecentFilesLoaded(payload)` with product-owned channels plus `ProductWake`.
- [ ] Keep reshape and file-safety events as shell events.
- [ ] Run `cargo test -p textora-app --lib app_lifecycle` and `cargo test -p textora-app --lib sync_controller`.
- [ ] Commit with `refactor(events): wake opaque product services`.

---

### Task 13: Move sync settings UI into the product crate

#### Task 13A: Copy product-owned sync widget types

**Files:**
- Create: `crates/app/src/sync_settings_types.rs`
- Create: `crates/app/src/sync_settings_page.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Port the existing `sync_types.rs` and `sync_page.rs` tests into app.
- [ ] Change imports to public `ui::form`, `ui::text_box`, button, label, and scrolling primitives.
- [ ] Run `cargo test -p textora-app --lib sync_settings`.
- [ ] Commit with `refactor(sync-ui): host sync settings in textora`.

#### Task 13B: Remove sync semantics from generic settings types

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/types.rs`
- Modify: `crates/ui/src/widgets/settings_view/widget.rs`
- Modify: `crates/ui/src/widgets/settings_view/mod.rs`

- [ ] Write a boundary test asserting `SettingsCategory` has no `Sync` variant and `SettingsViewAction` has no `Sync` variant.
- [ ] Remove sync module declarations/re-exports and sync rendering branches.
- [ ] Run `cargo test -p textora-ui`.
- [ ] Commit with `refactor(ui): remove textora sync settings semantics`.

#### Task 13C: Wire the product sync page

**Files:**
- Modify: `crates/app/src/settings_overlay.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/ui_shell.rs`

- [ ] Add `TextoraAction::Sync(SyncSettingsAction)` in the app reducer and an integration test covering opening settings, selecting Sync, emitting that typed action, and dispatching it to `TextoraProduct`.
- [ ] Compose the sync page in the textora layer beside the generic settings widget.
- [ ] Remove `ui::settings_view::SyncSettingsAction` imports.
- [ ] Run `cargo test -p textora-app --lib settings_overlay`, `cargo test -p textora-app --lib app_dispatch`, and `cargo test -p textora-app --lib ui_shell`.
- [ ] Commit with `refactor(app): compose product sync settings`.

#### Task 13D: Delete obsolete UI sync modules

**Files:**
- Delete: `crates/ui/src/widgets/settings_view/sync_page.rs`
- Delete: `crates/ui/src/widgets/settings_view/sync_types.rs`

- [ ] Confirm their tests now exist under `crates/app`.
- [ ] Delete both dead source files.
- [ ] Run `rg -n 'SyncSettings|textora_sync' crates/ui` and expect no output.
- [ ] Run `cargo test -p textora-ui` and `cargo test -p textora-app --lib sync_settings`.
- [ ] Commit with `refactor(ui): delete obsolete sync settings modules`.

---

### Task 14: Extract the physical shell modules

Perform these moves only after Tasks 9–13 have removed product types from the runtime files. Each table row is an independent subtask: move one module, update `appkit-shell/src/lib.rs`, and replace the old `mod` declaration in `app/src/lib.rs` with a temporary re-export. Each row therefore touches at most 3 files.

| Order | Source | Destination | Required verification |
|---|---|---|---|
| 14A | `canvas_viewport.rs` | `appkit-shell/src/canvas_viewport.rs` | `cargo test -p textora-appkit-shell canvas_viewport` |
| 14B | `snap_tree.rs` | `appkit-shell/src/snap_tree.rs` | `cargo test -p textora-appkit-shell snap_tree` |
| 14C | `display_line_map.rs` | `appkit-shell/src/display_line_map.rs` | `cargo test -p textora-appkit-shell display_line_map` |
| 14D | `render_cache.rs` | `appkit-shell/src/render_cache.rs` | `cargo test -p textora-appkit-shell render_cache` |
| 14E | `frame_cache.rs` | `appkit-shell/src/frame_cache.rs` | `cargo test -p textora-appkit-shell frame_cache` |
| 14F | `reshape_worker.rs` | `appkit-shell/src/reshape_worker.rs` | `cargo test -p textora-appkit-shell reshape_worker` |
| 14G | `render_state.rs` | `appkit-shell/src/render_state.rs` | `cargo test -p textora-appkit-shell render_state` |
| 14H | `gpu.rs` | `appkit-shell/src/gpu.rs` | `cargo test -p textora-appkit-shell gpu` |
| 14I | `paint_backend.rs` | `appkit-shell/src/paint_backend.rs` | `cargo test -p textora-appkit-shell paint_backend` |
| 14J | `text_rasterize.rs` | `appkit-shell/src/text_rasterize.rs` | `cargo test -p textora-appkit-shell text_rasterize` |
| 14K | `measure_adapter.rs` | `appkit-shell/src/measure_adapter.rs` | `cargo test -p textora-appkit-shell measure_adapter` |
| 14L | `render_pipeline.rs` | `appkit-shell/src/render_pipeline.rs` | `cargo test -p textora-appkit-shell render_pipeline` |
| 14M | `render_pipeline_tests.rs` | `appkit-shell/src/render_pipeline_tests.rs` | `cargo test -p textora-appkit-shell render_pipeline` |
| 14N | `document_presentation.rs` | `appkit-shell/src/document_presentation.rs` | `cargo test -p textora-appkit-shell document_presentation` |

For every subtask:

- [ ] Confirm the source contains no `textora_sync`, `sync_controller`, `NativeMenu`, or `textora_markdown`.
- [ ] Run the existing test before moving it.
- [ ] Use `git mv`.
- [ ] Make only visibility/import changes required by the crate boundary.
- [ ] Run the row verification and `cargo check -p textora-app`.
- [ ] Commit each row with its exact destination module name, for example `refactor(appkit-shell): move canvas viewport` for 14A and `refactor(appkit-shell): move document presentation` for 14N.

After 14N:

```bash
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
```

Expected: PASS.

---

### Task 15: Move input, dispatch, and lifecycle mechanisms

#### Task 15A: Split domain command from winit mapping

**Files:**
- Create: `crates/appkit-core/src/edit_command.rs`
- Modify: `crates/appkit-core/src/lib.rs`
- Modify: `crates/app/src/input.rs`

- [ ] Move `EditCommand` into core unchanged.
- [ ] Keep `key_to_command(Key, ModifiersState)` in app temporarily.
- [ ] Re-export `EditCommand` from `input.rs` to preserve call sites.
- [ ] Run all input mapping tests and `cargo check -p textora-app`.
- [ ] Commit with `refactor(input): separate edit intent from winit mapping`.

#### Task 15B: Move the winit mapper

**Files:**
- Move: `crates/app/src/input.rs` → `crates/appkit-shell/src/input_mapper.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Export `key_to_command`.
- [ ] Run `cargo test -p textora-appkit-shell input_mapper`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(appkit-shell): move input mapper`.

#### Task 15C: Extract window-input normalization

**Files:**
- Create: `crates/appkit-shell/src/window_input.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/events.rs`

- [ ] Move only winit key/mouse/IME normalization and product-independent guards into `window_input`.
- [ ] Keep textora widget composition and widget-action translation in `app/events.rs`.
- [ ] Add tests for `NamedKey::Process`, IME preedit suppression, modifier conversion and scroll normalization.
- [ ] Run `cargo test -p textora-appkit-shell window_input`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(appkit-shell): extract window input normalization`.

---

### Task 16: Introduce the final shell runtime facade

#### Task 16A: Define `ShellRuntime`

**Files:**
- Create: `crates/appkit-shell/src/runtime.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/app.rs`

`ShellRuntime` owns only shared state: window/GPU/text, workspace model, tab runtime store, settings/theme snapshots needed for rendering, file-safety worker, mouse/input state, render caches and reshape state.

`App` becomes:

```rust
pub struct App {
    shell: appkit_shell::ShellRuntime,
    product: crate::textora_product::TextoraProduct,
}
```

- [ ] Add a compile-time constructor test that creates `ShellRuntime` with a fake registry/routes and no textora sync types.
- [ ] Move fields from `App` to `ShellRuntime`.
- [ ] Add focused accessors; do not expose all fields as `pub`.
- [ ] Run `cargo test -p textora-appkit-shell runtime`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(shell): own shared runtime state`.

#### Task 16B: Delegate winit lifecycle

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/appkit-shell/src/runtime.rs`
- Modify: `crates/app/src/app.rs`

- [ ] Keep `ApplicationHandler<ShellEvent>` implemented for local `App`.
- [ ] Delegate `resumed`, `window_event`, `user_event`, `about_to_wait` and shutdown handling to explicit `ShellRuntime` methods that receive `&mut self.product` only when product lifecycle work is required.
- [ ] Add lifecycle tests for resumed, redraw, close, IME, product wake, and shutdown ordering.
- [ ] Run `cargo test -p textora-app --lib app_lifecycle`.
- [ ] Commit with `refactor(app): delegate lifecycle to shell`.

#### Task 16C: Move runtime reducers and frame orchestration

After `ShellRuntime` exists, move these modules one at a time:

| Source | Destination | Test filter |
|---|---|---|
| `crates/app/src/dispatch/editor.rs` | `crates/appkit-shell/src/dispatch/editor.rs` | `dispatch::editor` |
| `crates/app/src/dispatch/mouse.rs` | `crates/appkit-shell/src/dispatch/mouse.rs` | `dispatch::mouse` |
| `crates/app/src/dispatch/search.rs` | `crates/appkit-shell/src/dispatch/search.rs` | `dispatch::search` |
| `crates/app/src/dispatch/tabs.rs` | `crates/appkit-shell/src/dispatch/tabs.rs` | `dispatch::tabs` |
| `crates/app/src/dispatch/viewport.rs` | `crates/appkit-shell/src/dispatch/viewport.rs` | `dispatch::viewport` |
| `crates/app/src/dispatch/wysiwyg.rs` | `crates/appkit-shell/src/dispatch/wysiwyg.rs` | `dispatch::wysiwyg` |
| `crates/app/src/app_reshape.rs` | `crates/appkit-shell/src/reshape.rs` | `reshape` |
| `crates/app/src/app_renderer.rs` | `crates/appkit-shell/src/renderer.rs` | `renderer` |

For each module:

- [ ] First change its receiver from `&mut App` to `&mut ShellRuntime` while the file remains in `crates/app`; modify only that source, `appkit-shell/src/runtime.rs`, and its nearest app caller.
- [ ] Run the module's existing tests and `cargo check -p textora-app`.
- [ ] In a second subtask, move the source file, modify `appkit-shell/src/lib.rs`, and replace the app module with a temporary semantic re-export.
- [ ] Run `cargo test -p textora-appkit-shell` with the exact filter from the table, then run `cargo check -p textora-app`.
- [ ] Commit receiver conversion and physical movement separately.

Keep `dispatch/chrome.rs`, product settings reduction and native-menu command translation in `crates/app`; they implement the textora composition rather than reusable shell mechanics.

#### Task 16D: Remove compatibility derefs and re-exports

**Files:**
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/appkit-shell/src/runtime.rs`

- [ ] Replace remaining implicit `DocumentView → DocumentModel` dereferences with `model()` / `model_mut()`.
- [ ] Delete temporary module re-exports whose call sites now use `appkit_core` or `appkit_shell`.
- [ ] Run `cargo check --workspace`.
- [ ] Run `cargo test -p textora-app --lib`.
- [ ] Commit with `refactor(appkit): remove migration compatibility layer`.

---

### Task 17: Move workspace persistence DTOs and store

#### Task 17A: Move persisted DTOs

**Files:**
- Modify: `crates/appkit-core/src/workspace/types.rs`
- Modify: `crates/app/src/workspace.rs`

- [ ] Move `PersistedTab` and `PersistedWorkspace` into core.
- [ ] Preserve every serde name/default and add a golden TOML round-trip test using the existing format.
- [ ] Run `cargo test -p textora-appkit-core workspace::types`.
- [ ] Run workspace restore tests in app.
- [ ] Commit with `refactor(workspace): move persistence schema to core`.

#### Task 17B: Move workspace store

**Files:**
- Move: `crates/app/src/workspace_store.rs` → `crates/appkit-core/src/workspace/store.rs`
- Modify: `crates/appkit-core/src/workspace/mod.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] Keep all paths injected.
- [ ] Move store tests with the module.
- [ ] Run `cargo test -p textora-appkit-core workspace::store`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Commit with `refactor(appkit-core): move workspace store`.

---

### Task 18: Final boundary and behavior verification

**Files:**
- Modify: `scripts/verify.sh`
- Modify: `crates/ui/tests/public_boundaries.rs`
- Modify: `crates/app/tests/public_api.rs`

- [ ] **Step 1: Make architecture checks unconditional**

Remove `TEXTORA_ARCHITECTURE_MIGRATION` handling and always call:

```bash
bash scripts/check_architecture.sh
```

- [ ] **Step 2: Strengthen source boundaries**

Extend UI boundary tests to reject:

```rust
for forbidden in ["SyncSettingsAction", "SyncSettingsInput", "Syncthing", "textora_sync"] {
    assert!(!source.contains(forbidden), "ui contains product sync type {forbidden}");
}
```

Extend app public API tests to assert only the `textora` binary entry and expected re-exports remain.

- [ ] **Step 3: Run dependency checks**

```bash
bash scripts/check_architecture.sh
cargo tree -p textora-appkit-core
cargo tree -p textora-appkit-shell
```

Expected: no forbidden dependency.

- [ ] **Step 4: Run crate verification**

```bash
cargo fmt --all -- --check
cargo check -p textora-appkit-core
cargo check -p textora-appkit-shell
cargo check -p textora-app
cargo test -p textora-appkit-core
cargo test -p textora-appkit-shell
cargo test -p textora-ui
cargo test -p textora-app
```

Expected: all PASS.

- [ ] **Step 5: Run full verification**

```bash
./scripts/verify.sh
```

Expected: formatting, workspace clippy and workspace tests all PASS.

- [ ] **Step 6: Run manual regression protocol**

Follow `docs/manual_test_protocol.md`, covering at minimum:

- launch and first frame;
- open/edit/save/reopen `.txt` and `.md`;
- WYSIWYG/source toggle and scroll restoration;
- `.mmap.md` edit, canvas pan/zoom and style panel;
- tab open/switch/pin/close/reopen;
- dirty hot-exit and workspace restore;
- settings save/reload;
- sync settings open, connection test, library status refresh;
- macOS open-document callback and native menu recent files.

- [ ] **Step 7: Commit**

```bash
git add scripts/verify.sh crates/ui/tests/public_boundaries.rs crates/app/tests/public_api.rs
git commit -m "test(architecture): verify final appkit split"
```

---

## Completion Gate

Work is complete only when all of the following are true:

- `crates/appkit-core` and `crates/appkit-shell` contain the responsibilities defined by the design.
- `crates/app` contains product composition/services rather than duplicated runtime implementations.
- No compatibility re-export, temporary path adapter, migration environment switch or dead module remains.
- `cargo tree` and source boundary checks pass.
- `./scripts/verify.sh` passes from a clean worktree.
- The manual regression protocol has recorded passing results for textora behavior.
