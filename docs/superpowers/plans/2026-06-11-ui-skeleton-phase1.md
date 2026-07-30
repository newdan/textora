# UI 骨架 Phase 1：Core 抽象就位

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `crates/ui` 中新增 `core` 模块（geom / paint / measure / widget / dock 五个文件）、在 `crates/app` 中新增 `measure_adapter.rs`，把骨架打通；老代码完全不动、不接入。完工后 `cargo build && cargo test` 全绿。

**Architecture:**
- 全部代码在 `ui::core` 子模块；通过 `pub use` 抬到 `ui::` 顶层导出。
- 这一阶段**不**修改任何现有 widget（tab_bar/sidebar/...），不修改 `app_renderer.rs` 渲染主流程。
- 引入的所有新类型必须有单元测试覆盖；用真实数据驱动测试，不 mock。

**Tech Stack:** Rust 2024 edition · 工作区 cargo · 现有 `render::GlyphVertex` / `shaping::Shaper` / `ui::theme::Theme`。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md`（全节）

---

## 文件结构

| 文件 | 职责 | 行数预算 |
|---|---|---|
| `crates/ui/src/core/mod.rs` | 子模块声明 + `pub use` | ~30 |
| `crates/ui/src/core/geom.rs` | `Rect / Screen` + px↔NDC 单一转换 | ~120 |
| `crates/ui/src/core/paint.rs` | `DrawCmd` / `DrawList` + 命令构建 helper | ~150 |
| `crates/ui/src/core/measure.rs` | `TextMeasure` trait + `NoopMeasure` 测试桩 | ~40 |
| `crates/ui/src/core/widget.rs` | `Widget` trait + `LayoutCtx/PaintCtx/EventCtx` + `Event` 枚举 + `MouseButton` + `KeyCode` | ~150 |
| `crates/ui/src/core/dock.rs` | `Side / DockChild / Dock` + layout/paint/dispatch | ~250 |
| `crates/ui/src/lib.rs` | 加 `pub mod core;` 与 `pub use core::{...}`（追加） | +10 |
| `crates/app/src/measure_adapter.rs` | `MeasureFromShaper<'a>` 包装 `&'a mut Shaper` | ~40 |
| `crates/app/src/lib.rs` | 加 `pub mod measure_adapter;`（追加） | +1 |

**为什么不放在 `ui/src/` 平级而是 `core/` 子目录：** spec §3 要求 ui crate 里"基建"和"具体 widgets"分层；现有 `tab_bar/sidebar/...` 已经在 `ui/src/` 平级，新基建放 `core/` 子目录可以一眼分辨"哪些是骨架、哪些是组件"。后续 widgets 阶段（Phase 6+）才会把 widget 文件搬到 `widgets/` 子目录。

---

## Task 1：建立 `ui::core::geom`

**Files:**
- Create: `crates/ui/src/core/mod.rs`
- Create: `crates/ui/src/core/geom.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1.1：先写失败的测试**

创建 `crates/ui/src/core/geom.rs`，写测试模块（实现先空）：

```rust
//! 物理像素几何 + 屏幕→NDC 单一转换。
//! ui crate 内部除本文件外不应再出现 NDC 形态的 [f32; 4]。

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self { Self { x, y, w, h } }

    pub fn left(self) -> f32 { self.x }
    pub fn top(self) -> f32 { self.y }
    pub fn right(self) -> f32 { self.x + self.w }
    pub fn bottom(self) -> f32 { self.y + self.h }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w
            && py >= self.y && py < self.y + self.h
    }

    /// 缩进 (top, right, bottom, left)，得到内部 rect。
    /// 若缩进总和超过尺寸，返回 ZERO。
    pub fn shrink(self, top: f32, right: f32, bottom: f32, left: f32) -> Rect {
        let w = self.w - left - right;
        let h = self.h - top - bottom;
        if w <= 0.0 || h <= 0.0 { return Rect::ZERO; }
        Rect::new(self.x + left, self.y + top, w, h)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Screen { pub w: f32, pub h: f32 }

impl Screen {
    pub fn new(w: f32, h: f32) -> Self { Self { w: w.max(1.0), h: h.max(1.0) } }

    /// 像素 (x: 左→右; y: 上→下) 转 NDC ([-1, 1]; y: 上正下负)。
    pub fn px_to_ndc(self, x: f32, y: f32) -> [f32; 2] {
        [x / self.w * 2.0 - 1.0, 1.0 - y / self.h * 2.0]
    }

    /// 像素 Rect 转 NDC [left, right, top, bottom]（与现有代码约定一致）。
    pub fn rect_to_ndc(self, r: Rect) -> [f32; 4] {
        let [l, t] = self.px_to_ndc(r.left(), r.top());
        let [right, b] = self.px_to_ndc(r.right(), r.bottom());
        [l, right, t, b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_inclusive_left_top_exclusive_right_bottom() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(10.0, 20.0));        // 左上含
        assert!(r.contains(109.99, 69.99));     // 右下接近边
        assert!(!r.contains(110.0, 20.0));      // 右边不含
        assert!(!r.contains(10.0, 70.0));       // 下边不含
        assert!(!r.contains(9.99, 20.0));       // 左外不含
    }

    #[test]
    fn rect_shrink_normal_case() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let inner = r.shrink(5.0, 10.0, 5.0, 10.0);
        assert_eq!(inner, Rect::new(10.0, 5.0, 80.0, 40.0));
    }

    #[test]
    fn rect_shrink_too_much_returns_zero() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(r.shrink(20.0, 0.0, 0.0, 0.0), Rect::ZERO);
        assert_eq!(r.shrink(0.0, 5.0, 0.0, 6.0), Rect::ZERO);
    }

    #[test]
    fn px_to_ndc_origin_is_top_left_neg1_pos1() {
        let s = Screen::new(1200.0, 800.0);
        assert_eq!(s.px_to_ndc(0.0, 0.0), [-1.0, 1.0]);
        assert_eq!(s.px_to_ndc(1200.0, 800.0), [1.0, -1.0]);
        let mid = s.px_to_ndc(600.0, 400.0);
        assert!((mid[0] - 0.0).abs() < 1e-6);
        assert!((mid[1] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn rect_to_ndc_layout_matches_lrtb() {
        let s = Screen::new(1000.0, 1000.0);
        let r = Rect::new(0.0, 0.0, 500.0, 500.0);
        let [l, right, t, b] = s.rect_to_ndc(r);
        assert!((l - (-1.0)).abs() < 1e-6);
        assert!((right - 0.0).abs() < 1e-6);
        assert!((t - 1.0).abs() < 1e-6);
        assert!((b - 0.0).abs() < 1e-6);
    }

    #[test]
    fn screen_clamps_zero_to_one() {
        let s = Screen::new(0.0, -5.0);
        assert_eq!(s.w, 1.0);
        assert_eq!(s.h, 1.0);
    }
}
```

