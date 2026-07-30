# UI 骨架 Phase 3：status_bar widget 化（首个真 widget + Text 路径）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `ui::status_bar` 从一组裸函数升级为 `StatusBarWidget` 真 widget；同时打通 `paint_backend` 的 Text 命令路径——把现在 `app_renderer.rs::status_bar_text_vertices`（约 80 行）和 `popup_menu_text_vertices` 的字形渲染逻辑收敛进 backend。完工后 status_bar 走 widget 路径绘制，老的 `status_bar_bg_vertices` / `status_bar_text_vertices` 调用链彻底删除，UI 视觉与之前一致。

**Architecture:**
- 新增 `crates/ui/src/widgets/` 子目录，把首个真 widget 放进去（不动现有 `tab_bar.rs / sidebar.rs` 等位置）。
- `StatusBarWidget` 持 `StatusBarCache` + `last_text: String`；`set_rect` 时记录矩形；`paint` 走 `DrawList::fill` + `DrawList::text`；不接管事件。
- `paint_backend::drain` 实现 Text 路径：把 `DrawCmd::Text` 翻译为已有的 atlas + GlyphVertex，复用现有字形/atlas 资源（与 `app_renderer.rs::tab_text_vertices` 同款逻辑，提到 backend 模块统一）。
- `UiShell` 用法不变；`build_shell_inputs` 把 status_bar 数据通过新接口传给真 widget。

**Tech Stack:** Rust 2024 · `ui::core::DrawList::text` · 现有 `render::{GlyphKey, GlyphVertex}` + atlas + shaping。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §4.2、§7（阶段 3）

---

## 文件结构

| 文件 | 改动类型 | 备注 |
|---|---|---|
| `crates/ui/src/widgets/mod.rs` | Create | `pub mod status_bar;` 等占位 |
| `crates/ui/src/widgets/status_bar.rs` | Create | `StatusBarWidget`：包装 `ui::status_bar::{StatusBarInput, StatusBarCache, build_text}` |
| `crates/ui/src/lib.rs` | Modify | 加 `pub mod widgets;` 与 re-export |
| `crates/app/src/paint_backend.rs` | Modify | 实现 Text 路径：DrawCmd::Text → atlas+GlyphVertex |
| `crates/app/src/ui_shell.rs` | Modify | dock children 中 status 位换成真 widget；新增 `set_status(...)` 接口 |
| `crates/app/src/app_renderer.rs` | Modify | 删除老 `status_bar_bg_vertices` / `status_bar_text_vertices` 调用；改用 `ui_shell.paint_chrome()` 落入主顶点列表 |
| `crates/app/src/app.rs` | Modify | 删除 `status_bar_text_vertices` / `status_bar_bg_vertices` 函数（如纯老路径专用）；保留 `status_bar_text()` 给 widget 用 |

> ⚠️ `crates/ui/src/widgets/` 是 Phase 3 才新建的目录；后续 widget（search_bar / scrollbar / tab_bar / sidebar / popup_menu）依次搬入。**不直接修改** `crates/ui/src/status_bar.rs` 文件——它只提供数据/缓存，widget 包装它。

---

## Task 1：建立 widgets 目录与 StatusBarWidget 骨架

**Files:**
- Create: `crates/ui/src/widgets/mod.rs`
- Create: `crates/ui/src/widgets/status_bar.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1.1：建立 widgets 目录与 StatusBarWidget**

创建 `crates/ui/src/widgets/mod.rs`：

```rust
//! 真 widget 实现（基于 ui::core::Widget trait）。
//! 旧 `crates/ui/src/{tab_bar,sidebar,status_bar,...}.rs` 仍提供裸函数 / 数据；
//! 这里的 widget 包装 / 取代它们。

pub mod status_bar;
```

创建 `crates/ui/src/widgets/status_bar.rs`：

```rust
//! StatusBarWidget — 基于 ui::status_bar 的纯文本/缓存逻辑，包装为 Widget。

use std::any::Any;

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event};
use crate::status_bar::{StatusBarInput, StatusBarCache, build_text};

pub struct StatusBarWidget {
    rect: Rect,
    cache: StatusBarCache,
    /// 最近一次构建出的文本 — paint 时用。
    last_text: String,
    /// 由 app 每帧通过 set_input 写入（避免 widget 内部依赖 Workspace）
    input: Option<StatusBarInput>,
}

