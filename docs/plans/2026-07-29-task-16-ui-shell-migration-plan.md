# Task 16 UiShell Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 清除 `UiShell` 中最后的 textora 产品语义，封装 app 对 shell 私有状态的直访，并把通用 dock/overlay/focus/layout runtime 迁入 `appkit-shell`。

**Architecture:** 产品 overlay 的具体 downcast 与 `SyncSettingsAction` 提取留在 `textora-app::settings_overlay`；`UiShell` 只保留泛型 widget/overlay API。迁移前先用语义化方法替代 app 对 sidebar、frame 和 dock 状态字段的直接访问，物理移动时字段保持私有，只公开真正跨 crate 使用的方法。

**Tech Stack:** Rust、Cargo workspace、`ui::Widget`/Dock/Overlay、`appkit-shell`、winit 输入适配。

## Global Constraints

- 全程保持单一 `textora` 二进制；不得新增兼容二进制或入口。
- `appkit-shell` 禁止依赖 `textora-app`、`textora-markdown`、`textora-sync`，禁止出现 `TextoraSettingsOverlay`、`SyncSettingsAction` 或 `NativeMenu` 产品类型。
- UI 跨层仍只接收纯数据/widget trait；绝对禁止让 `ui` 依赖 app 状态。
- 每个实现任务最多修改 3 个逻辑文件；Git move 的源和目标算一个逻辑文件。
- 不允许把 `UiShell` 所有字段改成 `pub`；sidebar、frame counter、dock dirty 和 overlay 存储必须保持私有。
- 纯移动前后运行同组测试；行为调整遵循 RED-GREEN。
- 每次提交前运行 `cargo fmt --all -- --check`、相关测试、`cargo check -p textora-app` 和 `bash scripts/check_architecture.sh`。
- 不引入 `Deref`、`Box<dyn Any>` 状态袋或 shell → app 反向依赖。
- 本计划不创建 `ShellRuntime`，也不拆分 `Workspace`；完成后另写 `PreparedTab`/Workspace 计划。

---

### Task 1: Move product overlay action extraction into app

**Files:**

- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/settings_overlay.rs`
- Modify: `crates/app/src/events.rs`

**Interfaces:**

- Consumes: `UiShell::active_overlay_widget_mut<T: Any>() -> Option<&mut T>`。
- Produces: `App::take_pending_sync_settings_action(&mut self) -> Option<SyncSettingsAction>`。

- [ ] **Step 1: Record the current positive and negative behavior**

Run:

```bash
cargo test -p textora-app --lib ui_shell::tests::overlay_modal_tests
cargo test -p textora-app --lib sync_settings_action_is_extracted_after_product_overlay_dispatch
```

Expected: shell modal tests and the end-to-end product action extraction test pass.

- [ ] **Step 2: Add the app-owned action extractor**

Add this method inside the existing `impl App` in `settings_overlay.rs`:

```rust
pub(crate) fn take_pending_sync_settings_action(
    &mut self,
) -> Option<crate::sync_settings_types::SyncSettingsAction> {
    self.ui_shell
        .active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()?
        .content_as_any_mut()
        .downcast_mut::<crate::textora_settings_overlay::TextoraSettingsOverlay>()?
        .take_pending_sync_action()
}
```

Add a negative test in `settings_overlay.rs`:

```rust
#[test]
fn pending_sync_action_requires_an_active_textora_settings_modal() {
    let mut app = App::new(None);
    assert_eq!(app.take_pending_sync_settings_action(), None);

    let generic_settings_input = app.settings_view_input();
    app.ui_shell.push_overlay_with_policy(
        Box::new(ui::modal_frame::ModalFrame::new(
            "设置",
            Box::new(ui::settings_view::SettingsView::new(generic_settings_input)),
        )),
        ui::OverlayLayout::Fixed(ui::Rect::new(0.0, 0.0, 720.0, 560.0)),
        ui::OverlayInputPolicy::Modal,
        ui::DismissPolicy::ExplicitOnly,
    );

    assert_eq!(app.take_pending_sync_settings_action(), None);
}
```

- [ ] **Step 3: Route events through the app-owned extractor**

In `dispatch_mouse`, replace:

```rust
app.ui_shell.take_pending_sync_settings_action()
```

with:

```rust
app.take_pending_sync_settings_action()
```

Update the final assertion in
`sync_settings_action_is_extracted_after_product_overlay_dispatch` the same
way. Keep the positive click-through test in `events.rs`; it is the product
integration coverage.

- [ ] **Step 4: Remove product knowledge from UiShell**

Delete `UiShell::take_pending_sync_settings_action`.

From the `ui_shell.rs` test module, delete:

- `SyncSettingsInput` and `TextoraSettingsOverlay` imports;
- `textora_settings_input`;
- `shell_with_textora_settings_modal`;
- `labeled_control_center`;
- `click_modal_control`;
- `active_textora_settings_modal_takes_pending_sync_action_once`;
- `taking_sync_action_without_an_active_textora_settings_modal_returns_none`.

Keep all generic modal input, focus restore, dock, overlay, tooltip and layout
tests.

Add a source boundary test without embedding complete forbidden identifiers in
its own source:

```rust
#[test]
fn ui_shell_source_has_no_product_settings_types() {
    let production_source = include_str!("ui_shell.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("UiShell production source must precede tests");
    let forbidden = [
        ["Textora", "Settings"].concat(),
        ["Sync", "Settings", "Action"].concat(),
        ["Native", "Menu"].concat(),
    ];

    for product_type in forbidden {
        assert!(
            !production_source.contains(&product_type),
            "UiShell must not depend on product type {product_type}"
        );
    }
}
```

- [ ] **Step 5: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib settings_overlay::tests
cargo test -p textora-app --lib sync_settings_action_is_extracted_after_product_overlay_dispatch
cargo test -p textora-app --lib ui_shell::tests
cargo check -p textora-app
bash scripts/check_architecture.sh
```

Expected: all pass; `rg -n "TextoraSettings|SyncSettingsAction|NativeMenu" crates/app/src/ui_shell.rs`
has no production match.

Commit:

```bash
git add crates/app/src/ui_shell.rs crates/app/src/settings_overlay.rs crates/app/src/events.rs
git commit -m "refactor(app): keep product overlay actions outside ui shell"
```

---

### Task 2: Introduce semantic UiShell state methods

**Files:**

- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/app/src/app_lifecycle.rs`

**Interfaces:**

- Consumes: existing private `sidebar_config`, `sidebar_persistent`,
  `frames_rendered`, and `dock_dirty` fields.
- Produces:
  - `set_sidebar_width(&mut self, width: f32)`
  - `scale_sidebar_width(&mut self, ratio: f32)`
  - `set_sidebar_visibility(&mut self, visibility: ui::sidebar::Visibility)`
  - `sidebar_visibility(&self) -> ui::sidebar::Visibility`
  - `set_sidebar_suppress_hover_enter(&mut self, suppress: bool)`
  - `sidebar_settings_button_rect(&self) -> Rect`
  - `dock_is_dirty(&self) -> bool`
  - `mark_dock_dirty(&mut self)`
  - `mark_layout_initialized_for_test(&mut self)`

- [ ] **Step 1: Add failing accessor behavior tests**

Add tests in `ui_shell.rs`:

```rust
#[test]
fn semantic_state_methods_update_private_shell_state() {
    let mut shell = UiShell::new();

    shell.set_sidebar_width(240.0);
    shell.scale_sidebar_width(1.5);
    shell.set_sidebar_visibility(ui::sidebar::Visibility::Pinned);
    shell.set_sidebar_suppress_hover_enter(true);
    shell.dock_dirty = false;
    shell.mark_dock_dirty();
    shell.mark_layout_initialized_for_test();

    assert_eq!(shell.sidebar_width(), 360.0);
    assert_eq!(shell.sidebar_visibility(), ui::sidebar::Visibility::Pinned);
    assert!(shell.sidebar_persistent.suppress_hover_enter);
    assert!(shell.dock_is_dirty());
    assert_eq!(shell.frames_rendered, 1);
}
```

Run:

```bash
cargo test -p textora-app --lib semantic_state_methods_update_private_shell_state
```

Expected: FAIL because the semantic methods do not exist.

- [ ] **Step 2: Implement the semantic methods**

Add:

```rust
pub fn set_sidebar_width(&mut self, width: f32) {
    self.sidebar_config.width = width;
}

pub fn scale_sidebar_width(&mut self, ratio: f32) {
    self.sidebar_config.width *= ratio;
}

pub fn set_sidebar_visibility(&mut self, visibility: ui::sidebar::Visibility) {
    self.sidebar_persistent.visibility = visibility;
}

pub fn sidebar_visibility(&self) -> ui::sidebar::Visibility {
    self.sidebar_persistent.visibility
}

pub fn set_sidebar_suppress_hover_enter(&mut self, suppress: bool) {
    self.sidebar_persistent.suppress_hover_enter = suppress;
}

pub fn sidebar_settings_button_rect(&self) -> Rect {
    self.sidebar_persistent.settings_btn_rect
}

pub fn dock_is_dirty(&self) -> bool {
    self.dock_dirty
}

pub fn mark_dock_dirty(&mut self) {
    self.dock_dirty = true;
}

#[doc(hidden)]
pub fn mark_layout_initialized_for_test(&mut self) {
    self.frames_rendered = 1;
}
```

Do not add getters returning `&mut SidebarPersistent` or expose internal
overlay/dock collections.

- [ ] **Step 3: Migrate app initialization and lifecycle**

In `app_init.rs`, replace persisted width assignment with:

```rust
app.ui_shell.set_sidebar_width(persisted.sidebar_width);
```

In `app_lifecycle.rs`, replace width multiplication with:

```rust
self.ui_shell.scale_sidebar_width(ratio);
```

Replace test-only `frames_rendered = 1` writes with:

```rust
app.ui_shell.mark_layout_initialized_for_test();
```

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib semantic_state_methods_update_private_shell_state
cargo test -p textora-app --lib app_lifecycle
cargo check -p textora-app
bash scripts/check_architecture.sh
```

Commit:

```bash
git add crates/app/src/ui_shell.rs crates/app/src/app_init.rs crates/app/src/app_lifecycle.rs
git commit -m "refactor(app): encapsulate ui shell runtime state"
```

---

### Task 3: Migrate app_window off UiShell fields

**Files:**

- Modify: `crates/app/src/app_window.rs`

**Interfaces:**

- Consumes: Task 2 semantic state methods plus existing `set_sidebar_pinned`,
  `sidebar_current_width`, and `sidebar_clamp_width`.
- Produces: no new interface; removes all `ui_shell.sidebar_config` and
  `ui_shell.sidebar_persistent` direct access from `app_window.rs`.

- [ ] **Step 1: Replace production field access**

Use:

```rust
self.ui_shell.scale_sidebar_width(self.scale_factor as f32);
```

for scale changes.

Use these exact methods for state setup:

```rust
app.ui_shell.set_sidebar_pinned(true);
app.ui_shell.set_sidebar_width(220.0);
app.ui_shell.set_sidebar_visibility(ui::sidebar::Visibility::Pinned);
```

For hidden/hover-peek cases, use `set_sidebar_pinned(false)` followed by
`set_sidebar_visibility(...)`. Replace:

```rust
app.ui_shell
    .sidebar_persistent
    .current_width(&app.ui_shell.sidebar_config)
```

with:

```rust
app.ui_shell.sidebar_current_width()
```

- [ ] **Step 2: Replace test fixtures**

Apply the same semantic setters in `sidebar_hover_peek_tests`,
`app_window_tests`, and any other test module in this file. Do not weaken
assertions about pinned, hidden or hover-peek geometry.

- [ ] **Step 3: Verify the file has no direct field access**

Run:

```bash
rg -n "ui_shell\\.(sidebar_config|sidebar_persistent)" crates/app/src/app_window.rs
```

Expected: no matches.

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib app_window
cargo check -p textora-app
bash scripts/check_architecture.sh
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/app_window.rs
git commit -m "refactor(app): use ui shell sidebar methods"
```

---

### Task 4: Migrate renderer and dispatch geometry off UiShell fields

**Files:**

- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Interfaces:**

- Consumes: `UiShell::sidebar_cfg()`, `dock_is_dirty()`, and
  `mark_dock_dirty()`.
- Produces: renderer and geometry projection no longer access shell private
  fields.

- [ ] **Step 1: Migrate app_dispatch**

Replace:

```rust
!self.ui_shell.dock_dirty
```

with:

```rust
!self.ui_shell.dock_is_dirty()
```

Keep the cached editor-rect fast path unchanged.

- [ ] **Step 2: Migrate app_renderer**

Replace:

```rust
self.ui_shell.sidebar_config.clone()
```

with:

```rust
self.ui_shell.sidebar_cfg().clone()
```

Replace dirty reads and writes with:

```rust
self.ui_shell.dock_is_dirty()
self.ui_shell.mark_dock_dirty()
```

Do not change redraw conditions or layout ordering.

- [ ] **Step 3: Verify and commit**

Run:

```bash
rg -n "ui_shell\\.(sidebar_config|dock_dirty)" crates/app/src/app_renderer.rs crates/app/src/app_dispatch.rs
cargo fmt --all -- --check
cargo test -p textora-app --lib app_renderer
cargo test -p textora-app --lib app_dispatch
cargo check -p textora-app
bash scripts/check_architecture.sh
```

Expected: source scan has no matches and all checks pass.

Commit:

```bash
git add crates/app/src/app_renderer.rs crates/app/src/app_dispatch.rs
git commit -m "refactor(app): use ui shell layout methods"
```

---

### Task 5: Migrate remaining chrome and test fixtures off UiShell fields

**Files:**

- Modify: `crates/app/src/dispatch/chrome.rs`
- Modify: `crates/app/src/app_reshape.rs`
- Modify: `crates/app/src/events.rs`

**Interfaces:**

- Consumes: Task 2 semantic state methods and existing
  `sidebar_set_open_menu`.
- Produces: no external app file directly accesses private UiShell fields.

- [ ] **Step 1: Migrate chrome dispatch**

Replace direct sidebar persistent mutations with:

```rust
self.ui_shell.set_sidebar_visibility(if pinned {
    ui::sidebar::Visibility::Pinned
} else {
    ui::sidebar::Visibility::Hidden
});
if !pinned {
    self.ui_shell.set_sidebar_suppress_hover_enter(true);
}
```

Read the settings button with:

```rust
let button = self.ui_shell.sidebar_settings_button_rect();
```

Build the menu as before, then store it with:

```rust
self.ui_shell.sidebar_set_open_menu(menu);
```

- [ ] **Step 2: Migrate cross-crate test setup**

In `app_reshape.rs` and `events.rs`, replace all:

```rust
app.ui_shell.frames_rendered = 1;
```

with:

```rust
app.ui_shell.mark_layout_initialized_for_test();
```

Do not alter event routing or reshape assertions.

- [ ] **Step 3: Enforce zero direct field access outside UiShell**

Run:

```bash
rg -n "ui_shell\\.(sidebar_config|sidebar_persistent|frames_rendered|dock_dirty)" \
  crates/app/src \
  --glob '*.rs' \
  --glob '!ui_shell.rs'
```

Expected: no matches.

- [ ] **Step 4: Verify and commit**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib dispatch::chrome
cargo test -p textora-app --lib app_reshape
cargo test -p textora-app --lib events
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

Commit:

```bash
git add crates/app/src/dispatch/chrome.rs crates/app/src/app_reshape.rs crates/app/src/events.rs
git commit -m "refactor(app): close ui shell field boundary"
```

---

### Task 6: Move UiShell into appkit-shell

**Files:**

- Move: `crates/app/src/ui_shell.rs` → `crates/appkit-shell/src/ui_shell.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**

- Consumes: product-free `UiShell`, semantic state methods, shell-local
  `editor_host` and `measure_adapter`.
- Produces: `appkit_shell::ui_shell::{UiShell, ShellInputs, KeyboardFocusTarget}`。

- [ ] **Step 1: Record the complete pre-move behavior**

Run:

```bash
cargo test -p textora-app --lib ui_shell::tests
cargo check -p textora-app --tests
```

Expected: generic dock/overlay/focus/layout tests pass and test targets compile
without warnings.

- [ ] **Step 2: Move the module**

Run:

```bash
git mv crates/app/src/ui_shell.rs crates/appkit-shell/src/ui_shell.rs
```

The production imports must resolve only to std, `ui`, `shaping`, and
shell-local modules:

```rust
use crate::editor_host::EditorHostWidget;
```

`crate::measure_adapter::MeasureFromShaper` remains shell-local after the
move.

- [ ] **Step 3: Set the cross-crate visibility boundary**

Keep these internal:

- `OverlayChild`
- `OverlayEntry`
- `TooltipTimer`
- `OverlayDispatchOutcome`
- all `UiShell` fields
- widget-input builders and internal dispatch/layout helpers

Make app-consumed methods public, including:

- `mindmap_style_panel_thickness`
- `search_bar_has_keyboard_focus`
- `search_ime_cursor_rect`
- `search_bar_x_offset`
- `sync_sidebar_persistent`
- tab-bar layout/hover/scroll methods
- `compute_autoscroll_target`
- `active_overlay_is_modal`
- `active_overlay_widget_ref`
- `active_overlay_layout_rect`
- `forward_ime`

Methods already declared `pub` remain public. Do not mechanically turn every
`pub(crate)` item into `pub`; use `cargo check -p textora-app --tests` to catch
only real cross-crate consumers.

- [ ] **Step 4: Export shell module and preserve app path**

In `appkit-shell/src/lib.rs` add:

```rust
pub mod ui_shell;
```

In app `lib.rs`, replace:

```rust
mod ui_shell;
```

with:

```rust
pub(crate) use appkit_shell::ui_shell;
```

- [ ] **Step 5: Verify the moved tests and dependency boundary**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell ui_shell::tests
cargo check -p textora-app --tests
cargo test -p textora-app --lib
bash scripts/check_architecture.sh
rg -n "textora_sync|TextoraSettings|SyncSettingsAction|NativeMenu|textora_markdown|crate::app" \
  crates/appkit-shell/src/ui_shell.rs
```

Expected:

- all tests/checks pass without warnings;
- product scan has no matches;
- the move plus visibility changes are the only implementation differences.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/ui_shell.rs crates/appkit-shell/src/ui_shell.rs crates/appkit-shell/src/lib.rs crates/app/src/lib.rs
git commit -m "refactor(appkit-shell): move ui shell runtime"
```

---

### Task 7: Verify the UiShell migration stage

**Files:** No implementation files.

**Interfaces:**

- Consumes: `appkit_shell::ui_shell` and app product action adapter.
- Produces: a verified boundary ready for the separate Workspace/PreparedTab
  plan.

- [ ] **Step 1: Verify unique ownership**

Run:

```bash
rg -n "struct UiShell|struct ShellInputs|enum KeyboardFocusTarget" crates/app crates/appkit-shell
```

Expected: definitions exist only in `crates/appkit-shell/src/ui_shell.rs`.

- [ ] **Step 2: Verify product isolation and field closure**

Run:

```bash
rg -n "textora_sync|TextoraSettings|SyncSettingsAction|NativeMenu|textora_markdown" crates/appkit-shell
rg -n "ui_shell\\.(sidebar_config|sidebar_persistent|frames_rendered|dock_dirty)" \
  crates/app/src \
  --glob '*.rs'
```

Expected: both commands have no matches.

- [ ] **Step 3: Run stage verification**

Run:

```bash
cargo fmt --all -- --check
bash scripts/check_architecture.sh
cargo check --workspace
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
```

Expected: all pass without warnings.

- [ ] **Step 4: Prepare the next isolated plan**

After this stage is reviewed, write a separate Workspace plan that:

1. introduces `PreparedTab { document: DocumentModel, runtime: TabRuntime }`;
2. separates textora file/plugin construction from the generic workspace
   controller;
3. moves generic workspace navigation/runtime ownership into shell;
4. does not create `ShellRuntime` until workspace ownership is closed.