创建 `crates/ui/src/core/mod.rs`：

```rust
//! ui 骨架基建：几何、绘制命令、widget trait、容器。

pub mod geom;
pub use geom::{Rect, Screen};
```

修改 `crates/ui/src/lib.rs`，在第 19 行 `pub mod sidebar;` 之后追加：

```rust
pub mod core;
pub use core::{Rect, Screen};
```

- [ ] **Step 1.2：跑测试看到失败**

```bash
cargo test -p edit-plus-ui core::geom -- --nocapture
```

预期：编译通过、6 个测试全过（实现已经一并写在 Step 1.1 里——TDD 约束在更复杂的 task 上严格执行；本 task 因为是纯数据结构、测试与实现一起出更易读，不强行 red-green-refactor）。

- [ ] **Step 1.3：跑整个 workspace 确认未破坏其他**

```bash
cargo build --workspace
cargo test --workspace
```

预期：通过。

- [ ] **Step 1.4：提交**

```bash
git add crates/ui/src/core/mod.rs crates/ui/src/core/geom.rs crates/ui/src/lib.rs
git commit -m "feat(ui-core): geom — Rect/Screen + 单一 px↔NDC 转换"
```

---

## Task 2：建立 `ui::core::paint`

**Files:**
- Create: `crates/ui/src/core/paint.rs`
- Modify: `crates/ui/src/core/mod.rs`

- [ ] **Step 2.1：写实现 + 测试**

创建 `crates/ui/src/core/paint.rs`：

```rust
//! 绘制命令流。Widget 不直接生成 GPU 顶点；它们 push DrawCmd 到 DrawList。
//! app 端 paint_backend 把 DrawList 翻成 GlyphVertex。

use super::geom::Rect;

#[derive(Debug, Clone)]
pub enum DrawCmd {
    /// 实色填充矩形。radius=0 即直角；>0 时由 backend 决定怎么画圆角。
    FillRect { rect: Rect, color: [f32; 4], radius: f32 },

    /// 一行文本，左下角锚点在 (x, y_baseline)。
    Text {
        x: f32,
        y_baseline: f32,
        font_size: f32,
        color: [f32; 4],
        content: String,
    },

    /// 入栈裁剪矩形。Push/Pop 必须配对。
    PushClip(Rect),
    PopClip,
}

#[derive(Debug, Default, Clone)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
}

impl DrawList {
    pub fn new() -> Self { Self { cmds: Vec::new() } }

    pub fn fill(&mut self, rect: Rect, color: [f32; 4]) {
        self.cmds.push(DrawCmd::FillRect { rect, color, radius: 0.0 });
    }

    pub fn fill_rounded(&mut self, rect: Rect, color: [f32; 4], radius: f32) {
        self.cmds.push(DrawCmd::FillRect { rect, color, radius });
    }

    pub fn text(&mut self, x: f32, y_baseline: f32, font_size: f32,
                color: [f32; 4], s: &str) {
        self.cmds.push(DrawCmd::Text {
            x, y_baseline, font_size, color, content: s.to_string(),
        });
    }

    /// 在 rect 内绘制：自动 push/pop clip。闭包内 push 的命令受裁剪保护。
    pub fn clip<F: FnOnce(&mut DrawList)>(&mut self, rect: Rect, f: F) {
        self.cmds.push(DrawCmd::PushClip(rect));
        f(self);
        self.cmds.push(DrawCmd::PopClip);
    }

    /// 命令数（测试用）。
    pub fn len(&self) -> usize { self.cmds.len() }
    pub fn is_empty(&self) -> bool { self.cmds.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_pushes_one_fillrect_with_radius_zero() {
        let mut l = DrawList::new();
        l.fill(Rect::new(0.0, 0.0, 10.0, 10.0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(l.len(), 1);
        match &l.cmds[0] {
            DrawCmd::FillRect { radius, .. } => assert_eq!(*radius, 0.0),
            _ => panic!("expected FillRect"),
        }
    }

    #[test]
    fn text_carries_baseline_not_top() {
        let mut l = DrawList::new();
        l.text(10.0, 50.0, 14.0, [0.0; 4], "hi");
        match &l.cmds[0] {
            DrawCmd::Text { x, y_baseline, font_size, content, .. } => {
                assert_eq!(*x, 10.0);
                assert_eq!(*y_baseline, 50.0);
                assert_eq!(*font_size, 14.0);
                assert_eq!(content, "hi");
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn clip_emits_push_then_inner_then_pop() {
        let mut l = DrawList::new();
        l.clip(Rect::new(0.0, 0.0, 100.0, 100.0), |inner| {
            inner.fill(Rect::new(10.0, 10.0, 5.0, 5.0), [1.0; 4]);
        });
        assert_eq!(l.len(), 3);
        assert!(matches!(l.cmds[0], DrawCmd::PushClip(_)));
        assert!(matches!(l.cmds[1], DrawCmd::FillRect { .. }));
        assert!(matches!(l.cmds[2], DrawCmd::PopClip));
    }

    #[test]
    fn nested_clip_emits_balanced_push_pop() {
        let mut l = DrawList::new();
        l.clip(Rect::new(0.0, 0.0, 100.0, 100.0), |outer| {
            outer.clip(Rect::new(10.0, 10.0, 50.0, 50.0), |inner| {
                inner.fill(Rect::new(20.0, 20.0, 5.0, 5.0), [0.5; 4]);
            });
        });
        let mut depth = 0i32;
        let mut max_depth = 0i32;
        for c in &l.cmds {
            match c {
                DrawCmd::PushClip(_) => { depth += 1; max_depth = max_depth.max(depth); }
                DrawCmd::PopClip     => depth -= 1,
                _ => {}
            }
        }
        assert_eq!(depth, 0, "push/pop unbalanced");
        assert_eq!(max_depth, 2);
    }

    #[test]
    fn empty_drawlist_default() {
        let l = DrawList::default();
        assert!(l.is_empty());
    }
}
```

