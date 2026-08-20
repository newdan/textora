# Textora 加密笔记创建链路修改方案

日期：2026-08-19

状态：待安全评审与实施

关联文档：

- [`2026-08-03-notora-note-encryption-design.md`](../specs/2026-08-03-notora-note-encryption-design.md)
- [`2026-08-03-notora-editor-area-design.md`](../specs/2026-08-03-notora-editor-area-design.md)
- [`2026-08-03-notora-editor-area-implementation.md`](./2026-08-03-notora-editor-area-implementation.md)

## 1. 目标

恢复并完成“新建加密笔记”的真实纵向链路：

```text
创建面板
  -> 类型化创建意图
  -> 工作区密钥准备或解锁
  -> 加密 envelope 生成
  -> vault 与 catalog 一致提交
  -> 解密后编辑会话
  -> 加密自动保存、冲突副本与 dirty snapshot
```

完成后必须满足：

- 创建面板一次确认文档类型、逻辑目录和存储方式；
- 加密创建失败时不产生明文文件、伪加密 catalog 行或不可恢复半成品；
- 加密笔记的标题、标签、逻辑目录、摘要、文档类型和正文不进入明文 catalog、FTS、路径、日志或快照；
- 加载、标题提交、正文编辑、自动保存、移动、回收站和冲突处理不会绕回普通明文文件路径；
- 创建后不存在把普通笔记原地切换为加密笔记的 action；
- 密钥服务、协议或保存链路未就绪时，产品不得展示一个可提交但不安全的加密选项。

## 2. 当前断点与根因

### 2.1 创建 UI 已失去存储方式

提交 `7e3751c` 将原创建面板替换为三项文档类型菜单，并删除了 `NewNoteDraft`、
`NoteCreationStorageMode` 和提交状态机。当前 `CreateRequested(DocumentKind)` 只表达文档类型，
无法表达逻辑目录与加密方式。

### 2.2 产品层强制降级为普通笔记

`notora-app::runtime::request_note_creation` 无条件写入
`NoteEncryption::Unencrypted`。即使未来 UI 重新产生 `Encrypted` 意图，当前 effect 边界也会丢失它。

### 2.3 core 有意拒绝加密请求

`notora-core::note_command::create_configured_note` 对任何非 `Unencrypted` 请求返回
`EncryptionUnavailable`。这是已有安全门槛，不应在真实引擎完成前简单删除。

### 2.4 下游全是明文假设

- 工作区扫描只识别 `.txt`、`.md`、`.mmap.md`，并使用 `read_to_string`；
- 文档加载直接读取 UTF-8 文件；
- editor runtime 的保存快照包含明文 `serialized_contents`，通用 worker 原样写盘；
- dirty snapshot 保存完整正文行；
- 标题、标签、摘要、路径和 FTS 都以明文 catalog 字段为真源；
- `DocumentOrigin::Note` 没有表达普通与加密存储的互斥状态；
- 编辑器插件由实体文件扩展名路由，无法从不含文档类型的密文文件名选择 Markdown 或 Mindmap。

因此本任务不是恢复一个 UI 枚举，而是建立新的加密存储边界。

## 3. 已确定的设计决策

### 3.1 安全模型

沿用现有规格的完整元数据加密，不采用“正文加密、标题和目录明文”的弱化方案。

明文 catalog 只允许保存：

- 随机 `NoteId`；
- vault 中不含语义的对象路径；
- 生命周期和创建事务阶段；
- envelope 协议版本和工作区密钥槽；
- 密文大小、密文 hash 和经安全评审批准的排序时间；
- 固定占位展示值，不得保存真实标题、摘要、标签、逻辑目录、文档类型或星标状态。

加密笔记不进入持久化 FTS。首版不实现加密笔记全文搜索；UI 明确提示锁定或当前不支持，
不得以写入 SQLite 明文索引作为替代。

### 3.2 物理路径与逻辑目录分离

加密实体统一放在：

```text
.notora/encrypted/objects/<NoteId>.txenc
```

回收站实体放在：

```text
.notora/encrypted/trash/<NoteId>.txenc
```

用户在创建面板选择的目录只作为密文 payload 中的逻辑目录。实体文件不得放在
`客户/项目/` 等语义目录下，否则目录结构本身已经泄漏。

`Workspace::resolve_relative_path` 当前明确禁止进入 `.notora`。加密对象必须使用新的专用
`EncryptedVault` API；禁止放宽通用路径校验，否则普通文件命令可能越权操作 catalog、manifest
或备份文件。

### 3.3 密钥层级

