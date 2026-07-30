# Button + ListWidget 基础模块 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 button.rs（通用 Button Widget），将 VerticalListWidget 重构为 ListWidget（支持 Vertical/Horizontal 方向 + 新 item 字段）。

**Architecture:** Button 是独立 Widget 实现，负责 hover/active 绘制和 click 事件。ListWidget 在现有 VerticalListWidget 上直接修改：加 Orientation 枚举、ListItem 新字段，item_rect/paint/hit/scroll 按方向分支。

**Tech Stack:** Rust, existing widget trait system, draw_icon SVG renderer, harfbuzz text shaping

---

### Task 1: Create button.rs

**Files:**
- Create: `crates/ui/src/widgets/button.rs`

- [ ] **Step 1: Write button.rs**

```rust
//! Button Widget — icon + optional text label.
//! Icon and text are both optional; whichever is set gets drawn.

use crate::core::{Widget, Rect, LayoutCtx, PaintCtx, EventCtx, Event, MouseButton, WidgetAction};
use crate::widgets::icon::draw_icon;
use std::any::Any;

/// Visual style for a Button.
#[derive(Clone, Debug)]
pub struct ButtonStyle {
    pub font_size_logical: f32,
    pub pad_x_logical: f32,
    pub fg: [f32; 4],
    pub hover_bg: [f32; 4],
}

/// Action emitted by Button on click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ButtonAction {
    Click,
}

pub struct Button {
    rect: Rect,
    icon: Option<String>,
    icon_size_logical: f32,
    text: Option<String>,
    style: ButtonStyle,
    hovered: bool,
    is_active: bool,
}

impl Button {
    pub fn new(style: ButtonStyle) -> Self {
        Self {
            rect: Rect::ZERO,
            icon: None,
            icon_size_logical: crate::constants::BUTTON_SIZE,
            text: None,
            style,
            hovered: false,
            is_active: false,
        }
    }

    pub fn set_icon(&mut self, name: Option<String>) { self.icon = name; }
    pub fn set_text(&mut self, text: Option<String>) { self.text = text; }
    pub fn set_active(&mut self, active: bool) { self.is_active = active; }
    pub fn set_icon_size(&mut self, sz: f32) { self.icon_size_logical = sz; }
    pub fn set_style(&mut self, s: ButtonStyle) { self.style = s; }
    pub fn rect(&self) -> Rect { self.rect }
}

impl Widget for Button {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        let alpha = ctx.global_alpha;

        // Background on hover or active
        if self.hovered || self.is_active {
            let mut bg = self.style.hover_bg;
            bg[3] *= alpha;
            ctx.list.fill_rounded(self.rect, bg, 4.0 * dpi);
        }

        let font_size = self.style.font_size_logical * dpi;
        let icon_size = self.icon_size_logical * dpi;
        let pad_x = self.style.pad_x_logical * dpi;
        let mut fg = self.style.fg;
        fg[3] *= alpha;

        let icon_gap = 4.0 * dpi;
        let mut cursor_x = self.rect.x + pad_x;

        if let Some(ref icon_name) = self.icon {
            let icon_y = self.rect.y + (self.rect.h - icon_size) * 0.5;
            draw_icon(ctx.list, icon_name, cursor_x, icon_y, icon_size, fg);
            cursor_x += icon_size + icon_gap;
        }

        if let Some(ref text) = self.text {
            let baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(cursor_x, baseline, font_size, fg, text, shaper);
            }
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            Event::MouseMove { px, py } => {
                let inside = self.rect.contains(*px, *py);
                if inside {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                if inside != self.hovered {
                    self.hovered = inside;
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            Event::MouseDown { button: MouseButton::Left, .. } => {
                if self.hovered {
                    Some(WidgetAction::Button(ButtonAction::Click))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::paint::{DrawList, DrawCmd};
    use crate::core::measure::NoopMeasure;
    use crate::core::widget::LayoutCtx;
    use crate::Theme;

    fn test_theme() -> Theme {
        Theme::dark()
    }

    fn test_style() -> ButtonStyle {
        ButtonStyle {
            font_size_logical: 13.0,
            pad_x_logical: 8.0,
            fg: [0.9, 0.9, 0.9, 1.0],
            hover_bg: [0.3, 0.3, 0.3, 0.5],
        }
    }

    fn make_button(rect: Rect) -> Button {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &theme, dpi: 1.0 };
        let mut b = Button::new(test_style());
        b.set_rect(rect, &mut lc);
        b
    }

    #[test]
    fn paint_text_only_emits_text() {
        let theme = test_theme();
        let mut b = make_button(Rect::new(0.0, 0.0, 100.0, 28.0));
        b.set_text(Some("Hello".into()));
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0), shaper: Some(&mut shaper) };
        b.paint(&mut pc);
        let text_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).count();
        assert_eq!(text_count, 1);
    }

    #[test]
    fn paint_icon_only_emits_triangle() {
        let theme = test_theme();
        let mut b = make_button(Rect::new(0.0, 0.0, 32.0, 28.0));
        b.set_icon(Some("plus".into()));
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0), shaper: Some(&mut shaper) };
        b.paint(&mut pc);
        let tri_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillTriangle { .. })).count();
        assert!(tri_count > 0, "icon should emit fill triangles");
    }

    #[test]
    fn paint_hover_emits_bg() {
        let theme = test_theme();
        let mut b = make_button(Rect::new(0.0, 0.0, 100.0, 28.0));
        b.set_text(Some("X".into()));
        // Force hovered state
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        b.on_event(&Event::MouseMove { px: 50.0, py: 14.0 }, &mut ec);
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0), shaper: Some(&mut shaper) };
        b.paint(&mut pc);
        let rect_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).count();
        assert!(rect_count >= 1, "hover should emit background fill rect");
    }

    #[test]
    fn paint_active_emits_bg() {
        let theme = test_theme();
        let mut b = make_button(Rect::new(0.0, 0.0, 100.0, 28.0));
        b.set_active(true);
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx { global_alpha: 1.0, list: &mut dl, theme: &theme, dpi: 1.0, offset: (0.0, 0.0), shaper: Some(&mut shaper) };
        b.paint(&mut pc);
        let rect_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).count();
        assert!(rect_count >= 1, "active should emit background fill rect");
    }

    #[test]
    fn click_emits_button_action() {
        let theme = test_theme();
        let mut b = make_button(Rect::new(0.0, 0.0, 100.0, 28.0));
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        // First hover
        b.on_event(&Event::MouseMove { px: 50.0, py: 14.0 }, &mut ec);
        // Then click
        let result = b.on_event(&Event::MouseDown { px: 50.0, py: 14.0, button: MouseButton::Left }, &mut ec);
        assert!(matches!(result, Some(WidgetAction::Button(ButtonAction::Click))));
    }

    #[test]
    fn click_outside_no_action() {
        let theme = test_theme();
        let mut b = make_button(Rect::new(0.0, 0.0, 100.0, 28.0));
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let result = b.on_event(&Event::MouseDown { px: 200.0, py: 200.0, button: MouseButton::Left }, &mut ec);
        assert!(result.is_none());
    }

    #[test]
    fn hit_contains() {
        let b = make_button(Rect::new(10.0, 10.0, 80.0, 20.0));
        assert!(b.hit(50.0, 20.0));
        assert!(!b.hit(5.0, 20.0));
    }

    #[test]
    fn mouse_move_updates_hovered() {
        let theme = test_theme();
        let mut b = make_button(Rect::new(0.0, 0.0, 100.0, 28.0));
        let mut ec = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let r = b.on_event(&Event::MouseMove { px: 50.0, py: 14.0 }, &mut ec);
        assert!(r.is_some()); // Consumed on hover state change
        assert!(matches!(r.unwrap(), WidgetAction::Consumed));
    }
}
```

