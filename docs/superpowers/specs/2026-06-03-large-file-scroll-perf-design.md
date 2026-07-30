# 大文件滚动性能 — 设计文档

> 制定日期：2026-06-03
> 输入痛点：4.1MB / 18151 行 `段落标题.json` 滚动条拖动卡顿
> 参考实现：`/Users/dan/proj/llmws/zed/crates/editor/src/display_map/wrap_map.rs`
> 范围：彻底对齐 Zed `DisplayMap` 的 Snapshot/Patch 模型，并在其上加一层 `RenderCache`

## 1. 目标与非目标

**目标**：

1. 4.1MB JSON 文件滚动一帧 < 2ms（M1，viewport 约 40 行）
2. 编辑后视口锚定行不漂（ScrollAnchor 内容锚定）
3. 主题切换 0 cache invalidate
4. 大编辑（粘贴 5000 行）不阻塞主线程
5. RenderCache 内存上限 ≤ viewport_visual_rows + 2 × OVERSCAN（约 1000 行 / ~3MB）

**非目标**：

1. 不引入 GPU instanced rendering（保持现有 GlyphRenderer 接口）
2. 不重写 shaper / atlas（只扩展 atlas insert 返回驱逐 keys）
3. 不在本方案处理 IME / undo-redo 行为差异

## 2. 现状与瓶颈定位

### 2.1 当前架构

```
TextBuffer
  └─ WrapIndex (segment tree)        ← 仅存 visual_line_count, O(log n) 双向映射
  └─ TextState.shape_cache (LRU)
  └─ TextState.wrap_cache  (LRU)
  └─ render_pipeline::shape_visible_lines()
       每帧对每个可见 doc 行：
         shape lookup → wrap lookup → 行号 format!+shape →
         build_advance_cache_entries → generate_glyph_vertices →
         返回全新 Vec<GlyphVertex>
```

### 2.2 关键观察

跳过 profile，按经验定位四点："顶点生成"是瓶颈的常规说法不准确，真正成本是：

1. 每帧每行 `format!("{}", doc_line+1)` + `Shaper::shape()` 行号
2. `visual_lines.clone()` 在 `render_pipeline.rs:266 / 284 / 293` 出现 3 次
3. `dv.highlights_for_line(doc_line_idx)` 每行收集 `Vec<(usize, HighlightKind)>`
4. `GlyphVertex.position` 是绝对屏幕坐标 + NDC 化后的值，颜色直接嵌入顶点 — 任何"成品顶点缓存"在滚动 1px / 主题切换 / 选区变化时都失效

`atlas.get(&key)` 已经命中 LRU 不再调 `rasterize_glyph` / `write_texture` —— 这两项不是热点，无需优化。

## 3. 整体架构

```
TextBuffer
  ▼
DisplayLineMap (SnapTree<DisplayLineEntry>)
  ├─ Snapshot { tree: Arc<SnapTree>, generation, viewport_width, font_size }   ← O(1) clone
  ├─ sync(edits) → (Snapshot, DisplayPatch)                                     ← Zed 模式
  └─ ReshapeWorker (单 worker 线程 + mpsc channel)
  ▼
RenderCache (LruCache<doc_line, CachedLine>, 容量 ≈ viewport_rows + 2*OVERSCAN)
  ├─ instances: Vec<GlyphInstance>           ← 行内相对 x，atlas_slot_id 间接引用
  ├─ line_number_glyphs                       ← 行号 GlyphInstance（来自 line_number_pool）
  ├─ atlas_generation                         ← atlas 驱逐失效
  └─ theme_independent: true                  ← color 渲染时查，主题切换 0 失效
  ▼
render_visible(snapshot, scroll_anchor, render_cache) → Vec<GlyphVertex>
  对每个可见行：
    cache hit  → for inst in instances: 加 y_offset / 查 highlight color / 推 6 顶点
    cache miss → 主线程兜底 shape 1 行（预算 2 行 / 帧），其余画占位 + 入队 worker

Viewport.scroll_top: f64
  ↓ 替换为
Viewport.scroll_anchor: ScrollAnchor { doc_line, pixel_offset }
```

**三个核心抽象**：

