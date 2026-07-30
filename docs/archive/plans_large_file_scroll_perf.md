# 大文件滚动性能优化 — 完整方案

> 基于 Zed 编辑器 DisplayMap 架构的完整参考实现
> 制定日期：2026-06-03
> 输入：4.1MB / 18151 行 `段落标题.json` 滚动卡顿
> 约束：每阶段独立可编译、可测试；先设计接口再实现

---

## 一、现状诊断

### 1.1 当前架构（每帧）

```
App::render()
  └─ render_pipeline::shape_visible_lines()
       ├─ for each visible doc line (40~80 行):
       │   ├─ WrapIndex::display_to_doc(i)           O(log n) ✓
       │   ├─ text_buf.get_line_content()             I/O     ✓
       │   ├─ shape_cache.lookup(key)                 LRU     ✓
       │   ├─ wrap_cache.lookup(key)                  LRU     ✓
       │   ├─ line_number shaping + atlas write       每帧 ✗
       │   ├─ build_advance_cache_entries()           每帧 ✗
       │   └─ generate_glyph_vertices() + atlas write 每帧 ✗ ← 瓶颈
       └─ 返回 Vec<GlyphVertex>                       全新 Vec ✗
```

**核心问题：** shape/wrap 有 LRU 缓存命中。但 **顶点生成完全没有缓存**——每帧 40~80 行全部重建。滚动时内容完全不变，这些全是重复计算。

### 1.2 瓶颈量化

| 操作 | 每帧开销 | 可否缓存 |
|------|---------|---------|
| 行号格式化 + shaping + atlas 写入 | ~40 次 | ✓ |
| 文本行 glyph 顶点生成 + atlas 写入 | ~40 次 | ✓ |
| advance_cache 构建 | ~40 条 | ✓ |
| 全新 Vec 分配 | 1 次 | ✓ 可复用 |

**结论：** 纯滚动场景帧时间应接近 0（仅调整 y 偏移），当前却是全量重建。

---

## 二、Zed 架构参考

### 2.1 分层 DisplayMap

```
MultiBuffer (原始文本)
  ↓ InlayMap → FoldMap → TabMap
  ↓ WrapMap    (soft wrap)        ← 核心参考层
  ↓ BlockMap → DisplayMap (高亮)
```

每层核心模式：**Snapshot（不可变快照）+ sync(edits) → (new_snapshot, patch)**

### 2.2 WrapMap 关键设计

```rust
WrapSnapshot {
    tab_snapshot: TabSnapshot,       // 底层文本
    transforms: SumTree<Transform>,  // 每个 Transform = 一个行的映射
}

Transform {
    summary: TransformSummary { input: TextSummary, output: TextSummary },
    // Isomorphic → input == output (1:1 映射)
    // Wrap       → input: 1 行, output: N 行（换行展开）
}
```

SumTree 特性：
- **持久化快照**：`Arc<Node>` 实现 O(1) clone，旧快照可继续读取
- **增量 patch**：sync 返回 `WrapPatch`（编辑范围），上层只重绘受影响区域
- **O(log n) seek**：`Cursor::seek()` 按 DisplayRow 定位
- **后台计算**：大文件 wrapping 通过 `background_spawn()` 异步，每 100 行 yield

### 2.3 ScrollAnchor

```rust
struct ScrollAnchor {
    offset: Point<f64>,   // 像素偏移
    anchor: Anchor,       // buffer 中的锚点
}
// 编辑时锚点不变 → 滚动位置自动跟随内容，不抖动
```

### 2.4 Zed 渲染 vs edit+

Zed 通过 gpui text_system 直接 layout 文本行，gpui 框架自动缓存。edit+ 自研 wgpu 渲染栈，**必须自己管理顶点缓存**。

---

## 三、整体方案

### 3.1 核心思路：对齐 Zed 的 DisplayMap + Snapshot 模型

```
当前：
  TextBuffer → WrapIndex(段树) → shape_visible_lines() → Vec<GlyphVertex>
               ↑ 每帧 O(log n) 查找
               只存 count，不存渲染数据

目标：
  TextBuffer → DisplayLineMap(SnapTree) → DisplaySnapshot → Vertices
               ↑ snapshot() O(1) 生成不可变快照
               ↑ sync(edits) 增量更新，返回 patch
               ↑ 每个 entry 存储 visual_lines + 渲染顶点数据
```

### 3.2 关键设计决策：不移植完整 SumTree

