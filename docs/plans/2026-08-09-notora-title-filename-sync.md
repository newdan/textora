# Notora 标题与实体文件名双向同步修改方案

日期：2026-08-09

状态：分阶段实施中；新笔记双向同步主链路已完成，旧工作区批量迁移 UI 待后续阶段

关联规格：

- [`2026-07-30-notora-product-design.md`](../specs/2026-07-30-notora-product-design.md)
- [`2026-08-03-notora-editor-area-design.md`](../specs/2026-08-03-notora-editor-area-design.md)
- [`2026-08-03-notora-note-encryption-design.md`](../specs/2026-08-03-notora-note-encryption-design.md)

当前实现快照：

- 已完成文件名规范化、稳定重复编号、no-replace 移动和仅大小写安全改名；
- 已完成 `UpdateTitle` 复合命令、title revision、dirty 保存屏障和打开文档路径重绑；
- 已完成 Finder 类型化 rename hint、扫描前后 NoteId 路径差异传播及反向标题更新；
- 已完成未提交路径操作的启动恢复；歧义状态停止扫描并要求人工处理；
- 已达到相对链接发布门槛：解析到受影响目标时阻止改名，不做字符串级替换；
- 已删除应用层独立改文件名入口；
- 旧 schema 笔记保持 `LegacyUnmanaged`，批量迁移预览、确认和结果 UI 尚未开放。

## 1. 目标

普通 Notora 工作区笔记建立持续的标题—路径约束：

```text
Notora 内修改标题 ──> 标题驱动实体文件名
Finder 中修改文件名 ──> 文件名反向更新 Notora 标题
移动目录 ────────────> 只改变目录；必要时重新分配重复序号
正文 H1 / 根节点 ─────> 首次初始化后继续与 Notora 标题独立
```

最终消除工作区中大量长期停留在 `未命名 N.md`、`未命名 N.txt` 和
`未命名 N.mmap.md` 的实体文件，同时满足：

- 同一目录允许多篇 Notora 标题相同的笔记；
- 文件名冲突时不覆盖任何已有文件；
- 标题、路径、全文索引和打开文档路径最终一致；
- Finder 外部改名保留稳定 `NoteId`；
- dirty、自动保存、外部修改和崩溃恢复期间不产生旧路径复活或数据丢失；
- 加密笔记继续使用不泄漏标题的随机稳定文件名。

## 2. 当前基线与根因

### 2.1 新建笔记直接固化占位路径

`notora-core::note_command` 当前在创建时立即寻找 `未命名 N`，写入实体文件后把该路径
存入 catalog。后续标题提交只更新 catalog 元数据，没有文件名物化阶段，因此占位路径会
永久保留。

### 2.2 标题更新与文件重命名是两条独立链路

当前 `MetadataMutation::SetTitle` 只调用 `Catalog::update_note_title`。文件重命名则通过
`NoteCommand::Rename` 单独执行。两条链路没有共同的 revision、保存屏障或失败恢复协议，
不能直接串接为可靠的“先改标题、再尽力重命名”。

### 2.3 watcher 丢失重命名语义

`WorkspaceFileMonitor` 当前只把 notify 事件压缩成 `WorkspaceFileBatch { relative_paths }`：

- `RenameMode::From`、`To`、`Both` 和 tracker 信息被丢弃；
- 重扫只能用旧路径、内容 hash 和新路径做保守推断；
- 两篇空文档或正文完全相同的笔记无法通过 hash 唯一确认身份；
- 无法区分 Notora 自己触发的文件移动与 Finder 外部改名。

### 2.4 当前移动保存屏障没有覆盖重命名

目录移动对已打开 dirty 文档会先强制保存，重命名则可直接执行。标题持续驱动文件名后，
每次标题提交都可能触发路径变化，必须统一走同一套保存与 relocation 协议。

### 2.5 既有规范需要改写

当前编辑区规格明确写着“修改标题不自动修改文件名”。本方案生效前必须先更新规格，
并明确标题真源、Finder 反向同步、正文标题独立和加密例外。

## 3. 范围与非目标

### 3.1 本次范围

- 普通工作区笔记的标题—文件名双向同步；
- 新建、标题提交、移动、Finder 改名和首次扫描导入的命名规则；
- 目录内重复名的稳定分配；
- 标题/path/catalog/FTS 的一致提交；
- 打开文档路径、自动保存和外部冲突协调；
- watcher 的类型化文件变化协议；
- 既有 `未命名 N` 文件的预览式迁移；
- 标准 Markdown 相对链接的改名安全门槛。

### 3.2 非目标

