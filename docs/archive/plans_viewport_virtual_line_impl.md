# 虚拟行与视口逻辑 — 实施计划

> 基于 `plans_viewport_virtual_line_analysis.md` 分析文档
> 制定日期：2026-06-01
> 前置：`plans_viewport_0601.md` Phase 0–5 已完成

---

## 一、现状对比（分析文档 vs 实现）

### 已完成项 ✅

| 分析文档条目 | 状态 | 说明 |
|---|---|---|
| P0-1: `post_shape_update` 接回渲染流程 | ✅ 已完成 | `app.rs:607` 已调用，`#[allow(dead_code)]` 已移除 |
| P0-1: `ensure_cursor_visible_sync` 接受 `Option<&WrapIndex>` | ✅ 已完成 | 签名已改造，有 `Some(wi)` 精确分支和 `None` fallback 分支 |
| P1-2: 移除 `total_visual_lines` 字段 | ✅ 已完成 | 字段已删除，测试中仅剩注释 |
| P1-2: `clamp_scroll_top` 接受 WrapIndex | ✅ 已完成 | `clamp_scroll_top(&self, wi: &WrapIndex)` 已存在 |
| P1-2: `is_at_visual_bottom` 改为接受 WrapIndex | ✅ 已完成 | 签名已改造 |
| 阶段 3.1: 删除 `is_at_bottom` | ✅ 已完成 | 方法已删除 |
| 阶段 3.3: 删除 `ensure_cursor_visible`（不带 _sync） | ✅ 已完成 | 方法已删除 |

### 未完成项 ❌

| 分析文档条目 | 状态 | 当前问题 |
|---|---|---|
| **P0-1: 消除三重滚动，收敛为两层** | ❌ 未完成 | 三层仍全部活跃：`ensure_cursor_visible_sync`(6次调用全传None) + `pre_shape_autoscroll` + `post_shape_update` |
| **P0-2: `visible_line`/`visible_line_count`/`visible_line_key` 改用精确范围** | ❌ 未完成 | 三个方法仍内部调用 `visible_doc_line_range_approx`，render_pipeline.rs 通过它们间接使用近似版 |
| **P1-1: `ensure_cursor_visible_sync` 内部调用传 WrapIndex** | ❌ 未完成 | `document_view/mod.rs` 中 6 处调用全部传 `None`，始终走 fallback 近似路径 |
| **P1-1: `scroll_to_doc_line` 近似调用** | ❌ 未完成 | `ensure_cursor_visible_sync` 的 None 分支仍调用 `scroll_to_doc_line` |
| **P1-2: 移除 `total_lines` 字段** | ❌ 未完成 | `Viewport.total_lines` 仍存在，与 WrapIndex 重复 |
| **阶段 3.2: 删除 `scroll_down`/`scroll_up`** | ❌ 未完成 | 方法仍存在（viewport.rs:219,224 + document_view/mod.rs:250,254） |
| **阶段 3.3: 评估 `post_shape_update`** | ⚠️ 需调整 | 已不再是死代码，应保留；但 `pre_shape_autoscroll` 应被取代后删除 |
| **阶段 3.4: 评估 `scroll_to_doc_line`** | ❌ 未完成 | 仍存在，fallback 路径仍在使用 |
| **阶段 3.5: 评估 `visible_doc_line_range_approx`** | ❌ 未完成 | 仍存在，3 个 visible_* 方法 + 测试仍在使用 |
| **阶段 4: 统一 dirty 追踪** | ❌ 未完成 | 可选阶段，未开始 |

---

## 二、实施计划

### 阶段 1：消除三重滚动 → 双层精确滚动（P0）

**目标**：将三层自动滚动收敛为两层职责清晰的精确滚动。
**文件**：`document_view/mod.rs`、`app.rs`

#### 1.1 `ensure_cursor_visible_sync` 传入 WrapIndex

**问题**：`document_view/mod.rs` 中 6 处调用 `ensure_cursor_visible_sync(None)`，始终走近似 fallback。

**方案**：
- 需要让 `DocumentView` 的编辑/光标移动方法能访问到 `WrapIndex`
- 有两种路径：
  - **方案 A**：将 `WrapIndex` 引用传入 `DocumentView` 的编辑方法（如 `insert_char`、`delete_char`、`move_cursor_*` 等）
  - **方案 B**：将 `ensure_cursor_visible_sync` 的调用从 `DocumentView` 内部移到 `App` 层，在 `App` 中持有 `WrapIndex` 后调用