1. **DisplayLineMap + Snapshot/Patch** — 对齐 Zed `wrap_map`。`SnapTree` 精简版 sum tree，仅 `DisplayRow` 一个维度（~500 行 vs Zed 的 2800 行）。`Arc<Node>` 实现 O(1) snapshot clone，渲染层无锁读取。
2. **RenderCache（行级、行内相对坐标）** — `Vec<GlyphInstance>` 24B/字形，行内坐标。滚动时只加 `y_offset` 推顶点 ≈ 零计算。viewport ±500 行 LRU 淘汰，约 3MB。
3. **ScrollAnchor** — `{ doc_line, pixel_offset }` 替代 `scroll_top: f64`。编辑插入行后，锚定 doc_line 不变 → 视口内容不抖动。

## 4. 组件接口

### 4.1 `crates/app/src/snap_tree.rs`

```rust
//! 持久化 B-tree。叶子最大 32 项；Arc 包装实现 O(1) clone。

const TREE_BASE: usize = 16;

enum Node {
    Leaf  { entries: Vec<DisplayLineEntry>, total_rows: usize },
    Inner { children: Vec<Arc<Node>>,        total_rows: usize, child_count: usize },
}

pub struct SnapTree {
    root: Arc<Node>,
    line_count: usize,
}

impl SnapTree {
    pub fn new() -> Self;
    pub fn from_entries(it: impl IntoIterator<Item = DisplayLineEntry>) -> Self;
    pub fn line_count(&self) -> usize;
    pub fn total_rows(&self) -> usize;
    pub fn find_by_row(&self, row: usize) -> Option<RowLookup<'_>>;
    pub fn line_to_row(&self, doc_line: usize) -> usize;
    pub fn splice(&mut self, range: Range<usize>, replacements: Vec<DisplayLineEntry>) -> SpliceResult;
    pub fn iter_lines(&self, range: Range<usize>) -> LineIter<'_>;
    pub fn iter_rows(&self, rows: Range<usize>)   -> RowIter<'_>;
}

pub struct RowLookup<'a> {
    pub doc_line: usize,
    pub visual_idx_in_doc: usize,
    pub entry: &'a DisplayLineEntry,
}

pub struct SpliceResult {
    pub old_rows: Range<usize>,
    pub new_rows: Range<usize>,
}
```

clone = `Arc::clone(&root)`，O(1)。splice 走标准 B-tree 分裂/合并，复杂度 O(k log n)。

### 4.2 `crates/app/src/display_line_map.rs`

```rust
#[derive(Clone)]
pub struct DisplayLineEntry {
    pub visual_line_count: u16,
    pub visual_breaks: SmallVec<[VisualBreak; 1]>,   // 不换行行不堆分配
    pub byte_offset: usize,
    pub byte_length: u32,
    pub content_hash: u64,                            // xxhash 快速 dirty 检测
}

#[derive(Clone, Copy)]
pub struct VisualBreak {
    pub byte_start: u32,
    pub byte_end:   u32,
    pub pixel_width: f32,
}

#[derive(Clone)]
pub struct DisplaySnapshot {
    tree: SnapTree,
    pub generation: u64,
    pub viewport_width: f32,
    pub font_size: f32,
}

impl DisplaySnapshot {
    pub fn line_count(&self)  -> usize;
    pub fn total_rows(&self)  -> usize;
    pub fn resolve_row(&self, row: usize) -> Option<RowLookup<'_>>;
    pub fn line_to_row(&self, doc_line: usize) -> usize;
    pub fn iter_rows(&self, rows: Range<usize>) -> RowIter<'_>;
}

pub struct DisplayLineMap {
    tree: SnapTree,
    generation: u64,
    viewport_width: f32,
    font_size: f32,
    worker: ReshapeWorker,
    pending_render_inserts: Vec<(usize, CachedLine)>,
}

#[derive(Clone)]
pub struct DisplayPatch {
    pub affected_rows: Vec<Range<usize>>,
    pub line_shift: Option<LineShift>,
    pub generation: u64,
}

pub struct LineShift { pub at: usize, pub delta: i64 }

impl DisplayLineMap {
    pub fn from_buffer(buffer: &TextBuffer, viewport_width: f32, font_size: f32) -> Self;
    pub fn snapshot(&self) -> DisplaySnapshot;
    pub fn sync(&mut self, buffer: &TextBuffer, edits: &[Edit]) -> (DisplaySnapshot, DisplayPatch);
    pub fn set_viewport_size(&mut self, width: f32, font_size: f32) -> DisplayPatch;
    pub fn poll_worker(&mut self) -> Option<DisplayPatch>;
    pub fn drain_pending_render_inserts(&mut self) -> Vec<(usize, CachedLine)>;
}
```

