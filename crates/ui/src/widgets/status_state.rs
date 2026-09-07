//! 通用空、加载与可恢复错误状态组件。

use std::any::Any;

use crate::core::text_util::compute_text_width;
use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    DrawCmd, Event, EventCtx, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::widgets::button::{ButtonStyle, ButtonVisualState};
use crate::widgets::icon::draw_icon;

const STATUS_ICON_TITLE_GAP: f32 = 12.0;

/// 状态的视觉种类，不包含产品错误类型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusStateKind {
    #[default]
    Empty,
    Loading,
    RecoverableError,
}

/// 状态组件的纯展示输入。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusStateInput {
    pub kind: StatusStateKind,
    pub title: String,
    pub description: String,
    pub icon: Option<String>,
    pub action_label: Option<String>,
    pub action_id: Option<WidgetId>,
}

/// 用于空、加载和可恢复错误状态的通用组件。
pub struct StatusStateWidget {
    rect: Rect,
    action_rect: Rect,
    input: StatusStateInput,
    hovered_action: bool,
    pressed_action: bool,
}

impl Default for StatusStateWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusStateWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            action_rect: Rect::ZERO,
            input: StatusStateInput::default(),
            hovered_action: false,
            pressed_action: false,
        }
    }

    pub fn set_input(&mut self, input: StatusStateInput) {
        self.input = input;
        if self.input.action_id.is_none() || self.input.action_label.is_none() {
            self.hovered_action = false;
            self.pressed_action = false;
        }
    }

    pub fn action_rect(&self) -> Rect {
        self.action_rect
    }

    fn has_action(&self) -> bool {
        self.input.action_id.is_some() && self.input.action_label.is_some()
    }
}

