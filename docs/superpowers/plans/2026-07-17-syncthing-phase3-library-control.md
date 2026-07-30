# Syncthing Phase 3: Device and Library Control Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Textora 把整个资料库目录注册给本机 Syncthing，与用户自建远端配对，并提供扫描、暂停、恢复等显式控制，同时不覆盖用户在 Syncthing Web UI 中的独立配置。

**Architecture:** `textora-sync` 只对单个 device/folder 资源做精确读写；app 的 `LibraryRegistry` 保存 Textora 所拥有的映射。注册流程是可恢复状态机，每一步先读后比较；检测到漂移只报告，不自动修复。

**Tech Stack:** Syncthing REST config/defaults/db APIs、serde/toml、现有 app overlay/widget 系统、rfd 目录选择、阶段 1/2 的 worker protocol。

## Global Constraints

- 不 PUT 全局配置；Syncthing 数组字段写入前必须读取当前资源并保留未知/非 Textora 成员。
- 不修改 discovery、relay、NAT、GUI、listen address、upgrade 或全局 options。
- 资料库必须是绝对规范路径；拒绝彼此嵌套的已注册资料库。
- “移除资料库”默认只移除 Textora 映射；从 Syncthing 注销必须二次确认，且永不删除本地文件。
- 远端已有资料库首次接收时，本地目录必须为空，避免把无关文件上传。
- 配置漂移只展示差异和前往 Web UI，不做后台自动覆盖。

---

### Task 1: 扩展单资源配置 DTO 与只读比较

**Files:**
- Modify: `crates/sync/src/dto.rs`
- Modify: `crates/sync/src/client.rs`
- Modify: `crates/sync/src/lib.rs`

- [ ] 写 mock 契约测试：读取单 device/folder、读取 defaults；未知字段不破坏解析；不存在映射为明确结果而非通用 500。
- [ ] Run: `cargo test -p textora-sync config_read`
- [ ] Expected: FAIL。
- [ ] 定义：

```rust
pub struct DeviceConfig { pub device_id: DeviceId, pub name: String, pub addresses: Vec<String>, pub paused: bool }
pub struct FolderConfig { pub folder_id: FolderId, pub label: String, pub path: PathBuf, pub paused: bool, pub devices: Vec<DeviceId> }
pub enum ConfigurationDifference { MissingDevice, MissingFolder, DeviceAddressChanged, PathChanged, DeviceMembershipChanged, ManagedIgnoreChanged, PauseStateChanged }

impl SyncthingClient {
    pub fn device_config(&self, id: &DeviceId) -> Result<Option<DeviceConfig>, SyncError>;
    pub fn folder_config(&self, id: &FolderId) -> Result<Option<FolderConfig>, SyncError>;
    pub fn default_device(&self) -> Result<DeviceConfig, SyncError>;
    pub fn default_folder(&self) -> Result<FolderConfig, SyncError>;
}
```

- [ ] 验证静态同步地址只接受 Syncthing 支持的显式 scheme/非空 host；`ConfigurationDifference` 由纯函数比较期望所有权与实际配置，暂停状态差异可报告但不视作自动修复授权。
- [ ] Run: `cargo test -p textora-sync config_read`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): inspect Syncthing device and folder config"`

### Task 2: 实现窄范围配置 mutation

**Files:**
- Modify: `crates/sync/src/client.rs`
- Modify: `crates/sync/src/dto.rs`
- Create: `crates/sync/tests/config_contract.rs`

- [ ] 用 mock server 先断言 HTTP method/path/body，尤其验证 PUT/PATCH 的数组字段保留已有成员，且无 `/rest/config` 全量写入。
- [ ] Run: `cargo test -p textora-sync --test config_contract`
- [ ] Expected: FAIL。
- [ ] 实现：

```rust
pub fn put_device(&self, config: &DeviceConfig) -> Result<(), SyncError>;
pub fn put_folder(&self, config: &FolderConfig) -> Result<(), SyncError>;
pub fn remove_folder(&self, folder: &FolderId) -> Result<(), SyncError>;
pub fn patch_folder_paused(&self, folder: &FolderId, paused: bool) -> Result<(), SyncError>;
pub fn pause_device(&self, device: &DeviceId) -> Result<(), SyncError>;
pub fn resume_device(&self, device: &DeviceId) -> Result<(), SyncError>;
pub fn scan_folder(&self, folder: &FolderId) -> Result<(), SyncError>;
```

