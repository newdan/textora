# Tooltip Component Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract inline tooltip rendering from SearchBarWidget into a general-purpose tooltip system via `Widget::tooltip_at()` trait method, with delayed hover display and 4-direction auto-flip positioning.

**Architecture:** Widget trait gains `tooltip_at(&self, px, py) -> Option<TooltipHint>` defaulting to None. UiShell walks dock children after MouseMove dispatch, captures hints, runs a 400ms timer, and manages a dedicated tooltip overlay slot. TooltipWidget handles positioning and rendering. Three widgets adopt it: SearchBar (buttons), TabBar (truncated titles), StatusBar (toggle button).

**Tech Stack:** Rust, winit, existing ui/widget framework (no new dependencies)

---

### Task 1: Add tooltip colors to Theme

**Files:**
- Modify: `crates/ui/src/theme.rs` — add 3 fields, update 5 constructors, update `gamma_correct`
- Modify: `crates/ui/src/core/dock.rs` — update `dummy_theme()` in tests

- [ ] **Step 1: Add fields to Theme struct**

In `crates/ui/src/theme.rs`, add after `menu_text`:

```rust
    /// Tooltip background
    pub tooltip_bg: [f32; 4],
    /// Tooltip text color
    pub tooltip_fg: [f32; 4],
    /// Tooltip border color
    pub tooltip_border: [f32; 4],
```

- [ ] **Step 2: Add values to `Theme::dark()`**

In `Theme::dark()`, after `menu_text`:

```rust
            tooltip_bg: [0.15, 0.15, 0.15, 0.92],
            tooltip_fg: [0.9, 0.9, 0.9, 1.0],
            tooltip_border: [0.35, 0.35, 0.35, 0.6],
```

- [ ] **Step 3: Add values to `Theme::light()`**

In `Theme::light()`, after `menu_text`:

```rust
            tooltip_bg: [0.92, 0.92, 0.92, 0.92],
            tooltip_fg: [0.15, 0.15, 0.15, 1.0],
            tooltip_border: [0.55, 0.55, 0.55, 0.4],
```

- [ ] **Step 4: Add values to `Theme::claude_light()`**

In `Theme::claude_light()`, after `t.menu_text = text_main;`:

```rust
        t.tooltip_bg = [0.15, 0.15, 0.15, 0.88];
        t.tooltip_fg = [0.95, 0.95, 0.95, 1.0];
        t.tooltip_border = [0.35, 0.35, 0.35, 0.5];
```

- [ ] **Step 5: Add values to `Theme::claude_dark()`**

In `Theme::claude_dark()`, after `t.menu_text = text_main;`:

```rust
        t.tooltip_bg = [0.12, 0.12, 0.12, 0.92];
        t.tooltip_fg = [0.9, 0.9, 0.9, 1.0];
        t.tooltip_border = [0.3, 0.3, 0.3, 0.5];
```

- [ ] **Step 6: Add to `test_theme()`**

In `test_theme()`, after `menu_text: [0.0; 4],`:

```rust
        tooltip_bg: [0.0; 4],
        tooltip_fg: [0.0; 4],
        tooltip_border: [0.0; 4],
```

- [ ] **Step 7: Add to `gamma_correct()`**

In the array of color references within `gamma_correct()`, add after `self.menu_text`:

```rust
                   &mut self.tooltip_bg, &mut self.tooltip_fg, &mut self.tooltip_border] {
```

- [ ] **Step 8: Add to `dummy_theme()` in dock.rs tests**

In `crates/ui/src/core/dock.rs`, inside `dummy_theme()` after `menu_text: [0.0; 4],`:

```rust
            tooltip_bg: [0.0; 4],
            tooltip_fg: [0.0; 4],
            tooltip_border: [0.0; 4],
```

- [ ] **Step 9: Verify compilation**

Run: `cargo check -p ui 2>&1`
Expected: no errors

- [ ] **Step 10: Commit**

```bash
git add crates/ui/src/theme.rs crates/ui/src/core/dock.rs
git commit -m "feat: add tooltip colors to Theme"
```

---

### Task 2: Add `tooltip_at` default method to Widget trait

**Files:**
- Modify: `crates/ui/src/core/widget.rs`

- [ ] **Step 1: Import TooltipHint type**

Add after existing imports in `widget.rs`:

```rust
use crate::widgets::tooltip::TooltipHint;
```

- [ ] **Step 2: Add default method to Widget trait**