`sync` 内部分两路：

- 小编辑（受影响行 ≤ 100）：同步 shape，splice 进 tree。
- 大编辑：插入占位 entry `{ vl_count: 1, byte_length, content_hash, dirty }`，将每行入 worker 队列。

### 4.3 `crates/app/src/reshape_worker.rs`

```rust
pub struct ReshapeRequest {
    pub generation: u64,
    pub doc_line: usize,
    pub line_bytes: Arc<[u8]>,
    pub viewport_width: f32,
    pub font_size: f32,
}

pub struct ReshapeResult {
    pub generation: u64,
    pub doc_line: usize,
    pub entry: DisplayLineEntry,
    pub cached_render: CachedLine,
}

pub struct ReshapeWorker {
    tx: mpsc::Sender<WorkerMsg>,
    rx: mpsc::Receiver<ReshapeResult>,
    current_generation: Arc<AtomicU64>,
    pending_count: Arc<AtomicUsize>,           // 背压用
}

enum WorkerMsg { Request(ReshapeRequest), Shutdown }

impl ReshapeWorker {
    pub fn spawn() -> Self;
    pub fn submit(&self, req: ReshapeRequest) -> SubmitOutcome;   // Accepted / Backpressured
    pub fn drain_completed(&self, max: usize) -> Vec<ReshapeResult>;
    pub fn cancel_before(&self, generation: u64);
    pub fn pending(&self) -> usize;
}

pub enum SubmitOutcome { Accepted, Backpressured }
```

worker 持有自己的 `Shaper`（不能跨线程共享）。每条消息 `if req.generation < current_gen.load() { drop }`。背压上限 1000 pending；超过 → 主线程改走兜底同步。

### 4.4 `crates/app/src/render_cache.rs`

```rust
const OVERSCAN: usize = 500;

#[derive(Clone)]
pub struct GlyphInstance {
    pub atlas_slot_id: u32,        // RenderCache 自管 slot 表
    pub x_local: f32,              // 行内 x（含 gutter offset）
    pub advance: f32,
    pub byte_start: u32,           // 用于查 highlight color
    pub vl_index: u8,              // 第几个 visual line
}

#[derive(Clone)]
pub struct CachedLine {
    pub instances: Vec<GlyphInstance>,
    pub vl_count: u8,
    pub line_number_glyphs: Vec<GlyphInstance>,
    pub atlas_generation: u64,
    pub theme_independent: bool,
}

pub struct RenderCache {
    cache: LruCache<usize, CachedLine>,
    atlas_generation: u64,
    line_number_pool: HashMap<u32, ShapedRun>,   // 行号 shape 池
    atlas_slot_table: Vec<AtlasSlotEntry>,       // slot_id → GlyphSlot
    reverse_index: HashMap<GlyphKey, SmallVec<[u32; 4]>>,    // GlyphKey → 占用的 slot_id 列表
}

impl RenderCache {
    pub fn new(capacity: usize) -> Self;
    pub fn get(&mut self, doc_line: usize) -> Option<&CachedLine>;
    pub fn insert(&mut self, doc_line: usize, line: CachedLine);
    pub fn invalidate_rows(&mut self, doc_lines: Range<usize>);
    pub fn invalidate_all(&mut self);
    pub fn shift(&mut self, at: usize, delta: i64);
    pub fn handle_atlas_eviction(&mut self, evicted: &[GlyphKey]);
    pub fn bump_atlas_generation(&mut self);
    pub fn shape_line_number(&mut self, n: u32, shaper: &mut Shaper) -> &ShapedRun;
}
```