- 不让 Markdown H1 或 Mindmap 根节点持续驱动文件名；
- 不新增反向链接列表、知识图谱或 wikilink 产品功能；
- 不让外部文件会话 `ExternalFileSession` 进入工作区命名协议；
- 不改变加密笔记必须使用不含语义文件名的安全要求；
- 不静默批量改写现有工作区路径；
- 不依赖 macOS 扩展属性作为唯一 `NoteId` 存储，因为它不能可靠跨平台或跨同步工具保留。

## 4. 核心产品决策

### 4.1 Notora 标题是普通工作区笔记的命名真源

应用内标题提交成功后，实体文件名必须由规范化后的 Notora 标题派生。标题提交只发生于
Enter 或失焦，不跟随每次键入，避免连续产生路径抖动。

Markdown H1、Mindmap 根节点和 TXT 正文标题投影继续遵守既有首次初始化规则：

- `AwaitingFirstCommit` 阶段仍由标题栏或正文首次提交竞争；
- 初始化完成后正文结构与 Notora 标题永久独立；
- 后续修改 H1 或根节点不会移动文件；
- 标题栏后续修改会更新实体文件名，但不改正文。

### 4.2 Finder 外部改名代表用户的明确意图

Finder 改名不能被 Notora 自动改回。识别为外部改名后：

- 同目录改名：更新 `relative_path`，并把新文件 stem 按字面写入 Notora 标题；
- 只移动目录且 basename 不变：只更新路径，标题不变；
- 移动时 Finder 改变 basename：按外部改名处理；
- 正文 H1、Mindmap 根节点和标签不变；
- 已打开文档同步到新路径。

外部文件名中的 `(2)`、`(最终版)` 等文本一律按字面进入标题。只有 Notora 自己分配并记录
的数字才是非标题的重复名消歧信息，不能猜测用户输入的括号是否应被删除。

### 4.3 首次发现的工作区文件以文件 stem 初始化标题

对 catalog 中从未出现、由 Finder 拷入或由其他工具新建的受支持文件：

- 以文件 stem 初始化 Notora 标题；
- 不因正文 H1 与 stem 不同而立即重命名用户文件；
- 直接进入正文标题独立状态；
- 后续在 Notora 中修改标题时，再由标题驱动文件名。

该规则取代“首次扫描优先用 H1 生成 Notora 标题”的旧行为，避免扫描工作区时擅自重排用户
已有路径。

### 4.4 加密笔记是强制例外

加密笔记使用 `Opaque` 命名模式，文件名由随机稳定标识或不含语义的 `NoteId` 编码派生：

- 标题修改不改变密文实体文件名；
- Finder 改名只更新受控路径，不从 stem 反推加密标题；
- UI 必须明确加密实体名为安全实现细节；
- 本方案不得弱化加密规格中“路径不能泄漏标题”的要求。

## 5. 目标领域模型

### 5.1 类型驱动的命名状态

禁止用多个布尔字段表达命名模式，建议引入：

```rust
pub enum NoteFileNameBinding {
    LegacyUnmanaged,
    TitleBound { disambiguator: u32 },
    Opaque,
}
```

- `LegacyUnmanaged`：schema 升级后的既有普通笔记；迁移确认前不自动改名；
- `TitleBound`：普通工作区笔记；标题与文件名双向同步；
- `Opaque`：加密笔记；实体名不得包含标题。

`LegacyUnmanaged` 是迁移过渡态，不是长期产品模式。首次扫描发现的新普通文件直接进入
`TitleBound { disambiguator: 1 }`，标题采用实际 stem。

### 5.2 标题 revision

异步标题提交需要显式 revision，避免快速连续提交或旧 worker 结果覆盖新标题：

```rust
pub struct UpdateNoteTitleRequest {
    pub note_id: NoteId,
    pub expected_title_revision: u64,
    pub title: String,
}
```

Catalog 为每篇笔记持久化单调递增的 `title_revision`。命令发现 revision 过期时返回稳定的
`StaleTitleRevision`，产品层只保留并重试最新草稿，不覆盖更新结果。

### 5.3 可恢复的 relocation 意图

SQLite 与文件系统不能组成真正的跨系统原子事务。Catalog 需要记录短生命周期的路径操作：

```rust
pub enum NotePathOperationKind {
    TitleRename,
    DirectoryMove,
    ExternalRename,
    Migration,
}

pub struct PendingNotePathOperation {
    pub operation_id: Uuid,
    pub note_id: NoteId,
    pub kind: NotePathOperationKind,
    pub source_relative_path: PathBuf,
    pub target_relative_path: PathBuf,
    pub expected_title_revision: u64,
}
```

操作记录用于：

- 崩溃后判断磁盘已移动但 catalog 未提交，还是尚未移动；
- watcher 区分内部移动确认与 Finder 外部改名；
- catalog 更新失败时回滚路径；
- 避免内部重复名后缀被反向写进 Notora 标题。

