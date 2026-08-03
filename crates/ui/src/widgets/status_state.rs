//! 通用空、加载与可恢复错误状态组件。

use std::any::Any;

use crate::core::text_util::compute_text_width;
use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    DrawCmd, Event, EventCtx, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::widgets::icon::draw_icon;

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
                title_y - icon_size - 12.0 * ctx.dpi,
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
            let background = if self.pressed_action {
                ctx.theme.palette.bg_active
            } else if self.hovered_action {
                ctx.theme.palette.bg_hover
            } else {
                ctx.theme.palette.bg_elevated
            };
            ctx.list.fill_rounded(self.action_rect, background, 6.0 * ctx.dpi);
            let label = self.input.action_label.as_deref().unwrap_or_default();
            let label_width =
                compute_text_width(label, description_font_size, ctx.shaper.as_deref_mut());
            let label_x = self.action_rect.x + (self.action_rect.w - label_width) * 0.5;
            ctx.text(
                label_x,
                self.action_rect.y + self.action_rect.h * 0.5 + description_font_size * 0.35,
                description_font_size,
                ctx.theme.palette.text_main,
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
                self.hovered_action = self.action_rect.contains(*px, *py);
                if self.hovered_action {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.pressed_action = self.action_rect.contains(*px, *py);
                self.pressed_action.then_some(WidgetAction::Consumed)
            }
            Event::MouseUp { px, py, button: MouseButton::Left } if self.pressed_action => {
                self.pressed_action = false;
                if self.action_rect.contains(*px, *py) {
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
    fn missing_action_does_not_consume_input() {
        let mut widget = StatusStateWidget::new();
        widget.set_input(StatusStateInput {
            kind: StatusStateKind::Loading,
            title: "Loading".to_owned(),
            ..StatusStateInput::default()
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 320.0, 240.0), 1.0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 160.0, py: 200.0, button: MouseButton::Left },
                &mut context
            ),
            None
        );
    }
}
