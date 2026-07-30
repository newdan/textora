# TextLayout: 统一文本渲染管线 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除预览和 UI 管线的每帧 re-shape，统一到 CachedLine + emit_vertices 路径

**Architecture:** Preview 在 layout 阶段保存 ShapedRun 到 LaidOutLine；render 阶段对可见行通过独立 RenderCache 做 atlas rasterize + emit。UI 通过 DrawCmd::TextLayout 携带预 shape 数据，在 app 层 drain 时处理。DrawCmd::Text / emit_text() 删除。

**Tech Stack:** Rust, wgpu, harfbuzz (shaping crate), edit+ crates

---

### Task 1: UiTextLayout — 纯数据类型

**Files:**
- Create: `crates/ui/src/core/text_layout.rs`
- Modify: `crates/ui/src/core/mod.rs`

- [ ] **Step 1: 创建 UiTextLayout 模块文件**

```rust
// crates/ui/src/core/text_layout.rs
//! UiTextLayout — 纯 harfbuzz shape 结果，零 GPU 依赖。
//! Widget 在内容变化时构建，paint 时通过 DrawCmd::TextLayout 传递到 app 层。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 全局自增 ID，用于 RenderCache key
static NEXT_LAYOUT_ID: AtomicU64 = AtomicU64::new(1);

/// 预 shape 的文本布局数据（纯 harfbuzz 产出，无 atlas）。
/// 在 crates/ui 定义，app 层消费。
#[derive(Clone)]
pub struct UiTextLayout {
    /// 全局唯一 ID（RenderCache key）
    pub id: u64,
    /// 原始文本
    pub text: String,
    /// 字号
    pub font_size: f32,
    /// 字体族
    pub font_family: Option<String>,
    /// 字重
    pub font_weight: shaping::Weight,
    /// 字体样式
    pub font_style: shaping::Style,
    /// Harfbuzz shape 结果（clusters with glyph IDs, advances, positions）
    pub shaped: shaping::ShapedRun,
}

impl UiTextLayout {
    /// 从已有的 ShapedRun 构造（避免 re-shape）。
    /// 调用时机：已有 shape 结果（如 preview layout 产出）。
    pub fn from_shaped(
        text: &str,
        font_size: f32,
        font_family: Option<String>,
        font_weight: shaping::Weight,
        font_style: shaping::Style,
        shaped: shaping::ShapedRun,
    ) -> Self {
        Self {
            id: NEXT_LAYOUT_ID.fetch_add(1, Ordering::Relaxed),
            text: text.to_string(),
            font_size,
            font_family,
            font_weight,
            font_style,
            shaped,
        }
    }

    /// Shape text 并创建 UiTextLayout。
    /// 调用时机：widget 内容或样式变化时。
    pub fn new(
        text: &str,
        font_size: f32,
        font_family: Option<String>,
        font_weight: shaping::Weight,
        font_style: shaping::Style,
        shaper: &mut shaping::Shaper,
    ) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        // Save and restore shaper state
        let old_size = shaper.font_size();
        let old_weight = shaper.font_weight();
        let old_style = shaper.font_style();
        let old_family = shaper.font_family().map(|s| s.to_string());
        shaper.set_font_size(font_size);
        shaper.set_font_weight(font_weight);
        shaper.set_font_style(font_style);
        if font_family.is_some() {
            shaper.set_font_family(font_family.as_deref());
        }

        let result = shaper.shape(text).ok().map(|shaped| {
            let id = NEXT_LAYOUT_ID.fetch_add(1, Ordering::Relaxed);
            Self {
                id,
                text: text.to_string(),
                font_size,
                font_family,
                font_weight,
                font_style,
                shaped,
            }
        });

        // Restore
        shaper.set_font_size(old_size);
        shaper.set_font_weight(old_weight);
        shaper.set_font_style(old_style);
        if font_family.is_some() {
            shaper.set_font_family(old_family.as_deref());
        }
        result
    }
}
```

- [ ] **Step 2: 注册模块**

```rust
// crates/ui/src/core/mod.rs — 在末尾添加：
pub mod text_layout;
```