## 6. 文件名派生与重复名规则

### 6.1 纯函数接口

在 `notora-core` 建立不访问文件系统的纯命名模块：

```rust
pub fn normalize_title_file_stem(title: &str) -> String;

pub fn title_bound_file_name(
    normalized_stem: &str,
    kind: DocumentKind,
    disambiguator: u32,
) -> String;
```

示例：

```text
项目计划 + Markdown + 1 -> 项目计划.md
项目计划 + Markdown + 2 -> 项目计划 (2).md
项目计划 + Mindmap + 2 -> 项目计划 (2).mmap.md
```

### 6.2 规范化规则

- 使用 Unicode NFC；
- 保留中文和正常 Unicode，不转拼音；
- `/`、`\`、`:`、`*`、`?`、`"`、`<`、`>`、`|` 和控制字符替换为空格或连字符；
- 合并连续空白，移除首尾空白；
- 移除尾部句点和空格；
- 拒绝 `.`、`..` 和跨平台保留设备名；
- 规范化后为空时回退为 `无标题`；
- 使用语义常量限制 stem 长度，按 grapheme 安全截断；
- 扩展名由 `DocumentKind` 决定，标题不能改变文档类型；
- `.mmap.md` 必须作为完整复合扩展名处理。

### 6.3 目录内分配

分配目标时同时检查 Catalog 和真实文件系统：

1. 当前笔记已经占用同一派生文件名时直接复用；
2. 尝试无后缀名称；
3. 已占用则从 `2` 开始寻找第一个可用编号；
4. 普通文件、目录、符号链接和大小写折叠后的等价路径都算占用；
5. 外部进程在检查后抢占目标时，no-replace 移动返回冲突并重新分配；
6. 达到语义化上限后返回 `AutomaticNameExhausted`，不得覆盖目标。

### 6.4 不主动压缩已有编号

假设已有：

```text
项目计划.md
项目计划 (2).md
```

第一篇改成“年度计划”后结果为：

```text
年度计划.md
项目计划 (2).md
```

第二篇不会被连带改成 `项目计划.md`。只有当前发生标题变化或目录移动的笔记重新参与分配，
避免无关笔记路径 churn、同步噪声和链接破坏。

### 6.5 大小写与仅规范化差异

- `Plan` 与 `plan` 在大小写不敏感文件系统中视为冲突；
- 仅大小写变化必须通过同目录唯一临时名完成安全两跳；
- 临时名使用受 watcher 明确忽略的协议格式；
- 标题变化但规范化后 stem 相同，不移动文件，只提交标题元数据。

## 7. 应用内标题提交协议

### 7.1 统一命令

`MetadataMutation::SetTitle` 不再独立提交标题。普通笔记标题改为进入复合领域命令：

```rust
pub enum NoteCommand {
    CreateConfigured(ConfiguredCreateNoteRequest),
    UpdateTitle(UpdateNoteTitleRequest),
    Move(MoveNoteRequest),
}
```

现有允许用户输入任意文件名的 `RenameNoteRequest` 和 Save Dialog 式重命名入口删除。文件名在
属性区只读展示；用户需要改变文件名时修改 Notora 标题，需要改变目录时使用移动操作。

### 7.2 dirty 保存屏障

标题提交前按打开状态分流：

- 未打开或已打开且 clean：直接提交 `UpdateTitle`；
- 已打开且 dirty：记录 `PendingTitleUpdate`，先保存对应 `content_revision`；
- 保存成功且 revision 仍一致：提交最新标题；
- 保存期间正文再次变化：旧请求作废，保留最新标题草稿并重新调度；
- 保存失败或外部冲突：标题提交保持 pending，并显示明确错误，不移动文件。

移动、标题改名和移入回收站应共享统一的 `PendingDocumentRelocation` 状态，避免三套相似但行为
不同的保存屏障继续扩散。

### 7.3 文件与 Catalog 提交顺序

后台 worker 串行执行：

1. 读取活动笔记、命名状态和 title revision；
2. 规范化标题并分配目标路径；
3. 在 Catalog 写入 pending operation，保留旧标题与旧路径；
4. 使用 no-replace 语义移动实体文件；
5. 在一个 SQLite 事务中更新标题、title revision、相对路径、disambiguator 和 FTS；
6. 将 pending operation 标记为已提交；
7. 向产品层返回包含 `operation_id`、旧路径和新路径的成功结果；
8. 产品层更新打开文档路径并刷新卡片、导航和窗口；
9. watcher 确认内部事件后清理操作记录。

若第 4 步失败，不修改 Catalog。若第 5 步失败，优先把文件移动回旧路径；回滚也失败时保留
operation 记录并进入显式恢复状态，启动恢复不得猜测或覆盖文件。

