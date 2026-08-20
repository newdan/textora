# Notora 笔记加密设计

日期：2026-08-03

状态：自动化实施与安全回归完成，GUI 人工验收待完成

关联规范：[`2026-08-03-notora-editor-area-design.md`](./2026-08-03-notora-editor-area-design.md)

## 1. 范围与非目标

本文档定义 Textora 加密 Markdown 笔记的威胁边界、文件协议和生命周期。实现以 2026-08-20 的加密笔记实施方案为准；真实创建、解锁和加密保存链路全部就绪前，不得仅通过 catalog 字段或 UI 文案宣称文件已加密。

加密保护工作区笔记正文，不保护标题、实体文件名、相对目录、catalog 中明确允许明文的正式 metadata、操作系统、编辑器进程或用户主动导出的明文。密码丢失时不提供绕过恢复。

## 2. 威胁边界

攻击者假设可以读取工作区目录、catalog、FTS 数据库、临时文件、冲突副本和备份文件，但不能在应用运行期间注入代码或读取受操作系统保护的密钥存储。应用崩溃、断电、外部修改和半写入文件均属于需要处理的正常故障。

必须防止：

- 从文件头、catalog、FTS、dirty snapshot、冲突副本或崩溃恢复文件获得正文、正文派生摘要、正文标签、链接或代码；
- 通过篡改版本头、nonce、密文或认证标签使应用接受损坏或篡改内容；
- 保存失败时用空文件、明文文件或错误 catalog 状态覆盖仍可恢复的旧密文；
- 解锁后把明文写入普通 autosave、日志、诊断信息或临时导出路径。

不承诺防护：已解锁期间运行在同一用户权限下的恶意进程、用户截屏、用户复制明文，或操作系统已经失陷的场景。

## 3. 加密文件协议

使用经过审计的成熟密码库提供 AEAD，不自行实现密码学原语。首个实现必须在评审中明确库与版本，并锁定依赖哈希。推荐采用 XChaCha20-Poly1305 或同等级的现代 AEAD；密钥派生使用经过审计的 Argon2id 实现，不使用自制 KDF 或直接 hash 密码。

文件扩展名固定为 `.md`，内容是严格规范化的文本 envelope：

````text
<!-- textora-encrypted-markdown:v1
document-id=<uuid>
kdf-profile=argon2id-64m-t3-p1
salt=<base64url-no-padding>
key-nonce=<base64url-no-padding>
wrapped-key=<base64url-no-padding>
content-nonce=<base64url-no-padding>
-->

```textora-encrypted
<固定宽度换行的 base64url-no-padding 密文与认证标签>
````
```

字段顺序、换行、编码和字段长度固定。未知版本、未知或重复字段、非法长度、非法 Base64、截断与尾随内容全部拒绝。只有精确 magic 才进入加密解析；普通 Markdown 的解析错误不得被猜测为密文。

密文载荷是原始 UTF-8 Markdown 字节，保留正文换行风格。标题不重复写入 envelope，以实体文件 stem 为磁盘真源。持久化领域模型保持 `DocumentKind::Markdown + NoteEncryption::Encrypted`，不得新增 `DocumentKind::Encrypted`。

## 4. 密钥与解锁生命周期

- 每个加密笔记使用随机 256-bit 数据密钥（DEK）；用户密码通过固定参数的 Argon2id 派生 256-bit KEK，再由 XChaCha20-Poly1305 封装 DEK。
- 首版不把密码、KEK 或 DEK 存入系统钥匙串、catalog、设置或会话持久化，也不提供恢复密钥。
- 应用启动时不自动解锁；打开加密笔记时进入独立的 locked/unlocked 状态，未解锁只能读取非敏感状态和执行解锁操作。
- 解锁密钥只在内存中存在于最小生命周期；关闭笔记、切换工作区、锁定应用或显式退出时清除会话密钥，并取消尚未完成的明文保存任务。
- tab 关闭、preview 替换、LRU 淘汰、永久删除、切换工作区或退出应用时，通过集中入口销毁解锁会话。
- 用户取消、密码错误、认证失败和协议错误均不得回退到明文读取或安装可保存的空 tab。

## 5. 明文与密文元数据