- [ ] device 暂停/恢复使用 `/rest/system/pause|resume?device=...`；folder 暂停通过读取并 PATCH 单 folder 的 `paused` 字段；“立即同步”只调用 `/rest/db/scan?folder=...`。
- [ ] 对 mutation 增加“写后读回验证”；并发 Web UI 改动导致不一致时返回 drift，不重试覆盖。
- [ ] Run: `cargo test -p textora-sync --test config_contract`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): add scoped Syncthing control operations"`

### Task 3: 安全维护 Textora ignore 规则

**Files:**
- Create: `crates/sync/src/ignore.rs`
- Modify: `crates/sync/src/client.rs`
- Modify: `crates/sync/src/lib.rs`

- [ ] 写测试：保留用户注释、顺序、空行和规则；仅追加一次完整 Textora managed block；重复执行幂等；managed block 被修改/缺失时报告 drift；读取失败时不写。
- [ ] Run: `cargo test -p textora-sync ignore`
- [ ] Expected: FAIL。
- [ ] 通过 `/rest/db/ignores?folder=...` 读取现有规则，首次注册时追加 `// BEGIN TEXTORA MANAGED`、`(?d).textora-save-*.tmp`、`// END TEXTORA MANAGED` 三行，再提交完整原数组；后续缺失只报告差异，除非用户显式修复。
- [ ] 明确不忽略 Syncthing 冲突副本、Textora 冲突副本或用户文档。
- [ ] Run: `cargo test -p textora-sync ignore`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): preserve user ignore rules"`

### Task 4: 建立资料库注册表与路径所有权

**Files:**
- Create: `crates/app/src/library_registry.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/Cargo.toml`

- [ ] 写测试：原子持久化、稳定 library id、canonical root、最长前缀匹配、拒绝相同/父子嵌套目录、symlink alias、损坏 TOML。
- [ ] Run: `cargo test -p textora-app --lib -- library_registry`
- [ ] Expected: FAIL。
- [ ] 定义：

```rust
pub(crate) enum LibraryOrigin { PublishedLocally, AcceptedFromRemote }
pub(crate) enum ProvisioningStage { ValidateLocalPath, EnsureRemoteDevice, EnsureFolder, EnsureIgnoreRule, AwaitRemoteAcceptance, Complete, Drift(ConfigurationDifference) }
pub(crate) enum LibraryRegistrationState { Provisioning { stage: ProvisioningStage }, Active, ConfigurationDrift }
pub(crate) struct LibraryRecord {
    pub library_id: String,
    pub root: PathBuf,
    pub folder_id: FolderId,
    pub remote_device_id: DeviceId,
    pub origin: LibraryOrigin,
    pub registration_state: LibraryRegistrationState,
    pub device_created_by_textora: bool,
    pub folder_created_by_textora: bool,
    pub managed_ignore_version: u16,
}
pub(crate) struct LibraryRegistry;
```

- [ ] 默认存储 `~/.edit+/libraries.toml`；提供 `owner_of(path)`，按 component 边界匹配规范路径，不用字符串 `starts_with`；Provisioning draft 每跨过一个不可回滚远端步骤就原子落盘。
- [ ] Run: `cargo test -p textora-app --lib -- library_registry`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): persist synchronized library ownership"`

### Task 5: 实现可恢复的发布资料库流程

**Files:**
- Create: `crates/app/src/library_provisioning.rs`
- Modify: `crates/app/src/sync_controller.rs`
- Modify: `crates/app/src/library_registry.rs`

- [ ] 用 fake client 写状态机测试：新增 remote device、新增 folder、已有等价资源幂等、部分成功后重试、写后出现 drift 停止、ignore 失败不登记成功；遇到非 Textora 创建且不等价的资源时停止并要求显式确认，不自动接管。
- [ ] Run: `cargo test -p textora-app --lib -- publish_library`
- [ ] Expected: FAIL。
- [ ] 使用 registry 已定义的互斥 `ProvisioningStage`，为每个转换写允许/拒绝测试：

```rust
ProvisioningStage::ValidateLocalPath
    -> ProvisioningStage::EnsureRemoteDevice
    -> ProvisioningStage::EnsureFolder
    -> ProvisioningStage::EnsureIgnoreRule
    -> ProvisioningStage::AwaitRemoteAcceptance
    -> ProvisioningStage::Complete
```