- 每个工作区生成一个随机 256-bit 工作区主密钥；
- 主密钥由 macOS Keychain 保存，使用 `WorkspaceId + key_slot` 标识；
- 每篇加密笔记生成独立随机 256-bit DEK；
- DEK 使用工作区主密钥封装，封装结果写入 note envelope；
- 正文 payload 使用 DEK 加密，每次保存生成新的内容 nonce；
- `key_slot` 只标识工作区主密钥版本，不承担保存每篇笔记 DEK 的职责；
- Argon2id 只用于口令保护的主密钥导出/导入，不在每次笔记保存时执行；
- 密钥类型禁止实现 `Copy`，最小化 `Clone`，并在 `Drop` 时清零。

首版必须实现主密钥恢复包的导出和导入。恢复能力未完成前，加密创建入口保持 feature
capability 禁用，避免用户在 Keychain 丢失后永久失去数据。

### 3.4 密钥解锁粒度

应用启动时不读取主密钥。首次创建或打开加密笔记时触发工作区解锁；解锁成功后建立
`UnlockedWorkspaceSession`。会话只存产品 runtime，不进入 reducer state、session 文件或 catalog。

以下事件销毁会话并取消尚未开始的明文任务：

- 切换工作区；
- 显式锁定；
- 退出应用；
- Keychain 项被删除或版本不匹配。

已经交给 worker 的保存任务不能靠丢弃 completion 假装取消；worker 必须先完成“加密后写盘”或返回
失败，任何路径都不得写入明文。

### 3.5 密码学依赖

建议锁定并经安全评审确认：

- `chacha20poly1305` 的 `XChaCha20Poly1305`；
- `argon2` 的 Argon2id；
- `zeroize` 的 `Zeroizing` / `ZeroizeOnDrop`；
- `getrandom` 或 AEAD crate 提供的操作系统随机源；
- 版本化、确定性的 payload 序列化库，禁止依赖 Rust 内存布局。

实现不得自行编写 AEAD、KDF、随机数生成器或常量时间比较。

参考：

- <https://docs.rs/chacha20poly1305/latest/chacha20poly1305/>
- <https://docs.rs/argon2/latest/argon2/>
- <https://docs.rs/zeroize/latest/zeroize/>
- <https://developer.apple.com/documentation/Technotes/tn3137-on-mac-keychains>

## 4. 协议修订

现有规格声明“每篇笔记使用随机 DEK 并由用户密钥封装”，但 envelope 没有保存
`wrapped_dek`，且 `salt` 的用途和 KDF 参数未定义。实施前将 note envelope 修订为：

```text
magic[8]
protocol_version[u16 LE]
flags[u16 LE]
note_id[16]
key_slot[u32 LE]
wrap_nonce[24]
wrapped_dek_len[u16 LE]
wrapped_dek[...]          // 32-byte DEK + AEAD tag
content_nonce[24]
ciphertext_len[u64 LE]
ciphertext[...]
content_tag[16]
```

约束：

- `magic`、版本、flags、长度和尾随字节严格校验；
- `note_id`、版本和 `key_slot` 进入 DEK 封装的 associated data；
- `note_id`、版本和不可变 header 字段进入 payload AEAD associated data；
- header 中不保存文档类型、标题、逻辑目录或任何用户语义；
- `NoteId` 同时出现在 catalog 与 envelope，用于检测替换攻击和恢复完整但未索引的密文对象；
- 每次保存必须生成新的 `content_nonce`，不得复用；
- 未知版本、flags、非法长度、截断和尾随字节在取密钥前拒绝；
- 认证失败不得返回部分明文，也不得把文件当成空笔记打开。

版本化 payload 至少包含：

```rust
struct EncryptedNotePayloadV1 {
    kind: PayloadDocumentKind,
    title: String,
    logical_directory: PathBuf,
    tags: Vec<String>,
    starred: bool,
    created_at: SystemTime,
    modified_at: SystemTime,
    line_ending: LineEnding,
    contents: Vec<u8>,
}
```

实际实现使用可稳定编码的整数时间与 UTF-8 逻辑路径组件，不直接序列化 `SystemTime` 或平台相关
`PathBuf`。`PayloadDocumentKind` 是协议内固定整数枚举，由 app 显式映射到
`notora_core::DocumentKind`；加密 crate 不反向依赖 core。解码后必须重新执行目录、标签、标题和
正文大小限制校验。

主密钥恢复包使用独立 magic 和版本：

```text
backup_magic | version | workspace_id | key_slot
kdf_id | Argon2id params | salt | nonce | wrapped_workspace_key | tag
```

note envelope 不保存 Argon2 salt；恢复包才保存完整 KDF 参数。

## 5. 类型与模块边界

### 5.1 新增纯密码学 crate

新增 `crates/notora-encryption`，包名 `notora-encryption`，crate 名 `notora_encryption`。

