# TextLayout: 统一文本渲染管线

## 动机

当前有三条独立的文本渲染路径，各自 shape、各自缓存（或不缓存）：

| 管线 | 路径 | 缓存 |
|------|------|------|
| 编辑器 | `shape_visible_lines` → `CachedLine` → `emit_vertices` | `RenderCache<doc_line, CachedLine>` (LRU 1000) |
| 预览 | `LaidOutDoc` → `DrawList::Text` → `drain()` → `emit_text()` | 仅缓存 `LaidOutDoc`，**文本每帧 re-shape** |
| UI | `DrawList::Text` → `drain()` → `emit_text()` | 无缓存，每帧 re-shape |

预览和 UI 的 `emit_text()` 每帧重复 shape 相同文本，是纯粹的性能浪费。

## 核心设计

### Harfbuzz shape 与 atlas rasterize 分离

| 步骤 | 做什么 | 代价 | 时机 |
|------|--------|------|------|
| Harfbuzz shape | 产出 glyph ID + advance + position | CPU，极快 | Layout 阶段（全部） |
| Atlas rasterize | glyph → 位图 → GPU upload | GPU，昂贵 | Render 阶段（仅可见行） |

- **Harfbuzz shape** 结果（`ShapedRun`）在 layout 阶段产出并存入 `LaidOutLine`，为折行提供精确宽度
- **Atlas rasterize** 仅在 render 阶段对可见行执行，结果缓存在 `RenderCache`

### 统一起点：CachedLine + GlyphInstance

复用编辑器现有的缓存结构：

- `GlyphInstance` — 行内坐标字形实例（atlas UV + bearing + highlight_kind）
- `CachedLine` — 单行渲染缓存（instances + cluster_data + content_hash）
- `RenderCache` — LRU 缓存（容量约 1000 行）
- `emit_vertices()` — 从 CachedLine 发射 GlyphVertex（零 shaping、零 atlas 查询）

### 缓存 key 策略

| 管线 | Key | 原因 |
|------|-----|------|
| 编辑器 | `doc_line` (行号) | 百万行规模，按需 shape，行号索引最直接 |
| 预览 | `UiTextLayout.id` (layout 时分配，跨帧稳定) | LaidOutLine 在 layout 时构建 UiTextLayout，ID 一次分配，render 阶段只传 Arc，不重建 |
| UI | `UiTextLayout.id` (全局自增 u64) | UI 文本短且稳定，widget 内容变化时重建 UiTextLayout |

各维护独立的 `RenderCache` 实例（编辑器和预览字体/字号不同，共享 LRU 反而互相 evict）。

### 颜色延迟解析

`GlyphInstance.highlight_kind` 存储颜色索引（u8），`emit_vertices` 时查 `color_map` 解析为 RGBA。主题切换只需重新 emit，无需 re-shape。

## 架构

```
                    Harfbuzz shaping (CPU)
                    ════════════════
                    ↓
预览:  MD → AST → layout_doc → LaidOutLine { shaped_run }
编辑器: Buffer → shape_visible_lines (按需) → clusters
UI:     widget.text_layout (内容变化时 shape 一次)

                    DrawList (Z-order 保证)
                    ════════════════════
                    ↓
预览:  render_doc → DrawList { FillRect, TextLayout, Clip, ... }
UI:     widget.paint → ctx.list.fill_rect() / .text_layout()

                    drain() in app 层
                    ════════════════════
                    ↓
                    DrawList 遍历（严格顺序）：
                      FillRect → push_quad()
                      TextLayout → atlas rasterize (按需) → CachedLine → emit_vertices()
                      Clip → clip_stack
                    ↓
                    GlyphVertex[] → GPU (单次 render pass)
```

## 各管线改动

### 预览

**Layout 阶段**（`layout.rs`）：

- `layout_doc_with_shaper()` 对每行调用 `shaper.shape()` 拿精确宽度做折行
- 产出 `ShapedRun` 存入 `LaidOutLine`（新增字段）
- 无 atlas 交互

**Render 阶段**（`md_preview.rs` + `render.rs`）：

- 持有独立 `RenderCache`（content_hash key）
- 遍历可见 `LaidOutLine`：
  - 计算 `content_hash = hash(text, font_size, font_weight, font_style)`
  - Cache hit → `CachedLine::emit_vertices()`
  - Cache miss → `shaped_run` + `resolve_glyph()` → 构建 `CachedLine` → `cache.insert()` → `emit_vertices()`
- `DrawList` 仅用于形状（`FillRect`/`StrokeRect`/`Clip`），不再产生 `Text` 命令

### 编辑器

行为不变（`shape_visible_lines` 按需 shape + 缓存）。本次不改动编辑器的 RenderCache key 策略（保持 `doc_line`）。

共享改进：
- 预览和编辑器使用相同的 `CachedLine`、`GlyphInstance`、`emit_vertices()` 实现
- `resolve_glyph()` 已是共享的，不变

### Atlas generation 失效

`CachedLine.atlas_generation` 记录缓存时的 atlas 代际。atlas eviction 时递增代际，缓存条目在 `emit_vertices` 前检查代际，过期则重新 rasterize（但无需 re-shape，因 ShapedRun 仍有效）。

### UI

UI 层持有纯数据的 `UiTextLayout`（ShapedRun + metadata），**不参与 atlas / GPU 操作**：

