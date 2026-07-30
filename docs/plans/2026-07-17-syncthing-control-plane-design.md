# Textora Syncthing 控制面集成设计

## 1. 目标

为 Textora 引入“资料库”级目录同步。用户在本机独立运行 Syncthing，Textora 通过本机 REST API 注册远端设备和资料库、驱动扫描与暂停/恢复，并在应用内展示同步状态。

远端是用户自建、常在线且可信的 Syncthing v2.1.1 节点。远端可以明文保存资料库。设备与资料库需要用户在远端 Syncthing Web UI 中手动接受。

Textora 不实现 Syncthing 协议，不打包或托管 Syncthing 进程。Textora 退出后，本机 Syncthing 继续独立同步。

## 2. 已确认的产品决策

- 同步单位是资料库；一个资料库对应一个本地目录和一个 Syncthing folder ID。
- 资料库中的用户文件默认全部同步，包括 Markdown、文本、图片、PDF 和其他附件。
- Textora 连接用户本机独立运行的 Syncthing，而非内置 sidecar。
- 本机 Syncthing 通过 BEP 与用户自建的可信远端节点同步。
- Textora 只配置本机；用户在远端 Web UI 接受设备与资料库。
- 本机 Syncthing 可以在 Textora 退出后继续运行和同步。
- dirty 文档遇到磁盘外部修改时自动生成冲突副本，保证两份内容都保留。
- 打开的文件被远端删除时，保留为未命名恢复文档。
- 首版只控制本机 loopback REST API，不连接公网 REST API。

## 3. 非目标

首版不包含：

- 实现 Block Exchange Protocol、发现协议或中继协议。
- 管理 Syncthing 二进制的安装、启动、退出、升级或崩溃重启。
- 自动配置远端 Syncthing REST API。
- 账户、租户、配额、计费或 Textora 官方托管服务。
- 不可信设备加密。
- 自动合并文本冲突。
- 修改 Syncthing 的全局发现、中继、监听端口、NAT 或升级策略。
- 在 Textora 退出时自动暂停资料库。

## 4. 总体架构

```text
Textora app
  ├── Workspace / DocumentView
  ├── LibraryFileMonitor
  └── textora-sync
       ├── SyncthingApiClient
       ├── SyncCommandWorker
       ├── SyncEventSource
       ├── SyncStateReducer
       └── SyncConfigDiff
              │ localhost REST API
              ▼
       本机独立 Syncthing 2.1.x
              │ BEP
              ▼
       用户自建远端 Syncthing v2.1.1
```

### 4.1 crate 边界

新增 `crates/sync`，包名 `textora-sync`。

`textora-sync` 不依赖 `app`、`ui`、`DocumentView` 或 winit。它只负责：

- Syncthing REST API 请求与响应 DTO。
- 版本和能力检查。
- 设备、资料库和忽略规则的配置差异与显式写入。
- 同步事件读取与领域状态归约。
- 命令工作线程、事件工作线程及 channel 通信。

`app` 负责：

- 将资料库、打开文档和同步状态关联起来。
- 将同步结果转换为 AppEffect。
- 文件系统变更、文档重载、冲突副本和删除恢复。
- 通过 EventLoopProxy 将后台结果送回主线程。
- 将同步领域状态映射成 UI 的纯数据输入。

`ui` 只定义和渲染纯数据输入，例如 `LibrarySyncViewModel`，绝不依赖 `app` 状态或 Syncthing DTO。

### 4.2 对外领域类型

`textora-sync` 对外暴露语义化类型，不泄露 Syncthing JSON：

```text
SyncthingConnectionSpec
RemoteDeviceSpec
LibrarySyncSpec
SyncCommand
SyncResult
SyncEvent
SyncRuntimeState
LibrarySyncState
SyncError
ConfigurationDifference
```

互斥运行状态使用 enum 表达，不组合多个 bool。

## 5. 本机 Syncthing 接入

### 5.1 首次连接

