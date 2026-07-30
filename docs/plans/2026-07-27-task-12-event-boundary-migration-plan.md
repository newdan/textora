# Task 12 Event Boundary Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 winit user event 收敛为无产品 payload 的 `ShellEvent`，并通过 `TextoraProduct` inbox 与 `ProductWakeHandle` 处理 sync、recent files 和 macOS open-document 数据。

**Architecture:** 先在 `appkit-shell` 建立最终 `ShellEvent`、`ShellEffect` 与 `ProductHost`，同时保留本地 `AppEvent` 作为可编译迁移壳。随后把 `sync_controller` 和 `native_menu` 的所有权迁入 `TextoraProduct`，分别迁移三类产品事件；最后把 `app_event.rs` 收敛为 `ShellEvent` re-export。

**Tech Stack:** Rust 2024、`std::sync::mpsc`、winit 0.30、现有 `textora-appkit-shell` / `textora-app` crates。

## Global Constraints

- 全程保持只生成 `textora` 一个 binary；不创建笔记 App。
- `appkit-core` 禁止依赖 `ui`、`winit`、`wgpu`、`render`、`shaping`、`textora-markdown`、`textora-sync`。
- `appkit-shell` 禁止依赖 `textora-markdown`、`textora-sync`、`textora-app`。
- `appkit-shell` 不得解析 sync、recent files、macOS open-document 或其他产品 payload。
- `ShellEvent` 最终只能包含 `StartBackgroundServices`、`ReshapeResultsReady`、`FileSafetyResultsReady`、`ProductWake`。
- `ShellEffect` 不得包含产品状态、产品 action 或文件路径。
- 禁止使用 `Any`、字符串 action 名、全局回调表或泛型产品 action。
- 每个实现子任务最多修改 3 个文件；超过 3 个文件必须继续拆分。
- 所有行为变更先写失败测试；每个提交前运行 `cargo fmt --all -- --check` 和相关 crate 编译。
- 不改变 `~/.edit+` 下设置、workspace、history、pinned paths 和 dirty snapshot 的兼容格式。

**Design addendum:** `docs/specs/2026-07-27-task-12-event-boundary-migration-addendum.md`

## File Structure

| File | Responsibility after Task 12 |
|---|---|
| `crates/appkit-shell/src/event.rs` | Payload-free `ShellEvent` and reusable `ShellEffect` |
| `crates/appkit-shell/src/product_host.rs` | `ProductHost` port and typed wake handle |
| `crates/app/src/textora_product.rs` | textora product services, product-result inbox, open-document inbox |
| `crates/app/src/app_event.rs` | Temporary `ShellEvent as AppEvent` re-export only |
| `crates/app/src/app.rs` | Product/shell composition and compatibility accessors during migration |
| `crates/app/src/app_lifecycle.rs` | Local App reduction of product wake and open-document commands |
| `crates/app/src/macos_open_documents.rs` | Objective-C URL bridge to typed product inbox |

---

### Task 12.1: Move shell event and effect contracts

**Files:**
- Create: `crates/appkit-shell/src/event.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/app_effect.rs`

**Interfaces:**
- Produces: `appkit_shell::{ShellEvent, ShellEffect, ShellEffectStep}`
- Preserves: `crate::app_effect::{AppEffect, AppEffectStep}` as temporary re-exports

- [ ] **Step 1: Write the shell contract tests**

Add tests in `event.rs` for the exact event variants and the existing effect laws:

