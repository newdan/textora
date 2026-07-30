# Syncthing Phase 1: API Client and Contract Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立独立的 `textora-sync` crate，以强类型、可测试的接口封装 Syncthing 2.1.x REST API，且不依赖 Textora 应用状态。

**Architecture:** `SyncthingClient` 是同步 HTTP 适配器；端点、凭据和标识符由不可非法构造的类型保护。业务状态由纯函数归约，HTTP 正确性由本机 mock server 和可选的真实 Syncthing 契约测试共同约束。

**Tech Stack:** Rust 2024、reqwest blocking 0.13、serde/serde_json、semver、thiserror、标准库 TCP 测试服务器。

## Global Constraints

- 先阅读总览与设计文档；本阶段不修改 `crates/app` 或 `crates/ui`。
- 每个任务最多修改 3 个文件；每个行为先写失败测试。
- API Key 的 `Debug`/`Display` 必须脱敏，响应错误不得拼接请求头。
- HTTP 超时必须有限：连接 2 秒、单次请求 10 秒，提取为常量。
- 仅接受 Syncthing `>=2.1.1,<2.2.0`，版本不兼容是类型化错误。

---

### Task 1: 创建 crate 与强类型边界

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/sync/Cargo.toml`
- Create: `crates/sync/src/lib.rs`

- [ ] 在根 workspace 添加 `crates/sync`（若使用 `crates/*` 已自动覆盖，则只补 workspace 依赖），并声明包名 `textora-sync`。
- [ ] 添加 `reqwest = { version = "0.13", default-features = false, features = ["blocking", "json"] }`、`serde`、`serde_json`、`semver`、`thiserror`。
- [ ] 在 `lib.rs` 只声明模块与稳定的 public re-export，不暴露 HTTP DTO。
- [ ] Run: `cargo check -p textora-sync`
- [ ] Expected: crate 可解析并编译；尚无 app 依赖。
- [ ] Commit: `git commit -m "feat(sync): scaffold Syncthing adapter crate"`

### Task 2: 限制本机 REST 地址

**Files:**
- Create: `crates/sync/src/endpoint.rs`
- Create: `crates/sync/src/error.rs`
- Modify: `crates/sync/src/lib.rs`

- [ ] 写 `LoopbackEndpoint` 测试，覆盖 `127.0.0.1`、`localhost`、`[::1]`；拒绝 HTTPS、非回环主机、userinfo、非根 path、query 和 fragment。
- [ ] Run: `cargo test -p textora-sync endpoint -- --nocapture`
- [ ] Expected: FAIL，因为类型尚不存在。
- [ ] 实现：

```rust
impl LoopbackEndpoint {
    pub fn parse(candidate: &str) -> Result<Self, SyncError>;
    pub(crate) fn join(&self, path: &str) -> Result<reqwest::Url, SyncError>;
    pub fn as_str(&self) -> &str;
}

pub enum SyncError {
    InvalidEndpoint { reason: String },
    ConnectionRefused,
    RequestTimeout { operation: &'static str },
    Authentication,
    IncompatibleVersion { found: semver::Version },
    InvalidResponse { operation: &'static str, message: String },
    Remote { operation: &'static str, status: u16 },
}
```

- [ ] 将合法地址规范化为无结尾 `/`；错误只含安全上下文。
- [ ] Run: `cargo test -p textora-sync endpoint`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): validate loopback Syncthing endpoints"`

### Task 3: 建立脱敏凭据与标识符

**Files:**
- Create: `crates/sync/src/identifiers.rs`
- Modify: `crates/sync/src/error.rs`
- Modify: `crates/sync/src/lib.rs`

- [ ] 写测试：空 API Key、非法 Device ID、空/含路径分隔符的 Folder ID 被拒绝；`format!("{:?}", key)` 不含明文。
- [ ] Run: `cargo test -p textora-sync identifiers`
- [ ] Expected: FAIL。
- [ ] 实现：

```rust
impl ApiKey {
    pub fn new(secret: String) -> Result<Self, SyncError>;
    pub(crate) fn expose_for_header(&self) -> &str;
}
impl DeviceId {
    pub fn parse(candidate: String) -> Result<Self, SyncError>;
    pub fn as_str(&self) -> &str;
}
impl FolderId {
    pub fn new(candidate: String) -> Result<Self, SyncError>;
    pub fn as_str(&self) -> &str;
}
```

- [ ] 手写 `Debug for ApiKey` 输出 `ApiKey([REDACTED])`，不实现 `Display`、`Serialize` 或 `Clone`。
- [ ] 在 `FolderId` 可用后扩展 `SyncError` 的强类型变体：`ConfigurationDrift`、`FolderPathMissing { folder: FolderId }`、`FolderMarkerMissing { folder: FolderId }`、`FolderScanFailed { folder: FolderId, status: u16 }`。
- [ ] Run: `cargo test -p textora-sync identifiers`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): add redacted Syncthing identifiers"`

### Task 4: 实现只读系统与数据库 API

**Files:**
- Create: `crates/sync/src/dto.rs`
- Create: `crates/sync/src/client.rs`
- Modify: `crates/sync/src/lib.rs`

- [ ] 先在 `client.rs` 单元测试中用最小 TCP mock server 断言 `X-API-Key`、方法、path/query，并覆盖 401、403、500、非法 JSON、超时。
- [ ] Run: `cargo test -p textora-sync client::tests`
- [ ] Expected: FAIL。
- [ ] 定义稳定输出类型：

```rust
pub struct InstanceInfo { pub version: semver::Version, pub device_id: DeviceId }
pub enum FolderPhase { Idle, Scanning, Syncing, Paused, Error, Unknown }
pub struct FolderStatus { pub phase: FolderPhase, pub need_bytes: u64, pub need_items: u64, pub completion_percent: f64, pub errors: u64 }
pub struct PendingDevice { pub device_id: DeviceId, pub name: Option<String> }
pub struct PendingFolder { pub folder_id: FolderId, pub label: Option<String>, pub offered_by: DeviceId }

impl SyncthingClient {
    pub fn new(endpoint: LoopbackEndpoint, api_key: ApiKey) -> Result<Self, SyncError>;
    pub fn probe(&self) -> Result<InstanceInfo, SyncError>;
    pub fn connections(&self) -> Result<Vec<DeviceId>, SyncError>;
    pub fn pending_devices(&self) -> Result<Vec<PendingDevice>, SyncError>;
    pub fn pending_folders(&self) -> Result<Vec<PendingFolder>, SyncError>;
    pub fn folder_status(&self, folder: &FolderId) -> Result<FolderStatus, SyncError>;
    pub fn folder_errors(&self, folder: &FolderId) -> Result<Vec<String>, SyncError>;
}
```

- [ ] `probe` 组合 `/rest/system/version` 与 `/rest/system/status`，在返回前验证版本范围。
- [ ] DTO 保持 private，并对 Syncthing 可新增字段使用 serde 的向前兼容默认行为；后续配置 mutation 的内部 round-trip DTO 必须保留未识别字段和数组成员，public 投影类型不得用于重建完整 JSON。
- [ ] Run: `cargo test -p textora-sync client::tests`
- [ ] Expected: PASS，且错误断言中无 API Key。
- [ ] Commit: `git commit -m "feat(sync): add read-only Syncthing REST client"`

### Task 5: 建立事件游标与纯状态归约

**Files:**
- Create: `crates/sync/src/state.rs`
- Modify: `crates/sync/src/client.rs`
- Modify: `crates/sync/src/lib.rs`

- [ ] 写状态表测试：禁用、连接中、离线、等待设备接受、等待 folder 接受、扫描中、同步中、空闲、暂停、错误、配置漂移互斥且优先级确定；远端离线归入等待而非错误。
- [ ] 写事件测试：`events_since(cursor, timeout)` 正确携带 `since`，空批次不回退 cursor，未知事件被忽略，事件 ID 回退/缺口产生全量刷新信号。
- [ ] Run: `cargo test -p textora-sync state`
- [ ] Expected: FAIL。
- [ ] 实现：

```rust
pub enum LibrarySyncState {
    Disabled,
    Connecting,
    Unavailable,
    AwaitingRemoteDevice,
    AwaitingRemoteFolder,
    Scanning,
    Syncing { remaining_bytes: u64, completion_percent: f64 },
    UpToDate,
    Paused,
    Error { summary: String },
    ConfigurationDrift,
}

pub struct LibraryObservation { /* typed inputs, no UI strings */ }
pub struct EventCursor(pub u64);
pub enum SyncEventKind { DeviceConnected, DeviceDisconnected, FolderStateChanged, ItemFinished, ConfigurationChanged, RemoteError }
pub enum SyncEvent {
    Remote { id: u64, kind: SyncEventKind },
    FullRefreshRequired,
}

