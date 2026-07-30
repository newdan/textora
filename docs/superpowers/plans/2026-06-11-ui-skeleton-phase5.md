# UI 骨架 Phase 5：scrollbar widget 化（首个带拖拽状态的 widget）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `ui::scrollbar` 那 8 个游离函数（`compute_layout / hit_test / generate_vertices / handle_mouse_*`）整体收进一个 `ScrollbarWidget`：内部持 `ScrollbarState`，paint 走 DrawList，事件路径返回 `ScrollbarAction`。同时把现在的 `events.rs::handle_mouse_input_left/right` / `handle_cursor_moved` 中的 scrollbar 分支删掉，改走 widget。

**Architecture:**
- `ScrollbarWidget` 持 `ScrollbarState`、最近一次的 `ScrollbarLayout`、最近一次的 `(viewport_height, total_display_rows, scroll_top)` 输入快照。
- `set_input(...)` 由 app 注入：`viewport_height / total_display_rows / scroll_top`。`set_rect` 时根据 rect + input 重算 layout（用 px 形式，而不是 NDC）。
- 老 `ui::scrollbar::{compute_layout, hit_test, handle_*}` 删除；状态结构 `ScrollbarState` 与 action `ScrollbarAction` 保留并搬到 widget 内部。
- 对 widget 的鼠标事件：MouseDown 在轨道空白返回 `PageUp/PageDown`，在 thumb 返回 `StartDrag` 并标记内部 dragging；MouseMove + dragging 返回 `DragTo(scroll_top: f64)`；MouseUp 返回 `EndDrag`；Hover 返回 `Hovered(true/false)`。

**Tech Stack:** Rust 2024 · 复用现有 ScrollbarState 字段（拖拽锚点、track 范围）。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §5、§7（阶段 5）

---

## 文件结构

| 文件 | 改动类型 | 备注 |
|---|---|---|
| `crates/ui/src/widgets/scrollbar.rs` | Create | `ScrollbarWidget + ScrollbarState + ScrollbarAction` 全套 |
| `crates/ui/src/widgets/mod.rs` | Modify | `pub mod scrollbar;` |
| `crates/ui/src/lib.rs` | Modify | re-export |
| `crates/ui/src/scrollbar.rs` | Modify | **删除** 所有游离函数；保留 `ScrollbarState/ScrollbarAction` 类型 *re-export 到 widgets/scrollbar.rs* 或直接搬过去 |
| `crates/app/src/events.rs` | Modify | scrollbar 鼠标分支改为 `ui_shell.dispatch` 路径 |
| `crates/app/src/ui_shell.rs` | Modify | 注册真 widget；新增 `set_scrollbar_input` |
| `crates/app/src/app.rs` | Modify | 删除 `pub(crate) scrollbar: ScrollbarState` 字段（widget 内部持有）；删除 `scrollbar_dragging` 字段 |

> ⚠️ 删字段会影响多处引用（如 `app.rs:806/810/813`）；本阶段必须把这些引用改成"通过 widget 路径"。`AppAction::ScrollbarAction / SetScrollbarDragging / UpdateScrollbarState` 这些枚举可以保留或不再使用——本阶段先**保留**，仅让它们的 handler 不再依赖 `self.scrollbar` 字段；Phase 9 收尾时一起清理。

---

## Task 1：把 ScrollbarLayout 改成 px 形态

**Files:**
- Modify: `crates/ui/src/scrollbar.rs`

老 `ScrollbarLayout` 是 NDC 形态。Phase 5 的目标是 widget 内部用 px；先把 layout 转成 px。

- [ ] **Step 1.1：新增 px 形态的 layout 函数**

读 `crates/ui/src/scrollbar.rs::compute_layout`。仿照其逻辑写一个 px 版本，新增到文件中：

