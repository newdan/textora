# UI 骨架 Phase 2：UiShell + EditorHost + paint_backend 骨架

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phase 1 已建好 ui::core；本阶段在 app crate 引入 `ui_shell` + `paint_backend` + `EditorHostWidget`，让 Dock 真接管 layout（不接管 paint），与老渲染路径**双跑、对齐**：dock 算出的 fill rect 必须等于现有的"扣掉 tab/status/sidebar/scrollbar 之后的 editor rect"。这一阶段不删任何老代码、不改一处显示。

**Architecture:**
- `app/src/ui_shell.rs` 持有 `Dock + overlays + chrome 标志位`，每帧调用 `update_frame()` 重算 layout。
- `app/src/paint_backend.rs` 是 `DrawList → Vec<GlyphVertex>` 的纯函数；本阶段先把骨架打通（FillRect 路径），Text 路径留 stub（panic 或返回空）。
- `app/src/editor_host.rs` 提供一个 stub Widget：`set_rect` 记录矩形供 app 读取，`paint` 是空操作。
- 引入一个对齐校验测试 / 调试日志：dock.fill_rect 与老 `screen_h - tbh - status_h - …` 计算得到的矩形一致；上线前肉眼确认。

**Tech Stack:** Rust 2024 · 本地新增 `app::ui_shell / paint_backend / editor_host`，对接 `ui::core::{Dock, DockChild, ...}` 与现有 `App / Workspace / Settings`。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §3、§4.6、§5、§6（A 列前两行）

---

## 文件结构

| 文件 | 职责 | 行数预算 |
|---|---|---|
| `crates/app/src/editor_host.rs` | `EditorHostWidget`：黑盒 widget，只记 rect | ~60 |
| `crates/app/src/paint_backend.rs` | `drain(list, screen, theme, ...) -> Vec<GlyphVertex>`，本阶段只实现 FillRect / Push/PopClip | ~150 |
| `crates/app/src/ui_shell.rs` | `UiShell { dock, overlays, ... }` + `update_frame / paint / dispatch` | ~200 |
| `crates/app/src/lib.rs` | 三个 `pub mod` 行 | +3 |
| `crates/app/src/app.rs` | 持有 `ui_shell: UiShell`；`render()` 起始处调用 `ui_shell.update_frame()`；旧逻辑保持不变 | ~+30 |

---

## Task 1：EditorHostWidget（黑盒 widget）

**Files:**
- Create: `crates/app/src/editor_host.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1.1：实现 + 测试**

创建 `crates/app/src/editor_host.rs`：

```rust
//! 编辑区"黑盒 widget"：只接收 dock 算给的 rect，不做任何渲染、不响应事件。
//! app 主流程读 self.rect 后用旧的 render_pipeline 自己画。

use std::any::Any;
use ui::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event};

#[derive(Default)]
pub struct EditorHostWidget {
    rect: Rect,
}

impl EditorHostWidget {
    pub fn new() -> Self { Self { rect: Rect::ZERO } }
    pub fn rect(&self) -> Rect { self.rect }
}

impl Widget for EditorHostWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, _ctx: &mut PaintCtx) {
        // 故意空：app 仍走旧 render_pipeline 画编辑器。
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx)
        -> Option<Box<dyn Any>>
    {
        // 编辑器输入由 app 直接处理（与 viewport / cursor 紧耦合）；
        // shell 不路由到这里。返回 None 等于"我不接管"。
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::core::{NoopMeasure, DrawList};
    use ui::Theme;

    #[test]
    fn set_rect_records_rect_and_paint_is_noop() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
        let mut w = EditorHostWidget::new();

        w.set_rect(Rect::new(220.0, 32.0, 968.0, 744.0), &mut layout);
        assert_eq!(w.rect(), Rect::new(220.0, 32.0, 968.0, 744.0));

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert_eq!(list.len(), 0, "EditorHostWidget::paint 必须是空操作");
    }

    #[test]
    fn hit_uses_rect_contains() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
        let mut w = EditorHostWidget::new();
        w.set_rect(Rect::new(100.0, 100.0, 50.0, 50.0), &mut layout);

        assert!(w.hit(120.0, 120.0));
        assert!(!w.hit(50.0, 50.0));
    }
}
```

修改 `crates/app/src/lib.rs`，追加：

```rust
pub mod editor_host;
```

- [ ] **Step 1.2：跑测试**

```bash
cargo test -p edit-plus-app editor_host
```

预期：2 个测试通过。

- [ ] **Step 1.3：提交**

```bash
git add crates/app/src/editor_host.rs crates/app/src/lib.rs
git commit -m "feat(app): editor_host — 黑盒 widget 仅记 rect"
```

---

## Task 2：paint_backend 骨架（FillRect + Clip 路径）

**Files:**
- Create: `crates/app/src/paint_backend.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 2.1：写实现 + 测试（仅 FillRect / PushClip / PopClip）**

