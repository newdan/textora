# Textora 加密笔记实施记录

日期：2026-08-20

状态：自动化实施与安全回归完成；真实 GUI 人工验收因 macOS 锁屏待完成

关联文档：

- [`../specs/2026-08-03-notora-note-encryption-design.md`](../specs/2026-08-03-notora-note-encryption-design.md)
- [`../specs/2026-08-03-notora-editor-area-design.md`](../specs/2026-08-03-notora-editor-area-design.md)
- [`2026-08-03-notora-editor-area-implementation.md`](./2026-08-03-notora-editor-area-implementation.md)

## 1. 评审结论

原方案的安全根因判断正确：创建、加载、保存、扫描、FTS、dirty snapshot、冲突副本和 session 生命周期必须作为一条纵向链路同时接入，不能只增加密码弹窗。

实施前确认并修正了以下问题：

- crate 目录统一为 `crates/encryption`，package 名保持 `textora-encryption`；原方案中的 `crates/notora-encryption` 与 package 名不一致；
- 加密依赖先加入 workspace，再创建 crate，避免原 Task 1.1 的中间状态无法编译；
- 标题和标题绑定文件名允许明文，以当前产品决策替代旧的 `Opaque` 命名要求；
- `DocumentKind::Markdown` 与 `NoteEncryption::Encrypted` 保持正交，不新增文档类型；
- 加密冲突副本必须再次询问密码并生成全新的 document-id、DEK 和 envelope，不能复用普通明文副本链路；
- 外部密文身份变化使用专用 `SessionMismatch` 分类，销毁旧 session 后重新要求密码；一般密文损坏不伪装成空文档或普通 Markdown；
- app 内部纵向回归保留在 `runtime.rs` 的私有运行时测试中，避免为了独立 integration test 暴露产品内部 API。

## 2. 冻结协议

### 2.1 依赖与参数

- `argon2 0.5.x`，Argon2id；
- `chacha20poly1305 0.10.x`，XChaCha20-Poly1305；
- `base64 0.22.x`，URL-safe、无 padding；
- `rand_core 0.6.x`，操作系统随机源；
- `zeroize` 与 `uuid` 使用 workspace 锁定版本；
- KDF profile：`argon2id-64m-t3-p1`，内存 64 MiB、迭代 3、并行度 1；
- 16-byte salt、32-byte DEK、24-byte XChaCha nonce。

目标平台基准：在本机 macOS/arm64 的 release 构建中，固定向量测试每次执行两次创建和一次解锁，共三次 Argon2id 派生；预热后连续五次均为 `0.24s`，折合单次约 `80ms`。该结果确认 64 MiB/t3/p1 在当前目标平台不会造成不可接受的创建或解锁延迟，因此冻结为 v1 profile。基准命令：

```text
cargo test --release -q -p textora-encryption \
  test_vectors::fixed_crypto_vector_is_deterministic_and_unlocks -- --exact
```

### 2.2 文件与认证边界

- `.md` 文件使用严格 canonical 文本 envelope；
- 标题不进入 envelope，也不进入 AEAD associated data；
- key wrapping 绑定协议、document-id、KDF profile 和 salt；
- content AEAD 绑定协议与 document-id；
- 字段顺序、长度、Base64、换行、未知字段、重复字段和尾随内容均严格校验；
- 每次保存生成新的 content nonce；
- session 只持有 document-id、DEK 和重新序列化所需的 envelope 字段，秘密类型的 `Debug` 永远脱敏。

## 3. 实施结果

### 3.1 创建与领域模型

- 新增 `CreateNoteStorage::{Unencrypted, Encrypted { password }}`，消除“标记已加密但没有密码”的不可执行状态；
- 加密笔记固定创建为 `Markdown + Encrypted + Independent + TitleBound`；
- 先生成完整密文文件，再写 catalog；catalog 插入失败会清理本次创建的文件；
- 创建完成直接安装空 Markdown 编辑器和解锁 session，不重复要求密码。

### 3.2 加载、解锁与生命周期

- worker 根据 catalog encryption metadata 分流；未输入密码前只严格检查 envelope，不安装 tab；
- 解锁结果带 selection generation，旧的 A→B→A completion 不会覆盖新选择；
- 已打开的加密 tab 直接激活，不重复执行 KDF；
- 用户关闭、preview 替换、LRU 淘汰、工作区关闭/切换和进程退出统一移除 session；
- 当前回收站产品语义会在移动或恢复完成后关闭对应 runtime，因此 session 会立即销毁。该行为比原方案“打开 tab 跨回收站移动保持 session”更严格，且与现有选择/只读模型一致。

### 3.3 保存、索引与快照