```rust
#[test]
fn product_wake_carries_no_payload() {
    assert!(matches!(ShellEvent::ProductWake, ShellEvent::ProductWake));
}

#[test]
fn merge_obeys_boolean_union_laws() {
    let x = ShellEffect::RESHAPE.merge(ShellEffect::PERSIST_SETTINGS);
    let y = ShellEffect::UPDATE_TITLE.merge(ShellEffect::PERSIST_WORKSPACE);
    let z = ShellEffect::SYNC_WINDOW_CHROME;

    assert_eq!(x.merge(ShellEffect::NONE), x);
    assert_eq!(x.merge(x), x);
    assert_eq!(x.merge(y), y.merge(x));
    assert_eq!(x.merge(y).merge(z), x.merge(y.merge(z)));
}

#[test]
fn execution_steps_have_fixed_order() {
    let effect = ShellEffect::RESHAPE
        .merge(ShellEffect::SYNC_WINDOW_CHROME)
        .merge(ShellEffect::UPDATE_TITLE)
        .merge(ShellEffect::PERSIST_SETTINGS)
        .merge(ShellEffect::PERSIST_WORKSPACE);

    assert_eq!(
        effect.steps().collect::<Vec<_>>(),
        vec![
            ShellEffectStep::Reshape,
            ShellEffectStep::SyncWindowChrome,
            ShellEffectStep::UpdateTitle,
            ShellEffectStep::PersistSettings,
            ShellEffectStep::PersistWorkspace,
            ShellEffectStep::Redraw,
        ]
    );
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p textora-appkit-shell event
```

Expected: FAIL because `event` and the shell contract types do not exist.

- [ ] **Step 3: Move the effect implementation**

Move the complete current `AppEffectStep` / `AppEffect` implementation into `event.rs`, rename them to `ShellEffectStep` / `ShellEffect`, make constants, fields, `merge`, and `steps` public, and define:

```rust
#[derive(Debug, Clone)]
pub enum ShellEvent {
    StartBackgroundServices,
    ReshapeResultsReady,
    FileSafetyResultsReady,
    ProductWake,
}
```

Export the types from `appkit-shell/src/lib.rs`:

```rust
pub mod event;

pub use event::{ShellEffect, ShellEffectStep, ShellEvent};
```

Replace `app_effect.rs` with:

```rust
pub(crate) use appkit_shell::{
    ShellEffect as AppEffect,
    ShellEffectStep as AppEffectStep,
};
```

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test -p textora-appkit-shell event
cargo test -p textora-app --lib app_effect
cargo check -p textora-app
cargo fmt --all -- --check
```

Expected: all PASS with no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/appkit-shell/src/event.rs crates/appkit-shell/src/lib.rs crates/app/src/app_effect.rs
git commit -m "refactor(shell): define runtime event and effect contract"
```

---

### Task 12.2: Add the typed product host port

**Files:**
- Create: `crates/appkit-shell/src/product_host.rs`
- Modify: `crates/appkit-shell/src/lib.rs`

**Interfaces:**
- Consumes: `ShellEvent`, `ShellEffect`
- Produces: `ProductHost`, `ProductWakeHandle`, `WakeError`

- [ ] **Step 1: Write failing contract tests**

Add:

```rust
#[test]
fn fake_host_exposes_only_shell_effects() {
    struct FakeHost {
        drained: bool,
        stopped: bool,
    }

    impl ProductHost for FakeHost {
        fn start_background_services(&mut self, _wake: ProductWakeHandle) {
            unreachable!("wake construction is covered separately");
        }

        fn drain_product_events(&mut self) -> ShellEffect {
            self.drained = true;
            ShellEffect::REDRAW
        }

        fn shutdown(&mut self) {
            self.stopped = true;
        }
    }

    let mut host = FakeHost { drained: false, stopped: false };
    assert_eq!(host.drain_product_events(), ShellEffect::REDRAW);
    host.shutdown();
    assert!(host.drained);
    assert!(host.stopped);
}

#[test]
fn wake_error_is_stable_and_payload_free() {
    assert_eq!(WakeError.to_string(), "event loop is unavailable");
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test -p textora-appkit-shell product_host`.

Expected: FAIL because `ProductHost`, `ProductWakeHandle`, and `WakeError` do not exist.

- [ ] **Step 3: Implement the exact port**

Implement:

