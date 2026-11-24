# Textora 应用状态与动作分发

`App` 装配 Textora 产品、共享 `EditorRuntime` 和动作队列。产品路径由 `ProductPaths` 提供。工作区、窗口和界面配置通过各自的状态对象访问。

| 状态对象 | 职责 |
| --- | --- |
| `WorkspaceServices` | 工作区存储、最近文件历史、文件监听、冲突通知以及保存后关闭 |
| `UiShell` | Dock、覆盖层、焦点、标签选择菜单的标签 ID 快照、侧边栏和标签滚动动画 |
| `SettingsUiState` | 主题目录、系统外观、设置覆盖层和持久化反馈。当前设置与解析后的主题由 `EditorRuntime` 持有 |
| `PlatformState` | 启动文件、字体准备、事件代理、无障碍适配、鼠标与重绘请求 |

`dispatch/mod.rs` 接收动作、汇总 `AppEffect` 并统一应用后续操作。搜索、设置与同步、视口、标签生命周期和编辑命令各有自己的分发模块。共享编辑命令的 Textora 回归用例和夹具位于 `dispatch/editor/`。

最近文件回归测试使用独立的配置目录，避免本机历史容量或并行测试更改进程环境影响测试结果。

## 验证

- `cargo test -p textora-app` 通过，包含 878 个库测试。
- 整仓复验通过 `./scripts/verify.sh`，覆盖新的分发边界和最新控件集成。