职责：

- envelope 严格编解码；
- payload 版本化编解码；
- DEK 生成、封装、解封与内容 AEAD；
- 主密钥恢复包导出/导入；
- 密钥清零和稳定错误分类；
- 固定测试向量。

禁止依赖：

- `notora-app`、UI、winit、wgpu；
- `notora-core`，避免 core 持久化层与加密协议形成循环依赖；
- SQLite、工作区路径或 macOS Keychain；
- editor runtime。

### 5.2 core 只持久化密文对象

`notora-core` 不读取 Keychain，也不接收明文 payload。它新增专用 `EncryptedVault` 与 catalog
repository，只消费已经认证生成的 `PreparedEncryptedObject`：

```rust
struct PreparedEncryptedObject {
    note_id: NoteId,
    protocol_version: u16,
    key_slot: u32,
    envelope: Vec<u8>,
    ciphertext_hash: Vec<u8>,
}
```

该类型包含密文，不包含 DEK、主密钥或明文。普通 `NoteCommand::CreateConfigured` 不再接受一个实际
无法执行的 `Encrypted` 分支；加密对象使用独立命令，防止错误路由到明文创建函数。

### 5.3 app 持有解锁会话与编排

`notora-app` 新增：

- `WorkspaceKeyStore`：平台安全存储接口；
- `MacKeychainWorkspaceKeyStore`：macOS 实现；
- `EncryptionController`：持有解锁会话并在 worker 中执行加解密；
- `UnlockedEncryptedNote`：内存中的 payload metadata 与 DEK 会话；
- `EncryptionCapability`：`Unavailable / NeedsSetup / Locked / Unlocked / Busy` 互斥状态。

reducer 和 UI 只能看到 capability、创建草稿和稳定错误分类，绝不能持有密钥或明文 envelope
中间态。

### 5.4 UI 继续保持纯数据输入

`ui::widgets::note_creation_panel` 只定义纯输入与通用 widget action，不依赖 `NotoraState`、
`DocumentView`、Keychain 或加密 crate。`notora-app::render` 负责把产品状态映射成 UI 输入，遵守
现有跨层解耦红线。

## 6. 创建、加载和保存状态机

### 6.1 创建

```text
EditingDraft
  -> AwaitingWorkspaceKey
  -> PreparingEnvelope
  -> PersistingEncryptedObject
  -> OpeningCreatedNote
  -> Completed
```

任一步失败进入 `Failed { stage, error_kind }`，保留不含秘密的创建草稿供重试。提交中禁止重复提交。

持久化使用两阶段状态：

1. Keychain 主密钥已经成功创建或读取；
2. app worker 生成 NoteId、DEK 和完整 envelope；
3. catalog 事务插入 `Creating` 对象记录；
4. vault 同目录临时文件以严格权限写入、`sync_all` 后原子发布；
5. catalog 事务把对象切换为 `Ready`，记录密文 hash 与大小；
6. 失败时清理临时文件并回滚 `Creating`；无法判断归属时保留并交给启动恢复，不盲删。

启动恢复按 `NoteId + envelope header + hash` 判断：

- `Creating` 且完整最终文件存在：验证结构后完成提交；
- `Creating` 且只有明确归属的临时文件：验证成功后发布，否则清理；
- `Creating` 且文件不存在：回滚 catalog 行；
- 完整密文对象存在但 catalog 缺失：生成可恢复诊断，不自动猜测逻辑元数据；
- 认证需要密钥但当前锁定时，只检查公开 header、大小与 hash，不尝试伪造恢复结果。

### 6.2 加载

```text
读取公开对象描述
  -> 严格解析 envelope header
  -> 请求工作区解锁
  -> 解封 DEK
  -> 验证并解密 payload
  -> 按 payload.kind 选择编辑器插件
  -> 建立 UnlockedEncryptedNote 会话
```

编辑器插件路由使用显式 `DocumentKind`，不从 `.txenc` 扩展名推断。实体密文路径只用于磁盘 revision
和冲突检测。

### 6.3 保存

普通和加密保存必须在产品层按类型分流：

```rust
enum WorkspaceNoteSavePlan {
    Plain(PreparedDocumentSave),
    Encrypted(PreparedEncryptedNoteSave),
}
```

加密保存：

1. 从 editor runtime 取得当前 revision 的不可变明文快照；
2. 在 encryption worker 中更新 payload 并生成全新 content nonce；
3. worker 只把完整 envelope 交给条件式原子写入；
4. 保存前比较预期密文 `DiskRevision`；
5. 新密文完成 flush/sync/replace 前保留旧密文；
6. completion 只在 tab、workspace session generation 和 content revision 全部匹配时确认干净状态。