- [ ] **Step 3: 编译验证**

Run: `cargo build -p ui 2>&1 | head -20`
Expected: 编译成功

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/core/text_layout.rs crates/ui/src/core/mod.rs
git commit -m "feat(ui): add UiTextLayout — pure harfbuzz shape data, no GPU dep"
```

---

### Task 2: DrawCmd::TextLayout — 替换 DrawCmd::Text

**Files:**
- Modify: `crates/ui/src/core/paint.rs` (lines 1-200)

- [ ] **Step 1: 替换 DrawCmd::Text 为 TextLayout**

```rust
// crates/ui/src/core/paint.rs
// 修改 DrawCmd 枚举，替换 Text 变体：

use crate::core::geom::Rect;
use crate::core::text_layout::UiTextLayout;
use shaping::{Weight, Style};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum DrawCmd {
    FillRect {
        rect: Rect,
        color: [f32; 4],
        radius: f32,
    },
    StrokeRect {
        rect: Rect,
        color: [f32; 4],
        radius: f32,
        line_width: f32,
    },
    /// 预 shape 的文本布局 — 替代原来的 Text 变体。
    /// 携带 harfbuzz 结果和绘制参数，app 层 drain 时做 atlas + emit。
    TextLayout {
        layout: Arc<UiTextLayout>,
        x: f32,
        y_baseline: f32,
        color: [f32; 4],
    },
    FillTriangle {
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        color: [f32; 4],
    },
    PushClip(Rect),
    PopClip,
}
```

- [ ] **Step 2: 添加 DrawList::text_layout() 方法，替换 text() 和 text_styled()**

```rust
// crates/ui/src/core/paint.rs — 在 impl DrawList 中：

    /// 使用预 shape 的 UiTextLayout 绘制文本。
    /// 替代 text() 和 text_styled() — widget 需先构建 UiTextLayout。
    pub fn text_layout(
        &mut self,
        layout: Arc<UiTextLayout>,
        x: f32,
        y_baseline: f32,
        color: [f32; 4],
    ) {
        self.cmds.push(DrawCmd::TextLayout {
            layout,
            x: x + self.offset.0,
            y_baseline: y_baseline + self.offset.1,
            color,
        });
    }
```

删除 `text()` 和 `text_styled()` 方法及 `font_family`、`font_weight`、`font_style` 字段（这些现在在 `UiTextLayout` 中）。

- [ ] **Step 3: 删除 DrawList 上的字体字段**

```rust
// DrawList struct 变为：
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
    pub offset: (f32, f32),
}
```

- [ ] **Step 4: 更新所有测试**

替换测试中所有 `dl.text()` 和 `dl.text_styled()` 调用为 `dl.text_layout()`。需要先创建 `UiTextLayout`（需要 `Shaper`），对于纯形状测试可跳过文本测试或使用简化的 mock。

```rust
// 示例：更新 text_command_carries_baseline_not_top 测试
#[test]
fn text_layout_command_carries_baseline_not_top() {
    // Skip — UiTextLayout requires Shaper; tested in integration
}
```

删除依赖旧 `DrawCmd::Text` 的测试，新增：
```rust
#[test]
fn text_layout_preserves_color() {
    // 验证 DrawCmd::TextLayout 存储 color
    let layout = Arc::new(unsafe { std::mem::zeroed() }); // placeholder
    let mut dl = DrawList::new();
    dl.text_layout(layout, 10.0, 20.0, [0.2, 0.4, 0.6, 0.8]);
    match &dl.cmds[0] {
        DrawCmd::TextLayout { color, .. } => assert_eq!(*color, [0.2, 0.4, 0.6, 0.8]),
        _ => panic!("expected TextLayout"),
    }
}
```

- [ ] **Step 5: 编译验证**

Run: `cargo build -p ui 2>&1 | head -20`
Expected: 编译成功（可能有 unused import warning）

- [ ] **Step 6: 运行 UI 测试**

Run: `cargo test -p ui 2>&1 | tail -20`
Expected: 所有测试 PASS

- [ ] **Step 7: Commit**

```bash
git add crates/ui/src/core/paint.rs crates/ui/src/core/mod.rs
git commit -m "feat(ui): replace DrawCmd::Text with DrawCmd::TextLayout carrying pre-shaped data"
```

---

### Task 3: LaidOutLine 持 ShapedRun + UiTextLayout

**Files:**
- Modify: `crates/markdown/src/layout.rs`
- Modify: `crates/markdown/Cargo.toml` (if `ui` dep not already present)

- [ ] **Step 1: LaidOutLine 新增字段**

```rust
// crates/markdown/src/layout.rs — 在 LaidOutLine struct 中添加：

