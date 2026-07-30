# 虚拟行与视口逻辑 — 问题分析与修改方案

> 制定日期：2026-06-01
> 评审修订：2026-06-01（修正"三重滚动"为"双重滚动+死代码"，精简阶段 1 范围）
> 二次修订：2026-06-01（补救 wrap 长行下半段 autoscroll 漏洞 — 见阶段 1 1.5 节）
> 范围：`viewport.rs`、`wrap_index.rs`、`document_view/mod.rs`、`app.rs`、`render_pipeline.rs`
> 前置：`plans_viewport_0601.md` 所列 Phase 0–5 已全部完成（393 passed）

---

## 一、问题清单

### P0-1 双重自动滚动 + 一个死代码（逻辑重复 + wrap 下半段漏洞）

当前存在两套**活跃**的光标自动滚动机制 + 一套**从未被调用**的死代码：

| # | 位置 | 方法 | 精度 | 时机 | 状态 |
|---|------|------|------|------|------|
| 1 | `document_view/mod.rs:848` | `ensure_cursor_visible_sync` | 粗略（`visible_doc_line_range_approx`，假设 DisplayRow ≈ DocLine） | 每次编辑/光标移动后立即调用 | **活跃** |
| 2 | `app.rs:471` | `pre_shape_autoscroll` | 中等（`WrapIndex::doc_to_display`，对到 doc line 起点 DisplayRow） | render 前 | **活跃** |
| 3 | `app.rs:425` | `post_shape_update` | 最精确（`doc_to_display + cursor_visual_line_in_doc`） | render 后 | **死代码**（`#[allow(dead_code)]`，从未调用） |

**问题**：
- 第 1 层用近似算法，在长行 wrap 场景下计算不准（把 DisplayRow 编号当 DocLine 用）
- 第 2 层只滚到 doc line 起点 DisplayRow，**看不到光标所在的具体 visual 行** — 当光标处在长行 wrap 的下半段（visual line 偏移 > 0）时，滚动跟不上
- 第 3 层从未被调用，但它原本是消费 `cursor_visual_line_in_doc` 唯一的地方 — 没有它，wrap 长行下半段的光标可见性永远做不到精确
- 每次按键触发 2 次滚动计算，存在不必要的开销

**关键事实**：`cursor_visual_line_in_doc` 在 `render_pipeline.rs:291` 被写入后，**没有任何后续 autoscroll 消费它**。修复 wrap 下半段必须把 `post_shape_update` 接回去（或将其逻辑内联到 render 流程末尾）。

### P0-2 `visible_doc_line_range_approx` 在 wrap 场景下不可靠

`visible_doc_line_range_approx` 的实现假设 DisplayRow ≈ DocLine（1:1 映射），在长行自动换行时会错误估算可见范围。

当前在 `document_view/mod.rs` 中有 **6 处**使用，按是否生产调用区分：

| 位置 | 方法 | 是否生产调用 | 调用方 |
|------|------|------|------|
| L143 | `visible_line` | ✅ 生产 | `render_pipeline.rs:198` |
| L181 | `visible_lines` | ❌ 测试/bench 专用 | `commands.rs` 测试、`benches/scroll_bench.rs` |
| L193 | `visible_line_count` | ✅ 生产 | `render_pipeline.rs:155` |
| L199 | `visible_line_key` | ✅ 生产 | `render_pipeline.rs:170` |
| L643 | `ensure_cursor_visible` | ❌ 无调用 | （疑似死代码，需阶段 1 二次确认） |
| **L850** | **`ensure_cursor_visible_sync`** | ✅ **生产 — 关键路径** | 被所有编辑/光标移动调用 |

四个生产 API（`visible_line`/`visible_line_count`/`visible_line_key` + `ensure_cursor_visible_sync`）都依赖近似版可见范围。修复策略：
- `ensure_cursor_visible_sync` — 改用精确版（阶段 1.1）
- 其余三个 `visible_*` — `render_pipeline.rs` 是按 doc line 迭代渲染的核心 API，需要在阶段 1 确认它们使用 `visible_doc_line_range_approx` 是否会引入正确性问题。如果只是用于"按 vis_idx 取行"的粗略迭代上限，影响仅限于 wrap 场景下多迭代/少迭代几行（非崩溃），可在阶段 3 决定是改造为精确版还是接受该近似。

### P1-1 `scroll_to_doc_line` 混淆坐标空间