本阶段只实现 FillRect 与 Push/PopClip。Text 路径留 `unimplemented!`，等 Phase 3 status_bar 接入时再补。

创建 `crates/app/src/paint_backend.rs`：

```rust
//! DrawList → Vec<GlyphVertex> 翻译。
//!
//! Phase 2 只实现 FillRect / PushClip / PopClip。
//! Text 路径在 Phase 3 接入 status_bar 时补。

use render::GlyphVertex;
use ui::core::{DrawCmd, DrawList, Rect, Screen};

/// Backend 状态：clip 栈（NDC 形式：[l, r, t, b]）。
#[derive(Default)]
struct Backend {
    clip_stack: Vec<[f32; 4]>,
}

impl Backend {
    fn current_clip(&self) -> Option<[f32; 4]> {
        self.clip_stack.last().copied()
    }
}

/// 把 DrawList 翻译成 GPU 顶点。
///
/// `screen` — 当前帧屏幕尺寸（px）。
/// 顶点坐标系：NDC（与现有 GlyphVertex 约定一致，y 上正下负）。
pub fn drain(list: &DrawList, screen: Screen) -> Vec<GlyphVertex> {
    let mut out = Vec::new();
    let mut backend = Backend::default();

    for cmd in &list.cmds {
        match cmd {
            DrawCmd::FillRect { rect, color, radius: _ } => {
                // Phase 2：先不实现圆角，统一画直角矩形。
                // 圆角在后续 widget 接入时按需在这里加路径。
                push_quad(&mut out, *rect, *color, screen, backend.current_clip());
            }
            DrawCmd::PushClip(rect) => {
                backend.clip_stack.push(screen.rect_to_ndc(*rect));
            }
            DrawCmd::PopClip => {
                backend.clip_stack.pop();
            }
            DrawCmd::Text { .. } => {
                unimplemented!("Phase 2: Text 路径暂未接入；将在 Phase 3 status_bar 落地");
            }
        }
    }

    out
}

fn push_quad(
    out: &mut Vec<GlyphVertex>,
    rect: Rect,
    color: [f32; 4],
    screen: Screen,
    clip: Option<[f32; 4]>,
) {
    // 应用裁剪（CPU 侧最小可行实现：求交后丢弃完全裁掉的）
    let [l, r, t, b] = screen.rect_to_ndc(rect);
    let (l, r, t, b) = if let Some([cl, cr, ct, cb]) = clip {
        let l = l.max(cl);
        let r = r.min(cr);
        let t = t.min(ct);
        let b = b.max(cb);
        if l >= r || t <= b { return; }
        (l, r, t, b)
    } else {
        (l, r, t, b)
    };

    // 复用现有顶点格式：tex_coords=[0, 0] 走 atlas 中的纯白像素。
    let v = |x: f32, y: f32| GlyphVertex {
        position: [x, y],
        tex_coords: [0.0, 0.0],
        color,
    };
    out.push(v(l, t));
    out.push(v(r, t));
    out.push(v(r, b));
    out.push(v(l, t));
    out.push(v(r, b));
    out.push(v(l, b));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Screen { Screen::new(1000.0, 1000.0) }

    #[test]
    fn fill_rect_emits_six_vertices() {
        let mut list = DrawList::new();
        list.fill(Rect::new(0.0, 0.0, 100.0, 100.0), [1.0, 0.0, 0.0, 1.0]);
        let verts = drain(&list, screen());
        assert_eq!(verts.len(), 6, "1 quad = 2 triangles = 6 vertices");
    }

    #[test]
    fn fillrect_color_is_preserved() {
        let mut list = DrawList::new();
        list.fill(Rect::new(0.0, 0.0, 10.0, 10.0), [0.5, 0.25, 0.75, 1.0]);
        let v = drain(&list, screen());
        for vv in &v {
            assert_eq!(vv.color, [0.5, 0.25, 0.75, 1.0]);
        }
    }

    #[test]
    fn clip_culls_outside_quad() {
        let mut list = DrawList::new();
        list.clip(Rect::new(0.0, 0.0, 50.0, 50.0), |inner| {
            // 完全在 clip 之外 → 应被丢弃
            inner.fill(Rect::new(60.0, 60.0, 10.0, 10.0), [1.0; 4]);
        });
        let verts = drain(&list, screen());
        assert!(verts.is_empty(), "完全裁掉的 quad 不该产生顶点");
    }

    #[test]
    fn clip_keeps_inside_quad() {
        let mut list = DrawList::new();
        list.clip(Rect::new(0.0, 0.0, 100.0, 100.0), |inner| {
            inner.fill(Rect::new(10.0, 10.0, 20.0, 20.0), [1.0; 4]);
        });
        let verts = drain(&list, screen());
        assert_eq!(verts.len(), 6);
    }

    #[test]
    fn pop_clip_restores_stack() {
        let mut list = DrawList::new();
        list.clip(Rect::new(0.0, 0.0, 50.0, 50.0), |inner| {
            inner.fill(Rect::new(10.0, 10.0, 5.0, 5.0), [1.0; 4]);
        });
        // 同样的矩形在 clip 外应该全顶点保留
        list.fill(Rect::new(60.0, 60.0, 5.0, 5.0), [1.0; 4]);
        let verts = drain(&list, screen());
        assert_eq!(verts.len(), 12, "clip 内 6 + clip 外 6");
    }

    #[test]
    #[should_panic(expected = "Text 路径")]
    fn text_command_panics_in_phase2() {
        let mut list = DrawList::new();
        list.text(0.0, 10.0, 14.0, [0.0; 4], "x");
        let _ = drain(&list, screen());
    }
}
```

