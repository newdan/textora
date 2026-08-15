//! Tooltip overlay widget — renders a single-line label pill with
//! auto-positioning relative to a target rect.

use crate::core::geom::Rect;
use crate::core::text_layout::wrap_text_to_lines;
use crate::core::text_util::estimate_text_width_px;
use crate::core::widget::{Event, EventCtx, LayoutCtx, PaintCtx, Widget, WidgetAction};
use crate::core::{AccessibilityContext, AccessibilityId, AccessibilityNode, AccessibilityRole};
use std::any::Any;

const TOOLTIP_ACCESSIBILITY_ID: AccessibilityId = AccessibilityId(0x746f_6f6c_7469_7001);

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
const LINE_HEIGHT_RATIO: f32 = 1.25;
const MAX_WIDTH_LOGICAL: f32 = 320.0;
const SCREEN_MARGIN_LOGICAL: f32 = 8.0;

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
    display_lines: Vec<String>,
    /// Local-space rect for the tooltip pill (origin at 0,0).
    rect: Rect,
    line_height: f32,
}

struct TooltipLayoutMetrics {
    font_size: f32,
    pad_x: f32,
    pad_y: f32,
    gap: f32,
    line_height: f32,
    screen_w: f32,
    screen_h: f32,
    margin_x: f32,
    margin_y: f32,
    max_width: f32,
    available_height: f32,
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
        let Some(metrics) = TooltipLayoutMetrics::new(dpi, screen_w, screen_h) else {
            return Self::empty(hint, dpi);
        };
        let (display_lines, tooltip_w, tooltip_h) = Self::wrap_content(hint, &metrics);
        if display_lines.is_empty() {
            return Self::empty(hint, dpi);
        }
        let layout_rect = Self::position(hint.target_rect, tooltip_w, tooltip_h, &metrics);

        let widget = Self {
            label: hint.label.clone(),
            display_lines,
            rect: Rect::new(0.0, 0.0, tooltip_w, tooltip_h),
            line_height: metrics.line_height,
        };
        (widget, layout_rect)
    }

    fn empty(hint: &TooltipHint, dpi: f32) -> (Self, Rect) {
        let dpi = normalized_dpi(dpi);
        (
            Self {
                label: hint.label.clone(),
                display_lines: Vec::new(),
                rect: Rect::ZERO,
                line_height: FONT_SIZE * dpi * LINE_HEIGHT_RATIO,
            },
            Rect::ZERO,
        )
    }

    fn wrap_content(hint: &TooltipHint, metrics: &TooltipLayoutMetrics) -> (Vec<String>, f32, f32) {
        let content_max_width = (metrics.max_width - metrics.pad_x * 2.0).max(0.0);
        let max_lines = (((metrics.available_height - metrics.pad_y * 2.0).max(0.0)
            / metrics.line_height)
            .floor() as usize)
            .max(1);
        let lines = wrap_text_to_lines(&hint.label, content_max_width, max_lines, |text| {
            estimate_text_width_px(text, metrics.font_size)
        });
        let text_width = lines
            .iter()
            .map(|line| estimate_text_width_px(line, metrics.font_size))
            .fold(0.0, f32::max);
        let width = (text_width + metrics.pad_x * 2.0).min(metrics.max_width);
        let height = (metrics.line_height * lines.len() as f32 + metrics.pad_y * 2.0)
            .min(metrics.available_height);
        (lines, width, height)
    }

    fn position(
        target: Rect,
        tooltip_w: f32,
        tooltip_h: f32,
        metrics: &TooltipLayoutMetrics,
    ) -> Rect {
        let target = normalized_rect(target);
        let below_y = target.bottom() + metrics.gap;
        let above_y = target.y - tooltip_h - metrics.gap;
        if below_y + tooltip_h <= metrics.screen_h - metrics.margin_y {
            let x = Self::clamp_x(&target, tooltip_w, metrics.screen_w, metrics.margin_x);
            return Rect::new(x, below_y, tooltip_w, tooltip_h);
        }
        if above_y >= metrics.margin_y {
            let x = Self::clamp_x(&target, tooltip_w, metrics.screen_w, metrics.margin_x);
            return Rect::new(x, above_y, tooltip_w, tooltip_h);
        }

        let left_space = target.x - metrics.margin_x;
        let right_space = metrics.screen_w - target.right();
        let x = if right_space >= left_space {
            (target.right() + metrics.gap).min(metrics.screen_w - metrics.margin_x - tooltip_w)
        } else {
            (target.x - tooltip_w - metrics.gap).max(metrics.margin_x)
        };
        let y = Self::clamp_y(&target, tooltip_h, metrics.screen_h, metrics.margin_y);
        Rect::new(x, y, tooltip_w, tooltip_h)
    }

    /// Clamp x so tooltip is horizontally centered on target but within screen.
    /// Falls back to right-aligning with target when near screen edge.
    fn clamp_x(target: &Rect, tooltip_w: f32, screen_w: f32, margin: f32) -> f32 {
        // Try centering on target
        let raw = target.x + (target.w - tooltip_w) / 2.0;
        let clamped = raw.max(margin).min(screen_w - margin - tooltip_w);
        // If centering pushes right edge past target right + margin, pin to target right
        let right_limit = (target.x + target.w - tooltip_w).max(margin);
        if clamped + tooltip_w > target.x + target.w + GAP {
            right_limit.min(clamped)
        } else {
            clamped
        }
    }

    /// Clamp y so tooltip is vertically centered on target but within screen.
    fn clamp_y(target: &Rect, tooltip_h: f32, screen_h: f32, margin: f32) -> f32 {
        let raw = target.y + (target.h - tooltip_h) / 2.0;
        raw.max(margin).min(screen_h - margin - tooltip_h)
    }
}