- **推荐方案 B**：因为 `WrapIndex` 是 `App` 级资源，`DocumentView` 不应持有它。将 `ensure_cursor_visible_sync(None)` 调用点改为在 `App` 层调用 `ensure_cursor_visible_sync(Some(&self.wrap_index))`

**影响面**：6 处调用点（L712, L743, L755, L803, L830, L839）

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 长行 wrap 场景下光标移动，viewport 精确跟随

#### 1.2 移除 `pre_shape_autoscroll`（收敛为两层）

**问题**：`ensure_cursor_visible_sync` 变精确后，`pre_shape_autoscroll` 的 doc-level 粗略滚动变得多余。

**方案**：
- 删除 `app.rs` 中的 `pre_shape_autoscroll` 方法
- 删除 `render()` 中对 `self.pre_shape_autoscroll()` 的调用（约 L504）
- 保留 `post_shape_update`（visual-level 精确滚动，消费 `cursor_visual_line_in_doc`）

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 光标跨越 viewport 边界时不出现 1 帧延迟闪烁

---

### 阶段 2：`visible_*` 方法改用精确范围（P0）

**目标**：`visible_line`/`visible_line_count`/`visible_line_key` 改用 WrapIndex 精确计算可见范围。
**文件**：`document_view/mod.rs`、`render_pipeline.rs`

#### 2.1 给 `visible_line`/`visible_line_count`/`visible_line_key` 加 WrapIndex 参数

**问题**：这三个方法内部调用 `visible_doc_line_range_approx`，在 wrap 场景下范围不准。

**方案**：
- 新增带 WrapIndex 参数的版本：
  - `visible_line_wrap(&self, vis_idx: usize, wi: &WrapIndex) -> Option<Cow<'_, [u8]>>`
  - `visible_line_count_wrap(&self, wi: &WrapIndex) -> usize`
  - `visible_line_key_wrap(&self, vis_idx: usize, wi: &WrapIndex) -> Option<(usize, usize)>`
- 内部改用 `self.viewport.visible_doc_line_range(wi)` 精确版
- `render_pipeline.rs` 已经持有 `wrap_index`，改为调用 `_wrap` 版本

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 长行 wrap 场景下渲染行数正确（不多不少）

#### 2.2 评估是否保留原版方法

- `visible_lines()` 仅在测试/bench 中使用 → 标记 `#[cfg(test)]` 或删除
- `visible_line`/`visible_line_count`/`visible_line_key` 如果所有生产调用都改为 `_wrap` 版本 → 标记 `#[cfg(test)]` 或删除

---

### 阶段 3：统一数据源 — 移除 `total_lines`（P1）

**目标**：Viewport 不再维护 `total_lines`，统一由 WrapIndex 提供。
**文件**：`viewport.rs`、`document_view/mod.rs`、`app.rs`

#### 3.1 搜索所有 `total_lines` 引用并逐一替换

**当前引用点**（viewport.rs）：
- 字段定义（L102）
- `new()` 构造函数（L107, L111）
- `set_total_lines()`（L118-121）
- `resize()`（L127）
- `visible_doc_line_range_approx()`（L164-167）
- `scroll_by()`（L174）
- `scroll_to_row()`（L180）
- `clamp_scroll_top_no_wrap()`（L193-194）

**方案**：
- `Viewport::new()` 不再接收 `total_lines`，移除该字段
- `set_total_lines()` → 删除（改由 `WrapIndex` 管理行数）
- `resize()` → 不再调用 `clamp_scroll_top_no_wrap(self.total_lines)`，改为需要外部传入 WrapIndex 或调用 `clamp_scroll_top(wi)`
- `scroll_by()` / `scroll_to_row()` → 不再内部 clamp，或改为需要 WrapIndex 参数
- `document_view/mod.rs` 中 `Viewport::new(visible_rows, total_lines)` 调用需要适配

**风险**：`WrapIndex` 为 `None` 时（初始化阶段）需要 fallback。
**缓解**：保留 `clamp_scroll_top_no_wrap` 作为 fallback，但参数由调用方传入（如 `line_index.line_count()`）。

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 大文件加载 → 滚动到底 → scrollbar 不飘
- resize 后滚动位置正确