### Task 2: Register button module and add ButtonAction to WidgetAction

**Files:**
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/core/widget.rs`

- [ ] **Step 1: Add `pub mod button;` to widgets/mod.rs**

Add after `pub mod icon;`:

```rust
pub mod button;
```

- [ ] **Step 2: Add Button variant to WidgetAction enum in core/widget.rs**

After the `StatusBar` variant, add:

```rust
Button(crate::widgets::button::ButtonAction),
```

- [ ] **Step 3: Build check**

Run: `cargo build -p ui 2>&1 | head -30`
Expected: compiles successfully (warnings OK)

- [ ] **Step 4: Run button tests**

Run: `cargo test -p ui -- widgets::button 2>&1 | tail -20`
Expected: all 8 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/button.rs crates/ui/src/widgets/mod.rs crates/ui/src/core/widget.rs
git commit -m "feat(ui): add Button widget with optional icon and text"
```

---

### Task 3: Refactor list.rs — rename + add Orientation

**Files:**
- Modify: `crates/ui/src/widgets/list.rs`

- [ ] **Step 1: Add Orientation enum at top of list.rs**

Insert after the module doc comment (after the `use` block):

```rust
/// Layout direction for the list.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}
```

- [ ] **Step 2: Add new fields to ListItem**

Update `ListItem` struct to include `icon`, `extra_label`, `is_active`, `closeable`:

```rust
#[derive(Clone, Debug, Default)]
pub struct ListItem {
    pub label: String,
    pub kind: ListItemKind,
    pub icon: Option<String>,
    pub indicator: ListItemIndicator,
    pub pinned: bool,
    pub extra_label: Option<String>,
    pub is_active: bool,
    pub closeable: bool,
}
```

- [ ] **Step 3: Add item_w_logical to ListStyle**

```rust
pub struct ListStyle {
    pub row_h_logical: f32,
    pub item_w_logical: f32,       // Horizontal 模式列宽（Vertical 忽略）
    pub pad_x_logical: f32,
    pub pad_y_logical: f32,
    pub font_size_logical: f32,
    pub bg: [f32; 4],
    pub item_active_bg: [f32; 4],
    pub item_hover_bg: [f32; 4],
    pub item_fg: [f32; 4],
    pub item_accent: [f32; 4],
    pub separator: [f32; 4],
    pub indicator_color: [f32; 4],
}
```

- [ ] **Step 4: Rename struct, add orientation field**

Rename `VerticalListWidget` → `ListWidget`:

```rust
pub struct ListWidget {
    rect: Rect,
    items: Vec<ListItem>,
    active_index: Option<usize>,
    hovered_index: Option<usize>,
    close_hovered: bool,
    style: ListStyle,
    scroll_offset: f32,
    orientation: Orientation,
    truncated_labels: Vec<String>,
    truncated_label_widths: Vec<f32>,
}
```

- [ ] **Step 5: Update constructor**

```rust
impl ListWidget {
    pub fn new(style: ListStyle, orientation: Orientation) -> Self {
        Self {
            rect: Rect::ZERO,
            items: Vec::new(),
            active_index: None,
            hovered_index: None,
            close_hovered: false,
            style,
            scroll_offset: 0.0,
            orientation,
            truncated_labels: Vec::new(),
            truncated_label_widths: Vec::new(),
        }
    }
```

- [ ] **Step 6: Update item_rect for both orientations**

```rust
    pub(crate) fn item_rect(&self, i: usize, dpi: f32) -> Rect {
        match self.orientation {
            Orientation::Vertical => {
                let row_h = self.style.row_h_logical * dpi;
                let pad_y = self.style.pad_y_logical * dpi;
                let top = self.rect.y + pad_y + i as f32 * row_h;
                Rect::new(self.rect.x, top, self.rect.w, row_h)
            }
            Orientation::Horizontal => {
                let col_w = self.style.item_w_logical * dpi;
                let pad_x = self.style.pad_x_logical * dpi;
                let left = self.rect.x + pad_x + i as f32 * col_w;
                Rect::new(left, self.rect.y, col_w, self.rect.h)
            }
        }
    }
```

