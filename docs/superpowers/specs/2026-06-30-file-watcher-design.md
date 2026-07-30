# 文件外部变更监控 (FileWatcher) 设计

## 概述

为编辑器增加文件外部变更检测能力：当活跃标签页对应的文件被外部进程修改时，通过模态对话框提示用户，用户确认后重新加载文件并恢复滚动位置。

## 约束

- 仅监控当前活跃标签页的文件
- 不监控 dirty 文件（有未保存修改时跳过检查）
- mtime 轮询，间隔 2 秒
- 使用 rfd::MessageDialog 模态提示

## 模块：`crates/app/src/file_watcher.rs`

### 数据结构

```rust
pub(crate) struct FileChange {
    pub path: PathBuf,
    pub new_size: u64,
    pub new_mtime: SystemTime,
}

struct WatchingState {
    path: PathBuf,
    recorded_mtime: SystemTime,
    recorded_size: u64,
}

pub(crate) struct FileWatcher {
    watching: Option<WatchingState>,
    last_check: Instant,
    interval: Duration, // 默认 2s
    pending: bool,      // true 表示已检测到变更但用户尚未处理
}
```

### 公开接口

| 方法 | 说明 |
|------|------|
| `new() -> Self` | 初始化，interval=2s，无监控目标 |
| `start_watching(path, mtime, size)` | 文件打开/重新加载后开始监控 |
| `stop_watching()` | 切换标签或关闭文件时停止 |
| `should_check() -> bool` | 距上次检查是否已过 interval |
| `check() -> Option<FileChange>` | 同时比较 mtime 和 size，任一变化则返回 FileChange |
| `confirm_reload(mtime, size)` | 用户确认重新加载后更新记录 |

### 行为细节

- `check()` 同时比较 mtime **和** size，降低 mtime 精度问题或误触发概率
- `check()` 返回 `Some` 后，内部标记 `pending: true`，后续 `should_check()` 仍返回 `true` 但 `check()` 不再重复返回 `Some`，直到调用 `confirm_reload` 或 `stop_watching` 清除标记
- 文件被删除（metadata 返回 Err）：`check()` 返回 `None`，同时 `stop_watching`
- dirty 文件跳过检查：由调用方在调用 `check()` 前判断。若用户编辑导致 dirty，轮询自动暂停；保存后恢复监控时需更新 mtime（见下方「保存更新」）
- **保存更新**：用户保存文件后，须用保存后的新 mtime/size 调用 `start_watching`，避免保存操作本身触发「外部变更」误报

## App 集成

### 新增字段

`App` 结构体新增 `file_watcher: FileWatcher`

### 生命周期

1. **打开文件**（`open_file`）→ `start_watching(path, mtime, size)`
2. **切换标签**（`ActiveChanged` 处理）→ 对新活跃文件 `start_watching`，旧目标自动替换
3. **关闭标签** → 如果关闭的是监控目标，`stop_watching`
4. **`about_to_wait`** → 若 `should_check()` 且文件非 dirty，调用 `check()`；有变更则弹 dialog
5. **用户确认重新加载** → 记录 `scroll_anchor.doc_line` 和 `scroll_anchor.pixel_offset` → 关闭旧 DocumentView → `DocumentView::from_file` 重新加载 → 设置 `scroll_anchor` 恢复阅读位置 → `confirm_reload(new_mtime, new_size)`

### Dialog 规格

- 标题：「文件已变更」
- 描述：「「{文件名}」已被外部程序修改，是否重新加载？」
- 按钮：「重新加载」/「忽略」

## 测试策略

- 单元测试：`FileWatcher` 状态机（start/stop/check/confirm 周期，mtime 不变/变化/文件消失）
- 集成测试：在 `workspace.rs` 已有测试区域添加 switch-to-dirty-file 和 reload 流程测试

## 文件清单

| 文件 | 变更 |
|------|------|
| `crates/app/src/file_watcher.rs` | 新增 |
| `crates/app/src/app.rs` | +1 字段 `file_watcher` |
| `crates/app/src/app_lifecycle.rs` | `about_to_wait` 中 +检查逻辑 |
| `crates/app/src/dispatch/tabs.rs` | `open_file` 中 +启动监控，新增 reload 方法 |