pub struct LaidOutLine {
    pub text: String,
    pub rect: Rect,
    pub font_size: f32,
    pub is_code: bool,
    pub color_override: Option<[f32; 4]>,
    pub styles: Vec<StyleSpan>,
    pub style_segments: Vec<StyleSegment>,
    /// Harfbuzz shape 结果（layout 阶段产出，render 阶段消费）
    pub shaped: Option<shaping::ShapedRun>,
    /// 预构建的 TextLayout（layout 阶段创建，跨帧复用，ID 稳定）。
    /// render 阶段直接取 Arc 传给 DrawCmd，无需重建。
    pub text_layout: Option<Arc<ui::core::text_layout::UiTextLayout>>,
}
```

- [ ] **Step 2: Shape 并构建 UiTextLayout — layout_text_block**

在 `layout_text_block()` 中，对整行文本 shape，同时构建 `UiTextLayout`（ID 在此时分配，跨帧稳定）：

```rust
// 在 layout_text_block() 中，创建 LaidOutLine 之前：

use ui::core::text_layout::UiTextLayout;

let (line_shape, line_text_layout) = if let Some(ref mut s) = shaper {
    let old = s.font_size();
    s.set_font_size(font_size);
    let shaped = s.shape(&w).ok();
    s.set_font_size(old);
    let layout = shaped.as_ref().map(|shaped_run| {
        Arc::new(UiTextLayout::from_shaped(
            &w,
            font_size,
            None,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            shaped_run.clone(),
        ))
    });
    (shaped, layout)
} else {
    (None, None)
};

laid_out_lines.push(LaidOutLine {
    text: w.clone(),
    rect: Rect::new(ctx.indent, ly, ctx.available_width(), line_h),
    font_size,
    is_code: false,
    color_override: Some(/* ... existing color logic ... */),
    styles: seg_styles.clone(),
    style_segments: compute_style_segments(...),
    shaped: line_shape,
    text_layout: line_text_layout,  // 跨帧复用，ID 稳定
});
```

关键：`UiTextLayout` 在 layout 时构建**一次**，ID 在此时分配。全生命周期内不变。render 阶段只传递 `Arc`，不重建。

- [ ] **Step 3: 对 layout_line_with_styles 做同样处理**

在 `layout_line_with_styles()` 中保存每行的 shape 结果。

- [ ] **Step 4: CodeBlock 行也保存**

对代码块行（`LaidOutBlockKind::CodeBlock`），在创建 `LaidOutLine` 时 shape 并保存。

- [ ] **Step 5: 编译和测试**

Run: `cargo test -p edit_plus_markdown 2>&1 | tail -20`
Expected: 所有测试 PASS

- [ ] **Step 6: Commit**

```bash
git add crates/markdown/src/layout.rs
git commit -m "feat(md): store ShapedRun in LaidOutLine during layout phase"
```

---

### Task 4: 预览 RenderCache + emit 路径

**Files:**
- Modify: `crates/app/src/md_preview.rs`
- Modify: `crates/app/src/paint_backend.rs`
- Modify: `crates/markdown/src/render.rs`
- Modify: `crates/app/src/render_cache.rs`

- [ ] **Step 1: RenderCache 支持 u64 key**

```rust
// crates/app/src/render_cache.rs — 新增 hash-keyed 方法：

/// 预览专用缓存：key = UiTextLayout.id (u64)，layout 时分配，跨帧稳定
pub struct PreviewRenderCache {
    cache: LruCache<u64, CachedLine>,
}

