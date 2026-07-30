# Syncthing Phase 2: Local Connection and Read-Only Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让用户在 Textora 内连接已独立运行的本机 Syncthing，安全保存 API Key，并在不阻塞编辑器的前提下显示只读连接状态。

**Architecture:** 普通连接元数据写入独立 `sync.toml`，API Key 通过 `SyncSecretStore` 存入 macOS Keychain。`SyncService` 独占阻塞式 REST 客户端并通过 channel 与 app 通信；`ui` 仅定义设置面板和状态栏的纯输入/动作。

**Tech Stack:** `textora-sync`、security-framework 3.7、serde/toml、std::sync::mpsc、winit EventLoopProxy、现有 TextBox/Button/StatusBar widgets。

## Global Constraints

- 本阶段严格只读，不调用任何 Syncthing 配置写入或控制端点。
- API Key 不进入 app settings、workspace、日志、错误文本、剪贴板或 UI 回显。
- 所有 HTTP 和 Keychain I/O 均不得在 winit/UI 线程执行。
- Syncthing 不可用时编辑、打开、保存等核心能力保持可用。
- UI 新类型只能依赖纯数据；`crates/ui` 不得依赖 `textora-sync`。

---

### Task 1: 持久化非敏感连接元数据

**Files:**
- Modify: `crates/app/Cargo.toml`
- Create: `crates/app/src/sync_connection_store.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] 添加 `textora-sync`、`security-framework`、`semver` 依赖；为测试添加 `tempfile`（若 workspace 已有则复用）。
- [ ] 写测试：缺失文件返回未配置；只持久化 endpoint 与稳定的 Keychain account；非法/额外敏感字段被拒绝；原子替换失败不破坏旧文件。
- [ ] Run: `cargo test -p textora-app --lib -- sync_connection_store`
- [ ] Expected: FAIL。
- [ ] 实现 `SyncConnectionStore`，默认路径 `~/.edit+/sync.toml`：

```rust
pub(crate) struct StoredSyncConnection {
    pub endpoint: LoopbackEndpoint,
    pub keychain_account: String,
}

impl SyncConnectionStore {
    pub(crate) fn load(&self) -> Result<Option<StoredSyncConnection>, SyncConnectionStoreError>;
    pub(crate) fn save(&self, connection: &StoredSyncConnection) -> Result<(), SyncConnectionStoreError>;
    pub(crate) fn remove(&self) -> Result<(), SyncConnectionStoreError>;
}
```

- [ ] 将 Keychain service 常量定为 `com.textora.syncthing-api-key`；TOML 序列化类型中不得存在 secret 字段。
- [ ] Run: `cargo test -p textora-app --lib -- sync_connection_store`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): persist Syncthing connection metadata"`

### Task 2: 通过抽象接入 macOS Keychain

**Files:**
- Create: `crates/app/src/sync_secret_store.rs`
- Modify: `crates/app/src/sync_connection_store.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] 用内存 fake 写失败测试，覆盖写入、读取、删除、Keychain 拒绝和不存在；测试错误字符串不含 secret。
- [ ] Run: `cargo test -p textora-app --lib -- sync_secret_store`
- [ ] Expected: FAIL。
- [ ] 实现：

```rust
pub(crate) trait SyncSecretStore: Send + Sync {
    fn load_api_key(&self, account: &str) -> Result<Option<ApiKey>, SyncSecretStoreError>;
    fn save_api_key(&self, account: &str, new_secret: &str) -> Result<(), SyncSecretStoreError>;
    fn delete_api_key(&self, account: &str) -> Result<(), SyncSecretStoreError>;
}