```rust
// viewport.rs — 近似：把 doc line 编号直接当 DisplayRow
pub fn scroll_to_doc_line(&mut self, line: usize) {
    self.scroll_to_row(line as f64);
}

// viewport.rs — 精确：通过 WrapIndex 转换
pub fn scroll_to_doc_line_wrap(&mut self, line: usize, wi: &WrapIndex) {
    let display_row = wi.doc_to_display(line);
    self.scroll_to_row(display_row as f64);
}
```

- 生产代码中 `document_view/mod.rs` 全部使用近似的 `scroll_to_doc_line`（L645, L647, L852, L855）
- 精确的 `scroll_to_doc_line_wrap` 仅在 viewport 单元测试中被调用
- 当第 0 行有 100 行 wrap 时，`scroll_to_doc_line(5)` 滚到 DisplayRow 5 而非 doc line 5 对应的 DisplayRow 100

### P1-2 `total_lines` / `total_visual_lines` 双数据源

Viewport 同时维护两个总行数：

```rust
pub struct Viewport {
    pub total_lines: usize,                  // 文档行数
    pub total_visual_lines: Option<usize>,   // 可视行数（懒计算，可能过时）
}
```

而 WrapIndex 已经精确记录了：
- `WrapIndex::len()` → 文档行数（与 `total_lines` 重复）
- `WrapIndex::total_display_rows()` → 可视行数（O(1)，始终精确）

这导致：
- `total_lines` 在 Viewport 和 WrapIndex 之间重复存储，需要手动同步
- `total_visual_lines` 是 `Option`，resize 时被设为 `None`，`clamp_scroll_top` 退化为按 doc line clamp
- 编辑时 `document_view/mod.rs` 手动同步 `viewport.total_lines`，但 `total_visual_lines` 统一设为 `None`（L710, L753）

### P2-1 `is_at_bottom` 语义错误 + 两个方法均是死代码

```rust
// viewport.rs:199 — 按 doc line 判断（语义矛盾：用 WrapIndex 转成 doc line，再和 total_lines 比）
pub fn is_at_bottom(&self, wi: &WrapIndex) -> bool {
    let first = wi.display_to_doc(self.scroll_top.floor() as usize);
    first + self.visible_rows >= self.total_lines
}

// viewport.rs:220 — 按 DisplayRow 判断
pub fn is_at_visual_bottom(&self) -> bool {
    let first = self.first_visible_row().as_usize();
    first + self.visible_rows >= self.total_visual_lines()
}
```

- `is_at_bottom` 把 DisplayRow 通过 WrapIndex 转成 doc line，再和 `total_lines` 比较 — 混合了两个坐标空间
- `is_at_visual_bottom` 依赖可能过时的 `total_visual_lines`
- **两个方法在生产代码中均无调用**，仅在测试中使用

### P2-2 `scroll_down` / `scroll_up` 混淆坐标空间 + 仅测试使用

```rust
// viewport.rs — 把 doc line delta 直接当 DisplayRow delta
pub fn scroll_down(&mut self, delta: usize) { self.scroll_by(delta as f64); }
pub fn scroll_up(&mut self, delta: usize) { self.scroll_by(-(delta as f64)); }
```

- `document_view/mod.rs` 中定义了同名 wrapper（L235-241），但生产代码无调用
- 仅在 `test_tests.rs` 中使用

### P2-3 双重 dirty 追踪机制

| 层 | 追踪方式 | 重算触发 |
|---|---|---|
| `WrapIndex.dirty[]` + `generation` | 逐行 dirty 标记 | shape 时按需重算单行 |
| `Viewport.total_visual_lines = None` | 全部标记为需要重算 | `clamp_scroll_top` 退化 |

resize 时两层同时触发：`WrapIndex.mark_all_dirty()` + `viewport.total_visual_lines = None`。

---

## 二、修改方案

### 阶段 1：精确化 + 接回精确层（P0）

**目标**：让所有自动滚动路径都精确，并补上 wrap 长行下半段的覆盖。
**文件**：`document_view/mod.rs`、`app.rs`、`render_pipeline.rs`
**约束**：不超过 3 个文件。

#### 1.1 精确化 `ensure_cursor_visible_sync`

给 `ensure_cursor_visible_sync` 添加 `&WrapIndex` 参数，改用 `scroll_to_doc_line_wrap`：