禁止把加密笔记交给当前通用 `execute_prepared_save`，因为它会把 `serialized_contents` 原样写盘。

### 6.4 dirty snapshot 与冲突副本

- `DirtySnapshotPlan` 增加类型驱动的 `Plain / Encrypted` 变体；
- 加密 snapshot 使用独立 snapshot envelope，保存 NoteId、基线密文 revision 和加密后的 payload；
- snapshot 文件名只使用 NoteId，不使用标题或实体路径 hash；
- 恢复列表在锁定时只展示通用占位，解锁后才解码；
- 加密冲突副本必须是完整 note envelope，不能调用接收 `current_content: &str` 的普通冲突 API；
- 保存冲突期间旧密文、外部密文和本地加密候选均保持可恢复。

## 7. catalog 与产品展示

catalog schema 升级后，为加密对象保存协议字段和创建状态。已有 `notes` 必填字段在加密行中只允许
固定占位值；所有查询必须先根据存储类型构造判别联合，禁止把占位 `kind/title/path` 当成真实业务值。

建议领域类型：

```rust
enum StoredNoteDescriptor {
    Plain(PlainNoteDescriptor),
    Encrypted(EncryptedObjectDescriptor),
}

enum NoteDisplayProjection {
    Plain(CatalogCard),
    LockedEncrypted { note_id: NoteId, modified_at: SystemTime },
    UnlockedEncrypted(UnlockedCardProjection),
}
```

锁定时：

- 卡片显示“加密笔记”和锁定状态；
- 不显示真实标题、摘要、标签、文档类型或逻辑目录；
- 不进入普通目录、标签、星标和搜索范围；
- 可在独立“加密笔记”范围或工作区根的通用分组中出现。

解锁后，真实展示投影只存在内存。首版不将其写回 SQLite，也不要求跨重启保留。

标题、标签、星标和逻辑移动对加密笔记必须更新内存 payload 并走加密保存。现有 catalog metadata
mutation 和标题驱动文件名命令只服务普通笔记；加密笔记实体名始终稳定、Opaque。

## 8. 分阶段实施计划

所有子任务最多修改 3 个文件。每个子任务先增加失败测试，再实现；提交前至少执行对应
`cargo check` 和目标测试。跨阶段不得用临时明文实现保持 UI 可用。

### 阶段零：规格与基线

#### Task 0.1：修订安全规格

**文件：**

- Modify: `docs/specs/2026-08-03-notora-note-encryption-design.md`

**内容：**

- 固定修订后的 envelope 和恢复包格式；
- 明确 `.notora/encrypted` vault 与逻辑目录；
- 明确工作区解锁粒度、Keychain 与恢复包职责；
- 明确锁定状态下的卡片、目录、标签和搜索行为；
- 将状态保持为“待安全评审”，不得由实现提交自行标记通过。

#### Task 0.2：建立可信基线

**文件：** 无。

- 先完成或隔离当前 Markdown 性能相关未提交修改；
- 记录 `git status --short`；
- 运行 `cargo check -p notora-core`；
- 运行 `cargo check -p notora-app`；
- 运行 `cargo test -p notora-core`；
- 运行 `cargo test -p notora-app --lib`。

### 阶段一：纯加密引擎

#### Task 1.1：创建独立 crate 与锁定依赖

**文件：**

- Modify: `Cargo.toml`
- Create: `crates/notora-encryption/Cargo.toml`
- Create: `crates/notora-encryption/src/lib.rs`

- 只暴露版本化协议、密钥和错误类型；
- `#![forbid(unsafe_code)]`；
- 检查依赖 feature，避免引入不需要的默认能力；
- 运行 `cargo check -p notora-encryption`。

#### Task 1.2：实现严格 envelope 编解码

**文件：**

- Modify: `crates/notora-encryption/src/lib.rs`
- Create: `crates/notora-encryption/src/envelope.rs`
- Create: `crates/notora-encryption/tests/envelope_vectors.rs`

- 先写固定向量和所有 header 破坏测试；
- 使用语义化常量定义字段长度、magic、版本和 flags；
- 拒绝截断、溢出、未知 flags、尾随字节和 NoteId 不匹配；
- 运行 `cargo test -p notora-encryption --test envelope_vectors`。

#### Task 1.3：实现 payload 与密钥生命周期

**文件：**

- Create: `crates/notora-encryption/src/payload.rs`
- Create: `crates/notora-encryption/src/secret.rs`
- Create: `crates/notora-encryption/tests/payload_roundtrip.rs`

- 先覆盖中文、emoji、NUL、CRLF、空文档和大文档；
- 密钥类型禁止 `Debug` 输出秘密，禁止 `Copy`；
- 解密缓冲和密钥在 drop 时清零；
- 为 payload 字段设置显式大小上限；
- 运行 `cargo test -p notora-encryption --test payload_roundtrip`。