- [ ] **Step 7: Update hit_row for both orientations**

Replace `shifted_py` logic:

```rust
    pub(crate) fn hit_row(&self, px: f32, py: f32, scroll_offset: f32, dpi: f32) -> Option<usize> {
        if !self.rect.contains(px, py) { return None; }
        match self.orientation {
            Orientation::Vertical => {
                let shifted_py = py + scroll_offset;
                for (i, item) in self.items.iter().enumerate() {
                    let r = self.item_rect(i, dpi);
                    if r.contains(px, shifted_py) {
                        return matches!(item.kind, ListItemKind::Normal).then_some(i);
                    }
                }
            }
            Orientation::Horizontal => {
                let shifted_px = px + scroll_offset;
                for (i, item) in self.items.iter().enumerate() {
                    let r = self.item_rect(i, dpi);
                    if Rect::new(r.x, self.rect.y, r.w, self.rect.h).contains(shifted_px, py) {
                        return matches!(item.kind, ListItemKind::Normal).then_some(i);
                    }
                }
            }
        }
        None
    }
```

- [ ] **Step 8: Update hit_close_btn for both orientations**

Replace the inner rect check:

```rust
    pub(crate) fn hit_close_btn(&self, px: f32, py: f32, scroll_offset: f32, dpi: f32) -> Option<usize> {
        if !self.rect.contains(px, py) { return None; }
        let pad_x = self.style.pad_x_logical * dpi;
        let btn_size = CLOSE_BTN_SIZE_LOGICAL * dpi;
        for (i, item) in self.items.iter().enumerate() {
            if !item.closeable || item.kind != ListItemKind::Normal { continue; }
            if self.hovered_index != Some(i) { continue; }
            let row_rect = self.item_rect(i, dpi);
            let hit_contains = match self.orientation {
                Orientation::Vertical => {
                    let shifted_py = py + scroll_offset;
                    row_rect.contains(px, shifted_py)
                }
                Orientation::Horizontal => {
                    let shifted_px = px + scroll_offset;
                    Rect::new(self.rect.x, row_rect.y, self.rect.w, row_rect.h).contains(shifted_px, py)
                }
            };
            if !hit_contains { continue; }
            let btn_x = row_rect.x + row_rect.w - pad_x - btn_size;
            let btn_y = row_rect.y + (row_rect.h - btn_size) * 0.5;
            let hit_pad = CLOSE_BTN_HIT_PAD_LOGICAL * dpi;
            let btn_rect = Rect::new(btn_x - hit_pad, btn_y - hit_pad, btn_size + hit_pad * 2.0, btn_size + hit_pad * 2.0);
            let btn_hit = match self.orientation {
                Orientation::Vertical => btn_rect.contains(px, py + scroll_offset),
                Orientation::Horizontal => btn_rect.contains(px + scroll_offset, py),
            };
            if btn_hit {
                return Some(i);
            }
        }
        None
    }
```

- [ ] **Step 9: Update set_rect for orientation-aware truncation**

In `fn set_rect`, compute truncated labels differently for each orientation:

```rust
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let dpi = ctx.dpi;
        let font_size = self.style.font_size_logical * dpi;
        let measure: &mut dyn TextMeasure = ctx.measure;
        let pad_x = self.style.pad_x_logical * dpi;
        let dot_r = (font_size * 0.18).max(2.0);
        self.truncated_labels = self.items.iter().map(|item| {
            if !matches!(item.kind, ListItemKind::Normal | ListItemKind::Header) {
                return String::new();
            }
            let left_offset = self.pinned_left_offset(item, dpi);
            let dot_extra = if matches!(item.indicator, ListItemIndicator::Dot) {
                dot_r * 2.0 + 4.0 * dpi
            } else { 0.0 };
            let icon_extra = if item.icon.is_some() { (self.style.font_size_logical + 4.0) * dpi } else { 0.0 };
            let close_extra = if item.closeable {
                (CLOSE_BTN_SIZE_LOGICAL + CLOSE_BTN_LABEL_GAP_LOGICAL) * dpi
            } else { 0.0 };
            let row_w = match self.orientation {
                Orientation::Vertical => rect.w,
                Orientation::Horizontal => self.style.item_w_logical * dpi,
            };
            let label_max_w = (row_w - pad_x * 2.0 - left_offset - dot_extra - icon_extra - close_extra).max(0.0);
            truncate_title_precise(&item.label, label_max_w, font_size, measure)
        }).collect();
        self.truncated_label_widths = self.truncated_labels.iter().map(|label| {
            if label.is_empty() { 0.0 } else { measure.measure(label, font_size) }
        }).collect();
    }
```