```rust
fn ensure_cursor_visible_sync(&mut self, wi: &WrapIndex) {
    let line = self.cursor_line();
    // 用 WrapIndex 精确计算可见 doc line 范围
    let first_doc = wi.display_to_doc(self.viewport.first_visible_row().as_usize());
    let last_doc = wi.display_to_doc(self.viewport.first_visible_row().as_usize()
        + self.viewport.visible_rows);
    if line < first_doc {
        self.viewport.scroll_to_doc_line_wrap(line, wi);
    } else if line >= last_doc {
        let target = line.saturating_sub(self.viewport.visible_rows - 1);
        self.viewport.scroll_to_doc_line_wrap(target, wi);
    }
}
```

所有调用点（L712, L743, L755, L803, L830, L839）同步传入 `&WrapIndex`。

> 注意：此处 `ensure_cursor_visible_sync` 仍是 **doc-line 级精确** — 仅保证 doc line 在视口内，**不能保证光标所在的 visual 行在视口内**。wrap 长行下半段的覆盖由 1.2 节的 `post_shape_update` 兜底。

#### 1.2 接回 `post_shape_update`（修复 wrap 下半段漏洞）

删除 `post_shape_update` 上的 `#[allow(dead_code)]` 标注，并在 `render()` 中调用：

**1.2.1 让 `render_pipeline::shape_visible_lines` 返回 `pending_wrap_updates`**

```rust
// render_pipeline.rs
pub fn shape_visible_lines(...) -> (Vec<Vertex>, Vec<(usize, usize)>) {
    // ...
    // 移除 L431-436 的内联 update_batch / set_total_visual_lines —
    // 这两件事改由 post_shape_update 统一做
    (all_vertices, pending_wrap_updates)
}
```

**1.2.2 在 `app.rs::render` 中调用 `post_shape_update`**

```rust
// app.rs::render
let (mut vertices, pending_wrap_updates) = self.shape_visible_lines();
// ...构造其它 vertices...
self.post_shape_update(pending_wrap_updates); // ← 新增调用
```

`post_shape_update` 内部已有：
- `wrap_index.update_batch(&pending)` → 索引精确化（替代 render_pipeline 内联版本）
- `viewport.set_total_visual_lines(total)` → 总数同步
- 基于 `cursor_visual_line_in_doc` 的精确 autoscroll → **修复 wrap 下半段漏洞**

> redraw 死循环防护：`post_shape_update` 内部仅在 `cursor_offset != last_cursor_offset` 时触发；下一帧 `cursor_offset` 不再变 → 不会循环。

#### 1.3 删除 `pre_shape_autoscroll`

阶段 1.1 + 1.2 完成后，`pre_shape_autoscroll` 不再需要：
- `ensure_cursor_visible_sync` 已 doc-line 精确
- `post_shape_update` 已 visual-line 精确

删除该方法（`app.rs:471`），并在 `render()` 中移除调用（`app.rs:499`）。

> 若实测发现"按下方向键到 render 之间存在 1 帧延迟"的视觉抖动，可保留 `pre_shape_autoscroll` 作为 render 前的快速 doc-level 校正（精确版，不再用近似）。

#### 1.4 处理 5 个 `visible_*` API 的近似依赖

经实测 grep（见 P0-2 表格），它们的状态如下：

| 方法 | 处置 |
|------|------|
| `visible_line` (L143) | **生产用**（render_pipeline.rs:198）— 改造为接受 `&WrapIndex`，使用精确版 |
| `visible_line_count` (L193) | **生产用**（render_pipeline.rs:155）— 同上 |
| `visible_line_key` (L199) | **生产用**（render_pipeline.rs:170）— 同上 |
| `visible_lines` (L181) | 仅测试/bench 用 — 标记 `#[cfg(any(test, feature = "bench"))]` 或在 bench 中改为按需调用其它 API |
| `ensure_cursor_visible` (L643) | 无调用方 — 直接删除（已被 `ensure_cursor_visible_sync` 取代） |

`visible_doc_line_range_approx` 在阶段 1 完成后将仅剩 `visible_lines` 一个调用方（测试/bench），可在阶段 3 决定是保留为 `#[cfg(test)]` 还是删除。

#### 1.5 WrapIndex 可用性 fallback

`document_view` 中需持有 `Option<&WrapIndex>` 引用（或在调用处传入）。当 WrapIndex 为 `None` 时（如初始化阶段），fallback 到当前的近似逻辑：

