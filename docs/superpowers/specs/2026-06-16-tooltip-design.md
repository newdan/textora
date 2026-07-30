# Tooltip Component Design

## Summary

Extract the inline tooltip rendering from `SearchBarWidget::paint_tooltip` into a
general-purpose tooltip system. Any widget can expose tooltip hints for its
sub-regions by overriding one trait method; the shell handles delay, positioning,
and rendering.

## Architecture

```
Widget trait
  └─ fn tooltip_at(&self, px, py) -> Option<TooltipHint>   // default None

UiShell
  ├─ MouseMove dispatch:
  │   1. Walk dock children, hit-test, call tooltip_at()
  │   2. Compare hint to previous; reset/start delay timer (400ms)
  │   3. On timer fire → push TooltipWidget into overlays
  │   4. On hint change/None → pop TooltipWidget
  └─ TooltipWidget in overlays: paints, hit() always false
```

### TooltipHint

```rust
pub struct TooltipHint {
    pub label: String,
    /// Target rect in widget-local coordinates.
    pub target_rect: Rect,
}
```

### TooltipWidget

Independent overlay widget:
- **Layout**: receives target rect in screen space (shell converts from widget-local)
- **Rendering**: dark rounded pill, border, single-line text
- **Positioning**: 4-direction auto-flip (see below)
- **Events**: `hit()` returns false, `on_event()` returns None — fully transparent

## Positioning (4-direction auto-flip)

1. **Default**: below target, center-aligned horizontally, 4dp DPI gap
2. Below overflows → **flip above**
3. Both overflow → pick **side with more space**
4. Horizontal: center-aligned preferred, clamp within screen insets (8dp DPI)

All positioning uses `estimate_text_width_px` for size and screen rect from shell.

## Rendering

Same visual style as the current search bar tooltip:

| Property | Value |
|----------|-------|
| Background | `theme.tooltip_bg` (dark: `[0.15,0.15,0.15,0.92]`, light: `[0.92,0.92,0.92,0.92]`) |
| Border | `theme.tooltip_border` (dark: `[0.35,0.35,0.35,0.6]`, light: `[0.55,0.55,0.55,0.4]`) |
| Text | `theme.tooltip_fg` (dark: `[0.9,0.9,0.9,1.0]`, light: `[0.15,0.15,0.15,1.0]`) |
| Font size | 11dp DPI |
| Padding | 6dp DPI horizontal, 3dp DPI vertical |
| Corner radius | 4dp DPI |
| Gap from target | 4dp DPI |

## Delay Timer

- Hover same hint for 400ms → show
- Hint changes or becomes None → hide immediately
- Timer resets on every hint change (including None)
- Timer managed by UiShell; `TooltipWidget` is stateless regarding timing

## Files Changed

### New
- `crates/ui/src/widgets/tooltip.rs` — `TooltipHint`, `TooltipWidget`

### Modified
- `crates/ui/src/core/widget.rs` — add `tooltip_at` default method to `Widget` trait
- `crates/ui/src/widgets/mod.rs` — register `pub mod tooltip;`
- `crates/ui/src/theme.rs` — add `tooltip_bg`, `tooltip_fg`, `tooltip_border` to Theme
- `crates/app/src/ui_shell.rs` — timer state, dispatch logic, overlay management

### Adopter Changes
- `crates/ui/src/widgets/search_bar.rs`:
  - Remove `paint_tooltip()`, `hovered_tooltip()`, and their call sites
  - Override `tooltip_at()` using existing `HoveredButton` + button rect logic
  - Keep `HoveredButton` enum, `update_hover()`, `hovered_btn` for hover highlight and click handling
- `crates/ui/src/widgets/tab_bar/widget.rs`:
  - Override `tooltip_at()` — hit-test tab, check truncation, return full title
- `crates/ui/src/widgets/status_bar.rs`:
  - Override `tooltip_at()` — check toggle button hover region

## Widget Trait Change

```rust
fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
    None  // default: no tooltip
}
```

Only widgets that need tooltips override it. Initial adopters:
1. `SearchBarWidget` — buttons (close, prev/next, toggle, regex, replace, replace all)
2. `TabBarWidget` — full filename when tab title is truncated with ellipsis
3. `StatusBarWidget` — markdown preview toggle button ("Toggle Markdown Preview")

## UiShell Changes (key additions)

```rust
// New fields
tooltip_timer: Option<TooltipTimer>,

struct TooltipTimer {
    hint: TooltipHint,
    target_screen_rect: Rect,    // target in screen coords (converted at dispatch time)
    start: Instant,
}
```

In `dispatch()`:
- After dock dispatch, if no overlay consumed the event and it's a MouseMove:
  - Walk dock children, hit-test, call `tooltip_at()`
  - Compare hint to current timer: same → no-op, different → reset/clear timer
- In `update_frame()` (runs every frame):
  - Check timer → if elapsed >= 400ms, push `TooltipWidget` overlay

## Adopter Details

### SearchBarWidget

Replace inline `paint_tooltip()` + `hovered_tooltip()` with `tooltip_at()` override:

```rust
fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
    let label = match self.hovered_btn {
        HoveredButton::None => return None,
        HoveredButton::CloseBar => "Close",
        HoveredButton::ToggleReplace =>
            if self.snap.replace_mode { "Hide Replace" } else { "Show Replace" },
        HoveredButton::Regex => "Regex",
        HoveredButton::Next => "Next Match",
        HoveredButton::Prev => "Previous Match",
        HoveredButton::Replace => "Replace",
        HoveredButton::ReplaceAll => "Replace All",
    };
    let btn_rect = match self.hovered_btn { ... }; // map to stored rect
    Some(TooltipHint { label: label.into(), target_rect: btn_rect })
}
```

Delete: `paint_tooltip()`, `hovered_tooltip()`, and the two `hovered_tooltip()` call sites in paint methods.

### TabBarWidget

Override `tooltip_at()`:
- Call `self.state.hit_test_px(px, py)` — if `Some(TabHit::Tab(idx))`, check if that tab's title was truncated
- Truncation check: `estimate_text_width_px(tab.title, font_size)` exceeds `tab.rect_px.w - padding`
- If truncated, return `TooltipHint { label: tab.title, target_rect: tab.rect_px }`
- No extra state needed — `hit_test_px` and layout already cached

### StatusBarWidget

Override `tooltip_at()`:
- If `self.toggle_rect.w > 0` and `self.toggle_rect.contains(px, py)`:
  - Return `TooltipHint { label: "Toggle Markdown Preview", target_rect: self.toggle_rect }`
- Otherwise `None`

## Non-Goals

- Multi-line tooltips
- Rich text / markup in tooltips
- Tooltip arrow pointer
- Configurable delay (hardcoded 400ms for now)