### 7.4 首次标题初始化兼容

- 标题栏先提交：复合命令竞争 `AwaitingFirstCommit`，成功后再通过 EditorRuntime 填充空 H1
  或 Mindmap 根节点；后续正文保存写入新路径；
- 正文先保存：从正文获得首次标题后执行相同的 `UpdateTitle` 路径；
- 标题栏竞争失败：按最新用户标题重新发起普通 title revision 更新；
- 正文没有有效标题：只结束初始化，不移动文件；
- 初始化状态与文件名绑定状态分别建模，禁止复用同一个 enum 表达两个生命周期。

## 8. Finder 外部改名协议

### 8.1 类型化 watcher 事件

将 `WorkspaceFileBatch` 从裸路径列表升级为类型化变化：

```rust
pub enum WorkspaceFileChange {
    Renamed {
        from: PathBuf,
        to: PathBuf,
        tracker: Option<u64>,
    },
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

pub struct WorkspaceFileBatch {
    pub changes: Vec<WorkspaceFileChange>,
}
```

配对优先级：

1. `RenameMode::Both`；
2. 相同 notify tracker 的 `From` 与 `To`；
3. 同批次唯一且可验证的一对 `From` 与 `To`；
4. 平台瞬时文件身份；
5. 唯一内容 hash；
6. 无法唯一确认则产生 `AmbiguousExternalRename`，不创建新 `NoteId`、不删除旧身份。

### 8.2 内部事件确认

worker 维护最近内部 relocation 的 `(operation_id, note_id, from, to)`：

- watcher 命中内部记录时只确认操作完成；
- 不把自动分配的 `(2)` 写进 Notora 标题；
- 迟到、重复事件必须幂等；
- worker 重启后通过 catalog 与 pending operation 恢复，不依赖内存 TTL 作为唯一真源。

### 8.3 外部事件应用

确认是 Finder 外部 rename 后：

1. 通过旧路径直接取得稳定 `NoteId`；
2. 验证目标仍在工作区内且不是 `.notora`；
3. 同文档类型改名时，以目标 stem 按字面更新标题；
4. basename 未变的目录移动只更新路径；
5. 更新 title revision，防止旧标题任务随后覆盖 Finder 结果；
6. 原子更新 Catalog path、title 和 FTS；
7. 发送类型化 `ExternalNoteRelocated` 产品事件；
8. App 更新打开文档路径、标题输入和卡片状态。

### 8.4 扩展名变化

- `.md`、`.txt`、`.mmap.md` 之间变化不是普通重命名，而是外部类型转换；
- 未打开且内容可被新类型读取时，可进入显式类型转换结果并重新索引；
- 已打开或 dirty 时进入冲突状态，等待用户确认重新加载或保留副本；
- 改成不支持的扩展名按外部移除处理，经过现有缺失确认窗口后从活动列表移除；
- 不自动把 Finder 选择的扩展名改回原类型。

### 8.5 目标覆盖与身份冲突

Finder 若覆盖另一篇受管笔记的目标路径：

- 不合并两个 `NoteId`；
- 被覆盖笔记进入外部缺失/冲突状态；
- 移入笔记保留来源 `NoteId`；
- 保留 catalog 操作证据并提示用户恢复、保留当前文件或另存副本；
- 无法证明数据等价时禁止自动删除 catalog 记录或备份。

## 9. 打开文档、自动保存与并发

### 9.1 Finder 改名时文档 dirty

Finder 移动的是磁盘旧版本，EditorRuntime 可能仍持有更新的 dirty 内容。处理顺序：

1. 取消尚未提交的定时自动保存；
2. 暂停新的保存提交；
3. 将 Catalog 和 EditorRuntime 路径绑定到 Finder 新路径；
4. 校验新路径磁盘 revision 是否仍是已知旧版本；
5. 无额外外部内容变化时，在新路径继续保存 dirty 内容；
6. 同时检测到内容变化时进入现有保存冲突流程，不覆盖 Finder 版本。

禁止自动保存继续使用旧路径，否则会重新创建已被用户改掉的文件。

### 9.2 保存已经在飞行中

若 Finder 改名发生时保存线程已经写向旧路径：

- 标记 `ExternalRenameConflict`；
- 保存完成后检查旧路径是否被重新创建；
- 只有能证明旧路径是本次保存任务产生且内容已安全合并时，才能清理副本；
- 不能证明时保留两份，提供重新加载、保留新路径、保存副本或取消；
- 禁止按文件名猜测并删除其中之一。

### 9.3 快速连续标题提交

- reducer 只保留每个 `NoteId` 最新的 pending 标题；
- 尚未开始的旧请求被新请求替代；
- 已开始的命令通过 `expected_title_revision` 拒绝过期提交；
- 成功结果必须携带 revision，App 只接受当前 generation；
- 不允许 `A -> B -> C` 的迟到 `B` 结果把路径从 `C.md` 改回 `B.md`。

