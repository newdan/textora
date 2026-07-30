# Syncthing Phase 4: File Monitoring and Conflict Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在整个资料库被 Syncthing 改写时，Textora 能及时识别外部修改、删除和重命名；干净文档安全刷新，脏文档自动生成冲突副本，任何路径都不静默丢失用户内容。

**Architecture:** `textora-core` 提供带磁盘前置条件的原子保存；`DocumentView` 持有最后确认的 `DiskRevision`。app 用资料库级 `notify` watcher 归并事件并在后台计算 revision，再由纯分类函数决定 reload、detach、rename 或 conflict recovery。

**Tech Stack:** notify 8、blake3、现有 textora-core 文件 API、app 事件循环、临时文件原子 rename/fsync。

## Global Constraints

- 外部变化和保存竞态必须由磁盘 revision 判定，不能只依赖 watcher 事件顺序或 mtime。
- dirty buffer 永不被外部内容覆盖；冲突副本落盘成功前不改变当前 tab 绑定。
- 外部删除的已打开文档转为 dirty 的未命名恢复文档，不自动关闭。
- clean reload 尽量保留 cursor、selection、scroll，并将位置 clamp 到新文档范围。
- 所有递归监控、文件读取与 blake3 哈希不在 UI 线程执行。
- 旧 `FileWatcher` 仅在新路径的回归测试全部通过后删除。
- 冲突和恢复操作失败必须显式展示，且原 buffer 继续留在内存与 dirty snapshot 保护范围内。

---

### Task 1: 定义可比较的磁盘版本

**Files:**
- Modify: `crates/core/Cargo.toml`
- Create: `crates/core/src/disk_revision.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] 写测试：相同内容/metadata 相等；同 mtime 同 size 但内容不同可区分；替换 inode 可区分；不存在返回 `None`；目录/非普通文件报错。
- [ ] Run: `cargo test -p textora-core --lib -- disk_revision`
- [ ] Expected: FAIL。
- [ ] 定义：

```rust
pub struct DiskRevision {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub content_hash: blake3::Hash,
    pub file_identity: Option<FileIdentity>,
}

pub fn read_disk_revision(path: &Path) -> Result<Option<DiskRevision>, FileError>;
```

- [ ] macOS/Unix 使用 metadata 的 device+inode 构造 `FileIdentity`；hash 流式读取，不把大文件整块复制到第二份 buffer。
- [ ] Run: `cargo test -p textora-core --lib -- disk_revision`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(core): fingerprint on-disk document revisions"`

### Task 2: 为原子保存增加磁盘前置条件

**Files:**
- Modify: `crates/core/src/file.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/disk_revision.rs`

- [ ] 写失败测试：expected revision 匹配时保存；外部先改写时返回 `ConcurrentModification` 且不覆盖；目标消失时不重建；新文件 expected=None 可创建；临时文件不残留。
- [ ] Run: `cargo test -p textora-core --lib -- save_file_if_unchanged`
- [ ] Expected: FAIL。
- [ ] 实现 typed error 和 API：

```rust
pub enum SaveError {
    ConcurrentModification { expected: Option<DiskRevision>, actual: Option<DiskRevision> },
    Io { operation: &'static str, source: std::io::Error },
}

pub fn save_file_if_unchanged(
    path: &Path,
    contents: &[u8],
    expected: Option<&DiskRevision>,
) -> Result<DiskRevision, SaveError>;
```

- [ ] 临时命名固定 `.textora-save-<pid>-<counter>-<basename>.tmp`，同目录 `create_new` → write_all → file sync_all → 再次检查目标 revision → rename → parent directory sync。
- [ ] 第二次检查失败时删除临时文件并返回 concurrent；不得 fallback 为直接覆盖。
- [ ] Run: `cargo test -p textora-core --lib -- save_file_if_unchanged`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(core): reject saves over external changes"`

### Task 3: 让 DocumentView 持有基线与类型化保存错误