```rust
fn ensure_cursor_visible_sync(&mut self, wi: Option<&WrapIndex>) {
    let line = self.cursor_line();
    if let Some(wi) = wi {
        // 精确路径
        let first_doc = wi.display_to_doc(self.viewport.first_visible_row().as_usize());
        let last_doc = wi.display_to_doc(
            self.viewport.first_visible_row().as_usize() + self.viewport.visible_rows
        );
        if line < first_doc {
            self.viewport.scroll_to_doc_line_wrap(line, wi);
        } else if line >= last_doc {
            let target = line.saturating_sub(self.viewport.visible_rows - 1);
            self.viewport.scroll_to_doc_line_wrap(target, wi);
        }
    } else {
        // fallback：近似路径（与当前逻辑相同）
        let range = self.viewport.visible_doc_line_range_approx();
        if line < range.start {
            self.viewport.scroll_to_doc_line(line);
        } else if line >= range.end {
            self.viewport.scroll_to_doc_line(line.saturating_sub(self.viewport.visible_rows - 1));
        }
    }
}
```

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- **关键回归用例**：构造一个超长行（让它 wrap 到 30+ 行），把光标移到该行中段/末尾，确认视口跟到光标所在 visual 行（而非停在 doc line 起点）
- 普通短行的方向键移动行为不变
- 连续快速按键不出现闪烁

---

### 阶段 2：统一数据源（P1）

**目标**：消除 Viewport 中的 `total_lines` / `total_visual_lines`，统一由 WrapIndex 提供。
**文件**：`viewport.rs`、`document_view/mod.rs`、`app.rs`
**约束**：不超过 3 个文件。

#### 2.1 移除 Viewport 的 `total_lines` 字段

```rust
pub struct Viewport {
    pub scroll_top: f64,
    pub visible_rows: usize,
    // 删除: pub total_lines: usize,
    // 删除: pub total_visual_lines: Option<usize>,
}
```

所有需要 `total_lines` 的地方改为查询 `WrapIndex::len()`。
所有需要 `total_visual_lines` 的地方改为查询 `WrapIndex::total_display_rows()`。

#### 2.2 修改 `clamp_scroll_top` 接受 WrapIndex

```rust
pub fn clamp_scroll_top(&mut self, wi: &WrapIndex) {
    let total = wi.total_display_rows().max(wi.len()); // 取较大值兜底
    let max_visual = total.saturating_sub(self.visible_rows) as f64;
    if self.scroll_top > max_visual {
        self.scroll_top = max_visual.max(0.0);
    }
}

/// Fallback: WrapIndex 不可用时（如初始化阶段），仅用 visible_rows 兜底。
pub fn clamp_scroll_top_no_wrap(&mut self) {
    // 无 total_lines 信息时，只保证 scroll_top >= 0
    if self.scroll_top < 0.0 {
        self.scroll_top = 0.0;
    }
}
```

#### 2.3 更新所有构造和调用点

- `Viewport::new(visible_rows)` — 不再需要 `total_lines` 参数
- `set_total_lines` → 删除，改由 `WrapIndex.resize` + `clamp_scroll_top(wi)` 处理
- `set_total_visual_lines` → 删除
- `total_visual_lines()` → 删除，改由 `WrapIndex.total_display_rows()` 替代
- `is_at_visual_bottom()` → 改为 `is_at_visual_bottom(&self, wi: &WrapIndex)`
- 编辑后更新路径：`document_view/mod.rs` 中不再手动同步 `viewport.total_lines`

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 大文件加载 → 滚动到底 → scrollbar 不飘
- resize 后滚动位置正确

---

### 阶段 3：清理死代码和近似方法（P2）

**目标**：删除仅测试使用的生产代码，减少维护负担。
**文件**：`viewport.rs`、`document_view/mod.rs`、`app.rs`

#### 3.1 删除 `is_at_bottom`（语义错误 + 死代码）

仅在测试中使用，且实现有语义矛盾。删除方法和对应测试。

#### 3.2 删除 `scroll_down` / `scroll_up`（死代码）

仅在测试中使用。Viewport 和 DocumentView 中的方法均删除。
测试改为直接使用 `scroll_by(delta as f64)` 或 `scroll_to_doc_line_wrap`。

#### 3.3 删除 `post_shape_update`（死代码）

`app.rs:425` — 标注 `#[allow(dead_code)]`，从未被调用。删除该方法。

#### 3.4 评估 `scroll_to_doc_line` 是否保留