```rust
/// Px 形态的 scrollbar layout — 取代老的 NDC 版 compute_layout。
/// 入参：scrollbar 在屏幕上的矩形（由 dock 给的 rect）+ 内容滚动信息。
#[derive(Debug, Clone)]
pub struct ScrollbarLayoutPx {
    /// 整个 scrollbar 区域（含 hover 时变宽留白）
    pub bar_rect: Rect,
    /// thumb 矩形
    pub thumb_rect: Rect,
    /// 是否需要画 thumb（视觉等于 show_thumb，total > visible 时为 true）
    pub show_thumb: bool,
    pub max_scroll: f64,
}

use crate::core::Rect;

pub fn compute_layout_px(
    bar_rect: Rect,
    dpi: f32,
    viewport_height: f64,
    total_display_rows: usize,
    scroll_top: f64,
) -> ScrollbarLayoutPx {
    let min_thumb_px = 25.0 * dpi;
    let total = total_display_rows.max(1) as f64;
    let visible = viewport_height.max(1.0);
    let max_scroll = (total - visible).max(0.0);
    let show_thumb = total > visible;

    let thumb_ratio = (visible / total).min(1.0) as f32;
    let thumb_h = (bar_rect.h * thumb_ratio).max(min_thumb_px).min(bar_rect.h);

    let scroll_ratio = if max_scroll > 0.0 {
        (scroll_top / max_scroll).clamp(0.0, 1.0) as f32
    } else { 0.0 };

    let thumb_y = bar_rect.y + scroll_ratio * (bar_rect.h - thumb_h);
    let thumb_rect = Rect::new(bar_rect.x, thumb_y, bar_rect.w, thumb_h);

    ScrollbarLayoutPx { bar_rect, thumb_rect, show_thumb, max_scroll }
}

#[cfg(test)]
mod px_tests {
    use super::*;

    #[test]
    fn thumb_height_proportional_to_visible_ratio() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 200, 0.0);
        // visible/total = 100/200 = 0.5, thumb_h = 744 * 0.5 = 372
        assert!((lay.thumb_rect.h - 372.0).abs() < 0.5);
    }

    #[test]
    fn thumb_top_at_scroll_zero() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 200, 0.0);
        assert_eq!(lay.thumb_rect.y, 32.0);
    }

    #[test]
    fn thumb_at_bottom_at_max_scroll() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 200, 100.0);
        let expected = bar.y + bar.h - lay.thumb_rect.h;
        assert!((lay.thumb_rect.y - expected).abs() < 0.5);
    }

    #[test]
    fn no_overflow_means_no_thumb() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 200.0, 100, 0.0);
        assert!(!lay.show_thumb);
        assert_eq!(lay.max_scroll, 0.0);
    }

    #[test]
    fn min_thumb_height_enforced() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 1000.0);
        // 100k 行只可见 10：比例 0.0001, thumb_h 默认 0.1, min 25
        let lay = compute_layout_px(bar, 1.0, 10.0, 100_000, 0.0);
        assert!(lay.thumb_rect.h >= 25.0);
    }
}
```

- [ ] **Step 1.2：跑测试**

```bash
cargo test -p edit-plus-ui scrollbar::px_tests
```

预期：5 个测试通过。

- [ ] **Step 1.3：提交**

```bash
git add crates/ui/src/scrollbar.rs
git commit -m "feat(ui-scrollbar): compute_layout_px — Rect-based layout"
```

---

## Task 2：ScrollbarWidget

**Files:**
- Create: `crates/ui/src/widgets/scrollbar.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 2.1：实现**

把老 `ScrollbarState` 字段（`hovered / dragging / drag_start_*`）搬到这里，但**改用 px 单位**（因为 layout 已经是 px）。

创建 `crates/ui/src/widgets/scrollbar.rs`：

```rust
//! ScrollbarWidget — 持 state、layout、画图、鼠标事件。

use std::any::Any;

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton};
use crate::scrollbar::{ScrollbarLayoutPx, compute_layout_px};

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollbarAction {
    PageUp,
    PageDown,
    StartDrag,
    DragTo(f64), // scroll_top 目标值
    EndDrag,
    HoverChanged(bool),
}