---

### 阶段 4：清理死代码（P2）

**目标**：删除仅测试使用的生产代码，减少维护负担。
**文件**：`viewport.rs`、`document_view/mod.rs`、`app.rs`

#### 4.1 删除 `scroll_down` / `scroll_up`

- Viewport 中（L219, L224）和 DocumentView 中（L250, L254）
- 仅在测试中使用
- 测试改为直接使用 `scroll_by(delta as f64)` 或 `scroll_to_doc_line_wrap`

#### 4.2 评估 `scroll_to_doc_line` 是否保留

- 阶段 1 完成后，如果所有调用方都已改为精确版，可删除或标记 `#[deprecated]`

#### 4.3 评估 `visible_doc_line_range_approx` 是否保留

- 阶段 2 完成后，如果生产调用都改为精确版，可标记 `#[cfg(test)]` 或删除

#### 4.4 清理 `is_at_visual_bottom` 空测试

- 测试模块中所有 assert 都被注释掉了，要么补全测试，要么删除测试模块

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 无编译警告（dead code）

---

### 阶段 5：统一 dirty 追踪（P2，可选）

**目标**：消除 Viewport 和 WrapIndex 的双重 dirty 机制。
**文件**：`viewport.rs`、`app.rs`

#### 5.1 resize 路径统一

```rust
fn handle_resize(&mut self, new_size: ...) {
    let visible_rows = ...;
    if let Some(dv) = self.doc_view.as_mut() {
        dv.viewport.resize(visible_rows);
    }
    self.wrap_index.mark_all_dirty();
}
```

#### 5.2 `clamp_scroll_top` 使用 WrapIndex 兜底

当 `total_display_rows` 尚未精确计算时（dirty 行多），用 `wrap_index.len()` 作为下界。

**验证**：
- resize 后立即滚动到新位置，不出现跳动
- `cargo test -p edit-plus-app --lib` 全绿

---

## 三、依赖关系

```
阶段 1（消除三重滚动 → 双层精确）
  ↓
阶段 2（visible_* 改用精确范围）— 可与阶段 1 并行
  ↓
阶段 3（移除 total_lines）— 依赖阶段 1，因为 scroll_by/scroll_to_row 需要适配
  ↓
阶段 4（清理死代码）— 依赖阶段 1-3 完成，确认无调用方后删除
  ↓
阶段 5（统一 dirty）— 独立，建议在阶段 3 之后
```

---

## 四、风险与缓解

| 风险 | 缓解措施 |
|---|---|
| 阶段 1.1 方案 B 将 `ensure_cursor_visible_sync` 调用移到 App 层，可能遗漏某些编辑路径 | 先 grep 所有 `ensure_cursor_visible_sync` 调用点，逐一确认 |
| 阶段 3 移除 `total_lines` 后初始化路径缺少行数信息 | 保留 `clamp_scroll_top_no_wrap` fallback，参数由 `line_index.line_count()` 提供 |
| 阶段 2 给 `visible_*` 加 `_wrap` 版本增加 API 表面积 | 阶段 4 统一清理，删除近似版 |

---

## 五、预期收益

| 指标 | 当前 | 修改后 |
|---|---|---|
| 活跃自动滚动层数 | 3 层（`ensure_cursor_visible_sync` 近似 + `pre_shape_autoscroll` + `post_shape_update`） | 2 层（`ensure_cursor_visible_sync` 精确 + `post_shape_update` 精确） |
| `visible_doc_line_range_approx` 生产调用 | 3 处（`visible_line`/`visible_line_count`/`visible_line_key`） | 0 处 |
| 数据源数量（总行数） | 3 个（Viewport.total_lines + WrapIndex.len() + WrapIndex.total_display_rows()） | 2 个（仅 WrapIndex） |
| 死代码方法 | `scroll_down`/`scroll_up` + `scroll_to_doc_line` 待评估 | 0 |
| wrap 长行下半段光标可见性 | ✅ 已精确（`post_shape_update` 已接回） | ✅ 精确 |
| wrap 场景下 ensure_cursor_visible 精度 | ❌ 近似（6 处传 None） | ✅ 精确 |