#### Task 1.4：实现主密钥恢复包

**文件：**

- Create: `crates/notora-encryption/src/key_backup.rs`
- Modify: `crates/notora-encryption/src/lib.rs`
- Create: `crates/notora-encryption/tests/key_backup_vectors.rs`

- 固定 Argon2id 参数及允许的参数范围；
- 错误口令、篡改、未知 KDF 和非法高成本参数必须安全失败；
- 导入成功后验证 workspace ID 和 key slot；
- 运行 `cargo test -p notora-encryption --test key_backup_vectors`。

### 阶段二：catalog 与 encrypted vault

#### Task 2.1：增加类型驱动的存储描述

**文件：**

- Modify: `crates/notora-core/src/domain.rs`
- Modify: `crates/notora-core/src/lib.rs`
- Create: `crates/notora-core/tests/note_storage_domain.rs`

- 引入 `NoteStorageDescriptor::{Plain, Encrypted}`；
- `DocumentOrigin::Note` 使用互斥存储枚举，不组合多个 bool；
- 加密描述只包含公开对象信息；
- 运行 `cargo test -p notora-core --test note_storage_domain`。

#### Task 2.2：升级 catalog schema

**文件：**

- Modify: `crates/notora-core/src/catalog/migration.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Create: `crates/notora-core/tests/encrypted_catalog_migration.rs`

- schema v8 增加协议版本、key slot 和对象提交状态；
- 旧数据全部回填为普通笔记；
- 加密行拒绝真实 title、excerpt、kind 投影和 title initialization；
- 非法状态、协议版本和 key slot 严格拒绝；
- 运行 `cargo test -p notora-core --test encrypted_catalog_migration`。

#### Task 2.3：隔离加密卡片与 FTS

**文件：**

- Modify: `crates/notora-core/src/catalog/card_repository.rs`
- Modify: `crates/notora-core/src/catalog/search_repository.rs`
- Create: `crates/notora-core/tests/encrypted_catalog_privacy.rs`

- locked card 只返回非敏感投影；
- 加密行不能写入 `note_search`、`note_tags` 或普通目录查询；
- 测试直接查询 SQLite，断言已知秘密字节不存在；
- 运行 `cargo test -p notora-core --test encrypted_catalog_privacy`。

#### Task 2.4：实现专用 vault API

**文件：**

- Create: `crates/notora-core/src/encrypted_vault.rs`
- Modify: `crates/notora-core/src/workspace.rs`
- Modify: `crates/notora-core/src/lib.rs`

- vault 只接受 NoteId 派生的对象名；
- 禁止调用方传入任意相对路径或文件名；
- 创建对象使用严格权限、同目录临时文件、sync 和原子发布；
- 不放宽 `Workspace::resolve_relative_path` 对 `.notora` 的禁止；
- 运行 `cargo test -p notora-core encrypted_vault`。

#### Task 2.5：实现密文创建事务

**文件：**

- Modify: `crates/notora-core/src/encrypted_vault.rs`
- Modify: `crates/notora-core/src/note_command.rs`
- Create: `crates/notora-core/tests/encrypted_creation_transaction.rs`

- 增加只接受 `PreparedEncryptedObject` 的独立命令；
- 注入每个 I/O 与 catalog 故障点，验证旧数据、临时文件和状态；
- 普通创建命令不再用 `NoteEncryption` 表达无法执行的分支；
- 运行 `cargo test -p notora-core --test encrypted_creation_transaction`。

#### Task 2.6：实现启动恢复

**文件：**

- Modify: `crates/notora-core/src/encrypted_vault.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Create: `crates/notora-core/tests/encrypted_creation_recovery.rs`

- 覆盖每个事务阶段崩溃后的恢复矩阵；
- 不能认证时保留并报告，不盲删；
- 恢复日志只包含 NoteId、阶段和错误分类；
- 运行 `cargo test -p notora-core --test encrypted_creation_recovery`。

### 阶段三：Keychain 与解锁会话

#### Task 3.0：让产品层依赖纯加密引擎

**文件：**

- Modify: `crates/notora-app/Cargo.toml`

- 通过 workspace dependency 引入 `notora-encryption`；
- 不让 `notora-core`、`ui` 或 shared editor crates 依赖加密引擎；
- 运行 `cargo check -p notora-app`。

#### Task 3.1：建立可注入密钥服务

**文件：**

- Create: `crates/notora-app/src/encryption/key_store.rs`
- Create: `crates/notora-app/src/encryption/mod.rs`
- Modify: `crates/notora-app/src/lib.rs`

