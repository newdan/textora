# Task 13 Sync 设置页面过渡设计

## 目标

将 Sync 设置页面从 `textora-ui` 迁往 `textora-app` 的过程中，始终保持每个子任务可编译，且不让 textora 专属动作重新进入通用 UI 层。

## 问题

现有 Sync 页面通过 `WidgetAction::Settings(SettingsViewAction::Sync(...))` 上报动作。原计划顺序要求先在 Task 13B 删除 `SettingsViewAction::Sync`，再在 Task 13C 建立 `TextoraAction::Sync`；这会使 13A 已复制到 app 的页面在 13B 后无法编译。

## 决策

Task 13A 中的产品页面拥有一个私有的待取 `SyncSettingsAction`：

1. 控件激活时，页面将动作保存到该槽位，并向通用 widget 框架返回 `WidgetAction::Consumed`。
2. 页面提供 `take_pending_action() -> Option<SyncSettingsAction>`，不暴露 `SettingsViewAction` 或 `WidgetAction::Settings` 的 Sync payload。
3. Task 13B 可安全删除 UI 的 `Sync` 分类、模块和动作 variant。
4. Task 13C 在 textora 的 overlay/event 路径调用 `take_pending_action()`，并把结果映射到产品 reducer 的 `TextoraAction::Sync`。

## 边界与非目标

- `appkit-core`、`appkit-shell` 与 `textora-ui` 不新增 textora sync 依赖。
- 不修改同步业务逻辑、持久化格式或用户可见设置流程。
- 不扩展通用 `WidgetAction`，也不引入 `Any`、字符串动作名或全局回调。
- 每个 Task 13 子任务仍最多修改 3 个源文件，并分别执行聚焦测试与审查。

## 验证

- 13A：产品页测试断言激活动作可由 `take_pending_action()` 取回，且 UI action 为 `Consumed`。
- 13B：UI 边界测试断言 `SettingsCategory` 与 `SettingsViewAction` 不含 Sync。
- 13C：集成测试覆盖“打开设置 → 选择 Sync → 产生产品动作 → TextoraProduct 分派”。
- 13D：`rg -n 'SyncSettings|textora_sync' crates/ui` 无输出。