1. Textora 默认使用 `http://127.0.0.1:8384`。
2. 用户从本机 Syncthing Web UI 复制 API Key。
3. Textora 调用 ping、version 和 status 接口验证连接。
4. Textora 获取本机 Device ID。
5. API Key 写入 macOS Keychain；配置文件只保存 loopback 地址和 Keychain 引用。

首版只接受明文 HTTP loopback 地址：`127.0.0.1`、`localhost` 或 IPv6 loopback。拒绝局域网和公网主机，避免 API Key 离开本机。自定义 HTTPS Web UI 不在首版范围内。

### 5.2 版本策略

首版验证范围：

```text
>= 2.1.1 且 < 2.2.0
```

本机版本超出范围时进入 `UnsupportedVersion`，Textora 保持编辑功能，但禁用配置写入。远端只通过 BEP 与本机通信，不要求 patch 版本完全相同。

Syncthing 的版本策略把可能影响 REST 包装器的变化归入 minor 版本，因此不能简单接受所有 v2 版本。

## 6. 配置所有权与漂移

本机 Syncthing 是共享外部资源，可能由用户或其他工具修改。Textora 不把 Syncthing 配置视为自己的私有状态。

Textora 保存以下映射：

- `library_id`
- 资料库规范化根路径
- Syncthing `folder_id`
- 关联的远端 Device ID
- Textora 是否创建过该 folder/device 映射
- Textora 管理的忽略规则版本

Syncthing 实际配置是运行时事实来源。

### 6.1 写入原则

- 只使用 `/rest/config/folders/*id*`、`/rest/config/devices/*id*` 等细粒度端点。
- 禁止整体替换全部 Syncthing 配置。
- 所有配置写操作在单一工作线程串行执行。
- 创建资源时先读取 Syncthing 默认模板，再填写最小必要字段。
- 修改已有资源前读取完整对象并计算差异。
- PATCH 的数组字段会整体替换，因此必须保留所有未知数组元素。
- 非 Textora 创建的设备或资料库需要用户确认差异后才能修改。
- 写入后重新读取并验证结果。

Syncthing REST API 没有配置事务或乐观锁。Textora 通过减少写操作、串行化自身操作和不后台修复来缩小竞态窗口，但不宣称能阻止其他客户端在同一时刻修改配置。

### 6.2 配置漂移

Textora 登记的 folder/device 被删除或关键字段变化时，资料库进入 `ConfigurationMismatch`。

UI 提供：

- 查看差异。
- 显式修复配置。
- 从 Textora 移除资料库映射。

Textora 不在后台反复写回用户主动删除或修改的配置。

## 7. 远端设备配对

用户提供：

- 远端 Device ID。
- 显示名称。
- 静态同步地址，例如 `tcp://sync.example.com:22000`。

Textora 在本机 Syncthing 注册远端设备，然后展示：

- 本机 Device ID。
- 远端 Web UI 操作说明。

用户在远端 Web UI 添加或接受本机设备。Textora通过连接和 pending 状态判断远端是否完成接受。

首版不要求 Textora 修改本机 Syncthing 的发现、中继或全局监听设置。是否启用这些能力由用户现有 Syncthing 配置决定。

## 8. 资料库生命周期

### 8.1 发布本地资料库

1. 用户选择或创建本地目录。
2. Textora 检查目录未嵌套于另一个已登记资料库，也不包含其他资料库根目录。
3. Textora 生成稳定 `library_id` 和 `folder_id`。
4. Textora 持久化映射为 `Provisioning`。
5. Textora 添加 `sendreceive` folder，并关联本机与远端设备。
6. Textora 写入必要的临时文件忽略规则。
7. Textora请求初始扫描。
8. Textora 展示 folder ID、资料库名称和建议远端路径。
9. 用户在远端 Web UI 接受资料库。

### 8.2 接入远端已有资料库

1. 用户在远端把资料库共享给本机 Device ID。
2. Textora 通过 pending-folder API 发现邀请。
3. 用户选择一个空的本地目录。
4. Textora 接受资料库并写入本地路径。
5. Textora 写入必要的临时文件忽略规则。
6. Syncthing 开始首次下载。