```rust
#[derive(Clone)]
pub struct ProductWakeHandle {
    event_loop_proxy: winit::event_loop::EventLoopProxy<ShellEvent>,
}

impl ProductWakeHandle {
    pub fn new(
        event_loop_proxy: winit::event_loop::EventLoopProxy<ShellEvent>,
    ) -> Self {
        Self { event_loop_proxy }
    }

    pub fn wake(&self) -> Result<(), WakeError> {
        self.event_loop_proxy
            .send_event(ShellEvent::ProductWake)
            .map_err(|_| WakeError)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeError;

impl std::fmt::Display for WakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("event loop is unavailable")
    }
}

impl std::error::Error for WakeError {}

pub trait ProductHost {
    fn start_background_services(&mut self, wake: ProductWakeHandle);
    fn drain_product_events(&mut self) -> ShellEffect;
    fn shutdown(&mut self);
}
```

Re-export all three public types from `appkit-shell/src/lib.rs`.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test -p textora-appkit-shell product_host
cargo check -p textora-appkit-shell
cargo fmt --all -- --check
```

Expected: PASS.

Commit:

```bash
git add crates/appkit-shell/src/product_host.rs crates/appkit-shell/src/lib.rs
git commit -m "refactor(shell): add typed product host port"
```

---

### Task 12.3: Create the textora product container

**Files:**
- Create: `crates/app/src/textora_product.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Consumes: `ProductHost`, `ProductWakeHandle`, `ShellEffect`, `SyncController`, `NativeMenu`
- Produces: `TextoraProduct`, `ProductEventSender`, `OpenDocumentSender`

- [ ] **Step 1: Write failing inbox tests**

Test the concrete channel behavior:

```rust
#[test]
fn open_document_inbox_preserves_path_order() {
    let mut product = TextoraProduct::new();
    product
        .open_document_sender()
        .send(vec![PathBuf::from("/tmp/a.md"), PathBuf::from("/tmp/b.txt")])
        .expect("product receiver is alive");

    assert_eq!(
        product.drain_open_documents(),
        vec![PathBuf::from("/tmp/a.md"), PathBuf::from("/tmp/b.txt")]
    );
}

#[test]
fn sync_completion_drains_to_redraw() {
    let mut product = TextoraProduct::new();
    product
        .event_sender()
        .send_sync_results_ready()
        .expect("product receiver is alive");

    assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test -p textora-app --lib textora_product`.

Expected: FAIL because the module and types do not exist.

- [ ] **Step 3: Implement typed inboxes and lifecycle**

Use private `ProductEvent` variants:

```rust
enum ProductEvent {
    RecentFilesLoaded(Vec<PathBuf>),
    SyncResultsReady,
}
```

`ProductEventSender` and `OpenDocumentSender` must wrap `mpsc::Sender` and expose only typed send methods. `ProductEventSender` stays crate-private; `OpenDocumentSender` is public because the binary composition passes it into the public macOS installation function, while its `send` method remains `pub(crate)`. Define a zero-sized `ProductEventSendError` with stable `Display` text `"product event receiver is unavailable"`.

`TextoraProduct` owns:

```rust
pub(crate) struct TextoraProduct {
    sync_controller: Option<crate::sync_controller::SyncController>,
    native_menu: Option<crate::native_menu::NativeMenu>,
    event_sender: ProductEventSender,
    event_receiver: std::sync::mpsc::Receiver<ProductEvent>,
    open_document_sender: OpenDocumentSender,
    open_document_receiver: std::sync::mpsc::Receiver<Vec<PathBuf>>,
}
```

Implement:

```rust
pub(crate) fn new() -> Self;
pub(crate) fn event_sender(&self) -> ProductEventSender;
pub(crate) fn open_document_sender(&self) -> OpenDocumentSender;
pub(crate) fn drain_open_documents(&mut self) -> Vec<PathBuf>;
pub(crate) fn sync_controller(&self) -> Option<&SyncController>;
pub(crate) fn sync_controller_mut(&mut self) -> Option<&mut SyncController>;
pub(crate) fn set_sync_controller(&mut self, controller: SyncController);
pub(crate) fn take_sync_controller(&mut self) -> Option<SyncController>;
pub(crate) fn native_menu(&self) -> Option<&NativeMenu>;
pub(crate) fn set_native_menu(&mut self, native_menu: NativeMenu);
```