## 10. Markdown 相对链接安全

持续改文件名会破坏标准 Markdown 相对链接。该问题不能用全工作区字符串替换解决，因为必须
区分代码块、外部 URL、图片、URL 编码、同名文件和不同目录下的相对路径。

### 10.1 发布门槛

标题跟随文件名默认启用前，必须二选一：

1. 完成解析级相对链接索引与重写；或
2. 将功能保持为显式预览/确认模式，并在检测到引用时阻止静默重命名。

不能在已知会破坏链接的情况下默认静默启用。

### 10.2 最小引用索引

Catalog 只维护路径安全所需的内部引用，不新增反向链接 UI：

```text
source_note_id
source_relative_path
target_relative_path
source_byte_range
link_kind
```

扫描正文时使用 Markdown parser 的 offset 信息记录真正的相对文件链接。HTTP、mailto、锚点和
代码文本不进入该索引。

### 10.3 重命名时重写

- 根据旧目标路径查询引用来源；
- 从每个来源文件目录重新计算到新路径的相对 URL；
- 未打开且 clean 的引用文件通过可恢复多文件写入协议更新；
- 已打开且 clean 的引用文件通过 EditorRuntime 范围替换更新；
- 已打开且 dirty 的引用文件先进入协调状态，不直接在磁盘覆盖；
- 任一引用文件无法安全更新时，标题改名应暂停并展示受影响文件，而不是部分静默成功；
- 图片链接和普通链接共享解析基础，但只重写确实解析到目标笔记的路径。

## 11. 既有工作区迁移

### 11.1 schema 升级不自动移动文件

现有 catalog 升级时：

- 普通笔记回填 `LegacyUnmanaged`；
- 加密笔记回填 `Opaque`；
- 不通过 `未命名 N` 正则猜测用户意图；
- 不在应用启动或普通扫描时批量改名。

### 11.2 预览式迁移

提供“按 Notora 标题整理实体文件名”：

```text
未命名 1.md       -> 项目计划.md
未命名 2.md       -> 项目计划 (2).md
未命名 3.mmap.md  -> 产品脑图.mmap.md
```

预览必须展示：

- 当前路径、当前 Notora 标题和建议路径；
- 重复编号、非法字符规范化和长度截断结果；
- 将受影响的 Markdown 相对引用；
- dirty、保存冲突、缺失文件和只读文件；
- 无法自动迁移的原因。

用户确认后逐篇走同一个 relocation 命令。失败只影响当前项，其余项继续或暂停由产品层明确
选择；结果保留可重试记录。迁移成功后进入 `TitleBound`。

### 11.3 新工作区行为

- Notora 内新建普通笔记：使用 `无标题.ext` 或可用重复编号，立即进入 `TitleBound`；
- Finder 新增文件：标题采用实际 stem，进入 `TitleBound`；
- 从旧 schema 升级：先保持 `LegacyUnmanaged`，完成一次迁移确认后统一进入新协议。

## 12. UI 与错误反馈

- 标题栏继续是工作区笔记的主要命名入口；
- 删除独立的 Save Dialog 式“重命名文件”操作；
- 位置属性只负责移动目录；
- 实体文件名在属性区只读展示，重复编号可见但不写入 Notora 标题；
- 标题提交期间显示“正在保存并重命名”；
- 失败信息区分：保存失败、目标被占用、权限不足、过期 revision、外部改名冲突、链接无法
  安全重写和恢复未完成；
- Finder 外部改名成功后刷新标题栏，不弹无意义成功提示；
- 无法确认身份的外部 rename 必须提示用户，不静默生成第二个卡片。

## 13. 分阶段实施计划

所有子任务最多修改 3 个文件。每个行为变化先写失败测试，再做最小实现；每个子任务提交前
确保编译通过，最终运行 `./scripts/verify.sh`。

### 阶段零：规格收敛

#### Task 0.1：改写标题与文件名产品规范

**文件：**

- Modify: `docs/specs/2026-08-03-notora-editor-area-design.md`
- Modify: `docs/specs/2026-07-30-notora-product-design.md`
- Modify: `docs/specs/2026-08-03-notora-note-encryption-design.md`

- [ ] 写清普通笔记双向同步与加密例外。
- [ ] 写清首次发现文件使用 stem 初始化标题。
- [ ] 删除“修改标题不自动修改文件名”的旧规则。
- [ ] 确认相对链接发布门槛。

### 阶段一：纯命名协议

#### Task 1.1：建立文件名绑定类型和纯命名算法

**文件：**