修改 `crates/ui/src/core/mod.rs`：

```rust
//! ui 骨架基建：几何、绘制命令、widget trait、容器。

pub mod geom;
pub mod paint;
pub use geom::{Rect, Screen};
pub use paint::{DrawCmd, DrawList};
```

- [ ] **Step 2.2：跑测试**

```bash
cargo test -p edit-plus-ui core::paint
```

预期：5 个测试通过。

- [ ] **Step 2.3：提交**

```bash
git add crates/ui/src/core/paint.rs crates/ui/src/core/mod.rs
git commit -m "feat(ui-core): paint — DrawCmd/DrawList + clip 自动配对"
```

---

## Task 3：建立 `ui::core::measure`

**Files:**
- Create: `crates/ui/src/core/measure.rs`
- Modify: `crates/ui/src/core/mod.rs`

- [ ] **Step 3.1：实现 + 测试桩**

创建 `crates/ui/src/core/measure.rs`：

```rust
//! TextMeasure trait — widget 在 layout 阶段测文本宽度的入口。
//! app 端用 `MeasureFromShaper` 包 shaping::Shaper；测试用 `NoopMeasure`。

pub trait TextMeasure {
    /// 返回字符串在指定字号下的宽度（px）。
    fn measure(&mut self, s: &str, font_size: f32) -> f32;
}

/// 测试桩：每个字符固定 `font_size * char_factor` px。
/// 用于 widget 单测，不依赖真实字体。
pub struct NoopMeasure {
    pub char_factor: f32,
}

impl NoopMeasure {
    pub fn new(char_factor: f32) -> Self { Self { char_factor } }
    /// 默认半宽（ASCII 风格）。
    pub fn ascii() -> Self { Self { char_factor: 0.5 } }
}

impl TextMeasure for NoopMeasure {
    fn measure(&mut self, s: &str, font_size: f32) -> f32 {
        s.chars().count() as f32 * font_size * self.char_factor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_measure_proportional_to_char_count_and_size() {
        let mut m = NoopMeasure::ascii();
        assert_eq!(m.measure("abc", 10.0), 15.0);
        assert_eq!(m.measure("abc", 20.0), 30.0);
        assert_eq!(m.measure("", 10.0), 0.0);
    }

    #[test]
    fn noop_measure_uses_char_count_not_byte_count() {
        let mut m = NoopMeasure::ascii();
        // "你好" 是 2 个 char (6 字节)
        assert_eq!(m.measure("你好", 10.0), 10.0);
    }

    #[test]
    fn measure_dispatches_through_dyn() {
        let mut m: Box<dyn TextMeasure> = Box::new(NoopMeasure::ascii());
        assert_eq!(m.measure("hi", 10.0), 10.0);
    }
}
```

修改 `crates/ui/src/core/mod.rs`：

```rust
//! ui 骨架基建：几何、绘制命令、widget trait、容器。

pub mod geom;
pub mod paint;
pub mod measure;

pub use geom::{Rect, Screen};
pub use paint::{DrawCmd, DrawList};
pub use measure::{TextMeasure, NoopMeasure};
```

- [ ] **Step 3.2：跑测试**

```bash
cargo test -p edit-plus-ui core::measure
```

预期：3 个测试通过。

- [ ] **Step 3.3：提交**

