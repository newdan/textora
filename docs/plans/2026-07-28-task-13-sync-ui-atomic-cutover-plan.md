# Task 13 Sync UI 原子切换实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 以产品拥有的 Sync 页面替换 `textora-ui` 的 Sync 设置功能，并保持每一个可运行提交都保留同步设置入口。

**Architecture:** 先让通用 `SettingsView` 支持隐藏自身导航栏，使 app 可以复用其三个通用页面；app 再实现带四个分类按钮的 `TextoraSettingsOverlay`，其中 Sync 分类渲染产品页面。产品动作通过 overlay 的消费式读取进入 `AppAction::Sync`；最后一次已授权的原子提交切换 overlay、删除 UI Sync 类型/行为。

**Tech Stack:** Rust 2024、`textora-app`、`textora-ui` widget trait、现有 `ModalFrame` 与 `UiShell` overlay 路由。

## Global Constraints

- 只保留 `textora` binary；不改变 sync 协议、配置格式、密钥脱敏、后台服务生命周期或用户操作结果。
- `textora-ui` 不依赖 app、appkit 或 sync crate；最终不含 Sync 分类、Sync 动作、`SyncSettings` 或 `textora_sync`。
- 产品 `SyncSettingsPage` 不得导入 `SettingsViewAction` 或构造 `WidgetAction::Settings` 的 Sync payload。
- 除 Task 6 外，每任务最多改 3 个源文件；Task 6 已获用户授权，可原子改 5 个源文件以防止设置入口中断。
- 每个行为变化先写失败测试；每次提交前运行 `cargo fmt --all -- --check` 和任务指定的编译/测试命令。
- Task 6 后必须运行 `cargo check --workspace`、`cargo test -p textora-ui`、`cargo test -p textora-app --lib` 与 `bash scripts/check_architecture.sh`。

---