Zed 的 `sum_tree` crate 约 2800 行，支持泛型 Dimension、rayon 并行等。edit+ 仅需一个维度 (DisplayRow)，自己实现精简版 **SnapTree** 约 400 行。

```
Zed SumTree (2800 行)          SnapTree (400 行)
├─ 泛型 Dimension trait        ├─ 仅 DisplayRow
├─ Cursor + seek + bias        ├─ find_by_row() + iter_range()
├─ rayon 并行构建              ├─ 串行 (20000 行 < 10ms)
├─ FilterCursor / KeyedItem    └─ 不需要
└─ TreeMap / TreeSet
```

### 3.3 实施阶段

```
Phase 1  SnapTree 数据结构            ← 核心基础设施 (~400 行)
Phase 2  DisplayLineMap 核心           ← DisplayLineEntry + Snapshot + Map + Patch (~800 行)
Phase 3  渲染管线集成                  ← 替换 shape_visible_lines 迭代 (~200 行改动)
Phase 4  后台 wrapping                ← 大编辑不阻塞主线程 (~100 行)
Phase 5  ScrollAnchor                ← 锚点滚动 (~150 行)
Phase 6  清理 + 测试                  ← 删除 WrapIndex + 验证 (~100 行)
```

> **说明**：Phase 1-4 完成后，`DisplayLineEntry` 自带 `render_data` 字段，
> 天然包含原方案 Phase 1（顶点缓存）和 Phase 2（RenderCache）的收益。
> 因此不再需要独立的 Phase 1+2。

---

## 四、Phase 1：SnapTree（~400 行）

**文件**：`crates/app/src/snap_tree.rs`

```rust
//! 精简版 sum tree，专用于 DisplayLineMap。
//! B-tree 结构，Arc 包装实现 O(1) clone（快照）。

const TREE_BASE: usize = 16;  // 叶子容量 = 32 entries

enum Node {
    Leaf(LeafNode),
    Internal(InternalNode),
}

struct LeafNode {
    total_rows: usize,
    total_lines: usize,
    entries: Vec<DisplayLineEntry>,
    row_prefix: Vec<usize>,  // entries[..i] 的 total_rows 之和
}

struct InternalNode {
    total_rows: usize,
    total_lines: usize,
    children: Vec<Arc<Node>>,
    child_row_prefix: Vec<usize>,  // children[..i] 的 total_rows 之和
}

pub struct SnapTree {
    root: Arc<Node>,
    len: usize,
}
```

**核心 API**：

| 方法 | 复杂度 | 说明 |
|------|--------|------|
| `new()` | O(1) | 空树 |
| `from_entries(iter)` | O(n) | 批量构建 |
| `clone()` | O(1) | Arc 浅克隆 |
| `len()` / `total_rows()` | O(1) | |
| `find_by_row(row) → Option<(doc_line, &Entry)>` | O(log n) | DisplayRow → entry |
| `find_by_line(line) → Option<(display_start, &Entry)>` | O(log n) | doc_line → DisplayRow |
| `push(entry)` | 摊销 O(log n) | |
| `extend(iter)` | O(n) | |
| `splice(range, replacements) → SpliceResult` | O(k log n) | 增量更新 |
| `iter() / iter_range(range)` | — | 只读迭代 |

**与 Zed SumTree 的对应**：

| Zed | SnapTree | 备注 |
|-----|----------|------|
| `Cursor::seek(target, bias)` | `find_by_row(row)` | 不需要 Bias |
| `cursor.slice(end)` | `iter_range(0..line)` 收集 | |
| `cursor.suffix()` | `iter_range(line..len)` 收集 | |
| `push(item, cx)` | `push(entry)` | 无 context |
| `Dimension` trait | 不需要 | 仅一个维度 |

**自平衡策略**：叶子满时（> `2 * TREE_BASE` entries）分裂为两个，向上递归传播。与标准 B-tree 一致。

**单元测试**：
```rust
#[test] fn push_and_find();
#[test] fn splice_single();
#[test] fn splice_range();
#[test] fn clone_shallow();
#[test] fn large_build_20000_entries();
#[test] fn split_propagates_up();
```

---

## 五、Phase 2：DisplayLineMap 核心（~800 行）

**文件**：`crates/app/src/display_line_map.rs`

### 5.1 DisplayLineEntry

