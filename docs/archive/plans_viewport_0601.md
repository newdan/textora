# Viewport 架构重构 & 性能优化 — 执行计划

> 基于 `docs/viewport_0601.md` 审计结果
> 制定日期：2026-06-01
> 约束：每阶段独立可编译、可测试；改动不超过 3 个文件；先设计接口再实现

---

## 总览

将 12 个结构性问题 + 9 个性能问题分为 6 个阶段，按依赖顺序排列：

```
Phase 0  快速修复（哨兵值、小 Bug）
   ↓
Phase 1  WrapIndex 成为唯一事实来源（脏标记 + generation + 预热）
   ↓
Phase 2  消除 Viewport 双重派生缓存
   ↓
Phase 3  拆分 shape_visible_lines（390 行 → 6 个函数）
   ↓
Phase 4  性能优化（wrap 缓存、对象池、shape key、autoscroll 前移）
   ↓
Phase 5  剩余性能优化（行索引增量、选区零拷贝、cursor_line 缓存）
```

---

## Phase 0：快速修复（低风险、高收益）

**目标：** 修复明确 Bug，消除代码异味，为后续阶段扫清障碍。
**文件：** `app.rs`, `viewport.rs`
**预估：** 1 个会话

### 0.1 cursor_visual_line 哨兵值 → Option\<usize\>
- 问题 #11
- 将 `cursor_visual_line: usize` 改为 `Option<usize>`
- 所有 `usize::MAX` 判断改为 `None` 匹配
- 同理 `cursor_visual_line_in_doc`
- **验证：** `cargo test -p edit-plus-app --lib` 全绿

### 0.2 advance_cache display_row 语义统一
- 问题 #5
- 去掉 `AdvanceCacheEntry::display_row` 字段（当前仅 hit_test 返回，无人消费）
- hit_test 改为返回 `Option<(doc_line, byte_offset)>` 或仅返回 byte_offset
- advance_cache 保持纯粹的"相对屏幕行索引 → cluster 映射"
- **验证：** `cargo test -p edit-plus-app --lib` + 手动测试鼠标点击

### 0.3 修复 scroll_to_doc_line_wrap 假设等高
- 问题 #9
- 在 `scroll_to_doc_line_wrap` 中用 `WrapIndex::visual_line_count(line)` 取真实 count，不再假设 1
- 增加单元测试：多行不同 wrap 数的跳转

### 0.4 修复 set_text 全量重建 WrapIndex
- 问题 #8
- `set_text` 目前 `WrapIndex::new(line_count)` 全部初始化为 1
- 改为保留旧 WrapIndex 的 capacity，仅 `resize` + 对变更行 `shift_lines`
- 如果是全新加载（旧 len == 0），走 `new` 路径
- **验证：** 大文件加载后首次滚动不跳动

---

## Phase 1：WrapIndex 成为唯一事实来源

**目标：** 解决问题 #1（最关键）和 #6（resize 时 wrap width 变化）。
**文件：** `wrap_index.rs`, `viewport.rs`, `app.rs`
**预估：** 2 个会话

### 1.1 为 WrapIndex 增加状态追踪

**接口设计：**
```rust
pub enum LineState {
    Exact,      // wrap count 已按当前 viewport_width 精确计算
    Estimated,  // wrap count = 1 或旧值，未针对当前宽度验证
}

pub struct WrapIndex {
    // ... 现有字段 ...
    generation: u64,        // 每次 resize 递增
    dirty: BitVec,          // len 位，true = 需要重新 wrap
    viewport_width: f32,    // 当前 wrap 参考宽度
}
```

**新增方法：**
- `mark_all_dirty()` — resize 时调用，O(1)（`dirty.fill(true)`）
- `mark_exact(doc_line)` — shape 完成后标记
- `is_exact(doc_line) -> bool`
- `generation() -> u64`
- `set_viewport_width(width: f32)` — 宽度变化时触发 `mark_all_dirty`

### 1.2 Viewport 联动 generation

