# 视口滚动重新设计方案

## 1. Zed 是怎么做的

### 1.1 核心数据结构：SumTree Transform 链

Zed 的 DisplayMap 由 6 层 Transform 链组成：

```
Buffer → InlayMap → FoldMap → TabMap → WrapMap → BlockMap → DisplayMap
```

每一层持有一棵 **SumTree<Transform>**（持久化平衡树），每个 Transform 节点存储：
- `input: TextSummary` — 下层的文本摘要（行数、字节数等）
- `output: TextSummary` — 上层的文本摘要

SumTree 的每个节点维护子树的累计 summary，所以可以在 **O(log n)** 时间内做任意坐标空间之间的转换。

### 1.2 ScrollAnchor：Buffer 空间的锚点

```rust
pub struct ScrollAnchor {
    pub offset: gpui::Point<f64>,  // 亚行像素偏移
    pub anchor: Anchor,            // Buffer 中的稳定锚点（不受 wrap/编辑影响）
}
```

**不是 DisplayRow 号，而是 Buffer 中的锚点位置。**

滚动时：
1. `scroll_position.y` = `anchor.to_display_point(snapshot).row() + offset.y`（绝对 DisplayRow）
2. `start_row = DisplayRow(scroll_position.y.floor())`
3. `end_row = DisplayRow((scroll_position.y + visible_height).ceil())`

文本编辑或窗口 resize 导致 rewrap 后，anchor 不变，但 `to_display_point()` 的结果自动更新（因为 SumTree 已增量更新）。

### 1.3 WrapMap：SumTree 管理 wrap transforms

```rust
pub struct WrapSnapshot {
    tab_snapshot: TabSnapshot,
    transforms: SumTree<Transform>,  // 关键！
}
```

每个 Transform 是两种之一：
- **Isomorphic**（透传）：input 行数 == output 行数，如短行不需要 wrap
- **Wrapped**（换行）：1 条 input 行 → N 条 output 行

SumTree 维护 `Dimensions<WrapPoint, TabPoint>` 双向索引：
- `WrapPoint → TabPoint`：给定 DisplayRow，找到对应的 Buffer Row（O(log n)）
- `TabPoint → WrapPoint`：给定 Buffer Row，找到对应的 DisplayRow（O(log n)）

### 1.4 增量更新：只修改受影响的 Transform

文本编辑时：
1. 编辑产生 `TabEdit`（受影响的 TabRow 范围）
2. WrapMap 只对受影响的行重新计算 wrap
3. 用 SumTree 的 `slice` + `append` 操作，保留未受影响的 Transform
4. 如果 wrap width 变了（resize），后台异步任务逐批 rewrap，每 100 行 yield 一次

### 1.5 渲染：从 DisplayRow 直接迭代

```rust
let start_row = DisplayRow(scroll_position.y.floor() as u32);
let row_infos = snapshot.row_infos(start_row)
    .take(visible_rows)
    .collect();
```

`row_infos(start_row)` 内部：
1. SumTree cursor seek 到 `start_row`
2. 逐行 yield RowInfo（包含是否 soft wrapped、buffer row 号等）
3. 不需要 skip_remaining 之类的 hack——SumTree 精确定位

---

## 2. 当前 edit+ 的结构性问题

| 问题 | 根因 |
|---|---|
| `first_visible_doc_line` 不精确 | `sync_doc_line_from_scroll_top()` 用 `scroll_top.floor()` 近似，不感知 wrap |
| `advance_cache.display_row` 帧相对 | 每帧清空重建，从 0 开始计数，丢失绝对位置 |
| `update_first_visible_doc_line` 错误 | 用 absolute `first_visible_row()` 搜 frame-relative cache |
| `skip_visual` 计算依赖近似值 | 跟着 `first_visible_doc_line` 的错误走 |
| `visible_doc_line_range()` 不感知 wrap | 假设每条 doc line = 1 DisplayRow |
| 无持久化索引 | 每帧重建所有数据，滚动超出缓存范围就丢失 |
| 三处写 `first_visible_doc_line` 互相覆盖 | `sync_doc_line`、`update_first_visible`、autoscroll |

**核心缺陷**：没有一个持久化的、O(log n) 的 DisplayRow↔DocLine 映射。

---

## 3. 改造方案

### 3.1 目标

- `scroll_top`（absolute DisplayRow）↔ DocLine 转换精确，O(log n)
- 滚动任意距离都不需要猜测或近似
- 文本编辑后增量更新，不重建全量索引
- 去掉所有 frame-relative 混用