impl TooltipLayoutMetrics {
    fn new(dpi: f32, screen_w: f32, screen_h: f32) -> Option<Self> {
        let dpi = normalized_dpi(dpi);
        let screen_w = finite_non_negative(screen_w);
        let screen_h = finite_non_negative(screen_h);
        if screen_w == 0.0 || screen_h == 0.0 {
            return None;
        }
        let margin_x = (SCREEN_MARGIN_LOGICAL * dpi).min(screen_w * 0.5);
        let margin_y = (SCREEN_MARGIN_LOGICAL * dpi).min(screen_h * 0.5);
        let available_width = (screen_w - margin_x * 2.0).max(0.0);
        let available_height = (screen_h - margin_y * 2.0).max(0.0);
        let max_width = (MAX_WIDTH_LOGICAL * dpi).min(available_width);
        (max_width > 0.0 && available_height > 0.0).then_some(Self {
            font_size: FONT_SIZE * dpi,
            pad_x: PAD_X * dpi,
            pad_y: PAD_Y * dpi,
            gap: GAP * dpi,
            line_height: FONT_SIZE * dpi * LINE_HEIGHT_RATIO,
            screen_w,
            screen_h,
            margin_x,
            margin_y,
            max_width,
            available_height,
        })
    }
}

fn normalized_dpi(dpi: f32) -> f32 {
    if dpi.is_finite() && dpi > 0.0 { dpi } else { 1.0 }
}

