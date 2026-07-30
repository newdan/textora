# 视口驱动布局 (Viewport-Driven Lazy Layout)

## 问题

当前小说模式对全文全量 O(n) 处理，且 text buffer 被重复读取 3 次：

```
文档 101K 行 ──[build_from_novel_doc 38ms]──▶ 50K BlockNode   ← 第1次读全文
                         ──[from_doc 34ms]──▶ 50K LaidOutBlock  ← 第2次读全文
                                  ──[build_flat_lines 28ms]──▶ 101K FlatLine ← 第3次读全文
                                              ──[precise shape 29ms]──▶ 视口 30 行
```

首帧 138ms，视口只需 30 行。layout.rs 4580 行承担了布局引擎 + 文本测量 + 塑形 + 表格 + FlatLine + CJK + 92 测试，职责混乱。

## 目标

- 首帧 O(n) 轻量扫描 + O(visible) 布局，根除重复读取
- 首帧耗时与文档大小解耦
- `BlockSource` trait 统一输入，`LazyLayout<S>` 统一布局；一个引擎覆盖小说/预览/编辑三条路径
- `layout.rs` 拆分为职责单一的模块

## 核心设计

### BlockSource trait

LazyLayout 只需要知道"有哪些 block、各自多高"，不关心数据来源：

```rust
/// LazyLayout 通过此 trait 查询文档结构。
pub trait BlockSource {
    /// 可按视口范围物化的顶层 block 数量。
    fn block_count(&self) -> usize;

    /// 第 `i` 个 block 的基本信息。
    fn block_info(&self, i: usize) -> Option<BlockInfo>;

    /// 全文估算总高度（纯算术，<1ms）。
    fn total_height_estimate(&self, style: &MarkdownStyle) -> f32;

    /// 文档级标题列表（用于 ToC）。
    fn headings(&self) -> &[HeadingEntry];

    /// 将第 `i` 个 block 物化为 LaidOutBlock（精确塑形）。
    /// 调用方提供 LayoutCtx，实现方填充 LaidOutBlock。
    fn materialize_block(
        &self,
        i: usize,
        ctx: &mut LayoutCtx,
    ) -> LaidOutBlock;
}

struct BlockInfo {
    kind: BlockKind,
    line_count: usize,
    /// 估算高度（用于滚动定位，纯算术无塑形）。
    estimated_height: f32,
}
```

### 三种 BlockSource 实现

```
                     BlockSource trait
                           │
         ┌─────────────────┼─────────────────┐
         ▼                 ▼                  ▼
   MarkdownDoc         NovelStructure      (未来扩展)
   .md WYSIWYG/预览     .txt 阅读           .pdf, .epub...
   BlockNode 全量树      Vec<LineMeta> 扫描
```

| | MarkdownDoc | NovelStructure |
|------|-------------|----------------|
| 来源 | `parse_markdown()` | `NovelStructure::scan()` |
| 数据结构 | 全量 BlockNode 树 | 平铺 `Vec<LineMeta>` |
| block 粒度 | 递归树 (Container 展开) | 扁平 section (段落/标题/空行) |
| 内存 | 50K 个 BlockNode (大量空 children/text_lines) | 50K 个 LineMeta (field struct，无堆分配) |

### NovelStructure — 轻量扫描

从 `build_from_novel_doc` 的 body 提取，去掉 BlockNode 创建：

```rust
pub struct NovelStructure {
    sections: Vec<LineMeta>,
    headings: Vec<HeadingEntry>,
}

struct LineMeta {
    kind: LineKind,            // Empty / Heading { level } / Body
    byte_range: Range<usize>,  // 不含尾部换行
    line_count: usize,         // 本 section 占几行（正文段落可能多行）
}

enum LineKind { Empty, Heading { level: u8 }, Body }
```

`scan()` 循环和 `build_from_novel_doc` 相同（逐行读取 + `classify_title`），区别是结果只记录数量而不分配 String/Vec。101K 行预计 8-12ms。

高度估算在 `total_height_estimate()` 中完成——遍历 LineMeta，分类累加，纯算术 <1ms。

### LazyLayout<S: BlockSource> — 统一视口驱动布局

```rust
pub struct LazyLayout<S: BlockSource> {
    pub source: S,

    // ── 全局（O(n) 纯算术）──
    pub estimated_heights: Vec<f32>,   // 每个 block 的估算高度
    pub y_delta: Vec<f32>,             // 累积精确修正
    pub total_height: f32,

    // ── 稀疏（仅视口 ± buffer）──
    pub laid_out: Vec<Option<LaidOutBlock>>,  // None = 未物化
    pub precise: Vec<bool>,
    pub flat_lines: Vec<FlatLine>,
    pub block_line_map: Vec<(usize, usize)>,
    pub line_byte_offsets: Vec<usize>,

    // 缓存
    viewport_range: Range<usize>,       // 当前物化的 block 范围
    evict_ratio: f32,
}
```

核心方法：

