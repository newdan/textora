# Syncthing Integration Program Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Textora 通过本机 Syncthing 2.1.x REST API 管理资料库同步，并在外部文件变化时保证编辑内容不被静默覆盖。

**Architecture:** 实施拆为四个可独立审查的阶段：先建立 `textora-sync` 适配层，再接入本机连接和只读状态，然后增加设备/资料库控制，最后改造编辑器文件安全。Textora 不管理 Syncthing 进程；`ui` 只接收纯数据，所有 HTTP、文件监控和哈希工作均在后台执行。

**Tech Stack:** Rust 2024、Syncthing REST API 2.1.x、reqwest blocking 0.13、serde/serde_json、semver、security-framework 3.7（macOS Keychain）、notify 8、blake3、winit EventLoopProxy。

## Global Constraints

- 产品名是 `textora`；新 crate 包名必须是 `textora-sync`。
- 本机 Syncthing 支持范围固定为 `>= 2.1.1, < 2.2.0`。
- REST 地址只允许 `http://127.0.0.1`、`http://localhost` 或 `http://[::1]`。
- API Key 只能存入 macOS Keychain，禁止写入 TOML、日志、Debug 输出或错误文本。
- Textora 不启动、停止、重启、重置或升级 Syncthing。
- Textora 不修改发现、中继、NAT、监听端口和全局升级设置。
- `crates/ui` 不依赖 `app`、`DocumentView` 或 Syncthing DTO；app 层负责纯数据映射。
- 互斥状态必须使用 enum，不得组合多个 bool。
- 单任务修改超过 3 个文件时，必须继续拆分子任务并在每个子任务后编译。
- Bug/行为修改严格执行 TDD：先看到针对性测试失败，再写实现。
- 每个阶段提交前必须执行对应 crate 测试和 `cargo check -p textora-app`。
- 全部阶段结束后必须执行 `./scripts/verify.sh`。
- 设计依据：`docs/plans/2026-07-17-syncthing-control-plane-design.md`。

---

## Phase Documents

1. [Phase 1: API Client and Contract Harness](./2026-07-17-syncthing-phase1-api-client.md)
2. [Phase 2: Local Connection and Read-Only Status](./2026-07-17-syncthing-phase2-local-connection.md)
3. [Phase 3: Device and Library Control Plane](./2026-07-17-syncthing-phase3-library-control.md)
4. [Phase 4: File Monitoring and Conflict Safety](./2026-07-17-syncthing-phase4-file-safety.md)

## Shared Interface Contract

Later phases consume these names exactly; changing one requires updating all remaining plan documents before implementation continues.

```rust
// textora-sync public API
pub struct LoopbackEndpoint;
pub struct ApiKey;
pub struct DeviceId;
pub struct FolderId;
pub struct InstanceInfo;
pub struct FolderStatus;
pub enum FolderPhase;
pub struct EventCursor;
pub struct LibraryObservation;
pub struct PendingDevice;
pub struct PendingFolder;
pub struct DeviceConfig;
pub struct FolderConfig;
pub struct ConfigurationDifference;
pub enum LibrarySyncState;
pub enum SyncCommand;
pub enum SyncResult;
pub enum SyncEvent;
pub enum SyncEventKind;
pub enum SyncError;
pub struct SyncthingClient;
pub struct SyncService;

// app persistence and orchestration
pub(crate) struct SyncConnectionStore;
pub(crate) trait SyncSecretStore;
pub(crate) struct LibraryRegistry;
pub(crate) struct LibraryRecord;
pub(crate) struct SyncController;

// file safety
pub struct DiskRevision;
pub struct FileIdentity;
pub enum SaveError;
pub(crate) struct LibraryFileMonitor;
pub(crate) enum ExternalDocumentChange;
```

## Cross-Phase Gates

### Task 1: Complete Phase 1 before app integration

- [ ] Run `cargo test -p textora-sync`.
- [ ] Expected: all unit and mock REST contract tests pass; the real Syncthing test remains ignored unless `SYNCTHING_BIN` is set.
- [ ] Run `cargo check -p textora-app`.
- [ ] Expected: exit 0 with no new warning from `textora-sync`.
- [ ] Review that `textora-sync` has no dependency on `ui`, `app`, `winit` or macOS Keychain.

### Task 2: Complete Phase 2 before any config mutation

- [ ] Run `cargo test -p textora-sync` and `cargo test -p textora-app --lib -- sync_`.
- [ ] Expected: connection persistence, Keychain abstraction, worker wakeup and read-only UI tests pass.
- [ ] Manually verify an invalid API Key is never printed and leaves Textora editing usable.

### Task 3: Complete Phase 3 before file watcher replacement

- [ ] Run `SYNCTHING_BIN=/path/to/syncthing cargo test -p textora-sync --test two_node -- --ignored --nocapture`.
- [ ] Expected: two v2.1.1 nodes register a folder, transfer a fixture, pause/resume and report completion.
- [ ] Run `cargo test -p textora-app --lib -- library_sync`.
- [ ] Expected: library registry, path ownership, drift detection and command mapping tests pass.

### Task 4: Complete Phase 4 and final verification

- [ ] Run `cargo test -p textora-core --lib -- file`.
- [ ] Run `cargo test -p textora-app --lib -- external_change`.
- [ ] Run `cargo test -p textora-app --lib -- conflict`.
- [ ] Run `cargo check -p textora-app`.
- [ ] Run `./scripts/verify.sh`.
- [ ] Expected: every command exits 0; dirty-vs-external-change, delete recovery and rename tests demonstrate no silent overwrite.

## Delivery Rule

Each phase is a separate review boundary and may be rejected without invalidating prior phases. Do not start Phase 4 while Phase 3 storage identifiers or public `textora-sync` interfaces are unstable.