### 3.2 架构：引入 WrapIndex（SumTree 替代品）

Zed 用的是通用 SumTree 库（`sum_tree` crate），实现成本高。edit+ 可以用更轻量的方案达到同样效果：

**方案 A：Segment Tree（推荐）**

```rust
/// 持久化的 wrap 索引。每条 doc line 记录其 visual line count。
/// Segment Tree 维护区间 sum，支持 O(log n) 的：
///   - doc_line → absolute DisplayRow（前缀 sum）
///   - absolute DisplayRow → doc_line（二分搜索）
struct WrapIndex {
    /// leaf[i] = doc line i 的 visual line count（≥1）
    tree: Vec<usize>,  // segment tree 内部节点
    len: usize,        // doc line 数量
}
```

操作：
- `doc_to_display(doc_line)` → `sum(0..doc_line)` — O(log n)
- `display_to_doc(display_row)` → lower_bound search — O(log n)
- `update(doc_line, new_visual_count)` — O(log n)
- `update_range(start, end, counts)` — O(k log n)，k = 变更行数

空间：O(n)，n = doc line 数量。18000 行的 JSON 文件只需 ~144KB。

**方案 B：Sparse BTreeMap（更简单）**

只记录 wrap 点（visual_count > 1 的行），用 BTreeMap 做前缀 sum：
- 大多数行 visual_count=1，不存储
- 查找时：`display_row = doc_line + prefix_sum_of_wrap_excess(doc_line)`
- 更新：只更新变化的 wrap 点

空间更小，但最坏情况仍是 O(n)。

### 3.3 数据流改造

#### 当前（有问题）：
```
scroll_by(delta)
  → scroll_top += delta (absolute DisplayRow)
  → sync_doc_line_from_scroll_top() ← 近似！
    → first_visible_doc_line = scroll_top.floor() (假设 1:1)
  → 下一帧: update_first_visible_doc_line() ← 用上一帧残缺 cache 纠错
  → 渲染循环: visual_line_counter 从 0 开始 ← 不知道绝对位置
```

#### 改造后：
```
scroll_by(delta)
  → scroll_top += delta (absolute DisplayRow)
  → first_visible_doc_line = wrap_index.display_to_doc(scroll_top.floor()) ← 精确！
  → skip_visual = scroll_top.floor() - wrap_index.doc_to_display(first_visible_doc_line)
  → 渲染循环: 从 first_visible_doc_line 开始，skip skip_visual 行
  → 渲染完成后: wrap_index 增量更新（只更新变化的行）
```

### 3.4 具体修改

#### Phase 1：实现 WrapIndex

新增 `crates/app/src/wrap_index.rs`：

```rust
pub struct WrapIndex {
    // Segment tree: tree[i] = sum of leaf values in i's subtree
    tree: Vec<usize>,
    n: usize, // number of leaves (doc lines)
}

impl WrapIndex {
    pub fn new(doc_line_count: usize) -> Self;
    pub fn resize(&mut self, new_count: usize);

    /// absolute DisplayRow → doc line（二分搜索）
    pub fn display_to_doc(&self, display_row: usize) -> usize;

    /// doc line → absolute DisplayRow（前缀 sum）
    pub fn doc_to_display(&self, doc_line: usize) -> usize;

    /// doc line 的 visual line count
    pub fn visual_line_count(&self, doc_line: usize) -> usize;

    /// 批量更新 visual line counts（编辑或 rewrap 后）
    pub fn update(&mut self, doc_line: usize, visual_count: usize);

    /// 总 DisplayRow 数
    pub fn total_display_rows(&self) -> usize;
}
```

测试用例：
- 空文档、单行、多行无 wrap、多行有 wrap
- 超长行（1000+ visual lines）
- `display_to_doc` 和 `doc_to_display` 互逆
- 边界：display_row=0, display_row=total
- 性能：18000 行，1000 次随机查询 < 1ms

#### Phase 2：Viewport 改用 WrapIndex

修改 `viewport.rs`：

1. **去掉 `first_visible_doc_line` 字段**，改为通过 WrapIndex 实时计算：
   ```rust
   pub fn first_visible_doc_line(&self, index: &WrapIndex) -> usize {
       index.display_to_doc(self.scroll_top.floor() as usize)
   }
   ```

2. **去掉 `sync_doc_line_from_scroll_top()`** — 不再需要近似同步

3. **去掉 `update_first_visible_doc_line()` 参数** — 不再需要 VLI offset