```rust
/// 一个 doc line 的完整显示映射。
#[derive(Clone)]
pub struct DisplayLineEntry {
    /// visual line 数量（≥1）
    pub visual_line_count: usize,
    /// 每个 visual line 的详细信息
    pub visual_lines: Vec<VisualLineInfo>,
    /// 文本 xxhash（快速脏检测）
    pub content_hash: u64,
    /// buffer 中的字节范围
    pub byte_offset: usize,
    pub byte_length: usize,
    /// 脏标记（编辑后=true，shape 后=false）
    pub dirty: bool,
    /// 预计算的渲染顶点（None=未 shape，Some=已缓存）
    pub render_data: Option<CachedLineRender>,
}

#[derive(Clone)]
pub struct VisualLineInfo {
    pub cluster_start: usize,
    pub cluster_end: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub pixel_width: f32,
}

#[derive(Clone)]
pub struct CachedLineRender {
    pub glyph_vertices: Vec<GlyphVertex>,
    pub line_number_vertices: Vec<GlyphVertex>,
    pub advance_entries: Vec<AdvanceCacheEntry>,
}
```

### 5.2 DisplaySnapshot

```rust
/// 不可变快照，渲染层零锁读取。
#[derive(Clone)]
pub struct DisplaySnapshot {
    tree: SnapTree,
    pub generation: u64,
    pub viewport_width: f32,
    pub font_size: f32,
}

impl DisplaySnapshot {
    /// DisplayRow → (doc_line, visual_line_idx, byte_range, &entry)
    pub fn resolve_row(&self, row: usize) -> Option<RowRef<'_>>;

    /// doc_line → 起始 DisplayRow
    pub fn doc_to_display(&self, doc_line: usize) -> usize;

    /// DisplayRow 范围迭代
    pub fn iter_rows(&self, range: Range<usize>) -> DisplayRowIter<'_>;

    pub fn total_rows(&self) -> usize;
    pub fn len(&self) -> usize;  // doc line count
}
```

### 5.3 DisplayLineMap

```rust
/// 替代 WrapIndex，维护 buffer → display 的完整映射。
pub struct DisplayLineMap {
    tree: SnapTree,
    generation: u64,
    viewport_width: f32,
    font_size: f32,
    pending_task: Option<JoinHandle<SnapTree>>,
    pending_patch: DisplayPatch,
}

impl DisplayLineMap {
    /// 初始加载：全量 shape + wrap → 构建 SnapTree
    pub fn from_buffer(
        buffer: &TextBuffer,
        viewport_width: f32,
        font_size: f32,
    ) -> Self;

    /// O(1) 生成快照
    pub fn snapshot(&self) -> DisplaySnapshot;

    /// 增量同步 buffer 编辑
    /// → 小编辑（≤100 行）：同步完成
    /// → 大编辑：启动后台任务，返回插值快照
    pub fn sync(
        &mut self,
        buffer: &TextBuffer,
        edits: &[(Range<usize>, &str)],
    ) -> (DisplaySnapshot, DisplayPatch);

    /// viewport 尺寸变化 → 全部 re-wrap
    pub fn set_viewport_size(&mut self, width: f32, font_size: f32);

    /// 单行精确更新（编辑后）
    pub fn update_line(&mut self, doc_line: usize, content: &[u8]);

    /// 批次更新
    pub fn update_lines<I>(&mut self, updates: I)
    where I: IntoIterator<Item = (usize, &[u8])>;

    /// 尝试获取后台 re-wrap 结果（非阻塞）
    pub fn poll_background(&mut self) -> Option<DisplayPatch>;
}
```

### 5.4 DisplayPatch

```rust
/// sync 返回的影响范围。
#[derive(Clone, Default)]
pub struct DisplayPatch {
    pub affected_rows: Option<Range<usize>>,
    pub full_invalidate: bool,
}

impl DisplayPatch {
    pub fn none() -> Self;
    pub fn range(r: Range<usize>) -> Self;
    pub fn full() -> Self;
    pub fn union(&mut self, other: &DisplayPatch);
}
```

### 5.5 SnapTree::splice — 增量更新的核心

```rust
impl SnapTree {
    /// 替换 doc_line range 中的 entries。
    /// 返回 (old_display_rows, new_display_rows) 用于生成 patch。
    pub fn splice(
        &mut self,
        line_range: Range<usize>,
        replacements: Vec<DisplayLineEntry>,
    ) -> SpliceResult;
}

pub struct SpliceResult {
    pub old_display_rows: Range<usize>,
    pub new_display_rows: Range<usize>,
}
```

---

## 六、Phase 3：渲染管线集成（~200 行改动）

**文件**：`crates/app/src/render_pipeline.rs`、`app.rs`

### 6.1 数据流对比

