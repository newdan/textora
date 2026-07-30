use crate::core::Rect;

const DEFAULT_DPI_SCALE: f32 = 1.0;
const DEFAULT_MAX_WIDTH_RATIO: f32 = 0.92;
const DEFAULT_MAX_HEIGHT_RATIO: f32 = 0.90;
const MIN_RATIO: f32 = 0.0;
const MAX_RATIO: f32 = 1.0;
const DEFAULT_COORDINATE: f32 = 0.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OverlayLayout {
    Fixed(Rect),
    Centered {
        preferred_size: (f32, f32),
        min_margin: f32,
        max_width_ratio: f32,
        max_height_ratio: f32,
    },
}

impl OverlayLayout {
    pub fn resolve(&self, screen_rect: Rect, dpi: f32) -> Rect {
        match *self {
            Self::Fixed(rect) => sanitize_rect(rect),
            Self::Centered { preferred_size, min_margin, max_width_ratio, max_height_ratio } => {
                let screen_rect = sanitize_rect(screen_rect);
                let dpi = sanitize_dpi_scale(dpi);
                let preferred_width = scale_logical_dimension(preferred_size.0, dpi);
                let preferred_height = scale_logical_dimension(preferred_size.1, dpi);
                let min_margin = scale_logical_dimension(min_margin, dpi);
                let width_cap = resolve_centered_dimension_cap(
                    screen_rect.w,
                    min_margin,
                    max_width_ratio,
                    DEFAULT_MAX_WIDTH_RATIO,
                );
                let height_cap = resolve_centered_dimension_cap(
                    screen_rect.h,
                    min_margin,
                    max_height_ratio,
                    DEFAULT_MAX_HEIGHT_RATIO,
                );
                let resolved_width = preferred_width.min(width_cap);
                let resolved_height = preferred_height.min(height_cap);
                let resolved_x = screen_rect.x + (screen_rect.w - resolved_width) * 0.5;
                let resolved_y = screen_rect.y + (screen_rect.h - resolved_height) * 0.5;

                Rect::new(resolved_x, resolved_y, resolved_width, resolved_height)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayInputPolicy {
    Modal,
    PassThrough,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissPolicy {
    ExplicitOnly,
    EscapeOrExplicit,
    EscapeBackdropOrExplicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayAction {
    DismissRequested,
}

fn resolve_centered_dimension_cap(
    screen_dimension: f32,
    min_margin: f32,
    ratio: f32,
    fallback_ratio: f32,
) -> f32 {
    let margin_limited_dimension = (screen_dimension - min_margin * 2.0).max(0.0);
    let ratio_limited_dimension = screen_dimension * sanitize_ratio(ratio, fallback_ratio);

    margin_limited_dimension.max(ratio_limited_dimension).min(screen_dimension)
}

fn sanitize_rect(rect: Rect) -> Rect {
    Rect::new(
        sanitize_coordinate(rect.x),
        sanitize_coordinate(rect.y),
        sanitize_non_negative_dimension(rect.w),
        sanitize_non_negative_dimension(rect.h),
    )
}

fn sanitize_coordinate(value: f32) -> f32 {
    if value.is_finite() { value } else { DEFAULT_COORDINATE }
}

fn sanitize_non_negative_dimension(value: f32) -> f32 {
    if value.is_finite() { value.max(0.0) } else { 0.0 }
}

fn sanitize_dpi_scale(dpi: f32) -> f32 {
    if dpi.is_finite() && dpi > 0.0 { dpi } else { DEFAULT_DPI_SCALE }
}

fn scale_logical_dimension(logical_dimension: f32, dpi: f32) -> f32 {
    sanitize_non_negative_dimension(logical_dimension) * dpi
}

fn sanitize_ratio(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value.clamp(MIN_RATIO, MAX_RATIO) } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_layout_respects_preferred_size_and_min_margin() {
        let layout = OverlayLayout::Centered {
            preferred_size: (900.0, 640.0),
            min_margin: 24.0,
            max_width_ratio: 0.92,
            max_height_ratio: 0.90,
        };

        assert_eq!(
            layout.resolve(Rect::new(0.0, 0.0, 1200.0, 800.0), 1.0),
            Rect::new(150.0, 80.0, 900.0, 640.0)
        );
        assert_eq!(
            layout.resolve(Rect::new(0.0, 0.0, 640.0, 480.0), 1.0),
            Rect::new(24.0, 24.0, 592.0, 432.0)
        );
    }

    #[test]
    fn centered_layout_scales_logical_dimensions_once_by_dpi() {
        let layout = OverlayLayout::Centered {
            preferred_size: (100.0, 50.0),
            min_margin: 10.0,
            max_width_ratio: 1.0,
            max_height_ratio: 1.0,
        };

        assert_eq!(
            layout.resolve(Rect::new(0.0, 0.0, 1000.0, 800.0), 2.0),
            Rect::new(400.0, 350.0, 200.0, 100.0)
        );
    }

    #[test]
    fn centered_layout_clamps_invalid_inputs_to_finite_non_negative_rect() {
        let layout = OverlayLayout::Centered {
            preferred_size: (f32::NAN, -50.0),
            min_margin: f32::INFINITY,
            max_width_ratio: f32::NAN,
            max_height_ratio: -1.0,
        };

        let resolved = layout.resolve(Rect::new(f32::NAN, 10.0, -120.0, f32::INFINITY), -2.0);

        assert!(resolved.x.is_finite());
        assert!(resolved.y.is_finite());
        assert!(resolved.w.is_finite());
        assert!(resolved.h.is_finite());
        assert!(resolved.w >= 0.0);
        assert!(resolved.h >= 0.0);
    }
}