首版不把两个已有非空目录直接关联到同一 folder ID。用户必须为接入远端已有资料库选择空目录，避免首次连接就产生不可预测的合并和冲突。

### 8.3 移除资料库

默认操作只移除 Textora 映射，不修改 Syncthing，也不删除文件。

“同时从本机 Syncthing 取消注册”是独立、显式、二次确认的操作。该操作只移除 Syncthing folder 配置，绝不删除资料库目录或文件。

## 9. 同步状态

资料库主状态：

```text
Disabled
Connecting
AwaitingRemoteDevice
AwaitingRemoteFolder
Scanning
Syncing
UpToDate
Paused
ConfigurationMismatch
Error
Unavailable
```

语义：

- `AwaitingRemoteDevice`：远端尚未接受本机设备。
- `Connecting`：Textora 正在连接本机 Syncthing REST API 并读取初始状态。
- `AwaitingRemoteFolder`：设备可用，但资料库尚未在远端接受或共享。
- `Scanning`：Syncthing 正在建立或更新本地索引。
- `Syncing`：存在待上传或下载项。
- `UpToDate`：本地与集群全局索引一致。
- `Paused`：folder 或远端设备被暂停。
- `ConfigurationMismatch`：Textora 映射与 Syncthing 实际配置不一致。
- `Unavailable`：本机 Syncthing REST API 不可用。

状态由连接信息、pending device/folder、folder 状态、completion、错误和事件归约得到。UI 不接收 Syncthing 原始状态字符串。

## 10. 驱动同步

Textora 提供：

- 立即扫描资料库。
- 暂停资料库或远端设备。
- 恢复资料库或远端设备。
- 查看连接、完成度、待同步量和错误。
- 打开本机 Syncthing Web UI。

“立即扫描”只调用扫描 API。Syncthing 会在发现新索引后自动收敛，不存在单独的 push/pull 命令。

“立即扫描”不会擅自恢复用户在 Syncthing Web UI 暂停的资料库。恢复是单独的显式动作。

远端离线是正常等待状态，不作为同步失败。

## 11. 同步范围与忽略规则

资料库目录下的用户文件默认全部同步。Textora 的资料库 ID、工作区、标签页、滚动位置、dirty snapshot、API 连接和 UI 状态全部保存在资料库之外。

Textora 不创建资料库内 `.textora` 元数据目录。

### 11.1 原子保存临时文件

现有原子保存临时文件改为：

```text
.textora-save-<pid>-<counter>-<basename>.tmp
```

注册资料库时，通过 ignores API 读取并保留用户现有 `.stignore`，加入唯一强制规则：

```text
// BEGIN TEXTORA MANAGED
(?d).textora-save-*.tmp
// END TEXTORA MANAGED
```

Textora 不自动忽略 `.DS_Store`、构建产物或其他用户文件。用户可以继续在 Syncthing Web UI 或 `.stignore` 中管理自己的规则。

管理块缺失时进入配置不一致提示，但不后台反复写回。

## 12. 文件变更与编辑安全

### 12.1 两条事件通道

- Syncthing `/rest/events` 用于同步状态、连接、扫描和错误。
- 本地资料库文件系统监控用于文档内容变更、删除和重命名。

文件安全不能仅依赖 Syncthing 事件，因为 Git、终端和其他编辑器也能修改资料库。

现有只监控当前干净文件的两秒 mtime 轮询不能满足该需求，需要替换为资料库级监控。

### 12.2 磁盘版本

每个有路径的打开文档记录：

```text
DiskRevision {
    path,
    size,
    mtime,
    content_hash,
}
```

保存 dirty 文档前，重新读取并计算磁盘文件哈希，与基线比较。mtime、大小和文件标识只作为快速诊断信息，不能代替保存前内容哈希。

保存必须是带预期版本的事务：捕获编辑缓冲区 revision 和 DiskRevision，后台完成磁盘哈希后，在原子 rename 前再次核对目标文件标识、大小和 mtime。任一条件变化都返回 `ConcurrentModification`，转入冲突分叉流程。后台检查期间若编辑缓冲区 revision 已变化，本次结果作废并重新执行保存前检查。