修改 `crates/app/src/lib.rs`：追加 `pub mod paint_backend;`。

- [ ] **Step 2.2：跑测试**

```bash
cargo test -p edit-plus-app paint_backend
```

预期：6 个测试通过（含 1 个 `should_panic`）。

- [ ] **Step 2.3：提交**

```bash
git add crates/app/src/paint_backend.rs crates/app/src/lib.rs
git commit -m "feat(app): paint_backend — DrawList -> GlyphVertex 骨架（FillRect/Clip）"
```

---

## Task 3：UiShell（持 Dock + chrome 装配）

**Files:**
- Create: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 3.1：实现**

UiShell 现在装配的 dock children 全部是"空 widget"——下一阶段开始才换成真 widget。本阶段只验证：dock 接管 layout 后，editor host 拿到的 rect = 现有手算结果。

创建 `crates/app/src/ui_shell.rs`：

```rust
//! UiShell — 持有 Dock + overlays，驱动每帧 layout / paint / dispatch。
//!
//! Phase 2：dock children 全部为占位 widget（PlaceholderWidget），
//! thickness 回调返回当前各 chrome 的实际厚度（与老路径完全一致）。
//! 之所以装这些空 widget 是为了让 dock 真正按 spec §5 的次序"吃边"，
//! 这样 editor_host 拿到的 fill_rect 与老代码扣完所有 chrome 的 rect 等价。
//!
//! 下个阶段（Phase 3+）每接入一个真 widget，就把对应位置的 PlaceholderWidget
//! 替换为真 widget；shell 接口（pub API）保持稳定。

use std::any::Any;

use ui::core::{
    Dock, DockChild, Rect, Screen,
    Widget, LayoutCtx, PaintCtx, EventCtx, Event,
    DrawList, TextMeasure,
};
use ui::Theme;

use crate::editor_host::EditorHostWidget;

/// 占位 widget — paint/event 都是空操作；只为 dock layout 占边。
#[derive(Default)]
struct PlaceholderWidget {
    rect: Rect,
}
impl Widget for PlaceholderWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) { self.rect = rect; }
    fn paint(&self, _ctx: &mut PaintCtx) {}
    fn hit(&self, px: f32, py: f32) -> bool { self.rect.contains(px, py) }
    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx)
        -> Option<Box<dyn Any>> { None }
}

/// 每帧从 app 输入到 shell 的"chrome 显隐 + 厚度"决定：
/// 这些都是 shell 自己不会维护的 app 级状态，由 update_frame 入参注入。
pub struct ShellInputs {
    /// tab_bar 是否显示（多文档模式 + view_mode==Tabs）
    pub tabs_visible: bool,
    pub tabs_thickness: f32,
    /// search_bar 是否显示（active doc.search_state.panel_visible）
    pub search_visible: bool,
    pub search_thickness: f32,
    /// status_bar 厚度（始终显示）
    pub status_thickness: f32,
    /// sidebar 是否显示（view_mode==Sidebar）
    pub sidebar_visible: bool,
    pub sidebar_thickness: f32,
    /// scrollbar 厚度
    pub scrollbar_thickness: f32,
    /// 当前 dpi
    pub dpi: f32,
}

pub struct UiShell {
    dock: Dock,
    /// overlays 暂时空，Phase 8 接入 popup 时填
    overlays: Vec<Box<dyn Widget>>,
    /// child 顺序索引，方便 update_frame 直接改 visible / 重建 thickness
    idx_tabs: usize,
    idx_search: usize,
    idx_status: usize,
    idx_sidebar: usize,
    idx_scrollbar: usize,
}

impl UiShell {
    pub fn new() -> Self {
        let mut dock = Dock::new(EditorHostWidget::new());
        // 顺序：spec §5 — top tabs / top search / bottom status / left sidebar / right scrollbar
        let idx_tabs = push_with_thickness(&mut dock, ui::core::Side::Top, 0.0);
        let idx_search = push_with_thickness(&mut dock, ui::core::Side::Top, 0.0);
        let idx_status = push_with_thickness(&mut dock, ui::core::Side::Bottom, 0.0);
        let idx_sidebar = push_with_thickness(&mut dock, ui::core::Side::Left, 0.0);
        let idx_scrollbar = push_with_thickness(&mut dock, ui::core::Side::Right, 0.0);

        Self {
            dock,
            overlays: Vec::new(),
            idx_tabs, idx_search, idx_status, idx_sidebar, idx_scrollbar,
        }
    }

    /// 由 app 每帧调用：根据 ShellInputs 更新 visible/thickness 后重新 layout。
    pub fn update_frame(
        &mut self,
        screen: Screen,
        theme: &Theme,
        measure: &mut dyn TextMeasure,
        inputs: &ShellInputs,
    ) {
        // 1) 直接根据入参覆盖 visible 与 thickness 回调
        let set = |child: &mut DockChild, vis: bool, t: f32| {
            child.visible = vis;
            // 用闭包捕获新厚度。每帧都重新建一个闭包；开销 ~200ns，可忽略。
            let t_const = t;
            child.thickness = Box::new(move |_, _| t_const);
        };
        set(&mut self.dock.children[self.idx_tabs],     inputs.tabs_visible,     inputs.tabs_thickness);
        set(&mut self.dock.children[self.idx_search],   inputs.search_visible,   inputs.search_thickness);
        set(&mut self.dock.children[self.idx_status],   true,                    inputs.status_thickness);
        set(&mut self.dock.children[self.idx_sidebar],  inputs.sidebar_visible,  inputs.sidebar_thickness);
        set(&mut self.dock.children[self.idx_scrollbar],true,                    inputs.scrollbar_thickness);

        // 2) layout
        let screen_rect = Rect::new(0.0, 0.0, screen.w, screen.h);
        let mut ctx = LayoutCtx { measure, theme, dpi: inputs.dpi };
        self.dock.layout(screen_rect, &mut ctx);
    }

    /// dock 算出的 editor 矩形（fill rect）。
    pub fn editor_rect(&self) -> Rect { self.dock.fill_rect() }

    /// 派发事件：先 overlays（后入先派），再 dock。
    pub fn dispatch(
        &mut self,
        ev: &Event,
        theme: &Theme,
        dpi: f32,
    ) -> Option<Box<dyn Any>> {
        let mut ctx = EventCtx { theme, dpi };
        // overlays 暂为空；Phase 8 填
        for ov in self.overlays.iter_mut().rev() {
            if let Some(action) = ov.on_event(ev, &mut ctx) {
                return Some(action);
            }
        }
        self.dock.dispatch(ev, &mut ctx)
    }

    /// 把 chrome 绘制为 DrawList。Phase 2 这条仍返回空（占位 widget paint 为空）。
    pub fn paint_chrome(&self, theme: &Theme, dpi: f32) -> DrawList {
        let mut list = DrawList::new();
        let mut ctx = PaintCtx { list: &mut list, theme, dpi };
        self.dock.paint(&mut ctx);
        for ov in &self.overlays {
            ov.paint(&mut ctx);
        }
        list
    }
}

fn push_with_thickness(dock: &mut Dock, side: ui::core::Side, t: f32) -> usize {
    let idx = dock.children.len();
    let t_const = t;
    let child = match side {
        ui::core::Side::Top    => DockChild::top   (PlaceholderWidget::default(), move |_, _| t_const),
        ui::core::Side::Bottom => DockChild::bottom(PlaceholderWidget::default(), move |_, _| t_const),
        ui::core::Side::Left   => DockChild::left  (PlaceholderWidget::default(), move |_, _| t_const),
        ui::core::Side::Right  => DockChild::right (PlaceholderWidget::default(), move |_, _| t_const),
    };
    dock.push(child);
    idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::core::NoopMeasure;

    fn screen() -> Screen { Screen::new(1200.0, 800.0) }

    fn baseline_inputs() -> ShellInputs {
        ShellInputs {
            tabs_visible: true,    tabs_thickness:    32.0,
            search_visible: false, search_thickness:  0.0,
            status_thickness: 24.0,
            sidebar_visible: false, sidebar_thickness: 0.0,
            scrollbar_thickness: 12.0,
            dpi: 1.0,
        }
    }

    #[test]
    fn editor_rect_in_tabs_mode_matches_old_calculation() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut shell = UiShell::new();
        shell.update_frame(screen(), &theme, &mut m, &baseline_inputs());

        // 顶 32 + 底 24 + 右 12，左 0 → editor = (0, 32, 1200-12, 800-32-24)
        assert_eq!(shell.editor_rect(), Rect::new(0.0, 32.0, 1188.0, 744.0));
    }

    #[test]
    fn editor_rect_in_sidebar_mode_consumes_left_width() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut shell = UiShell::new();
        let mut inp = baseline_inputs();
        inp.tabs_visible = false; inp.tabs_thickness = 0.0;
        inp.sidebar_visible = true; inp.sidebar_thickness = 220.0;
        shell.update_frame(screen(), &theme, &mut m, &inp);

        // 顶 0 + 底 24 + 右 12 + 左 220 → editor = (220, 0, 968, 776)
        assert_eq!(shell.editor_rect(), Rect::new(220.0, 0.0, 968.0, 776.0));
    }

    #[test]
    fn editor_rect_with_search_bar_visible() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut shell = UiShell::new();
        let mut inp = baseline_inputs();
        inp.search_visible = true; inp.search_thickness = 28.0;
        shell.update_frame(screen(), &theme, &mut m, &inp);

        // 顶 32 + 顶 28 + 底 24 + 右 12 → editor = (0, 60, 1188, 716)
        assert_eq!(shell.editor_rect(), Rect::new(0.0, 60.0, 1188.0, 716.0));
    }

    #[test]
    fn dpi_2x_scales_chrome_thickness() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut shell = UiShell::new();
        let mut inp = baseline_inputs();
        // 调用方负责把入参乘上 dpi（与老代码一致：tab_bar_height = 32 * dpi）
        inp.tabs_thickness = 32.0 * 2.0;
        inp.status_thickness = 24.0 * 2.0;
        inp.scrollbar_thickness = 12.0 * 2.0;
        inp.dpi = 2.0;
        shell.update_frame(screen(), &theme, &mut m, &inp);

        // 顶 64 + 底 48 + 右 24 → editor = (0, 64, 1176, 688)
        assert_eq!(shell.editor_rect(), Rect::new(0.0, 64.0, 1176.0, 688.0));
    }

    #[test]
    fn paint_chrome_in_phase2_emits_empty_drawlist() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut shell = UiShell::new();
        shell.update_frame(screen(), &theme, &mut m, &baseline_inputs());

        let list = shell.paint_chrome(&theme, 1.0);
        assert!(list.is_empty(), "Phase 2 placeholders 不输出任何命令");
    }
}
```