fn normalized_rect(rect: Rect) -> Rect {
    Rect::new(
        finite_non_negative(rect.x),
        finite_non_negative(rect.y),
        finite_non_negative(rect.w),
        finite_non_negative(rect.h),
    )
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() { value.max(0.0) } else { 0.0 }
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
        if self.label.is_empty() || self.display_lines.is_empty() {
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
        let first_baseline = r.y + pad_y + font_size * BASELINE_SHIFT;
        if let Some(ref mut shaper) = ctx.shaper {
            for (line_index, line) in self.display_lines.iter().enumerate() {
                let text_y = first_baseline + line_index as f32 * self.line_height;
                ctx.list.text_shaped(
                    text_x,
                    text_y,
                    font_size,
                    theme.application_theme().text_primary,
                    line,
                    shaper,
                );
            }
        }
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        if self.label.is_empty() || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        Some(
            AccessibilityNode::new(
                TOOLTIP_ACCESSIBILITY_ID,
                AccessibilityRole::Tooltip,
                ctx.screen_bounds(self.rect),
            )
            .with_name(self.label.clone()),
        )
    }

    fn on_event(&mut self, _event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        None // Tooltip doesn't handle events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::paint::{DrawCmd, DrawList};
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
        let widget = TooltipWidget {
            label: String::new(),
            display_lines: Vec::new(),
            rect: Rect::new(0.0, 0.0, 100.0, 30.0),
            line_height: FONT_SIZE * LINE_HEIGHT_RATIO,
        };
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
    fn paint_uses_the_elevated_surface_foreground_for_readable_text() {
        let theme = test_theme();
        let hint = make_hint("新建目录", Rect::new(20.0, 20.0, 24.0, 24.0));
        let (widget, _) = TooltipWidget::new(&hint, 1.0, 800.0, 600.0);
        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("tooltip paint test shaper should exist");
        let mut context = PaintCtx {
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };

        widget.paint(&mut context);

        let text_color = list.cmds.iter().find_map(|command| match command {
            DrawCmd::TextLayout { color, .. } => Some(*color),
            _ => None,
        });
        assert_eq!(text_color, Some(theme.application_theme().text_primary));
    }

    #[test]
    fn hit_always_false() {
        let hint = make_hint("Test", Rect::new(0.0, 0.0, 50.0, 20.0));
        let (widget, _) = TooltipWidget::new(&hint, 1.0, 800.0, 600.0);
        assert!(!widget.hit(0.0, 0.0));
        assert!(!widget.hit(25.0, 10.0));
    }

    #[test]
    fn accessibility_exposes_tooltip_text_without_focus_or_actions() {
        let hint = make_hint("保存文档", Rect::new(10.0, 20.0, 30.0, 20.0));
        let (widget, _) = TooltipWidget::new(&hint, 1.0, 800.0, 600.0);

        let node = widget
            .accessibility_node(&crate::core::AccessibilityContext::new(100.0, 200.0))
            .expect("visible tooltip should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::Tooltip);
        assert_eq!(node.name.as_deref(), Some("保存文档"));
        assert_eq!(node.bounds.x, 100.0);
        assert_eq!(node.bounds.y, 200.0);
        assert!(node.actions.is_empty());
        assert!(!widget.is_focusable());
    }

    #[test]
    fn long_multilingual_tooltip_is_wrapped_and_bounded_at_standard_and_high_dpi() {
        for dpi in [1.0, 2.0] {
            let screen_width = 300.0 * dpi;
            let screen_height = 180.0 * dpi;
            let hint = make_hint(
                "A very long tooltip with English words、中文说明和 emoji 👨‍👩‍👧‍👦 that must stay visible",
                Rect::new(290.0 * dpi, 160.0 * dpi, 10.0 * dpi, 10.0 * dpi),
            );
            let (widget, layout) = TooltipWidget::new(&hint, dpi, screen_width, screen_height);

            assert!(widget.display_lines.len() > 1, "dpi={dpi}");
            assert!(layout.x.is_finite() && layout.y.is_finite(), "dpi={dpi}");
            assert!(layout.x >= 0.0 && layout.y >= 0.0, "dpi={dpi}");
            assert!(layout.right() <= screen_width, "dpi={dpi}");
            assert!(layout.bottom() <= screen_height, "dpi={dpi}");
        }
    }

    #[test]
    fn tooltip_is_safe_when_screen_is_smaller_than_one_line_or_zero_sized() {
        let hint = make_hint("无法完整显示的提示", Rect::new(f32::NAN, f32::INFINITY, 4.0, 4.0));
        let (_, tiny_layout) = TooltipWidget::new(&hint, 2.0, 30.0, 10.0);
        let (_, zero_layout) = TooltipWidget::new(&hint, 1.0, 0.0, 0.0);

        assert!(tiny_layout.x.is_finite() && tiny_layout.y.is_finite());
        assert!(tiny_layout.w <= 30.0 && tiny_layout.h <= 10.0);
        assert_eq!(zero_layout, Rect::ZERO);
    }
}
