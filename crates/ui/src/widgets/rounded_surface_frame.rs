//! 将相邻内容表面收束到同一个圆角轮廓。

use crate::tapered_path::{TaperedMesh, TaperedMeshVertex};
use crate::{PaintCtx, Rect};
use std::sync::Arc;

/// 应用统一采用的 macOS 风格圆角；外窗描边与内容表面共用，按 DPI 缩放。
pub const SHELL_CORNER_RADIUS_LOGICAL: f32 = 16.0;
const CORNER_FEATHER_HALF_WIDTH_PX: f32 = 0.5;
const MAX_CORNER_ARC_ERROR_PX: f32 = 0.01;
const OUTLINE_WIDTH_LOGICAL: f32 = 1.0;

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
        if radius > 0.0 {
            context.list.clip(rect, |list| {
                for (corner, direction) in [
                    ([rect.x, rect.y], [1.0, 1.0]),
                    ([rect.right(), rect.y], [-1.0, 1.0]),
                    ([rect.x, rect.bottom()], [1.0, -1.0]),
                    ([rect.right(), rect.bottom()], [-1.0, -1.0]),
                ] {
                    list.tapered_mesh(
                        Self::corner_mask(corner, direction, radius),
                        [0.0; 2],
                        application.window_surface,
                    );
                }
            });
        }
        context.list.stroke_rounded(
            rect,
            application.divider,
            radius,
            OUTLINE_WIDTH_LOGICAL * context.dpi,
        );
    }

    fn corner_mask(corner: [f32; 2], direction: [f32; 2], radius: f32) -> Arc<TaperedMesh> {
        let feather = CORNER_FEATHER_HALF_WIDTH_PX.min(radius);
        let segment_angle =
            2.0 * (1.0 - MAX_CORNER_ARC_ERROR_PX / (radius + feather)).clamp(-1.0, 1.0).acos();
        let segments = (std::f32::consts::FRAC_PI_2 / segment_angle).ceil().max(1.0) as usize;
        let arc_vertex = |segment: usize, offset: f32, alpha_multiplier: f32| {
            let angle = std::f32::consts::FRAC_PI_2 * segment as f32 / segments as f32;
            TaperedMeshVertex {
                position: [
                    corner[0] + direction[0] * (radius - (radius + offset) * angle.cos()),
                    corner[1] + direction[1] * (radius - (radius + offset) * angle.sin()),
                ],
                alpha_multiplier,
            }
        };
        let anchor = TaperedMeshVertex { position: corner, alpha_multiplier: 1.0 };
        let vertices = (0..segments)
            .flat_map(|segment| {
                let solid_start = arc_vertex(segment, feather, 1.0);
                let solid_end = arc_vertex(segment + 1, feather, 1.0);
                let transparent_start = arc_vertex(segment, -feather, 0.0);
                let transparent_end = arc_vertex(segment + 1, -feather, 0.0);
                [
                    anchor,
                    solid_start,
                    solid_end,
                    solid_start,
                    transparent_start,
                    solid_end,
                    solid_end,
                    transparent_start,
                    transparent_end,
                ]
            })
            .collect();
        let bounds = Rect::new(
            corner[0] + direction[0].min(0.0) * radius - feather,
            corner[1] + direction[1].min(0.0) * radius - feather,
            radius + feather * 2.0,
            radius + feather * 2.0,
        );
        Arc::new(TaperedMesh { vertices, bounds })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DrawCmd, DrawList, Theme};

    #[test]
    fn corner_mask_has_continuous_pixel_coverage_at_fractional_dpi() {
        let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
        for dpi in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let radius = SHELL_CORNER_RADIUS_LOGICAL * dpi;
            let rect = Rect::new(17.0 * dpi, 23.0 * dpi, 200.0 * dpi, 120.0 * dpi);
            let mut list = DrawList::new();
            RoundedSurfaceFrame { rect, radius }.paint(&mut PaintCtx::new(&mut list, &theme, dpi));
            for (corner, direction) in [
                ([rect.x, rect.y], [1.0, 1.0]),
                ([rect.right(), rect.y], [-1.0, 1.0]),
                ([rect.x, rect.bottom()], [1.0, -1.0]),
                ([rect.right(), rect.bottom()], [-1.0, -1.0]),
            ] {
                for angle in [0.2_f32, 0.5, 0.8, 1.2] {
                    for (distance, expected) in [(-0.25, 0.25), (0.0, 0.5), (0.25, 0.75)] {
                        let point = [
                            corner[0] + direction[0] * (radius - (radius + distance) * angle.cos()),
                            corner[1] + direction[1] * (radius - (radius + distance) * angle.sin()),
                        ];
                        let coverage = mask_coverage(&list, point);
                        assert!(
                            (coverage - expected).abs() < 0.03,
                            "dpi={dpi}, angle={angle}, distance={distance}: {coverage} != {expected}"
                        );
                    }
                }
            }
        }
    }

    fn mask_coverage(list: &DrawList, point: [f32; 2]) -> f32 {
        list.cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::FillTriangle { p0, p1, p2, .. } => {
                    triangle_coverage([*p0, *p1, *p2], [1.0; 3], point)
                }
                DrawCmd::TaperedMesh { mesh, translation, .. } => {
                    let local = [point[0] - translation[0], point[1] - translation[1]];
                    mesh.vertices.chunks_exact(3).find_map(|triangle| {
                        triangle_coverage(
                            [triangle[0].position, triangle[1].position, triangle[2].position],
                            [
                                triangle[0].alpha_multiplier,
                                triangle[1].alpha_multiplier,
                                triangle[2].alpha_multiplier,
                            ],
                            local,
                        )
                    })
                }
                _ => None,
            })
            .fold(0.0, f32::max)
    }

    fn triangle_coverage(corners: [[f32; 2]; 3], alpha: [f32; 3], point: [f32; 2]) -> Option<f32> {
        let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        let area = cross(corners[0], corners[1], corners[2]);
        if area.abs() < f32::EPSILON {
            return None;
        }
        let weights = [
            cross(corners[1], corners[2], point) / area,
            cross(corners[2], corners[0], point) / area,
            cross(corners[0], corners[1], point) / area,
        ];
        weights
            .iter()
            .all(|weight| *weight >= -f32::EPSILON)
            .then(|| weights.iter().zip(alpha).map(|(weight, alpha)| weight * alpha).sum())
    }

    #[test]
    fn shared_outline_matches_the_internal_pane_divider_color() {
        for definition in [
            crate::theme::ThemeDefinition::default_light(),
            crate::theme::ThemeDefinition::default_dark(),
        ] {
            let theme = Theme::from_definition(&definition);
            let mut list = DrawList::new();
            RoundedSurfaceFrame {
                rect: Rect::new(0.0, 0.0, 800.0, 600.0),
                radius: SHELL_CORNER_RADIUS_LOGICAL,
            }
            .paint(&mut PaintCtx::new(&mut list, &theme, 1.0));
            assert!(matches!(list.cmds.last(), Some(DrawCmd::StrokeRect { color, .. })
                if *color == theme.palette.border_subtle));
        }
    }

    #[test]
    fn shared_outline_masks_only_the_outer_corners_at_each_dpi() {
        for dpi in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let rect = Rect::new(220.0 * dpi, 36.0 * dpi, 976.0 * dpi, 760.0 * dpi);
            let frame = RoundedSurfaceFrame { rect, radius: SHELL_CORNER_RADIUS_LOGICAL * dpi };
            let theme = Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
            let mut list = DrawList::new();
            frame.paint(&mut PaintCtx::new(&mut list, &theme, dpi));
            assert!(matches!(list.cmds.first(), Some(DrawCmd::PushClip(clip)) if *clip == rect));
            assert!(matches!(list.cmds[list.cmds.len() - 2], DrawCmd::PopClip));
            for command in &list.cmds[..list.cmds.len() - 1] {
                let DrawCmd::TaperedMesh { mesh, color, .. } = command else {
                    assert!(matches!(command, DrawCmd::PushClip(_) | DrawCmd::PopClip));
                    continue;
                };
                assert_eq!(*color, theme.application_theme().window_surface);
                assert!(mesh.vertices.iter().all(|vertex| {
                    vertex.position[0] >= mesh.bounds.x
                        && vertex.position[0] <= mesh.bounds.right()
                        && vertex.position[1] >= mesh.bounds.y
                        && vertex.position[1] <= mesh.bounds.bottom()
                }));
                assert!(mesh.bounds.w <= frame.radius + 1.0);
                assert!(mesh.bounds.h <= frame.radius + 1.0);
            }
            assert!(
                matches!(list.cmds.last(), Some(DrawCmd::StrokeRect { rect: outline, radius, line_width, .. })
                if *outline == rect && *radius == frame.radius && *line_width == dpi)
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
