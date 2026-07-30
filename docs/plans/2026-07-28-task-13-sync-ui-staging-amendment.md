# Task 13 Sync 设置 UI 过渡修订实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不依赖 `ui::settings_view::SettingsViewAction::Sync` 的前提下，将 Sync 设置页面的可复用 UI 状态与动作类型复制到 textora 产品层。

**Architecture:** `SyncSettingsPage` 留在产品层并维护一个私有 `Option<SyncSettingsAction>`。控件激活时记录该动作、返回 `WidgetAction::Consumed`；Task 13C 从页面取出动作并映射到 `TextoraAction::Sync`。通用 UI 不接收任何 textora payload。

**Tech Stack:** Rust 2024、`textora-app`、`textora-ui` 公共 widget API。

## Global Constraints

- 只保留 `textora` binary，不改变同步业务逻辑或持久化格式。
- `appkit-core`、`appkit-shell` 与 `textora-ui` 不新增 textora sync 依赖。
- 本任务最多修改 3 个源文件：两个新 app 模块和 `crates/app/src/lib.rs`。
- 不扩展 `ui::WidgetAction`，不使用 `Any`、字符串动作名或全局回调。
- 先运行旧 UI 同步页面测试作为表征基线；新行为先写失败测试，再写最小实现。
- 提交前运行 `cargo fmt --all -- --check`、`cargo check -p textora-app` 与相关测试。

---

### Task 13A-Stage: 复制产品 Sync 页面并隔离动作

**Files:**
- Create: `crates/app/src/sync_settings_types.rs`
- Create: `crates/app/src/sync_settings_page.rs`
- Modify: `crates/app/src/lib.rs`

**Consumes:** `ui` 根级公开的 `core`、`theme`、`form`、`text_box`、`button`、`label` 与 `inline_group` 模块；现有 `crates/ui/src/widgets/settings_view/sync_types.rs` 和 `sync_page.rs` 的行为测试。

**Produces:**
- `crate::sync_settings_types::{SyncSettingsAction, SyncSettingsInput, ...}`
- `crate::sync_settings_page::SyncSettingsPage`
- `SyncSettingsPage::take_pending_action(&mut self) -> Option<SyncSettingsAction>`

- [ ] **Step 1: 记录表征基线**

运行：

```bash
cargo test -p textora-ui sync_settings
```

预期：现有 13 个 Sync 页面/类型测试通过。

- [ ] **Step 2: 写入产品动作边界测试（RED）**

在新的 `sync_settings_page.rs` 测试模块中，先写下列行为：

```rust
#[test]
fn configure_activation_is_consumed_and_can_be_taken_as_a_product_action() {
    let mut page = SyncSettingsPage::new(SyncSettingsInput::default());
    page.handle_control_action(ControlAction::TextEdited {
        id: ENDPOINT_ID,
        value: TextPayload::Plain("http://127.0.0.1:8384".to_owned()),
    });
    page.handle_control_action(ControlAction::TextEdited {
        id: API_KEY_ID,
        value: TextPayload::Sensitive(SensitiveText::new("secret".to_owned())),
    });

    assert_eq!(
        page.handle_control_action(ControlAction::Activated { id: CONFIGURE_CONNECTION_ID }),
        Some(WidgetAction::Consumed),
    );
    assert!(matches!(
        page.take_pending_action(),
        Some(SyncSettingsAction::ConfigureConnection { .. })
    ));
    assert_eq!(page.take_pending_action(), None);
}
```

注册两个模块但不要实现 `take_pending_action`，然后运行：

```bash
cargo test -p textora-app --lib configure_activation_is_consumed_and_can_be_taken_as_a_product_action
```

预期：因 `SyncSettingsPage` 或 `take_pending_action` 尚不存在而失败。

- [ ] **Step 3: 复制类型与页面，实现最小产品动作槽**

1. 将 `sync_types.rs` 的类型和其两个测试移至 app，所有 `crate::core::widget::SensitiveText` 导入改为 `ui::core::widget::SensitiveText`；将源码路径断言改为 app 的新文件路径。
2. 将 `sync_page.rs` 移至 app，内部 UI 导入改为 `ui::core`、`ui::theme`、`ui::button`、`ui::form`、`ui::inline_group`、`ui::label` 与 `ui::text_box` 的公开路径。
3. 在 `SyncSettingsPage` 增加 `pending_action: Option<SyncSettingsAction>`，初始化为 `None`。
4. 将激活分支末尾替换为：

```rust
self.pending_action = Some(action);
Some(WidgetAction::Consumed)
```

5. 添加：

```rust
pub fn take_pending_action(&mut self) -> Option<SyncSettingsAction> {
    self.pending_action.take()
}
```

6. 将所有从 `SettingsViewAction::Sync` 解构动作的页面测试改为断言 `WidgetAction::Consumed` 并调用 `take_pending_action()`；保留其余输入、焦点、滚动和布局断言。
7. 在 `lib.rs` 注册 `mod sync_settings_page;` 与 `mod sync_settings_types;`，不向公共 API re-export。

- [ ] **Step 4: 验证 GREEN 与迁移等价性**

运行：

```bash
cargo test -p textora-app --lib sync_settings
cargo fmt --all -- --check
cargo check -p textora-app
```

预期：新产品模块中的迁移测试和新增动作边界测试均通过，格式及 app 编译通过。

- [ ] **Step 5: 提交并审查**

```bash
git add crates/app/src/sync_settings_types.rs crates/app/src/sync_settings_page.rs crates/app/src/lib.rs
git commit -m "refactor(sync-ui): stage product sync settings page"
```

审查必须确认：页面不导入 `SettingsViewAction`，页面不会构造 `WidgetAction::Settings` 的 Sync payload，且 `take_pending_action` 为消费式读取。

## 后续衔接

- 原 Task 13B 保持文件范围，删除 UI 的 Sync 类型、页面字段和渲染分支；产品页面在该提交后仍独立编译。
- 原 Task 13C 在其既定三个 app 文件中从产品页面取出 `SyncSettingsAction`，映射到 `TextoraAction::Sync` 并交给 `TextoraProduct`。
- 原 Task 13D 删除旧 UI Sync 模块，并以 `rg -n 'SyncSettings|textora_sync' crates/ui` 作为边界验收。