修改 `crates/app/src/lib.rs`：追加 `pub mod ui_shell;`。

- [ ] **Step 3.2：跑测试**

```bash
cargo test -p edit-plus-app ui_shell
```

预期：5 个测试通过。

- [ ] **Step 3.3：提交**

```bash
git add crates/app/src/ui_shell.rs crates/app/src/lib.rs
git commit -m "feat(app): ui_shell — Dock 装配 + ShellInputs + editor_rect 输出"
```

---

## Task 4：把 UiShell 接入 App 主流程（双跑）

这一步**只挂上、不替换任何老逻辑**。等于在每帧 render 入口处计算一遍 dock 的 layout，把结果暂存到 `app.ui_shell`；老的 `app_renderer::render()` 路径完全照旧。

**Files:**
- Modify: `crates/app/src/app.rs`（加字段 + 构造 + 每帧调用）
- Modify: `crates/app/src/app_renderer.rs`（render 入口处一次 update_frame）

- [ ] **Step 4.1：在 App 结构里加字段并初始化**

读 `crates/app/src/app.rs:90-110`（结构体定义），找一个合适位置加：

```rust
pub(crate) ui_shell: crate::ui_shell::UiShell,
```

读 `crates/app/src/app.rs:200-260`（`fn new` / 初始化），在合适位置追加：