pub(crate) struct MacKeychainSecretStore;
```

- [ ] adapter 仅在此模块调用 `set_generic_password`、`get_generic_password`、`delete_generic_password`；错误转换不包含返回字节。
- [ ] 保存路径直接接收用户刚输入的 secret 并写 Keychain；读取路径由 Keychain bytes 构造 `ApiKey`，不为 `ApiKey` 增加公开取回明文、`Serialize` 或 `Display` 能力。
- [ ] Run: `cargo test -p textora-app --lib -- sync_secret_store`
- [ ] Expected: PASS；fake 测试不访问真实 Keychain。
- [ ] Commit: `git commit -m "feat(app): store Syncthing API key in Keychain"`

### Task 3: 建立后台命令服务

**Files:**
- Create: `crates/sync/src/service.rs`
- Modify: `crates/sync/src/lib.rs`
- Modify: `crates/sync/Cargo.toml`

- [ ] 用 fake transport/短超时写测试：命令按序处理、结果关联 request id、关闭可 join、断连不杀死 worker、无 busy loop。
- [ ] Run: `cargo test -p textora-sync service`
- [ ] Expected: FAIL。
- [ ] 实现稳定协议：

```rust
pub enum SyncCommand {
    Probe { request_id: u64 },
    Refresh { request_id: u64, folders: Vec<FolderId> },
    Subscribe { cursor: EventCursor },
    Shutdown,
}

pub enum SyncResult {
    Probe { request_id: u64, outcome: Result<InstanceInfo, SyncError> },
    Refresh { request_id: u64, outcome: Result<Vec<(FolderId, FolderStatus)>, SyncError> },
}

pub enum SyncEvent {
    Remote { id: u64, kind: SyncEventKind },
    FullRefreshRequired,
}

impl SyncService {
    pub fn spawn(client: SyncthingClient, wake: impl Fn() + Send + 'static) -> Self;
    pub fn submit(&self, command: SyncCommand) -> Result<(), SyncError>;
    pub fn try_recv(&self) -> Option<SyncResult>;
    pub fn try_recv_event(&self) -> Option<SyncEvent>;
    pub fn shutdown(self);
}
```

- [ ] 将 `Subscribe` 长轮询与命令处理隔离，确保等待事件时 `Shutdown` 和 `Probe` 不饥饿；线程名带 `textora-syncthing-*`。连接错误采用阶段 1 的有上限退避，鉴权/版本错误停止自动重试。
- [ ] Run: `cargo test -p textora-sync service`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): run REST commands off the UI thread"`

### Task 4: 在 app 中编排连接生命周期

**Files:**
- Create: `crates/app/src/sync_controller.rs`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_lifecycle.rs`

- [ ] 写 controller 测试：未配置、载入成功、缺少 secret、鉴权失败、版本不兼容、断开连接、退出 shutdown，均映射为单一 enum 状态。
- [ ] Run: `cargo test -p textora-app --lib -- sync_controller`
- [ ] Expected: FAIL。
- [ ] 实现：

```rust
pub(crate) enum SyncConnectionState {
    NotConfigured,
    Connecting,
    Connected { instance: InstanceInfo },
    AuthenticationRequired,
    Incompatible { found: semver::Version },
    Unavailable { message: String },
}

pub(crate) struct SyncController { /* store, secret store, service, typed state */ }
```

- [ ] `AppEvent` 增加 `SyncResultsReady`，wake closure 只发送该事件；`user_event` 中 drain results 与 domain events，并请求重绘；`FullRefreshRequired` 触发一次受节流的全量刷新。
- [ ] App 退出时显式 shutdown worker；不影响外部 Syncthing 进程。
- [ ] Run: `cargo test -p textora-app --lib -- sync_controller`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): coordinate local Syncthing connection"`

### Task 5: 定义纯设置面板

**Files:**
- Create: `crates/ui/src/widgets/sync_settings.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/core/widget.rs`

- [ ] 写 widget 测试：endpoint 输入、API Key 密码态输入、Test/Save/Disconnect/Open Web UI/Close 动作；重建输入时不显示已保存 secret。
- [ ] Run: `cargo test -p textora-ui --lib -- sync_settings`
- [ ] Expected: FAIL。
- [ ] 定义纯类型：

```rust
pub enum SyncSettingsConnectionView { NotConfigured, Testing, Connected { device_id: String }, Error { message: String } }
pub struct SyncSettingsInput { pub endpoint: String, pub has_saved_key: bool, pub connection: SyncSettingsConnectionView }
pub enum SyncSettingsAction { TestConnection { endpoint: String, api_key: String }, SaveConnection { endpoint: String, api_key: Option<String> }, Disconnect, OpenWebUi, Close }
```

- [ ] 密码 TextBox 不提供复制明文动作；保存后的 input 只保留 `has_saved_key`。
- [ ] Run: `cargo test -p textora-ui --lib -- sync_settings`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(ui): add pure Syncthing connection panel"`

### Task 6: 将设置 widget action 翻译为 app intent

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/ui/src/core/widget.rs`

- [ ] 写翻译测试：每个 `WidgetAction::SyncSettings` 精确产生一个语义化 `AppAction::SyncSettings`，关闭/消费动作不泄漏到编辑器。
- [ ] Run: `cargo test -p textora-app --lib -- translate_sync_settings_action`
- [ ] Expected: FAIL。
- [ ] 扩展穷尽枚举并完成纯翻译，不在 `events.rs` 执行 Keychain、HTTP 或持久化。
- [ ] Run: `cargo test -p textora-app --lib -- translate_sync_settings_action`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): translate Syncthing settings actions"`

### Task 7: 接线设置面板副作用

**Files:**
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/sync_controller.rs`

- [ ] 写 app 测试：打开 overlay；Test 只做临时 probe；Save 顺序为 Keychain 成功后写 metadata；任一步失败回滚新 secret；Disconnect 删除两处并关闭 service。
- [ ] Run: `cargo test -p textora-app --lib -- sync_settings_action`
- [ ] Expected: FAIL。
- [ ] app 将 `SyncConnectionState` 映射为 `SyncSettingsInput`，将 app intent 映射为 controller 命令；首次打开默认地址为 `http://127.0.0.1:8384`。
- [ ] “打开 Web UI”只打开已验证的 loopback endpoint；使用现有平台 URL opener，禁止拼接 API Key。
- [ ] Run: `cargo test -p textora-app --lib -- sync_settings_action`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): wire Syncthing connection settings"`

