use std::any::Any;

use crate::core::text_util::estimate_text_width_px;
use crate::core::{
    AccessibilityContext, AccessibilityId, AccessibilityNode, AccessibilityRole, Event, EventCtx,
    LayoutCtx, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::widgets::icon::draw_icon;

const DEFAULT_LABEL_FONT_SIZE_LOGICAL: f32 = 13.0;
const DEFAULT_LABEL_ICON_GAP_LOGICAL: f32 = 6.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LabelForeground {
    #[default]
    ThemeMain,
    ThemeMuted,
    Explicit([f32; 4]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelStyle {
    pub font_size_logical: f32,
    pub font_weight: shaping::Weight,
    pub foreground: LabelForeground,
    pub gap_logical: f32,
}

impl Default for LabelStyle {
    fn default() -> Self {
        Self {
            font_size_logical: DEFAULT_LABEL_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::NORMAL,
            foreground: LabelForeground::ThemeMain,
            gap_logical: DEFAULT_LABEL_ICON_GAP_LOGICAL,
        }
    }
}

pub struct Label {
    rect: Rect,
    text: String,
    leading_icon: Option<String>,
    trailing_icon: Option<String>,
    style: LabelStyle,
    accessibility_id: Option<AccessibilityId>,
}

impl Label {
    pub fn new(text: impl Into<String>, style: LabelStyle) -> Self {
        Self {
            rect: Rect::ZERO,
            text: text.into(),
            leading_icon: None,
            trailing_icon: None,
            style,
            accessibility_id: None,
        }
    }

    pub fn set_leading_icon(&mut self, icon: Option<String>) {
        self.leading_icon = icon;
    }

    pub fn set_trailing_icon(&mut self, icon: Option<String>) {
        self.trailing_icon = icon;
    }

    pub fn set_accessibility_id(&mut self, id: Option<AccessibilityId>) {
        self.accessibility_id = id;
    }

    fn resolved_foreground(&self, ctx: &PaintCtx) -> [f32; 4] {
        let mut color = match self.style.foreground {
            LabelForeground::ThemeMain => ctx.theme.palette.text_main,
            LabelForeground::ThemeMuted => ctx.theme.palette.text_muted,
            LabelForeground::Explicit(color) => color,
        };
        color[3] *= ctx.global_alpha;
        color
    }
}

fn draw_check_icon(ctx: &mut PaintCtx, x: f32, y: f32, size: f32, color: [f32; 4]) {
    let left = x + size * 0.14;
    let top = y + size * 0.52;
    let mid_x = x + size * 0.38;
    let mid_y = y + size * 0.76;
    let right = x + size * 0.84;
    let bottom = y + size * 0.22;
    let thickness = size * 0.14;

    ctx.list.fill_triangle([left, top], [left + thickness, top - thickness], [mid_x, mid_y], color);
    ctx.list.fill_triangle(
        [left + thickness, top - thickness],
        [mid_x + thickness, mid_y - thickness],
        [mid_x, mid_y],
        color,
    );
    ctx.list.fill_triangle(
        [mid_x, mid_y],
        [mid_x + thickness, mid_y - thickness],
        [right, bottom],
        color,
    );
    ctx.list.fill_triangle(
        [mid_x + thickness, mid_y - thickness],
        [right + thickness, bottom - thickness],
        [right, bottom],
        color,
    );
}

fn draw_label_icon(ctx: &mut PaintCtx, name: &str, x: f32, y: f32, size: f32, color: [f32; 4]) {
    match name {
        "check" => draw_check_icon(ctx, x, y, size, color),
        _ => draw_icon(ctx.list, name, x, y, size, color),
    }
}

impl Widget for Label {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let dpi = ctx.dpi;
        let font_size = self.style.font_size_logical * dpi;
        let icon_gap = self.style.gap_logical * dpi;
        let icon_size = font_size;
        let color = self.resolved_foreground(ctx);
        let baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35;
        let icon_y = self.rect.y + (self.rect.h - icon_size) * 0.5;
        let mut cursor_x = self.rect.x;

        if let Some(icon_name) = &self.leading_icon {
            draw_label_icon(ctx, icon_name, cursor_x, icon_y, icon_size, color);
            cursor_x += icon_size + icon_gap;
        }

        let text_width = if let Some(ref mut shaper) = ctx.shaper {
            ctx.list.text_shaped_with_font(
                cursor_x,
                baseline,
                font_size,
                color,
                &self.text,
                None,
                self.style.font_weight,
                shaping::Style::Normal,
                false,
                shaper,
            )
        } else {
            estimate_text_width_px(&self.text, font_size)
        };
        cursor_x += text_width;

        if let Some(icon_name) = &self.trailing_icon {
            cursor_x += icon_gap;
            draw_label_icon(ctx, icon_name, cursor_x, icon_y, icon_size, color);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let id = self.accessibility_id?;
        Some(
            AccessibilityNode::new(id, AccessibilityRole::StaticText, ctx.screen_bounds(self.rect))
                .with_name(self.text.clone()),
        )
    }

    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::measure::NoopMeasure;
    use crate::core::{
        DrawCmd, DrawList, Event, EventCtx, KeyCode, LayoutCtx, Modifiers, PaintCtx, Rect, Widget,
    };

    fn set_test_rect(label: &mut Label, rect: Rect) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        label.set_rect(rect, &mut layout_ctx);
    }

    fn paint_for_test(label: &Label) -> DrawList {
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        label.paint(&mut paint_ctx);
        draw_list
    }

    fn event_ctx() -> EventCtx<'static> {
        let theme = Box::leak(Box::new(crate::theme::test_theme()));
        EventCtx { cursor_hint: None, theme, dpi: 1.0 }
    }

    #[test]
    fn label_paints_icon_before_text_and_never_emits_actions() {
        let mut label = Label::new("Connected", LabelStyle::default());
        label.set_leading_icon(Some("check".into()));
        set_test_rect(&mut label, Rect::new(0.0, 0.0, 180.0, 28.0));

        let draw_list = paint_for_test(&label);

        assert!(draw_list.cmds.len() >= 2);
        assert!(matches!(draw_list.cmds[0], DrawCmd::FillTriangle { .. }));
        assert!(draw_list.cmds.iter().any(|cmd| matches!(cmd, DrawCmd::TextLayout { .. })));
        assert_eq!(
            label.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx()),
            None
        );
    }

    #[test]
    fn accessibility_exposes_identified_label_as_static_text() {
        let mut label = Label::new("连接状态", LabelStyle::default());
        label.set_accessibility_id(Some(crate::core::AccessibilityId(71)));
        set_test_rect(&mut label, Rect::new(3.0, 4.0, 120.0, 28.0));

        let node = label
            .accessibility_node(&crate::core::AccessibilityContext::new(10.0, 20.0))
            .expect("identified label should expose semantics");

        assert_eq!(node.id, crate::core::AccessibilityId(71));
        assert_eq!(node.role, crate::core::AccessibilityRole::StaticText);
        assert_eq!(node.name.as_deref(), Some("连接状态"));
        assert_eq!(node.bounds, Rect::new(13.0, 24.0, 120.0, 28.0));
        assert!(node.actions.is_empty());
        assert!(!label.is_focusable());
    }
}