impl PreviewRenderCache {
    pub fn new() -> Self {
        Self { cache: LruCache::new(MAX_CACHED_LINES) }
    }

    pub fn get(&self, key: u64) -> Option<&CachedLine> {
        self.cache.peek(&key)
    }

    pub fn insert(&mut self, key: u64, line: CachedLine) {
        self.cache.insert(key, line);
    }

    pub fn invalidate_stale_atlas(&mut self, current_generation: u64) {
        let stale: Vec<u64> = self
            .cache.iter()
            .filter(|(_, v)| v.atlas_generation != current_generation)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            self.cache.remove(&k);
        }
    }

    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }
}
```

- [ ] **Step 2: MarkdownPreview 持有 PreviewRenderCache**

```rust
// crates/app/src/md_preview.rs

use crate::render_cache::{CachedLine, GlyphInstance, PreviewRenderCache};

pub(crate) struct MarkdownPreview {
    // ... 现有字段保留 ...
    /// 行级渲染缓存（layout.id → CachedLine）
    render_cache: PreviewRenderCache,
}

impl MarkdownPreview {
    pub fn new() -> Self {
        Self {
            // ... existing ...
            render_cache: PreviewRenderCache::new(),
        }
    }
}
```

- [ ] **Step 3: render() 改为产生 DrawCmd::TextLayout**

修改 `MarkdownPreview::render()` — 不再调用 `render_doc_with_offset()` 产生 `DrawCmd::Text`，改为：

```rust
// 在 render() 中：
let mut dl = DrawList::new();

// 遍历可见 LaidOutLine，对每个：
//   如果 shaped 存在 → 构建 Arc<UiTextLayout> → dl.text_layout()
//   如果 shaped 不存在 → 跳过（不应该发生）

// 非文本形状（FillRect/StrokeRect/Clip）仍通过现有的 render 函数产生
```

实际上需要一个新函数 `render_doc_with_offset_v2()` 或修改现有 `render_line_with_offset()` 来产 `TextLayout` 命令而非 `Text`。

- [ ] **Step 4: 修改 render_line_with_offset**

在 `crates/markdown/src/render.rs` 中，修改 `render_line_with_offset()`：

```rust
fn render_line_with_offset(
    line: &LaidOutLine,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    ox: f32,
    oy: f32,
) {
    let x = line.rect.x + ox;
    let y_baseline = line.rect.y - scroll_y + oy + line.rect.h * ui::constants::BASELINE_RATIO;
    let color = line.color_override.unwrap_or(style.text_color);

    // 直接传递 layout 时构建的 Arc，不重建。ID 稳定，cache 可命中。
    if let Some(ref layout) = line.text_layout {
        dl.text_layout(layout.clone(), x, y_baseline, color);
    }
    // 非文本装饰（underline, strikethrough）仍走 FillRect
}
```

- [ ] **Step 5: drain() 处理 DrawCmd::TextLayout**

```rust
// crates/app/src/paint_backend.rs

use ui::core::text_layout::UiTextLayout;

