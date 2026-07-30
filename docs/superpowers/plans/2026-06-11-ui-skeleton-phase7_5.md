# UI 骨架 Phase 7.5：通用 VerticalListWidget + sidebar 接入

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Phase 7 完成后，`SidebarWidget` 和 `PopupMenu` 在"垂直列表+条目+hover/active"上是几乎重复的两份代码。本阶段抽出 `VerticalListWidget`：一个不知道"tab 还是菜单"的通用列表 primitive；`SidebarWidget` 内部用它取代手写的 items 渲染与命中。Phase 8 popup 接入时同样使用，自然消除重复。

**Architecture:**
- `VerticalListWidget` 持 `Vec<ListItem>` + `active_index` + `hovered_index`；输入只有"语义化条目"（label / indicator / disabled / separator），不知道 sidebar 或 menu 概念。
- 输出 `ListAction::Selected(index)` 由调用方翻译为自己的强类型 action。
- `SidebarWidget` 重构：内部嵌一个 `VerticalListWidget`；`set_input(tabs, active_index)` 只把 `tabs → ListItem` 喂进去；`paint`/`hit`/`on_event` 委托给 list（仅在 list rect 内）。
- 老 `SidebarLayoutItem` 字段不再使用——它们的语义被 ListItem 取代。
- 不引入滚动；list 在超出区域时**截断**（与 sidebar/menu 当前行为一致）。后续真要滚动时，把 ScrollbarWidget 嵌进 list 里——本阶段不做。

**Tech Stack:** Rust 2024 · 仅 `ui::core::*`，无新依赖。

**Spec：** `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` §7（在 Phase 7 之后、Phase 8 之前插入）

---

## 文件结构

| 文件 | 改动 | 备注 |
|---|---|---|
| `crates/ui/src/widgets/list.rs` | Create | `VerticalListWidget`、`ListItem`、`ListAction`、`ListStyle` |
| `crates/ui/src/widgets/mod.rs` | Modify | `pub mod list;` |
| `crates/ui/src/lib.rs` | Modify | re-export |
| `crates/ui/src/widgets/sidebar.rs` | Modify | 内部嵌 `VerticalListWidget`；删 SidebarLayoutItem 渲染逻辑 |
| `crates/ui/src/sidebar.rs` | Modify | `update_layout` 不再算 items 矩形；只算"列表整体的可用矩形"（list_clip） |

---

## Task 1：实现 VerticalListWidget

**Files:**
- Create: `crates/ui/src/widgets/list.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [ ] **Step 1.1：实现 + 测试**

创建 `crates/ui/src/widgets/list.rs`：

```rust
//! VerticalListWidget — 通用垂直列表 primitive。
//!
//! 不知道"tab/menu"概念。调用方喂入 ListItem，命中后返回 ListAction::Selected(index)。
//! 调用方负责把 index 翻译成自己的强类型 action。

use std::any::Any;

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton};

/// 行类型修饰：影响视觉与命中。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ListItemKind {
    #[default]
    Normal,
    /// 不可点击的分割行（仅画分隔线）。
    Separator,
    /// 不可点击的标题/分组（与 normal 同样画文字，不响应 click）。
    Header,
}

/// 行尾右侧小指示符：dirty 圆点 / conflict 等。语义化、不知道领域。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ListItemIndicator {
    #[default]
    None,
    /// 行尾画一个"未保存"圆点。
    Dot,
}

#[derive(Clone, Debug, Default)]
pub struct ListItem {
    pub label: String,
    pub kind: ListItemKind,
    pub indicator: ListItemIndicator,
}