**Before（每帧全量）**：
```
shape_visible_lines()
  → for i in 0..vis_count:
      doc_line = wrap_index.display_to_doc(i)    O(log n)
      content = get_line(doc_line)                 buffer read
      shaped = shape_cache.get_or_shape()          LRU
      vl = wrap_cache.get_or_wrap()                LRU
      vertices += generate_glyph_vertices()        顶点生成 ← 瓶颈
    → return all_vertices
```

**After（基于快照）**：
```
shape_visible_lines()
  → snapshot = display_map.snapshot()              O(1) Arc clone
  → for row_ref in snapshot.iter_rows(visible):
      entry = row_ref.entry
      if let Some(cached) = &entry.render_data:
        offset_and_append(cached)                  零计算！
      else if entry.dirty:
        data = shape_entry(entry)                  仅新/脏行
        entry.render_data = Some(data)
    → append selection_vertices()                   每帧（轻量）
    → append cursor_vertices()                     每帧（轻量）
```

### 6.2 App 集成

```rust
// app.rs TextState 中
struct TextState {
    // 移除: shape_cache, wrap_cache  (被 DisplayLineEntry 吸收)
    // 移除: 部分 advance_cache 逻辑  (进入 CachedLineRender)
    // 新增:
    display_map: DisplayLineMap,
    current_snapshot: DisplaySnapshot,
    pending_patch: DisplayPatch,
}

// 每帧渲染前
fn update(&mut self) {
    // 1. 检查后台 wrapping 结果
    if let Some(patch) = self.display_map.poll_background() {
        self.pending_patch.union(&patch);
    }
    // 2. 如果 viewport 尺寸变化
    if self.viewport_width_changed() {
        self.display_map.set_viewport_size(new_width, font_size);
    }
}

// 编辑后
fn on_edit(&mut self, edits: &[(Range<usize>, &str)]) {
    let (snapshot, patch) = self.display_map.sync(&self.buffer, edits);
    self.current_snapshot = snapshot;
    self.pending_patch.union(&patch);
}
```

### 6.3 行号缓存

行号 shaping 现在是 `DisplayLineEntry.render_data.line_number_vertices` 的一部分——每个 doc line 的 `visual_line_idx == 0` 那行自带行号顶点。不需独立缓存。

---

## 七、Phase 4：后台 Wrapping（~100 行）

```rust
impl DisplayLineMap {
    fn start_background_rewrap(&mut self, line_range: Range<usize>, buffer: Arc<TextBuffer>) {
        let width = self.viewport_width;
        let font_size = self.font_size;
        let old_tree = self.tree.clone();

        self.pending_task = Some(std::thread::spawn(move || {
            let mut shaper = Shaper::new().unwrap().with_font_size(font_size);
            let mut new_tree = old_tree;  // clone for mutation

            for doc_line in line_range {
                let content = buffer.get_line_content(doc_line);
                let entry = shape_and_wrap_entry(&mut shaper, content, width, doc_line);
                // splice 单行
                new_tree.splice(doc_line..doc_line + 1, vec![entry]);
            }
            new_tree
        }));
    }

    pub fn poll_background(&mut self) -> Option<DisplayPatch> {
        if let Some(task) = &self.pending_task {
            if task.is_finished() {
                let new_tree = self.pending_task.take().unwrap().join().unwrap();
                let old_rows = self.tree.total_rows();
                self.tree = new_tree;
                self.generation += 1;
                let new_rows = self.tree.total_rows();
                return Some(DisplayPatch::range(0..new_rows.max(old_rows)));
            }
        }
        None
    }
}
```

与 Zed 的关键差异：Zed 用 `cx.background_spawn()`（gpui 异步运行时），edit+ 用 `std::thread::spawn`。因为 edit+ 用 winit 事件循环，线程是可行的并行方式。shaper 需要在后台线程创建独立实例（不能跨线程共享）。

---

## 八、Phase 5：ScrollAnchor（~150 行）

**文件**：修改 `crates/app/src/viewport.rs`

### 8.1 数据结构

```rust
/// 滚动锚点：绑定到 buffer 内容而非像素。
pub struct ScrollAnchor {
    /// 锚定的 doc_line
    pub doc_line: usize,
    /// 该行顶部相对于视口顶部的像素偏移
    pub pixel_offset: f64,
}

pub struct Viewport {
    pub scroll_anchor: ScrollAnchor,
    pub visible_rows: usize,
}
```

### 8.2 转换公式