Implement `ProductHost`:

- `start_background_services` creates one `SyncController` only when absent; its completion callback sends `SyncResultsReady`, then calls `wake()`;
- `drain_product_events` drains all queued events, updates `NativeMenu` for recent paths, drains the controller for sync completion, and merges `ShellEffect::REDRAW` for sync completion;
- `shutdown` takes and shuts down the controller.

Declare `mod textora_product;` and re-export the bridge sender:

```rust
mod textora_product;

pub use textora_product::OpenDocumentSender;
```

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test -p textora-app --lib textora_product
cargo check -p textora-app
cargo fmt --all -- --check
```

Expected: PASS.

Commit:

```bash
git add crates/app/src/textora_product.rs crates/app/src/lib.rs
git commit -m "refactor(app): add textora product event container"
```

---

### Task 12.4: Attach `TextoraProduct` to `App`

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_init.rs`

**Interfaces:**
- Consumes: `TextoraProduct::new`
- Produces: `App::product`

- [ ] **Step 1: Write a failing constructor test**

In `app_init.rs`, add:

```rust
#[test]
fn app_constructor_creates_empty_product_inboxes() {
    let mut app = App::new(None);
    assert!(app.product.drain_open_documents().is_empty());
}
```

- [ ] **Step 2: Verify RED**

Run the exact test and expect failure because `App` has no `product` field.

- [ ] **Step 3: Add the field and initialize it**

Add:

```rust
pub(crate) product: crate::textora_product::TextoraProduct,
```

Initialize with:

```rust
product: crate::textora_product::TextoraProduct::new(),
```

Do not remove `sync_controller` or `native_menu` yet.

- [ ] **Step 4: Verify and commit**

Run constructor tests, `cargo check -p textora-app`, and formatting.

Commit:

```bash
git add crates/app/src/app.rs crates/app/src/app_init.rs
git commit -m "refactor(app): attach textora product container"
```

---

### Task 12.5: Route sync consumers through App accessors

#### Task 12.5A: Add accessors and migrate dispatch

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_dispatch.rs`

- [ ] Add a source-boundary test in `app_dispatch.rs` rejecting direct `.sync_controller` field access.
- [ ] Verify the boundary test fails.
- [ ] Add `App::{sync_controller, sync_controller_mut, set_sync_controller, take_sync_controller}` delegating to the current App field.
- [ ] Replace direct field access in `app_dispatch.rs` with these methods.
- [ ] Run `cargo test -p textora-app --lib app_dispatch`, `cargo check -p textora-app`, and formatting.
- [ ] Commit with `refactor(sync): route dispatch through product accessors`.

#### Task 12.5B: Migrate overlay and lifecycle consumers

**Files:**
- Modify: `crates/app/src/settings_overlay.rs`
- Modify: `crates/app/src/app_lifecycle.rs`

- [ ] Add source-boundary tests rejecting direct `.sync_controller` access in both files and verify RED.
- [ ] Replace reads/mutations with `sync_controller()` / `sync_controller_mut()` / `set_sync_controller(...)`.
- [ ] Run `cargo test -p textora-app --lib settings_overlay` and `cargo test -p textora-app --lib app_lifecycle`.
- [ ] Run `cargo check -p textora-app` and formatting.
- [ ] Commit with `refactor(sync): route lifecycle through product accessors`.

#### Task 12.5C: Migrate shutdown

**Files:**
- Modify: `crates/app/src/app_window.rs`

- [ ] Add a source-boundary test rejecting direct `.sync_controller.take()`, verify RED, and replace it with `take_sync_controller()`.
- [ ] Run `cargo test -p textora-app --lib app_window`, `cargo check -p textora-app`, and formatting.
- [ ] Commit with `refactor(sync): route shutdown through product accessors`.

#### Task 12.5D: Move sync ownership

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/app/src/textora_product.rs`