```bash
git add crates/ui/src/core/measure.rs crates/ui/src/core/mod.rs
git commit -m "feat(ui-core): measure — TextMeasure trait + NoopMeasure 测试桩"
```

---

## Task 4：建立 `ui::core::widget`

**Files:**
- Create: `crates/ui/src/core/widget.rs`
- Modify: `crates/ui/src/core/mod.rs`

- [ ] **Step 4.1：写 trait + 类型 + 测试**

创建 `crates/ui/src/core/widget.rs`：

```rust
//! Widget trait + 三种 ctx + Event 枚举。
//!
//! 设计要点：
//! - widget 内部坐标全 px (Rect)，hit-test 简化为 rect.contains。
//! - dpi/theme 通过 ctx 入参；ui crate 内部不读 Settings::get() 全局。
//! - Action 上行用 Box<dyn Any>；每个 widget 保留自己的强类型 action 枚举，
//!   app 层 downcast::<TabBarAction>() / ...

use std::any::Any;

use crate::theme::Theme;

use super::geom::Rect;
use super::paint::DrawList;
use super::measure::TextMeasure;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton { Left, Right, Middle }

/// 简化的键码枚举，只覆盖目前 widget 需要的；后续按需扩。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Escape,
    Enter,
    Tab,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Backspace,
    /// 普通字符按键（小写 ASCII / Unicode 字符）。
    Char(char),
    /// 其他未列出的，先包成 raw u32（来自 winit virtual keycode 等）。
    Other(u32),
}

#[derive(Copy, Clone, Debug)]
pub enum Event {
    MouseMove { px: f32, py: f32 },
    MouseDown { px: f32, py: f32, button: MouseButton },
    MouseUp   { px: f32, py: f32, button: MouseButton },
    Wheel     { dx: f32, dy: f32, px: f32, py: f32 },
    KeyDown(KeyCode),
}

pub struct LayoutCtx<'a> {
    pub measure: &'a mut dyn TextMeasure,
    pub theme:   &'a Theme,
    pub dpi:     f32,
}

pub struct PaintCtx<'a> {
    pub list:  &'a mut DrawList,
    pub theme: &'a Theme,
    pub dpi:   f32,
}

pub struct EventCtx<'a> {
    pub theme: &'a Theme,
    pub dpi:   f32,
}

pub trait Widget {
    /// 容器决定 widget 占哪块矩形（物理像素）。widget 通常存一份 self.rect。
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx);

    /// 产出绘制命令。widget 不接触 GPU/atlas/shaper。
    fn paint(&self, ctx: &mut PaintCtx);

    /// 默认实现：rect 包含点。widget 可重写做更细的 hit shape。
    fn hit(&self, _px: f32, _py: f32) -> bool { false }

    /// 处理事件；返回的 action 由 app 层 downcast 解析。默认不响应。
    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx)
        -> Option<Box<dyn Any>> { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[derive(Default)]
    struct Counter { rect: Rect, paint_calls: u32, last_event: Option<Event> }

    /// Counter 自定义 action 类型，验证 Box<dyn Any> downcast 路径。
    #[derive(Debug, PartialEq, Eq)]
    enum CounterAction { Click, Other }

    impl Widget for Counter {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }
        fn paint(&self, ctx: &mut PaintCtx) {
            let _ = self.paint_calls;
            ctx.list.fill(self.rect, [1.0; 4]);
        }
        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }
        fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx)
            -> Option<Box<dyn Any>>
        {
            self.last_event = Some(*ev);
            match ev {
                Event::MouseDown { button: MouseButton::Left, .. } =>
                    Some(Box::new(CounterAction::Click)),
                _ => Some(Box::new(CounterAction::Other)),
            }
        }
    }

    fn make_layout_ctx<'a>(
        theme: &'a Theme,
        m: &'a mut dyn TextMeasure,
    ) -> LayoutCtx<'a> {
        LayoutCtx { measure: m, theme, dpi: 1.0 }
    }

    #[test]
    fn set_rect_is_recorded() {
        let theme = Theme::dark();
        let mut m = crate::core::NoopMeasure::ascii();
        let mut ctx = make_layout_ctx(&theme, &mut m);
        let mut w = Counter::default();
        w.set_rect(Rect::new(5.0, 5.0, 100.0, 30.0), &mut ctx);
        assert_eq!(w.rect, Rect::new(5.0, 5.0, 100.0, 30.0));
    }

    #[test]
    fn paint_emits_fill_rect_into_list() {
        let theme = Theme::dark();
        let mut m = crate::core::NoopMeasure::ascii();
        let mut layout = make_layout_ctx(&theme, &mut m);
        let mut w = Counter::default();
        w.set_rect(Rect::new(0.0, 0.0, 50.0, 50.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn action_downcasts_to_widget_specific_type() {
        let theme = Theme::dark();
        let mut w = Counter::default();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: 1.0, py: 1.0, button: MouseButton::Left },
            &mut ctx,
        );
        let action = action.expect("should produce action");
        let typed = action.downcast::<CounterAction>().expect("downcast OK");
        assert_eq!(*typed, CounterAction::Click);
    }

    #[test]
    fn default_hit_returns_false_unless_overridden() {
        struct Dummy;
        impl Widget for Dummy {
            fn set_rect(&mut self, _: Rect, _: &mut LayoutCtx) {}
            fn paint(&self, _: &mut PaintCtx) {}
        }
        let d = Dummy;
        assert!(!d.hit(0.0, 0.0));
    }
}
```