- 定义 load/create/delete/import 的结构化结果；
- 区分 `NotFound`、`Cancelled`、`AccessDenied`、`CorruptValue` 和 backend failure；
- 提供内存 fake，测试不得访问真实 Keychain；
- 运行 `cargo test -p notora-app encryption::key_store`。

#### Task 3.2：实现 macOS Keychain adapter

**文件：**

- Modify: `crates/notora-app/Cargo.toml`
- Create: `crates/notora-app/src/encryption/mac_keychain.rs`
- Create: `crates/notora-app/tests/keychain_contract.rs`

- 使用 `WorkspaceId + key_slot` 稳定寻址；
- 明确选择 SecItem / Data Protection Keychain 与访问控制策略；
- 非 macOS 返回 `UnsupportedPlatform`，不得回退到磁盘明文；
- contract test 默认使用 fake，真实 Keychain 测试单独 opt-in；
- 运行 `cargo check -p notora-app` 和 contract test。

#### Task 3.3：实现 EncryptionController

**文件：**

- Create: `crates/notora-app/src/encryption/controller.rs`
- Modify: `crates/notora-app/src/encryption/mod.rs`
- Create: `crates/notora-app/tests/encryption_session.rs`

- 会话按 workspace generation 隔离；
- worker completion 必须匹配 generation；
- 切换、锁定和退出清除密钥；
- 创建、加载、保存分别使用类型化请求，不用字符串阶段判断；
- 运行 `cargo test -p notora-app --test encryption_session`。

#### Task 3.4：接入恢复包导出与导入

**文件：**

- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/src/encryption/recovery.rs`
- Create: `crates/notora-app/tests/encryption_recovery.rs`

- 首次启用加密前要求完成恢复包保存确认；
- 导入不覆盖不匹配 workspace/key slot 的现有密钥；
- “忘记密钥”只删除 Keychain 引用，不删除密文；
- 运行 `cargo test -p notora-app --test encryption_recovery`。

### 阶段四：加载、编辑与保存

#### Task 4.1：读取并解密加密对象

**文件：**

- Modify: `crates/notora-app/src/workspace_controller.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/tests/encrypted_load.rs`

- workspace worker 只读取密文 bytes 和公开 descriptor；
- EncryptionController 完成解密后构造 `LoadedDocument`；
- 认证失败不创建 tab；
- 运行 `cargo test -p notora-app --test encrypted_load`。

#### Task 4.2：使用显式文档类型选择插件

**文件：**

- Modify: `crates/notora-app/src/editor_adapter.rs`
- Create: `crates/notora-app/tests/encrypted_plugin_route.rs`

- `LoadedDocument` 显式携带 `DocumentKind`，不从 `.txenc` 推断；
- 实体路径继续只用于磁盘 revision，插件路由使用解密后的类型；
- 普通文档原有扩展名路由保持不变；
- 运行 `cargo test -p notora-app --test encrypted_plugin_route`。

#### Task 4.3：把 runtime origin 改为存储枚举

**文件：**

- Modify: `crates/notora-app/src/document_registry.rs`
- Modify: `crates/notora-app/src/runtime.rs`
- Create: `crates/notora-app/tests/encrypted_origin.rs`

- tab 注册时绑定 Plain 或 Encrypted origin；
- 路径变更、session restore 和 LRU 恢复不丢失存储类型；
- 加密 origin 不允许进入普通重命名命令；
- 运行 `cargo test -p notora-app --test encrypted_origin`。

#### Task 4.4：分流自动保存

**文件：**

- Modify: `crates/notora-app/src/runtime/document_runtime.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/tests/encrypted_autosave.rs`

- 先写复现测试证明当前通用 worker 会原样写 `serialized_contents`；
- 加密 origin 必须生成 `PreparedEncryptedNoteSave`；
- 保存期间检查 tab、workspace、session 和 content revision；
- 在测试目录递归搜索已知明文，断言保存全过程无落盘泄漏；
- 运行 `cargo test -p notora-app --test encrypted_autosave`。

#### Task 4.5：加密 dirty snapshot

**文件：**

- Modify: `crates/notora-app/src/dirty_snapshot.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/tests/encrypted_dirty_snapshot.rs`

- snapshot 计划使用枚举，不新增 `is_encrypted` bool；
- 锁定时只列出占位恢复候选；
- 恢复前验证 NoteId、基线 revision 和 AEAD；
- 运行 `cargo test -p notora-app --test encrypted_dirty_snapshot`。

#### Task 4.6：加密外部变化与冲突副本

**文件：**

- Modify: `crates/notora-app/src/runtime/document_runtime.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/tests/encrypted_conflict.rs`

- 比较密文 revision，不用明文 hash 监控；
- 本地冲突候选先加密再创建副本；
- 无法解密外部版本时禁止覆盖；
- RetrySave 使用新的 nonce 和最新预期 revision；
- 运行 `cargo test -p notora-app --test encrypted_conflict`。

### 阶段五：加密 metadata、移动与回收站

#### Task 5.1：按存储类型路由 metadata mutation

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/state.rs`
- Create: `crates/notora-app/tests/encrypted_metadata_routing.rs`

