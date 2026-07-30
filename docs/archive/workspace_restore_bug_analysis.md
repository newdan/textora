# Workspace 恢复只记住一个文件阅读位置 —— 根因分析

## 现象

重新打开程序时，workspace 功能只能记住一个 tab 的阅读位置（cursor_offset 和 scroll 位置），其他 tab 的位置信息丢失。

## 数据验证

`~/.edit+/workspace.yaml` 中保存了 8 个 tab，但除了 `active_index=7`（活跃 tab），其余 7 个 tab 的 `cursor_offset` 全部为 0：

```yaml
active_index: 7
tabs:
  - file_path: .../未命名4.txt
    cursor_offset: 0          # ← 丢失
    scroll_anchor_line: 0
  - file_path: .../《我家娘子》.txt
    cursor_offset: 0          # ← 丢失
    scroll_anchor_line: 15     # ← scroll 位置暂时保留
  # ... 其余 tab 类似
  - file_path: ~/.edit+/workspace.yaml
    cursor_offset: 138         # ← 只有活跃 tab 保留了
```

## 根因

### Bug 1（核心）: cursor_offset 在 stub 阶段被 clamp 到 0，永久丢失

**传播路径（循环退化）：**

```
第 N 次退出 → save_snapshot() 保存所有 tab 的真实 cursor_offset
     ↓
第 N+1 次启动 → load_snapshot() 创建 stub（空 buffer）
     ↓  cursor_offset = ts.cursor_offset.min(dv.buffer_len())
     ↓  buffer_len() == 0 → cursor_offset 被 clamp 到 0  ← 根因
     ↓
用户未切换到该 tab 就退出
     ↓
第 N+1 次退出 → save_snapshot() 保存 stub 状态（cursor_offset=0）
     ↓
所有未访问 tab 的 cursor_offset 退化为 0，不可逆
```

**代码位置：** [`workspace.rs`](file:///Users/dan/proj/llmws/edit+/crates/app/src/workspace.rs#L490)

```rust
// load_snapshot 第 490 行：stub 的 buffer_len() 是 0
dv.cursor_offset = ts.cursor_offset.min(dv.buffer_len());
// 结果：cursor_offset 永远被 clamp 到 0
```

**lazy_load_tab 也无法恢复**（[第 185 行](file:///Users/dan/proj/llmws/edit+/crates/app/src/workspace.rs#L185)）：

```rust
let cursor = dv.cursor_offset.min(loaded.buffer_len());
// dv.cursor_offset 已经是 0，恢复的也是 0
```

---

### Bug 2（同源）: scroll_anchor 也会逐步退化

虽然 `load_snapshot` 在 stub 上正确设置了 `scroll_anchor`，但 scroll_top 的计算依赖 `line_height`：

```rust
// load_snapshot 第 496-498 行
dv.viewport.scroll_top = ts.scroll_anchor_line.unwrap_or(0) as f64
    + ts.scroll_anchor_offset.unwrap_or(0.0) as f64 / lh.max(1.0);
```

这个值在 stub 阶段不会被其他逻辑修改，所以 **scroll_anchor 目前暂时安全**。但如果任何代码路径触发了 `clamp_scroll_top` 或 `sync_anchor_from_scroll`，stub 的 display_map 只有 1 行，会将 scroll_anchor 重置为 0。

---

### Bug 3（附带）: dirty + 有 file_path 的 tab 丢失修改内容

[`save_snapshot`](file:///Users/dan/proj/llmws/edit+/crates/app/src/workspace.rs#L386) 只在 `dirty && file_path.is_none()` 时保存 unsaved_lines：

```rust
let unsaved_lines = if dv.dirty && dv.file_path.is_none() { ... } else { None };
```

对于有 file_path 但 dirty 的 tab（如用户编辑了某文件但未保存就关闭程序），**修改内容丢失**，恢复时从磁盘加载旧版本，但 dirty 标记仍为 true，造成状态不一致。

## 修复方案

### 方案 A：stub 保存原始快照数据（推荐）

在 stub 中增加字段保存原始的 cursor_offset、selection_anchor 等快照数据，lazy_load 时从快照恢复：

```rust
// 新增字段
struct DocumentView {
    /// 从 workspace snapshot 恢复的原始 cursor_offset（stub 专用）
    snapshot_cursor_offset: Option<usize>,
}
```

**改动点：**

1. **`load_snapshot`**：stub 不 clamp cursor_offset，而是保存原始值到 `snapshot_cursor_offset`
2. **`lazy_load_tab`**：从 `snapshot_cursor_offset` 恢复 cursor，而非从已 clamp 的 `dv.cursor_offset`
3. **`save_snapshot`**：对 stub tab（未加载的），直接保存 `snapshot_cursor_offset` 而非被 clamp 后的值

### 方案 B：save_snapshot 时跳过 stub，保留原始 YAML 数据

save_snapshot 时，如果检测到 tab 是 stub（buffer_len()==0 && file_path.is_some()），直接复用上次保存的快照数据。

### 对 Bug 3 的修复

将 unsaved_lines 的条件改为 `dirty`（不限 `file_path.is_none()`），或者对有 file_path 的 dirty tab 也保存内容差异。

## 影响范围

| 场景 | 当前行为 | 修复后 |
|------|----------|--------|
| 非活跃 tab 的 cursor 位置 | 每次重启后退化为 0 | 保持不变 |
| 非活跃 tab 的 scroll 位置 | 目前暂时保持（但有退化风险） | 稳定保持 |
| dirty 有路径的 tab | 丢失修改、dirty 标记不一致 | 保留修改内容 |