```rust
ui_shell: crate::ui_shell::UiShell::new(),
```

- [ ] **Step 4.2：写一个组装 ShellInputs 的辅助方法**

在 `crates/app/src/app.rs` 的 `impl App` 块末尾追加：

```rust
pub(crate) fn build_shell_inputs(&self) -> crate::ui_shell::ShellInputs {
    use ui::settings::Settings;
    use ui::view_mode::ViewMode;
    let dpi = Settings::get().dpi_scale;
    let show_tabs = self.workspace.doc_views.len() > 1;
    let view_mode = Settings::get_static().view_mode;
    let search_visible = self.workspace.doc_views
        .get(self.workspace.active_index)
        .map(|dv| dv.search_state.panel_visible).unwrap_or(false);

    crate::ui_shell::ShellInputs {
        tabs_visible: show_tabs && matches!(view_mode, ViewMode::Tabs),
        tabs_thickness: ui::tab_bar::tab_bar_height(),
        search_visible,
        search_thickness: ui::search_bar::SEARCH_BAR_HEIGHT * dpi,
        status_thickness: Settings::get().status_bar_height,
        sidebar_visible: matches!(view_mode, ViewMode::Sidebar),
        sidebar_thickness: self.workspace.sidebar_state
            .current_width(&self.workspace.sidebar_cfg),
        scrollbar_thickness: Settings::get().scrollbar_reserve(),
        dpi,
    }
}
```

