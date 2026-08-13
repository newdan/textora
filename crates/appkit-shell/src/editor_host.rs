//! 编辑区"黑盒 widget"：只接收 dock 算给的 rect，不做任何渲染、不响应事件。

use ui::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget};

#[derive(Debug)]
pub struct EditorHostWidget {
    rect: Rect,
}

impl Default for EditorHostWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorHostWidget {
    pub fn new() -> Self {
        Self { rect: Rect::ZERO }
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }
}

impl Widget for EditorHostWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }
    fn paint(&self, _ctx: &mut PaintCtx) {}
    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }
    fn on_event(
        &mut self,
        _ev: &Event,
        _ctx: &mut EventCtx,
    ) -> Option<ui::core::widget::WidgetAction> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::Theme;
    use ui::core::{DrawList, NoopMeasure};

    fn theme() -> Theme {
        ui::theme::test_theme()
    }
    #[test]
    fn new_has_zero_rect() {
        assert_eq!(EditorHostWidget::new().rect(), Rect::ZERO);
    }

    #[test]
    fn set_rect_and_paint() {
        let t = theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        let mut w = EditorHostWidget::new();
        w.set_rect(Rect::new(220., 32., 968., 744.), &mut lc);
        assert_eq!(w.rect(), Rect::new(220., 32., 968., 744.));
        let mut dl = DrawList::new();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: None,
        };
        w.paint(&mut pc);
        assert_eq!(dl.cmds.len(), 0);
    }

    #[test]
    fn hit_test() {
        let t = theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        let mut w = EditorHostWidget::new();
        w.set_rect(Rect::new(100., 100., 50., 50.), &mut lc);
        assert!(w.hit(120., 120.));
        assert!(!w.hit(50., 50.));
    }

    #[test]
    fn on_event_returns_none() {
        let mut w = EditorHostWidget::new();
        let t = theme();
        let mut ec = EventCtx::new(&t, 1.0);
        assert!(w.on_event(&Event::MouseMove { px: 0., py: 0. }, &mut ec).is_none());
    }
}
