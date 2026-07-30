# Syncthing 无界面控制面范围设计

## 1. 决策

当前 Syncthing 集成采用“完全剥离 UI”方案。

在 widget 体系重构完成前，本轮只实现可测试的同步控制面、资料库生命周期和文件安全能力，不提供任何用户可见的同步入口。普通用户暂时无法在 Textora 内配置或操作 Syncthing；这些能力通过 app 层程序化接口供集成测试和未来 UI adapter 使用。

本文是 `docs/plans/2026-07-17-syncthing-control-plane-design.md` 的范围补充。两者冲突时，以本文为准。原设计第 15 节 UI 设计和第 17 节 UI 实施阶段均延后。

## 2. 当前实施范围

### 2.1 `textora-sync`

保持原范围：

- loopback REST 地址、API Key、Device ID、Folder ID 等强类型。
- Syncthing `>=2.1.1,<2.2.0` 版本与能力检查。
- 只读状态、事件游标、退避与状态归约。
- device、folder、ignore 规则的细粒度配置读写和漂移比较。
- 串行命令 worker、独立事件 worker 与可取消 long poll。
- mock REST 契约和双 Syncthing 节点测试。

`textora-sync` 继续不依赖 `app`、`ui`、winit、Keychain 或文件编辑器状态。

### 2.2 app 层无界面编排

保留以下非视觉能力：

- `SyncConnectionStore`：保存 loopback endpoint 和 Keychain account 引用。
- `SyncSecretStore` / `MacKeychainSecretStore`：保存和读取 API Key。
- `SyncController`：启动/关闭 worker，接收后台结果，维护连接与资料库状态。
- `LibraryRegistry`：保存资料库根路径、folder ID、远端设备和配置所有权。
- 发布本地资料库、接收远端资料库、扫描、暂停、恢复、移除映射、显式注销和显式修复漂移的程序化方法。
- `AppEvent` 唤醒与后台结果 drain；网络失败不得影响编辑器主流程。

这些入口保持 `pub(crate)`，输入使用纯领域类型：

```rust
impl SyncController {
    pub(crate) fn configure_connection(
        &mut self,
        endpoint: LoopbackEndpoint,
        new_api_key: String,
    ) -> Result<RequestId, SyncControllerError>;

    pub(crate) fn publish_library(
        &mut self,
        root: PathBuf,
        remote: RemoteDeviceSpec,
    ) -> Result<RequestId, SyncControllerError>;

    pub(crate) fn accept_remote_library(
        &mut self,
        folder_id: FolderId,
        empty_root: PathBuf,
    ) -> Result<RequestId, SyncControllerError>;

    pub(crate) fn snapshot(&self) -> &SyncControllerSnapshot;
    pub(crate) fn drain_notices(&mut self) -> Vec<SyncNotice>;
}
```

具体方法可继续按单一职责拆分，但不得引入 Widget、菜单或渲染类型。目录路径由调用者传入；controller 负责规范化、空目录、嵌套和所有权校验，不负责打开目录选择器。

### 2.3 文件安全

保持原范围：

- `DiskRevision` 和带预期版本的原子保存。
- 资料库级文件监控和资料库外已打开文件监控。
- clean 文档自动重载并恢复可恢复的编辑位置。
- dirty 文档遇到外部修改时生成冲突副本并保留两份内容。
- 外部删除转为 dirty 未命名恢复文档。
- 明确重命名跟随，模糊重命名按删除保守处理。
- Textora 退出期间发生同步后，dirty snapshot 在下次启动时进行基线核对。

需要用户关注的结果暂存为 app 层纯数据：

```rust
pub(crate) enum FileSafetyNotice {
    CleanDocumentReloaded { path: PathBuf },
    ConflictCopyCreated { original: PathBuf, conflict: PathBuf },
    DocumentDetachedAfterDeletion { original: PathBuf },
    ConflictCopyFailed { original: PathBuf, message: String },
    AmbiguousRename { original: PathBuf },
}
```

本轮只保证 notice 可被测试、查询和 drain，不负责显示。失败时仍必须保留内存 buffer 和 dirty snapshot。

## 3. 明确延后的范围

本轮不得新增或修改任何 Syncthing 专用 UI，包括：

- 同步设置页、API Key 输入框和连接测试界面。
- 资料库发布、接收、扫描、暂停、恢复、漂移修复和移除面板。
- 目录选择器接线。
- Settings 菜单、侧边栏或命令面板中的同步入口。
- `WidgetAction`、`AppAction` 的同步 UI 变体和事件翻译。
- overlay、弹窗、确认框、toast、进度条和错误提示。
- 状态栏中的连接、同步进度、错误或冲突标签。
- 冲突文件定位和“打开 Web UI”按钮。

因此当前实现计划不得因 Syncthing 修改以下区域：

```text
crates/ui/**
crates/app/src/ui_shell.rs
crates/app/src/events.rs
crates/app/src/actions.rs
crates/app/src/app_renderer.rs
crates/app/src/native_menu.rs
crates/app/src/menu_handler.rs
crates/app/src/dispatch/chrome.rs
```

若后台接入确实需要调整现有通用 app 事件类型，应修改事件类型的实际定义文件，并保持它与 Widget 输入事件翻译解耦；不得以此为理由进入上述 UI 文件。

## 4. 延后 UI 的接口契约

widget 重构完成后，单独设计和实施 UI adapter。未来 UI 只能：

1. 从 `SyncControllerSnapshot` 和 `FileSafetyNotice` 构造纯 ViewModel。
2. 将用户意图映射到 `SyncController` 的程序化方法。
3. 在 app 层选择目录、打开 loopback Web UI 和执行危险操作确认。

未来 UI 不得直接访问 REST DTO、SyncthingClient、Keychain、LibraryRegistry 文件或 DocumentView 内部状态。当前阶段不为旧 widget API 编写临时 adapter，也不预设重构后的 widget action 形态。

## 5. 测试与验收

当前阶段通过 API 和状态测试证明能力，不以界面可操作为验收条件：

- controller 使用 fake store、fake secret store 和 fake sync service 测试连接、重试与状态推进。
- provisioning 使用临时目录和 fake/真实双节点测试发布、接收、扫描、暂停、恢复与漂移。
- 文件安全使用确定性的竞态和故障注入测试证明不静默覆盖。
- integration tests 直接调用程序化方法，不模拟点击或 WidgetAction。
- 最终运行 `./scripts/verify.sh`。
- diff 审查确认没有 Syncthing 相关的 `crates/ui`、菜单、overlay、status bar 或 widget action 改动。

## 6. 对现有实施计划的修改要求

- Phase 1 保持不变。
- Phase 2 删除设置 widget、action 翻译、overlay、菜单入口和状态栏任务；补充 controller 程序化 API 与 snapshot/notice 测试。
- Phase 3 删除资料库 widget、action 翻译、overlay、rfd 和菜单入口任务；保留并强化无界面 controller/provisioning 测试。
- Phase 4 删除状态栏和冲突展示改动；将冲突计数与文件安全结果改为 app 层 notice/state 测试。
- 总览删除“只读 UI”“资料库 UI”等阶段门槛，增加 UI 禁改清单和延后说明。
- 原控制面设计将 UI 章节标为 deferred，避免与本文产生两套当前范围。