- `appkit-shell` 提供产品无关的 worker payload transform；
- 加密 tab 在 worker 中把不可变明文快照转换为密文后再写盘；缺少 session 时明确失败并保持 dirty；
- scanner 对精确 magic 做严格解析，损坏的已知密文不会降级为普通 Markdown；
- encrypted excerpt 和 FTS body 固定为空，标题与允许明文的 metadata 仍可搜索；
- 加密 tab 在构造 `DirtySnapshotPlan` 前被过滤，不产生含明文的 `.dirty` 文件。

### 3.4 冲突与路径

- Retry Save 复用加密 payload transform；
- Reload 使用当前 session 认证解密；外部 document-id、salt、wrapped key 或 DEK 身份发生变化时返回 `SessionMismatch`，集中关闭旧 tab/session，再进入密码解锁；
- Save Copy 显示敏感密码与确认密码输入，使用新 document-id、新 DEK 和新 nonce 写出独立加密 `.md`；
- 标题重命名只改变文件名和 catalog，不重加密正文；
- 移动、回收站和恢复沿用不覆盖路径策略，文件内容保持原 envelope。

### 3.5 UI 与通用 Textora 隔离

- `ui::encrypted_note_dialog` 只接收纯数据 input，支持创建、解锁和冲突副本三种模式；
- 密码输入只产生 `SensitiveText` / `TextPayload::Sensitive`；
- Notora 新建菜单映射 `EncryptedMarkdown`；通用 Textora 菜单不展示此产品能力，收到该类型时也不会创建伪加密明文文档；
- 卡片显示锁图标且不显示正文摘要。

## 4. 安全回归

已覆盖：

- canonical envelope、固定向量、错误密码、非法 UTF-8、字段/长度/Base64/尾随内容拒绝；
- magic、version、document-id、salt、key nonce、wrapped key、content nonce、ciphertext/tag 单字节篡改拒绝；
- 每次保存 nonce 更新且 document-id 保持；
- 创建、重启、错误密码、正确密码、保存与 session 缺失拒绝；
- 新加密冲突副本使用不同 document-id，并能以新密码恢复内存正文；
- 外部密文替换后旧 session 销毁并重新要求新密码；
- 递归检查工作区密文、SQLite/WAL、产品配置和 snapshot 目录，均不包含测试正文或密码标记；
- encrypted dirty tab 在 plan 生成前过滤；
- scanner/FTS 不索引正文，损坏 envelope 不降级。

最终验证命令：

```text
cargo test -p textora-encryption -p notora-app --lib
cargo clippy -p textora-encryption -p notora-app --all-targets -- -D warnings
./scripts/verify.sh
```

## 5. 人工验收状态

使用隔离的 `NOTORA_CONFIG_DIRECTORY` 和临时工作区成功启动了真实 Notora 构建；Computer Use 在读取窗口前发现 macOS 已锁屏，按安全策略不能自动解锁，因此未执行密码框、切换 tab、关闭 tab 和重启后的点击验收。

该阻塞不影响自动化验证结果，但在发布构建前仍需于解锁的 Mac 上完成原方案第 17 节的九项人工检查。临时验收实例已关闭，未读写用户现有 Notora 配置。

2026-08-20 连续三次隔离验收尝试均在应用枚举前被 macOS 锁屏阻止；目标已暂停在真实 GUI 验收步骤，待用户手动解锁 Mac 后恢复。以下矩阵区分“安全核心已有自动化证据”和“真实界面已人工观察”，避免用前者替代后者：

| 原方案人工项 | 自动化证据 | 真实 GUI 状态 |
|---|---|---|
| 1. 新建加密笔记并输入密码 | 创建成功安装空编辑器和解锁 session；密码策略与确认校验测试通过 | 待验收 |
| 2. 输入正文并等待自动保存 | 保存线程将明文快照变换为新 nonce 的严格 envelope | 待验收 |
| 3. 检查 `.md`、catalog、FTS、snapshot 无明文 | 泄漏回归递归检查工作区、SQLite/WAL、配置和 snapshot；扫描/FTS 正文为空 | 待以独立文本工具复核 |
| 4. 切换笔记后返回不重复输密码 | 已打开 identity 直接激活现有 tab/session 的运行时路径已覆盖 | 待验收 |
| 5. 关闭 tab 后重新打开必须输密码 | session 集中清理及重新打开锁定路径测试通过 | 待验收 |
| 6. 错误密码失败、正确密码恢复正文 | 重启后的错误密码和正确密码端到端测试通过 | 待验收 |
| 7. 修改标题同步 `.md` 文件名 | `TitleBound` 重命名事务和扩展名保持测试通过 | 待验收 |
| 8. 重启后必须重新输入密码 | 新 runtime 不复用旧 session 的端到端测试通过 | 待验收 |
| 9. 回收站、恢复和冲突保存保持密文 | 路径操作不改内容；冲突副本使用新 document-id/DEK/nonce；外部替换会重锁 | 待验收 |
