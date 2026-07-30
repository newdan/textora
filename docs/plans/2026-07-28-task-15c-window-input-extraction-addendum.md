# Task 15C Window Input Extraction Addendum

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不把 textora widget 组合或产品动作带入共享层的前提下，将 Task 15C 涉及的 winit 键盘、IME、修饰键与滚轮归一化迁入 `textora-appkit-shell`。

**Architecture:** 原计划只列出 `events.rs`，但滚轮与 `KeyCode` 归一化实际位于 `app_lifecycle.rs`。为遵守每子任务最多修改 3 个文件的硬约束，本 addendum 将 15C 拆成 15C1 和 15C2；共享层只返回纯判断、`ui::Modifiers`、`ui::KeyCode` 或像素 delta，app 继续负责 widget 路由与 `AppAction` 翻译。

**Tech Stack:** Rust 2024、winit 0.30、textora-ui。

## Global Constraints

- `appkit-shell` 禁止依赖 `textora-markdown`、`textora-sync`、`textora-app`。
- 每个实现子任务最多修改 3 个文件。
- 所有行为变更先写失败测试。
- textora widget 组合、focus 路由和 `AppAction` 翻译继续留在 `crates/app`。
- 每次提交前运行 `cargo fmt --all -- --check` 和相关 crate 编译。

---

### Task 15C1: Extract keyboard and IME guards

**Files:**
- Create: `crates/appkit-shell/src/window_input.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/app/src/events.rs`

**Interfaces:**
- Produces:
  - `pub fn is_ime_process_key(logical_key: &winit::keyboard::Key) -> bool`
  - `pub fn ui_modifiers(state: winit::keyboard::ModifiersState) -> ui::core::widget::Modifiers`
  - `pub fn command_allowed_during_preedit(preedit_text: &str, command: &appkit_core::edit_command::EditCommand) -> bool`
- Consumes: `appkit_core::edit_command::EditCommand`, winit keyboard types and UI `Modifiers`.

- [ ] **Step 1: Add failing shell tests**

Add tests proving:

```rust
assert!(is_ime_process_key(&Key::Named(NamedKey::Process)));
assert!(!is_ime_process_key(&Key::Named(NamedKey::Enter)));
assert!(!command_allowed_during_preedit("拼", &EditCommand::InsertChar("a".into())));
assert!(command_allowed_during_preedit("拼", &EditCommand::MoveLeft));
assert!(command_allowed_during_preedit("", &EditCommand::InsertChar("a".into())));
```

Build a `ModifiersState` containing SHIFT, SUPER, ALT and CONTROL and assert the four corresponding UI modifier fields are true.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p textora-appkit-shell window_input
```

Expected: FAIL because the tested normalization functions do not exist.

- [ ] **Step 3: Implement the pure helpers**

Implement the three interfaces exactly. `command_allowed_during_preedit` must reject only `EditCommand::InsertChar(_)` while `preedit_text` is non-empty.

- [ ] **Step 4: Route app keyboard handling through the helpers**

In `events.rs`, replace the direct `NamedKey::Process` comparison, inline modifier construction and preedit `InsertChar` match with the three helpers. Keep plugin intent mapping, reading-mode translation and `AppAction` construction local.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test -p textora-appkit-shell window_input
cargo test -p textora-app --lib events
cargo check -p textora-app
cargo fmt --all -- --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/appkit-shell/src/window_input.rs crates/appkit-shell/src/lib.rs crates/app/src/events.rs
git commit -m "refactor(appkit-shell): extract keyboard input guards"
```

---

### Task 15C2: Extract key-code and scroll normalization

**Files:**
- Modify: `crates/appkit-shell/src/window_input.rs`
- Modify: `crates/app/src/app_lifecycle.rs`

**Interfaces:**
- Produces:
  - `pub fn winit_key_to_keycode(logical_key: &winit::keyboard::Key, text: Option<&str>) -> Option<ui::core::widget::KeyCode>`
  - `pub fn scroll_delta_pixels(delta: &winit::event::MouseScrollDelta, line_height: f32) -> (f32, f32)`
- Consumes: winit key and scroll event types.

- [ ] **Step 1: Add failing shell tests**

Add tests that preserve the existing key conversion behavior for text, control-character fallback and named keys. Add scroll tests:

```rust
assert_eq!(
    scroll_delta_pixels(&MouseScrollDelta::LineDelta(1.0, -2.0), 10.0),
    (30.0, -60.0),
);
assert_eq!(
    scroll_delta_pixels(
        &MouseScrollDelta::PixelDelta(PhysicalPosition::new(4.5, -7.25)),
        10.0,
    ),
    (4.5, -7.25),
);
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p textora-appkit-shell window_input
```

Expected: FAIL because the two new functions do not exist.

- [ ] **Step 3: Move the normalizers**

Move `winit_key_to_keycode` unchanged from `app_lifecycle.rs`. Move the `MouseScrollDelta` match and the factor `3.0` from `modal_wheel_event` into `scroll_delta_pixels`; keep construction of `ui::core::Event::Wheel` in app.

- [ ] **Step 4: Replace lifecycle call sites**

Import the shared helpers in `app_lifecycle.rs`. Use `ui_modifiers` for the existing modal/search/panel modifier conversions, call the shared key-code converter, and have `modal_wheel_event` construct the widget event from `scroll_delta_pixels`.

- [ ] **Step 5: Verify GREEN**

Run:

```bash
cargo test -p textora-appkit-shell window_input
cargo test -p textora-app --lib app_lifecycle
cargo check -p textora-app
cargo fmt --all -- --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/appkit-shell/src/window_input.rs crates/app/src/app_lifecycle.rs
git commit -m "refactor(appkit-shell): normalize lifecycle input"
```

---

## Completion Gate

- `NamedKey::Process`, preedit suppression, modifier conversion, key-code conversion and modal scroll normalization are covered by `appkit-shell` tests.
- `events.rs` retains textora plugin/widget/action composition but no longer implements the extracted normalization rules.
- `app_lifecycle.rs` retains lifecycle and widget routing but no longer implements key-code or scroll-delta conversion.
- `cargo test -p textora-appkit-shell window_input` and `cargo check -p textora-app` pass.