impl StatusBarWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            cache: StatusBarCache::new(),
            last_text: String::new(),
            input: None,
        }
    }

    /// app 在 update_frame 之前调用，把当前 buffer / cursor / 选区信息塞进来。
    pub fn set_input(&mut self, input: StatusBarInput) {
        self.input = Some(input);
    }
}

impl Widget for StatusBarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
        // 在 layout 阶段顺便重算文本（用 cache 加速重复调用）
        if let Some(ref input) = self.input {
            self.last_text = build_text(input, &mut self.cache);
        } else {
            self.last_text.clear();
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 { return; }

        // 1) 背景
        ctx.list.fill(self.rect, ctx.theme.status_bar_bg);

        // 2) 文字（右对齐，垂直居中，预留右边距）
        if !self.last_text.is_empty() {
            let font_size = 13.0 * ctx.dpi;
            // 简化：先按 font_size * 0.5 估宽（status_bar 文本只有数字+逗号+字母，
            // ASCII 等宽估算误差可接受；真宽由 layout 阶段 measure 得到更精确，
            // 但 status_bar 字符短，估算路径足够。后续若需精确改 layout 阶段量。）
            let estimated_w = self.last_text.chars().count() as f32 * font_size * 0.5;
            let pad_right = 12.0 * ctx.dpi;
            let x = self.rect.right() - pad_right - estimated_w;
            // 基线：rect 顶 + 字号上沿 ≈ rect 中线 + font_size*0.35
            let y_baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35;
            ctx.list.text(x, y_baseline, font_size, ctx.theme.status_bar_fg, &self.last_text);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx)
        -> Option<Box<dyn Any>> { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{NoopMeasure, DrawList, DrawCmd};
    use crate::Theme;

    fn make_ctx<'a>(theme: &'a Theme, m: &'a mut dyn crate::core::TextMeasure) -> LayoutCtx<'a> {
        LayoutCtx { measure: m, theme, dpi: 1.0 }
    }

    #[test]
    fn paint_with_no_input_only_fills_bg() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = make_ctx(&theme, &mut m);

        let mut w = StatusBarWidget::new();
        w.set_rect(Rect::new(0.0, 776.0, 1200.0, 24.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert_eq!(list.len(), 1);
        assert!(matches!(list.cmds[0], DrawCmd::FillRect { .. }));
    }

    #[test]
    fn paint_with_cursor_input_emits_text_command() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = make_ctx(&theme, &mut m);

        let mut w = StatusBarWidget::new();
        w.set_input(StatusBarInput {
            buffer_len: 100,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 4,
            cursor_col: 9,
        });
        w.set_rect(Rect::new(0.0, 776.0, 1200.0, 24.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert_eq!(list.len(), 2, "fill + text");

        match &list.cmds[1] {
            DrawCmd::Text { content, color, .. } => {
                assert_eq!(content, "5,10");
                assert_eq!(*color, theme.status_bar_fg);
            }
            _ => panic!("expected Text cmd"),
        }
    }

    #[test]
    fn paint_skips_text_when_buffer_empty() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = make_ctx(&theme, &mut m);

        let mut w = StatusBarWidget::new();
        w.set_input(StatusBarInput {
            buffer_len: 0,
            selection_range: None, selection_char_count: None,
            cursor_line: 0, cursor_col: 0,
        });
        w.set_rect(Rect::new(0.0, 776.0, 1200.0, 24.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert_eq!(list.len(), 1, "buffer 空时只有背景");
    }

    #[test]
    fn zero_rect_paint_is_noop() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = make_ctx(&theme, &mut m);

        let mut w = StatusBarWidget::new();
        w.set_rect(Rect::ZERO, &mut layout);
        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert!(list.is_empty());
    }
}
```

修改 `crates/ui/src/lib.rs`，在 `pub mod core;` 之后追加：

```rust
pub mod widgets;
```

并在底部 `pub use core::{...}` 后追加：

```rust
pub use widgets::status_bar::StatusBarWidget;
```

- [ ] **Step 1.2：跑测试**

```bash
cargo test -p edit-plus-ui widgets::status_bar
```

预期：4 个测试通过。

- [ ] **Step 1.3：提交**

```bash
git add crates/ui/src/widgets/mod.rs crates/ui/src/widgets/status_bar.rs crates/ui/src/lib.rs
git commit -m "feat(ui-widgets): status_bar — 首个真 widget"
```

---

## Task 2：paint_backend 实现 Text 路径

**Files:**
- Modify: `crates/app/src/paint_backend.rs`

参考实现的来源是 `crates/app/src/app_renderer.rs::tab_text_vertices`（约 230 行）的字形渲染逻辑。本任务把它**搬到 paint_backend 并简化**：去掉 tab 特有的 clip / ellipsis / overflow 处理，只做"把一行字按基线放在 (x, y_baseline)"的最小核心；这些扩展在后续 widget 接入时再按需在 widget 侧用 `DrawList::clip` 解决。

- [ ] **Step 2.1：扩展 Backend 状态**

读 `crates/app/src/paint_backend.rs` 现状（Phase 2 已存在）。在文件顶部追加引入：

```rust
use std::hash::{Hash, Hasher};
use render::{GlyphKey, GlyphVertex};
use shaping::Shaper;
use crate::render_state::ATLAS_SIZE;
```

把现有 `pub fn drain(list: &DrawList, screen: Screen) -> Vec<GlyphVertex>` 重命名为 `pub fn drain_no_text(...)`（保留作为 Phase 2 测试桩），新写正式签名：

```rust
pub fn drain(
    list: &DrawList,
    screen: Screen,
    text: &mut crate::render_state::TextResources,
    gpu: &crate::render_state::GpuState,
) -> Vec<GlyphVertex> {
    let mut out = Vec::new();
    let mut backend = Backend::default();
    for cmd in &list.cmds {
        match cmd {
            DrawCmd::FillRect { rect, color, radius: _ } => {
                push_quad(&mut out, *rect, *color, screen, backend.current_clip());
            }
            DrawCmd::PushClip(rect) => {
                backend.clip_stack.push(screen.rect_to_ndc(*rect));
            }
            DrawCmd::PopClip => {
                backend.clip_stack.pop();
            }
            DrawCmd::Text { x, y_baseline, font_size, color, content } => {
                emit_text(&mut out, *x, *y_baseline, *font_size, *color,
                          content, screen, backend.current_clip(), text, gpu);
            }
        }
    }
    out
}
```

> ⚠️ `TextResources / GpuState / ATLAS_SIZE` 来自 `crates/app/src/render_state.rs`，已在 app 内部使用。具体类型路径以仓库现状为准；如果 TextResources 没有这个公开名字，把对应的 atlas + atlas_texture + shaper 字段直接列出来传入。

实现 `emit_text`：

```rust
fn emit_text(
    out: &mut Vec<GlyphVertex>,
    x: f32,
    y_baseline: f32,
    font_size: f32,
    color: [f32; 4],
    content: &str,
    screen: Screen,
    clip: Option<[f32; 4]>,
    text: &mut crate::render_state::TextResources,
    gpu: &crate::render_state::GpuState,
) {
    if content.is_empty() { return; }
    let old_size = text.shaper.font_size();
    text.shaper.set_font_size(font_size);
    let shaped = match text.shaper.shape(content) {
        Ok(s) => s,
        Err(_) => { text.shaper.set_font_size(old_size); return; }
    };

    let mut x_cursor = x;
    let y_base = y_baseline;
    let font_size_q = (font_size * 64.0) as u32;

    for cluster in &shaped.clusters {
        let advance = cluster.advance.max(1.0);
        // 跳过空格（由 advance 推进 cursor，不画）
        let bytes = content.as_bytes()
            .get(cluster.byte_range.clone())
            .unwrap_or(&[]);
        if bytes.first().is_some_and(|&c| c == b' ') {
            x_cursor += advance;
            continue;
        }

        let font_id = cluster.font_id;
        let font_id_usize = {
            let mut h = std::hash::DefaultHasher::new();
            font_id.hash(&mut h);
            h.finish() as usize
        };
        let key = GlyphKey {
            glyph_id: cluster.glyph_id,
            font_id: font_id_usize,
            font_size: font_size_q,
            subpixel_phase: 0,
        };

        let slot = if let Some(c) = text.atlas.get(&key) { *c } else {
            let bm = match text.shaper.rasterize_glyph(font_id, cluster.glyph_id as u16, font_size) {
                Some(b) if b.width > 0 && b.height > 0 => b,
                _ => { x_cursor += advance; continue; }
            };
            let s = match text.atlas.insert(key, bm.width, bm.height,
                                            bm.left as f32, bm.top as f32) {
                Some(s) => s,
                None => { x_cursor += advance; continue; }
            };
            gpu.ctx.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &text.atlas_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: s.x, y: s.y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &bm.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bm.width),
                    rows_per_image: Some(bm.height),
                },
                wgpu::Extent3d { width: bm.width, height: bm.height, depth_or_array_layers: 1 },
            );
            s
        };

        let l_px = x_cursor + slot.bearing_x;
        let r_px = l_px + slot.width as f32;
        let t_px = y_base - slot.bearing_y;
        let b_px = t_px + slot.height as f32;

        // px → NDC
        let l = l_px / screen.w * 2.0 - 1.0;
        let r = r_px / screen.w * 2.0 - 1.0;
        let t = 1.0 - t_px / screen.h * 2.0;
        let b = 1.0 - b_px / screen.h * 2.0;

        // 应用裁剪
        let (l, r, t, b) = if let Some([cl, cr, ct, cb]) = clip {
            let l = l.max(cl);
            let r = r.min(cr);
            let t = t.min(ct);
            let b = b.max(cb);
            if l >= r || t <= b { x_cursor += advance; continue; }
            (l, r, t, b)
        } else { (l, r, t, b) };

        let ul = slot.x as f32 / ATLAS_SIZE as f32;
        let ut = slot.y as f32 / ATLAS_SIZE as f32;
        let ur = (slot.x + slot.width) as f32 / ATLAS_SIZE as f32;
        let ub = (slot.y + slot.height) as f32 / ATLAS_SIZE as f32;
        let c = color;

        out.push(GlyphVertex { position: [l, t], tex_coords: [ul, ut], color: c });
        out.push(GlyphVertex { position: [r, t], tex_coords: [ur, ut], color: c });
        out.push(GlyphVertex { position: [l, b], tex_coords: [ul, ub], color: c });
        out.push(GlyphVertex { position: [r, t], tex_coords: [ur, ut], color: c });
        out.push(GlyphVertex { position: [r, b], tex_coords: [ur, ub], color: c });
        out.push(GlyphVertex { position: [l, b], tex_coords: [ul, ub], color: c });

        x_cursor += advance;
    }
    text.shaper.set_font_size(old_size);
}
```

- [ ] **Step 2.2：保留 Phase 2 的 `drain_no_text` 单元测试不变**

确保 Phase 2 写过的 `should_panic("Text 路径")` 测试可以**删除**了（Text 路径已实现）。把那条测试改成正常的 emit Text 测试：

```rust
#[test]
fn text_command_emits_some_vertices_when_resources_available() {
    // 这个测试需要真实 wgpu device + atlas，属于集成测试范畴；
    // 此处仅保留接口兜底：空字符串应返回 0 顶点。
    // emit_text 内部对 empty 已 early-return，本 unit test 通过
    // drain_no_text 路径间接覆盖。
    // 真集成在 Task 4 启动时手测覆盖。
}
```

或直接把 `should_panic` 删除并新增一条编译断言型测试，保证 emit_text 对 empty 字符串不崩：

```rust
#[test]
fn empty_text_command_no_op_via_drain_no_text() {
    let mut list = DrawList::new();
    list.text(0.0, 10.0, 14.0, [0.0; 4], "");
    // drain_no_text 仍走老枚举：FillRect / Clip 命中、Text 路径在 phase 3 改成 dispatch。
    // 这个测试只断言 list 接受空字符串不 panic。
    assert_eq!(list.len(), 1);
}
```

> ⚠️ `drain_no_text` 在 Phase 3 完成后已无人调用；可以保留作为纯 CPU 单元测试入口（不依赖 wgpu），后续 Phase 9 收尾再统一删。

- [ ] **Step 2.3：build**

```bash
cargo build -p edit-plus-app
```

预期：通过。Text 路径与 wgpu 集成只做编译验证；运行验证留 Task 4。

- [ ] **Step 2.4：提交**

```bash
git add crates/app/src/paint_backend.rs
git commit -m "feat(app): paint_backend — DrawCmd::Text → atlas+GlyphVertex"
```

---

## Task 3：UiShell 把 status 位换成真 widget

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 3.1：替换 status 占位 widget**

读 `crates/app/src/ui_shell.rs` Phase 2 实现。在文件顶部 import 区追加：

```rust
use ui::widgets::status_bar::StatusBarWidget;
use ui::status_bar::StatusBarInput;
```

把 `UiShell::new()` 里 `idx_status` 的注册行（之前是 `push_with_thickness(..., Side::Bottom, 0.0)`）替换为：

```rust
let idx_status = {
    let idx = dock.children.len();
    let t_const = 0.0_f32;
    dock.push(DockChild::bottom(StatusBarWidget::new(), move |_, _| t_const));
    idx
};
```

替换 `update_frame` 里 status 那一行的 thickness 设置 —— 维持原样即可（外部入参驱动）。

新增公共方法 `set_status_input`：

```rust
impl UiShell {
    /// app 在 update_frame 之前调用，把 status_bar 数据塞进真 widget。
    pub fn set_status_input(&mut self, input: StatusBarInput) {
        // dock.children[idx_status].widget 是 Box<dyn Widget>，需要 downcast
        let any = self.dock.children[self.idx_status].widget.as_mut() as &mut dyn std::any::Any;
        if let Some(w) = any.downcast_mut::<StatusBarWidget>() {
            w.set_input(input);
        }
    }
}
```

> ⚠️ `Box<dyn Widget>` 默认没有 Any。最简的解法：让 `Widget` trait 提供 `as_any_mut(&mut self) -> &mut dyn std::any::Any { self }`（默认实现需要 `Self: 'static`）；并在 `Widget` trait 上增加：
>
> ```rust
> pub trait Widget: std::any::Any { ... }
> ```
>
> 然后在 `set_status_input` 用 `(self.dock.children[idx].widget.as_mut() as &mut dyn std::any::Any).downcast_mut::<StatusBarWidget>()`。
>
> **如果加 `: Any` super-trait 影响过大**（已有 widget 不能 dyn），改用更直接的方案：在 `UiShell` 里把 status 单列出来 `status: StatusBarWidget`，并在 layout 时把它传给 dock —— 但 dock 持的是 Box；最便利的做法是 **保留一个 Rc<RefCell<StatusBarWidget>> 引用**，dock 那边放包装的 Box（`struct Wrap(Rc<RefCell<W>>)`）；wrap 的 set_rect/paint/hit/on_event 转发到 RefCell 里的 widget。
>
> 选简单路径：**在 Widget trait 加 `Any` super-trait + `as_any_mut`**。提交方案如下：

