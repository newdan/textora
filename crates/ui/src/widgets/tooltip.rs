//! Tooltip overlay widget — renders a single-line label pill with
//! auto-positioning relative to a target rect.

use crate::core::geom::Rect;
use crate::core::text_util::estimate_text_width_px;
use crate::core::widget::{Event, EventCtx, LayoutCtx, PaintCtx, Widget, WidgetAction};
use std::any::Any;

/// Font size in logical pixels (before DPI scaling).
const FONT_SIZE: f32 = 11.0;
/// Horizontal padding inside the tooltip pill.
const PAD_X: f32 = 6.0;
/// Vertical padding inside the tooltip pill.
const PAD_Y: f32 = 3.0;
/// Gap between target rect and tooltip.
const GAP: f32 = 4.0;
/// Corner radius of the tooltip pill.
const CORNER_RADIUS: f32 = 4.0;
/// Baseline offset multiplier for text vertical centering.
const BASELINE_SHIFT: f32 = 0.8;

/// A tooltip hint from a widget: label text + target rectangle
/// in widget-local coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipHint {
    pub label: String,
    pub target_rect: Rect,
}

/// Tooltip overlay widget. Computes screen-space rect during construction;
/// paint draws a dark rounded pill with white text.
pub struct TooltipWidget {
    label: String,
    /// Local-space rect for the tooltip pill (origin at 0,0).
    rect: Rect,
}

impl TooltipWidget {
    /// Create a new tooltip widget from a hint.
    /// Returns (widget, layout_rect) where layout_rect is the screen-space position.
    ///
    /// Positioning priority:
    /// 1. Below target (preferred)
    /// 2. Above target (if below overflows screen bottom)
    /// 3. Side with more horizontal space (if both vertical positions overflow)
    /// Horizontal clamping ensures tooltip stays within `[0, screen_w]`.
    pub fn new(hint: &TooltipHint, dpi: f32, screen_w: f32, screen_h: f32) -> (Self, Rect) {
        let font_size = FONT_SIZE * dpi;
        let pad_x = PAD_X * dpi;
        let pad_y = PAD_Y * dpi;
        let gap = GAP * dpi;

        let text_width = estimate_text_width_px(&hint.label, font_size);
        let tooltip_w = text_width + pad_x * 2.0;
        let tooltip_h = font_size + pad_y * 2.0;

        let target = &hint.target_rect;

        // Vertical positioning: prefer below, flip above, then side fallback
        let below_y = target.y + target.h + gap;
        let above_y = target.y - tooltip_h - gap;
        let below_fits = below_y + tooltip_h <= screen_h;
        let above_fits = above_y >= 0.0;

        let (x, y) = if below_fits {
            // Below target (preferred)
            let x = Self::clamp_x(target, tooltip_w, screen_w);
            (x, below_y)
        } else if above_fits {
            // Above target
            let x = Self::clamp_x(target, tooltip_w, screen_w);
            (x, above_y)
        } else {
            // Side fallback: pick side with more space
            let space_left = target.x;
            let space_right = screen_w - (target.x + target.w);
            if space_right >= space_left {
                // Right side
                let x = (target.x + target.w + gap).min(screen_w - tooltip_w);
                let y = Self::clamp_y(target, tooltip_h, screen_h);
                (x, y)
            } else {
                // Left side
                let x = (target.x - tooltip_w - gap).max(0.0);
                let y = Self::clamp_y(target, tooltip_h, screen_h);
                (x, y)
            }
        };

        let widget =
            Self { label: hint.label.clone(), rect: Rect::new(0.0, 0.0, tooltip_w, tooltip_h) };
        let layout_rect = Rect::new(x, y, tooltip_w, tooltip_h);
        (widget, layout_rect)
    }

    /// Clamp x so tooltip is horizontally centered on target but within screen.
    /// Falls back to right-aligning with target when near screen edge.
    fn clamp_x(target: &Rect, tooltip_w: f32, screen_w: f32) -> f32 {
        // Try centering on target
        let raw = target.x + (target.w - tooltip_w) / 2.0;
        let clamped = raw.max(0.0).min(screen_w - tooltip_w);
        // If centering pushes right edge past target right + margin, pin to target right
        let right_limit = (target.x + target.w - tooltip_w).max(0.0);
        if clamped + tooltip_w > target.x + target.w + GAP {
            right_limit.min(clamped)
        } else {
            clamped
        }
    }

    /// Clamp y so tooltip is vertically centered on target but within screen.
    fn clamp_y(target: &Rect, tooltip_h: f32, screen_h: f32) -> f32 {
        let raw = target.y + (target.h - tooltip_h) / 2.0;
        raw.max(0.0).min(screen_h - tooltip_h)
    }
}