**Files:**
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/dispatch/commands.rs`

- [ ] 写测试：open 记录 revision；每次编辑递增 `content_revision`；成功 save 更新 revision/清 dirty；并发修改返回 typed error 且保持 dirty；Save As 对新路径使用正确基线。
- [ ] Run: `cargo test -p textora-app --lib -- document_save_revision`
- [ ] Expected: FAIL。
- [ ] 增加：

```rust
pub(crate) enum DocumentSaveError { Untitled, ConcurrentModification, Io { message: String } }

// DocumentView fields
disk_revision: Option<DiskRevision>,
content_revision: u64,
```

- [ ] 将 `save()`/`save_as()` 改为 `Result<(), DocumentSaveError>`；删除调用方对字符串 `"no file path"` 的判断。
- [ ] Run: `cargo test -p textora-app --lib -- document_save_revision`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): track document disk revisions"`

### Task 4: 建立资料库级 notify 监控器

**Files:**
- Create: `crates/app/src/library_file_monitor.rs`
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/Cargo.toml`

- [ ] 用临时目录写测试：递归 create/modify/remove/rename；短时间重复事件归并；未注册路径忽略；动态增删 root；shutdown 后无事件。
- [ ] Run: `cargo test -p textora-app --lib -- library_file_monitor`
- [ ] Expected: FAIL。
- [ ] 定义：

```rust
pub(crate) struct LibraryFileMonitor;
pub(crate) struct ExternalPathBatch { pub paths: BTreeSet<PathBuf>, pub observed_at: Instant }

impl LibraryFileMonitor {
    pub(crate) fn spawn(wake: impl Fn() + Send + 'static) -> Result<Self, MonitorError>;
    pub(crate) fn replace_roots(&self, roots: Vec<PathBuf>) -> Result<(), MonitorError>;
    pub(crate) fn try_recv(&self) -> Option<ExternalPathBatch>;
    pub(crate) fn shutdown(self);
}
```

- [ ] debounce 常量设为 200ms；notify callback 仅入队，app 先筛出已打开文档和冲突文件名，再由 worker 规范路径/读取 revision；不得为每个附件事件计算内容 hash；Textora 临时文件直接过滤。
- [ ] Run: `cargo test -p textora-app --lib -- library_file_monitor`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): watch synchronized library trees"`

### Task 5: 对外部变化做纯分类

**Files:**
- Create: `crates/app/src/external_document_change.rs`
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] 写表驱动测试：clean modified、dirty modified、clean/dirty deleted、exact rename、ambiguous same-hash candidate、self-save event、unchanged metadata event。
- [ ] Run: `cargo test -p textora-app --lib -- classify_external_change`
- [ ] Expected: FAIL。
- [ ] 定义互斥结果：

```rust
pub(crate) enum ExternalDocumentChange {
    Unchanged,
    ReloadClean { revision: DiskRevision },
    PreserveDirtyConflict { disk_revision: DiskRevision },
    DetachDeleted,
    RebindRename { new_path: PathBuf, revision: DiskRevision },
    AmbiguousRename,
}
```

- [ ] rename 仅在 notify 明确 from/to，或同 batch 中“旧路径消失 + 唯一候选具有相同 file identity/hash”时成立；多候选视为删除/新增，不能猜。
- [ ] self-save 通过当前 `disk_revision` 比对归为 `Unchanged`，不靠时间窗口忽略。
- [ ] Run: `cargo test -p textora-app --lib -- classify_external_change`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): classify external document changes"`