- [ ] 顺序固定：校验路径 → `probe` 获取本机 ID → 校验远端 Device ID/显示名/静态同步地址 → 读取/合并 remote device → 读取 defaults 构造 sendreceive folder → 只添加本机与远端 device membership → ignore → 初始 scan → 写 active registry。
- [ ] folder label 可读，folder id 使用稳定随机/哈希标识而非绝对路径；失败后只记录可安全重试的 draft，不谎报完成。
- [ ] Run: `cargo test -p textora-app --lib -- publish_library`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): publish a library through Syncthing"`

### Task 6: 实现接收远端资料库流程

**Files:**
- Modify: `crates/app/src/library_provisioning.rs`
- Modify: `crates/app/src/sync_controller.rs`
- Create: `crates/app/src/library_provisioning_tests.rs`

- [ ] 写测试：pending folder 仅来自已选 remote device；非空目录拒绝；空目录创建成功；中途失败可重试；已有 folder path 不被静默改写。
- [ ] Run: `cargo test -p textora-app --lib -- accept_remote_library`
- [ ] Expected: FAIL。
- [ ] 流程：刷新 pending folder → 用户选择目标空目录 → 从 default folder 构造配置 → 保留 offer 的 folder id/label → 加入双方 device → 写后读回 → ignore → registry。
- [ ] 若本地目录含任何条目（允许系统生成条目的名单必须显式常量化），停止并提示用户选择空目录，不自动移动/删除文件。
- [ ] Run: `cargo test -p textora-app --lib -- accept_remote_library`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): accept a remote Syncthing library"`

### Task 7: 扩展后台命令协议

**Files:**
- Modify: `crates/sync/src/service.rs`
- Modify: `crates/sync/src/lib.rs`
- Modify: `crates/app/src/sync_controller.rs`

- [ ] 先写 service 测试，覆盖 mutation 命令 request id、同 folder 命令串行、不同命令失败互不污染、refresh 合并。
- [ ] Run: `cargo test -p textora-sync service::tests::control`
- [ ] Expected: FAIL。
- [ ] 扩展 `SyncCommand`/`SyncResult`：`EnsureDevice`、`EnsureFolder`、`EnsureIgnoreRule`、`ScanFolder`、`SetFolderPaused`、`SetDevicePaused`、`RepairConfiguration`、`RemoveFolderRegistration`；payload 使用已定义强类型。
- [ ] app 的 `ProvisioningStage` 仅根据异步结果推进；每个 REST 步骤都由 worker 串行执行。UI 只接收阶段快照，绝不在 app render/dispatch 中同步 HTTP。
- [ ] Run: `cargo test -p textora-sync service::tests::control`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): dispatch library control commands"`

### Task 8: 定义纯资料库同步 UI

**Files:**
- Create: `crates/ui/src/widgets/library_sync.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/core/widget.rs`

- [ ] 写 widget 测试：发布、接收、选择目录、立即同步、暂停/恢复、打开 Web UI、移除映射、显式注销、漂移提示；危险动作先产生确认动作。
- [ ] Run: `cargo test -p textora-ui --lib -- library_sync`
- [ ] Expected: FAIL。
- [ ] 所有输入为纯数据：

```rust
pub struct LibrarySyncInput { pub rows: Vec<LibrarySyncRow>, pub pending_devices: Vec<PendingDeviceView>, pub pending_folders: Vec<PendingFolderView> }
pub enum LibrarySyncAction {
    ChoosePublishDirectory { remote_device_id: String, device_name: String, addresses: Vec<String> },
    ChooseAcceptDirectory { folder_id: String },
    Scan { library_id: String },
    SetPaused { library_id: String, paused: bool },
    ReviewDifference { library_id: String },
    ConfirmRepair { library_id: String },
    OpenWebUi,
    RemoveMapping { library_id: String },
    ConfirmUnregister { library_id: String },
    Close,
}
```

- [ ] 等待远端接受视图明确显示本机 Device ID、folder ID、资料库名称、建议远端路径，以及“前往远端 Syncthing Web UI 接受设备和资料库”的分步提示。
- [ ] `ui` 中不得出现 app 状态对象、REST DTO 或 client；目录选择结果由 app/rfd 处理，widget 只发起选择动作。
- [ ] Run: `cargo test -p textora-ui --lib -- library_sync`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(ui): add pure library sync controls"`