该策略消除正常 Syncthing 原子替换和绝大多数外部写入竞态。由于普通文件系统不提供跨进程、强制遵守的内容 compare-and-swap，无法对不遵守文件锁且刻意保留全部元数据的第三方原地写入作绝对保证；保存后文件监控和下一次哈希检查仍作为后续保护。

现有同目录临时文件、fsync 和原子 rename 保存策略继续保留。

### 12.3 干净文档外部修改

- 自动重新加载。
- 尽量恢复光标、选区和滚动锚点。
- 显示短暂的“已同步远端修改”提示。
- 更新 DiskRevision。

### 12.4 dirty 文档外部修改

1. 禁止覆盖原路径。
2. 将当前编辑缓冲区原子保存为同目录冲突副本。
3. 当前标签页重新绑定到冲突副本，用户可继续编辑。
4. 原路径加载磁盘新版本，并在相邻标签页打开。
5. 两个文件都作为普通资料库内容继续同步。

冲突副本使用 `create_new` 和递增后缀防止重名。只有冲突副本完成写入和 fsync 后才能重新绑定标签页。若磁盘满、权限不足或其他原因导致副本创建失败，Textora 保持当前缓冲区和 dirty snapshot，禁止覆盖原路径并显示可重试错误。

冲突副本命名：

```text
<stem>.textora-conflict-<YYYYMMDD-HHMMSS>-<local-device-short-id>.<ext>
```

保存前哈希检查是文件监控事件丢失时的最终保护。

### 12.5 外部删除

打开文件被删除时：

- 保留当前缓冲区。
- 清除原路径和 DiskRevision。
- 标签页改名为“恢复：原文件名”。
- 标记为 dirty 未命名文档。
- 不自动写回原路径。

干净和 dirty 文档使用相同行为。

### 12.6 外部重命名

- 文件系统能明确报告 rename 时更新标签页路径。
- delete + create 事件批次中，仅当存在唯一的同内容候选文件时跟随重命名。
- 无法唯一判断时按删除处理，优先保留内容。

### 12.7 冲突列表

Textora 检测 Syncthing `sync-conflict` 和自身 `textora-conflict` 文件，在资料库状态中显示冲突数量并提供定位功能。首版不自动合并或删除冲突文件。

## 13. 异步执行

所有 HTTP、文件读取和哈希操作不得阻塞 winit 主线程。

`textora-sync` 内部使用：

- 一个串行命令工作线程处理短 REST 请求和配置写入。
- 一个事件工作线程进行可取消的有限时长 long poll。
- 标准 channel 传递命令、结果和领域事件。

`app` 使用 EventLoopProxy 唤醒主线程。`textora-sync` 不依赖 winit。

事件流记录最后事件 ID。Syncthing 重启、事件 ID 失效或发现事件缺口时，执行一次全量状态刷新，再继续订阅。

`db/status` 是高开销接口。同步活跃时低频读取，空闲时进一步降低频率；正常状态更新优先依赖事件。

## 14. 错误模型与恢复

```text
ConnectionRefused
AuthenticationFailed
UnsupportedVersion
ConfigurationMismatch
RemoteOffline
FolderPathMissing
FolderMarkerMissing
PermissionDenied
DiskFull
FolderScanFailed
ApiProtocolError
```

处理原则：

- `RemoteOffline` 显示等待，不弹错误框。
- 短暂连接和 API 失败使用有上限的指数退避。
- API Key 错误停止自动重试，等待用户重新配置。
- 路径、权限、磁盘和扫描错误归属到具体资料库。
- 不自动调用 reset、restart、shutdown 或 upgrade。
- 所有非破坏性操作可重试并保持幂等。
- 同步不可用不影响 Textora 打开、编辑和保存非冲突文件。

## 15. UI 设计边界

### 15.1 设置页

“设置 → 同步”显示：