如果阶段 1 完成后所有调用方都已改为 `scroll_to_doc_line_wrap`，则：
- `scroll_to_doc_line` 变为死代码，可删除
- 或保留为 convenience 方法，但加 `#[deprecated]` 注解提醒使用精确版

#### 3.5 评估 `visible_doc_line_range_approx` 是否保留

阶段 1 完成后，`visible_*` 系列生产 API 已改用 WrapIndex 精确版，`visible_doc_line_range_approx` 仅剩 `visible_lines`（测试/bench）一个调用方：
- 直接标记 `#[cfg(test)]` 或与 `visible_lines` 一起删除
- 同时删除已被取代的 `ensure_cursor_visible`（生产中无调用方）

**验证**：
- `cargo test -p edit-plus-app --lib` 全绿
- 确认无编译警告（dead code）

---

### 阶段 4：统一 dirty 追踪（P2，可选）

**目标**：消除 Viewport 和 WrapIndex 的双重 dirty 机制。
**文件**：`viewport.rs`、`app.rs`

#### 4.1 resize 路径统一

```rust
// app.rs — resize handler
fn handle_resize(&mut self, new_size: ...) {
    let visible_rows = ...;
    if let Some(dv) = self.doc_view.as_mut() {
        dv.viewport.resize(visible_rows); // 只更新 visible_rows
    }
    self.wrap_index.mark_all_dirty(); // WrapIndex 负责 dirty
    // clamp_scroll_top 在下一帧 shape 前调用，传入 wrap_index
}
```

#### 4.2 `clamp_scroll_top` 使用 WrapIndex 兜底

当 `total_display_rows` 尚未精确计算时（dirty 行多），用 `wrap_index.len()` 作为下界：
- 至少不会 clamp 到 doc line 范围之外
- 随着 shape 进行逐行变精确，scroll_top 会自然收敛

**验证**：
- resize 后立即滚动到新位置，不出现跳动
- `cargo test -p edit-plus-app --lib` 全绿

---

## 三、依赖关系

```
阶段 1（消除近似方法）
  ↓
阶段 2（统一数据源）— 依赖阶段 1 完成，因为 clamp_scroll_top 需要 WrapIndex
  ↓
阶段 3（清理死代码）— 依赖阶段 2 完成，确认无调用方后删除
  ↓
阶段 4（统一 dirty）— 独立，但建议在阶段 2 之后做
```

---

## 四、风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| 阶段 1 给 DocumentView 加 WrapIndex 参数影响面大 | 实际只需改 `ensure_cursor_visible_sync` 1 处生产方法（其余 5 处为测试专用），影响面可控 |
| WrapIndex 为 None 时（如初始化阶段）无法调用精确方法 | `ensure_cursor_visible_sync` 接受 `Option<&WrapIndex>`，为 `None` 时 fallback 到近似逻辑；`clamp_scroll_top` 保留无参版本 `clamp_scroll_top_no_wrap` |
| 阶段 2 移除 `total_lines` 后某些边缘路径遗漏 | 先搜索所有 `total_lines` 和 `total_visual_lines` 的引用，逐一替换 |
| 阶段 3 删除方法导致测试编译失败 | 同步更新或删除对应测试 |

---

## 五、预期收益

| 指标 | 当前 | 修改后 |
|------|------|--------|
| 活跃自动滚动层数 | 2 层（`ensure_cursor_visible_sync` 近似 + `pre_shape_autoscroll` doc-level 精确） | 2 层职责清晰：`ensure_cursor_visible_sync`（doc-level 精确）+ `post_shape_update`（visual-level 精确） |
| 死代码自动滚动 | 1 层（`post_shape_update`） | 0（已接回，不再是死代码） |
| `visible_doc_line_range_approx` 生产调用 | 4 处（`visible_line`/`visible_line_count`/`visible_line_key`/`ensure_cursor_visible_sync`） | 0 处 |
| 数据源数量（总行数） | 4 个字段（Viewport 2 + WrapIndex 2） | 2 个字段（仅 WrapIndex） |
| 死代码方法 | 6 个（is_at_bottom, is_at_visual_bottom, scroll_down/up, ensure_cursor_visible, scroll_to_doc_line 待评估） | 0 |
| wrap 短行光标可见性 | 近似（可能出错） | 精确（doc-level 经 WrapIndex O(log n)） |
| **wrap 长行下半段光标可见性** | **缺失（无任何层覆盖）** | **精确（`cursor_visual_line_in_doc` 接入 post_shape_update）** |