pub struct ScrollbarWidget {
    rect: Rect,
    dpi: f32,
    /// 输入快照
    viewport_height: f64,
    total_display_rows: usize,
    scroll_top: f64,
    /// 算出的 px layout（cache 一帧）
    layout: ScrollbarLayoutPx,
    /// 状态
    hovered: bool,
    dragging: bool,
    /// 拖拽起点：thumb_top 偏移（鼠标在 thumb 上的 y 偏移）
    drag_offset_in_thumb: f32,
}

impl ScrollbarWidget {
    pub fn new() -> Self {
        let zero_layout = ScrollbarLayoutPx {
            bar_rect: Rect::ZERO,
            thumb_rect: Rect::ZERO,
            show_thumb: false,
            max_scroll: 0.0,
        };
        Self {
            rect: Rect::ZERO,
            dpi: 1.0,
            viewport_height: 0.0,
            total_display_rows: 0,
            scroll_top: 0.0,
            layout: zero_layout,
            hovered: false,
            dragging: false,
            drag_offset_in_thumb: 0.0,
        }
    }

    pub fn set_input(
        &mut self,
        viewport_height: f64,
        total_display_rows: usize,
        scroll_top: f64,
    ) {
        self.viewport_height = viewport_height;
        self.total_display_rows = total_display_rows;
        self.scroll_top = scroll_top;
    }

    pub fn is_hovered(&self) -> bool { self.hovered }
    pub fn is_dragging(&self) -> bool { self.dragging }

    fn recompute_layout(&mut self) {
        self.layout = compute_layout_px(
            self.rect, self.dpi,
            self.viewport_height, self.total_display_rows, self.scroll_top,
        );
    }
}