### 4.5 `crates/app/src/viewport.rs`

```rust
#[derive(Clone, Copy)]
pub struct ScrollAnchor {
    pub doc_line: usize,
    pub pixel_offset: f32,    // 该 doc_line 顶部相对视口顶部
}

pub struct Viewport {
    pub scroll_anchor: ScrollAnchor,
    pub visible_rows: usize,
    // ... 其余字段保留
}

impl ScrollAnchor {
    pub fn to_scroll_top(&self, snapshot: &DisplaySnapshot, line_height: f32) -> f64;
    pub fn from_scroll_top(top: f64, snapshot: &DisplaySnapshot, line_height: f32) -> Self;
    pub fn adjust_after_edit(&mut self, patch: &DisplayPatch);
    pub fn refold_on_resize(&mut self, old_line_height: f32, new_line_height: f32);
    pub fn clamp(&mut self, snapshot: &DisplaySnapshot, viewport_rows: usize, line_height: f32);
}
```

### 4.6 主线程 / `crates/app/src/app.rs`

`TextState` 调整：

```rust
pub(crate) struct TextState {
    pub(crate) shaper: Shaper,
    pub(crate) atlas: GlyphAtlas,
    pub(crate) atlas_texture: wgpu::Texture,

    // 删除：shape_cache, wrap_cache

    // 新增：
    pub(crate) display_map: DisplayLineMap,
    pub(crate) current_snapshot: DisplaySnapshot,
    pub(crate) render_cache: RenderCache,
}
```

### 4.7 `crates/render/src/lib.rs` 扩展

```rust
impl GlyphAtlas {
    /// 旧签名保留，新增带驱逐返回的版本。
    pub fn insert_with_eviction(
        &mut self, key: GlyphKey, w: u32, h: u32, bx: f32, by: f32,
    ) -> InsertOutcome;
}

pub enum InsertOutcome {
    Allocated { slot: GlyphSlot, evicted: SmallVec<[GlyphKey; 4]> },
    Oversized,
}
```

## 5. 数据流时序

### 5.1 冷启动（4.1MB / 18151 行）

```
T=0      Buffer::load_from_file()
T=10ms   DisplayLineMap::from_buffer
         ├─ 18151 个占位 entry { vl_count: 1, byte_length, content_hash }
         ├─ SnapTree::from_entries 自底向上 ~5ms
         └─ generation = 1
T=20ms   snapshot 拷贝 (Arc clone)
T=20ms   worker.submit × (viewport_rows + OVERSCAN) 个请求
T=22ms   第一帧 render_visible：
         ├─ 40 行 cache miss → 主线程兜底 shape，每行 ~0.05ms ≈ 2ms 总计
         └─ insert RenderCache，推顶点
T=24ms   首屏可见
T=24ms~  worker 后台精修，每帧 poll 16 条 → splice 进 tree → invalidate 对应行
```

### 5.2 滚动

```
scroll delta = +120px
  viewport.scroll_anchor.pixel_offset += 120
  if pixel_offset >= line_height: anchor.doc_line += k; pixel_offset %= line_height

  for row in snapshot.iter_rows(visible):
    match render_cache.get(row.doc_line):
      Some(c) if c.atlas_generation == cache.atlas_gen():
        // 热路径：每个 GlyphInstance 推 6 顶点（加 y_offset / 查 color）
      Some(_) stale:                     reshape_inline
      None:
        if budget > 0 { reshape_inline; budget -= 1 }
        else { worker.submit; 画背景色占位 }

  // 预取
  prefetch(visible.end .. visible.end + OVERSCAN);
```

### 5.3 小编辑（≤ 100 行）

```
buffer.apply_edit
display_map.sync(...) → 同步 shape 受影响行 → splice → patch
self.current_snapshot = snap
render_cache.shift(at, delta)
render_cache.invalidate_rows(patch.affected_rows)
viewport.scroll_anchor.adjust_after_edit(&patch)
```

### 5.4 大编辑（> 100 行，例如粘贴 5000 行）

