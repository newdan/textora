# 每日代码洁癖评审报告 (2026-06-21)

## 评审范围
- **目标**：今天 0 点以后的所有提交 (共计 40+ 个 commit，聚焦于重构与边界清理)。
- **标准**：参照 `AGENTS.md` 中定义的“核心原则”、“代码洁癖 (Clean Code)”及“跨层解耦规范”。

## 总体评价
今天的提交聚焦于**应用层边界解耦 (Public Boundaries)** 与 **UI/主题系统重构**。整体代码质量极高，在架构解耦和职责单一性上取得了显著进展。
- `cargo fmt` 执行完美，完全满足“视觉整洁”原则。
- 引入了专门的边界测试 (Boundary tests) 以强制约束架构红线，这是一个极为出色的工程实践。

## 详细洁癖审查

### 1. 命名严谨 (严格达标)
- **要求**：禁用 `data/info/temp/res/flag` 等宽泛词。
- **现状**：对所有新增和修改的生产代码进行扫描，未发现在核心逻辑中违规使用宽泛词作为独立变量。
- *注：新增代码中出现的 `TabInfo` 属纯数据结构命名，并在 `AGENTS.md` 中已作为正确示范；其余如 `tempdir()` 属标准库/第三方库，`scrollbar_reserve` 中的 `res` 不受限制。*

### 2. 消灭魔法值 (达标)
- 随着 `ui::theme_registry` 的引入，大量零散的魔法颜色值和尺寸被统一收拢。今日提交进一步消除了底层组件中对常量的硬编码。

### 3. 职责单一 & 提前返回 (达标)
- 今日大量的重构工作（如 `AppEffect` 机制和各种拆分的 `dispatch` 模块）极大地降低了单函数的复杂度。`reduce_action` 的引入和 UI 事件的解耦展现了清晰的单一职责。

### 4. 作用域最小化 & 清理废弃代码 (极小瑕疵)
- **要求**：提交前必删死代码、多余注释及未使用的引入 (Unused Imports)。
- **现状**：
  - 🚨 `cargo clippy` 在代码库中捕获到 **1 处** 未使用的 import：
    - `crates/app/src/measure_adapter.rs:22:9`: `use super::*;`

### 5. 类型驱动状态 (发现潜在改进点)
- **要求**：优先用 `enum` 表示互斥状态，严禁组合多个 `bool` 字段。
- **现状**：
  - 🚨 `crates/ui/src/widgets/tab_bar/widget.rs` 在 `TabBarWidget` 中定义了：
    ```rust
    pub back_enabled: bool,
    pub forward_enabled: bool,
    ```
    *建议改进：虽然这两者在某些逻辑上可能是正交的，但它们从属于同一个历史导航功能集。可以考虑将其收拢为一个 `NavigationState { None, BackOnly, ForwardOnly, Both }` enum。*

### 6. 严谨处理错误 (发现问题)
- **要求**：严禁图省事滥用 `.unwrap()`。若确信不会 panic，必须用 `.expect("详细说明理由")`。
- **现状**：新增提交中伴随了数十个 `.unwrap()` 调用。绝大多数存在于测试文件 (`tempdir().unwrap()`, `fs::write().unwrap()`)，尚可理解。但在**生产代码**中发现了以下违规：
  - 🚨 `crates/app/src/app_scroll.rs`: 存在对 `app.workspace.active_doc_mut().unwrap()` 的调用。
  - 🚨 `crates/app/src/mouse.rs`: 存在 `hit_test(...).unwrap()`。
  - 🚨 `crates/core/src/icu.rs`: 存在 `Arena::new(4096).unwrap()` 和 `handle.join().unwrap()`。
  - 🚨 `crates/shaping/src/lib.rs`: 获取锁时 `self.font_system.lock().unwrap()`。
  - 🚨 `crates/ui/src/theme_registry.rs`: 闭包内部和查找逻辑中多处使用 `.unwrap()`（如 `min_by_key(...).unwrap()`, `position(...).unwrap()`）。

### 7. 跨层解耦规范 (表现优异)
- **要求**：必须在 `ui` (或 `ui::widgets`) 中定义纯数据输入 struct。绝对禁止让 `ui` 直接依赖或访问 `app` 层的状态结构体。
- **现状**：
  - ⭐ **卓越实践**：新增了 `crates/ui/tests/public_boundaries.rs` 静态测试拦截，彻底杜绝 UI 层偷偷引入 `DocumentView` / `Workspace` 等 App 层核心结构。
  - 各 UI 组件（如 `ScrollbarInput`, `SidebarWidgetInput`, `TabBarWidgetInput`）已全面切换为纯数据 struct 输入，数据提取已完全上浮至 App 层处理，完全遵守架构红线。

---

## 结论及 Action Items

今日代码重构取得了巨大的架构进步，整体规范遵守度很高。为达到完美的“代码洁癖”，建议尽快处理以下 Action Items：