In the `Widget` trait, add after `is_capturing()`:

```rust
    /// Return a tooltip hint if (px, py) in widget-local coordinates
    /// hovers over a sub-region that should show a tooltip.
    fn tooltip_at(&self, _px: f32, _py: f32) -> Option<TooltipHint> {
        None
    }
```

Note: Tasks 2-4 form a circular dependency (widget.rs ← tooltip.rs → widget.rs). Write all three then verify together.

- [ ] **Step 3: Commit** (after Tasks 2-4 together)

---

### Task 3: Create TooltipWidget

**Files:**
- Create: `crates/ui/src/widgets/tooltip.rs`

- [ ] **Step 1: Write the file**

```rust
//! Tooltip overlay widget — renders a single-line label pill with
//! auto-positioning relative to a target rect.

use crate::core::geom::Rect;
use crate::core::text_util::estimate_text_width_px;
use crate::core::widget::{Event, EventCtx, LayoutCtx, PaintCtx, Widget};
use std::any::Any;

/// A tooltip hint from a widget: label text + target rectangle
/// in widget-local coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipHint {
    pub label: String,
    pub target_rect: Rect,
}

/// Tooltip overlay widget. Computes screen-space rect during construction;
/// paint draws a dark rounded pill with border at (0,0) within its rect.
pub struct TooltipWidget {
    label: String,
    local_rect: Rect,
}

impl TooltipWidget {
    /// Create a new tooltip widget and its screen-space layout rect.
    /// `hint.target_rect` must be in screen coordinates.
    /// Returns `(widget, screen_rect)` where `screen_rect` is the overlay position.
    pub fn new(hint: &TooltipHint, dpi: f32, screen_w: f32, screen_h: f32) -> (Self, Rect) {
        let font_size = 11.0 * dpi;
        let pad_x = 6.0 * dpi;
        let pad_y = 3.0 * dpi;
        let gap = 4.0 * dpi;
        let screen_inset = 8.0 * dpi;

        let text_w = estimate_text_width_px(&hint.label, font_size);
        let tip_w = text_w + pad_x * 2.0;
        let tip_h = font_size + pad_y * 2.0;

        let target = &hint.target_rect;

        // Horizontal: centered on target, clamped to screen
        let mut tip_x = target.x + target.w * 0.5 - tip_w * 0.5;
        tip_x = tip_x.max(screen_inset).min(screen_w - tip_w - screen_inset);

        // Vertical: prefer below, flip above if not enough space
        let below_y = target.y + target.h + gap;
        let above_y = target.y - tip_h - gap;
        let fits_below = below_y + tip_h <= screen_h - screen_inset;
        let fits_above = above_y >= screen_inset;

        let tip_y = if fits_below {
            below_y
        } else if fits_above {
            above_y
        } else {
            let space_below = screen_h - target.y - target.h;
            let space_above = target.y;
            if space_below >= space_above { below_y } else { above_y }
        };

        let screen_rect = Rect::new(tip_x, tip_y, tip_w, tip_h);
        let widget = Self {
            label: hint.label.clone(),
            local_rect: Rect::new(0.0, 0.0, tip_w, tip_h),
        };
        (widget, screen_rect)
    }
}

impl Widget for TooltipWidget {
    fn set_rect(&mut self, _rect: Rect, _ctx: &mut LayoutCtx) {}

    fn paint(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        let font_size = 11.0 * dpi;
        let pad_x = 6.0 * dpi;
        let pad_y = 3.0 * dpi;
        let r = &self.local_rect;

        // Background
        ctx.list.fill_rounded(*r, ctx.theme.tooltip_bg, 4.0 * dpi);

        // Border (4 thin fills)
        let border = ctx.theme.tooltip_border;
        ctx.list.fill_rounded(Rect::new(r.x, r.y, r.w, 1.0), border, 0.0);
        ctx.list.fill_rounded(Rect::new(r.x, r.y + r.h - 1.0, r.w, 1.0), border, 0.0);
        ctx.list.fill_rounded(Rect::new(r.x, r.y, 1.0, r.h), border, 0.0);
        ctx.list.fill_rounded(Rect::new(r.x + r.w - 1.0, r.y, 1.0, r.h), border, 0.0);

        // Text
        ctx.list.text(
            r.x + pad_x,
            r.y + pad_y + font_size * 0.8,
            font_size,
            ctx.theme.tooltip_fg,
            &self.label,
        );
    }

    fn hit(&self, _px: f32, _py: f32) -> bool {
        false
    }

    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<crate::core::widget::WidgetAction> {
        None
    }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

---

### Task 4: Register tooltip module

**Files:**
- Modify: `crates/ui/src/widgets/mod.rs`

- [ ] **Step 1: Add module declaration**

Add after `pub mod text_box;`:

```rust
pub mod tooltip;
```

- [ ] **Step 2: Verify compilation (Tasks 2-4 together)**

Run: `cargo check -p ui 2>&1`
Expected: no errors

- [ ] **Step 3: Commit**

```bash
git add crates/ui/src/core/widget.rs crates/ui/src/widgets/tooltip.rs crates/ui/src/widgets/mod.rs
git commit -m "feat: add TooltipHint type, TooltipWidget, and Widget::tooltip_at trait method"
```

---

### Task 5: Integrate tooltip into UiShell

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 1: Add imports**

Add at top of `ui_shell.rs`:

```rust
use ui::widgets::tooltip::{TooltipHint, TooltipWidget};
use std::time::Instant;
```

- [ ] **Step 2: Add TooltipTimer struct**

Add after existing imports, before `impl UiShell`:

```rust
struct TooltipTimer {
    hint: TooltipHint,
    target_screen_rect: ui::core::geom::Rect,
    start: Instant,
}
```

- [ ] **Step 3: Add fields to UiShell**

Add to `UiShell` struct, after `last_tabs_thickness`:

```rust
    tooltip_timer: Option<TooltipTimer>,
    tooltip_overlay: Option<OverlayChild>,
    screen_w: f32,
    screen_h: f32,