### Task 8: 增加“设置 → 同步”入口

**Files:**
- Modify: `crates/app/src/native_menu.rs`
- Modify: `crates/app/src/menu_handler.rs`
- Modify: `crates/app/src/dispatch/commands.rs`

- [ ] 写菜单测试：Settings 菜单出现独立“同步…”项，映射到 `AppCommand::OpenSyncSettings`；原“打开 Settings 文件”行为保持不变。
- [ ] Run: `cargo test -p textora-app --lib -- sync_settings_menu`
- [ ] Expected: FAIL。
- [ ] dispatch command 只调用 Task 7 的 overlay 打开方法并请求 redraw，不直接访问 HTTP/Keychain。
- [ ] Run: `cargo test -p textora-app --lib -- sync_settings_menu`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): expose Syncthing settings entry"`

### Task 9: 在状态栏显示只读连接状态

**Files:**
- Modify: `crates/ui/src/widgets/status_bar.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/sync_controller.rs`

- [ ] 写 statusbar 测试，覆盖无配置不显示、连接/离线/鉴权/版本不兼容的短文案；确保编辑器已有光标信息不退化。
- [ ] Run: `cargo test -p textora-ui --lib -- status_bar`
- [ ] Expected: FAIL。
- [ ] 给 `StatusBarInput` 增加 `sync_label: Option<String>`，由 app 生成本地化前的短文案；ui 只负责布局和绘制。
- [ ] controller 采用事件驱动刷新并加 30 秒保底刷新常量，禁止每帧请求 `/rest/db/status`。
- [ ] Run: `cargo test -p textora-ui --lib -- status_bar`
- [ ] Run: `cargo test -p textora-app --lib -- sync_`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): show local Syncthing connection status"`

### Task 10: 阶段验收

**Files:**
- Modify only to fix a reproduced verification failure.

- [ ] Run: `cargo fmt --all -- --check`
- [ ] Run: `cargo test -p textora-sync`
- [ ] Run: `cargo test -p textora-ui --lib -- sync_`
- [ ] Run: `cargo test -p textora-app --lib -- sync_`
- [ ] Run: `cargo check -p textora-app`
- [ ] Manual: 连接本机 v2.1.1，重启 Textora 后无需重新输入 key；填写错误 key 时 UI 可恢复且终端无 secret。
- [ ] Manual: 退出 Textora 后确认 Syncthing 进程仍运行。
- [ ] Expected: 本阶段没有任何配置 mutation 请求；编辑与保存路径不等待网络。
