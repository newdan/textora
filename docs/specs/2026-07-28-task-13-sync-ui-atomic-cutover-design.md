# Task 13 Sync UI 原子切换设计

## 决策

用户授权最终 Sync 设置入口切换可以超过“单任务最多三个源文件”的通常限制。此例外仅用于一个经过完整验证与跨模块审查的原子提交，优先保证同步设置入口在任一可运行提交中都不中断。

## 原因

旧入口横跨 `textora-ui` 的设置分类/页面与 `textora-app` 的输入映射、overlay、事件路由和同步 reducer。先删除 UI 入口会导致用户暂时失去 Sync 设置；先启用产品入口又必须同时去除旧入口以避免两套页面并存。因此最终入口切换无法被拆为各自独立、同时保持用户行为的三个文件提交。

## 执行顺序

1. 在 app 内预置产品 overlay、产品输入映射、pending action 取用和 reducer 路径，但继续让现有 UI Sync 页面作为唯一可见入口。
2. 用一个原子 cutover 提交同时：使 overlay 显示产品页面、路由产品 Sync 动作、删除 UI Sync 分类/页面/动作及 app 对 UI Sync DTO 的最后引用。
3. 该提交后运行全量 workspace 验证、架构源码检查和任务级跨模块审查；仅在这些检查通过时接受切换。

## 不变量

- 在切换前后，用户均可从设置中访问 Sync 页面，且连接测试、库操作、通知刷新与焦点/滚动行为保持可用。
- `textora-ui` 不依赖 app 或 sync crate；切换完成后不再包含 `SyncSettings`、`textora_sync`、Sync 分类或 Sync 动作。
- 产品页面只在 app 内产生/消费 `SyncSettingsAction`，不将其装入 `WidgetAction::Settings`。
- 不改变同步协议、配置文件格式、密钥脱敏或后台服务生命周期。

## 验证门槛

- `cargo fmt --all -- --check`
- `cargo check --workspace`
- `cargo test -p textora-ui`
- `cargo test -p textora-app --lib`
- `bash scripts/check_architecture.sh`
- `rg -n 'SyncSettings|textora_sync' crates/ui` 无输出
- 任务级只读审查必须不存在 Critical 或 Important 问题。
