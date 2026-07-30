# 拖拽文件打开副作用修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让拖入文件的打开路径立即应用 `AppEffect`，避免首次显示时延后重排造成卡顿。

**Architecture:** 在 `App` 的生命周期模块中提取可单元测试的 `handle_dropped_file(&Path)` 方法。该方法复用既有 `open_file`，成功时立即执行其返回的 effect；`DroppedFile` 事件分支仅负责把事件路径传入此方法。

**Tech Stack:** Rust、winit、现有 `AppEffect` 管线、`tempfile` 测试工具。

## Global Constraints

- 保持 `App::open_file` 作为唯一文件打开入口。
- 不新增应用动作、不改变文件监听、插件路由或加载策略。
- 失败时保留现有错误日志且不应用副作用。
- 使用测试驱动：先观察回归测试在修复前失败。

---

### Task 1: 拖拽路径立即应用文件打开副作用

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs:169-382`
- Test: `crates/app/src/app_lifecycle.rs:tests module`

**Interfaces:**
- Consumes: `App::open_file(&Path) -> Result<AppEffect, String>`。
- Produces: `App::handle_dropped_file(&Path)`，供 `WindowEvent::DroppedFile` 调用。

- [x] **Step 1: 写入失败的回归测试**

在 `app_lifecycle.rs` 的测试模块加入：

```rust
#[test]
fn dropped_file_applies_open_effect_immediately() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("dropped.md");
    std::fs::write(&path, "# dropped file\n")
        .expect("temporary markdown file should be written");
    let mut app = App::new(None);
    let generation_before = app.reshape_generation;
    app.needs_redraw = false;

    app.handle_dropped_file(&path);

    assert_eq!(app.reshape_generation, generation_before + 1);
    assert!(app.needs_redraw);
}
```

- [x] **Step 2: 运行测试，确认它因遗漏 effect 而失败**

运行：`cargo test -p textora-app --lib dropped_file_applies_open_effect_immediately -- --exact`

预期：失败，`reshape_generation` 未递增且/或 `needs_redraw` 为 `false`。

- [x] **Step 3: 实现最小修复**

在 `impl App` 内新增并让事件分支调用：

```rust
fn handle_dropped_file(&mut self, path: &std::path::Path) {
    match self.open_file(path) {
        Ok(effect) => self.apply_effect(effect),
        Err(error) => eprintln!("Error opening dropped file: {error}"),
    }
}
```

将 `WindowEvent::DroppedFile(path)` 分支改为：

```rust
WindowEvent::DroppedFile(path) => self.handle_dropped_file(&path),
```

- [x] **Step 4: 运行定向测试，确认修复生效**

运行：`cargo test -p textora-app --lib app_lifecycle::tests::dropped_file_applies_open_effect_immediately -- --exact`

预期：通过，且显示 1 个通过测试、0 个失败。

- [x] **Step 5: 格式化并运行应用库测试**

运行：`cargo fmt --check && cargo test -p textora-app --lib`

预期：两个命令均以退出码 0 完成。

- [x] **Step 6: 提交实现**

运行：

```bash
git add crates/app/src/app_lifecycle.rs docs/superpowers/plans/2026-07-11-dropped-file-effect.md
git commit -m "fix(app): apply effects for dropped files"
```
