# Save & Close Tab Fixes

## Scope

Fix three related bugs in tab close and save flows.

## Changes

### Fix 1: `close_tab_inner` 缺少 `lazy_load_tab` → 关闭后空白

**文件**: `crates/app/src/workspace.rs` ~line 407

**根因**: `close_tab_inner` 在关闭 active tab 后直接返回 `ActiveTabChanged`，但没有对新 active 的 tab 调用 `lazy_load_tab`。如果该 tab 是 stub（workspace 恢复后未被访问过），内容始终为空，渲染空白。

**修改**: 返回 `ActiveTabChanged` 前加一行：

```rust
if was_active {
    self.lazy_load_tab(self.active_index);
    Ok(WorkspaceEffect::ActiveTabChanged)
}
```

---

### Fix 2: Save 命令对未命名文件无 fallback

**涉及文件**:
- `crates/app/src/dispatch/editor.rs` (Cmd+S)
- `crates/app/src/dispatch/commands.rs` (菜单「保存」)

**根因**: 两处都直接调 `dv.save()`，未命名文件 `file_path==None` 返回 `Err("no file path")`，只打日志不处理。

**修改**:

A. `dispatch/editor.rs` `EditCommand::Save` 分支 (~line 306)：
`save()` 返回 `Err` 且错误为 "no file path" 时，fallback 到 SaveAs 逻辑（弹 rfd 保存对话框，与 Cmd+Shift+S 一致）。

B. `dispatch/commands.rs` `AppCommand::SaveActiveTab` 分支 (~line 22)：
同上。

C. 为避免重复，抽取公共函数 `save_active_tab_with_fallback`，Save 的两处 + SaveAs 菜单共用。

---

### Fix 3: 菜单「另存为…」空壳

**文件**: `crates/app/src/dispatch/commands.rs` line 36

**根因**: handler 只有注释，未实现。

**修改**: 填入完整 SaveAs 逻辑（复用 Fix 2-C 的公共函数）。

---

### Fix 4: Tab 键无操作

**文件**: `crates/app/src/dispatch/editor.rs` line 493

**修改**: `EditCommand::Tab => {}` 改为执行缩进（插入 tab 字符 `\t`）。

---

## Files Touched

| 文件 | 改动 |
|------|------|
| `crates/app/src/workspace.rs` | `close_tab_inner` 加 `lazy_load_tab` (1 行) |
| `crates/app/src/dispatch/editor.rs` | Save fallback; Tab → 缩进 (~15 行) |
| `crates/app/src/dispatch/commands.rs` | Save fallback; SaveAs 实现; 抽取公共函数 (~30 行) |