```rust
impl<S: BlockSource> LazyLayout<S> {
    /// 轻量初始化：只算 estimated_heights，不做布局。
    pub fn new(source: S, style: &MarkdownStyle) -> Self;

    /// 确保 [scroll_y - buf, scroll_y + viewport_h + buf] 范围内所有 block 已物化。
    /// 新进入范围的 block 创建并精确塑形，离开的淘汰。
    pub fn ensure_visible(
        &mut self,
        scroll_y: f32,
        viewport_h: f32,
        style: &MarkdownStyle,
        shaper: &mut shaping::Shaper,
        doc: &dyn core::document::DocView,
    );

    /// 淘汰远离视口的 block（释放 HarfBuzz shaped data）。
    fn evict_distant(&mut self, scroll_y: f32, viewport_h: f32);

    /// 重建 flat_lines（仅 visible 范围）。
    fn rebuild_flat_lines(&mut self, doc: &dyn core::document::DocView);
}
```

### 数据流

```
文件打开
  │
  ├── .txt → NovelStructure::scan()   8-12ms  O(n) 轻量
  ├── .md  → parse_markdown()         ~5ms    O(n) 全量树（现状不变）
  │
  ▼
LazyLayout::new(source, style)        <1ms    纯算术 estimated_heights
  │
  ▼
每帧 render():
  ├─ ensure_visible(scroll_y, viewport_h)
  │    ├─ 二分查找 estimated_heights 定位可见 block 范围
  │    ├─ 物化新进入范围的 block（source.materialize_block）
  │    └─ 淘汰远离视口的 block（设 laid_out[i] = None）
  ├─ rebuild_flat_lines（仅 visible 范围，~30 条）
  ├─ 渲染 DrawList
  └─ 缓存 DrawList（scroll_y + viewport 不变时复用）
```

### 与现有流程对比

| 阶段 | 现状 (101K 行) | 新方案 (101K 行) |
|------|---------------|-----------------|
| 源解析 | build_from_novel_doc 38ms | scan() 10ms |
| 估算布局 | from_doc 34ms | new() <1ms |
| 精确塑形 | ensure_precise 29ms | materialize ~5ms |
| FlatLine | build_flat_lines 28ms | rebuild ~1ms |
| **首帧** | **138ms** | **~18ms** |

Markdown 预览路径首帧不变（全量 BlockNode 已在解析时建好，布局改为视口驱动）。WYSIWYG 编辑路径保持现状（全量布局，首次点击/输入时扩展）。

## layout.rs 拆分

layout.rs 当前 4580 行。按职责拆为独立模块：

```
markdown/src/
  layout/
    mod.rs           (~300) — BlockSource trait, LazyLayout<S>, apply_deltas
    types.rs         (~400) — LaidOutDoc, LaidOutBlock, LaidOutLine, FlatLine, StyleSegment
    context.rs       (~500) — LayoutCtx, char_width, CJK helpers
    block.rs         (~500) — layout_block, layout_text_block, layout_table
    text.rs          (~200) — collect_text_lines, collect_text_lines_with_styles
    shaping.rs       (~500) — shape_line, segment_text_layout, compute_style_segments
    flat_lines.rs    (~400) — build_flat_lines, flatten_block_into, collect_line_byte_offsets
    estimation.rs    (~150) — estimate_line_count, heading_spacing_scale
```

各模块职责单一，内部耦合通过 `use super::*` 访问基本类型。`mod.rs` 只暴露公共 API，测试留在各自模块内。

## 实施阶段

### Phase 1: layout.rs 拆分（无行为变更）

- 纯代码搬家，不改变任何逻辑
- 全部现有测试保持绿色
- 输出：`layout/` 目录 + 原有功能不变

### Phase 2: BlockSource trait + LazyLayout 泛型化

- 定义 `BlockSource` trait
- `impl BlockSource for MarkdownDoc`
- `LazyLayout` 改为 `LazyLayout<S: BlockSource>`
- 所有现有调用点适配（MarkdownView, MarkdownEditorView）
- 行为不变，测试不变

### Phase 3: NovelStructure + scan()

- 从 `build_from_novel_doc` 提取分类逻辑
- 实现 `NovelStructure::scan()` + `impl BlockSource for NovelStructure`
- `NovelView` 改用 `NovelStructure`，不再调用 `build_from_novel_doc`
- 此时布局仍是全量（Phase 4 前不变）

### Phase 4: LazyLayout 视口驱动

- `new()` 只算 `estimated_heights`，不创建 `laid_out.blocks`
- `ensure_visible()` 按需物化
- `evict_distant()` 按需淘汰
- `rebuild_flat_lines()` 只构建 visible 范围
- `render()` 适配缓存策略

### Phase 5: Markdown 预览路径切换

- MarkdownView 复用 LazyLayout（非编辑态走视口驱动）
- 滚动性能提升，首帧布局成本降低

## 风险

| 风险 | 缓解 |
|------|------|
| scroll bar 高度估算不准 | 物化后 y_delta 修正 total_height |
| Phase 1 代码搬家破坏功能 | 搬家不改逻辑，每个 phase 跑全量测试 |
| 泛型化影响编译时间 | S 仅在 LazyLayout 一处泛化，下游不传播 |
| 编辑态仍全量，入口判断复杂 | 编辑态触发时 `ensure_all_blocks()` 一次性补全 |