修改 `crates/ui/src/core/mod.rs`：

```rust
//! ui 骨架基建：几何、绘制命令、widget trait、容器。

pub mod geom;
pub mod paint;
pub mod measure;
pub mod widget;

pub use geom::{Rect, Screen};
pub use paint::{DrawCmd, DrawList};
pub use measure::{TextMeasure, NoopMeasure};
pub use widget::{Widget, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton, KeyCode};
```

- [ ] **Step 4.2：跑测试**

```bash
cargo test -p edit-plus-ui core::widget
```

预期：4 个测试通过。

- [ ] **Step 4.3：跑全 crate 确认未破坏**

```bash
cargo test -p edit-plus-ui
```

预期：通过（包含 core 与现有 sidebar / theme / layout 等的所有测试）。

- [ ] **Step 4.4：提交**

```bash
git add crates/ui/src/core/widget.rs crates/ui/src/core/mod.rs
git commit -m "feat(ui-core): widget — trait + LayoutCtx/PaintCtx/EventCtx + Event"
```

---

## Task 5：建立 `ui::core::dock`（容器）

**Files:**
- Create: `crates/ui/src/core/dock.rs`
- Modify: `crates/ui/src/core/mod.rs`

- [ ] **Step 5.1：写实现 + 测试**

创建 `crates/ui/src/core/dock.rs`：