```
display_map.sync 内部：
  generation += 1
  worker.cancel_before(generation - 1)
  tree.splice(at..at, vec![placeholder; 5000])
  for chunk in chunks(at..at+5000, 200):
    for line in chunk: worker.submit(...)
render_cache.shift(at, +5000)
render_cache.invalidate_rows(at..line_count)

主线程每帧：
  for r in display_map.poll_worker():
    if r.generation == current { tree.splice(r.line..r.line+1, vec![r.entry]); render_cache.insert(...) }
  → 滚动条逐渐变长，画面无停顿
```

### 5.5 atlas LRU 驱逐

`GlyphAtlas::insert_with_eviction` 返回 `evicted: SmallVec<[GlyphKey; 4]>`。
RenderCache 维护 `reverse_index: GlyphKey → SmallVec<slot_id>`。
驱逐时遍历 evicted keys → 找到所有占用这些 key 的 slot_id → 找到使用这些 slot 的 doc_line → invalidate。

约 80 KB 反向索引内存预算（~80 字符/行 × 1000 行 cache）。

### 5.6 主题切换

`CachedLine.theme_independent = true`。`GlyphInstance` 不存 color，渲染时 `highlight_color_for_offset(spans, inst.byte_start, theme)` 即时查询。**主题切换 0 invalidate**。

## 6. 失效策略

### 6.1 失效矩阵

| 触发事件 | DisplayLineMap | RenderCache | atlas | snapshot | scroll_anchor |
|---------|---------------|-------------|-------|----------|---------------|
| 编辑 ≤ 100 行 | sync 同步精修 | invalidate_rows + shift | 不动 | 重新 snapshot | doc_line 不变；锚行被删则回退 |
| 编辑 > 100 行 | sync 占位 + 入队 worker | shift + 受影响段 invalidate | 不动 | 重新 snapshot | 同上 |
| viewport 宽度变化 | set_viewport_size + 入队 | invalidate_all | 不动 | 重新 snapshot | doc_line 不变；pixel_offset 折算 |
| 字号变化 | 同上 | invalidate_all | 不需要 flush（subpixel + size 已是 atlas key） | 重新 snapshot | 同上 |
| 主题切换 | 不动 | 不动 | 不动 | 不动 | 不动 |
| atlas LRU 驱逐 | 不动 | reverse_index 精确 invalidate | — | 不动 | 不动 |
| atlas full 兜底 | 不动 | bump atlas_generation 全 stale | — | 不动 | 不动 |
| 高亮 spans 变化（LSP） | 不动 | 不动（color 渲染时查） | 不动 | 不动 | 不动 |
| 选区变化 | 不动 | 不动 | 不动 | 不动 | 不动 |
| 文件重载 | from_buffer 重建 | invalidate_all | 不动 | 全新 | 重置 (0, 0) |
| DPI 变化 | set_viewport_size | invalidate_all | flush | 重新 snapshot | doc_line 不变 |

### 6.2 generation 三层校验

```
DisplayLineMap.generation: u64                    主线程递增
                            ▲ ▲ ▲
                            │ │ └── ReshapeWorker.current_generation: AtomicU64
                            │ └──── DisplaySnapshot.generation
                            └────── ReshapeRequest.generation
```

任何 sync 都 `generation += 1` + `worker.cancel_before(generation)`。worker 进入 `shape_and_wrap` 前 check generation；主线程 `poll_worker` 拿到 result 时再校验。

### 6.3 ScrollAnchor 不变量

1. `doc_line` 始终是有效行号或 `usize::MAX`（哨兵：空文档）。
2. `pixel_offset ≥ 0`。
3. sync 后必须 `anchor.adjust_after_edit(&patch)`。
4. resize 后 `pixel_offset *= new_line_height / old_line_height`。
5. 跳到末尾时 `total_rows < visible_rows` 钳制为 `(0, 0)`。

### 6.4 worker 队列与背压

- 队列上限 1000 pending。超过 → submit 返回 `Backpressured`，主线程走兜底同步。
- 大编辑分批入队：viewport ±100 行先入；其余 200 行/批后入。
- worker drop 时显式 `Shutdown` + join。

### 6.5 必守不变量