impl Widget for ScrollbarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.dpi = ctx.dpi;
        self.recompute_layout();
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 || !self.layout.show_thumb { return; }

        // 1) track 背景
        ctx.list.fill(self.layout.bar_rect, ctx.theme.scrollbar_track);

        // 2) thumb
        let thumb_color = if self.dragging || self.hovered {
            ctx.theme.scrollbar_thumb
        } else {
            // 非 hover 时降低 alpha（与老视觉对齐）
            let mut c = ctx.theme.scrollbar_thumb;
            c[3] *= 0.7;
            c
        };
        ctx.list.fill(self.layout.thumb_rect, thumb_color);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        // 拖拽时 hit 任意位置（避免拖出 bar 失联）
        self.dragging || self.layout.bar_rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        match ev {
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if !self.layout.show_thumb { return None; }
                if self.layout.thumb_rect.contains(*px, *py) {
                    self.dragging = true;
                    self.drag_offset_in_thumb = *py - self.layout.thumb_rect.y;
                    Some(Box::new(ScrollbarAction::StartDrag))
                } else if self.layout.bar_rect.contains(*px, *py) {
                    if *py < self.layout.thumb_rect.y {
                        Some(Box::new(ScrollbarAction::PageUp))
                    } else {
                        Some(Box::new(ScrollbarAction::PageDown))
                    }
                } else {
                    None
                }
            }
            Event::MouseMove { px, py } => {
                if self.dragging {
                    let track_top = self.layout.bar_rect.y;
                    let track_h = self.layout.bar_rect.h - self.layout.thumb_rect.h;
                    let new_thumb_top = (*py - self.drag_offset_in_thumb).clamp(
                        track_top, track_top + track_h,
                    );
                    let scroll_ratio = if track_h > 0.0 {
                        ((new_thumb_top - track_top) as f64) / (track_h as f64)
                    } else { 0.0 };
                    let new_scroll = scroll_ratio * self.layout.max_scroll;
                    Some(Box::new(ScrollbarAction::DragTo(new_scroll)))
                } else {
                    let was = self.hovered;
                    self.hovered = self.layout.bar_rect.contains(*px, *py);
                    if was != self.hovered {
                        Some(Box::new(ScrollbarAction::HoverChanged(self.hovered)))
                    } else {
                        let _ = px; let _ = py; None
                    }
                }
            }
            Event::MouseUp { button: MouseButton::Left, .. } => {
                if self.dragging {
                    self.dragging = false;
                    Some(Box::new(ScrollbarAction::EndDrag))
                } else { None }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{NoopMeasure, DrawList};
    use crate::Theme;

    fn layout_ctx<'a>(theme: &'a Theme, m: &'a mut dyn crate::core::TextMeasure) -> LayoutCtx<'a> {
        LayoutCtx { measure: m, theme, dpi: 1.0 }
    }

    fn make_widget(viewport: f64, total: usize, scroll: f64) -> ScrollbarWidget {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut ctx = layout_ctx(&theme, &mut m);
        let mut w = ScrollbarWidget::new();
        w.set_input(viewport, total, scroll);
        w.set_rect(Rect::new(1188.0, 32.0, 12.0, 744.0), &mut ctx);
        w
    }

    #[test]
    fn click_below_thumb_returns_page_down() {
        let mut w = make_widget(100.0, 200, 0.0);
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        // thumb 在顶部 (32-404)；点 700 在 thumb 下方
        let action = w.on_event(
            &Event::MouseDown { px: 1192.0, py: 700.0, button: MouseButton::Left },
            &mut ctx,
        ).unwrap();
        let typed = action.downcast::<ScrollbarAction>().unwrap();
        assert_eq!(*typed, ScrollbarAction::PageDown);
    }

    #[test]
    fn click_thumb_starts_drag() {
        let mut w = make_widget(100.0, 200, 0.0);
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: 1192.0, py: 100.0, button: MouseButton::Left },
            &mut ctx,
        ).unwrap();
        let typed = action.downcast::<ScrollbarAction>().unwrap();
        assert_eq!(*typed, ScrollbarAction::StartDrag);
        assert!(w.is_dragging());
    }

    #[test]
    fn drag_move_returns_dragto_with_clamped_scroll() {
        let mut w = make_widget(100.0, 200, 0.0);
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        // 先按下 thumb
        w.on_event(&Event::MouseDown { px: 1192.0, py: 100.0, button: MouseButton::Left }, &mut ctx);
        // 拖到底
        let action = w.on_event(&Event::MouseMove { px: 1192.0, py: 5000.0 }, &mut ctx).unwrap();
        let typed = action.downcast::<ScrollbarAction>().unwrap();
        match *typed {
            ScrollbarAction::DragTo(s) => assert!((s - 100.0).abs() < 1.0,
                "drag-to-bottom 应该 ~ max_scroll=100, got {s}"),
            _ => panic!("expected DragTo"),
        }
    }

    #[test]
    fn mouse_up_ends_drag() {
        let mut w = make_widget(100.0, 200, 0.0);
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        w.on_event(&Event::MouseDown { px: 1192.0, py: 100.0, button: MouseButton::Left }, &mut ctx);
        let action = w.on_event(
            &Event::MouseUp { px: 1192.0, py: 200.0, button: MouseButton::Left },
            &mut ctx,
        ).unwrap();
        let typed = action.downcast::<ScrollbarAction>().unwrap();
        assert_eq!(*typed, ScrollbarAction::EndDrag);
        assert!(!w.is_dragging());
    }

    #[test]
    fn hover_change_emits_action() {
        let mut w = make_widget(100.0, 200, 0.0);
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(&Event::MouseMove { px: 1192.0, py: 100.0 }, &mut ctx).unwrap();
        let typed = action.downcast::<ScrollbarAction>().unwrap();
        assert_eq!(*typed, ScrollbarAction::HoverChanged(true));

        // 同位置再 move 不应该再触发
        let action = w.on_event(&Event::MouseMove { px: 1192.0, py: 100.0 }, &mut ctx);
        assert!(action.is_none());

        // 移出
        let action = w.on_event(&Event::MouseMove { px: 100.0, py: 100.0 }, &mut ctx).unwrap();
        let typed = action.downcast::<ScrollbarAction>().unwrap();
        assert_eq!(*typed, ScrollbarAction::HoverChanged(false));
    }

    #[test]
    fn paint_emits_track_and_thumb() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ScrollbarWidget::new();
        w.set_input(100.0, 200, 0.0);
        w.set_rect(Rect::new(1188.0, 32.0, 12.0, 744.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert_eq!(list.len(), 2, "track + thumb");
    }

    #[test]
    fn no_overflow_paint_is_empty() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ScrollbarWidget::new();
        w.set_input(200.0, 100, 0.0);
        w.set_rect(Rect::new(1188.0, 32.0, 12.0, 744.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert!(list.is_empty());
    }
}
```

修改 `crates/ui/src/widgets/mod.rs` 追加 `pub mod scrollbar;`。

修改 `crates/ui/src/lib.rs` 追加 `pub use widgets::scrollbar::{ScrollbarWidget, ScrollbarAction};`。

- [ ] **Step 2.2：跑测试**

```bash
cargo test -p edit-plus-ui widgets::scrollbar
```

预期：7 个测试通过。

- [ ] **Step 2.3：提交**

```bash
git add crates/ui/src/widgets/scrollbar.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui-widgets): scrollbar — px 形态 widget + 拖拽事件"
```

---

## Task 3：UiShell 接 scrollbar widget

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 3.1：注册 widget + set_scrollbar_input**

读 `crates/app/src/ui_shell.rs` Phase 4 末态。

import 区追加：

```rust
use ui::widgets::scrollbar::{ScrollbarWidget, ScrollbarAction};
```

替换 `idx_scrollbar` 注册行为：

```rust
let idx_scrollbar = {
    let idx = dock.children.len();
    let t_const = 0.0_f32;
    dock.push(DockChild::right(ScrollbarWidget::new(), move |_, _| t_const));
    idx
};
```

新增方法：

```rust
impl UiShell {
    pub fn set_scrollbar_input(
        &mut self,
        viewport_height: f64,
        total_display_rows: usize,
        scroll_top: f64,
    ) {
        let any = self.dock.children[self.idx_scrollbar].widget.as_any_mut();
        if let Some(w) = any.downcast_mut::<ScrollbarWidget>() {
            w.set_input(viewport_height, total_display_rows, scroll_top);
        }
    }