impl Widget for TooltipWidget {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn hit(&self, _px: f32, _py: f32) -> bool {
        false // Tooltip doesn't intercept mouse events
    }

    fn set_rect(&mut self, _rect: Rect, _ctx: &mut LayoutCtx) {
        // Tooltip uses pre-computed rect
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.label.is_empty() {
            return;
        }

        let theme = ctx.theme;
        let r = self.rect;
        let radius = CORNER_RADIUS * ctx.dpi;

        // Draw background pill
        ctx.list.fill_rounded(r, theme.palette.bg_elevated, radius);

        // Draw border
        ctx.list.stroke_rounded(r, theme.palette.border_strong, radius, 1.0);

        // Draw text with baseline shift for vertical centering
        let font_size = FONT_SIZE * ctx.dpi;
        let pad_y = PAD_Y * ctx.dpi;
        let text_x = r.x + PAD_X * ctx.dpi;
        let text_y = r.y + pad_y + font_size * BASELINE_SHIFT;
        if let Some(ref mut shaper) = ctx.shaper {
            ctx.list.text_shaped(
                text_x,
                text_y,
                font_size,
                theme.palette.text_inverse,
                &self.label,
                shaper,
            );
        }
    }

    fn on_event(&mut self, _event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        None // Tooltip doesn't handle events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::paint::DrawList;
    use crate::theme::test_theme;

    fn make_hint(label: &str, target: Rect) -> TooltipHint {
        TooltipHint { label: label.to_string(), target_rect: target }
    }

    #[test]
    fn new_positions_below_by_default() {
        // Target with plenty of space below
        let hint = make_hint("Hello", Rect::new(100.0, 100.0, 50.0, 20.0));
        let (_, layout) = TooltipWidget::new(&hint, 1.0, 800.0, 600.0);
        // Tooltip should be below target
        assert!(
            layout.y >= 100.0 + 20.0,
            "tooltip should be below target by default, got y={} target_bottom={}",
            layout.y,
            120.0
        );
    }

    #[test]
    fn new_flips_above_when_below_overflows() {
        // Target near bottom of screen
        let hint = make_hint("Tip", Rect::new(100.0, 580.0, 50.0, 20.0));
        let (_, layout) = TooltipWidget::new(&hint, 1.0, 800.0, 600.0);
        // Tooltip should be above target
        assert!(
            layout.y + layout.h <= 580.0 + 0.1,
            "tooltip should flip above when below overflows, got y={} h={} target_y={}",
            layout.y,
            layout.h,
            580.0
        );
    }

    #[test]
    fn new_side_fallback_when_both_vertical_overflow() {
        // Very tall screen with target in the middle — both directions overflow
        // Screen height = 30, target at y=10 h=10, tooltip_h ~= 23 -> can't fit above or below
        let hint = make_hint("X", Rect::new(400.0, 10.0, 50.0, 10.0));
        let (_, layout) = TooltipWidget::new(&hint, 1.0, 800.0, 30.0);
        // Should be placed to the side (right, since more space)
        assert!(
            layout.x > 400.0 + 50.0 || layout.x + layout.w < 400.0,
            "tooltip should fall back to side placement, got x={} target_right={}",
            layout.x,
            450.0
        );
    }

    #[test]
    fn new_clamps_to_screen_bounds() {
        // Target near right edge
        let hint = make_hint("RightEdge", Rect::new(790.0, 100.0, 10.0, 20.0));
        let (_, layout) = TooltipWidget::new(&hint, 1.0, 800.0, 600.0);
        assert!(
            layout.x + layout.w <= 800.0 + 0.1,
            "tooltip should not exceed screen width, got x={} w={}",
            layout.x,
            layout.w
        );
        assert!(layout.x >= -0.1, "tooltip should not go negative x, got {}", layout.x);
    }

    #[test]
    fn paint_empty_label_emits_nothing() {
        let theme = test_theme();
        let widget = TooltipWidget { label: String::new(), rect: Rect::new(0.0, 0.0, 100.0, 30.0) };
        let mut list = DrawList::new();
        let mut ctx = PaintCtx {
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: None,
        };
        widget.paint(&mut ctx);
        assert!(list.cmds.is_empty(), "empty label should produce no draw commands");
    }

    #[test]
    fn hit_always_false() {
        let hint = make_hint("Test", Rect::new(0.0, 0.0, 50.0, 20.0));
        let (widget, _) = TooltipWidget::new(&hint, 1.0, 800.0, 600.0);
        assert!(!widget.hit(0.0, 0.0));
        assert!(!widget.hit(25.0, 10.0));
    }
}