```
I1.  ∀ doc_line ∈ [0, line_count): tree.line_to_row(doc_line) ∈ [0, total_rows]
I2.  tree.total_rows == Σ entry.visual_line_count
I3.  Snapshot::clone 不复制 entries（Arc::strong_count > 1）
I4.  worker 收到的所有 result.generation ≤ current_generation
I5.  RenderCache.size ≤ visible_rows + 2 × OVERSCAN
```

## 7. 边界情况

- **空文件**：line_count=0, total_rows=0, scroll_anchor=(0,0), render_visible 空 vec。
- **超长行**：单行 byte_length > `max_line_bytes_for_shaping`（settings 开关，默认 0=关闭）时只显示前 N 字节 + 截断标记。
- **shape 失败**（unicode / 缺字）：worker 返回 placeholder entry，主线程接收后填空 instances，记录日志，不阻塞渲染。
- **快速远距离滚动**：cache miss 集中爆发 → 队列饱和 → 兜底超预算时画背景占位 → < 200ms 后稳定。
- **resize 拖动**：连续 resize 会触发 invalidate_all。**优化**：resize 节流 16ms，最后一次为准。

## 8. 测试策略

### 8.1 单元

```rust
// snap_tree
#[test] fn splice_preserves_total_rows();
#[test] fn clone_is_arc_shared();
#[test] fn find_by_row_matches_line_to_row_inverse();
#[test] fn large_build_20000_entries_under_50ms();

// display_line_map
#[test] fn sync_small_edit_synchronously_completes();
#[test] fn sync_large_edit_returns_placeholder_then_worker_refines();
#[test] fn worker_drops_stale_generation();
#[test] fn poll_worker_ignores_stale_results();
#[test] fn set_viewport_size_marks_all_dirty();

// render_cache
#[test] fn shift_offsets_keys_correctly();
#[test] fn invalidate_rows_only_drops_intersecting();
#[test] fn lru_capacity_capped();
#[test] fn theme_change_does_not_invalidate();
#[test] fn atlas_eviction_invalidates_only_affected_lines();

// scroll_anchor
#[test] fn anchor_doc_line_unchanged_after_edit_above();
#[test] fn anchor_pixel_offset_refolds_on_resize();
#[test] fn anchor_clamps_when_doc_shrinks();
```

### 8.2 集成

```rust
#[test] fn scrolling_does_not_call_shaper();         // mock shaper 计数
#[test] fn cold_start_returns_first_frame_under_30ms();
#[test] fn parallel_assert_display_map_matches_wrap_index();   // Phase 2 阶段使用
```

### 8.3 基准

`benches/scroll_bench.rs` 增加：
- `bench_scroll_4mb_json_frame`（目标 < 2ms）
- `bench_paste_5000_lines_sync_path`（目标 < 5ms）
- `bench_cold_open_4mb_to_first_frame`（目标 < 30ms）

## 9. 阶段切分

```
Phase 1  SnapTree                                    +500 / -0     基础数据结构 + 单测
Phase 2  DisplayLineMap + ReshapeWorker             +1000 / -0    与 WrapIndex 并行 + assert
Phase 3  RenderCache + 顶点重构                       +700 / -300  渲染从 WrapIndex 切到 Snapshot
Phase 4  ScrollAnchor                                +250 / -80   scroll_top → ScrollAnchor
Phase 5  清理 + 收尾                                  +50 / -900   删 wrap_index，加开关与节流
```

| 阶段 | 验收 |
|------|------|
| 1 | `cargo test -p app snap_tree` 全绿；`large_build_20000_entries < 50ms` |
| 2 | debug 模式下 DisplayLineMap 与 WrapIndex 在 1000 次随机查询中 100% 一致 |
| 3 | 4.1MB JSON 滚动一帧 < 2ms；主题切换 0 invalidate（计数器验证） |
| 4 | 在 doc_line=10 处插入 1000 行后 scroll_anchor.doc_line 不变；resize 时 anchor 不漂 |
| 5 | wrap_index.rs 不存在；超长行开关 + resize 节流均落地 |

每阶段独立可回退。

## 10. 文件改动总表