- [ ] **Step 10: Update Widget impl for orientation-aware paint**

Replace the entire `fn paint` with:

```rust
    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 { return; }

        let alpha = ctx.global_alpha;

        if self.style.bg[3] > 0.0 {
            let mut bg = self.style.bg;
            bg[3] *= alpha;
            ctx.list.fill(Rect::new(self.rect.x, self.rect.y, self.rect.w, self.rect.h), bg);
        }

        let dpi = ctx.dpi;
        let pad_x = self.style.pad_x_logical * dpi;
        let font_size = self.style.font_size_logical * dpi;

        for (i, item) in self.items.iter().enumerate() {
            let row_rect = self.item_rect(i, dpi);

            match item.kind {
                ListItemKind::Separator => {
                    let mut sep = self.style.separator;
                    sep[3] *= alpha;
                    match self.orientation {
                        Orientation::Vertical => {
                            let sep_h = (1.0 * dpi).max(1.0);
                            let y = row_rect.y + (row_rect.h - sep_h) * 0.5;
                            ctx.list.fill(
                                Rect::new(row_rect.x + pad_x, y, row_rect.w - pad_x * 2.0, sep_h),
                                sep,
                            );
                        }
                        Orientation::Horizontal => {
                            let sep_w = (1.0 * dpi).max(1.0);
                            let x = row_rect.x + (row_rect.w - sep_w) * 0.5;
                            ctx.list.fill(
                                Rect::new(x, row_rect.y + pad_x, sep_w, row_rect.h - pad_x * 2.0),
                                sep,
                            );
                        }
                    }
                    continue;
                }
                ListItemKind::Header | ListItemKind::Normal => {
                    let is_active = item.is_active || Some(i) == self.active_index;
                    let is_hovered = Some(i) == self.hovered_index;

                    if matches!(item.kind, ListItemKind::Normal) {
                        if is_active {
                            let mut color = self.style.item_active_bg;
                            color[3] *= alpha;
                            ctx.list.fill_menu_hover(row_rect, color, dpi);
                        } else if is_hovered {
                            let mut color = self.style.item_hover_bg;
                            color[3] *= alpha;
                            ctx.list.fill_menu_hover(row_rect, color, dpi);
                        }
                    }

                    // Pin bar
                    if item.pinned {
                        let bar_len = PIN_BAR_WIDTH_LOGICAL * dpi;
                        let bar_pad = 8.0 * dpi;
                        let mut bar_color = self.style.item_accent;
                        bar_color[3] *= alpha;
                        match self.orientation {
                            Orientation::Vertical => {
                                let bar_x = row_rect.x + pad_x;
                                ctx.list.fill_rounded(
                                    Rect::new(bar_x, row_rect.y + bar_pad, bar_len, row_rect.h - bar_pad * 2.0),
                                    bar_color,
                                    bar_len * 0.5,
                                );
                            }
                            Orientation::Horizontal => {
                                let bar_y = row_rect.y + pad_x;
                                ctx.list.fill_rounded(
                                    Rect::new(row_rect.x + bar_pad, bar_y, row_rect.w - bar_pad * 2.0, bar_len),
                                    bar_color,
                                    bar_len * 0.5,
                                );
                            }
                        }
                    }

                    let baseline = row_rect.y + row_rect.h * 0.5 + font_size * 0.35;
                    let mut fg = self.style.item_fg;
                    fg[3] *= alpha;
                    let left_offset = self.pinned_left_offset(item, dpi);
                    let icon_extra = if item.icon.is_some() { (self.style.font_size_logical + 4.0) * dpi } else { 0.0 };
                    let mut text_x = row_rect.x + pad_x + left_offset;

                    // Icon
                    if let Some(ref icon_name) = item.icon {
                        let icon_sz = font_size;
                        let icon_y = row_rect.y + (row_rect.h - icon_sz) * 0.5;
                        crate::widgets::icon::draw_icon(ctx.list, icon_name, text_x, icon_y, icon_sz, fg);
                        text_x += icon_extra;
                    }

                    // Label
                    let label = self.truncated_labels.get(i)
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .unwrap_or_else(|| item.label.clone());
                    if let Some(ref mut shaper) = ctx.shaper {
                        ctx.list.text_shaped(text_x, baseline, font_size, fg, &label, shaper);
                    }

                    // Indicator dot
                    if matches!(item.indicator, ListItemIndicator::Dot) {
                        let mut ind = self.style.indicator_color;
                        ind[3] *= alpha;
                        let actual_w = self.truncated_label_widths.get(i).copied().unwrap_or(0.0);
                        if let Some(ref mut shaper) = ctx.shaper {
                            ctx.list.text_shaped(
                                text_x + actual_w + 2.0 * dpi,
                                baseline, font_size,
                                ind, "*", shaper);
                        }
                    }

                    // Extra label (right side)
                    if let Some(ref extra) = item.extra_label {
                        let extra_x = row_rect.x + row_rect.w - pad_x - 40.0 * dpi;
                        if let Some(ref mut shaper) = ctx.shaper {
                            ctx.list.text_shaped(extra_x, baseline, font_size, fg, extra, shaper);
                        }
                    }

                    // Close button on hovered closeable items
                    if item.closeable && is_hovered {
                        let btn_size = CLOSE_BTN_SIZE_LOGICAL * dpi;
                        let close_fg = [0.4, 0.4, 0.4, alpha * 0.9];
                        if let Some(ref mut shaper) = ctx.shaper {
                            ctx.list.text_shaped(
                                row_rect.x + row_rect.w - pad_x - btn_size * 0.5,
                                baseline, font_size,
                                close_fg, "x", shaper);
                        }
                    }
                }
            }
        }
    }
```