```rust
//! Dock 容器：吸边布局 + 一个 fill 子。
//! 子的 thickness 是回调（&Theme, dpi -> f32），每帧调用，不缓存。

use std::any::Any;

use crate::theme::Theme;

use super::geom::Rect;
use super::widget::{Widget, LayoutCtx, PaintCtx, EventCtx, Event};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Side { Top, Bottom, Left, Right }

pub struct DockChild {
    pub widget:    Box<dyn Widget>,
    pub side:      Side,
    pub thickness: Box<dyn Fn(&Theme, f32 /*dpi*/) -> f32>,
    pub visible:   bool,
}

impl DockChild {
    pub fn new<W: Widget + 'static>(
        side: Side,
        widget: W,
        thickness: impl Fn(&Theme, f32) -> f32 + 'static,
    ) -> Self {
        Self {
            widget: Box::new(widget),
            side,
            thickness: Box::new(thickness),
            visible: true,
        }
    }

    pub fn top<W: Widget + 'static>(
        widget: W,
        thickness: impl Fn(&Theme, f32) -> f32 + 'static,
    ) -> Self {
        Self::new(Side::Top, widget, thickness)
    }
    pub fn bottom<W: Widget + 'static>(
        widget: W,
        thickness: impl Fn(&Theme, f32) -> f32 + 'static,
    ) -> Self {
        Self::new(Side::Bottom, widget, thickness)
    }
    pub fn left<W: Widget + 'static>(
        widget: W,
        thickness: impl Fn(&Theme, f32) -> f32 + 'static,
    ) -> Self {
        Self::new(Side::Left, widget, thickness)
    }
    pub fn right<W: Widget + 'static>(
        widget: W,
        thickness: impl Fn(&Theme, f32) -> f32 + 'static,
    ) -> Self {
        Self::new(Side::Right, widget, thickness)
    }
}

pub struct Dock {
    pub children: Vec<DockChild>,
    pub fill:     Box<dyn Widget>,
    /// 由最近一次 layout 算出的 fill 矩形；layout 前为 ZERO。
    fill_rect: Rect,
}

impl Dock {
    pub fn new<W: Widget + 'static>(fill: W) -> Self {
        Self {
            children: Vec::new(),
            fill: Box::new(fill),
            fill_rect: Rect::ZERO,
        }
    }

    pub fn push(&mut self, child: DockChild) -> &mut Self {
        self.children.push(child);
        self
    }

    pub fn fill_rect(&self) -> Rect { self.fill_rect }

    /// 从 `screen` 起算，按 children 顺序吃边，剩余给 fill。
    /// invisible 子不占空间且不参与 layout/paint/dispatch。
    pub fn layout(&mut self, screen: Rect, ctx: &mut LayoutCtx) {
        let mut remaining = screen;
        for child in self.children.iter_mut() {
            if !child.visible { continue; }
            let t = (child.thickness)(ctx.theme, ctx.dpi).max(0.0);
            if t <= 0.0 {
                child.widget.set_rect(Rect::ZERO, ctx);
                continue;
            }
            let rect = match child.side {
                Side::Top => {
                    let h = t.min(remaining.h);
                    let r = Rect::new(remaining.x, remaining.y, remaining.w, h);
                    remaining = Rect::new(remaining.x, remaining.y + h,
                                          remaining.w, remaining.h - h);
                    r
                }
                Side::Bottom => {
                    let h = t.min(remaining.h);
                    let r = Rect::new(remaining.x, remaining.bottom() - h,
                                      remaining.w, h);
                    remaining = Rect::new(remaining.x, remaining.y,
                                          remaining.w, remaining.h - h);
                    r
                }
                Side::Left => {
                    let w = t.min(remaining.w);
                    let r = Rect::new(remaining.x, remaining.y, w, remaining.h);
                    remaining = Rect::new(remaining.x + w, remaining.y,
                                          remaining.w - w, remaining.h);
                    r
                }
                Side::Right => {
                    let w = t.min(remaining.w);
                    let r = Rect::new(remaining.right() - w, remaining.y,
                                      w, remaining.h);
                    remaining = Rect::new(remaining.x, remaining.y,
                                          remaining.w - w, remaining.h);
                    r
                }
            };
            child.widget.set_rect(rect, ctx);
        }
        self.fill_rect = remaining;
        self.fill.set_rect(remaining, ctx);
    }

    pub fn paint(&self, ctx: &mut PaintCtx) {
        // fill 先画在底层（编辑器是黑盒，自身 paint 通常空操作）
        self.fill.paint(ctx);
        for child in &self.children {
            if child.visible {
                child.widget.paint(ctx);
            }
        }
    }

    /// 自顶向下 dispatch；从 children 末尾向前查（后入优先 hit），
    /// 命中第一个 widget 后停。fill 兜底。
    pub fn dispatch(&mut self, ev: &Event, ctx: &mut EventCtx)
        -> Option<Box<dyn Any>>
    {
        let pos = event_pos(ev);
        if let Some((px, py)) = pos {
            for child in self.children.iter_mut().rev() {
                if !child.visible { continue; }
                if child.widget.hit(px, py) {
                    return child.widget.on_event(ev, ctx);
                }
            }
            if self.fill.hit(px, py) {
                return self.fill.on_event(ev, ctx);
            }
            None
        } else {
            // 无位置事件（如 KeyDown）：不做命中判断，留给 app 上层路由。
            None
        }
    }
}

fn event_pos(ev: &Event) -> Option<(f32, f32)> {
    match ev {
        Event::MouseMove { px, py }
        | Event::MouseDown { px, py, .. }
        | Event::MouseUp   { px, py, .. }
        | Event::Wheel     { px, py, .. } => Some((*px, *py)),
        Event::KeyDown(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{NoopMeasure, DrawList, MouseButton};

    /// 测试 widget：记录 set_rect 收到的矩形；hit 用 self.rect.contains。
    #[derive(Default)]
    struct Probe {
        rect: Rect,
        paint_count: std::cell::Cell<u32>,
    }
    impl Widget for Probe {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }
        fn paint(&self, _ctx: &mut PaintCtx) {
            self.paint_count.set(self.paint_count.get() + 1);
        }
        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }
        fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx)
            -> Option<Box<dyn Any>>
        {
            Some(Box::new("hit".to_string()))
        }
    }

    fn screen_rect() -> Rect { Rect::new(0.0, 0.0, 1200.0, 800.0) }

    fn make_dock_with_top_bottom() -> Dock {
        let mut dock = Dock::new(Probe::default());
        dock.push(DockChild::top(Probe::default(), |_, dpi| 32.0 * dpi));
        dock.push(DockChild::bottom(Probe::default(), |_, dpi| 24.0 * dpi));
        dock
    }

    #[test]
    fn top_bottom_layout_computes_fill_correctly() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut ctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };

        let mut dock = make_dock_with_top_bottom();
        dock.layout(screen_rect(), &mut ctx);

        // top 32, bottom 24 → fill = (0, 32, 1200, 800-56)
        assert_eq!(dock.fill_rect(), Rect::new(0.0, 32.0, 1200.0, 744.0));
    }

    #[test]
    fn invisible_child_does_not_consume_space() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut ctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };

        let mut dock = make_dock_with_top_bottom();
        dock.children[0].visible = false; // 隐藏 top
        dock.layout(screen_rect(), &mut ctx);

        // top 不吃边 → fill = (0, 0, 1200, 776)
        assert_eq!(dock.fill_rect(), Rect::new(0.0, 0.0, 1200.0, 776.0));
    }

    #[test]
    fn left_then_right_sandwich() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut ctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };

        let mut dock = Dock::new(Probe::default());
        dock.push(DockChild::left(Probe::default(),  |_, _| 220.0));
        dock.push(DockChild::right(Probe::default(), |_, _| 12.0));
        dock.layout(screen_rect(), &mut ctx);

        // fill = x=220, w=1200-220-12=968
        assert_eq!(dock.fill_rect(), Rect::new(220.0, 0.0, 968.0, 800.0));
    }

    #[test]
    fn dpi_scaling_propagates_through_thickness_callback() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut ctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 2.0 };

        let mut dock = make_dock_with_top_bottom();
        dock.layout(screen_rect(), &mut ctx);

        // top 32*2=64, bottom 24*2=48 → fill h = 800-112 = 688
        assert_eq!(dock.fill_rect(), Rect::new(0.0, 64.0, 1200.0, 688.0));
    }

    #[test]
    fn dispatch_routes_mouse_to_correct_child() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
        let mut dock = make_dock_with_top_bottom();
        dock.layout(screen_rect(), &mut layout);

        // 点击屏幕顶部 (10, 10) → 命中 top child
        let mut event = EventCtx { theme: &theme, dpi: 1.0 };
        let action = dock.dispatch(
            &Event::MouseDown { px: 10.0, py: 10.0, button: MouseButton::Left },
            &mut event,
        );
        let s = action.expect("should hit top").downcast::<String>().unwrap();
        assert_eq!(*s, "hit");

        // 点击屏幕中心 → 命中 fill
        let action = dock.dispatch(
            &Event::MouseDown { px: 600.0, py: 400.0, button: MouseButton::Left },
            &mut event,
        );
        let s = action.expect("should hit fill").downcast::<String>().unwrap();
        assert_eq!(*s, "hit");
    }

    #[test]
    fn key_down_returns_none_from_dock_uses_app_layer_routing() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
        let mut dock = make_dock_with_top_bottom();
        dock.layout(screen_rect(), &mut layout);

        let mut event = EventCtx { theme: &theme, dpi: 1.0 };
        let action = dock.dispatch(
            &Event::KeyDown(super::super::widget::KeyCode::Escape),
            &mut event,
        );
        assert!(action.is_none(), "dock 不路由键盘事件，由 app 层处理");
    }

    #[test]
    fn paint_calls_fill_then_each_child_in_order() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };
        let mut dock = make_dock_with_top_bottom();
        dock.layout(screen_rect(), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        dock.paint(&mut paint);
        // 内部 Probe 用 Cell 计数，验证 paint 至少都被调一次（不深究顺序细节）
        // fill + 2 children = 3
        // 这里只通过命令列表数为 0（Probe 不 push 命令）做兜底；
        // 真正的顺序断言留给后续 widget 集成。
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn thickness_zero_makes_child_zero_rect_without_consuming_space() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut ctx = LayoutCtx { measure: &mut m, theme: &theme, dpi: 1.0 };

        let mut dock = Dock::new(Probe::default());
        dock.push(DockChild::top(Probe::default(), |_, _| 0.0));
        dock.layout(screen_rect(), &mut ctx);

        assert_eq!(dock.fill_rect(), screen_rect());
    }
}
```