4. **`visible_doc_line_range()` 改为精确版本**：
   ```rust
   pub fn visible_doc_line_range(&self, index: &WrapIndex) -> Range<usize> {
       let start = index.display_to_doc(self.scroll_top.floor() as usize);
       let end_display = (self.scroll_top + self.visible_rows as f64).ceil() as usize;
       let end = index.display_to_doc(end_display.min(index.total_display_rows()));
       start..end + 1
   }
   ```

5. **`scroll_to_doc_line()` 改为精确版本**：
   ```rust
   pub fn scroll_to_doc_line(&mut self, line: usize, index: &WrapIndex) {
       let display_row = index.doc_to_display(line);
       self.scroll_to_row(display_row as f64);
   }
   ```

#### Phase 3：渲染循环改造

修改 `shape_visible_lines()`：

1. **删除 `update_first_visible_doc_line()` 调用** — 用 WrapIndex 实时计算

2. **`skip_visual` 直接从 WrapIndex 计算**：
   ```rust
   let first_doc = wrap_index.display_to_doc(scroll_top.floor() as usize);
   let first_doc_display = wrap_index.doc_to_display(first_doc);
   let skip_visual = scroll_top.floor() as usize - first_doc_display;
   ```
   精确，不需要 carry-over hack。

3. **advance_cache.store` display_row 存 absolute 值**：
   ```rust
   let absolute_display_row = wrap_index.doc_to_display(doc_line_idx) + vl_in_doc - skip_visual;
   display_row: DisplayRow(absolute_display_row as u32),
   ```

4. **删除 `first_visible_vl_offset`、`skip_remaining` 等 workaround**

5. **autoscroll 简化**：
   ```rust
   let cursor_abs_display = wrap_index.doc_to_display(cursor_doc_line) + cursor_visual_line_in_doc;
   // 直接用 absolute 值，不需要 doc_line_map 或 fallback
   ```

#### Phase 4：增量更新

编辑后更新 WrapIndex：

1. 文本编辑 → 行数变化 → `wrap_index.resize(new_line_count)`
2. 编辑影响的行重新计算 visual_line_count → `wrap_index.update(line, count)`
3. WrapIndex 内部 segment tree 自动更新区间 sum

不需要重建整个索引。每次编辑只更新受影响的 O(k) 行，每行 O(log n)。

#### Phase 5：cleanup

- 删除 `VisualLineIndex`（viewport.rs 中已废弃的旧结构）
- 删除 `doc_line_map`（被 WrapIndex 替代）
- 删除 `first_visible_vl_offset`（不再需要）
- 删除 `first_line_visual_lines` / `first_line_clusters` 中的 skip 相关逻辑
- 简化 `ensure_cursor_visible_sync` — 直接用 WrapIndex 做精确判断

### 3.5 边界情况

| 场景 | 处理 |
|---|---|
| WrapIndex 为空（文件未加载） | `display_to_doc` 返回 0，`doc_to_display` 返回 0 |
| 编辑导致行数变化 | `resize()` 扩展/收缩 segment tree |
| wrap width 变化（窗口 resize） | 后台 rewrap → 批量 `update()` |
| 超长行（>10000 visual lines） | Segment tree 正常处理，值域 usize 够用 |
| 滚动到文件末尾 | `clamp_scroll_top` 用 `total_display_rows()` |

### 3.6 与 Zed 的差异

| 维度 | Zed | edit+（改造后） |
|---|---|---|
| 数据结构 | SumTree（通用，支持任意 transform） | Segment Tree（专用，只做 sum） |
| 坐标空间 | 6 层 Transform 链 | 1 层 WrapIndex（够用） |
| 滚动锚点 | Buffer Anchor（稳定） | DisplayRow（够用，无多 buffer 需求） |
| 增量更新 | SumTree splice | Segment tree point update |
| 复杂度 | 高（通用框架） | 低（专用实现） |

Zed 的 6 层 Transform 是为了支持 inlay hints、folds、tabs、wraps、blocks 等复杂功能。
edit+ 当前只需要 wrap 一层，Segment Tree 足矣。未来加 fold/inlay 时可以扩展。

---

## 4. 风险和缓解

| 风险 | 缓解 |
|---|---|
| Segment Tree 实现 bug | 充分的单元测试 + 与暴力前缀 sum 对比测试 |
| 编辑后 WrapIndex 更新不及时 | 在 shape_visible_lines 末尾同步更新 |
| wrap width 变化时的 rewrap 延迟 | 可以先同步 rewrap（小文件），后改为异步 |
| 旧代码残留导致回归 | Phase 5 彻底 cleanup，删除所有 workaround |