### Task 1: 让通用 SettingsView 可嵌入产品 overlay

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/widget.rs`

**Produces:**
- `SettingsView::set_category_navigation_visible(bool)`
- `SettingsView::set_active_category(SettingsCategory)`

- [ ] 写测试：隐藏导航时，三个通用 category button 不绘制/不命中；调用 `set_active_category(Editor)` 后重建 Editor form。
- [ ] 运行测试确认缺少公开 API 时失败：`cargo test -p textora-ui settings_view`。
- [ ] 加入 `category_navigation_visible: bool`（默认 `true`）；隐藏时不布局、不绘制、不分派 category buttons，且 content rect 使用完整宽度；`set_active_category` 仅接受 Appearance、Editor、Interface。
- [ ] 运行 `cargo test -p textora-ui settings_view` 与 `cargo check -p textora-app`。
- [ ] 提交：`refactor(ui): allow product settings composition`。

### Task 2: 创建产品组合设置 overlay

**Files:**
- Create: `crates/app/src/textora_settings_overlay.rs`
- Modify: `crates/app/src/lib.rs`

**Produces:**
- `TextoraSettingsOverlay::new(SettingsViewInput, SyncSettingsInput)`
- `set_settings_input`、`set_sync_input`、`take_pending_sync_action`

- [ ] 写测试：默认显示 Appearance；点击 Sync 分类后，页面将 `SyncSettingsPage` 的 `WidgetAction::Consumed` 留在组件内，`take_pending_sync_action()` 返回一次产品动作；切回通用分类后 generic `WidgetAction::Settings` 不变。
- [ ] 运行 RED：`cargo test -p textora-app --lib textora_settings_overlay`。
- [ ] 实现 app 私有 `Widget`：复制旧四项分类导航的文本、WidgetId 与布局常量；内部持有隐藏导航的 `SettingsView` 和 `SyncSettingsPage`，只将 Sync 页面动作存入产品 pending 槽。
- [ ] 运行 `cargo test -p textora-app --lib textora_settings_overlay` 与 `cargo check -p textora-app`。
- [ ] 提交：`refactor(app): compose textora settings overlay`。

### Task 3: 让 UiShell 取出产品 overlay 的待处理动作

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

**Produces:**
- `UiShell::take_pending_sync_settings_action() -> Option<SyncSettingsAction>`

- [ ] 写测试：含 `ModalFrame<TextoraSettingsOverlay>` 的活动 overlay 可以消费一次 pending sync action；空或非设置 overlay 返回 `None`。
- [ ] 运行 RED：`cargo test -p textora-app --lib ui_shell`。
- [ ] 通过 `ModalFrame::content_as_any_mut` 精确 downcast 到 `TextoraSettingsOverlay` 后调用其消费式方法；不得向 `WidgetAction` 添加产品 payload。
- [ ] 运行 `cargo test -p textora-app --lib ui_shell` 与 `cargo check -p textora-app`。
- [ ] 提交：`refactor(app): expose pending product settings actions`。

### Task 4: 预置产品 Sync action 路径

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Produces:**
- `AppAction::Sync(SyncSettingsAction)`
- 产品 action 到现有 sync controller reducer 的路由

- [ ] 写测试：在 `UiShell` 消费产品 pending action 后，event translation 产生 `AppAction::Sync`；dispatch 到既有 controller validation 时保留 redaction 与错误处理。
- [ ] 运行 RED：`cargo test -p textora-app --lib sync_settings_action_reaches_existing_controller_validation`。
- [ ] `events.rs` 在 UI dispatch 完成且借用释放后调用 `take_pending_sync_settings_action`；`actions.rs` 定义产品 action；`app_dispatch.rs` 接受产品类型。旧 `SettingsViewAction::Sync` 分支在 Task 6 前保留并显式转换到同一 reducer。
- [ ] 运行 `cargo test -p textora-app --lib app_dispatch`、`cargo test -p textora-app --lib events`、`cargo check -p textora-app`。
- [ ] 提交：`refactor(app): route product sync settings actions`。

### Task 5: 让同步 view model 以产品 DTO 为源

**Files:**
- Modify: `crates/app/src/sync_view_model.rs`
- Modify: `crates/app/src/settings_overlay.rs`

**Produces:**
- `build_sync_settings_input` 返回 `crate::sync_settings_types::SyncSettingsInput`
- 临时旧 UI adapter，仅在旧入口仍可见时使用

- [ ] 写测试：产品 DTO 的 connection、library、notice 映射与现有断言等价；临时 adapter 不含 API key 明文。
- [ ] 运行 RED：`cargo test -p textora-app --lib sync_view_model`。
- [ ] 将 mapper 改为 app 类型；在 `settings_overlay.rs` 的旧入口处保留局部、单向的 product→旧 UI adapter，且明确标记为 Task 6 删除。
- [ ] 运行 `cargo test -p textora-app --lib sync_view_model`、`cargo test -p textora-app --lib settings_overlay`、`cargo check -p textora-app`。
- [ ] 提交：`refactor(sync): own settings view model in textora`。

### Task 6: 原子切换产品设置入口（已授权超过 3 文件）

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/types.rs`
- Modify: `crates/ui/src/widgets/settings_view/widget.rs`
- Modify: `crates/ui/src/widgets/settings_view/mod.rs`
- Modify: `crates/app/src/settings_overlay.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Consumes:** Tasks 1–5 的嵌入式 generic SettingsView、TextoraSettingsOverlay、pending action 路由和产品 DTO。

- [ ] 写集成 RED：打开设置、点击“同步”、输入 endpoint/API key、触发连接测试，断言产品 reducer 收到 `SyncSettingsAction`；同时断言 `SettingsCategory`/`SettingsViewAction` 不含 Sync。
- [ ] 运行 RED：`cargo test -p textora-app --lib settings_overlay` 和 `cargo test -p textora-ui settings_view`。
- [ ] 在 `settings_overlay.rs` 用 `TextoraSettingsOverlay` 取代 `SettingsView` 和临时 adapter；刷新路径分别更新 generic 与产品输入。
- [ ] 在 `app_dispatch.rs` 删除旧 `SettingsViewAction::Sync` 分支与转换函数，仅保留 `AppAction::Sync` 的产品 reducer。
- [ ] 在 UI 三文件删除 Sync category、Sync page field、渲染/焦点/IME/event 分支、Sync action variant 与 Sync 模块声明/re-export；保留旧源文件供 Task 7 删除。
- [ ] 运行：

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test -p textora-ui
cargo test -p textora-app --lib
bash scripts/check_architecture.sh
```

- [ ] 提交：`refactor(app): cut over sync settings to textora product`。

### Task 7: 删除已断开的 UI Sync 源文件并完成边界验收

**Files:**
- Delete: `crates/ui/src/widgets/settings_view/sync_page.rs`
- Delete: `crates/ui/src/widgets/settings_view/sync_types.rs`

- [ ] 确认相应页面和类型测试已在 `crates/app/src/sync_settings_page.rs` 与 `sync_settings_types.rs` 中存在。
- [ ] 删除两个源文件。
- [ ] 运行：

```bash
rg -n 'SyncSettings|textora_sync' crates/ui
cargo test -p textora-ui
cargo test -p textora-app --lib sync_settings
```

预期：`rg` 无输出，测试通过。

- [ ] 提交：`refactor(ui): delete obsolete sync settings modules`。