```

- [ ] **Step 4: Initialize in `UiShell::new()`**

Add in `UiShell::new()`:

```rust
            tooltip_timer: None,
            tooltip_overlay: None,
            screen_w: 0.0,
            screen_h: 0.0,
```

- [ ] **Step 5: Add `check_tooltips()` method**

Add to `impl UiShell`:

```rust
    fn check_tooltips(&mut self, px: f32, py: f32) {
        for child in self.dock.children.iter().rev() {
            if !child.visible || child.layout_rect.w <= 0.0 || child.layout_rect.h <= 0.0 {
                continue;
            }
            let lx = px - child.layout_rect.x;
            let ly = py - child.layout_rect.y;
            if let Some(hint) = child.widget.tooltip_at(lx, ly) {
                let screen_target = ui::core::geom::Rect::new(
                    hint.target_rect.x + child.layout_rect.x,
                    hint.target_rect.y + child.layout_rect.y,
                    hint.target_rect.w,
                    hint.target_rect.h,
                );
                let same = match &self.tooltip_timer {
                    Some(t) => t.hint.label == hint.label
                        && (t.target_screen_rect.x - screen_target.x).abs() < 0.5
                        && (t.target_screen_rect.y - screen_target.y).abs() < 0.5,
                    None => false,
                };
                if same {
                    return;
                }
                self.tooltip_overlay = None;
                self.tooltip_timer = Some(TooltipTimer {
                    hint,
                    target_screen_rect: screen_target,
                    start: Instant::now(),
                });
                return;
            }
        }
        self.tooltip_overlay = None;
        self.tooltip_timer = None;
    }

    fn update_tooltip(&mut self, dpi: f32) {
        if let Some(ref timer) = self.tooltip_timer {
            if timer.start.elapsed().as_millis() >= 400 {
                if self.tooltip_overlay.is_none() {
                    let (widget, layout_rect) = TooltipWidget::new(
                        &timer.hint, dpi, self.screen_w, self.screen_h,
                    );
                    self.tooltip_overlay = Some(OverlayChild {
                        widget: Box::new(widget),
                        layout_rect,
                    });
                }
            }
        }
    }
```

- [ ] **Step 6: Cache screen dims and call `update_tooltip()` in `update_frame()`**

In `update_frame()`, after `let screen_rect = ...`:

```rust
        self.screen_w = screen.w;
        self.screen_h = screen.h;
```

At end of `update_frame()`, after `self.frames_rendered += 1;`:

```rust
        self.update_tooltip(dpi);
```

- [ ] **Step 7: Add tooltip check and event dismissal in `dispatch()`**

At the top of `dispatch()`, before overlay handling:

```rust
        if !matches!(ev, Event::MouseMove { .. }) {
            self.tooltip_overlay = None;
            self.tooltip_timer = None;
        }