- [ ] Add a source-boundary test using `include_str!("app.rs")` and the split literal `["pub(crate) sync_", "controller:"].concat()` to require that `App` no longer declares the field; verify RED.
- [ ] Remove `App::sync_controller`.
- [ ] Change the four App accessor bodies to delegate to `self.product`.
- [ ] Remove `sync_controller: None` from `app_init.rs`; `TextoraProduct::new()` remains the single initializer.
- [ ] Run `rg -n 'pub\\(crate\\) sync_controller|sync_controller: None' crates/app/src/app.rs crates/app/src/app_init.rs` and expect no output.
- [ ] Run app dispatch, lifecycle, settings overlay, app window tests, `cargo check -p textora-app`, and formatting.
- [ ] Commit with `refactor(sync): move controller ownership into textora product`.

---

### Task 12.6: Move native menu ownership

#### Task 12.6A: Add accessors and migrate tab updates

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_tab.rs`

- [ ] Add a boundary test rejecting direct assignment to `self.native_menu` in `app_tab.rs`; verify RED.
- [ ] Add `App::{native_menu, set_native_menu}` delegating to the current App field.
- [ ] Replace direct assignment in `rebuild_native_menu`.
- [ ] Run `cargo test -p textora-app --lib app_tab`, app check, and formatting.
- [ ] Commit with `refactor(menu): route tab updates through product accessors`.

#### Task 12.6B: Migrate lifecycle menu access

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`

- [ ] Add a boundary test rejecting `self.native_menu` field access and verify RED.
- [ ] Replace reads with `native_menu()` and assignments with `set_native_menu(...)`.
- [ ] Preserve main-thread polling and `MenuAction` dispatch order.
- [ ] Run `cargo test -p textora-app --lib app_lifecycle`, app check, and formatting.
- [ ] Commit with `refactor(menu): route lifecycle through product accessors`.

#### Task 12.6C: Move native menu ownership

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/app/src/textora_product.rs`

- [ ] Add a source-boundary test using `include_str!("app.rs")` and the split literal `["pub(crate) native_", "menu:"].concat()` to require that `App` no longer declares the field; verify RED.
- [ ] Remove `App::native_menu`.
- [ ] Delegate accessors to `self.product`.
- [ ] Remove `native_menu: None` from `app_init.rs`.
- [ ] Run lifecycle and app-tab tests, app check, and formatting.
- [ ] Commit with `refactor(menu): move native menu ownership into textora product`.

---

### Task 12.7: Migrate recent files to the product inbox

**Files:**
- Modify: `crates/app/src/app_event.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/textora_product.rs`

**Interfaces:**
- Produces: local migration event `AppEvent::ProductWake`
- Removes: `AppEvent::RecentFilesLoaded(Vec<PathBuf>)`

- [ ] **Step 1: Write the failing regression test**

Add a test that creates a product event sender, runs the recent loader with a fake history, receives `AppEvent::ProductWake`, drains the product, and verifies `native_menu().is_some()`.

- [ ] **Step 2: Verify RED**

Run the exact `app_lifecycle` test and expect it to fail because recent paths still travel in `AppEvent`.

- [ ] **Step 3: Route payload through the inbox**

Add `AppEvent::ProductWake`. Change `spawn_recent_file_loader` to accept `ProductEventSender`, send paths through `send_recent_files_loaded(paths)`, and only then send `AppEvent::ProductWake`.

Remove `RecentFilesLoaded(Vec<PathBuf>)` and its handler. Handle `ProductWake` by calling `ProductHost::drain_product_events(&mut self.product)` and applying the returned effect.

- [ ] **Step 4: Verify and commit**

Run app lifecycle and textora product tests, app check, and formatting.

Commit:

```bash
git add crates/app/src/app_event.rs crates/app/src/app_lifecycle.rs crates/app/src/textora_product.rs
git commit -m "refactor(events): queue recent files in textora product"
```

---

### Task 12.8: Migrate macOS open-document to the product inbox

#### Task 12.8A: Make the Objective-C bridge payload-free

**Files:**
- Modify: `crates/app/src/macos_open_documents.rs`
- Modify: `crates/app/src/textora_product.rs`

- [ ] Add a failing test around a test-only delivery helper: two paths are sent to `OpenDocumentSender`, and the wake callback is invoked exactly once.
- [ ] Introduce a private `OpenDocumentBridge` holding `OpenDocumentSender` plus the event-loop proxy during migration.
- [ ] Store that bridge in the existing `OnceLock`; do not store a closure or App reference.
- [ ] Change the Objective-C callback to enqueue paths, then send `AppEvent::ProductWake`.
- [ ] Run `cargo test -p textora-app --lib macos_open_documents`, app check, and formatting.
- [ ] Commit with `refactor(macos): queue open documents in textora product`.

#### Task 12.8B: Install the bridge from product composition

**Files:**
- Modify: `crates/app/src/main.rs`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/macos_open_documents.rs`