| 阶段 | 文件 | 操作 | 估计行数 |
|------|------|------|----------|
| 1 | `crates/app/src/snap_tree.rs` | 新建 | +500 |
| 1 | `crates/app/src/lib.rs` | 注册 module | +1 |
| 2 | `crates/app/src/display_line_map.rs` | 新建 | +650 |
| 2 | `crates/app/src/reshape_worker.rs` | 新建 | +250 |
| 2 | `crates/app/src/lib.rs` | 注册 module | +2 |
| 2 | `crates/app/src/app.rs` | 添加 display_map 字段 + parallel-assert 钩子 | +80 |
| 3 | `crates/app/src/render_cache.rs` | 新建 | +400 |
| 3 | `crates/app/src/render_pipeline.rs` | 重写 shape_visible_lines | +400 / -300 |
| 3 | `crates/app/src/render_geom.rs` | GlyphInstance 类型迁入 | +50 / -10 |
| 3 | `crates/render/src/lib.rs` | GlyphAtlas::insert_with_eviction | +30 / -5 |
| 3 | `crates/app/src/app.rs` | TextState 接 RenderCache | +40 / -20 |
| 4 | `crates/app/src/viewport.rs` | scroll_top → ScrollAnchor | +120 / -40 |
| 4 | `crates/app/src/mouse.rs` | 滚轮 delta 转 anchor | +30 / -10 |
| 4 | `crates/app/src/commands.rs` | 跳转命令构造 anchor | +30 / -10 |
| 4 | `crates/app/src/scrollbar.rs` | 滚动条与 anchor 互转 | +40 / -20 |
| 5 | `crates/app/src/wrap_index.rs` | 删除 | -900 |
| 5 | `crates/app/src/app.rs` | 删 shape_cache/wrap_cache + parallel-assert | -50 |
| 5 | `crates/app/src/settings.rs` | max_line_bytes_for_shaping 开关 | +10 |
| 5 | `crates/app/src/app.rs` | resize 16ms 节流 | +20 |
| 1-5 | `crates/app/Cargo.toml` | 添加 smallvec / xxhash-rust 依赖 | +2 |
| 1-5 | tests / benches | 各阶段单测 + 集成 | +800 |

合计：新增约 3700 行（含测试），删除约 1400 行，净增约 2300 行。

## 11. 风险登记

| 风险 | 缓解 |
|------|------|
| atlas 驱逐反向索引内存爆 | OVERSCAN=500、字形上限 ~80K、反向索引绑定 cache LRU 同步缩减 |
| worker 与主线程消息序破坏 | generation 三层校验（§6.2）+ I4 测试 |
| Snapshot Arc 共享让 worker 误读旧 tree | worker 不持有 Snapshot，只读 ReshapeRequest 拷贝字节 |
| smallvec / xxhash-rust 依赖引入 | 都是社区主流 crate，体积小、无 transitive 风险 |
| Phase 2 并行 assert 性能拖慢 debug 构建 | 仅在 `cfg(debug_assertions) && env "EDIT_PARALLEL_ASSERT=1"` 时启用 |
| 单行 100MB 罕见极端 | settings 开关 max_line_bytes_for_shaping，默认关闭 |

## 12. 与原方案 (`docs/plans_large_file_scroll_perf.md`) 的差异

| 维度 | 原方案 | 本设计 |
|------|--------|--------|
| 顶点缓存粒度 | 成品 GlyphVertex（每行 Vec<GlyphVertex>） | GlyphInstance 行内相对坐标，渲染时合成 |
| 内存策略 | 全文档缓存（估算 40MB，实际约 460MB） | viewport ±500 LRU 淘汰，约 3MB |
| 后台并发 | std::thread::spawn 每次新建 | 单 worker 线程 + mpsc，generation 取消 |
| 主题切换 | 全 cache invalidate | 0 invalidate（color 渲染时查） |
| atlas LRU 驱逐 | 全 cache flush | 反向索引精确 invalidate |
| ScrollAnchor | 同方案 | 同方案 |
| WrapIndex | 同方案最终删除 | 同方案最终删除 |
| 文件计划 | plans_large_file_scroll_perf.md | docs/superpowers/specs + 后续 writing-plans 产物 |

---

设计稿就绪，writing-plans 阶段会按 §9 的 5 阶段进一步拆 task / step。