首版允许标题、由标题派生的 `.md` 文件名、相对目录、`NoteId`、随机 `document-id`、协议与 KDF 配置标识、文件大小、修改时间、密文 hash、星标和生命周期保持明文。正文、正文中的 H1/链接/代码/标签文本以及正文派生摘要必须加密。

FTS 只允许索引标题、相对路径和已批准的明文 metadata，body 固定为空；首版不建立解锁正文的持久化或内存全文索引。

加密笔记使用 `TitleInitialization::Independent` 与 `NoteFileNameBinding::TitleBound`。标题提交沿用标题—文件名同步和同名消歧；Finder 改名从 file stem 反向更新 catalog 标题。文件名不进入 associated data，因此重命名不需要解密或重写正文。

## 6. 创建、加载、保存与冲突

### 创建

创建请求必须以类型驱动的 `CreateNoteStorage::Encrypted { password }` 携带受控密码类型。只有生成完整加密 envelope 并原子安装 `.md` 文件后才创建 catalog 记录；catalog 插入失败时仅删除 identity 匹配的本次新文件。任何失败都不能留下明文 `.md`、半成品密文或标记为加密但不可加载的记录。

### 加载

worker 先读取并严格验证 envelope，在用户提交密码后执行 KDF、解封 DEK、认证解密和 UTF-8 校验。认证失败或协议错误时保留原文件不动；登记解锁 session 之前不得安装编辑 tab。

### 自动保存

保存先在后台使用 tab 对应的解锁 session 生成新 content nonce 并加密不可变正文快照，再采用同目录临时文件、flush/sync、原子替换的顺序写入。缺少 session 时保存失败并保持 dirty，绝不能回退到明文。加密 tab 不生成 dirty snapshot；过滤必须发生在构造任何含正文的 snapshot plan 之前。

### 外部修改与冲突副本

通过密文 content hash、文件大小和修改时间检测外部变化。无法解密外部版本时不得覆盖；冲突副本继续使用加密 envelope，不能把两个版本导出为明文临时文件。冲突解决必须在已解锁会话中生成新的认证密文。

### 回收站与恢复

回收站移动只改变路径和 catalog 生命周期；文件内容保持同一加密协议。恢复与普通移动一样使用“不覆盖目标”策略，NoteId、document-id、数据密钥和协议版本保持稳定。

### 备份与崩溃

catalog backup 和替换前副本只能包含允许明文的 catalog 字段和加密 envelope。加密 tab 不生成崩溃恢复正文；启动清理只能删除已确认完成或明确过期的临时文件，无法判断归属时保留并报告稳定诊断。

## 7. 错误分类与安全日志

加密引擎至少提供：`NotEncryptedDocument`、`UnsupportedVersion`、`MalformedEnvelope`、`UnsupportedKdfProfile`、`PasswordRejected`、`SessionMismatch`、`AuthenticationFailed`、`InvalidUtf8Payload`、`RandomSourceUnavailable` 和 `EncryptionFailed`。`SessionMismatch` 只表示磁盘 envelope 与现有解锁 session 的 document-id 或密钥封装身份不一致，用于销毁旧 session 并重新要求密码。面向用户的解锁错误统一为“密码错误或文件已损坏”；内部错误不得包含正文、密码、密钥、nonce 或完整密文。

测试日志使用固定占位符。生产日志只允许记录 NoteId、协议版本、错误分类和操作阶段。

## 8. 测试向量与验收门槛

加密引擎计划必须提供固定测试向量，覆盖：

- 固定随机源下 document-id、salt、nonce、associated data 和明文到 envelope 的确定性结果；
- magic、版本、document-id、salt、nonce、wrapped key、密文和 tag 单字节篡改均被拒绝；
- 截断、尾随字节、未知/重复字段、错误 Base64 和非法长度均被拒绝；
- 正文包含中文、emoji、NUL、CRLF、空文档和大文档时可往返；
- 创建、自动保存、断电替换、外部冲突、回收站恢复和 catalog backup 的故障恢复；
- 关闭 tab 后 session 不可访问，快照、日志和临时文件不残留明文；
- 错误密码、用户取消和认证失败不会改变旧密文或 catalog 状态。

加密创建入口只能与创建、解锁、保存、扫描、session 清理和 snapshot 隔离的完整纵向链路一同启用；任何安全门槛未完成时保持入口不可提交。