- Modify: `crates/notora-core/src/domain.rs`
- Create: `crates/notora-core/src/file_name.rs`
- Modify: `crates/notora-core/src/lib.rs`

- [ ] 先写中英文、非法字符、NFC、空标题、保留名、grapheme 截断测试。
- [ ] 覆盖 `.txt`、`.md`、`.mmap.md` 和重复编号。
- [ ] 覆盖大小写折叠与规范化后同名。
- [ ] 运行 `cargo test -p notora-core file_name`。

#### Task 1.2：建立目录占用与稳定编号分配器

**文件：**

- Modify: `crates/notora-core/src/file_name.rs`
- Modify: `crates/notora-core/src/note_command.rs`

- [ ] 先写已有文件、目录、符号链接和 catalog 占用测试。
- [ ] 同一笔记现有目标可复用。
- [ ] 编号空洞不触发其他笔记重排。
- [ ] 外部抢占后重试下一个编号。

### 阶段二：Catalog 状态与恢复协议

#### Task 2.1：持久化绑定状态、disambiguator 和 title revision

**文件：**

- Modify: `crates/notora-core/src/catalog/migration.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Modify: `crates/notora-core/src/domain.rs`

- [ ] 新 schema 直接包含完整字段。
- [ ] 旧普通笔记回填 `LegacyUnmanaged`，加密笔记回填 `Opaque`。
- [ ] 拒绝非法 enum、零 disambiguator 和负 revision。
- [ ] 运行 migration 与 repository 往返测试。

#### Task 2.2：增加可恢复路径操作记录

**文件：**

- Modify: `crates/notora-core/src/catalog/migration.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Modify: `crates/notora-core/src/catalog/mod.rs`

- [ ] pending operation 的 source/target 和 note identity 受唯一约束保护。
- [ ] 写 prepared、moved、committed、rolled-back 状态往返测试。
- [ ] 启动时可列出未完成操作，不静默丢弃。

#### Task 2.3：原子更新标题、路径与 FTS

**文件：**

- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Modify: `crates/notora-core/src/catalog/search_repository.rs`

- [ ] 一个 SQLite 事务同时更新 notes 与 note_search。
- [ ] title revision 不匹配时无字段变化。
- [ ] path 唯一冲突不产生部分标题更新。
- [ ] 保留标签、星标、正文索引和 `NoteId`。

### 阶段三：复合领域命令

#### Task 3.1：把标题修改收敛为 `NoteCommand::UpdateTitle`

**文件：**

- Modify: `crates/notora-core/src/note_command.rs`
- Modify: `crates/notora-core/src/lib.rs`

- [ ] 先写标题变更、无路径变化、重复名、过期 revision 和目标冲突测试。
- [ ] 命令结果显式区分 Created、TitleUpdated、Moved，不再用
  `previous_relative_path == None` 猜测创建。
- [ ] 保持 `NoteId` 不变。

#### Task 3.2：实现 no-replace 和仅大小写安全移动

**文件：**

- Modify: `crates/notora-core/src/workspace.rs`
- Modify: `crates/notora-core/src/note_command.rs`
- Modify: `crates/notora-core/Cargo.toml`

- [ ] 评估标准库能力和最小跨平台依赖。
- [ ] Unix/macOS 不得依赖“先 exists 再 rename”的竞争窗口。
- [ ] 仅大小写改名使用受控临时路径。
- [ ] 任意失败都保留至少一份可恢复文件。

#### Task 3.3：实现 relocation 失败回滚与启动恢复

**文件：**

- Modify: `crates/notora-core/src/note_command.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Modify: `crates/notora-core/src/backup.rs`

- [ ] 注入“文件已移动、catalog 提交失败”的故障测试。
- [ ] 注入回滚也失败的故障测试。
- [ ] 恢复逻辑按 operation 状态和明确文件存在性决策，不按时间盲删。

### 阶段四：产品标题提交与保存屏障

#### Task 4.1：迁移 action/effect 契约

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/state.rs`
- Modify: `crates/notora-app/src/effect_executor.rs`

- [ ] 删除独立 `RenameRequested` 和 `ChooseNoteRenameDestination`。
- [ ] 标题提交产生类型化 `UpdateTitle` effect。
- [ ] reducer 保留每篇笔记最新标题 generation。

#### Task 4.2：统一 dirty relocation 保存屏障

**文件：**

- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/autosave.rs`

- [ ] 把 move、title rename、trash 的 pending 状态收敛为 enum。
- [ ] 覆盖保存成功、保存失败、保存期间再次编辑和过期 completion。
- [ ] 标题 command 只在对应 content revision clean 后提交。

#### Task 4.3：后台执行与产品事件

**文件：**

- Modify: `crates/notora-app/src/workspace_controller.rs`
- Modify: `crates/notora-app/src/product.rs`

- [ ] command 结果携带 operation id、title revision、旧路径和新路径。
- [ ] worker 内部文件命令保持串行。
- [ ] 过期工作区 generation 的完成事件不能污染新工作区。

#### Task 4.4：同步打开文档路径并删除旧重命名 UI

**文件：**

- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/render.rs`

