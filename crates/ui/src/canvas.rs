//! 画布视口的纯数据几何计算。

use crate::core::geom::Rect;

pub const MIN_MANUAL_ZOOM: f32 = 0.25;
pub const MIN_INITIAL_FIT_ZOOM: f32 = 0.40;
pub const MAX_CANVAS_ZOOM: f32 = 4.0;
pub const DEFAULT_CANVAS_ZOOM: f32 = 1.0;
pub const BASE_CONTENT_PADDING_LOGICAL: f32 = 64.0;
pub const MIN_SCREEN_PADDING_LOGICAL: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasPoint {
    pub x: f32,
    pub y: f32,
}

impl CanvasPoint {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasViewPosition {
    pub zoom: f32,
    pub scroll: CanvasPoint,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasViewportConfig {
    pub base_content_padding: f32,
    pub min_screen_padding: f32,
    pub min_initial_fit_zoom: f32,
}

impl CanvasViewportConfig {
    pub const DEFAULT: Self = Self {
        base_content_padding: BASE_CONTENT_PADDING_LOGICAL,
        min_screen_padding: MIN_SCREEN_PADDING_LOGICAL,
        min_initial_fit_zoom: DEFAULT_CANVAS_ZOOM,
    };

    pub fn for_dpi(dpi_scale: f32) -> Self {
        let safe_dpi_scale = if dpi_scale.is_finite() && dpi_scale > 0.0 { dpi_scale } else { 1.0 };
        Self {
            base_content_padding: BASE_CONTENT_PADDING_LOGICAL * safe_dpi_scale,
            min_screen_padding: MIN_SCREEN_PADDING_LOGICAL * safe_dpi_scale,
            ..Self::DEFAULT
        }
    }