### Task 9: 将资料库 widget action 翻译为 app intent

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/ui/src/core/widget.rs`

- [ ] 写翻译测试：每个 `WidgetAction::LibrarySync` 产生对应 `AppAction::LibrarySync`；确认动作与初始请求不可混淆。
- [ ] Run: `cargo test -p textora-app --lib -- translate_library_sync_action`
- [ ] Expected: FAIL。
- [ ] 只做纯 action 翻译，不在 `events.rs` 访问目录选择器、registry 或 REST。
- [ ] Run: `cargo test -p textora-app --lib -- translate_library_sync_action`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): translate library sync actions"`

### Task 10: 将资料库控制副作用接入 app

**Files:**
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/sync_controller.rs`

- [ ] 写 action 测试：目录选择取消无副作用；scan 不等价于等待完成；暂停/恢复命令明确；repair 必须由差异确认页显式触发；默认 remove 仅删 registry；unregister 不触碰磁盘文件。
- [ ] Run: `cargo test -p textora-app --lib -- library_sync_action`
- [ ] Expected: FAIL。
- [ ] 使用现有 rfd 目录选择器；app 把 `LibraryRecord`/`LibrarySyncState` 映射成 pure row。
- [ ] 打开活跃文档时通过 `LibraryRegistry::owner_of` 绑定资料库状态，不向 `Workspace` 强塞 Syncthing字段。
- [ ] Run: `cargo test -p textora-app --lib -- library_sync_action`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): wire library synchronization controls"`

### Task 11: 增加资料库同步入口

**Files:**
- Modify: `crates/app/src/native_menu.rs`
- Modify: `crates/app/src/menu_handler.rs`
- Modify: `crates/app/src/dispatch/commands.rs`

- [ ] 写菜单测试：Settings 菜单出现“资料库同步…”；未配置本机连接时仍可打开面板并看到先配置连接的引导。
- [ ] Run: `cargo test -p textora-app --lib -- library_sync_menu`
- [ ] Expected: FAIL。
- [ ] `AppCommand::OpenLibrarySync` 只打开 Task 10 已接线的 overlay；连接状态决定控件 enabled state，不隐藏入口。
- [ ] Run: `cargo test -p textora-app --lib -- library_sync_menu`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): expose library synchronization entry"`

### Task 12: 建立双节点真实集成测试

**Files:**
- Create: `crates/sync/tests/support/syncthing_node.rs`
- Create: `crates/sync/tests/two_node.rs`
- Modify: `crates/sync/Cargo.toml`

- [ ] 测试 harness 为两个节点创建隔离 home、GUI/API 端口和 sync 端口；日志写临时目录；Drop 总能终止进程。
- [ ] `#[ignore]` 测试覆盖：互加 device、注册 folder、传输 Markdown/Unicode/二进制与可配置的大文件 fixture、scan、folder pause/resume、状态最终 up-to-date、删除 folder config 后本地文件仍在。
- [ ] 增加故障场景：远端离线后恢复、本机节点重启后的 event cursor 全量刷新、错误 API Key 停止重试、Web UI 制造配置漂移后不自动修复、两端同时修改产生可见 Syncthing conflict。
- [ ] Run: `SYNCTHING_BIN=/path/to/syncthing cargo test -p textora-sync --test two_node -- --ignored --nocapture`
- [ ] Expected: 使用 v2.1.1 时 PASS；超时错误含两节点末尾日志但不含 API key。
- [ ] Commit: `git commit -m "test(sync): exercise two-node library control"`

### Task 13: 阶段验收

**Files:**
- Modify only to fix a reproduced verification failure.

- [ ] Run: `cargo fmt --all -- --check`
- [ ] Run: `cargo test -p textora-sync`
- [ ] Run: `cargo test -p textora-ui --lib -- library_sync`
- [ ] Run: `cargo test -p textora-app --lib -- library_sync`
- [ ] Run: `cargo check -p textora-app`
- [ ] Manual: 本地发布后，在远端 Web UI 接受 device/folder；回到 Textora 看到同步完成。
- [ ] Manual: 在远端 Web UI 修改 folder path/device membership；Textora 只显示 drift，不自动写回。
- [ ] Manual: 退出 Textora，确认 Syncthing 继续同步。
- [ ] Expected: Textora 仅拥有自己的 registry 和精确资源修改，不拥有 Syncthing 全局配置。