> ⚠️ `current_width / sidebar_cfg / sidebar_state` 等命名以仓库当前现状为准。如果调用对应方法不存在，参考 `crates/ui/src/sidebar.rs::SidebarState::current_width(&SidebarConfig)`，自行调用合适签名。**不要新增** sidebar API；本任务只读现状。

- [ ] **Step 4.3：在 render() 入口处调用 update_frame**

读 `crates/app/src/app_renderer.rs:438` 附近的 `pub(crate) fn render`。在 `let screen_w = self.screen_width(); let screen_h = self.screen_height();` 之后插入：

```rust
// Phase 2：让 dock 与老路径并行计算 layout
{
    use ui::core::Screen;
    let screen = Screen::new(screen_w, screen_h);
    let inputs = self.build_shell_inputs();
    let theme = self.current_theme.clone(); // 临时 clone 一份避开借用冲突
    if let Some(text) = self.text.as_mut() {
        let mut measure = crate::measure_adapter::MeasureFromShaper::new(&mut text.shaper);
        self.ui_shell.update_frame(screen, &theme, &mut measure, &inputs);
    }
}
```

> ⚠️ `current_theme.clone()` 走得通是因为 Theme 实现 Clone（见 `crates/ui/src/theme.rs`）。clone HashMap 在每帧 ~5μs，可接受；后续阶段会把 theme 借用从 App 字段下沉成方法返回值，避免 clone。