- reducer 根据存储描述生成普通 catalog mutation 或加密 payload mutation；
- 加密标题不产生文件重命名；
- action 与 effect 只携带 NoteId 和用户变更，不携带 DEK 或主密钥；
- 运行 `cargo test -p notora-app --test encrypted_metadata_routing`。

#### Task 5.2：执行加密 metadata mutation

**文件：**

- Modify: `crates/notora-app/src/runtime.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/tests/encrypted_metadata_persistence.rs`

- 标题、标签、星标和逻辑目录只更新内存 payload；
- mutation 与正文保存共享 revision 屏障，不能用旧 metadata 覆盖新正文；
- mutation 失败保持原密文、当前 payload 和 dirty 状态；
- SQLite 与临时目录中不得出现 mutation 明文；
- 运行 `cargo test -p notora-app --test encrypted_metadata_persistence`。

#### Task 5.3：内存展示投影

**文件：**

- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/tests/encrypted_projection.rs`

- locked card 使用固定占位；
- unlocked projection 只从会话映射；
- 锁定后立即移除标题、摘要、标签和逻辑目录投影；
- 运行 `cargo test -p notora-app --test encrypted_projection`。

#### Task 5.4：逻辑移动

**文件：**

- Modify: `crates/notora-app/src/runtime.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`
- Create: `crates/notora-app/tests/encrypted_move.rs`

- 普通笔记继续执行物理移动；
- 加密笔记只更新 payload logical directory；
- vault 对象路径和 NoteId 保持稳定；
- 运行 `cargo test -p notora-app --test encrypted_move`。

#### Task 5.5：回收站与恢复

**文件：**

- Modify: `crates/notora-core/src/trash.rs`
- Modify: `crates/notora-core/src/encrypted_vault.rs`
- Create: `crates/notora-core/tests/encrypted_trash.rs`

- 只在 encrypted vault 的 objects/trash 间移动完整 envelope；
- NoteId、key slot 和 DEK 不改变；
- 恢复不覆盖同名对象；
- 永久删除不删除 Keychain 工作区主密钥；
- 运行 `cargo test -p notora-core --test encrypted_trash`。

### 阶段六：创建面板与产品接入

#### Task 6.1：恢复纯 UI 创建面板

**文件：**

- Create: `crates/ui/src/widgets/note_creation_panel.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- 输入只包含文档类型、目录选项、存储选项、capability 文案和提交状态；
- widget action 只返回 option key、提交或取消；
- 不导入 `notora-core` 或 `notora-app` 类型；
- 运行 `cargo test -p textora-ui note_creation_panel`。

#### Task 6.2：恢复创建草稿状态机

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/state.rs`
- Modify: `crates/notora-app/src/render.rs`

- `NewNoteDraft` 同时保存 kind、logical directory、storage mode 和 submission；
- 使用枚举表达 `Editing / AwaitingKey / Preparing / Persisting / Failed`；
- Escape 清空草稿；失败保留草稿；提交中防重复；
- 回收站和外部文件范围不能选择加密工作区创建；
- 运行 `cargo test -p notora-app state` 与 `cargo test -p notora-app render`。

#### Task 6.3：连接加密创建 effect

**文件：**

- Modify: `crates/notora-app/src/effect_executor.rs`
- Modify: `crates/notora-app/src/runtime.rs`
- Modify: `crates/notora-app/src/encryption/controller.rs`

- 普通创建进入原 plain command；
- 加密创建先确保恢复包与解锁会话，再准备 envelope，最后提交密文对象命令；
- UI action、effect 和 command 中不携带主密钥或 DEK；
- 完成后直接以 NoteId 打开新笔记，不依赖扫描器发现 `.txenc`；
- 运行 `cargo test -p notora-app --lib create`。

#### Task 6.4：增加纯 UI 加密提示组件

**文件：**

- Create: `crates/ui/src/widgets/encryption_prompt.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- 输入只包含提示模式、说明文字、字段状态和按钮状态；
- widget action 只表达提交、选择恢复包或取消；
- 不依赖产品、Keychain 或加密领域类型；
- 运行 `cargo test -p textora-ui encryption_prompt`。

