# Task 16 UiShell 焦点字段边界补充实施计划

> **执行要求：** 使用 `subagent-driven-development`，每个任务由新的实现代理完成，
> 再由新的审查代理核对规格和差异。

## 背景

UiShell 迁移计划 Task 6 要求移动后所有字段保持私有，但迁移前验证发现
`UiShell::keyboard_focus` 仍是公开字段，并被 app 的 6 个文件直接消费 34 次。
原 Task 6 只允许移动模块和修改两个 crate root，无法同时迁移这些调用点。

本补充计划先用最小语义 API 关闭焦点字段边界，再原样恢复执行 Task 6。它不引入
可变引用 getter，不增加产品语义，也不扩大 `UiShell` 的状态职责。

## 目标 API

在 `UiShell` 上增加：

```rust
pub fn keyboard_focus(&self) -> KeyboardFocusTarget {
    self.keyboard_focus
}

pub fn focus_editor(&mut self) {
    self.keyboard_focus = KeyboardFocusTarget::Editor;
}

pub fn focus_widget(&mut self, widget_id: ui::core::widget::WidgetId) {
    self.keyboard_focus = KeyboardFocusTarget::Widget(widget_id);
}
```

搜索专用判断优先复用已有的 `search_bar_has_keyboard_focus()`。只有需要复制完整
焦点目标或区分通用 widget 的调用点才使用 `keyboard_focus()`。

---

### Task 1：增加焦点 API 并迁移生命周期与渲染

**文件：**

- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app_renderer.rs`

**步骤：**

1. 在 `ui_shell.rs` 增加焦点 getter 与两个语义 transition 方法，并增加
   `semantic_focus_methods_update_shell_state` 单元测试。
2. `app_lifecycle.rs`：
   - mindmap style panel 等精确目标判断改用 `keyboard_focus()`；
   - search-only 判断改用 `search_bar_has_keyboard_focus()`；
   - 测试夹具用 `focus_editor()` / `focus_widget(...)`。
3. `app_renderer.rs` 的两个 search-only 判断改用
   `search_bar_has_keyboard_focus()`。
4. 此任务暂不私有化字段，因为后续两个任务的消费者尚未迁移。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-app --lib semantic_focus_methods_update_shell_state
cargo test -p textora-app --lib app_lifecycle
cargo test -p textora-app --lib app_renderer
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

**提交：**

```text
refactor(app): add ui shell focus transitions
```

---

### Task 2：迁移搜索与编辑 dispatch

**文件：**

- Modify: `crates/app/src/app_search.rs`
- Modify: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/dispatch/editor.rs`

**步骤：**

1. 搜索关闭/取消路径用 `focus_editor()`。
2. `ToggleFind` 路径用 `focus_widget(SEARCH_BAR)`。
3. editor dispatch 的焦点复位/设置使用 transition 方法；需要在可变借用 tab
   之前保存完整目标时使用 `keyboard_focus()`，保持原借用顺序。
4. 不改变 `should_route_edit_command_to_search` 的纯函数接口和动作路由。

**验证：**

```bash
rg -n "ui_shell\\.keyboard_focus" \
  crates/app/src/app_search.rs \
  crates/app/src/dispatch/commands.rs \
  crates/app/src/dispatch/editor.rs
cargo fmt --all -- --check
cargo test -p textora-app --lib app_search
cargo test -p textora-app --lib dispatch::commands
cargo test -p textora-app --lib dispatch::editor
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

字段扫描预期无匹配。

**提交：**

```text
refactor(app): use ui shell focus transitions
```

---

### Task 3：迁移事件并私有化焦点字段

**文件：**

- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/ui_shell.rs`

**步骤：**

1. events 生产路径：
   - shell dispatch 返回的 widget id 用 `focus_widget(id)`；
   - 左键编辑器命中的目标判断用 `keyboard_focus()`；
   - 焦点复位用 `focus_editor()`。
2. events 测试夹具和断言全部改用语义方法。
3. 将 `UiShell::keyboard_focus` 从 `pub` 改为私有字段。
4. 保留 `KeyboardFocusTarget` 为公开类型；不增加 `&mut` getter。

**验证：**