```rust
// ScrollAnchor → scroll_top (像素行号)
fn to_scroll_top(anchor: &ScrollAnchor, snapshot: &DisplaySnapshot) -> f64 {
    let display_row = snapshot.doc_to_display(anchor.doc_line) as f64;
    display_row + anchor.pixel_offset / line_height
}

// 当前视口顶部 → ScrollAnchor
fn from_viewport_top(
    scroll_top: f64,
    line_height: f32,
    snapshot: &DisplaySnapshot,
) -> ScrollAnchor {
    let display_row = scroll_top.floor() as usize;
    let row_ref = snapshot.resolve_row(display_row).unwrap();
    ScrollAnchor {
        doc_line: row_ref.doc_line,
        pixel_offset: (scroll_top - display_row as f64) * line_height as f64,
    }
}
```

### 8.3 影响范围

- `viewport.rs`：`scroll_top: f64` → `scroll_anchor: ScrollAnchor`
- `mouse.rs`：滚轮 delta 转为 `pixel_offset` 调整 + 必要时跨行
- `commands.rs`：跳转命令构造 `ScrollAnchor { doc_line: target, pixel_offset: 0.0 }`
- 所有读写 `scroll_top` 的调用点

---

## 九、Phase 6：清理 + 测试

### 9.1 删除项

| 文件 | 操作 |
|------|------|
| `crates/app/src/wrap_index.rs` | 删除（~900 行） |
| `app.rs` 中的 `shape_cache` / `wrap_cache` 字段 | 删除（被 DisplayLineEntry 吸收） |

### 9.2 测试计划

```rust
// snap_tree
#[test] fn push_and_find();
#[test] fn splice_single();
#[test] fn splice_range();
#[test] fn clone_is_shallow();
#[test] fn large_build_20000_entries();

// display_line_map
#[test] fn from_buffer_builds_all_entries();
#[test] fn sync_single_edit_marks_line_dirty();
#[test] fn sync_range_edit_produces_correct_patch();
#[test] fn resolve_row_returns_correct_visual_line();
#[test] fn resize_marks_all_dirty();
#[test] fn render_data_cached_after_first_shape();

// scroll_anchor
#[test] fn anchor_unchanged_after_edit_above();
#[test] fn to_scroll_top_accounts_for_wrapped_lines();

// 集成
#[test] fn scroll_reuses_cached_vertices();
#[test] fn edit_only_reshapes_affected_lines();

// benchmark (已有 benches/scroll_bench.rs)
#[bench] fn scroll_large_file_with_display_map();
```

---

## 十、文件改动总览

| 阶段 | 文件 | 操作 | 行数 |
|------|------|------|------|
| 1 | `crates/app/src/snap_tree.rs` | **新建** | +400 |
| 2 | `crates/app/src/display_line_map.rs` | **新建** | +800 |
| 3 | `crates/app/src/render_pipeline.rs` | 改造 shape_visible_lines | ~200 |
| 3 | `crates/app/src/app.rs` | 集成 DisplayLineMap | ~80 |
| 4 | `crates/app/src/display_line_map.rs` | 添加后台 wrapping | +100 |
| 5 | `crates/app/src/viewport.rs` | scroll_top → ScrollAnchor | ~100 |
| 5 | `crates/app/src/mouse.rs` | 适配 ScrollAnchor | ~30 |
| 5 | `crates/app/src/commands.rs` | 适配 ScrollAnchor | ~20 |
| 6 | `crates/app/src/wrap_index.rs` | **删除** | -900 |
| 6 | `crates/app/src/app.rs` | 移除 shape_cache/wrap_cache | -30 |
| - | `crates/app/src/lib.rs` | 模块注册 | +5 |

**总计**：新增 ~1700 行，删除 ~930 行，净增 ~770 行。

---

## 十一、风险与注意事项

1. **atlas 生命周期**：`CachedLineRender` 的 GlyphVertex 含 atlas UV 坐标。atlas 重建时（纹理满），所有 `render_data` 必须失效。在 atlas 重建点设置 `full_invalidate` 标志。

2. **主题变化**：顶点颜色硬编码，主题变化 → `full_invalidate`，下次渲染时全部重新 shape。

3. **后台线程安全**：`DisplaySnapshot` 是纯只读数据（Arc 共享），无锁。shaper 在后台线程需独立实例。

4. **Snapshot 一致性**：编辑后立即 `snapshot()` 返回的仍是旧数据（因为 sync 是异步的）。app.rs 需在 `poll_background()` 返回后才能更新 `current_snapshot`。

5. **内存**：20000 行 × ~2KB/行（含顶点数据）= ~40MB。可通过 LRU 淘汰离视口较远的行的 `render_data` 来控制。