```rust
// viewport.rs
pub fn resize(&mut self, ...) {
    // ... 现有逻辑 ...
    // 新增：通知 wrap_index viewport width 变化
    // （wrap_index 通过 App 传入，这里只记录 pending resize）
    self.pending_resize = true;
}
```

`App::shape_visible_lines` 开头检查 `pending_resize`，若为 true 则调 `wrap_index.mark_all_dirty()` + 重置 `cached_visible_range`。

### 1.3 total_visual_lines 精确化

当前实现（app.rs:1259-1264）用 `visible_total + remaining` 估算。改为：

```rust
fn estimate_total_display_rows(&self) -> usize {
    let exact_rows = self.wrap_index.total_display_rows();
    let exact_lines = self.wrap_index.exact_count(); // 已标记 Exact 的行数
    if exact_lines == self.wrap_index.len() {
        exact_rows  // 全部精确，直接返回
    } else {
        // 对未精确行，假设 count=1（最保守下界）
        let remaining = self.wrap_index.len() - exact_lines;
        exact_rows + remaining
    }
}
```

**验证：**
- 18000 行文件：加载 → 向下滚到底 → scrollbar 不飘
- resize 窗口后滚到中间位置：scrollbar 准确
- `cargo test -p edit-plus-app --lib`

---

## Phase 2：消除 Viewport 双重派生缓存

**目标：** 解决问题 #4。Viewport 只保留 `scroll_top: DisplayRow`，其他状态全部实时计算。
**文件：** `viewport.rs`, `app.rs`
**预估：** 1-2 个会话

### 2.1 移除 `first_visible_doc_line` 字段

当前用途：
- `shape_visible_lines` 每帧覆盖写（app.rs:924-925）
- `move_cursor_visual` 读取（app.rs:513）
- `visible_doc_line_range` fallback 读取

替换方案：
```rust
// viewport.rs — 新增实时计算方法
pub fn first_visible_doc_line(&self, wrap_index: &WrapIndex) -> usize {
    wrap_index.display_to_doc(self.scroll_top.floor() as usize)
}
```

所有读取点改为调用此方法，传入 `&self.wrap_index`。

### 2.2 移除 `cached_visible_range` 字段

当前用途：
- `shape_visible_lines` 写入
- `visible_doc_line_range` 读取

替换方案：`shape_visible_lines` 返回 `(start_doc_line, end_doc_line)`，由调用方持有。如果其他地方需要，也走实时计算。

### 2.3 简化 `sync_doc_line_from_scroll_top`

删除此方法。之前它用近似值填充 `first_visible_doc_line`，现在不需要了。

**验证：**
- 所有之前读 `first_visible_doc_line` / `cached_visible_range` 的路径都工作正常
- `cargo test -p edit-plus-app --lib`
- 手动测试：快速滚动、点击、resize

---

## Phase 3：拆分 shape_visible_lines

**目标：** 解决问题 #2（390 行 6 职责）和 #3（循环中改索引）。
**文件：** `app.rs`
**预估：** 2 个会话

### 3.1 函数拆分方案

将 `shape_visible_lines`（app.rs:906-1299）拆为：

```rust
fn shape_visible_lines(&mut self) -> Vec<GlyphVertex> {
    let (range, skip) = self.compute_visible_range();
    let shaped = self.shape_lines(range, skip);
    self.post_shape_update(range);
    shaped.vertices
}

/// 1. 计算可见行范围 + skip 逻辑
fn compute_visible_range(&mut self) -> (Range<usize>, usize) { ... }

/// 2. 纯渲染：shape 每一行，返回 vertices + 临时数据
fn shape_lines(&mut self, range: Range<usize>, skip: usize) -> ShapeResult { ... }

/// 3. 后处理：更新 WrapIndex、autoscroll、缓存
fn post_shape_update(&mut self, range: Range<usize>) { ... }
```

**ShapeResult 临时结构：**
```rust
struct ShapeResult {
    vertices: Vec<GlyphVertex>,
    advance_cache: Vec<AdvanceCacheEntry>,
    doc_line_map: BTreeMap<usize, (usize, usize)>,
    cursor_visual_line: Option<usize>,
    cursor_visual_line_in_doc: Option<usize>,
    char_width: f32,
}
```