// 在 drain() 的 match 中添加：
DrawCmd::TextLayout { layout, x, y_baseline, color } => {
    let (Some(text_state), Some(gpu)) = (text.as_deref_mut(), gpu) else {
        continue;
    };
    // layout.id 在 layout 时分配，跨帧稳定 → 缓存可命中
    let cache_key = layout.id;

    // 尝试从预览 cache 获取
    let cached = text_state.preview_cache.get(cache_key);

    if let Some(cached_line) = cached {
        // Cache hit — 从 GlyphInstance 直接发射（类似编辑器路径）
        let verts = emit_from_instances(
            &cached_line.instances, x, y_baseline,
            sw, sh, color, &clip_stack,
        );
        vertices.extend(verts);
    } else {
        // Cache miss — rasterize from shaped data
        let mut instances = Vec::new();
        let mut x_cursor = x;

        for cluster in &layout.shaped.clusters {
            let advance = cluster.advance.max(1.0);
            if layout.text.as_bytes().get(cluster.byte_range.clone())
                .is_some_and(|bytes| bytes.iter().all(|&b| b == b' ' || b == b'\t'))
            {
                x_cursor += advance;
                continue;
            }

            let (int_x, phase) = render::split_subpixel(x_cursor);
            if let Some(slot) = crate::text_rasterize::resolve_glyph(
                cluster.font_id, cluster.glyph_id as u16, layout.font_size, phase,
                &mut text_state.shaper, &mut text_state.atlas,
                &text_state.atlas_texture, &gpu.ctx.queue,
            ) {
                let aw = crate::render_state::ATLAS_SIZE as f32;
                let ah = crate::render_state::ATLAS_SIZE as f32;
                instances.push(GlyphInstance {
                    x: int_x,
                    y: 0.0,
                    bearing_x: slot.bearing_x,
                    bearing_y: slot.bearing_y,
                    width: slot.width as f32,
                    height: slot.height as f32,
                    uv: [slot.x as f32 / aw, slot.y as f32 / ah,
                         (slot.x + slot.width) as f32 / aw,
                         (slot.y + slot.height) as f32 / ah],
                    atlas_page: slot.page,
                    highlight_kind: 0, // preview uses color, not highlight_kind
                });
            }
            x_cursor += advance;
        }

        // 构建并缓存 CachedLine
        let cluster_data: Vec<_> = layout.shaped.clusters.iter()
            .map(|c| (c.byte_range.start, c.byte_range.end, c.advance.max(1.0)))
            .collect();

        let cached_line = CachedLine {
            instances,
            line_number_glyphs: vec![],
            atlas_generation: text_state.atlas.generation(),
            visual_line_count: 1,
            content_hash: cache_key,  // 复用 layout.id 作为 content_hash
            visual_lines: vec![(0, layout.shaped.clusters.len(), x_cursor - x)],
            visual_line_instance_starts: vec![0],
            cluster_data,
            subset_start: 0,
        };

            // 发射顶点（使用 cached_line 中刚构建的 instances）
        let verts = emit_from_instances(
            &cached_line.instances, x, y_baseline, layout.font_size,
            sw, sh, color, &clip_stack,
        );
        vertices.extend(verts);

        text_state.preview_cache.insert(cache_key, cached_line);
    }
}

/// 从 GlyphInstance 列表直接发射 NDC 顶点（用于预览单行，无需 CachedLine 包装）。
fn emit_from_instances(
    instances: &[GlyphInstance],
    origin_x: f32,
    baseline_y: f32,
    font_size: f32,
    screen_w: f32,
    screen_h: f32,
    color: [f32; 4],
    clip_stack: &[Rect],
) -> Vec<render::GlyphVertex> {
    let mut verts = Vec::with_capacity(instances.len() * 6);
    for inst in instances {
        let px = (origin_x + inst.x + inst.bearing_x).round();
        let py = (baseline_y - inst.bearing_y).round();
        let c_rect = apply_clip(clip_stack, ui::core::Rect::new(px, py, inst.width, inst.height));
        if c_rect.w <= 0.0 || c_rect.h <= 0.0 {
            continue;
        }
        let left = c_rect.x / screen_w * 2.0 - 1.0;
        let top = 1.0 - c_rect.y / screen_h * 2.0;
        let right = (c_rect.x + c_rect.w) / screen_w * 2.0 - 1.0;
        let bottom = 1.0 - (c_rect.y + c_rect.h) / screen_h * 2.0;
        let (ul, ut, ur, ub) = (inst.uv[0], inst.uv[1], inst.uv[2], inst.uv[3]);
        verts.push(render::GlyphVertex { position: [left, top], tex_coords: [ul, ut], color });
        verts.push(render::GlyphVertex { position: [right, top], tex_coords: [ur, ut], color });
        verts.push(render::GlyphVertex { position: [left, bottom], tex_coords: [ul, ub], color });
        verts.push(render::GlyphVertex { position: [right, top], tex_coords: [ur, ut], color });
        verts.push(render::GlyphVertex { position: [right, bottom], tex_coords: [ur, ub], color });
        verts.push(render::GlyphVertex { position: [left, bottom], tex_coords: [ul, ub], color });
    }
    verts
}
```

删除旧的 `DrawCmd::Text` 分支。

- [ ] **Step 6: 在 TextState 中新增 preview_cache**

```rust
// crates/app/src/render_state.rs
use crate::render_cache::PreviewRenderCache;

