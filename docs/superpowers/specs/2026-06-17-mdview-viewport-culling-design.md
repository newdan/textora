# MdView 视口裁剪 — 设计文档

> 制定日期：2026-06-17
> 输入痛点：5MB .md 文件打开/滑动卡顿，全链路无视口裁剪
> 参考架构：`crates/app/src/render_pipeline.rs` shape_visible_lines, `crates/ui/src/viewport.rs` ScrollAnchor
> 范围：对齐文本编辑器的 ScrollAnchor + visible-range-render 模型，给 markdown 渲染加视口裁剪

## 1. 根因

| | Text Editor (DocumentView) | Markdown Preview |
|---|---|---|
| 起点 | `scroll_anchor.doc_line` | 0（全量） |
| 迭代 | `visual_line_counter >= vp_height+2` 即 break | `for block in &doc.blocks` 遍历全树 |
| CPU 工作量 | O(viewport_lines) ≈ 40 行 | O(total_blocks) ≈ 全文档数千 block |
| GPU 裁剪 | N/A | `PushClip` 剔除顶点，CPU 不省 |

Markdown 的 `render_doc_with_offset()` 每一帧遍历完整 `LaidOutDoc` block tree，为每行生成 `DrawCmd`。`PushClip` 只让 GPU 剔除屏幕外几何，CPU 侧的 tree walk + DrawCmd 生成全量执行。

Layout pass 全量处理是正常的且已缓存（仅 dirty 时重算），问题出在 render pass 没有视口裁剪。

## 2. 两套架构的对应关系

```
Text Editor:
  TextBuffer → DisplayLineMap (SnapTree<DisplayLineEntry>)
    → DisplaySnapshot (Arc clone, O(1))
    → ScrollAnchor { doc_line, pixel_offset }
    → shape_visible_lines: 从 anchor 起，填满 viewport 即 break → O(viewport)
    → RenderCache (LruCache<doc_line, CachedLine<GlyphInstance>>)

Markdown (当前):
  Source → Parse → MarkdownDoc (BlockNode tree)
    → Layout → LaidOutDoc (LaidOutBlock tree, 缓存)
    → Render: for block in ALL blocks → DrawCmds → GPU clip → O(total)
    → 无 ScrollAnchor, 无 row index, 无 block cache
```

对齐策略：给 `LaidOutDoc` 加 `BlockRowIndex`，render 按视口裁剪，scroll 用 `ScrollAnchor`。

## 3. 数据结构

### 3.1 BlockRowIndex（新增）

`LaidOutBlock` 是树结构 — 顶层 `LaidOutDoc.blocks: Vec<LaidOutBlock>`，`BlockQuote`/`ListItem` 内嵌 `blocks: Vec<LaidOutBlock>`。RowIndex 需要穿越这棵树定位到具体的 line。

```rust
/// 平铺的行索引，layout 完成后构建。
/// 把树状 block 结构展平成 row→(block_path, line_idx) 映射。
pub struct BlockRowIndex {
    /// 每行 y 起始（累加像素），二分查找可见行起点。
    row_y_starts: Vec<f32>,
    /// 每行对应的 block 树路径。
    /// path[0] = 顶层 block 索引，path[1] = 该容器内子 block 索引，…
    /// 绝大多数 block 是顶层的 → path 长度 1，用 SmallVec 避免堆分配。
    row_block_paths: Vec<SmallVec<[usize; 4]>>,
    /// 该行在目标 block 的 lines[] 中的索引。
    row_line_idxs: Vec<usize>,
}
```

**构建**：layout 完成后，递归遍历 `LaidOutBlock` 树，对每个 `LaidOutLine` push 一行到 index。O(total_lines)，与 layout 同频。

### 3.2 LaidOutDoc 加字段

```rust
pub struct LaidOutDoc {
    pub blocks: Vec<LaidOutBlock>,
    pub total_height: f32,
    pub row_index: BlockRowIndex,    // 新增
}
```

### 3.3 MarkdownPreview 改 scroll 模型

```rust
// before
pub scroll_y: f32,

// after
pub viewport: Viewport,  // 含 scroll_anchor + visible_rows + viewport_height
```

`BlockRowIndex` 实现 `ui::viewport::LineMap`：

```rust
impl ui::viewport::LineMap for BlockRowIndex {
    fn map_line_count(&self) -> usize { self.row_y_starts.len() }
    fn map_total_rows(&self) -> usize { self.row_y_starts.len() }
    fn map_display_to_doc(&self, row: usize) -> usize { row }
    fn map_doc_to_display(&self, doc: usize) -> usize { doc }
    fn visual_line_count(&self, _row: usize) -> u16 { 1 }
}
```

1:1 映射使 markdown 复用 `Viewport::scroll_pixels` / `clamp_anchor` / `derive_scroll_top`。

## 4. 渲染裁剪

### 4.1 Block 级 — 用 BlockRowIndex 定位可见 block

```
render_doc_with_offset():
  first_row = row_index.binary_search(scroll_y)
  last_y = scroll_y + viewport_h
  rendered_paths = HashSet::new()

  for row in first_row..row_index.len():
    if row_y_starts[row] > last_y: break
    path = &row_block_paths[row]
    if rendered_paths.contains(path): continue   // 该 block 已在前面 row 渲染过
    block = resolve_path(doc, path)               // 沿 path 递归取引用
    if block.rect.bottom() < scroll_y: continue
    if block.rect.top() > last_y: continue
    render_block_with_offset(block, ...)
    rendered_paths.insert(path)
```