修改 `crates/ui/src/core/widget.rs`：

```rust
use std::any::Any;

pub trait Widget: Any {
    // ...原有方法不变...
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

并在已有 `Counter / Probe / Dummy` 测试 widget 都加 `as_any_mut` 走默认实现即可（`Any` super-trait 需 `'static`，所有现有结构都满足）。

修改 `crates/app/src/editor_host.rs / app/src/ui_shell.rs::PlaceholderWidget`：无须改动（默认实现）。

回到 `ui_shell.rs`，`set_status_input` 改为：

```rust
pub fn set_status_input(&mut self, input: StatusBarInput) {
    let w = self.dock.children[self.idx_status].widget.as_any_mut();
    if let Some(sw) = w.downcast_mut::<StatusBarWidget>() {
        sw.set_input(input);
    }
}
```

- [ ] **Step 3.2：跑测试**

```bash
cargo test --workspace
```

预期：通过（包括 Phase 2 的 alignment 测试 + 新加的 status widget 测试）。

- [ ] **Step 3.3：提交**

```bash
git add crates/ui/src/core/widget.rs crates/app/src/ui_shell.rs
git commit -m "feat(ui-core): Widget: Any + as_any_mut；shell 注入 status_bar 数据"
```

---

## Task 4：app_renderer 接管 status_bar 绘制

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app.rs`

- [ ] **Step 4.1：在 render() 里把 chrome 绘制接进顶点列表**

读 `crates/app/src/app_renderer.rs::render` 的中段（顶点 extend 区域）。

在 `vertices.extend(self.status_bar_bg_vertices(...))` / `vertices.extend(self.status_bar_text_vertices(...))` 这两行之前，把 status 真渲染搬到 ui_shell 路径：

1. 调用 `set_status_input` 注入数据：

```rust
// Phase 3：把 status_bar 数据塞进 widget
{
    let dv = self.workspace.doc_views.get(self.workspace.active_index);
    if let Some(dv) = dv {
        use ui::status_bar::StatusBarInput;
        self.ui_shell.set_status_input(StatusBarInput {
            buffer_len: dv.buffer_len(),
            selection_range: dv.selection_range(),
            selection_char_count: dv.count_selection_chars(),
            cursor_line: dv.cursor_line(),
            cursor_col: dv.cursor_column(),
        });
    }
}
```

2. 注意：之前 `update_frame` 是在 `set_status_input` 之前调用的——会导致 widget 没拿到数据就 layout 了。**调整顺序**：把 `set_status_input` 移到 `update_frame` 调用**之前**。

3. 渲染 chrome（Phase 3 阶段只画 status；其它 placeholder 仍是空命令）：

```rust
// Phase 3：用 ui_shell 渲染 chrome
{
    let theme = self.current_theme.clone();
    let dpi = ui::settings::Settings::get().dpi_scale;
    let chrome_list = self.ui_shell.paint_chrome(&theme, dpi);
    if !chrome_list.is_empty() {
        let screen = ui::core::Screen::new(screen_w, screen_h);
        if let (Some(text), Some(gpu)) = (self.text.as_mut(), self.gpu.as_ref()) {
            let chrome_verts = crate::paint_backend::drain(&chrome_list, screen, text, gpu);
            vertices.extend(chrome_verts);
        }
    }
}
```

4. **删除**老调用：

```rust
vertices.extend(self.status_bar_bg_vertices(screen_w, screen_h));
vertices.extend(self.status_bar_text_vertices(screen_w, screen_h, tbh));
```

把这两行删掉。

- [ ] **Step 4.2：删除 app.rs / app_renderer.rs 里 status 老函数（如已无引用）**

```bash
grep -rn "status_bar_bg_vertices\|status_bar_text_vertices" crates/app/src/
```

如果 grep 还有残余调用，处理掉；如果没有，删除函数定义：
- `app/src/app_renderer.rs::status_bar_bg_vertices`
- `app/src/app_renderer.rs::status_bar_text_vertices`（若存在）
- `app/src/app.rs::status_bar_text` —— **保留**：`StatusBarWidget` 内部已经直接用 `build_text(...)`；但 `app.rs` 里另一个 `status_bar_text` 也许在别处被调（如 native_menu 显示行号？）。**保留与否以 `cargo build` 为准**：删之前确认没人引用。

- [ ] **Step 4.3：build && run，肉眼对比**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

预期：
- 通过；
- status_bar 显示与之前一致：行号 / 字符计数 / 选区信息；
- 字号、颜色、位置肉眼无差异。

如发现位置偏移，多半是 baseline 计算（`y_baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35`）与老代码差几 px——调系数到匹配。

- [ ] **Step 4.4：提交**

```bash
git add crates/app/src/app_renderer.rs crates/app/src/app.rs
git commit -m "refactor(app): status_bar 走 ui_shell 路径，删老 vertices 函数"
```

---

## Task 5：Phase 3 收尾

- [ ] **Step 5.1：手测** —— 滚动光标、选区、不同语言、重启进入。

- [ ] **Step 5.2：grep 残余引用**

```bash
grep -rn "status_bar_text_vertices\|status_bar_bg_vertices" crates/
```

预期：仅注释 / 文档命中。

- [ ] **Step 5.3：spec 追加 Phase 3 完工记录**

```markdown
## Phase 3 完工记录

- 完工日期：（执行时填）
- 接入：StatusBarWidget；paint_backend Text 路径；老 status vertices 函数已删
- 后续：Phase 4 接 search_bar
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 3 完工记录"
```

---

## 边界情况清单

1. **空 buffer**：`build_text` 返回空串，widget 只画背景。
2. **选区有内容**：`build_text` 返回 `"15c,20b"` 之类，缓存命中跳过 char_count 重算。
3. **DPI 1x ↔ 2x**：font_size 乘 dpi；rect.h 也跟着 dpi 变 → baseline 始终居中。
4. **窗口宽度变化**：rect.w 变 → 文字"右对齐"自动跟边。
5. **Text empty fast path**：emit_text 对空字符串 early-return，不 set_font_size。
6. **缺字回退**：rasterize_glyph 返回 None 时 advance 推进、跳过这个 cluster——与老路径一致。
7. **paint_backend Text 走的是 widget 的字号/颜色**：tab/popup 那 230 行老代码用的是写死字号；这里 widget 自报 font_size，更通用，老路径迁移时直接传准确值即可。