- [ ] Add an `App::open_document_sender()` accessor.
- [ ] Make that accessor public with the exact signature `pub fn open_document_sender(&self) -> crate::OpenDocumentSender`.
- [ ] Construct `App` before installing the macOS handler.
- [ ] Change `install_macos_open_document_handler` to accept both the event-loop proxy and `OpenDocumentSender`.
- [ ] Preserve handler installation before `run_app`.
- [ ] Run macOS bridge tests, `cargo check -p textora-app`, and formatting.
- [ ] Commit with `refactor(macos): install typed open document bridge`.

#### Task 12.8C: Drain open-document commands in App

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/textora_product.rs`

- [ ] Add a failing regression test enqueueing valid and invalid paths, dispatching one `ProductWake`, and proving later paths continue after one open error.
- [ ] In the `ProductWake` arm, drain open paths first and pass them to `handle_open_file_requests`, then drain `ProductHost` events and apply the merged effect.
- [ ] Preserve existing per-path error logging and continuation behavior.
- [ ] Run app lifecycle and product tests, app check, and formatting.
- [ ] Commit with `refactor(events): drain typed open document requests`.

---

### Task 12.9: Switch sync completion and the event loop to shell types

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app_event.rs`

**Interfaces:**
- Removes: `AppEvent::SyncResultsReady`
- Uses: `ProductHost::start_background_services(ProductWakeHandle)`
- Produces: temporary `pub use appkit_shell::ShellEvent as AppEvent`

- [ ] **Step 1: Write failing sync and source-boundary regressions**

Add a test proving one controller completion enqueues one product event, emits one `ProductWake`, `ProductHost::drain_product_events` drains the controller, and the result includes `ShellEffect::REDRAW`.

In `app_event.rs`, add a test using `include_str!` and split string literals:

```rust
#[test]
fn app_event_is_only_a_shell_event_reexport() {
    let source = include_str!("app_event.rs");
    assert!(!source.contains(&["pub enum App", "Event"].concat()));
    assert!(!source.contains(&["RecentFiles", "Loaded"].concat()));
    assert!(!source.contains(&["SyncResults", "Ready"].concat()));
    assert!(!source.contains(&["Open", "Files"].concat()));
}
```

- [ ] **Step 2: Verify RED**

Run the exact sync lifecycle test and `cargo test -p textora-app --lib app_event`.

Expected: FAIL because the callback still sends `SyncResultsReady` and the local enum still exists.

- [ ] **Step 3: Collapse the local enum and start sync through `ProductHost`**

Replace the production enum in `app_event.rs` with:

```rust
pub use appkit_shell::ShellEvent as AppEvent;
```

In `App::start_background_services`, construct:

```rust
let wake = appkit_shell::ProductWakeHandle::new(event_loop_proxy);
appkit_shell::ProductHost::start_background_services(&mut self.product, wake);
```

Remove the direct `SyncController::new_default` construction. Remove `AppEvent::SyncResultsReady` and its lifecycle arm.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo test -p textora-app --lib app_lifecycle
cargo test -p textora-app --lib app_event
cargo test -p textora-app --lib sync_controller
cargo test -p textora-app --lib textora_product
cargo check -p textora-app
cargo fmt --all -- --check
```

Expected: PASS.

Commit:

```bash
git add crates/app/src/app.rs crates/app/src/app_lifecycle.rs crates/app/src/app_event.rs
git commit -m "refactor(events): expose only shell user events"
```

---

### Task 12.10: Finish the typed macOS wake bridge

**Files:**
- Modify: `crates/app/src/main.rs`
- Modify: `crates/app/src/macos_open_documents.rs`

**Interfaces:**
- Consumes: `ProductWakeHandle`
- Removes: raw `EventLoopProxy<AppEvent>` storage from the Objective-C bridge

- [ ] **Step 1: Write a failing source-boundary test**

In `macos_open_documents.rs`, add:

```rust
#[test]
fn bridge_stores_only_the_typed_wake_handle() {
    let source = include_str!("macos_open_documents.rs");
    assert!(!source.contains(&["EventLoopProxy", "<AppEvent>"].concat()));
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test -p textora-app --lib macos_open_documents`.

Expected: FAIL because the bridge still stores a raw event-loop proxy.

- [ ] **Step 3: Store the typed handle**

Change `install_macos_open_document_handler` to accept `ProductWakeHandle` instead of a raw event-loop proxy. Store the handle in the bridge and call `wake()`. In `main.rs`, pass:

```rust
let product_wake = appkit_shell::ProductWakeHandle::new(event_loop_proxy.clone());
textora_app::install_macos_open_document_handler(
    product_wake,
    app.open_document_sender(),
)
```

Keep the public `AppEvent` re-export in `lib.rs` for migration compatibility.

- [ ] **Step 4: Verify final Task 12 boundary**

Run:

```bash
rg -n 'RecentFilesLoaded|SyncResultsReady|OpenFiles\\(' crates/app/src crates/appkit-shell/src
cargo test -p textora-appkit-shell event
cargo test -p textora-appkit-shell product_host
cargo test -p textora-app --lib app_event
cargo test -p textora-app --lib macos_open_documents
cargo test -p textora-app --lib app_lifecycle
cargo test -p textora-app --lib sync_controller
cargo test -p textora-app --lib textora_product
cargo check -p textora-appkit-shell
cargo check -p textora-app
cargo fmt --all -- --check
```

Expected: `rg` has no output; all commands PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/main.rs crates/app/src/macos_open_documents.rs
git commit -m "refactor(macos): use typed product wake handle"
```

---

### Task 12.11: Task-wide verification

**Files:**
- No production file changes
- Modify only the ignored SDD progress ledger after review

- [ ] Run:

```bash
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
cargo check -p textora-appkit-core
cargo check -p textora-appkit-shell
cargo check -p textora-app
cargo fmt --all -- --check
```

- [ ] Inspect dependency boundaries:

```bash
cargo tree -p textora-appkit-shell
rg -n 'textora_sync|RecentFilesLoaded|SyncResultsReady|OpenFiles\\(' crates/appkit-shell crates/app/src/app_event.rs
```

Expected: no forbidden dependency or product payload in shell events.

- [ ] Dispatch a whole-Task-12 code review using the diff from the Task 11 completion commit through Task 12 HEAD.

- [ ] Fix every Critical/Important finding with focused tests and re-review.

- [ ] Mark Task 12 complete in `.superpowers/sdd/progress.md` only after review approval.