`resolve_path(doc, &[top_idx, child_idx, ...])` 沿树逐级取值：`&doc.blocks[top_idx]` → `.kind` 解引用到子 `blocks[child_idx]` → …

大多数 block 是顶层 → path 长度 1，`rendered_paths` 用 `BTreeSet` 或 `Vec<bool>` 映射。

### 4.2 Line 级 — Text/CodeBlock 分支内 line guard

```rust
LaidOutBlockKind::Text { lines } => {
    for line in lines {
        let ly = line.rect.y - scroll_y + oy;
        if ly + line.rect.h < 0.0 { continue; }
        if ly > viewport_h { continue; }
        render_line_with_offset(line, ...);
    }
}
```

容器 block（BlockQuote, ListItem, Table）：各自 rect 与视口相交才进入，进入后递归渲染子 block（子 block 各自再有 line 级 guard）。容器 border/bg 在 block 级绘制。

### 4.3 复杂度

| | 当前 | 裁剪后 |
|---|---|---|
| 迭代 block 数 | O(total_blocks) | O(viewport_blocks) |
| 迭代 line 数 | O(total_lines) | O(viewport_lines) |
| 生成 DrawCmd 数 | 全量 | 仅可见 |

5MB 文档视口内通常 ~50 block、~200 line。

## 5. ScrollAnchor 对齐

`BlockRowIndex` 的 row index 映射为 `doc_line`：

```
ScrollAnchor.doc_line  ←→  row index
ScrollAnchor.pixel_offset ←→  行内像素偏移
line_height             ←→  MarkdownStyle.line_height
```

`BlockRowIndex` 的 `LineMap` 实现：

```rust
impl ui::viewport::LineMap for BlockRowIndex {
    fn map_line_count(&self) -> usize { self.row_sources.len() }
    fn map_total_rows(&self) -> usize { self.row_sources.len() }
    fn map_display_to_doc(&self, row: usize) -> usize { row }
    fn map_doc_to_display(&self, doc: usize) -> usize { doc }
    fn visual_line_count(&self, _row: usize) -> u16 { 1 }
}
```

## 6. 数据流

| 事件 | Layout | RowIndex | Render | 备注 |
|------|--------|----------|--------|------|
| 首次打开 | 全量 parse+build+layout | 全量 build | 仅可见 | - |
| 滚动 | 命中缓存 | 命中缓存 | 仅可见 | 仅 block 级 + line 级裁剪 |
| 主题切换 | dirty（style_hash 变） | 重建 | 仅可见 | - |
| 视口 resize | dirty（viewport_w 变） | 重建 | 仅可见 | - |
| 源码编辑后切回 | dirty（source_hash 变） | 重建 | 仅可见 | - |

### 6.1 每帧渲染热路径（scroll 1px）

```
1. derive_scroll_top → scroll_y (O(1))
2. binary_search row_index → first_visible_row (O(log n))
3. for visible rows → render blocks (O(viewport))
4. PushClip → GPU (不变)
```

### 6.2 冷启动（5MB .md，首屏）

```
T=0      MdPreviewState::set_source + render
T≈Nms    parse + build + layout（全量，已有缓存）
T≈N+2ms  build BlockRowIndex（与 layout 同频，O(total_lines)）
T≈N+5ms  首帧 render: binary_search + render visible blocks + drain vertices
         后续帧: 命中 layout 缓存 + row_index，仅 render visible
```

## 7. 非目标

- 不引入 per-block DrawCmd cache（当前 cached_dl + cached_vertices 已覆盖 idle 帧场景）
- 不改 layout pass 的计算量（layout 全量是语义需要，且已缓存）
- 不改 markdown parser/builder

## 8. 文件改动

| 文件 | 操作 | 估计行数 |
|------|------|----------|
| `crates/markdown/src/layout.rs` | 新增 `BlockRowIndex` + `build_row_index()` | +60 |
| `crates/markdown/src/render.rs` | 重写 `render_doc_with_offset`，加 block/line 级裁剪 | +50 / -20 |
| `crates/app/src/md_preview.rs` | `scroll_y: f32` → `viewport: Viewport`；`scroll()` / `render()` 适配 | +40 / -30 |
| `crates/app/src/app_renderer.rs` | markdown 渲染路径传入 viewport 参数 | +10 / -5 |
| `crates/app/src/app_scroll.rs` | markdown 滚动路径用 `viewport.scroll_pixels()` | +10 / -10 |
| `crates/markdown/src/lib.rs` | 暴露 `BlockRowIndex` + `LineMap` impl | +15 |
| 各文件 tests | 裁剪正确性 + 性能回归 | +80 |

合计：新增约 265 行，删除约 65 行，净增约 200 行。

## 9. 风险

| 风险 | 缓解 |
|------|------|
| `BlockRowIndex` 的 row 粒度（1 row = 1 visual line）与实际行高不完全一致 | row_y_starts 存实际像素 y，二分查找精确；`visual_line_count` 返回 1 是保守上界（可能少算视口行数，但不会遗漏） |
| 容器 block 的子 block 可能在视口内但父 block rect 不完全覆盖 | 进入容器后递归检查每个子 block 的 rect，不依赖父 rect 做子级裁剪 |
| 二分查找边界 off-by-one | 用 `row_y_starts.binary_search_by()` 的标准语义 + overscan 3 行 |