- [ ] **清理废弃导入**：移除 `crates/app/src/measure_adapter.rs` 中的 `use super::*;`。
- [ ] **替换 Unwrap**：扫描并替换生产代码（尤其是 `app_scroll.rs`, `icu.rs`, `shaping/src/lib.rs`, `theme_registry.rs`）中的 `.unwrap()`，使用 `.expect("描述不会panic的原因")` 或使用安全的回退机制。
- [ ] **优化状态组合**：重新评估 `tab_bar` 中 `back_enabled` 和 `forward_enabled` 的布尔值组合，探索使用 enum 的可能性。

---

## 专项审查：faaa3f5c — 选区扁平化重构

> 提交：`faaa3f5c` feat(preview): refactor selection to flat line indices
> 范围：5 files, +265/-291

### 总体评价

方向正确，重构干净，删除了大量缺陷补偿代码。有 2 个实质性问题需要关注。

### 做得好的部分

- **删除了 `block_lines` + `block_count` + `hit_test_blocks`**：255 行的递归逻辑 + `None => continue` 补偿代码一次性消除，净删除 26 行。
- **hit test 用二分查找**：`binary_search_by` 替代 O(n) 扫描，大文档时 hit test 更快。
- **`flat_line.rect.y` 是绝对坐标**：已包含 `block_y + y_delta`，`selection_highlights` 不再需要分别加 `y_delta`，消除了加错 delta 的风险。
- **`from_doc` 和 `ensure_precise_range` 都调用 `build_flat_lines()`**：y_delta 变更后扁平行数组保持同步。
- **`PreviewPos` 从 3 元组简化为 2 元组**：范围比较从 `(block_idx, line_idx, char_pos)` 变为 `(flat_line_idx, char_pos)`，更简洁且不易出错。

### ⚠️ 问题

#### 1. 破坏封装性 —— `lazy` 字段从 `private` 改为 `pub(crate)`

**文件:** `crates/app/src/md_preview.rs:125`

```rust
// 原来
lazy: Option<LazyLayout>,

// 改为
pub(crate) lazy: Option<LazyLayout>,
```

`dispatch/editor.rs` 中多处直接访问 `mv.preview.lazy.as_ref().and_then(|l| l.flat_lines.get(...))`。暴露了内部数据结构，将来改 `LazyLayout` 时需要同步修改 dispatch 代码。

**建议修复：** 在 `MarkdownPreview` 上增加两个访问器，把 `lazy` 改回 `private`：

```rust
pub(crate) fn flat_line_at(&self, idx: usize) -> Option<&FlatLine> {
    self.lazy.as_ref()?.flat_lines.get(idx)
}
pub(crate) fn flat_line_count(&self) -> usize {
    self.lazy.as_ref().map_or(0, |l| l.flat_lines.len())
}
```

#### 2. `build_flat_lines()` 在 `ensure_precise_range` 中无条件调用

**文件:** `crates/markdown/src/layout.rs:276`

```rust
if !deltas.is_empty() {
    apply_deltas(&mut self.y_delta, &deltas);
}
self.build_flat_lines(); // ← 即使没 delta 也重建
```

当 `deltas` 为空时（scroll 区域外没有新块进入 viewport，频繁发生），`build_flat_lines()` 做了无意义的 O(n) 重建。应移入 `if !deltas.is_empty()` 块内。

### 🔍 需确认

#### 3. Table 多行 cell 的遍历顺序（预存问题，非回归）

**文件:** `crates/markdown/src/layout.rs` — `flatten_block_into` Table 分支

```rust
// 列优先：col0 所有行 → col1 所有行 → ...
for cell_lines in header {
    for line in cell_lines { ... }
}
```

正确阅读序应为行优先：`col0-line0, col1-line0, col0-line1, col1-line1`。旧代码同样有此行为，不阻塞本次提交。

### 小问题

| # | 位置 | 问题 |
|---|------|------|
| 4 | `crates/shaping/src/lib.rs` | 不相关的格式化变更（两行合并），应独立 commit |
| 5 | `dispatch/editor.rs:111-115` | `ExtendRight` 中两次调用 `mv.preview.lazy.as_ref()` 查同一行，可合并 |
| 6 | 命名不一致 | `PreviewPos.flat_line_idx` vs `FlatLine.flat_idx`，建议统一 |
| 7 | `layout.rs` HR 分支 | `font_size: 14.0` 是魔数，text 为空虽不会被使用，建议用 `0.0` 或加注释 |

### 建议优先级

| 优先级 | 项 | 理由 |
|--------|-----|------|
| **必须修** | #1 封装性 | 每新增一个 flat_lines 消费者就要重复 `lazy.as_ref().and_then(...)` 模式 |
| **应该修** | #2 性能 | 无 delta 时不重建，改动一行 |
| **可延后** | #3-7 | 预存行为/代码清洁度，非功能性问题 |