### 3.2 消除循环中索引变异（问题 #3）

当前问题：循环内 `wrap_index.update()` 改变状态 → 循环后续迭代读到不一致的 `range`。

修复方案：
- 循环开始前 snapshot `range_pre`（已有）
- 循环内收集 `(doc_line, new_count)` 但**不立即 update**
- 循环结束后批量 `wrap_index.update_batch(updates)`
- 循环内的 `doc_line_idx` 只依赖 `range_pre`，不依赖被修改的 wrap_index

### 3.3 简化 move_cursor_visual 的 5 份缓存（问题 #12）

`first_line_visual_lines`、`first_line_clusters`、`last_line_visual_lines`、`last_line_clusters`、`first_line_doc_offset` 这 5 个字段仅用于 `move_cursor_visual` 的 4b/4c 分支（跨视口光标移动时需要 sticky_x）。

替代方案：
- 删除这 5 个字段
- 4b/4c 需要 sticky_x 时，实时调用 `shape_single_line(doc_line)` 获取 cluster 数据
- `shape_single_line` 从 `shape_lines` 中提取的子函数

**验证：**
- `cargo test -p edit-plus-app --lib`
- 手动测试：上下箭头跨视口边界、超长行换行后光标移动

---

## Phase 4：核心性能优化

**目标：** 解决 P1、P2、P3、P4、P9。
**文件：** `app.rs`, `document_view.rs`, `wrap_index.rs`
**预估：** 2-3 个会话

### 4.1 per-doc-line wrap 缓存（P1）

**接口设计：**
```rust
// wrap_cache.rs（新文件）或内嵌 WrapIndex
pub struct WrapCache {
    /// key = (line_content_hash, viewport_width), value = Vec<(start, end, pixel_width)>
    cache: LruCache<(u64, u32), Vec<(usize, usize, f32)>>,
}
```

- `char_width` 在等宽字体下为常量，全局缓存一次（P1 第二部分）
- shape 时先查缓存，命中直接用，未命中计算后写入
- 编辑导致行内容变化时，通过 `line_content_hash` 自然失效

### 4.2 visible_line 返回 Cow（P2）

```rust
// document_view.rs
pub fn visible_line(&self, line: usize) -> Cow<'_, [u8]> {
    // 单 chunk 时返回引用（零拷贝）
    // 跨 chunk 时才拷贝
}
```

- 删除 `visible_lines` 批量方法（当前只是 for 循环 + from_utf8_lossy）
- shape 阶段一次性算出 `is_whitespace` bitmap，下游直接读

### 4.3 advance_cache 对象池 + doc_line_map 删除（P3）

- advance_cache：`clear()` 保留 capacity，内部 `clusters: Vec` 改为 slab 复用
- doc_line_map：当前仅在 `move_cursor_visual 4c`（app.rs:579）被读 → 改为线性扫 advance_cache 找匹配 doc_line → 删除 BTreeMap

### 4.4 shape_cache key 用 content hash（P4）

```rust
// 当前 key（碰撞风险）：
let cache_key = (offset as u64) << 32 | (length as u64);

// 改为：
let cache_key = xxhash(line_content);  // 与 offset 解耦，跨 buffer 不碰撞
```

- 编辑后只有实际变化的行 miss → 视口内未变化行继续命中
- 增加 `doc_id: usize` 到 key，防止跨文档碰撞

### 4.5 autoscroll 前移到 shape 之前（P9）

```rust
fn render_frame(&mut self) {
    // 1. pre-layout autoscroll（当前在 shape 末尾）
    self.autoscroll_if_needed();
    // 2. shape
    let vertices = self.shape_visible_lines();
    // 3. gpu submit
    ...
}
```

这样按方向键时本帧即渲染正确位置，消除 16ms 延迟。

**验证：**
- 18000 行文件连续向下滚动：CPU 占用下降（可用 `cargo bench` 或 `Instruments`）
- 大文件编辑（中间插入一行）：视口不闪
- `cargo test -p edit-plus-app --lib`