### Task 6: 实现 clean reload 与删除恢复

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/document_view/mod.rs`
- Create: `crates/app/src/external_change_tests.rs`

- [ ] 集成测试先覆盖：clean 外部修改自动 reload 并保留/clamp cursor、selection、scroll，同时产生短暂“已同步远端修改”提示；open file 删除后 path 变 `None`、标题为未命名恢复文档、内容不变且 dirty。
- [ ] Run: `cargo test -p textora-app --lib -- external_change_clean`
- [ ] Expected: FAIL。
- [ ] drain monitor batch 后只处理当前打开文档相关路径；后台读取完成时带上捕获的 `content_revision`，若期间用户编辑则升级为 dirty conflict，禁止 clean reload。
- [ ] 删除恢复调用专用 `detach_as_recovery()`：保留内存文本/undo history，清 file path/revision，标 dirty，并立即进入现有 dirty snapshot 周期。
- [ ] Run: `cargo test -p textora-app --lib -- external_change_clean`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): reload clean files and recover deletions"`

### Task 7: 原子创建 Textora 冲突副本

**Files:**
- Create: `crates/app/src/conflict_copy.rs`
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/Cargo.toml`

- [ ] 写测试：命名碰撞递增、create_new 防覆盖、完整 write+fsync、扩展名保留、Unicode 文件名、权限/磁盘错误返回且原 tab 不变。
- [ ] Run: `cargo test -p textora-app --lib -- conflict_copy`
- [ ] Expected: FAIL。
- [ ] 命名规则常量化为 `<stem>.textora-conflict-YYYYMMDD-HHMMSS-<origin-short-id>.<ext>`；同步资料库使用本机 Syncthing Device ID 短码，非同步文档使用常量 `local`；碰撞时追加递增序号。
- [ ] `create_conflict_copy(path, buffer, origin_id)` 只在同目录创建新文件，不覆盖 Syncthing 自己的 `.sync-conflict-*` 文件；成功后返回新 path/revision。
- [ ] Run: `cargo test -p textora-app --lib -- conflict_copy`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): atomically preserve dirty conflict copies"`

### Task 8: 完成 dirty conflict 工作流

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/document_view/mod.rs`
- Modify: `crates/app/src/external_change_tests.rs`

- [ ] 写集成测试：dirty+external modify 先创建 conflict copy，再让当前 tab 指向 conflict copy；原路径打开/刷新为远端内容；两份内容均精确保留；副本失败时当前 dirty tab/path/content 不变；保存命令发现 `ConcurrentModification` 时即使 watcher 漏事件也进入相同流程。
- [ ] Run: `cargo test -p textora-app --lib -- dirty_external_conflict`
- [ ] Expected: FAIL。
- [ ] 固定事务顺序：冻结 `(content_revision, buffer)` → 创建并 fsync conflict copy → 再确认 content revision 未变 → 将 dirty tab rebind 到 conflict path → 打开/刷新原 path；若用户期间继续输入，保留已落盘副本但不 rebind，并用新 revision 重新调度；连续活跃时提示“等待停止输入后完成冲突保护”，不得删除任何已创建副本。
- [ ] 自动生成的 conflict tab 保持 dirty 或 clean 需统一：落盘副本与 buffer 一致后标 clean，并保留明确 conflict badge；后续编辑正常变 dirty。
- [ ] Run: `cargo test -p textora-app --lib -- dirty_external_conflict`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): preserve both sides of external conflicts"`

### Task 9: 接入重命名与 Syncthing 冲突提示

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/ui/src/widgets/status_bar.rs`

- [ ] 写测试：唯一 rename 保持 tab/undo 并更新 path；模糊 rename 走删除恢复；资料库中新增/删除 `.sync-conflict-*` 或 `.textora-conflict-*` 时冲突计数更新，状态栏出现可操作提示。
- [ ] Run: `cargo test -p textora-app --lib -- external_rename`
- [ ] Expected: FAIL。
- [ ] app 映射为 `StatusBarInput` 的独立 `conflict_label: Option<String>`；ui 只布局，不扫描磁盘。
- [ ] 不解析 Syncthing conflict filename 来自动合并，仅提供“打开冲突文件所在目录/文件”的显式动作。
- [ ] Run: `cargo test -p textora-app --lib -- external_rename`
- [ ] Run: `cargo test -p textora-ui --lib -- status_bar`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): surface renamed and conflicting files"`