- [ ] 标题更新成功后先同步 EditorRuntime 路径，再允许后续自动保存。
- [ ] 删除 Save Dialog 式文件重命名入口和辅助函数。
- [ ] 属性区显示只读实体文件名与保存/重命名状态。

### 阶段五：类型化 watcher 与身份恢复

#### Task 5.1：保留 notify rename 语义

**文件：**

- Modify: `crates/notora-core/src/file_monitor.rs`
- Modify: `crates/notora-core/src/lib.rs`

- [ ] 覆盖 Both、split From/To、tracker、重复事件和 debounce。
- [ ] 临时原子写入文件仍被忽略。
- [ ] 多个同时 rename 不允许任意错配。

#### Task 5.2：让 reconciliation 接受明确 rename hint

**文件：**

- Modify: `crates/notora-core/src/reconciliation.rs`
- Modify: `crates/notora-core/src/scan.rs`

- [ ] 相同内容的两篇空笔记仍能按 from/to 保持各自 `NoteId`。
- [ ] hash 只作为最后的唯一匹配回退。
- [ ] 歧义结果不新增身份、不确认删除。

#### Task 5.3：传播类型化工作区变化

**文件：**

- Modify: `crates/notora-app/src/workspace_controller.rs`
- Modify: `crates/notora-app/src/product.rs`

- [ ] `WorkspaceChanged` 不再只携带裸 changed paths。
- [ ] 内部 operation 确认和外部 rename 分流。
- [ ] watcher 迟到事件保持幂等。

### 阶段六：Finder 双向同步与冲突

#### Task 6.1：应用外部 rename 到 Catalog

**文件：**

- Modify: `crates/notora-core/src/note_command.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Modify: `crates/notora-core/src/scan.rs`

- [ ] 同目录改名反向更新标题。
- [ ] 只移动目录保持标题。
- [ ] 外部括号文本按字面进入标题。
- [ ] 内部 `(2)` 不反向污染标题。

#### Task 6.2：更新打开文档并处理 in-flight save

**文件：**

- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/autosave.rs`
- Modify: `crates/notora-app/src/product.rs`

- [ ] Finder rename 后旧路径不会被下一次自动保存重新创建。
- [ ] dirty 文档在新路径安全保存。
- [ ] in-flight save 产生旧路径时进入显式冲突，不自动删除。

#### Task 6.3：扩展名变化和覆盖冲突

**文件：**

- Modify: `crates/notora-core/src/reconciliation.rs`
- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/action.rs`

- [ ] 覆盖 supported-kind 转换、unsupported 扩展名和目标 note 已存在。
- [ ] 两个 `NoteId` 永不静默合并。
- [ ] dirty 类型转换必须等待用户决策。

### 阶段七：相对链接发布门槛

#### Task 7.1：建立解析级相对链接提取

**文件：**

- Create: `crates/notora-core/src/note_link.rs`
- Modify: `crates/notora-core/src/lib.rs`
- Modify: `crates/notora-core/Cargo.toml`

- [ ] 使用 parser offset，不使用正则或全局字符串替换。
- [ ] 覆盖 URL 编码、锚点、图片、代码块、外部 URL 和不同目录。
- [ ] 输出 UTF-8 安全的 source byte range。

#### Task 7.2：持久化最小路径引用索引

**文件：**

- Modify: `crates/notora-core/src/catalog/migration.rs`
- Create: `crates/notora-core/src/catalog/link_repository.rs`
- Modify: `crates/notora-core/src/catalog/mod.rs`

- [ ] 引用跟随 source note 生命周期清理。
- [ ] 重扫原子替换单篇笔记的引用集合。
- [ ] 不新增反向链接 UI 或搜索权重。

#### Task 7.3：扫描与查询受影响引用

**文件：**

- Modify: `crates/notora-core/src/scan.rs`
- Modify: `crates/notora-core/src/catalog/link_repository.rs`

- [ ] 正文变化时刷新引用索引。
- [ ] 给定旧目标路径返回所有精确解析命中的来源。
- [ ] 同名文件和不同目录不会误匹配。

#### Task 7.4：重命名前安全重写引用

**文件：**

- Modify: `crates/notora-core/src/note_command.rs`
- Modify: `crates/notora-core/src/note_link.rs`
- Modify: `crates/notora-app/src/app.rs`

- [ ] 未打开 clean 文件通过可恢复写入更新。
- [ ] 打开文档通过 EditorRuntime 范围替换更新。
- [ ] dirty 引用来源阻止静默 rename 并展示受影响项。
- [ ] 部分失败不留下指向一半旧路径、一半新路径的不可解释状态。

### 阶段八：迁移与收尾

#### Task 8.1：建立迁移预览领域模型

**文件：**

- Create: `crates/notora-core/src/naming_migration.rs`
- Modify: `crates/notora-core/src/lib.rs`
- Modify: `crates/notora-core/src/note_command.rs`

- [ ] 预览输出目标、编号、规范化差异、链接影响和阻塞原因。
- [ ] 预览不写文件、不改 Catalog。
- [ ] 不把任意 `未命名 N` 自动认定为系统占位文件。

#### Task 8.2：接入迁移确认与结果 UI

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/state.rs`
- Modify: `crates/notora-app/src/render.rs`