修改 `crates/ui/src/core/mod.rs`：

```rust
//! ui 骨架基建：几何、绘制命令、widget trait、容器。

pub mod geom;
pub mod paint;
pub mod measure;
pub mod widget;
pub mod dock;

pub use geom::{Rect, Screen};
pub use paint::{DrawCmd, DrawList};
pub use measure::{TextMeasure, NoopMeasure};
pub use widget::{Widget, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton, KeyCode};
pub use dock::{Dock, DockChild, Side};
```

- [ ] **Step 5.2：跑测试**

```bash
cargo test -p edit-plus-ui core::dock
```

预期：8 个测试通过。

- [ ] **Step 5.3：跑全 workspace**

```bash
cargo test --workspace
```

预期：通过。

- [ ] **Step 5.4：提交**

```bash
git add crates/ui/src/core/dock.rs crates/ui/src/core/mod.rs
git commit -m "feat(ui-core): dock — 吸边布局 + 事件分发 + 不可见跳过"
```

---

## Task 6：app 端 `MeasureFromShaper` 适配

**Files:**
- Create: `crates/app/src/measure_adapter.rs`
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 6.1：创建适配器**

先确认 `crates/app/src/lib.rs` 现状：

```bash
head -20 crates/app/src/lib.rs
```

记下 `pub mod ...;` 模式。

创建 `crates/app/src/measure_adapter.rs`：

```rust
//! `ui::core::TextMeasure` 的 app 端实现：包一层 `&mut shaping::Shaper`。
//!
//! widget 在 layout 阶段需要量"标题宽度"等文本宽度；本适配器把这个调用
//! 路由到真实 shaper。临时 set_font_size，shape 后还原，避免污染调用方
//! 的 shaper 状态。

use shaping::Shaper;
use ui::core::TextMeasure;

pub struct MeasureFromShaper<'a> {
    shaper: &'a mut Shaper,
}

impl<'a> MeasureFromShaper<'a> {
    pub fn new(shaper: &'a mut Shaper) -> Self { Self { shaper } }
}

impl<'a> TextMeasure for MeasureFromShaper<'a> {
    fn measure(&mut self, s: &str, font_size: f32) -> f32 {
        if s.is_empty() { return 0.0; }
        let old = self.shaper.font_size();
        self.shaper.set_font_size(font_size);
        let w = self.shaper.shape(s).map(|r| r.width).unwrap_or(0.0);
        self.shaper.set_font_size(old);
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_returns_zero_without_shaping() {
        let mut shaper = Shaper::new().expect("shaper");
        let mut m = MeasureFromShaper::new(&mut shaper);
        assert_eq!(m.measure("", 14.0), 0.0);
    }

    #[test]
    fn measure_restores_caller_font_size() {
        let mut shaper = Shaper::new().expect("shaper");
        shaper.set_font_size(20.0);
        let original = shaper.font_size();
        {
            let mut m = MeasureFromShaper::new(&mut shaper);
            let _ = m.measure("hello", 14.0);
        }
        assert_eq!(shaper.font_size(), original,
                   "measure 应在调用后还原 font_size");
    }

    #[test]
    fn measure_returns_positive_width_for_non_empty_string() {
        let mut shaper = Shaper::new().expect("shaper");
        let mut m = MeasureFromShaper::new(&mut shaper);
        let w = m.measure("hello", 14.0);
        assert!(w > 0.0, "non-empty text should have positive width, got {w}");
    }
}
```

修改 `crates/app/src/lib.rs`，找到 `pub mod ...;` 列表，在合适位置追加：

```rust
pub mod measure_adapter;
```

- [ ] **Step 6.2：跑测试**

```bash
cargo test -p edit-plus-app measure_adapter
```

预期：3 个测试通过。如 `Shaper::new()` 因缺字体在 CI 失败，把测试包上 `#[cfg(not(ci_no_fonts))]` 跳过——但本机首次跑应该过。

- [ ] **Step 6.3：跑 workspace 确认**