- 本机 Syncthing loopback 地址。
- API Key 是否已配置，不回显明文。
- 连接测试。
- 本机 Device ID。
- Syncthing 版本。
- 打开本机 Web UI。
- 断开 Textora 控制连接。

### 15.2 资料库操作

- 启用同步。
- 立即扫描。
- 暂停/恢复。
- 查看远端连接。
- 查看错误和冲突。
- 查看或修复配置差异。
- 从 Textora 移除。
- 显式从本机 Syncthing 取消注册。

### 15.3 简要状态

```text
未连接
等待远端
扫描中
同步中 38%
已同步
已暂停
存在 3 个错误
```

`app` 从同步领域状态构造 `LibrarySyncViewModel`。`ui` 不访问 Syncthing API、配置或 app 层状态。

## 16. 测试策略

### 16.1 textora-sync 单元测试

- API DTO 解析与错误映射。
- API Key 和版本验证。
- 状态归约器。
- folder/device 配置差异。
- 数组字段保留。
- `.stignore` 管理块的插入、保留、缺失与冲突。
- 退避状态机。
- 事件 ID 恢复。
- 资料库路径规范化和嵌套拒绝。

### 16.2 app 单元与集成测试

- 后台结果通过 EventLoopProxy 应用，不阻塞 UI。
- 干净文件自动重载。
- dirty 文件生成冲突副本并重新绑定。
- 保存前哈希发现遗漏的外部变更。
- 外部删除转换为恢复文档。
- 明确 rename 跟随。
- 不确定 rename 降级为恢复文档。
- 冲突文件发现和计数。

### 16.3 双 Syncthing 节点测试

测试基座启动两个使用独立临时 config/data 目录和端口的 Syncthing v2.1.1 节点，覆盖：

- 新资料库首次同步。
- 远端已有资料库首次下载。
- 双向修改、删除和重命名。
- 同时修改产生冲突。
- 远端离线后恢复。
- 本机 Syncthing 重启与事件续接。
- API Key 错误。
- 配置漂移。
- Unicode 文件名。
- 二进制附件和大文件。

## 17. 分阶段实施

### 阶段 1：API 契约和测试基座

- 建立 Syncthing v2.1.1 API fixtures。
- 建立双节点集成测试工具。
- 明确版本和能力探测。

### 阶段 2：只读连接与状态

- 新增 `textora-sync`。
- app 层实现 Keychain 连接配置。
- 获取版本、Device ID、连接和 folder 状态。
- 建立后台命令与事件通道。

### 阶段 3：设备与资料库注册

- 远端设备注册。
- 发布本地资料库。
- 接受远端已有资料库。
- 忽略规则管理。
- 配置差异与显式修复。

### 阶段 4：设置与资料库 UI

- 同步设置页。
- 资料库同步操作。
- 状态、错误和冲突显示。
- app 到 ui 的纯数据映射。

### 阶段 5：文件安全

- 资料库级文件监控。
- DiskRevision 和保存前哈希。
- 自动重载、冲突分叉、删除恢复和 rename 处理。

### 阶段 6：硬化与全面验证

- 离线、重启、磁盘和权限故障。
- 配置竞态和兼容性。
- 性能和大文件验证。
- 执行 `./scripts/verify.sh`。

每个阶段独立拆分任务，修改前先写失败测试，提交前确保编译通过。涉及多模块的阶段先固定接口与协议，避免 UI、app 和同步适配层交叉依赖。

## 18. 参考资料

- [Syncthing REST API](https://docs.syncthing.net/dev/rest.html)
- [Syncthing Config Endpoints](https://docs.syncthing.net/rest/config.html)
- [Syncthing Ignore Patterns](https://docs.syncthing.net/users/ignoring.html)
- [Syncthing Release Policy](https://docs.syncthing.net/users/releases.html)
- [Syncthing Folder Status API](https://docs.syncthing.net/v1.30.0/rest/db-status-get.html)
- [Syncthing Security Principles](https://docs.syncthing.net/users/security)
- [Syncthing Block Exchange Protocol v1](https://docs.syncthing.net/specs/bep-v1.html)
