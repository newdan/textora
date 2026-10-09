//! 将相邻内容表面收束到同一个圆角轮廓。

use crate::{PaintCtx, Rect};

/// 应用统一采用的 macOS 风格圆角；外窗描边与内容表面共用，按 DPI 缩放。
pub const SHELL_CORNER_RADIUS_LOGICAL: f32 = 16.0;
const CORNER_ARC_SEGMENTS: usize = 24;
const OUTLINE_WIDTH_PHYSICAL: f32 = 1.0;

#[derive(Clone, Copy, Debug)]
pub struct RoundedSurfaceFrame {
    pub rect: Rect,
    pub radius: f32,
}

impl RoundedSurfaceFrame {
    /// 在内容之后、浮层之前绘制，只遮去四个外角，保留内部表面分界。
    pub fn paint(&self, context: &mut PaintCtx<'_>) {
        let rect = self.rect;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let radius = self.radius.max(0.0).min(rect.w * 0.5).min(rect.h * 0.5);
        let application = context.theme.application_theme();
        for (corner, direction) in [
            ([rect.x, rect.y], [1.0, 1.0]),
            ([rect.right(), rect.y], [-1.0, 1.0]),
            ([rect.x, rect.bottom()], [1.0, -1.0]),
            ([rect.right(), rect.bottom()], [-1.0, -1.0]),
        ] {
            Self::paint_corner(context, corner, direction, radius, application.window_surface);
        }
        context.list.stroke_rounded(rect, application.divider, radius, OUTLINE_WIDTH_PHYSICAL);
    }

    fn paint_corner(
        context: &mut PaintCtx<'_>,
        corner: [f32; 2],
        direction: [f32; 2],
        radius: f32,
        color: [f32; 4],
    ) {
        if radius <= 0.0 {
            return;
        }
        let arc_point = |segment: usize| {
            let angle = std::f32::consts::FRAC_PI_2 * segment as f32 / CORNER_ARC_SEGMENTS as f32;
            [
                corner[0] + direction[0] * radius * (1.0 - angle.cos()),
                corner[1] + direction[1] * radius * (1.0 - angle.sin()),
            ]
        };
        for segment in 0..CORNER_ARC_SEGMENTS {
            context.list.fill_triangle(corner, arc_point(segment), arc_point(segment + 1), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawCmd, DrawList, Theme};

    #[test]
    fn shared_outline_masks_only_the_outer_corners_at_each_dpi() {
        for dpi in [1.0, 1.5, 2.0] {
            let rect = Rect::new(220.0 * dpi, 36.0 * dpi, 976.0 * dpi, 760.0 * dpi);
            let frame = RoundedSurfaceFrame { rect, radius: SHELL_CORNER_RADIUS_LOGICAL * dpi };
            let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
            let mut list = DrawList::new();
            frame.paint(&mut PaintCtx::new(&mut list, &theme, dpi));
            for command in &list.cmds[..list.cmds.len() - 1] {
                let DrawCmd::FillTriangle { p0, p1, p2, color } = command else {
                    panic!("corner mask should contain only triangles");
                };
                assert_eq!(*color, theme.application_theme().window_surface);
                for point in [p0, p1, p2] {
                    assert!((point[0] - p0[0]).abs() <= frame.radius);
                    assert!((point[1] - p0[1]).abs() <= frame.radius);
                }
                assert!(
                    (p0[0] == rect.x || p0[0] == rect.right())
                        && (p0[1] == rect.y || p0[1] == rect.bottom())
                );
            }
            assert!(
                matches!(list.cmds.last(), Some(DrawCmd::StrokeRect { rect: outline, radius, line_width, .. })
                if *outline == rect && *radius == frame.radius && *line_width == OUTLINE_WIDTH_PHYSICAL)
            );
        }
    }

    #[test]
    fn tiny_surfaces_clamp_the_radius_and_empty_surfaces_emit_nothing() {
        let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_dark());
        let mut list = DrawList::new();
        RoundedSurfaceFrame { rect: Rect::ZERO, radius: SHELL_CORNER_RADIUS_LOGICAL }
            .paint(&mut PaintCtx::new(&mut list, &theme, 1.0));
        assert!(list.cmds.is_empty());
        RoundedSurfaceFrame {
            rect: Rect::new(0.0, 0.0, 8.0, 4.0),
            radius: SHELL_CORNER_RADIUS_LOGICAL,
        }
        .paint(&mut PaintCtx::new(&mut list, &theme, 1.0));
        assert!(
            matches!(list.cmds.last(), Some(DrawCmd::StrokeRect { radius, .. }) if *radius == 2.0)
        );
    }
}