---

## Phase 5：剩余性能优化

**目标：** 解决 P5、P6、P7、P8。
**文件：** `document_view.rs`, `app.rs`, `wrap_index.rs`
**预估：** 1-2 个会话

### 5.1 rebuild_line_index 增量化（P6）

当前 `rebuild_line_index_from_tb` 每次 multi-line edit / undo / redo 全量扫描。

方案：
- selection delete 路径改用 `rescan_lines_from(start_line)` — 只重扫从删除起点到文件末尾
- 提供 `TextBuffer::line_breaks_in_range(start, end) -> usize` 供增量计算
- 至少 `select_delete` 不走 full rebuild

### 5.2 cursor_line 缓存（P7）

```rust
// app.rs
struct CachedCursorLine {
    offset: usize,      // cursor_offset 时的快照
    doc_line: usize,    // 对应的 doc_line
}
```

- `cursor_line()` 先比对 offset，命中返回缓存值
- `cursor_offset` 改变时自然失效

### 5.3 extract_selected_text 零拷贝（P5）

- 增加 `count_selection_chars(start, end) -> usize` 方法
- 走 chunked 扫描，不构造 `Vec<u8>`
- `selection_counts_cache` miss 时调用此方法

### 5.4 WrapIndex 大文件内存优化（P8）

- 18000 行 → 512KB，1M 行 → 16MB
- 对超大文件（>100K 行）考虑 chunked segment tree
- 或 sparse 模式：只存储已 Exact 标记的行

**验证：**
- 1M 行文件加载不 OOM
- 多次 undo/redo 大选区不卡顿
- `cargo test -p edit-plus-app --lib`

---

## 接口变更汇总

| 阶段 | 接口变更 | 影响范围 |
|------|---------|---------|
| Phase 0 | `cursor_visual_line: usize → Option<usize>` | app.rs 内部 |
| Phase 0 | 删除 `AdvanceCacheEntry::display_row` | app.rs 内部 |
| Phase 1 | `WrapIndex` 新增 `mark_all_dirty / is_exact / generation` | wrap_index.rs + viewport.rs + app.rs |
| Phase 2 | 删除 `Viewport::first_visible_doc_line` 字段 | viewport.rs + app.rs |
| Phase 2 | 删除 `Viewport::cached_visible_range` 字段 | viewport.rs + app.rs |
| Phase 3 | `shape_visible_lines` 拆为 3 个函数 | app.rs 内部 |
| Phase 3 | 删除 5 个 first/last line 缓存字段 | app.rs 内部 |
| Phase 4 | 新增 `WrapCache` / 修改 `visible_line` 返回类型 | 新文件 + document_view.rs |
| Phase 5 | 新增 `count_selection_chars` | document_view.rs |

---

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| Phase 1 改 WrapIndex 影响面大 | 先写单元测试覆盖新接口，再改调用方 |
| Phase 3 拆分函数容易引入回归 | 拆分前后输出 vertices 做 diff 对比 |
| Phase 4 wrap 缓存 key 设计不当导致渲染错误 | 先在 debug 模式加 assert 校验缓存一致性 |
| 多阶段连续改动累积 regression | 每阶段结束跑全量测试 + 手动测试 |

---

## 进度追踪

| 阶段 | 状态 | 备注 |
|------|------|------|
| Phase 0 | ✅ 完成 | 374 tests passed |
| Phase 1 | ✅ 完成 | 380 tests passed |
| Phase 2 | ✅ 完成 | 375 tests passed |
| Phase 3 | ✅ 完成 | 322 lines (was 392) |
| Phase 4 | ✅ 完成 | P1 wrap_cache, P2 Cow, P3 cluster_pool+doc_line_map删除, P4 content hash key, P7 cursor_line_cached, P9 pre_shape_autoscroll |
| Phase 5 | ✅ 完成 | P5 count_selection_chars零拷贝, P6 delete_selection+undo/redo增量, P8 memory_usage监控 |

**最终测试：393 passed, 0 failed**
