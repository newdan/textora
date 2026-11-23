# Reliable Application Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 统一 settings、history、workspace、pinned paths 与 dirty snapshot 的原子写入、权限、同步、清理和错误传播策略。

**Architecture:** app infrastructure 提供单一 `atomic_write`，领域对象只负责序列化快照。写入流程是同目录 `create_new` 临时文件 → 保留权限 → write/flush/sync → rename → 父目录 sync；失败时保留旧文件并清理临时文件，调用方决定展示或记录错误。

**Tech Stack:** Rust `std::fs`/`std::io`、serde/toml、tempfile tests。

---

### Task 1: 建立可测试的 atomic_write primitive

**Files:**
- Create: `crates/app/src/persistence.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 先写原子写入测试**

测试至少包含：

```rust
#[test]
fn atomic_write_replaces_existing_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.toml");
    std::fs::write(&path, b"old").unwrap();
    atomic_write(&path, b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(temp_files(dir.path()), Vec::<PathBuf>::new());
}

#[test]
fn atomic_write_creates_missing_parent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/state.toml");
    atomic_write(&path, b"state").unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"state");
}

#[test]
fn failed_rename_does_not_leave_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let target_directory = dir.path().join("target");
    std::fs::create_dir(&target_directory).unwrap();
    assert!(atomic_write(&target_directory, b"state").is_err());
    assert_eq!(temp_files(dir.path()), Vec::<PathBuf>::new());
}
```

- [ ] **Step 2: 运行测试确认实现缺失**

Run: `cargo test -p edit-plus-app --lib persistence::tests:: -- --nocapture`

Expected: FAIL，`persistence` module/`atomic_write` 尚不存在。

- [ ] **Step 3: 实现 atomic_write**

公开接口固定为：

```rust
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()>;
```

内部使用进程级 `AtomicU64` 生成 `.<filename>.<pid>.<counter>.tmp`；`OpenOptions::new().write(true).create_new(true)`；已有目标时复制 `metadata.permissions()`；调用 `write_all`、`flush`、`sync_all`、`rename`。Unix 上 rename 后打开父目录并 `sync_all`。用 guard 的 `Drop` 删除尚未 rename 的临时文件，清理失败不能覆盖主错误。

`lib.rs` 添加：

```rust
pub(crate) mod persistence;
```

- [ ] **Step 4: 验证并提交**

```bash
cargo test -p edit-plus-app --lib persistence::tests::
cargo clippy -p edit-plus-app --lib -- -D warnings
git add crates/app/src/persistence.rs crates/app/src/lib.rs
git commit -m "feat(app): add shared atomic persistence primitive"
```

### Task 2: 迁移 settings 与 file history

**Files:**
- Modify: `crates/app/src/settings_io.rs:77-96`
- Modify: `crates/app/src/file_history.rs:43-70`

- [ ] **Step 1: 先增加错误传播测试**

为两个模块各加入 `save_to(path, value) -> io::Result<()>` 的定向测试，目标传入已存在目录并断言 `is_err()`；settings 另加损坏 TOML 测试，断言 `load_from` 返回含 path 的 parse error，不静默 default。

- [ ] **Step 2: 实现可注入路径的 load/save**

settings 接口：

```rust
pub(crate) fn load() -> io::Result<PersistedSettings> {
    load_from(&settings_toml_path())
}

fn load_from(path: &Path) -> io::Result<PersistedSettings>;
pub(crate) fn save(settings: &PersistedSettings) -> io::Result<()>;
fn save_to(path: &Path, settings: &PersistedSettings) -> io::Result<()>;
```

不存在文件返回 `Ok(PersistedSettings::default())`；读取或解析失败返回带路径上下文的 `io::Error`。history 的 `save` 保持现有 `io::Result<()>`，将最终 `std::fs::write` 替换为 `crate::persistence::atomic_write`。

- [ ] **Step 3: 修正调用方，不吞保存失败**

本任务只修改两个 persistence module；调用点在 Task 3 统一迁移。新的调用模式固定为：

```rust
if let Err(error) = settings_io::save(&settings) {
    eprintln!("[settings] failed to save settings: {error}");
}
```

用户主动触发的保存应进入现有可见错误/状态栏通道；仅退出时的 best-effort 保存可 `eprintln!`，不得 `let _ =`。

- [ ] **Step 4: 验证并提交**

```bash
cargo test -p edit-plus-app --lib settings_io::tests::
cargo test -p edit-plus-app --lib file_history::tests::
git add crates/app/src/settings_io.rs crates/app/src/file_history.rs
git commit -m "fix(app): make settings and history writes atomic"
```

### Task 3: 迁移 settings/history 调用方

**Files:**
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/app/src/app_dispatch.rs`
- Modify: `crates/app/src/app_window.rs`

- [ ] **Step 1: 初始化时显式处理 load error**

`app_init.rs` 使用：

```rust
let persisted = match crate::settings_io::load() {
    Ok(settings) => settings,
    Err(error) => {
        eprintln!("[settings] failed to load settings: {error}");
        PersistedSettings::default()
    }
};
```

保留损坏文件，不覆盖为默认设置。

- [ ] **Step 2: 所有 dispatch/window 保存显式处理 Result**

每个 `settings_io::save(&value);` 改为：

```rust
if let Err(error) = crate::settings_io::save(&value) {
    eprintln!("[settings] failed to save settings: {error}");
}
```