- [ ] **Step 11: Update on_event — replace `item.pinned` with `item.closeable`**

In `hit_close_btn` it's already updated above. No other changes needed in `on_event`.

- [ ] **Step 12: Build check**

Run: `cargo build -p ui 2>&1 | head -50`
Expected: compilation errors in sidebar/mod.rs and tests (missing new fields in ListItem/ListStyle)

---

### Task 4: Fix compilation errors in sidebar and tests

**Files:**
- Modify: `crates/ui/src/widgets/sidebar/mod.rs`
- Modify: `crates/ui/src/widgets/list.rs` (test helpers)

- [ ] **Step 1: Fix sidebar/mod.rs imports and ListStyle construction**

Change the import line:
```rust
use crate::widgets::list::{
    ListWidget, ListItem, ListItemKind, ListItemIndicator,
    ListStyle, Orientation,
};
```

Change the struct field:
```rust
    pub(crate) list: ListWidget,
```

Change `make_style_from_theme`:
```rust
fn make_style_from_theme(theme: &crate::theme::Theme) -> ListStyle {
    ListStyle {
        row_h_logical: crate::constants::ROW_HEIGHT,
        item_w_logical: 140.0,
        pad_x_logical: 12.0,
        pad_y_logical: 0.0,
        font_size_logical: 15.0,
        bg: [0.0, 0.0, 0.0, 0.0],
        item_active_bg: theme.menu_hover,
        item_hover_bg: theme.menu_hover,
        item_fg: theme.sidebar_item_fg,
        item_accent: theme.sidebar_accent,
        separator: theme.menu_separator,
        indicator_color: theme.foreground,
    }
}
```