impl Widget for StatusStateWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let action_width = (120.0 * ctx.dpi).min((rect.w - 24.0 * ctx.dpi).max(0.0));
        let action_height = 32.0 * ctx.dpi;
        self.action_rect = if self.has_action() {
            Rect::new(
                rect.x + (rect.w - action_width) * 0.5,
                rect.bottom() - 24.0 * ctx.dpi - action_height,
                action_width,
                action_height,
            )
        } else {
            Rect::ZERO
        };
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let clip_rect = Rect::new(
            self.rect.x + ctx.list.offset.0,
            self.rect.y + ctx.list.offset.1,
            self.rect.w,
            self.rect.h,
        );
        ctx.list.cmds.push(DrawCmd::PushClip(clip_rect));
        let center_x = self.rect.x + self.rect.w * 0.5;
        let title_font_size = 16.0 * ctx.dpi;
        let description_font_size = 13.0 * ctx.dpi;
        let horizontal_inset = 12.0 * ctx.dpi;
        let icon_size = 28.0 * ctx.dpi;
        let title_y = self.rect.y + self.rect.h * 0.38;
        if let Some(icon) = &self.input.icon {
            draw_icon(
                ctx.list,
                icon,
                center_x - icon_size * 0.5,
                title_y - title_font_size - icon_size - STATUS_ICON_TITLE_GAP * ctx.dpi,
                icon_size,
                ctx.theme.palette.text_muted,
            );
        }
        let title_width =
            compute_text_width(&self.input.title, title_font_size, ctx.shaper.as_deref_mut());
        let title_x = (center_x - title_width * 0.5).max(self.rect.x + horizontal_inset);
        ctx.text(title_x, title_y, title_font_size, ctx.theme.palette.text_main, &self.input.title);
        let description_width = compute_text_width(
            &self.input.description,
            description_font_size,
            ctx.shaper.as_deref_mut(),
        );
        let description_x =
            (center_x - description_width * 0.5).max(self.rect.x + horizontal_inset);
        ctx.text(
            description_x,
            title_y + 28.0 * ctx.dpi,
            description_font_size,
            ctx.theme.palette.text_muted,
            &self.input.description,
        );
        if self.has_action() {
            let state = if self.pressed_action && self.hovered_action {
                ButtonVisualState::Pressed
            } else if self.hovered_action {
                ButtonVisualState::Hovered
            } else {
                ButtonVisualState::Normal
            };
            let foreground = ButtonStyle::from_theme(ctx.theme).paint(ctx, self.action_rect, state);
            let label = self.input.action_label.as_deref().unwrap_or_default();
            let action_font_size = ctx.theme.control_metrics().font_size_logical * ctx.dpi;
            let label_width =
                compute_text_width(label, action_font_size, ctx.shaper.as_deref_mut());
            let label_x = self.action_rect.x + (self.action_rect.w - label_width) * 0.5;
            ctx.text(
                label_x,
                self.action_rect.y + self.action_rect.h * 0.5 + action_font_size * 0.35,
                action_font_size,
                foreground,
                label,
            );
        }
        ctx.list.cmds.push(DrawCmd::PopClip);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.has_action() {
            return None;
        }
        match event {
            Event::MouseMove { px, py } => {
                let hovered = self.action_rect.contains(*px, *py);
                let hover_changed = self.hovered_action != hovered;
                self.hovered_action = hovered;
                if hovered {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                    Some(WidgetAction::Consumed)
                } else {
                    hover_changed.then_some(WidgetAction::Consumed)
                }
            }
            Event::PointerLeave => {
                std::mem::take(&mut self.hovered_action).then_some(WidgetAction::Consumed)
            }
            Event::InteractionCancel => {
                let changed = std::mem::take(&mut self.hovered_action)
                    | std::mem::take(&mut self.pressed_action);
                changed.then_some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.pressed_action = self.action_rect.contains(*px, *py);
                self.hovered_action = self.pressed_action;
                self.pressed_action.then_some(WidgetAction::Consumed)
            }
            Event::MouseUp { px, py, button: MouseButton::Left } if self.pressed_action => {
                self.pressed_action = false;
                self.hovered_action = self.action_rect.contains(*px, *py);
                if self.hovered_action {
                    self.input
                        .action_id
                        .map(|id| WidgetAction::Control(ControlAction::Activated { id }))
                } else {
                    Some(WidgetAction::Consumed)
                }
            }
            _ => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.pressed_action
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{DrawCmd, DrawList, EventCtx, LayoutCtx, NoopMeasure, PaintCtx};

    fn layout(widget: &mut StatusStateWidget, rect: Rect, dpi: f32) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut context = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
        widget.set_rect(rect, &mut context);
    }

    #[test]
    fn status_action_button_font_scales_consistently() {
        let theme = crate::theme::test_theme();
        let mut shaper =
            shaping::Shaper::new().expect("status action typography test requires fonts");
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            action_label: Some("设置根目录".to_owned()),
            action_id: Some(WidgetId(81)),
            ..StatusStateInput::default()
        });
        for dpi in [1.0, 1.5, 2.0] {
            layout(&mut widget, Rect::new(0.0, 0.0, 320.0 * dpi, 240.0 * dpi), dpi);
            let mut draw_list = DrawList::new();
            let mut paint_context = PaintCtx::new(&mut draw_list, &theme, dpi);
            paint_context.shaper = Some(&mut shaper);

            widget.paint(&mut paint_context);

            let (text_layout, text_x) = draw_list
                .cmds
                .iter()
                .find_map(|command| match command {
                    DrawCmd::TextLayout { layout, x, .. } if layout.text == "设置根目录" => {
                        Some((layout, *x))
                    }
                    _ => None,
                })
                .expect("操作按钮应绘制文字");
            assert_eq!(
                text_layout.font_size,
                theme.control_metrics().font_size_logical * dpi,
                "操作按钮应使用标准控件字号，DPI={dpi}"
            );
            let action_rect = widget.action_rect();
            assert_eq!(text_x, action_rect.x + (action_rect.w - text_layout.shaped.width) * 0.5);
            assert!(
                text_x >= action_rect.x && text_x + text_layout.shaped.width <= action_rect.right(),
                "操作文字应居中且不超出按钮边界"
            );
        }
    }

    #[test]
    fn action_cancellation_clears_pointer_feedback_and_capture() {
        let theme = crate::theme::test_theme();
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            action_label: Some("重试".to_owned()),
            action_id: Some(WidgetId(81)),
            ..StatusStateInput::default()
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 320.0, 240.0), 1.0);
        let mut context = EventCtx::new(&theme, 1.0);
        let rect = widget.action_rect();
        let (px, py) = (rect.x + 1.0, rect.y + 1.0);
        widget.on_event(&Event::MouseMove { px, py }, &mut context);
        widget.on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context);
        assert_eq!(
            widget.on_event(&Event::PointerLeave, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert!(!widget.hovered_action);
        assert!(widget.is_capturing());
        assert_eq!(
            widget.on_event(&Event::InteractionCancel, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert!(!widget.is_capturing());
    }

    #[test]
    fn status_action_uses_standard_button_surface() {
        let theme = crate::theme::test_theme();
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            action_label: Some("重试".to_owned()),
            action_id: Some(WidgetId(81)),
            ..StatusStateInput::default()
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 320.0, 240.0), 1.0);
        let mut expected = DrawList::new();
        let mut context = PaintCtx::new(&mut expected, &theme, 1.0);
        context.global_alpha = 0.5;
        crate::button::ButtonStyle::from_theme(&theme).paint(
            &mut context,
            widget.action_rect(),
            crate::button::ButtonVisualState::Normal,
        );
        let mut actual = DrawList::new();
        let mut context = PaintCtx::new(&mut actual, &theme, 1.0);
        context.global_alpha = 0.5;
        widget.paint(&mut context);
        for command in &expected.cmds {
            assert!(
                actual.cmds.contains(command),
                "空状态操作按钮应共享普通按钮绘制规则: {command:?}"
            );
        }
    }

    #[test]
    fn creates_a_status_widget() {
        assert_eq!(StatusStateWidget::new().action_rect(), Rect::ZERO);
    }

    #[test]
    fn narrow_layout_keeps_action_inside_status_rect() {
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            kind: StatusStateKind::RecoverableError,
            title: "Could not open workspace".to_owned(),
            description: "Choose another folder and try again.".to_owned(),
            icon: Some("triangle-alert".to_owned()),
            action_label: Some("Choose folder".to_owned()),
            action_id: Some(WidgetId(81)),
        });
        let status_rect = Rect::new(0.0, 0.0, 80.0, 240.0);
        layout(&mut widget, status_rect, 2.0);

        assert!(widget.action_rect().x >= status_rect.x);
        assert!(widget.action_rect().right() <= status_rect.right());
    }

    #[test]
    fn long_chinese_description_is_clipped_and_centered_inside_status_rect() {
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "暂无笔记".to_owned(),
            description: "新建一篇笔记，或者从左侧选择其他位置。".to_owned(),
            ..StatusStateInput::default()
        });
        let status_rect = Rect::new(220.0, 0.0, 244.0, 400.0);
        layout(&mut widget, status_rect, 1.0);
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };

        widget.paint(&mut paint_context);

        assert!(
            matches!(draw_list.cmds.first(), Some(DrawCmd::PushClip(rect)) if *rect == status_rect)
        );
        assert!(matches!(draw_list.cmds.last(), Some(DrawCmd::PopClip)));
        assert!(
            draw_list
                .cmds
                .iter()
                .filter_map(|command| match command {
                    DrawCmd::TextLayout { x, .. } => Some(*x),
                    _ => None,
                })
                .all(|text_x| text_x >= status_rect.x)
        );
    }

    #[test]
    fn icon_keeps_visual_gap_above_title() {
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "请选择笔记".to_owned(),
            description: "编辑器将在此处显示。".to_owned(),
            icon: Some("file-text".to_owned()),
            ..StatusStateInput::default()
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 320.0, 240.0), 1.0);
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };

        widget.paint(&mut paint_context);

        let icon_bottom = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::FillTriangle { p0, p1, p2, .. } => Some(p0[1].max(p1[1]).max(p2[1])),
                _ => None,
            })
            .reduce(f32::max)
            .expect("file-text icon should emit triangles");
        let title_top = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TextLayout { layout, y_baseline, .. } if layout.text == "请选择笔记" => {
                    Some(y_baseline - layout.font_size)
                }
                _ => None,
            })
            .expect("title should emit a text layout");

        assert!(title_top - icon_bottom >= STATUS_ICON_TITLE_GAP);
    }

    #[test]
    fn missing_action_does_not_consume_input() {
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            kind: StatusStateKind::Loading,
            title: "Loading".to_owned(),
            ..StatusStateInput::default()
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 320.0, 240.0), 1.0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);

        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 160.0, py: 200.0, button: MouseButton::Left },
                &mut context
            ),
            None
        );
    }
}
