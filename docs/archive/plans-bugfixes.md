# Plan：功能缺陷修复

> 独立可执行，与其他 plan 无依赖。
> 以下缺陷是代码验证中新发现的，原审计文档未覆盖。

---

## 缺陷 1：用户设置无法持久化（1.5 小时）

### 问题

`app/src/settings_io.rs:32` 中 `save()` 函数已完整实现，但全项目**零调用**。用户在 edit+ 中修改的任何设置（字体大小、主题等）在应用重启后丢失。

### 修复

| # | 步骤 |
|---|------|
| 1.1 | 找到设置变更的触发点（`Settings` 的写入口） |
| 1.2 | 在合适的时机调用 `settings_io::save()`：退出前（app 生命周期钩子）或设置变更时（debounce 写入） |
| 1.3 | 确认 `save()` 的序列化格式与 `load()` 兼容 |

### 边界

- 并发写入：如果设置频繁变更，加 debounce（如 1 秒内最多写一次）
- 写入失败：磁盘满或权限问题时不应崩溃，静默失败 + log
- 首次启动：`load()` 在 settings 文件不存在时返回默认值，`save()` 应能创建文件

---

## 缺陷 2：原生菜单"最近文件"永远为空（1 小时）

### 问题

`app/src/native_menu.rs:9` 定义了 `RECENT_FILES` 静态变量，但**没有任何代码写入它**。

### 修复

| # | 步骤 |
|---|------|
| 2.1 | 在文件打开成功时向 `RECENT_FILES` push 条目 |
| 2.2 | 去重：同一文件不重复出现 |
| 2.3 | 上限控制（`MAX_ENTRIES = 100` 已在 file_history.rs 定义） |
| 2.4 | 确认原生菜单构建代码已从 `RECENT_FILES` 读取（无需改构建逻辑） |

### 边界

- 文件不存在时：打开失败不应入列
- 多窗口：如果是单进程，共享 `RECENT_FILES`；多进程则各自维护

---

## 缺陷 3：导航历史功能未接入（评估 + 1.5 小时）

### 问题

`app/src/workspace.rs:138-169` 定义了 `go_back()`、`go_forward()`、`has_back_history()`、`has_forward_history()`，但全项目**零调用**。

`app/src/app.rs:2077` 有注释 `// TODO: restore from tab_history` 确认此功能待实现。

### 方案选择

两种方向，**先和用户确认**：

**方案 A：接入（1.5 小时）**
- 添加快捷键绑定（如 Cmd+Shift+[ 后退、Cmd+Shift+] 前进）
- 在 tab 切换时写入历史记录
- 确认 `go_back`/`go_forward` 的逻辑正确（切换 tab 后焦点回到上次编辑位置）

**方案 B：删除死代码（30 分钟）**
- 删除 `go_back`、`go_forward`、相关方法、相关数据结构
- 移除 app.rs:2077 的 TODO

### 边界（方案 A）

- 历史记录数量上限
- tab 关闭后其历史条目如何处理
- 前进历史在新增导航后是否需要清空

---

## 缺陷 4：mouse::handle_cursor_moved 死代码（30 分钟）

### 问题

`app/src/mouse.rs:95` 的 `handle_cursor_moved` 有 `#[allow(dead_code)]`，且全项目**零调用**。此函数处理鼠标拖拽时的光标移动。

### 操作

| # | 步骤 |
|---|------|
| 4.1 | 确认鼠标拖拽选中是否通过其他路径工作（events.rs 的 `handle_cursor_moved`） |
| 4.2 | 若已有替代 → 删除 `mouse.rs:95` |
| 4.3 | 若缺失 → 在 events.rs 中接入此函数 |

---

## 缺陷 5：`assume_loaded()` 使用 `unreachable_unchecked()`（评估，不建议改）

`core/src/icu.rs:684` 中：

```rust
fn assume_loaded() -> &'static LibraryFunctions {
    unsafe { LIBRARY_FUNCTIONS.as_ref().unwrap_or_else(|| unreachable_unchecked()) }
}
```

如果 `LIBRARY_FUNCTIONS` 未初始化（ICU 库加载失败或未调用 `init_if_needed`），这里触发 UB 而非 panic。

**建议：** 改为 `unwrap_or_else(|| panic!("ICU not initialized"))` 以便 debug。一行改动，极低风险。

---

## 验证

- 缺陷 1：改设置 → 重启 → 确认设置保留
- 缺陷 2：打开文件 → 检查原生菜单"最近文件"列表
- 缺陷 3A：快捷键前进后退 → 标签页历史正确跳转
- 缺陷 3B：`cargo check` 无警告
- 缺陷 4：鼠标拖拽选中文本正常工作
- 缺陷 5：`cargo test` 全通过

## 工作量

约 4-5 小时。各缺陷独立，可逐个提交。