    pub fn scrollbar_is_dragging(&self) -> bool {
        // 通过 immutable 路径读 dragging 状态。Box<dyn Widget> 没有 as_any
        // 的不可变版本（Phase 3 加的是 as_any_mut）；我们可以加一个 as_any。
        // 临时方案：在 Widget trait 加 as_any（不可变），所有 widget 默认实现。
        let any = (&self.dock.children[self.idx_scrollbar].widget).as_any();
        any.downcast_ref::<ScrollbarWidget>()
            .map(|w| w.is_dragging())
            .unwrap_or(false)
    }
}
```

修改 `crates/ui/src/core/widget.rs::Widget` trait：

```rust
pub trait Widget: Any {
    // ...
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

- [ ] **Step 3.2：测试**

```bash
cargo test --workspace
```

预期：通过（含原有 ui_shell / shell_alignment 测试）。

- [ ] **Step 3.3：提交**

```bash
git add crates/ui/src/core/widget.rs crates/app/src/ui_shell.rs
git commit -m "feat(app): ui_shell — 注册 scrollbar widget；as_any 双向"
```

---

## Task 4：app 端把 scrollbar 走 widget；删字段；删老路径

这是最大的改动 task。建议分 3 个独立小提交。

**Files:**
- Modify: `crates/app/src/app.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app_renderer.rs`

- [ ] **Step 4.1：每帧塞 scrollbar 数据 + 删除 chrome 老分支**

`app/src/app_renderer.rs::render`，在 Phase 4 已经有的 `set_search_input` / `set_status_input` 旁追加：

```rust
{
    let dv = self.workspace.doc_views.get(self.workspace.active_index);
    let viewport_height = dv.map(|d| d.viewport.viewport_height).unwrap_or(10.0);
    let total = dv.map(|d| d.line_count()).unwrap_or(0);
    let anchor = dv.map(|d| d.viewport.scroll_anchor.doc_line as f64).unwrap_or(0.0);
    self.ui_shell.set_scrollbar_input(viewport_height, total, anchor);
}
```

然后**删除**老 `app_renderer.rs::render` 末段的 scrollbar 顶点生成块：

```rust
{
    let dv = self.workspace.doc_views.get(self.workspace.active_index);
    let status_h = Settings::get().status_bar_height;
    ...
    let layout = ui::scrollbar::compute_layout(...);
    vertices.extend(ui::scrollbar::generate_vertices(&layout, &self.scrollbar, &self.current_theme));
}
```

整段删掉。Phase 3 已有的"chrome_list 走 ui_shell"路径会接管。

- [ ] **Step 4.2：events.rs 鼠标事件改走 widget**

读 `crates/app/src/events.rs:50-260` 三个 scrollbar 分支：

- `handle_cursor_moved` 中的 scrollbar drag 分支（约 50~65 行）
- `handle_cursor_moved` 中的 scrollbar hover 分支（约 125~165 行）
- `handle_mouse_input_left` 中的 scrollbar mousedown 分支（约 225~255 行）

把这三段全部删掉，改为统一前置：

```rust
// Phase 5：鼠标事件先丢给 ui_shell.dispatch，命中 scrollbar widget 时返回 ScrollbarAction
{
    use ui::widgets::scrollbar::ScrollbarAction;
    let ev = ui::core::Event::MouseMove { px, py };
    let action = app.ui_shell.dispatch(&ev, &app.current_theme, ui::settings::Settings::get().dpi_scale);
    if let Some(boxed) = action {
        if let Ok(typed) = boxed.downcast::<ScrollbarAction>() {
            match *typed {
                ScrollbarAction::HoverChanged(b) => {
                    actions.push(AppAction::ScrollbarHovered(b));
                }
                ScrollbarAction::DragTo(scroll_top) => {
                    actions.push(AppAction::UpdateScrollTop(scroll_top));
                }
                _ => {}
            }
            return actions;
        }
    }
}
```

mousedown 类似：用 `Event::MouseDown { px, py, button: MouseButton::Left }` 调 dispatch；按 action 决定 push `AppAction::ScrollbarAction(StartDrag) / PageUp / PageDown`。

mouseup 类似：用 `Event::MouseUp { ... }`；EndDrag → `AppAction::EndScrollbarDrag`。

> ⚠️ **关键**：现在 `app.scrollbar_dragging` 这种字段还存在，先把它当作"读 widget 状态"的镜像：在 `ScrollbarAction::StartDrag` 时设 true，`EndDrag` 时设 false。或者直接调 `app.ui_shell.scrollbar_is_dragging()` 返回——两种方案都行。**推荐**：删字段，每次需要时调 `scrollbar_is_dragging()`。简化 app 状态。

- [ ] **Step 4.3：删 app.rs 的 scrollbar 字段**

读 `crates/app/src/app.rs:98 / 122 / 240 / 246`。

```rust
pub(crate) scrollbar: ui::scrollbar::ScrollbarState,    // 删
pub(crate) scrollbar_dragging: bool,                    // 删
```

```rust
scrollbar: ui::scrollbar::ScrollbarState::new(),        // 删
scrollbar_dragging: false,                              // 删
```

`AppAction::SetScrollbarDragging / UpdateScrollbarState` 分支保留，但 handler 改为：

```rust
AppAction::SetScrollbarDragging(_dragging) => {
    // Phase 5：dragging 状态由 widget 持；这里 no-op
}
AppAction::UpdateScrollbarState(_state) => {
    // Phase 5：no-op；老路径已删
}
```

读 `app/src/app_renderer.rs:809` 附近 `if self.scrollbar_dragging { ... } else { self.needs_redraw = false; }` —— 改为：

```rust
if self.ui_shell.scrollbar_is_dragging() {
    // Keep redrawing
} else {
    self.needs_redraw = false;
}
```

`crates/app/src/app.rs:806/810/813` 这几个原 ScrollbarAction handler 也按 widget 路径修，或保留 no-op 等待 Phase 9 清理。

- [ ] **Step 4.4：build && run**

```bash
cargo build --workspace
cargo test --workspace
cargo run -p edit-plus-app -- README.md
```

进入应用：拖 scrollbar、点击轨道空白翻页、hover 变粗（如有）、放开。无回归。

- [ ] **Step 4.5：分 3 提交**

```bash
git add crates/app/src/app_renderer.rs
git commit -m "refactor(app): scrollbar 走 ui_shell，删 chrome 老分支"

git add crates/app/src/events.rs
git commit -m "refactor(app): events.rs scrollbar 鼠标分支改走 widget dispatch"

git add crates/app/src/app.rs
git commit -m "refactor(app): 删 scrollbar/scrollbar_dragging 字段，dragging 走 widget"
```

---

## Task 5：删 ui::scrollbar 老函数

**Files:**
- Modify: `crates/ui/src/scrollbar.rs`

- [ ] **Step 5.1：grep 残余引用**

```bash
grep -rn "ui::scrollbar::compute_layout\|ui::scrollbar::hit_test\|ui::scrollbar::generate_vertices\|ui::scrollbar::handle_mouse" crates/
```

预期：只剩 `crates/ui/src/scrollbar.rs` 自身的实现 / 测试。

- [ ] **Step 5.2：删除老 API + 老 ScrollbarLayout（NDC 形态）**

`crates/ui/src/scrollbar.rs`：
- 删 `pub struct ScrollbarLayout`（NDC 形态；px 版 `ScrollbarLayoutPx` 保留）
- 删 `pub fn compute_layout`
- 删 `pub enum ScrollbarHit + pub fn hit_test`
- 删 `pub fn generate_vertices`
- 删 `pub fn handle_mouse_move / handle_mouse_down / handle_drag / handle_mouse_up`
- **删** `pub struct ScrollbarState`（widget 内部已自带）
- **删** `pub enum ScrollbarAction`（widget 内部已 re-export）
- 保留 `pub fn compute_layout_px + pub struct ScrollbarLayoutPx`
- 删除老 `#[cfg(test)] mod tests` 里所有针对老函数的测试

如有 `make_theme()` 等测试辅助函数，跟着 NDC 测试一起删。

- [ ] **Step 5.3：build && test**

```bash
cargo build --workspace
cargo test --workspace
```

预期：全绿。

- [ ] **Step 5.4：提交**

```bash
git add crates/ui/src/scrollbar.rs
git commit -m "refactor(ui): scrollbar 老 NDC API 全部删除"
```

---

## Task 6：Phase 5 收尾

- [ ] **Step 6.1：手测**：拖动 scrollbar 大文件、轨道点击翻页、hover 变粗、不同 dpi、跨文档切换 scroll 位置不串台。

- [ ] **Step 6.2：grep 收尾**

```bash
grep -rn "ScrollbarState::new\|scrollbar_dragging" crates/
```

预期：仅命中 widget 内部 / 注释 / 文档。

- [ ] **Step 6.3：spec 追加**

```markdown
## Phase 5 完工记录

- 接入：ScrollbarWidget；事件路径走 ui_shell.dispatch
- 删除：ui::scrollbar 全部 NDC 函数；app::App 的 scrollbar/scrollbar_dragging 字段
- 后续：Phase 6 拆 tab_bar
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 5 完工记录"
```

---

## 边界情况清单

1. **拖拽中鼠标移出窗口**：widget 仍然 dragging=true，只要 event 还能送到 dispatch（winit 通常仍发 MouseMove），DragTo 继续生效。手测重点。
2. **小文件 (无 thumb)**：show_thumb=false → paint 直接 return；click 不返回 action。
3. **极大文件 (min_thumb 生效)**：thumb_h ≥ 25*dpi；scroll_ratio 仍线性映射 max_scroll。
4. **拖到底**：DragTo(max_scroll)；app 端 UpdateScrollTop 已有 clamp。
5. **dpi 切换**：set_rect 拿到新 rect → recompute_layout 自动用新 dpi。
6. **同位置反复 MouseMove**：HoverChanged 只在状态变化时返回，避免每帧都重新渲染。
7. **scrollbar 在 sidebar 模式下仍存在**：dock 顺序无关，sidebar 占左、scrollbar 占右，editor 居中。