- [ ] 明确区分预览、执行中、部分失败、完成和取消状态。
- [ ] 失败项可重试，不重复执行已成功项。
- [ ] 回收站和无工作区状态不能启动迁移。

#### Task 8.3：集成测试与废弃代码清理

**文件：**

- Modify: `crates/notora-app/tests/open_flow.rs`
- Modify: `crates/notora-app/tests/save_policy.rs`
- Create: `crates/notora-app/tests/title_filename_sync.rs`

- [ ] 删除旧 rename dialog、旧 action/effect 和未使用 imports。
- [ ] 覆盖应用内标题改名、Finder 改名、重复名、dirty、重启恢复和链接更新。
- [ ] 确保测试使用稳定等待条件，不依赖固定 sleep 猜时序。

## 14. 必测状态矩阵

### 14.1 命名与重复

- 空标题、中文、emoji、组合字符、非法字符、保留设备名；
- `.txt`、`.md`、`.mmap.md`；
- 同目录 1/2/3 个同标题；
- 不同目录同标题；
- 大小写不敏感冲突；
- 普通文件、目录和 symlink 占位；
- 编号空洞不触发无关笔记重排；
- 外部进程在检查与移动之间抢占目标。

### 14.2 标题与初始化

- 标题栏先提交并填充空 H1/根节点；
- 正文先保存并初始化标题与文件名；
- 两者竞态只有一个 initialization winner；
- Independent 后修改正文标题不移动文件；
- 快速连续标题提交只接受最新 revision；
- 规范化 stem 未变化时不触发文件移动。

### 14.3 Finder

- 同目录 rename、跨目录 move、move + rename、case-only rename；
- Finder 自动 Keep Both 后缀按字面更新标题；
- 两篇相同内容笔记同时 rename 仍保持正确 `NoteId`；
- rename event split、重复、迟到、丢失和 tracker 不可用；
- supported extension 转换、unsupported extension、目标覆盖；
- 内部 rename watcher 回声不反向污染标题。

### 14.4 保存与恢复

- clean、dirty、scheduled save、in-flight save、save failed；
- Finder rename 后旧路径不复活；
- 文件移动后 catalog 失败；
- catalog 成功事件返回前应用退出；
- 回滚成功、回滚失败、重启恢复；
- 工作区切换后旧 generation 结果被丢弃。

### 14.5 链接

- 同目录和跨目录相对链接；
- 链接标题、锚点、URL 编码和图片；
- 代码块和外部 URL 不误改；
- 多个来源引用同一目标；
- 来源文件 clean、打开、dirty、只读和外部修改；
- 重命名失败不留下部分链接更新。

## 15. 验证命令

每个子任务运行相关定向测试，并至少执行：

```bash
cargo fmt --all -- --check
cargo check -p notora-core
cargo check -p notora-app
cargo test -p notora-core
cargo test -p notora-app
```

涉及 EditorRuntime 路径或保存协议时补充：

```bash
cargo test -p textora-appkit-shell --lib
```

全部阶段完成后执行重大修改验证：

```bash
./scripts/verify.sh
```

## 16. 完成定义

- 普通新建笔记标题提交后，实体文件名持续跟随 Notora 标题；
- 同目录重复标题稳定生成 `标题 (N).ext`，不覆盖、不连锁重排；
- Finder 改名反向更新标题，Finder 只移动目录不改标题；
- 内部 watcher 回声不造成循环；
- 相同内容笔记外部改名仍保持各自 `NoteId`；
- dirty 和 in-flight save 场景不会重新创建旧路径或静默丢数据；
- title、path、disambiguator、FTS 和打开文档路径最终一致；
- 既有工作区只在用户确认迁移后批量改名；
- 相对链接达到发布门槛，不静默破坏已识别引用；
- 加密笔记文件名不泄漏标题；
- 无废弃 rename action、dialog、辅助函数和未使用 import；
- `./scripts/verify.sh` 全部通过。
