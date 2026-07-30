# Syncthing UI Adapter 设计

## 1. 目标与范围

在 UI widget 重构完成后，为已有的 `textora-sync` 控制面增加第一个可操作的同步面板。面板只负责展示纯 ViewModel 和发出纯用户意图；所有连接、Keychain、目录选择、资料库注册和危险操作仍由 `app` 层执行。

本阶段覆盖：

- 本机 Syncthing 连接状态、版本和 Device ID 展示。
- loopback 地址与 API Key 配置、连接测试。
- 发布本地资料库和接收 pending 资料库。
- 资料库扫描、暂停/恢复、显式修复、移除映射和注销。
- 同步 notice 的暂存和展示。

本阶段不实现 REST DTO 直通 UI、UI 内文件读写、Keychain 读写、自动后台修复、自动删除本地文件或打开 Syncthing 公网接口。

## 2. 组件边界

### 2.1 `crates/ui`

新增独立的 `SyncPanelWidget`，只依赖以下纯数据：

```rust
pub enum SyncConnectionView {
    NotConfigured,
    Connecting,
    Connected { device_id: String, version: String },
    AuthenticationRequired,
    Incompatible { found: String },
    Unavailable { message: String },
}

pub enum LibrarySyncState {
    Pending,
    Scanning,
    Syncing,
    UpToDate,
    Paused,
    AwaitingRemoteAcceptance,
    ConfigurationMismatch,
    Error { message: String },
}

pub struct LibraryView {
    pub name: String,
    pub root_display: String,
    pub state: LibrarySyncState,
    pub can_repair: bool,
    pub can_remove_mapping: bool,
    pub can_unregister: bool,
}

pub struct PendingFolderView {
    pub folder_id: String,
    pub offered_by: String,
}

pub enum SyncNoticeSeverity {
    Info,
    Warning,
    Error,
}

pub struct SyncNoticeView {
    pub severity: SyncNoticeSeverity,
    pub message: String,
}

pub struct SyncPanelInput {
    pub endpoint: String,
    pub has_api_key: bool,
    pub connection: SyncConnectionView,
    pub libraries: Vec<LibraryView>,
    pub pending_folders: Vec<PendingFolderView>,
    pub notices: Vec<SyncNoticeView>,
}

pub enum SyncPanelAction {
    Close,
    TestConnection { endpoint: String, api_key: String },
    ConfigureConnection { endpoint: String, api_key: String },
    PublishLibrary {
        remote_device_id: String,
        remote_name: String,
        remote_addresses: Vec<String>,
    },
    AcceptRemoteLibrary { pending_index: usize },
    ScanLibrary { library_index: usize },
    SetLibraryPaused { library_index: usize, paused: bool },
    RepairLibrary { library_index: usize },
    RemoveLibraryMapping { library_index: usize },
    UnregisterLibrary { library_index: usize },
}
```

UI 不认识 `SyncController`、`LibraryRecord`、`FolderId` 或 REST 错误类型。敏感 API Key 只在 widget 编辑期间存在，发出 action 后由 app 接收，不进入绘制输入、Debug 或 notice 文本。

`SyncConnectionView`、`LibrarySyncState` 和 `SyncNoticeSeverity` 是互斥状态枚举；`LibraryView` 的能力字段只用于按钮可用性，不承载 app 状态。`endpoint` 允许 ViewModel 回填非敏感配置，`api_key` 永远不进入 `SyncPanelInput`。

### 2.2 `crates/app`

新增 `sync_view_model.rs`，负责：

1. 将 `SyncControllerSnapshot` 和 `SyncNotice` 转换为 `SyncPanelInput`。
2. 将 `SyncPanelAction` 中的索引和字符串转换为 app 层命令。
3. 将连接状态、资料库状态和错误消息转换为稳定的中文展示文本。

`AppAction` 只增加一个 `SyncPanelAction` 分支，`events.rs` 只负责把 `WidgetAction` 翻译为这个 app intent。目录选择通过 app 的 `rfd` 接口完成，之后调用 `SyncController` 的 `pub(crate)` 方法。

面板的首个入口是 Sidebar 设置菜单中的“打开同步面板”；菜单 action 只产生 `AppAction::OpenSyncPanel`，不在 UI 层直接访问 controller。

### 2.3 `UiShell`

同步面板作为独立 overlay 持有，不加入编辑器 Dock，不改变编辑器、Sidebar、TOC、滚动条或状态栏的几何。面板打开时覆盖右侧固定宽度区域，事件优先发给面板；关闭 action 或 Escape 后移除 overlay，编辑器恢复接收事件。

## 3. 数据流

```text
SyncControllerSnapshot / SyncNotice
              │
              ▼
     app::sync_view_model
              │ SyncPanelInput
              ▼
       SyncPanelWidget
              │ SyncPanelAction
              ▼
          AppAction
              │
              ▼
    SyncController / rfd / AppEffect
```

后台结果仍通过 `AppEvent::SyncResultsReady` 唤醒主线程。面板只在 app renderer 注入新的 ViewModel；网络失败只更新状态和 notice，不阻塞编辑器事件循环。

## 4. 交互与安全规则

- 面板默认关闭，不新增全局后台轮询。
- 未配置连接时只显示配置表单；已连接时显示 Device ID、版本和资料库操作。
- 发布资料库先由 app 打开目录选择器，再调用 controller；UI 不接触路径选择器。
- 接收 pending 资料库先选择 pending 项，再由 app 选择空目录。
- 注销和移除映射保持 controller 的显式命令语义；UI 不删除本地文件。
- API Key 输入框使用密码样式，不回填明文；ViewModel 只保留 `has_api_key`。
- `SyncNotice` 只展示稳定消息，不展示原始 HTTP body 或 API Key。

## 5. 测试策略

- `crates/app/src/sync_view_model.rs`：连接状态、资料库状态、pending folder、notice 的映射测试；API Key 不进入 ViewModel 的测试。
- `crates/app/src/events.rs` / `app_dispatch.rs`：WidgetAction 到 AppAction 的纯翻译测试。
- `crates/app/src/ui_shell.rs`：面板 overlay 的布局、事件优先级和关闭后恢复 Dock 的测试。
- `crates/ui/src/widgets/sync_panel.rs`：上述纯数据类型、widget 布局、按钮命中、密码输入和 action 测试。
- 运行 `cargo test -p textora-app --lib -- sync_panel`、`cargo test -p textora-ui --lib -- sync_panel`、`cargo check -p textora-app`。

## 6. 后续阶段

本阶段完成后，继续执行原 Phase 4 硬化计划：将 `DiskRevision` 下沉至 `textora-core`、引入资料库级后台监控、补齐启动期间 dirty snapshot 对账，并在最后重新运行 `./scripts/verify.sh`。