#[derive(Copy, Clone, Debug)]
pub struct ListStyle {
    pub row_h_logical: f32,      // 例如 24
    pub pad_x_logical: f32,      // 行内左右内边距
    pub pad_y_logical: f32,      // 列表上下内边距
    pub font_size_logical: f32,  // 字号（未乘 dpi）
    pub bg: [f32; 4],            // 列表背景；调用方决定（0 alpha 即透明）
    pub item_active_bg: [f32; 4],
    pub item_hover_bg: [f32; 4],
    pub item_fg: [f32; 4],
    pub separator: [f32; 4],
    pub indicator_color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListAction {
    /// 用户单击了第 index 行（已过滤 Separator/Header）
    Selected(usize),
    /// 鼠标移入/移出某行（用于上层做悬停 tooltip 等；不强制处理）
    HoverChanged(Option<usize>),
}

pub struct VerticalListWidget {
    rect: Rect,
    items: Vec<ListItem>,
    active_index: Option<usize>,
    hovered_index: Option<usize>,
    style: ListStyle,
}

impl VerticalListWidget {
    pub fn new(style: ListStyle) -> Self {
        Self {
            rect: Rect::ZERO,
            items: Vec::new(),
            active_index: None,
            hovered_index: None,
            style,
        }
    }

    pub fn set_items(&mut self, items: Vec<ListItem>) { self.items = items; }
    pub fn set_active(&mut self, idx: Option<usize>) { self.active_index = idx; }
    pub fn rect(&self) -> Rect { self.rect }
    pub fn items(&self) -> &[ListItem] { &self.items }

    /// 计算第 i 行的矩形（px，超出 self.rect 范围的行返回 None — 截断行为）。
    fn item_rect(&self, i: usize, dpi: f32) -> Option<Rect> {
        let row_h = self.style.row_h_logical * dpi;
        let pad_y = self.style.pad_y_logical * dpi;
        let top = self.rect.y + pad_y + i as f32 * row_h;
        let bottom = top + row_h;
        if bottom > self.rect.bottom() { return None; }
        Some(Rect::new(self.rect.x, top, self.rect.w, row_h))
    }

    /// 把屏幕 (px, py) 翻译为命中的行 index（仅 Normal）；不可点行返回 None。
    fn hit_row(&self, px: f32, py: f32, dpi: f32) -> Option<usize> {
        if !self.rect.contains(px, py) { return None; }
        for (i, item) in self.items.iter().enumerate() {
            let Some(r) = self.item_rect(i, dpi) else { break; };
            if r.contains(px, py) {
                return matches!(item.kind, ListItemKind::Normal).then_some(i);
            }
        }
        None
    }
}

impl Widget for VerticalListWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 { return; }

        // 1) 列表背景（如调用方设了透明 [0,0,0,0]，paint_backend 会丢弃；
        //    这里照常 fill — 由 backend 决定是否优化）
        if self.style.bg[3] > 0.0 {
            ctx.list.fill(self.rect, self.style.bg);
        }

        let dpi = ctx.dpi;
        let pad_x = self.style.pad_x_logical * dpi;
        let font_size = self.style.font_size_logical * dpi;
        let dot_r = (font_size * 0.18).max(2.0);