load-modify-save 流程先对 `settings_io::load()` 做与 Step 1 相同的 match，load 失败时记录错误并直接返回，不继续覆盖。`save_history()` 已记录 error，不再增加吞错分支。

- [ ] **Step 3: 验证没有忽略返回值并提交**

```bash
rg -n "let _ = .*settings_io|settings_io::save\([^;]+\);" crates/app/src
cargo test -p edit-plus-app --lib settings_io::tests::
cargo test -p edit-plus-app --lib file_history::tests::
git add crates/app/src/app_init.rs crates/app/src/app_dispatch.rs crates/app/src/app_window.rs
git commit -m "fix(app): surface settings persistence failures"
```

Expected: `rg` 无输出；测试 PASS。

### Task 4: 迁移 dirty snapshot

**Files:**
- Modify: `crates/app/src/dirty_snapshot.rs`

- [ ] **Step 1: 删除模块内重复 temp/sync/rename 实现**

`write_snapshot` 序列化后直接：

```rust
crate::persistence::atomic_write(path, serialized.as_bytes())
```

保持 `io::Result<()>`，不改变 diff 格式与 snapshot id。

- [ ] **Step 2: 验证 roundtrip 和失败路径**

```bash
cargo test -p edit-plus-app --lib dirty_snapshot::tests::
cargo test -p edit-plus-app --lib workspace::tests::dirty_snapshot_roundtrip_with_real_file -- --exact
```

Expected: PASS；目标不可替换时返回 error，旧快照仍可读。

- [ ] **Step 3: 提交**

```bash
git add crates/app/src/dirty_snapshot.rs
git commit -m "refactor(app): reuse atomic writer for dirty snapshots"
```

### Task 5: 让 Workspace 只产生持久化快照

**Files:**
- Create: `crates/app/src/workspace_store.rs`
- Modify: `crates/app/src/workspace.rs:435-747,792-850`
- Modify: `crates/app/src/lib.rs`

- [x] **Step 1: 在 store 中定义路径与 I/O 接口**

```rust
pub(crate) struct WorkspaceStore {
    config_dir: PathBuf,
}

impl WorkspaceStore {
    pub(crate) fn new(config_dir: PathBuf) -> Self;
    pub(crate) fn load_workspace(&self) -> io::Result<Option<PersistedWorkspace>>;
    pub(crate) fn save_workspace(&self, snapshot: &PersistedWorkspace) -> io::Result<()>;
    pub(crate) fn load_pinned_paths(&self) -> io::Result<Vec<PathBuf>>;
    pub(crate) fn save_pinned_paths(&self, paths: &[PathBuf]) -> io::Result<()>;
}
```

所有 save 都调用 `atomic_write`。`PersistedWorkspace`/`PersistedTab` 可见性改为 `pub(crate)`，仍定义在 `workspace.rs`，store 不接触 `DocumentView`/`View`。

- [x] **Step 2: 将 Workspace 方法改为纯快照转换**

```rust
pub(crate) fn snapshot(
    &self,
    sidebar_pinned: bool,
    sidebar_width: Option<f32>,
) -> PersistedWorkspace;

pub(crate) fn pinned_paths(&self) -> Vec<PathBuf>;
pub(crate) fn restore(snapshot: PersistedWorkspace, screen_height: f32) -> io::Result<Self>;
```

删除 `workspace_toml_path`、`pinned_file` 及方法内部 fs 调用。dirty snapshot 的内容生成仍可委托 `dirty_snapshot`，但路径和写入错误由 store 协调。

- [x] **Step 3: 注册 module 并迁移现有测试**

`lib.rs` 添加 `pub(crate) mod workspace_store;`。原 workspace 序列化 roundtrip 测试保留在 workspace；真实文件 I/O 测试移动到 store。

- [x] **Step 4: 验证并提交**

```bash
cargo test -p edit-plus-app --lib workspace::tests::
cargo test -p edit-plus-app --lib workspace_store::tests::
git add crates/app/src/workspace_store.rs crates/app/src/workspace.rs crates/app/src/lib.rs
git commit -m "refactor(app): extract workspace persistence store"
```

### Task 6: 迁移 app 生命周期调用并总验收

**Files:**
- Modify: `crates/app/src/app_init.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/app.rs`

- [x] **Step 1: App 持有 WorkspaceStore**

在 `App` 加入：

```rust
pub(crate) workspace_store: WorkspaceStore,
```

初始化时从同一 config dir 构造，load error 记录诊断并显式决定是否回落空 workspace。

- [x] **Step 2: 生命周期保存调用传播结果**

退出/失焦保存使用：

```rust
let snapshot = self.workspace.snapshot(sidebar_pinned, sidebar_width);
if let Err(error) = self.workspace_store.save_workspace(&snapshot) {
    eprintln!("[workspace] failed to save workspace: {error}");
}
```

主动命令触发时将 error 转为 UI 可见消息，不得 `unwrap_or_default` 或 `let _ =`。

- [x] **Step 3: 查重与验收**

```bash
rg -n "std::fs::write|\.tmp\"|sync_all\(|std::fs::rename" crates/app/src
cargo test -p edit-plus-app --lib
cargo check --workspace --all-targets
```

Expected: app 的用户状态写入只在 `persistence.rs` 出现底层原子流程；测试和 all-targets PASS。

- [x] **Step 4: 提交**

```bash
git add crates/app/src/app_init.rs crates/app/src/app_lifecycle.rs crates/app/src/app.rs
git commit -m "refactor(app): route lifecycle persistence through workspace store"
```