```

In `dispatch()`, change the final `self.dock.dispatch(ev, ctx)` to:

```rust
        let result = self.dock.dispatch(ev, ctx);
        if let Event::MouseMove { px, py } = ev {
            self.check_tooltips(*px, *py);
        }
        result
```

- [ ] **Step 8: Paint tooltip overlay in `paint_chrome()`**

After the existing overlays loop in `paint_chrome()`:

```rust
        if let Some(ref tooltip) = self.tooltip_overlay {
            let saved = ctx.list.offset;
            ctx.list.offset = (
                saved.0 + tooltip.layout_rect.x,
                saved.1 + tooltip.layout_rect.y,
            );
            tooltip.widget.paint(&mut ctx);
            ctx.list.offset = saved;
        }
```

- [ ] **Step 9: Paint tooltip in `paint()`**

After the overlays loop in `paint()`:

```rust
        if let Some(ref tooltip) = self.tooltip_overlay {
            let saved = ctx.list.offset;
            ctx.list.offset = (saved.0 + tooltip.layout_rect.x, saved.1 + tooltip.layout_rect.y);
            tooltip.widget.paint(ctx);
            ctx.list.offset = saved;
        }
```

- [ ] **Step 10: Clear tooltip when popup overlay is shown**

In `push_overlay()`:

```rust
    pub fn push_overlay(&mut self, widget: Box<dyn Widget>, layout_rect: Rect) {
        self.overlays.clear();
        self.overlays.push(OverlayChild { widget, layout_rect });
        self.tooltip_overlay = None;
        self.tooltip_timer = None;
    }
```

- [ ] **Step 11: Verify compilation**

Run: `cargo check -p app 2>&1`
Expected: no errors

- [ ] **Step 12: Commit**

```bash
git add crates/app/src/ui_shell.rs
git commit -m "feat: integrate tooltip timer, detection, and overlay into UiShell"
```

---

### Task 6: Adopt tooltip_at in SearchBarWidget

**Files:**
- Modify: `crates/ui/src/widgets/search_bar.rs`

- [ ] **Step 1: Add import**

Add at top:

```rust
use crate::widgets::tooltip::TooltipHint;
```

- [ ] **Step 2: Add `tooltip_at()` override**

In `impl Widget for SearchBarWidget`, after `as_any_mut()`:

```rust
    fn tooltip_at(&self, _px: f32, _py: f32) -> Option<TooltipHint> {
        let label = match self.hovered_btn {
            HoveredButton::None => return None,
            HoveredButton::CloseBar => "Close",
            HoveredButton::ToggleReplace => {
                if self.snap.replace_mode { "Hide Replace" } else { "Show Replace" }
            }
            HoveredButton::Regex => "Regex",
            HoveredButton::Next => "Next Match",
            HoveredButton::Prev => "Previous Match",
            HoveredButton::Replace => "Replace",
            HoveredButton::ReplaceAll => "Replace All",
        };

        let btn_rect = match self.hovered_btn {
            HoveredButton::CloseBar => self.close_btn_rect.get(),
            HoveredButton::ToggleReplace => self.toggle_replace_btn_rect.get(),
            HoveredButton::Regex => self.regex_btn_rect.get(),
            HoveredButton::Next => self.next_btn_rect.get(),
            HoveredButton::Prev => self.prev_btn_rect.get(),
            HoveredButton::Replace => self.replace_btn_rect.get(),
            HoveredButton::ReplaceAll => self.replace_all_btn_rect.get(),
            HoveredButton::None => return None,
        };

        if btn_rect.w <= 0.0 || btn_rect.h <= 0.0 {
            return None;
        }

        Some(TooltipHint {
            label: label.to_string(),
            target_rect: btn_rect,
        })
    }
```

- [ ] **Step 3: Delete inline tooltip rendering**

Remove three things:
1. `paint_tooltip()` method (entire method)
2. `hovered_tooltip()` method (entire method)
3. In `paint_find_only()`: remove the `if let Some((label, btn_rect)) = self.hovered_tooltip() { self.paint_tooltip(...) }` block
4. In `paint_find_replace()`: remove the same block

- [ ] **Step 4: Verify compilation and tests**

Run: `cargo test -p ui -- search_bar 2>&1`
Expected: existing tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/search_bar.rs
git commit -m "refactor: replace inline tooltip with Widget::tooltip_at in SearchBarWidget"
```

---

### Task 7: Adopt tooltip_at in TabBarWidget

**Files:**
- Modify: `crates/ui/src/widgets/tab_bar/widget.rs`