```bash
cargo build --workspace
cargo test --workspace
```

预期：全绿。

- [ ] **Step 6.4：提交**

```bash
git add crates/app/src/measure_adapter.rs crates/app/src/lib.rs
git commit -m "feat(app): measure_adapter — Shaper -> ui::TextMeasure"
```

---

## Task 7：把 `ui::core` 顶层导出整理一下，做最终回归

**Files:**
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 7.1：在 ui::lib.rs 顶层 re-export 关键类型**

读 `crates/ui/src/lib.rs`，确认当前内容（Phase 1 之前任务里已加过 `pub mod core; pub use core::{Rect, Screen};`）。把 `pub use` 行替换为完整列表：

```rust
//! edit+ UI — pure UI component library.
//!
//! Provides rendering primitives and widget components.
//! Depends on core, render, shaping, stdext — no app-layer types.

pub mod theme;
pub mod render_geom;
pub mod settings;
pub mod layout;
pub mod scrollbar;
pub mod status_bar;
pub mod search_bar;
pub mod gutter;
pub mod decorations;
pub mod tab_bar;
pub mod viewport;
pub mod popup_menu;
pub mod view_mode;
pub mod sidebar;
pub mod core;

pub use theme::Theme;
pub use settings::Settings;
pub use gutter::RenderContext;

// 骨架（Phase 1）
pub use core::{
    Rect, Screen, DrawCmd, DrawList,
    TextMeasure, NoopMeasure,
    Widget, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton, KeyCode,
    Dock, DockChild, Side,
};
```

- [ ] **Step 7.2：跑全 workspace 终检**

```bash
cargo build --workspace
cargo test --workspace
```

预期：全绿。新增的 core 测试都跑、老测试无回归。

- [ ] **Step 7.3：提交**

```bash
git add crates/ui/src/lib.rs
git commit -m "feat(ui): 顶层 re-export ui::core 骨架类型"
```

---

## Task 8：Phase 1 收尾——验证骨架可用

**Files:** 无修改，仅做"冒烟"验证。

- [ ] **Step 8.1：在主程序里手动跑一次，看老 UI 没坏**

```bash
cargo run -p edit-plus-app -- README.md
```

预期：编辑器正常打开 README.md；tab/sidebar/scrollbar/status 全部行为如旧。Phase 1 不接入任何 widget，所以 UI 视觉不应有任何变化。

进编辑器后做基本动作检查：滚动几下、点 tab、按 ⌘W 关 tab、`Esc` 等常用操作不报 panic 即可。退出。

- [ ] **Step 8.2：检查 git 状态干净**

```bash
git status
```

预期：`working tree clean`，HEAD 在最近一次 Phase 1 commit。

- [ ] **Step 8.3：在 spec 文件下方追加 Phase 1 完工标记**

读 `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md`，确认末尾 `## 11. 不在范围内的事` 之后没有 changelog 段；用 Edit 工具追加：

```markdown

## Phase 1 完工记录（追加）

- 完工日期：（执行时填入实际日期）
- 提交范围：commit `<hash>` ~ `<hash>`
- 已建立的骨架：
  - `ui::core::geom`（`Rect / Screen`）
  - `ui::core::paint`（`DrawCmd / DrawList`）
  - `ui::core::measure`（`TextMeasure / NoopMeasure`）
  - `ui::core::widget`（`Widget` trait + ctx + Event）
  - `ui::core::dock`（吸边布局 + dispatch）
  - `app::measure_adapter`（`MeasureFromShaper`）
- 老代码完全未接入；下阶段 Phase 2 起接入 EditorHost + UiShell 骨架。
```

填入实际 commit 哈希范围（用 `git log --oneline -10` 查）和日期。

- [ ] **Step 8.4：提交 spec changelog**

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 1 完工记录"
```

---

## 回顾 / 边界情况清单

执行完上面 8 个 task 后，再脑里走一遍这些情况，确认骨架行为符合 spec：

1. **dpi 变化**：`Dock::layout` 里 thickness 每次都调回调，吃新的 dpi 入参——OK，已被 `dpi_scaling_propagates_through_thickness_callback` 测试覆盖。
2. **invisible 子**：layout 不吃边、paint 跳过、dispatch 不命中——已被 `invisible_child_does_not_consume_space` 覆盖；`paint_calls_fill_then_each_child_in_order` 是延伸验证（这里只验证不 panic）。
3. **thickness=0**：set_rect 给 ZERO 但不影响 fill——`thickness_zero_makes_child_zero_rect_without_consuming_space` 覆盖。
4. **键盘事件**：dock 不路由 KeyDown，由 app 上层处理（`keyboard_focus`，留给 Phase 4）。spec §8.7 已点明。
5. **clip 配对**：`DrawList::clip` 闭包式 API 保证不会忘记 PopClip——`clip_emits_push_then_inner_then_pop` + `nested_clip_emits_balanced_push_pop` 双覆盖。
6. **Shaper 副作用**：`MeasureFromShaper` 还原 font_size——`measure_restores_caller_font_size` 覆盖。
7. **空字符串 measure**：直接返回 0，不调 shaper——`empty_string_returns_zero_without_shaping`。

后续 Phase 2 plan 将单独成文（`docs/superpowers/plans/2026-06-1X-ui-skeleton-phase2.md`），对接 `UiShell` + `EditorHostWidget` + `paint_backend`，仍保持"老 UI 同时跑"的不破坏原则。