- [ ] **Step 4.4：build && run，肉眼验证 UI 不变**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

预期：
- 编译通过、所有测试通过；
- 编辑器视觉跟之前完全一致——tab/sidebar/status/scrollbar/搜索栏全部行为如旧；
- 启动时与每帧不应出现任何 panic。

切换 sidebar 模式（按对应快捷键 / 或 menu）、开搜索栏、滚动几下，再次确认无回归。退出。

- [ ] **Step 4.5：加一个对齐校验测试（可选 debug 路径）**

在 `crates/app/src/app.rs` 末尾 `#[cfg(test)] mod app_tests` 内或新建测试文件追加：

```rust
#[cfg(test)]
mod ui_shell_alignment_tests {
    //! 对齐校验：dock 算出的 editor_rect 应该等于"屏幕 - 各 chrome 厚度"的手算结果。
    //! 任何 chrome 厚度调整后这里要立刻报错。

    use ui::core::{Screen, NoopMeasure, Rect};
    use ui::Theme;
    use crate::ui_shell::{UiShell, ShellInputs};

    fn run(inputs: ShellInputs) -> Rect {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut shell = UiShell::new();
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);
        shell.editor_rect()
    }

    #[test]
    fn alignment_tabs_mode_with_scrollbar() {
        let r = run(ShellInputs {
            tabs_visible: true, tabs_thickness: 32.0,
            search_visible: false, search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: false, sidebar_thickness: 0.0,
            scrollbar_thickness: 12.0,
            dpi: 1.0,
        });
        // 屏幕 1200x800, 上 32, 下 24, 右 12 → editor = (0, 32, 1188, 744)
        assert_eq!(r, Rect::new(0.0, 32.0, 1188.0, 744.0));
    }
}
```

```bash
cargo test -p edit-plus-app ui_shell_alignment
```

预期：通过。

- [ ] **Step 4.6：提交**

```bash
git add crates/app/src/app.rs crates/app/src/app_renderer.rs
git commit -m "feat(app): ui_shell 接入 render 双跑 — dock 与老路径并行"
```

---

## Task 5：Phase 2 收尾

- [ ] **Step 5.1：手测确认双跑稳定**

启动主程序，跑 5 分钟正常使用：开 / 关 tab、切 sidebar、搜索、滚动、resize 窗口（如系统支持）、调 dpi（多显示器拖动）。无 panic、无视觉回归即可。

- [ ] **Step 5.2：spec 追加 Phase 2 完工记录**

在 `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` 末尾追加：

```markdown

## Phase 2 完工记录

- 完工日期：（执行时填）
- 提交范围：commit `<hash>` ~ `<hash>`
- 接入：UiShell + EditorHostWidget + paint_backend 骨架；Dock 与老路径双跑、editor_rect 与手算一致
- 老代码完全未删；Phase 3 起按 spec §7 顺序依次接入真 widget
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 2 完工记录"
```

---

## 边界情况清单

1. **show_tabs=false（单文档）**：tab_bar 隐藏 → tabs_visible=false → dock 顶部不吃边。已在 `editor_rect_in_sidebar_mode_consumes_left_width` 类比验证。
2. **search 显隐切换**：每帧 build_shell_inputs 重读 `panel_visible`，dock 自动重排。无显式重新 layout 调用。
3. **DPI 变化**：thickness 入参由 app 端 `* dpi` 后传入；`dpi` 字段也注入 ctx，方便后续真 widget 使用。
4. **窗口尺寸变化**：每帧 `Screen::new(screen_w, screen_h)`；不缓存。
5. **多文档但 view_mode=Sidebar**：tabs_visible=false（互斥）+ sidebar_visible=true，dock 表现正确。
6. **theme clone**：本阶段为兼容借用 clone Theme。Phase 9 收尾时检查是否还有简化空间。
7. **paint_backend Text 路径未实现**：Phase 2 placeholder widget 不 push Text；如未来接入新 widget 不慎调到 Text 会立刻 panic 提醒——这是设计内的强约束。