- [ ] **Step 1: Add imports**

Add at top:

```rust
use crate::widgets::tooltip::TooltipHint;
use crate::core::text_util::estimate_text_width_px;
use super::hit::TabHit;
```

- [ ] **Step 2: Add `tooltip_at()` override**

In `impl Widget for TabBarWidget`, after `as_any_mut()`:

```rust
    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        let layout = self.state.current_layout()?;
        let dpi = crate::settings::Settings::with(|s| s.dpi_scale);
        let font_size = crate::constants::TITLE_FONT_SIZE * dpi;

        if let Some(TabHit::Tab(idx)) = self.state.hit_test_px(px, py) {
            let tab = layout.tabs.iter().find(|t| t.index == idx)?;
            let padding = 16.0 * dpi;
            let avail_w = tab.rect_px.w - padding;
            if avail_w > 0.0 && estimate_text_width_px(&tab.title, font_size) > avail_w {
                return Some(TooltipHint {
                    label: tab.title.clone(),
                    target_rect: tab.rect_px,
                });
            }
        }
        None
    }
```

- [ ] **Step 3: Verify compilation and tests**

Run: `cargo test -p ui -- tab_bar 2>&1`
Expected: existing tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/tab_bar/widget.rs
git commit -m "feat: add tooltip_at to TabBarWidget for truncated tab titles"
```

---

### Task 8: Adopt tooltip_at in StatusBarWidget

**Files:**
- Modify: `crates/ui/src/widgets/status_bar.rs`

- [ ] **Step 1: Add import**

Add at top:

```rust
use crate::widgets::tooltip::TooltipHint;
```

- [ ] **Step 2: Add `tooltip_at()` override**

In `impl Widget for StatusBarWidget`, after `as_any_mut()`:

```rust
    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        if self.toggle_rect.w > 0.0 && self.toggle_rect.contains(px, py) {
            let label = if let Some(ref input) = self.input {
                if input.is_preview { "Switch to Edit Mode" } else { "Switch to Preview Mode" }
            } else {
                "Toggle Markdown Preview"
            };
            return Some(TooltipHint {
                label: label.to_string(),
                target_rect: self.toggle_rect,
            });
        }
        None
    }
```

- [ ] **Step 3: Verify compilation and tests**

Run: `cargo test -p ui -- status_bar 2>&1`
Expected: existing tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/status_bar.rs
git commit -m "feat: add tooltip_at to StatusBarWidget for toggle button"
```

---

### Task 9: Add test helper and smoke test

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 1: Add `#[cfg(test)]` helper method**

```rust
    #[cfg(test)]
    pub(crate) fn has_tooltip_timer(&self) -> bool {
        self.tooltip_timer.is_some()
    }
```

- [ ] **Step 2: Add smoke test**

In the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn tooltip_timer_absent_when_not_over_button() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        shell.set_search_input(ui::widgets::search_bar::SearchBarSnapshot {
            query: "test".into(),
            preedit_text: String::new(),
            match_count: 2,
            current_match: 0,
            visible: true,
            cursor_x: 0.0,
            blink_on: false,
            replace_query: String::new(),
            replace_mode: false,
            focus_replace: false,
            options_use_regex: false,
        });
        let inputs = ShellInputs {
            tabs_visible: false, tabs_thickness: 0.0,
            search_visible: true, search_thickness: 28.0,
            status_thickness: 0.0,
            sidebar_visible: false, sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            dpi: 1.0,
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);
        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let _ = shell.dispatch(&Event::MouseMove { px: 50.0, py: 4.0 }, &mut ectx);
        assert!(!shell.has_tooltip_timer(), "No tooltip when not over a button");
    }
```

- [ ] **Step 3: Run test**

Run: `cargo test -p app -- tooltip_timer 2>&1`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/ui_shell.rs
git commit -m "test: add tooltip detection smoke test"
```

---

### Task 10: Full build and manual verification

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1`
Expected: no errors

- [ ] **Step 2: Run all tests**

Run: `cargo test 2>&1`
Expected: all tests pass

- [ ] **Step 3: Manual smoke test**

Run the application. Verify:
1. Hover over search bar buttons (close, prev/next, regex, etc.) — tooltip appears after ~400ms
2. Hover over a truncated tab title — full filename appears
3. Hover over status bar toggle button (open a .md file) — tooltip appears
4. Move mouse away from target — tooltip disappears immediately
5. Click anywhere — tooltip disappears
