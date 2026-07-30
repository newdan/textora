# Task 13 Sync UI 兼容过渡设计

## 目标

在保持每个提交可编译、每个子任务最多修改三个源文件的前提下，将 Sync 设置页面和动作完全迁出 `textora-ui`。

## 已确认的依赖环

删除 UI 中的 `SettingsCategory::Sync`、`SettingsViewAction::Sync` 和 Sync DTO 后，`textora-app` 的 `sync_view_model.rs`、`settings_overlay.rs` 与 `app_dispatch.rs` 仍会引用这些类型。原 Task 13B 只修改 UI 的三个文件、Task 13C 又未覆盖 `sync_view_model.rs`，因此无法在每个提交均可编译的约束下直接执行。

## 决策：兼容 DTO 分阶段收敛

1. 通用 `SettingsView` 先删除 Sync 分类、页面状态、渲染和事件分支，使 UI 不再展示或产生 Sync 行为。
2. UI 暂时保留仅供 app 编译的 Sync DTO 与 `SettingsViewAction::Sync` 兼容项；这些项目不再被通用 widget 使用，也不代表可用的 UI 功能。
3. app 分批将 view model、overlay 和 reducer 改用 `sync_settings_types` 与 `sync_settings_page`；产品 overlay 负责显示页面、取出 pending action 并映射为 `TextoraAction::Sync`。
4. 当 app 不再引用 UI Sync DTO 后，删除 UI 的兼容项、旧页面和旧类型，并启用严格源码边界断言。

## 边界

- 兼容项只在迁移期存在，必须由最终删除任务移除；最终 `rg -n 'SyncSettings|textora_sync' crates/ui` 无输出。
- `textora-ui` 不新增对 app、appkit 或 sync crate 的依赖。
- `textora-app` 的产品页面不得使用 `SettingsViewAction` 或构造 Sync payload 的 `WidgetAction::Settings`。
- 不改变同步业务、设置持久化格式或用户可见操作结果。
- 不引入通用产品插件框架、`Any` 类型擦除、字符串动作名或全局回调。

## 验证

- 过渡 UI 任务：`cargo test -p textora-ui` 与 `cargo check -p textora-app` 均通过；`SettingsCategory` 不含 Sync，通用 widget 不再引用 `SyncSettingsPage`。
- app 迁移任务：同步页面输入、产品动作与 reducer 使用 app 自有类型；打开设置、选择 Sync、提交动作的集成测试通过。
- 最终删除任务：UI crate 不含 `SyncSettings` 或 `textora_sync`，`bash scripts/check_architecture.sh` 通过。