pub struct TextState {
    // ... existing fields ...
    pub preview_cache: PreviewRenderCache,
}
```

- [ ] **Step 7: 编译和测试**

Run: `cargo build 2>&1 | tail -30`
Expected: 编译成功

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/md_preview.rs crates/app/src/paint_backend.rs \
        crates/markdown/src/render.rs crates/app/src/render_cache.rs \
        crates/app/src/render_state.rs
git commit -m "feat(preview): Replace emit_text path with CachedLine + PreviewRenderCache"
```

---

### Task 5: 删除 emit_text 和旧代码

**Files:**
- Modify: `crates/app/src/paint_backend.rs`
- Modify: `crates/ui/src/core/paint.rs`

- [ ] **Step 1: 删除 emit_text 函数**

```bash
# 删除 crates/app/src/paint_backend.rs 中整个 emit_text() 函数（第 95-229 行）
```

- [ ] **Step 2: 删除 DrawCmd::Text 变体和旧方法**

- 删除 `DrawList::text()` 方法（如果尚未删除）
- 删除 `DrawList::text_styled()` 方法（如果尚未删除）
- 删除 `DrawList` 的 `font_family`、`font_weight`、`font_style` 字段
- 清理测试中所有旧引用

- [ ] **Step 3: 清理 TextState.shape_cache**

删除 `TextState` 中旧 `emit_text` 使用的 `shape_cache: HashMap<(u64, u32, u16, u8), ShapedRun>` 字段（被 `PreviewRenderCache` 替代）。

- [ ] **Step 4: 编译验证**

Run: `cargo build 2>&1 | tail -20`
Expected: 编译成功，无 unused 警告

- [ ] **Step 5: 运行全量测试**

Run: `cargo test 2>&1 | tail -40`
Expected: 所有测试 PASS

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/paint_backend.rs crates/ui/src/core/paint.rs crates/app/src/render_state.rs
git commit -m "chore: remove emit_text() and DrawCmd::Text — dead code after TextLayout migration"
```

---

### Task 6: UI Widget 迁移（独立 PR）

**Files:**
- Modify: `crates/ui/src/widgets/status_bar.rs`
- Modify: `crates/ui/src/widgets/text_box.rs`
- Modify: `crates/ui/src/widgets/tooltip.rs`
- Modify: `crates/ui/src/widgets/list.rs`

- [ ] **Step 1: 为需要绘制文本的 Widget 添加 text_layout 字段**

```rust
// 每个 widget 模式：
pub struct SomeWidget {
    // ... existing fields ...
    label_layout: Option<Arc<UiTextLayout>>,
    label_text: String,  // for dirty detection
}
```

- [ ] **Step 2: Widget paint 时构建或复用 TextLayout**

```rust
// 在 paint() 中：
if self.label_text != new_text || self.label_layout.is_none() {
    self.label_layout = UiTextLayout::new(
        &new_text, font_size, None, Weight::NORMAL, Style::Normal, shaper
    ).map(Arc::new);
    self.label_text = new_text;
}
if let Some(ref layout) = self.label_layout {
    ctx.list.text_layout(layout.clone(), x, y_baseline, color);
}
```

- [ ] **Step 3: 逐个迁移 widget**

StatusBar、TextBox、Tooltip、List — 每个 widget 的迁移类似，将 `ctx.list.text()` 或 `ctx.list.text_styled()` 替换为 `ctx.list.text_layout()`。

- [ ] **Step 4: 编译和测试**

Run: `cargo build 2>&1 | tail -20`
Run: `cargo test 2>&1 | tail -20`
Expected: 编译成功，所有测试 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/
git commit -m "feat(ui): migrate widgets to UiTextLayout + DrawCmd::TextLayout"
```
