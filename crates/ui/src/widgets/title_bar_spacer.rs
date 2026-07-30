//! TitleBarSpacer：Dock 内占位 widget，不绘制，不交互。
//! 用于 Sidebar 模式下让 Dock 感知 TitleBar 高度，
//! 避免 Scrollbar 顶端与 TitleBar overlay 重叠。

use crate::core::geom::Rect;
use crate::core::widget::Event;
use crate::core::widget::{EventCtx, LayoutCtx, PaintCtx, Widget, WidgetAction};

pub struct TitleBarSpacerWidget {
    rect: Rect,
}

impl Default for TitleBarSpacerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleBarSpacerWidget {
    pub fn new() -> Self {
        Self { rect: Rect::ZERO }
    }
}

impl Widget for TitleBarSpacerWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, _ctx: &mut PaintCtx) {
        // 透明占位，不绘制任何内容
    }

    fn hit(&self, _px: f32, _py: f32) -> bool {
        false
    }

    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