```bash
rg -n --pcre2 "ui_shell\\.keyboard_focus(?!\\s*\\()" \
  crates/app/src --glob '*.rs' --glob '!ui_shell.rs'
rg -n "pub(\\(crate\\))? keyboard_focus:" crates/app/src/ui_shell.rs
cargo fmt --all -- --check
cargo test -p textora-app --lib events
cargo test -p textora-app --lib ui_shell::tests
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

两项字段扫描预期均无匹配。

**提交：**

```text
refactor(app): privatize ui shell focus state
```

---

### Task 4：迁移遗漏的 app_window 测试夹具

Task 6 迁移前编译发现 `app_window.rs` 的测试通过局部变量
`shell.frames_rendered = 1` 直接写字段，未被此前只扫描 `ui_shell.` 前缀的命令
覆盖。

**文件：**

- Modify: `crates/app/src/app_window.rs`

**步骤：**

1. 将 `ui_shell_alignment_tests::run` 中的直接写入替换为
   `shell.mark_layout_initialized_for_test()`。
2. 不修改几何输入和断言。

**验证：**

```bash
rg -n "\\.frames_rendered\\s*=" crates/app/src --glob '*.rs' --glob '!ui_shell.rs'
cargo fmt --all -- --check
cargo test -p textora-app --lib app_window
cargo check -p textora-app --tests
bash scripts/check_architecture.sh
```

字段扫描预期无匹配。

**提交：**

```text
refactor(app): close ui shell frame test boundary
```

---

### Task 5：声明 shell 剪贴板依赖

物理移动后，UiShell 的通用剪贴板事件处理仍使用 `arboard::Clipboard`。根
workspace 已声明该依赖，但 `appkit-shell` 必须显式消费，避免把 manifest
修改混入三逻辑文件的移动任务。

**文件：**

- Modify: `crates/appkit-shell/Cargo.toml`

**步骤：**

1. 在 `[dependencies]` 增加 `arboard.workspace = true`。
2. 不移动代码、不增加产品依赖。

**验证：**

```bash
cargo fmt --all -- --check
cargo check -p textora-appkit-shell
bash scripts/check_architecture.sh
```

**提交：**

```text
build(appkit-shell): declare clipboard dependency
```

---

### Task 6：恢复原 UiShell Task 6

完成 Task 1–5 并通过独立审查后，重新执行
`docs/plans/2026-07-29-task-16-ui-shell-migration-plan.md` 的 Task 6。

迁移后的硬性检查保持不变：

- 所有 `UiShell` 字段为私有；
- app 仅通过方法消费焦点状态；
- `OverlayChild`、`OverlayEntry`、`TooltipTimer`、
  `OverlayDispatchOutcome` 保持内部；
- shell 产品关键字扫描为零；
- `cargo check -p textora-app --tests` 与 app/shell 测试通过。

---

### Task 7：修复 UiShell 移动审查中的公共边界

Task 6 正式审查确认移动与行为保持正确，但发现两个公共边界问题：

1. `ui_shell.rs` 直接构造 `arboard::Clipboard`，不满足该模块只依赖
   std、ui、shaping 与 shell-local 模块的约束；
2. `compute_autoscroll_target` 在 app 中没有消费者，不应提升为公共 API。

**文件：**

- Add: `crates/appkit-shell/src/clipboard.rs`
- Modify: `crates/appkit-shell/src/lib.rs`
- Modify: `crates/appkit-shell/src/ui_shell.rs`

**步骤：**

1. 在私有 `clipboard` 模块中封装保持原语义的
   `write_text(String)` / `read_text() -> String`：
   - 每次调用仍新建 `arboard::Clipboard`；
   - 写入失败继续忽略；
   - 读取或初始化失败继续返回空字符串。
2. `ui_shell.rs` 的搜索栏回调只调用 shell-local clipboard 函数，不再直接
   引用 `arboard`。
3. 将 `compute_autoscroll_target` 收紧为私有方法；同模块测试继续覆盖它。
4. 不改变 manifest、回调签名、tab scroll 算法或 app 调用点。

**验证：**

```bash
rg -n "arboard::|pub fn compute_autoscroll_target" \
  crates/appkit-shell/src/ui_shell.rs
cargo fmt --all -- --check
cargo test -p textora-appkit-shell ui_shell::tests
cargo check -p textora-app --tests
cargo test -p textora-app --lib
bash scripts/check_architecture.sh
```

源码扫描预期无匹配。

**提交：**

```text
refactor(appkit-shell): isolate ui shell clipboard
```

---

### Task 8：修复 UiShell 产品隔离测试截断

整阶段最终审查发现产品隔离测试按第一个 `#[cfg(test)]` 分割源码，而生产
`impl UiShell` 中已有更早的测试专用方法，导致后半段生产代码未被扫描。

**文件：**

- Modify: `crates/appkit-shell/src/ui_shell.rs`

**步骤：**

1. 按唯一的 `#[cfg(test)]\nmod tests {` 标记截取完整生产源码。
2. 增加哨兵断言，确认截取文本包含后半段的 `pub fn dispatch(`；若标记或结构
   变化导致过早截断，测试必须失败。
3. 在 `semantic_state_methods_update_private_shell_state` 中直接设置私有
   `settings_btn_rect` 并断言 `sidebar_settings_button_rect()` 返回同一矩形，
   关闭阶段旧 Minor。
4. 不改变生产实现或公共 API。

**验证：**

```bash
cargo fmt --all -- --check
cargo test -p textora-appkit-shell ui_shell_source_has_no_product_settings_types
cargo test -p textora-appkit-shell semantic_state_methods_update_private_shell_state
cargo test -p textora-appkit-shell
cargo check --workspace
bash scripts/check_architecture.sh
```

**提交：**

```text
test(appkit-shell): cover complete ui shell boundary
```