```
内容变化时:  widget.shape_text(text, style) → UiTextLayout (纯 harfbuzz, 无 atlas)
paint 时:    ctx.list.text_layout(x, y, color, &self.text_layout)
             → DrawCmd::TextLayout { layout: Arc<UiTextLayout>, x, y, color }
drain 时:    app 层遍历 DrawList，遇到 TextLayout → 查 UI RenderCache
             → atlas rasterize + CachedLine + emit_vertices
             → 顶点按 DrawList 顺序入队，Z-order 完整保留
```

- `UiTextLayout` 定义在 `crates/ui`（纯数据，零 GPU 依赖）
- 每个 widget 持有 `Option<UiTextLayout>`，内容或样式变化时重建
- UI 专属 `RenderCache` 在 app 层，key 用全局自增 ID（`UiTextLayout` 创建时分配）

### DrawList 和 DrawCmd

`DrawCmd::Text { content, font_size, color, ... }` 删除，替换为：

```rust
// crates/ui/src/core/paint.rs
pub enum DrawCmd {
    FillRect { rect, color, radius },
    StrokeRect { rect, color, radius, line_width },
    FillTriangle { p0, p1, p2, color },
    PushClip(Rect),
    PopClip,
    // 替代原来的 Text 变体 — 携带预 shape 数据，无原始文本
    TextLayout {
        layout: Arc<UiTextLayout>,  // 纯 harfbuzz 结果，无 GPU 依赖
        x: f32,
        y_baseline: f32,
        color: [f32; 4],
    },
}
```

`DrawList` 不再"退化为纯形状"，而是从"持有原始文本"升级为"持有预 shape 数据"。形状和文本的绘制顺序由 DrawList 的插入顺序保证。

### 依赖边界

```
crates/ui          → UiTextLayout (纯 harfbuzz shape 数据，无 atlas)
                      DrawCmd::TextLayout { Arc<UiTextLayout>, ... }

crates/app         → UiTextLayout → atlas rasterize → CachedLine
                      drain() 中统一处理：形状直接顶点，文本走 RenderCache
```

UI 层只接触 harfbuzz（通过现有的 `Shaper` 参数传入），不依赖 `GlyphAtlas`、`RenderCache`、`GpuState`、`emit_vertices`。架构约束完整保留。

## 迁移步骤

### Step 1: UiTextLayout 纯数据类型

- 在 `crates/ui` 定义 `UiTextLayout`：持有 harfbuzz shape 结果（ShapedRun）+ text + style + 全局自增 ID
- 定义 `DrawCmd::TextLayout { layout: Arc<UiTextLayout>, x, y_baseline, color }`
- UI widget（TextBox、StatusBar、Tooltip 等）在内容变化时调用 shaper 构建 `UiTextLayout`

### Step 2: LaidOutLine 持 ShapedRun

- `layout_doc_with_shaper()` 中 shape 结果存入 `LaidOutLine`
- 当前已有 shape 调用（用于 StyleSegment 宽度），改为保存结果而非丢弃
- 风险：内存增加（每行多存 ShapedRun 数据），预览文档通常 < 1000 行可忽略

### Step 3: 预览 RenderCache + emit 路径

- `MarkdownPreview` 新增 `render_cache: RenderCache`（content_hash key）
- `render_doc_with_offset()` 改为产生 `DrawCmd::TextLayout`（携带 LaidOutLine 的 ShapedRun），而非 `DrawCmd::Text`（原始字符串）
- `paint_backend::drain()` 中新增 `TextLayout` 分支：atlas rasterize → CachedLine → cache → emit_vertices
- `RenderCache` 需要支持 content_hash 作为 key

### Step 4: 删除死代码

- 移除 `emit_text()` 函数（整个函数）
- 移除 `DrawCmd::Text` 变体
- 清理 `drain()` 中的旧 `Text` 分支
- 所有 `DrawList::text()` 调用改为 `DrawList::text_layout()`

## 关键实现约束

- **依赖边界**：`crates/ui` 定义 `UiTextLayout`（harfbuzz shape 结果 + text + id）。UI 层不依赖 `crates/app` 的 `RenderCache`、`GlyphAtlas`、`GpuState`。UI widget 通过 `Shaper` 参数（已存在于 UI paint 上下文）执行 harfbuzz shape。
- **RenderCache key 泛化**：编辑器保持 `usize` key，预览用 `u64` content_hash，UI 用 `u64` layout.id。各自独立 cache 实例，无需泛化为 trait。
- **LaidOutLine 新增字段**：`text_layout: Option<Arc<UiTextLayout>>`，layout 阶段构建一次（ID 此时分配，跨帧稳定）。render 阶段直接传递 Arc，不重建。atlas 失效时 UiTextLayout 内的 ShapedRun 仍然有效，可直接用于重新 rasterize。
- **Z-order**：`TextLayout` 和 `FillRect` 在同一个 DrawList 中按插入顺序排列。`drain()` 顺序遍历，顶点按序入队，层级天然正确。

## 不变

- `GlyphAtlas` / shelf-packing 逻辑
- `resolve_glyph()` 函数
- `GlyphVertex` / `GlyphRenderer` / wgpu render pass
- 编辑器 `shape_visible_lines` 主循环结构
- `DisplayLineMap` / `SnapTree`