Change the ListWidget construction in `SidebarWidget::new`:
```rust
        let list = ListWidget::new(ListStyle {
            row_h_logical: crate::constants::ROW_HEIGHT,
            item_w_logical: 140.0,
            pad_x_logical: 12.0, pad_y_logical: 0.0,
            font_size_logical: 15.0,
            bg: [0.0; 4],
            item_active_bg: [0.0; 4],
            item_hover_bg: [0.0; 4],
            item_fg: [0.0; 4],
            item_accent: [0.0; 4],
            separator: [0.0; 4],
            indicator_color: [0.0; 4],
        }, Orientation::Vertical);
```

Change ListItem construction in `set_rect`:
```rust
            self.list_items = self.tabs.iter().map(|t| ListItem {
                label: t.title.clone(),
                kind: ListItemKind::Normal,
                icon: None,
                indicator: if t.is_dirty { ListItemIndicator::Dot } else { ListItemIndicator::None },
                pinned: t.pinned,
                extra_label: None,
                is_active: false,
                closeable: !t.pinned,
            }).collect();
```

- [ ] **Step 2: Fix list.rs test helpers**

Update `style()`:
```rust
    fn style() -> ListStyle {
        ListStyle {
            row_h_logical: 24.0,
            item_w_logical: 120.0,
            pad_x_logical: 8.0, pad_y_logical: 4.0,
            font_size_logical: 13.0,
            bg: [0.1, 0.1, 0.1, 1.0],
            item_active_bg: [0.2; 4],
            item_hover_bg: [0.15; 4],
            item_fg: [0.9; 4],
            item_accent: [0.5, 0.5, 0.8, 1.0],
            separator: [0.3; 4],
            indicator_color: [1.0, 0.5, 0.0, 1.0],
        }
    }
```

Update `make_list`:
```rust
    fn make_list(items: Vec<ListItem>, rect: Rect) -> ListWidget {
        let theme = Theme::dark();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(items);
        w.set_rect(rect, &mut layout);
        w
    }
```

Update `item` helper:
```rust
    fn item(label: &str) -> ListItem {
        ListItem { label: label.into(), kind: ListItemKind::Normal, icon: None, indicator: ListItemIndicator::None, pinned: false, extra_label: None, is_active: false, closeable: true }
    }
```

Update `pinned_item` helper:
```rust
    fn pinned_item(label: &str) -> ListItem {
        ListItem { label: label.into(), kind: ListItemKind::Normal, icon: None, indicator: ListItemIndicator::None, pinned: true, extra_label: None, is_active: false, closeable: false }
    }
```

Update all test functions that reference `VerticalListWidget` → `ListWidget`. Replace ALL occurrences of `VerticalListWidget::new(style())` with `ListWidget::new(style(), Orientation::Vertical)`, and `VerticalListWidget::new(s)` with `ListWidget::new(s, Orientation::Vertical)`.

Update the Separator test item construction:
```rust
                ListItem { label: "".into(), kind: ListItemKind::Separator, icon: None, indicator: ListItemIndicator::None, pinned: false, extra_label: None, is_active: false, closeable: false },
```

Update the indicator dot test item:
```rust
        w.set_items(vec![ListItem {
            label: "x".into(),
            kind: ListItemKind::Normal,
            icon: None,
            indicator: ListItemIndicator::Dot,
            pinned: false,
            extra_label: None,
            is_active: false,
            closeable: true,
        }]);
```

Update the pinned dirty dot test item similarly.

Also update `hit_close_btn` tests that check `item.pinned` — they now check `item.closeable`. The logic: pinned items have `closeable: false`, so they're covered.

- [ ] **Step 3: Fix widget_tests.rs sidebar tests**

Run: `grep -n 'ListItemIndicator' crates/ui/src/widgets/sidebar/widget_tests.rs`