### Task 10: 恢复启动期间发生的磁盘变化

**Files:**
- Modify: `crates/app/src/dirty_snapshot.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/external_change_tests.rs`

- [ ] 写测试：dirty snapshot 持久化其原路径和基线 revision；Textora 退出期间原文件被修改后，恢复时生成冲突副本而非覆盖任一侧；原文件被删除后恢复为未命名 dirty 文档；untitled snapshot 不受影响。
- [ ] Run: `cargo test -p textora-app --lib -- startup_external_change`
- [ ] Expected: FAIL。
- [ ] 为 snapshot 增加可向前兼容的 `PersistedDiskRevision`（hash 用 hex 字符串）；旧 snapshot 缺基线时采用保守恢复：保持未命名 dirty 文档，不自动写回原路径。
- [ ] 启动恢复先重建 buffer，再在后台读取当前 revision，最后复用 Task 5/8 的分类与冲突流程；不得在 UI 初始化线程同步哈希大文件。
- [ ] Run: `cargo test -p textora-app --lib -- startup_external_change`
- [ ] Expected: PASS。
- [ ] Commit: `git commit -m "feat(app): reconcile dirty snapshots after offline sync"`

### Task 11: 替换旧单文件轮询 watcher

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Delete: `crates/app/src/file_watcher.rs`

- [ ] 先运行旧 watcher 覆盖的全部测试并记录基线。
- [ ] 删除 `file_watcher: FileWatcher` 字段、2 秒当前文件 polling 和旧模块注册；资料库外的普通已打开文件继续使用新 monitor 的临时单文件 parent watch，不能回归外部修改检测。
- [ ] Run: `cargo test -p textora-app --lib -- external_change`
- [ ] Run: `cargo check -p textora-app`
- [ ] Expected: PASS；代码中 `rg "FileWatcher|poll_external" crates/app/src` 无结果。
- [ ] Commit: `git commit -m "refactor(app): replace polling with revision-aware monitoring"`

### Task 12: 竞态与故障注入验收

**Files:**
- Modify: `crates/app/src/external_change_tests.rs`
- Modify: `crates/core/src/file.rs`
- Modify: `crates/app/src/conflict_copy.rs`

- [ ] 增加 barriers/fake filesystem hooks，使以下竞态可确定复现：保存 temp 写完后外部改目标；后台 reload 读取时用户输入；conflict copy 后 rebind 前用户输入；rename 后立即 delete。
- [ ] 每个测试先在移除对应保护时确认会失败，再恢复实现。
- [ ] Run: `cargo test -p textora-core --lib -- save_race --nocapture`
- [ ] Run: `cargo test -p textora-app --lib -- external_race --nocapture`
- [ ] Expected: PASS，无 sleep 驱动的脆弱测试。
- [ ] Commit: `git commit -m "test(app): cover external file race windows"`

### Task 13: 全量验收

**Files:**
- Modify only to fix a reproduced verification failure.

- [ ] Run: `cargo fmt --all -- --check`
- [ ] Run: `cargo test -p textora-core --lib -- file`
- [ ] Run: `cargo test -p textora-app --lib -- external_change`
- [ ] Run: `cargo test -p textora-app --lib -- conflict`
- [ ] Run: `cargo check -p textora-app`
- [ ] Run: `./scripts/verify.sh`
- [ ] Manual: 两台设备同时编辑同一文档；确认原路径和 Textora conflict copy 均存在且内容分别正确。
- [ ] Manual: 远端删除正在编辑的文件；确认 tab 变为 dirty 的未命名恢复文档并可 Save As。
- [ ] Manual: 退出 Textora 后由 Syncthing 修改资料库，再启动 Textora；确认首次 revision 检查仍能发现变化。
- [ ] Expected: 所有验证通过，任何 I/O 注入失败均不导致 dirty buffer 丢失或静默覆盖。