    fn normalized(self) -> Self {
        Self {
            base_content_padding: finite_non_negative_or(
                self.base_content_padding,
                BASE_CONTENT_PADDING_LOGICAL,
            ),
            min_screen_padding: finite_non_negative_or(
                self.min_screen_padding,
                MIN_SCREEN_PADDING_LOGICAL,
            ),
            min_initial_fit_zoom: finite_positive_or(
                self.min_initial_fit_zoom,
                DEFAULT_CANVAS_ZOOM,
            )
            .clamp(MIN_INITIAL_FIT_ZOOM, DEFAULT_CANVAS_ZOOM),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasViewportInput {
    pub viewport: Rect,
    pub content_bounds: Rect,
    pub view_position: Option<CanvasViewPosition>,
    pub config: CanvasViewportConfig,
}

impl CanvasViewportInput {
    pub fn initial(viewport: Rect, content_bounds: Rect, config: CanvasViewportConfig) -> Self {
        Self { viewport, content_bounds, view_position: None, config }
    }

    pub fn positioned(
        viewport: Rect,
        content_bounds: Rect,
        view_position: CanvasViewPosition,
        config: CanvasViewportConfig,
    ) -> Self {
        Self { viewport, content_bounds, view_position: Some(view_position), config }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasViewportSnapshot {
    pub viewport: Rect,
    pub content_bounds: Rect,
    pub scaled_content_bounds: Rect,
    pub zoom: f32,
    pub scroll: CanvasPoint,
    pub max_scroll: CanvasPoint,
    pub content_offset: CanvasPoint,
    config: CanvasViewportConfig,
}

impl CanvasViewportSnapshot {
    pub const EMPTY: Self = Self {
        viewport: Rect::ZERO,
        content_bounds: Rect::ZERO,
        scaled_content_bounds: Rect::ZERO,
        zoom: DEFAULT_CANVAS_ZOOM,
        scroll: CanvasPoint::ZERO,
        max_scroll: CanvasPoint::ZERO,
        content_offset: CanvasPoint::ZERO,
        config: CanvasViewportConfig::DEFAULT,
    };

    pub fn position(self) -> CanvasViewPosition {
        CanvasViewPosition { zoom: self.zoom, scroll: self.scroll }
    }

    pub fn clamp_position(self, position: CanvasViewPosition) -> CanvasViewPosition {
        resolve_viewport(CanvasViewportInput::positioned(
            self.viewport,
            self.content_bounds,
            position,
            self.config,
        ))
        .position()
    }

    pub fn content_to_screen(self, point: CanvasPoint) -> CanvasPoint {
        CanvasPoint::new(
            self.viewport.x + self.content_offset.x + (point.x - self.content_bounds.x) * self.zoom
                - self.scroll.x,
            self.viewport.y + self.content_offset.y + (point.y - self.content_bounds.y) * self.zoom
                - self.scroll.y,
        )
    }

    pub fn screen_to_content(self, point: CanvasPoint) -> CanvasPoint {
        CanvasPoint::new(
            (point.x - self.viewport.x - self.content_offset.x + self.scroll.x) / self.zoom
                + self.content_bounds.x,
            (point.y - self.viewport.y - self.content_offset.y + self.scroll.y) / self.zoom
                + self.content_bounds.y,
        )
    }

    pub fn content_rect_to_screen(self, rect: Rect) -> Rect {
        let origin = self.content_to_screen(CanvasPoint::new(rect.x, rect.y));
        Rect::new(origin.x, origin.y, rect.w * self.zoom, rect.h * self.zoom)
    }

    pub fn screen_rect_to_content(self, rect: Rect) -> Rect {
        let origin = self.screen_to_content(CanvasPoint::new(rect.x, rect.y));
        Rect::new(origin.x, origin.y, rect.w / self.zoom, rect.h / self.zoom)
    }
}

pub fn resolve_viewport(input: CanvasViewportInput) -> CanvasViewportSnapshot {
    if !is_finite_rect(input.viewport) || !is_finite_rect(input.content_bounds) {
        return CanvasViewportSnapshot::EMPTY;
    }

    let config = input.config.normalized();
    let zoom = input
        .view_position
        .map(|position| clamp_zoom(position.zoom))
        .unwrap_or_else(|| initial_fit_zoom(input.viewport, input.content_bounds, config));
    let geometry = AxisGeometry::resolve(input.viewport, input.content_bounds, zoom, config);
    let scaled_content_bounds = Rect::new(
        input.content_bounds.x * zoom,
        input.content_bounds.y * zoom,
        input.content_bounds.w * zoom,
        input.content_bounds.h * zoom,
    );
    if !is_finite_rect(scaled_content_bounds) || !geometry.is_finite() {
        return CanvasViewportSnapshot::EMPTY;
    }
    let requested_scroll =
        input.view_position.map_or(CanvasPoint::ZERO, |position| position.scroll);

    CanvasViewportSnapshot {
        viewport: input.viewport,
        content_bounds: input.content_bounds,
        scaled_content_bounds,
        zoom,
        scroll: CanvasPoint::new(
            clamp_scroll(requested_scroll.x, geometry.horizontal.max_scroll),
            clamp_scroll(requested_scroll.y, geometry.vertical.max_scroll),
        ),
        max_scroll: CanvasPoint::new(geometry.horizontal.max_scroll, geometry.vertical.max_scroll),
        content_offset: CanvasPoint::new(
            geometry.horizontal.content_offset,
            geometry.vertical.content_offset,
        ),
        config,
    }
}

#[derive(Clone, Copy)]
struct AxisGeometry {
    horizontal: AxisLayout,
    vertical: AxisLayout,
}

impl AxisGeometry {
    fn resolve(
        viewport: Rect,
        content_bounds: Rect,
        zoom: f32,
        config: CanvasViewportConfig,
    ) -> Self {
        Self {
            horizontal: AxisLayout::resolve(viewport.w, content_bounds.w, zoom, config),
            vertical: AxisLayout::resolve(viewport.h, content_bounds.h, zoom, config),
        }
    }

    fn is_finite(self) -> bool {
        self.horizontal.is_finite() && self.vertical.is_finite()
    }
}

#[derive(Clone, Copy)]
struct AxisLayout {
    content_offset: f32,
    max_scroll: f32,
}

impl AxisLayout {
    fn resolve(
        viewport_extent: f32,
        content_extent: f32,
        zoom: f32,
        config: CanvasViewportConfig,
    ) -> Self {
        let scaled_content_extent = content_extent * zoom;
        let padding = (config.base_content_padding * zoom).max(config.min_screen_padding);
        let canvas_extent = scaled_content_extent + padding * 2.0;
        if canvas_extent <= viewport_extent {
            return Self {
                content_offset: (viewport_extent - scaled_content_extent) * 0.5,
                max_scroll: 0.0,
            };
        }
        Self { content_offset: padding, max_scroll: canvas_extent - viewport_extent }
    }

    fn is_finite(self) -> bool {
        self.content_offset.is_finite() && self.max_scroll.is_finite() && self.max_scroll >= 0.0
    }
}

fn initial_fit_zoom(viewport: Rect, content_bounds: Rect, config: CanvasViewportConfig) -> f32 {
    let horizontal_fit = fit_zoom_for_axis(viewport.w, content_bounds.w, config);
    let vertical_fit = fit_zoom_for_axis(viewport.h, content_bounds.h, config);
    horizontal_fit.min(vertical_fit).max(config.min_initial_fit_zoom)
}

fn fit_zoom_for_axis(
    viewport_extent: f32,
    content_extent: f32,
    config: CanvasViewportConfig,
) -> f32 {
    let content_padding_fit =
        viewport_extent / (content_extent + 2.0 * config.base_content_padding);
    let screen_padding_fit =
        (viewport_extent - 2.0 * config.min_screen_padding).max(1.0) / content_extent.max(1.0);
    content_padding_fit.min(screen_padding_fit).min(DEFAULT_CANVAS_ZOOM)
}

fn clamp_zoom(zoom: f32) -> f32 {
    if !zoom.is_finite() {
        return DEFAULT_CANVAS_ZOOM;
    }
    zoom.clamp(MIN_MANUAL_ZOOM, MAX_CANVAS_ZOOM)
}

fn clamp_scroll(scroll: f32, max_scroll: f32) -> f32 {
    if !scroll.is_finite() {
        return 0.0;
    }
    scroll.clamp(0.0, max_scroll)
}

fn is_finite_rect(rect: Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.w.is_finite()
        && rect.h.is_finite()
        && rect.w >= 0.0
        && rect.h >= 0.0
        && (rect.x + rect.w).is_finite()
        && (rect.y + rect.h).is_finite()
}

fn finite_positive_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 { value } else { fallback }
}

fn finite_non_negative_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= 0.0 { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geom::Rect;

    #[test]
    fn small_content_is_centered_on_both_axes() {
        let snapshot = resolve_viewport(CanvasViewportInput::initial(
            Rect::new(100.0, 50.0, 800.0, 600.0),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            CanvasViewportConfig::for_dpi(1.0),
        ));
        assert_eq!(snapshot.max_scroll, CanvasPoint::ZERO);
        assert_eq!(snapshot.content_to_screen(CanvasPoint::ZERO), CanvasPoint::new(400.0, 300.0));
    }

    #[test]
    fn initial_view_opens_large_content_at_actual_size() {
        let snapshot = resolve_viewport(CanvasViewportInput::initial(
            Rect::new(0.0, 0.0, 320.0, 240.0),
            Rect::new(0.0, 0.0, 2_000.0, 1_200.0),
            CanvasViewportConfig::for_dpi(1.0),
        ));

        assert_eq!(snapshot.zoom, DEFAULT_CANVAS_ZOOM);
        assert!(snapshot.max_scroll.x > 0.0 && snapshot.max_scroll.y > 0.0);
    }

    #[test]
    fn initial_fit_stops_at_readable_floor() {
        let config = CanvasViewportConfig {
            min_initial_fit_zoom: MIN_INITIAL_FIT_ZOOM,
            ..CanvasViewportConfig::for_dpi(1.0)
        };
        let snapshot = resolve_viewport(CanvasViewportInput::initial(
            Rect::new(0.0, 0.0, 320.0, 240.0),
            Rect::new(0.0, 0.0, 2_000.0, 1_200.0),
            config,
        ));
        assert_eq!(snapshot.zoom, 0.40);
        assert!(snapshot.max_scroll.x > 0.0 && snapshot.max_scroll.y > 0.0);
    }

    #[test]
    fn negative_content_coordinates_use_the_same_centering_transform() {
        let snapshot = resolve_viewport(CanvasViewportInput::positioned(
            Rect::new(10.0, 20.0, 300.0, 200.0),
            Rect::new(-100.0, -50.0, 600.0, 50.0),
            CanvasViewPosition { zoom: DEFAULT_CANVAS_ZOOM, scroll: CanvasPoint::ZERO },
            CanvasViewportConfig::for_dpi(1.0),
        ));

        assert_eq!(
            snapshot.content_to_screen(CanvasPoint::new(-100.0, -50.0)),
            CanvasPoint::new(74.0, 95.0)
        );
        assert_eq!(snapshot.max_scroll, CanvasPoint::new(428.0, 0.0));
    }

    #[test]
    fn overflow_is_calculated_independently_for_each_axis() {
        let config = CanvasViewportConfig::for_dpi(1.0);
        let viewport = Rect::new(0.0, 0.0, 400.0, 300.0);

        let positioned_input = |content_bounds| {
            CanvasViewportInput::positioned(
                viewport,
                content_bounds,
                CanvasViewPosition { zoom: DEFAULT_CANVAS_ZOOM, scroll: CanvasPoint::ZERO },
                config,
            )
        };
        let horizontal = resolve_viewport(positioned_input(Rect::new(0.0, 0.0, 800.0, 50.0)));
        let vertical = resolve_viewport(positioned_input(Rect::new(0.0, 0.0, 50.0, 800.0)));
        let both = resolve_viewport(positioned_input(Rect::new(0.0, 0.0, 800.0, 800.0)));

        assert!(horizontal.max_scroll.x > 0.0 && horizontal.max_scroll.y == 0.0);
        assert!(vertical.max_scroll.x == 0.0 && vertical.max_scroll.y > 0.0);
        assert!(both.max_scroll.x > 0.0 && both.max_scroll.y > 0.0);
    }

    #[test]
    fn point_and_rect_transforms_round_trip() {
        let snapshot = resolve_viewport(CanvasViewportInput::positioned(
            Rect::new(10.0, 20.0, 500.0, 400.0),
            Rect::new(-120.0, 40.0, 900.0, 600.0),
            CanvasViewPosition { zoom: 0.5, scroll: CanvasPoint::new(12.0, 18.0) },
            CanvasViewportConfig::for_dpi(1.0),
        ));
        let point = CanvasPoint::new(180.0, 250.0);
        let content_rect = Rect::new(100.0, 150.0, 120.0, 80.0);

        assert_point_close(snapshot.screen_to_content(snapshot.content_to_screen(point)), point);
        assert_rect_close(
            snapshot.screen_rect_to_content(snapshot.content_rect_to_screen(content_rect)),
            content_rect,
        );
    }

    #[test]
    fn positioned_view_clamps_zoom_and_scroll_to_valid_ranges() {
        let snapshot = resolve_viewport(CanvasViewportInput::positioned(
            Rect::new(0.0, 0.0, 320.0, 240.0),
            Rect::new(0.0, 0.0, 2_000.0, 1_200.0),
            CanvasViewPosition { zoom: f32::INFINITY, scroll: CanvasPoint::new(f32::MAX, -10.0) },
            CanvasViewportConfig::for_dpi(1.0),
        ));

        assert_eq!(snapshot.zoom, DEFAULT_CANVAS_ZOOM);
        assert_eq!(snapshot.scroll.x, snapshot.max_scroll.x);
        assert_eq!(snapshot.scroll.y, 0.0);
        assert_eq!(
            snapshot.clamp_position(CanvasViewPosition {
                zoom: MIN_MANUAL_ZOOM,
                scroll: CanvasPoint::new(-1.0, f32::INFINITY),
            }),
            CanvasViewPosition { zoom: MIN_MANUAL_ZOOM, scroll: CanvasPoint::ZERO }
        );
    }

    #[test]
    fn non_finite_rectangles_return_a_safe_empty_snapshot() {
        let snapshot = resolve_viewport(CanvasViewportInput::initial(
            Rect::new(f32::NAN, 0.0, 800.0, 600.0),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            CanvasViewportConfig::for_dpi(1.0),
        ));

        assert_eq!(snapshot, CanvasViewportSnapshot::EMPTY);
    }

    #[test]
    fn zoom_limits_remain_fixed_when_configuration_requests_other_bounds() {
        let config = CanvasViewportConfig {
            min_initial_fit_zoom: 2.0,
            ..CanvasViewportConfig::for_dpi(1.0)
        };
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let content_bounds = Rect::new(0.0, 0.0, 100.0, 100.0);

        let initial =
            resolve_viewport(CanvasViewportInput::initial(viewport, content_bounds, config));
        let minimum = resolve_viewport(CanvasViewportInput::positioned(
            viewport,
            content_bounds,
            CanvasViewPosition { zoom: 0.1, scroll: CanvasPoint::ZERO },
            config,
        ));
        let maximum = resolve_viewport(CanvasViewportInput::positioned(
            viewport,
            content_bounds,
            CanvasViewPosition { zoom: 8.0, scroll: CanvasPoint::ZERO },
            config,
        ));

        assert_eq!(initial.zoom, DEFAULT_CANVAS_ZOOM);
        assert_eq!(minimum.zoom, MIN_MANUAL_ZOOM);
        assert_eq!(maximum.zoom, MAX_CANVAS_ZOOM);
    }

    #[test]
    fn finite_values_that_overflow_derived_geometry_return_an_empty_snapshot() {
        let snapshot = resolve_viewport(CanvasViewportInput::positioned(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            Rect::new(f32::MAX, 0.0, f32::MAX, 100.0),
            CanvasViewPosition { zoom: DEFAULT_CANVAS_ZOOM, scroll: CanvasPoint::ZERO },
            CanvasViewportConfig::for_dpi(1.0),
        ));

        assert_eq!(snapshot, CanvasViewportSnapshot::EMPTY);
    }

    fn assert_point_close(actual: CanvasPoint, expected: CanvasPoint) {
        const FLOAT_TOLERANCE: f32 = 0.0001;
        assert!((actual.x - expected.x).abs() < FLOAT_TOLERANCE);
        assert!((actual.y - expected.y).abs() < FLOAT_TOLERANCE);
    }

    fn assert_rect_close(actual: Rect, expected: Rect) {
        const FLOAT_TOLERANCE: f32 = 0.0001;
        assert!((actual.x - expected.x).abs() < FLOAT_TOLERANCE);
        assert!((actual.y - expected.y).abs() < FLOAT_TOLERANCE);
        assert!((actual.w - expected.w).abs() < FLOAT_TOLERANCE);
        assert!((actual.h - expected.h).abs() < FLOAT_TOLERANCE);
    }
}