If any test constructs a ListItem directly with old fields, update it.

- [ ] **Step 4: Fix lib.rs exports**

Update `crates/ui/src/lib.rs`:
```rust
pub use widgets::list::{
    ListWidget, ListItem, ListItemKind, ListItemIndicator,
    ListStyle, ListAction, Orientation,
};
```

- [ ] **Step 5: Build check**

Run: `cargo build -p ui 2>&1 | tail -30`
Expected: compiles successfully

- [ ] **Step 6: Run all existing list tests**

Run: `cargo test -p ui -- widgets::list 2>&1 | tail -30`
Expected: all 20+ tests pass

- [ ] **Step 7: Run sidebar tests**

Run: `cargo test -p ui -- widgets::sidebar 2>&1 | tail -30`
Expected: all sidebar tests pass

- [ ] **Step 8: Commit**

```bash
git add crates/ui/src/widgets/list.rs crates/ui/src/widgets/sidebar/mod.rs crates/ui/src/widgets/sidebar/widget_tests.rs crates/ui/src/lib.rs
git commit -m "refactor(ui): rename VerticalListWidget to ListWidget, add Orientation and new ListItem fields"
```

---

### Task 5: Add Horizontal orientation tests

**Files:**
- Modify: `crates/ui/src/widgets/list.rs` (test module)

- [ ] **Step 1: Add Horizontal item_rect test**

```rust
    #[test]
    fn horizontal_item_rects_layout_left_to_right() {
        let theme = Theme::dark();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Horizontal);
        w.set_items(vec![item("a"), item("b"), item("c")]);
        w.set_rect(Rect::new(0.0, 0.0, 400.0, 28.0), &mut layout);

        let r0 = w.item_rect(0, 1.0);
        let r1 = w.item_rect(1, 1.0);
        let r2 = w.item_rect(2, 1.0);

        assert_eq!(r0.x, 8.0); // pad_x
        assert_eq!(r1.x, 8.0 + 120.0); // pad_x + item_w
        assert_eq!(r2.x, 8.0 + 240.0);
        assert_eq!(r0.h, 28.0);
    }
```

- [ ] **Step 2: Add Horizontal hit test**

```rust
    #[test]
    fn horizontal_hit_row() {
        let theme = Theme::dark();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Horizontal);
        w.set_items(vec![item("a"), item("b")]);
        w.set_rect(Rect::new(0.0, 0.0, 400.0, 28.0), &mut layout);

        // First item center
        assert_eq!(w.hit_row(8.0 + 60.0, 14.0, 0.0, 1.0), Some(0));
        // Second item center
        assert_eq!(w.hit_row(8.0 + 120.0 + 60.0, 14.0, 0.0, 1.0), Some(1));
    }
```

- [ ] **Step 3: Run new tests**

Run: `cargo test -p ui -- widgets::list 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/list.rs
git commit -m "test(ui): add Horizontal orientation tests for ListWidget"
```

---

### Task 6: Run full test suite

**Files:** (none — verification only)

- [ ] **Step 1: Run all ui crate tests**

Run: `cargo test -p ui 2>&1 | tail -40`
Expected: all tests pass

- [ ] **Step 2: Run full workspace tests**

Run: `cargo test 2>&1 | tail -40`
Expected: all tests pass (or same failures as before this branch)

---

### Task 7: Cleanup — verify no VerticalListWidget references remain

**Files:** (none — verification only)

- [ ] **Step 1: Search for stale references**

Run: `grep -rn 'VerticalListWidget' crates/ --include='*.rs'`
Expected: no results

- [ ] **Step 2: Search for stale test helpers**

Run: `grep -rn 'CLOSE_BTN_SIZE_LOGICAL\|CLOSE_BTN_HIT_PAD_LOGICAL\|CLOSE_BTN_LABEL_GAP_LOGICAL' crates/ui/src/ --include='*.rs'`
Expected: only in list.rs (correct)
```