#### Task 6.5：连接解锁、恢复包与错误 UI

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/state.rs`

- 区分首次设置、解锁、导入恢复包、用户取消和安全存储失败；
- 错误文案不包含路径、密钥、nonce、密文或明文；
- capability 未满足时加密选项禁用并给出明确原因；
- 运行相关 UI、state 和 render 测试。

### 阶段七：迁移、审计与发布门槛

#### Task 7.1：禁止旧伪加密行被静默打开

**文件：**

- Modify: `crates/notora-core/src/catalog/migration.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`
- Create: `crates/notora-core/tests/legacy_encrypted_rows.rs`

- 历史 `encryption = 1` 但没有合法协议字段的行标记为 `LegacyInvalidEncrypted`；
- 不按明文打开，不自动生成新密钥，不覆盖原文件；
- 提供诊断和手工恢复指引；
- 运行 `cargo test -p notora-core --test legacy_encrypted_rows`。

#### Task 7.2：安全日志与内存清理审计

**文件：**

- Modify: `crates/notora-app/src/encryption/controller.rs`
- Modify: `crates/notora-encryption/src/secret.rs`
- Create: `crates/notora-app/tests/encryption_privacy_audit.rs`

- 搜索日志、panic、Debug、session、snapshot 和临时目录中的测试秘密；
- 锁定后移除所有会话 projection 和待执行明文请求；
- 验证错误链不包含敏感输入；
- 运行 privacy audit test。

#### Task 7.3：完整验证与安全评审

**文件：** 无。

- `cargo fmt --all -- --check`；
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`；
- `cargo test -p notora-encryption`；
- `cargo test -p notora-core`；
- `cargo test -p notora-app --lib`；
- `./scripts/verify.sh`；
- 运行固定向量、故障注入、递归明文扫描和人工 Keychain/解锁 UI 验收；
- 安全评审通过前保持加密创建 capability 关闭。

## 9. 必须先写的回归测试

实施第一批测试应直接锁定当前断链：

1. 选择加密存储后，创建意图到 effect 仍保持 `Encrypted`，不能被 runtime 改成普通存储；
2. 加密创建命令失败时，工作区内不存在 `.md/.txt/.mmap.md` 明文副产物；
3. 已知正文、标题、标签和逻辑目录不出现在 catalog、FTS、文件名、临时文件和 snapshot；
4. 创建成功后文件以 envelope magic 开头，不能被 `read_to_string` 当成普通笔记；
5. 首次编辑自动保存后，磁盘仍是可认证密文，重启解锁可往返；
6. 任意 header、wrapped DEK、ciphertext 或 tag 单字节变化均被拒绝；
7. 加密 tab 永远不会调用普通 `execute_prepared_save`；
8. 锁定、切换工作区和退出后，旧 worker completion 不能重新安装解锁状态；
9. catalog 提交与文件发布每个故障点都能恢复或给出明确诊断；
10. 标题提交、逻辑移动和回收站操作不改变稳定密文实体名。

## 10. 非目标

首版不包含：

- 普通笔记原地转换为加密笔记；
- 加密笔记全文搜索或持久化内存索引；
- Windows Credential Manager、Linux Secret Service 的生产实现；
- 多用户共享密钥、团队权限或远程密钥服务器；
- 在 Finder 中直接编辑密文文件；
- 防护已解锁进程内的恶意代码、系统已失陷、截屏或用户主动复制明文；
- 通过压缩、分块或流式加密优化超大笔记；相关优化必须另立协议版本与计划。

## 11. 发布与回滚

- 新代码合入后仍以 `EncryptionCapability::Unavailable` 为默认，直到安全评审完成；
- feature 开启只影响新建与打开加密对象，不改变普通笔记格式；
- 回滚应用版本不得尝试把未知 `.txenc` 当成普通文件；旧版本只会忽略 `.notora` vault；
- schema 升级前创建 catalog backup，但 backup 只能包含受限公开字段；
- 一旦创建 v1 密文文件，协议读取兼容性必须长期保留；写入升级使用新版本，禁止原地重新解释旧字节；
- 出现安全问题时可以关闭创建与保存入口，但不得删除、降级或自动解密已有密文。

## 12. 完成定义

只有同时满足以下条件，才可认为“新建加密笔记链路已接通”：

- 创建 UI、key setup/unlock、envelope、vault、catalog 和打开编辑全部走通；
- 标题、正文、metadata、自动保存、snapshot、冲突和回收站均无明文旁路；
- 固定向量、篡改、故障恢复和隐私扫描测试全部通过；
- 普通笔记行为与性能没有回归；
- `./scripts/verify.sh` 通过；
- 安全评审明确批准协议、依赖版本、Keychain 策略、Argon2 参数和锁定态产品行为；
- 文档状态从“待安全评审”更新为已批准，并记录评审结论。