pub fn reduce_library_state(observation: &LibraryObservation) -> LibrarySyncState;
pub fn events_since(&self, cursor: EventCursor, timeout_seconds: u16)
    -> Result<Vec<SyncEvent>, SyncError>;
```

- [ ] 将显示文案留给 app/ui，不让 `LibrarySyncState` 依赖 Textora UI。
- [ ] 增加无抖动、有上限的指数退避纯状态机；鉴权/版本错误停止自动重试，短暂连接错误达到上限后保留手动重试能力。
- [ ] Run: `cargo test -p textora-sync state`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(sync): reduce Syncthing library state"`

### Task 6: 建立 REST 契约夹具

**Files:**
- Create: `crates/sync/tests/fixtures/v2_1_1_read_api.json`
- Create: `crates/sync/tests/read_contract.rs`
- Create: `crates/sync/tests/real_syncthing.rs`

- [ ] 将 v2.1.1 读取端点的最小脱敏响应集中到 fixture，契约测试逐项反序列化并断言稳定字段。
- [ ] 添加 `#[ignore]` 的真实实例测试：从 `SYNCTHING_BIN` 启动隔离 home、随机端口，等待就绪后调用 `probe`；测试退出必须终止子进程并删除临时目录。
- [ ] Run: `cargo test -p textora-sync --test read_contract`
- [ ] Expected: PASS。
- [ ] Run: `cargo test -p textora-sync --test real_syncthing -- --ignored --nocapture`
- [ ] Expected: 未设置 `SYNCTHING_BIN` 时打印明确跳过原因；设置为 v2.1.1 二进制时 PASS。
- [ ] Commit: `git commit -m "test(sync): pin Syncthing 2.1.1 read contracts"`

### Task 7: 阶段验收

**Files:**
- Modify only if verification exposes a defect; return to the failing task first.

- [ ] Run: `cargo fmt --all -- --check`
- [ ] Run: `cargo test -p textora-sync`
- [ ] Run: `cargo check -p textora-app`
- [ ] Run: `cargo tree -p textora-sync`
- [ ] Expected: 全部通过；依赖树不含 `textora-app`、`textora-ui`、`winit`、Keychain 或 TLS backend。
- [ ] Review: 搜索 `unwrap()`、API Key 字段序列化、完整响应 body 进入错误文本的路径并清零。
- [ ] Commit any verification-only correction with a focused message; do not squash behavior and test history before review.