        // 2) 行
        for (i, item) in self.items.iter().enumerate() {
            let Some(row_rect) = self.item_rect(i, dpi) else { break; };

            match item.kind {
                ListItemKind::Separator => {
                    let sep_h = (1.0 * dpi).max(1.0);
                    let y = row_rect.y + (row_rect.h - sep_h) * 0.5;
                    ctx.list.fill(
                        Rect::new(row_rect.x + pad_x, y,
                                  row_rect.w - pad_x * 2.0, sep_h),
                        self.style.separator,
                    );
                    continue;
                }
                ListItemKind::Header | ListItemKind::Normal => {
                    // hover/active 仅 normal
                    if matches!(item.kind, ListItemKind::Normal) {
                        if Some(i) == self.active_index {
                            ctx.list.fill(row_rect, self.style.item_active_bg);
                        } else if Some(i) == self.hovered_index {
                            ctx.list.fill(row_rect, self.style.item_hover_bg);
                        }
                    }

                    // label：基线 row 中线 + font_size*0.35（与 status_bar 一致）
                    let baseline = row_rect.y + row_rect.h * 0.5 + font_size * 0.35;
                    ctx.list.text(
                        row_rect.x + pad_x, baseline, font_size,
                        self.style.item_fg, &item.label,
                    );

                    // 指示符（行尾右侧）
                    if matches!(item.indicator, ListItemIndicator::Dot) {
                        let cx = row_rect.right() - pad_x - dot_r;
                        let cy = row_rect.y + row_rect.h * 0.5;
                        ctx.list.fill_rounded(
                            Rect::new(cx - dot_r, cy - dot_r, dot_r * 2.0, dot_r * 2.0),
                            self.style.indicator_color, dot_r,
                        );
                    }
                }
            }
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        match ev {
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.hit_row(*px, *py, ctx.dpi)
                    .map(|i| Box::new(ListAction::Selected(i)) as Box<dyn Any>)
            }
            Event::MouseMove { px, py } => {
                let new = if self.rect.contains(*px, *py) {
                    let mut found = None;
                    for (i, item) in self.items.iter().enumerate() {
                        let Some(r) = self.item_rect(i, ctx.dpi) else { break; };
                        if r.contains(*px, *py)
                            && matches!(item.kind, ListItemKind::Normal) {
                            found = Some(i);
                            break;
                        }
                    }
                    found
                } else {
                    None
                };
                if new != self.hovered_index {
                    self.hovered_index = new;
                    Some(Box::new(ListAction::HoverChanged(new)))
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

    fn style() -> ListStyle {
        ListStyle {
            row_h_logical: 24.0, pad_x_logical: 8.0, pad_y_logical: 4.0,
            font_size_logical: 13.0,
            bg: [0.1, 0.1, 0.1, 1.0],
            item_active_bg: [0.2; 4],
            item_hover_bg: [0.15; 4],
            item_fg: [0.9; 4],
            separator: [0.3; 4],
            indicator_color: [1.0, 0.5, 0.0, 1.0],
        }
    }

    fn layout_ctx<'a>(theme: &'a Theme, m: &'a mut dyn crate::core::TextMeasure) -> LayoutCtx<'a> {
        LayoutCtx { measure: m, theme, dpi: 1.0 }
    }

    fn make_list(items: Vec<ListItem>, rect: Rect) -> VerticalListWidget {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = VerticalListWidget::new(style());
        w.set_items(items);
        w.set_rect(rect, &mut layout);
        w
    }

    fn item(label: &str) -> ListItem {
        ListItem { label: label.into(), kind: ListItemKind::Normal, indicator: ListItemIndicator::None }
    }

    #[test]
    fn paint_emits_bg_plus_text_per_visible_row() {
        let theme = Theme::dark();
        let w = make_list(
            vec![item("a.rs"), item("b.rs"), item("c.rs")],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // bg + 3 text = 4
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn rows_overflowing_rect_are_truncated() {
        // rect 高 60px, pad_y=4*2=8, row=24 → 容纳 (60-8)/24 = 2 行
        let theme = Theme::dark();
        let w = make_list(
            vec![item("a"), item("b"), item("c"), item("d")],
            Rect::new(0.0, 0.0, 220.0, 60.0),
        );
        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // bg + 2 text = 3
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn click_in_row_returns_selected_index() {
        let mut w = make_list(
            vec![item("a"), item("b"), item("c")],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        // 第二行：top = 4 + 24 = 28；中线 40
        let action = w.on_event(
            &Event::MouseDown { px: 100.0, py: 40.0, button: MouseButton::Left },
            &mut ctx,
        ).unwrap();
        let typed = action.downcast::<ListAction>().unwrap();
        assert_eq!(*typed, ListAction::Selected(1));
    }

    #[test]
    fn click_on_separator_returns_none() {
        let mut w = make_list(
            vec![
                item("a"),
                ListItem { label: "".into(), kind: ListItemKind::Separator, indicator: ListItemIndicator::None },
                item("c"),
            ],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: 100.0, py: 40.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(action.is_none());
    }

    #[test]
    fn click_outside_rect_returns_none() {
        let mut w = make_list(
            vec![item("a")],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
        let action = w.on_event(
            &Event::MouseDown { px: 500.0, py: 500.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(action.is_none());
    }

    #[test]
    fn hover_change_emits_action_only_on_change() {
        let mut w = make_list(
            vec![item("a"), item("b")],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let theme = Theme::dark();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };

        let act1 = w.on_event(&Event::MouseMove { px: 100.0, py: 16.0 }, &mut ctx).unwrap();
        let typed = act1.downcast::<ListAction>().unwrap();
        assert_eq!(*typed, ListAction::HoverChanged(Some(0)));

        // 同行再 move 不再触发
        let act2 = w.on_event(&Event::MouseMove { px: 100.0, py: 16.0 }, &mut ctx);
        assert!(act2.is_none());

        // 跨行触发
        let act3 = w.on_event(&Event::MouseMove { px: 100.0, py: 40.0 }, &mut ctx).unwrap();
        let typed = act3.downcast::<ListAction>().unwrap();
        assert_eq!(*typed, ListAction::HoverChanged(Some(1)));
    }

    #[test]
    fn active_row_paints_active_bg() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = VerticalListWidget::new(style());
        w.set_items(vec![item("a"), item("b")]);
        w.set_active(Some(1));
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // bg + active_bg(行 1) + text(行 0) + text(行 1) = 4
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn dot_indicator_emits_extra_fill() {
        let theme = Theme::dark();
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = VerticalListWidget::new(style());
        w.set_items(vec![ListItem {
            label: "x".into(),
            kind: ListItemKind::Normal,
            indicator: ListItemIndicator::Dot,
        }]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);

        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // bg + text + dot = 3
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn empty_items_paint_emits_only_bg() {
        let theme = Theme::dark();
        let w = make_list(Vec::new(), Rect::new(0.0, 0.0, 220.0, 100.0));
        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn transparent_bg_emits_no_bg_fill() {
        let theme = Theme::dark();
        let mut s = style();
        s.bg = [0.0, 0.0, 0.0, 0.0];
        let mut m = NoopMeasure::ascii();
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = VerticalListWidget::new(s);
        w.set_items(vec![item("x")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);
        let mut list = DrawList::new();
        let mut paint = PaintCtx { list: &mut list, theme: &theme, dpi: 1.0 };
        w.paint(&mut paint);
        // text only (no bg)
        assert_eq!(list.len(), 1);
    }
}
```

修改 `crates/ui/src/widgets/mod.rs` 追加：

```rust
pub mod list;
```

修改 `crates/ui/src/lib.rs` 追加：

```rust
pub use widgets::list::{
    VerticalListWidget, ListItem, ListItemKind, ListItemIndicator,
    ListStyle, ListAction,
};
```

- [ ] **Step 1.2：跑测试**

```bash
cargo test -p edit-plus-ui widgets::list
```

预期：10 个测试通过。

- [ ] **Step 1.3：跑 workspace 确认未破坏其他**

```bash
cargo build --workspace
cargo test --workspace
```

- [ ] **Step 1.4：提交**

```bash
git add crates/ui/src/widgets/list.rs crates/ui/src/widgets/mod.rs crates/ui/src/lib.rs
git commit -m "feat(ui-widgets): list — VerticalListWidget 通用列表 primitive"
```

---

## Task 2：sidebar.rs 简化——删除 items 矩形计算

**Files:**
- Modify: `crates/ui/src/sidebar.rs`

Phase 7 完成后 `sidebar.rs::update_layout` 仍在算 `SidebarLayoutItem` 矩形 + paint 仍在画行。本任务把这两段挪到 list widget 头上。

- [ ] **Step 2.1：删 items 算法 + paint 行**

读 `crates/ui/src/sidebar.rs::update_layout`（约 137-227 行）。

把"计算每个 tab 的 item rect"那一段（约 183-196 行）整段删除：

```rust
// 删除：
let mut items = Vec::with_capacity(input.tabs.len());
for (idx, tab) in input.tabs.iter().enumerate() {
    let item_top = list_top_px + idx as f32 * row_h;
    ...
    items.push(SidebarLayoutItem { ... });
}
```

`SidebarLayout::items` 字段保留**为空 Vec**（不破坏老 fields struct）；下个 task 中 `SidebarWidget` 不再读它。**Phase 9 收尾**时彻底删 items 字段。

`paint(&self, ctx, active_index)` 改为：只画 `bg / header / new_btn / settings_btn`，**删除** items 行的渲染：

```rust
impl SidebarState {
    pub fn paint(&self, ctx: &mut PaintCtx, _active_index: Option<usize>) {
        let Some(layout) = &self.layout else { return; };
        ctx.list.fill(layout.bg_rect, ctx.theme.sidebar_bg);
        ctx.list.fill(layout.header_rect, ctx.theme.sidebar_header_bg);
        ctx.list.fill(layout.new_btn_rect, ctx.theme.sidebar_button_bg);
        ctx.list.fill(layout.settings_btn_rect, ctx.theme.sidebar_header_bg);
        // items 由 SidebarWidget 内部的 list 子 widget 负责
    }
}
```

`hit_test_px` 同样**删除**对 items 的命中（但保留对 menu_btn / new_btn / settings_btn / edge_resize_rect 的命中）：

```rust
pub fn hit_test_px(&self, px: f32, py: f32) -> Option<SidebarAction> {
    let layout = self.layout.as_ref()?;
    if layout.menu_btn_rect.contains(px, py)     { return Some(SidebarAction::TogglePin); }
    if layout.new_btn_rect.contains(px, py)      { return Some(SidebarAction::NewDocument); }
    if layout.settings_btn_rect.contains(px, py) { return Some(SidebarAction::OpenSettingsMenu); }
    if layout.edge_resize_rect.contains(px, py)  { return Some(SidebarAction::StartResize); }
    None
}
```

`list_clip` 字段（Rect 形态）保留——它就是 list widget 要占的矩形。

- [ ] **Step 2.2：调测试**

读 sidebar.rs 测试。`sidebar_layout_items_match_tab_count / sidebar_click_file_emits_switch_tab` 这两个针对 items 的测试 —— 整体删除（行为已迁移到 list widget，单元测试在 list.rs 里覆盖）。

保留：clamp_width / visibility / menu_btn / new_btn 等测试。

- [ ] **Step 2.3：build + test**

```bash
cargo test -p edit-plus-ui sidebar
```

预期：通过。

- [ ] **Step 2.4：提交**

```bash
git add crates/ui/src/sidebar.rs
git commit -m "refactor(ui-sidebar): 删 items 算法/paint/hit；items 移交 list widget"
```

---

## Task 3：SidebarWidget 内嵌 VerticalListWidget

**Files:**
- Modify: `crates/ui/src/widgets/sidebar.rs`

- [ ] **Step 3.1：嵌入 list 子 widget**

读 `crates/ui/src/widgets/sidebar.rs`（Phase 7 末态）。

加字段：

```rust
use crate::widgets::list::{
    VerticalListWidget, ListItem, ListItemKind, ListItemIndicator,
    ListStyle, ListAction,
};

pub struct SidebarWidget {
    state: SidebarState,
    cfg: SidebarConfig,
    rect: Rect,
    list: VerticalListWidget,
    tabs: Vec<TabInfo>,
    active_index: Option<usize>,
    traffic_light_inset: (f32, f32),
    screen_w: f32, screen_h: f32,
    drag_anchor_x: Option<f32>,
}
```

`new()`：

```rust
impl SidebarWidget {
    pub fn new(dpi: f32) -> Self {
        let cfg = SidebarConfig::new_default(dpi);
        let state = SidebarState::new(&cfg);
        let list = VerticalListWidget::new(make_sidebar_list_style());
        Self {
            state, cfg, rect: Rect::ZERO, list,
            tabs: Vec::new(), active_index: None,
            traffic_light_inset: (0.0, 0.0),
            screen_w: 0.0, screen_h: 0.0,
            drag_anchor_x: None,
        }
    }
}

fn make_sidebar_list_style() -> ListStyle {
    // 与 sidebar 主题对齐；色值在 set_rect/paint 时通过 theme 重置（见 set_rect）
    // 这里给"占位默认值"，paint 前会被覆盖。
    ListStyle {
        row_h_logical: 24.0, pad_x_logical: 8.0, pad_y_logical: 0.0,
        font_size_logical: 13.0,
        bg: [0.0, 0.0, 0.0, 0.0],   // 透明：sidebar 主背景已铺好
        item_active_bg: [0.0; 4],
        item_hover_bg: [0.0; 4],
        item_fg: [0.0; 4],
        separator: [0.0; 4],
        indicator_color: [0.0; 4],
    }
}
```

- [ ] **Step 3.2：set_rect 时把 list rect = list_clip + 注入 theme 颜色**

```rust
impl Widget for SidebarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        // 1) 先让 SidebarState 算出 layout（含 list_clip）
        let input = SidebarInput {
            tabs: &self.tabs,
            active_index: self.active_index,
            screen_w: self.screen_w,
            screen_h: self.screen_h,
            traffic_light_inset: self.traffic_light_inset,
        };
        self.state.update_layout(&input, &self.cfg);

        // 2) list 子 widget 的矩形 = list_clip
        let list_rect = self.state.current_layout()
            .map(|l| l.list_clip)
            .unwrap_or(Rect::ZERO);

        // 3) 把 theme 颜色灌进 list style（每帧重灌一次）
        let theme = ctx.theme;
        self.list = VerticalListWidget::new(ListStyle {
            row_h_logical: 24.0, pad_x_logical: 8.0, pad_y_logical: 0.0,
            font_size_logical: 13.0,
            bg: [0.0, 0.0, 0.0, 0.0],
            item_active_bg: theme.sidebar_item_active_bg,
            item_hover_bg:  {
                // 没有专门的 hover bg；在 active_bg 基础上调暗
                let mut c = theme.sidebar_item_active_bg;
                c[3] *= 0.5;
                c
            },
            item_fg: theme.sidebar_item_fg,
            separator: theme.menu_separator,
            indicator_color: theme.foreground, // dirty 圆点用前景色
        });

        // 4) items：tab → ListItem
        let items: Vec<ListItem> = self.tabs.iter().map(|t| ListItem {
            label: t.title.clone(),
            kind: ListItemKind::Normal,
            indicator: if t.is_dirty { ListItemIndicator::Dot } else { ListItemIndicator::None },
        }).collect();
        self.list.set_items(items);
        self.list.set_active(self.active_index);

        // 5) 把 list_rect 灌进 list（必须在 set_items 之后；item_rect 用 self.rect）
        self.list.set_rect(list_rect, ctx);
    }
}
```

> ⚠️ 每帧 `self.list = VerticalListWidget::new(...)` 会丢掉 hovered_index。代价是 hover 闪——本阶段接受。Phase 9 优化：让 ListStyle 支持 `set_style(...)` 不重建。

更简洁：把 `VerticalListWidget` 加 `set_style(s)` 方法即可；本任务直接做：

修改 `crates/ui/src/widgets/list.rs::VerticalListWidget`：

```rust
pub fn set_style(&mut self, s: ListStyle) { self.style = s; }
```

`set_rect` 改用：

```rust
self.list.set_style(make_style_from_theme(theme));
self.list.set_items(items);
self.list.set_active(self.active_index);
self.list.set_rect(list_rect, ctx);
```

提取 `make_style_from_theme(theme: &Theme) -> ListStyle` 即可。

- [ ] **Step 3.3：paint / on_event 委托 list**

```rust
impl Widget for SidebarWidget {
    fn paint(&self, ctx: &mut PaintCtx) {
        if !self.state.is_visible() { return; }
        // 1) sidebar 整体框架（bg / header / 按钮）
        self.state.paint(ctx, self.active_index);
        // 2) list 子 widget
        self.list.paint(ctx);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.state.is_visible() && self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        match ev {
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                // 1) 先看 sidebar 框架按钮
                if let Some(action) = self.state.hit_test_px(*px, *py) {
                    if matches!(action, SidebarAction::StartResize) {
                        self.drag_anchor_x = Some(*px);
                    }
                    return Some(Box::new(action));
                }
                // 2) 再看 list 子 widget
                if let Some(boxed) = self.list.on_event(ev, ctx) {
                    if let Ok(la) = boxed.downcast::<ListAction>() {
                        if let ListAction::Selected(i) = *la {
                            return Some(Box::new(SidebarAction::SwitchTab(i)));
                        }
                    }
                }
                None
            }
            Event::MouseMove { px, .. } => {
                if let Some(anchor) = self.drag_anchor_x {
                    let new_w = self.cfg.width + (*px - anchor);
                    let mut clamped = SidebarConfig { pinned: self.cfg.pinned, width: new_w };
                    clamped.clamp_width(1.0);
                    self.cfg.width = clamped.width;
                    self.drag_anchor_x = Some(*px);
                    return Some(Box::new(SidebarAction::ResizeTo(self.cfg.width)));
                }
                // 把 hover 转给 list（吃掉 list 返回的 HoverChanged 不上行）
                let _ = self.list.on_event(ev, ctx);
                None
            }
            Event::MouseUp { button: MouseButton::Left, .. } => {
                if self.drag_anchor_x.take().is_some() {
                    Some(Box::new(SidebarAction::EndResize))
                } else { None }
            }
            _ => None,
        }
    }
}
```

- [ ] **Step 3.4：测试**

读 `widgets/sidebar.rs::tests`（Phase 7 写过 4 个）。把 `pinned_paint_emits_bg_header_buttons` 中的命令计数从"≥4"改成"≥4"（行为不变；还是 bg/header/new/settings 之外加 list bg/text，只是 list 自己 bg 透明，所以只多 text 命令）。手测时再验证视觉。

新增测试：

```rust
#[test]
fn click_in_list_emits_switch_tab() {
    let theme = Theme::dark();
    let mut m = NoopMeasure::ascii();
    let mut layout = layout_ctx(&theme, &mut m);
    let mut w = SidebarWidget::new(1.0);
    w.set_visibility(Visibility::Pinned);
    let tabs = vec![
        TabInfo { title: "a.rs".into(), file_path: None, is_dirty: false, language: "rust".into() },
        TabInfo { title: "b.rs".into(), file_path: None, is_dirty: true,  language: "rust".into() },
    ];
    w.set_input(tabs, Some(0), (0.0, 0.0), 1200.0, 800.0);
    w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut layout);

    // list_clip 在 sidebar 中部；通过 state 拿
    let list_clip = w.state.current_layout().unwrap().list_clip;
    let cy = list_clip.y + 12.0; // 第一行中点附近

    let mut ctx = EventCtx { theme: &theme, dpi: 1.0 };
    let action = w.on_event(
        &Event::MouseDown { px: 100.0, py: cy, button: MouseButton::Left },
        &mut ctx,
    ).unwrap();
    let typed = action.downcast::<SidebarAction>().unwrap();
    assert!(matches!(*typed, SidebarAction::SwitchTab(0)));
}
```

```bash
cargo test -p edit-plus-ui widgets::sidebar
```

预期：通过。

- [ ] **Step 3.5：手测**

```bash
cargo run -p edit-plus-app -- README.md
```

切到 sidebar 模式：
- 文件列表显示与 Phase 7 末态一致；
- 点击文件切换；
- dirty 圆点（如有未保存 tab）显示；
- 滚动按钮 / + / 设置 / resize 边缘 — 全部继续工作。

肉眼对比：item 行高、字号、active 高亮颜色应基本一致。如果颜色不准，调 `make_style_from_theme` 中的 `item_hover_bg` 等。

- [ ] **Step 3.6：提交**

```bash
git add crates/ui/src/widgets/sidebar.rs crates/ui/src/widgets/list.rs
git commit -m "refactor(sidebar): 内嵌 VerticalListWidget；删除 items 内联渲染"
```

---

## Task 4：Phase 7.5 收尾

- [ ] **Step 4.1：grep 验证**

```bash
grep -rn "SidebarLayoutItem" crates/
```

`SidebarLayoutItem` 类型仍在（layout 字段保留为空 Vec）。**Phase 9** 收尾时随老 NDC 残骸一起删。本阶段不动。

- [ ] **Step 4.2：spec 追加完工记录**

在 `docs/superpowers/specs/2026-06-11-ui-skeleton-design.md` 末尾追加：

```markdown
## Phase 7.5 完工记录

- 抽出：VerticalListWidget（list/popup 共享）
- SidebarWidget：items 渲染从手写改委托 list；保持视觉等价
- 后续：Phase 8 popup 接 list；Phase 9 删 SidebarLayoutItem 残留
```

```bash
git add docs/superpowers/specs/2026-06-11-ui-skeleton-design.md
git commit -m "docs(spec): UI 骨架 Phase 7.5 完工记录"
```

---

## 对 Phase 8 的连带影响（不在本 plan 执行范围）

Phase 8 plan 的 Task 1 / Task 2 中 `popup_menu.rs::PopupMenu::paint` 与 `PopupMenuWidget` 应改为内部嵌 `VerticalListWidget`：

- `PopupMenu::items` 直接对应 `Vec<ListItem>`（label + Separator/Normal）；
- `PopupMenu::menu_rect` 即 list 的外框；list rect 留 8px 内边距即可。
- `PopupMenuWidget::paint` 简化为：先画"菜单壳"（圆角 bg + shadow + border），再委托 list 画 items。

执行 Phase 8 前先**回看本 plan 的 Task 1**，确认 `ListItem` / `ListItemKind::Separator` / `ListAction` 三件套已就绪——之后 Phase 8 popup 几乎不用再写"行渲染"代码。

> 本 plan 不修改 Phase 8 plan 文档；执行 Phase 8 时按此提示落地即可。

---

## 边界情况清单

1. **空 tabs**：`set_items(Vec::new())`，list paint 只输出 bg（透明则 0 命令）。
2. **截断**：list rect 容纳不下所有行 → `item_rect` 返回 None → paint 自然跳过。比 Phase 7 末态行为对齐（Phase 7 也截断）。
3. **dirty 圆点**：`indicator_color = theme.foreground`，颜色与文字同；视觉与老 sidebar 不完全一致（老 sidebar 没画过圆点）——这是新增能力，需要肉眼确认是否接受；如不接受，本 task 暂时把 indicator 设 `ListItemIndicator::None`，留 Phase 8/9 补主题。
4. **点击行尾圆点**：list `hit_row` 是按整行命中；圆点也算行内。点圆点等于点这一行 → SwitchTab(idx)。OK。
5. **dpi 切换**：style.row_h_logical 等都未乘 dpi；list paint 内部乘 ctx.dpi。一致。
6. **list 子 widget 与父 widget 的 hit/分发**：父 widget 在 on_event 里**显式**调 list.on_event，而不是把 list 当作"独立 dock 子"——避免 dispatch 路径被 list 抢命中。这个嵌套模式后续可以在 popup 复用。
7. **滚动**：本阶段不做。如果 sidebar 文件超 ~30 个被截断，是预期行为。
