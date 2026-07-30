//! 每个标签页独立持有的画布视口会话。

use ui::canvas::{
    CanvasAxis, CanvasPoint, CanvasViewPosition, CanvasViewportConfig, CanvasViewportInput,
    CanvasViewportSnapshot, resolve_viewport,
};
use ui::core::geom::Rect;
use ui::plugin::CanvasContentMetrics;
use ui::scrollbar::ScrollbarInput;

/// 画布视口的互斥状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CanvasViewportState {
    /// 首次收到有效画布指标时，根据内容与可视区进行适配。
    AwaitingInitialFit,
    /// 用户已经建立的缩放和滚动位置。
    Positioned(CanvasViewPosition),
}

/// 由输入层和滚动条转译的画布视口动作。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasViewportAction {
    PanBy(CanvasPoint),
    ZoomBy { factor: f32, screen_anchor: CanvasPoint },
    SetAxisPosition { axis: CanvasAxis, position: f32 },
    Page { axis: CanvasAxis, direction: f32 },
    ResetView,
}

/// 画布两个方向滚动条的每帧纯数据输入。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasViewportScrollbarsInput {
    pub horizontal: Option<ScrollbarInput>,
    pub vertical: Option<ScrollbarInput>,
}

/// 标签页内存中的画布视口状态；不会持久化到文档或工作区快照。
pub struct CanvasViewportSession {
    state: CanvasViewportState,
    latest_metrics: Option<CanvasContentMetrics>,
    latest_viewport: Option<Rect>,
    latest_config: Option<CanvasViewportConfig>,
    latest_snapshot: Option<CanvasViewportSnapshot>,
}

impl Default for CanvasViewportSession {
    fn default() -> Self {
        Self {
            state: CanvasViewportState::AwaitingInitialFit,
            latest_metrics: None,
            latest_viewport: None,
            latest_config: None,
            latest_snapshot: None,
        }
    }
}

impl CanvasViewportSession {
    /// 保存本帧画布指标，并解析可供渲染和交互共用的视口快照。
    pub fn prepare(
        &mut self,
        metrics: CanvasContentMetrics,
        viewport: Rect,
        config: CanvasViewportConfig,
    ) -> Option<CanvasViewportSnapshot> {
        if !is_valid_rect(metrics.content_bounds) || !is_valid_rect(viewport) {
            return self.latest_snapshot;
        }

        let metrics_changed = self.latest_metrics != Some(metrics);
        let viewport_changed = self.latest_viewport != Some(viewport);
        let previous_snapshot = self.latest_snapshot;
        self.latest_metrics = Some(metrics);
        self.latest_viewport = Some(viewport);
        self.latest_config = Some(config);

        let requested_position = match self.state {
            CanvasViewportState::AwaitingInitialFit => None,
            CanvasViewportState::Positioned(position) => Some(position),
        };
        let Some(unanchored_snapshot) = self.resolve(requested_position) else {
            return self.latest_snapshot;
        };

        let snapshot = match (metrics_changed, viewport_changed, previous_snapshot) {
            (true, _, Some(previous_snapshot)) => {
                self.resolve_preserving_anchor(previous_snapshot, metrics, unanchored_snapshot)
            }
            (false, true, Some(previous_snapshot)) => {
                self.resolve_preserving_viewport_center(previous_snapshot, unanchored_snapshot)
            }
            (false, false, _) | (_, _, None) => unanchored_snapshot,
        };
        self.store_snapshot(snapshot)
    }

    /// 返回最近一次成功解析的不可变快照。
    pub fn snapshot(&self) -> Option<CanvasViewportSnapshot> {
        self.latest_snapshot
    }

    /// 生成 UI 覆盖式双向滚动条所需的纯数据输入。
    pub fn scrollbars_input(&self) -> CanvasViewportScrollbarsInput {
        let Some(snapshot) = self.latest_snapshot else {
            return CanvasViewportScrollbarsInput::default();
        };

        CanvasViewportScrollbarsInput {
            horizontal: scrollbar_input(
                snapshot.viewport.w,
                snapshot.max_scroll.x,
                snapshot.scroll.x,
            ),
            vertical: scrollbar_input(
                snapshot.viewport.h,
                snapshot.max_scroll.y,
                snapshot.scroll.y,
            ),
        }
    }

    /// 对最近快照归约一个交互动作；没有快照时保持无副作用。
    pub fn apply(&mut self, action: CanvasViewportAction) {
        let Some(snapshot) = self.latest_snapshot else {
            return;
        };

        match action {
            CanvasViewportAction::PanBy(delta) => self.pan(snapshot, delta),
            CanvasViewportAction::ZoomBy { factor, screen_anchor } => {
                self.zoom(snapshot, factor, screen_anchor)
            }
            CanvasViewportAction::SetAxisPosition { axis, position } => {
                self.set_axis_position(snapshot, axis, position)
            }
            CanvasViewportAction::Page { axis, direction } => self.page(snapshot, axis, direction),
            CanvasViewportAction::ResetView => self.reset_view(),
        }
    }

    fn pan(&mut self, snapshot: CanvasViewportSnapshot, delta: CanvasPoint) {
        if !is_valid_point(delta) {
            return;
        }

        self.resolve_and_store(CanvasViewPosition {
            zoom: snapshot.zoom,
            scroll: CanvasPoint::new(snapshot.scroll.x + delta.x, snapshot.scroll.y + delta.y),
        });
    }

    fn zoom(&mut self, snapshot: CanvasViewportSnapshot, factor: f32, screen_anchor: CanvasPoint) {
        if !factor.is_finite() || factor <= 0.0 || !is_valid_point(screen_anchor) {
            return;
        }

        let requested_zoom = snapshot.zoom * factor;
        if !requested_zoom.is_finite() || requested_zoom <= 0.0 {
            return;
        }

        let content_anchor = snapshot.screen_to_content(screen_anchor);
        let Some(unanchored_snapshot) = self
            .resolve(Some(CanvasViewPosition { zoom: requested_zoom, scroll: snapshot.scroll }))
        else {
            return;
        };
        let position =
            position_for_screen_anchor(unanchored_snapshot, content_anchor, screen_anchor);
        self.resolve_and_store(position);
    }

    fn set_axis_position(
        &mut self,
        snapshot: CanvasViewportSnapshot,
        axis: CanvasAxis,
        position: f32,
    ) {
        if !position.is_finite() {
            return;
        }

        let scroll = match axis {
            CanvasAxis::Horizontal => CanvasPoint::new(position, snapshot.scroll.y),
            CanvasAxis::Vertical => CanvasPoint::new(snapshot.scroll.x, position),
        };
        self.resolve_and_store(CanvasViewPosition { zoom: snapshot.zoom, scroll });
    }

    fn page(&mut self, snapshot: CanvasViewportSnapshot, axis: CanvasAxis, direction: f32) {
        if !direction.is_finite() {
            return;
        }

        let distance = match axis {
            CanvasAxis::Horizontal => snapshot.viewport.w,
            CanvasAxis::Vertical => snapshot.viewport.h,
        } * direction;
        self.set_axis_position(snapshot, axis, axis_position(snapshot.scroll, axis) + distance);
    }

    fn reset_view(&mut self) {
        self.state = CanvasViewportState::AwaitingInitialFit;
        let Some(snapshot) = self.resolve(None) else {
            return;
        };
        self.store_snapshot(snapshot);
    }

    fn resolve_preserving_anchor(
        &self,
        previous_snapshot: CanvasViewportSnapshot,
        metrics: CanvasContentMetrics,
        unanchored_snapshot: CanvasViewportSnapshot,
    ) -> CanvasViewportSnapshot {
        let old_viewport_center = viewport_center(previous_snapshot.viewport);
        let content_anchor = match metrics.focus_anchor {
            Some(focus_anchor) => focus_anchor,
            None => previous_snapshot.screen_to_content(old_viewport_center),
        };
        let screen_anchor = previous_snapshot.content_to_screen(content_anchor);
        let position =
            position_for_screen_anchor(unanchored_snapshot, content_anchor, screen_anchor);

        match self.resolve(Some(position)) {
            Some(snapshot) => snapshot,
            None => unanchored_snapshot,
        }
    }

    fn resolve_preserving_viewport_center(
        &self,
        previous_snapshot: CanvasViewportSnapshot,
        unanchored_snapshot: CanvasViewportSnapshot,
    ) -> CanvasViewportSnapshot {
        let content_anchor =
            previous_snapshot.screen_to_content(viewport_center(previous_snapshot.viewport));
        let position = position_for_screen_anchor(
            unanchored_snapshot,
            content_anchor,
            viewport_center(unanchored_snapshot.viewport),
        );

        match self.resolve(Some(position)) {
            Some(snapshot) => snapshot,
            None => unanchored_snapshot,
        }
    }

    fn resolve_and_store(&mut self, position: CanvasViewPosition) {
        let Some(snapshot) = self.resolve(Some(position)) else {
            return;
        };
        self.store_snapshot(snapshot);
    }

    fn resolve(&self, position: Option<CanvasViewPosition>) -> Option<CanvasViewportSnapshot> {
        let (Some(metrics), Some(viewport), Some(config)) =
            (self.latest_metrics, self.latest_viewport, self.latest_config)
        else {
            return None;
        };

        let input = match position {
            Some(position) => {
                CanvasViewportInput::positioned(viewport, metrics.content_bounds, position, config)
            }
            None => CanvasViewportInput::initial(viewport, metrics.content_bounds, config),
        };
        Some(resolve_viewport(input))
    }

    fn store_snapshot(
        &mut self,
        snapshot: CanvasViewportSnapshot,
    ) -> Option<CanvasViewportSnapshot> {
        self.state = CanvasViewportState::Positioned(snapshot.position());
        self.latest_snapshot = Some(snapshot);
        self.latest_snapshot
    }
}

fn position_for_screen_anchor(
    snapshot: CanvasViewportSnapshot,
    content_anchor: CanvasPoint,
    screen_anchor: CanvasPoint,
) -> CanvasViewPosition {
    CanvasViewPosition {
        zoom: snapshot.zoom,
        scroll: CanvasPoint::new(
            snapshot.viewport.x
                + snapshot.content_offset.x
                + (content_anchor.x - snapshot.content_bounds.x) * snapshot.zoom
                - screen_anchor.x,
            snapshot.viewport.y
                + snapshot.content_offset.y
                + (content_anchor.y - snapshot.content_bounds.y) * snapshot.zoom
                - screen_anchor.y,
        ),
    }
}

fn scrollbar_input(viewport_extent: f32, max_scroll: f32, scroll: f32) -> Option<ScrollbarInput> {
    if !viewport_extent.is_finite() || !max_scroll.is_finite() || max_scroll <= 0.0 {
        return None;
    }

    let total_extent = viewport_extent + max_scroll;
    if !total_extent.is_finite() || total_extent <= 0.0 {
        return None;
    }

    Some(ScrollbarInput {
        viewport_height_px: f64::from(viewport_extent.max(0.0)),
        total_display_rows: total_extent.ceil() as usize,
        scroll_top_rows: f64::from(scroll.max(0.0)),
    })
}

fn axis_position(point: CanvasPoint, axis: CanvasAxis) -> f32 {
    match axis {
        CanvasAxis::Horizontal => point.x,
        CanvasAxis::Vertical => point.y,
    }
}

fn viewport_center(viewport: Rect) -> CanvasPoint {
    CanvasPoint::new(viewport.x + viewport.w * 0.5, viewport.y + viewport.h * 0.5)
}

fn is_valid_point(point: CanvasPoint) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

fn is_valid_rect(rect: Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.w.is_finite()
        && rect.h.is_finite()
        && rect.w >= 0.0
        && rect.h >= 0.0
        && (rect.x + rect.w).is_finite()
        && (rect.y + rect.h).is_finite()
}

#[cfg(test)]
mod tests {
    use ui::canvas::{CanvasAxis, CanvasPoint, CanvasViewportConfig};
    use ui::core::geom::Rect;
    use ui::plugin::CanvasContentMetrics;

    use super::{CanvasViewportAction, CanvasViewportSession};
    fn viewport() -> Rect {
        Rect::new(100.0, 50.0, 1_000.0, 800.0)
    }

    fn prepared_session() -> CanvasViewportSession {
        let mut session = CanvasViewportSession::default();
        session.prepare(
            CanvasContentMetrics {
                content_bounds: Rect::new(0.0, 0.0, 2_000.0, 2_000.0),
                focus_anchor: None,
            },
            viewport(),
            CanvasViewportConfig::for_dpi(1.0),
        );
        session
    }

    fn snapshot(session: &CanvasViewportSession) -> ui::canvas::CanvasViewportSnapshot {
        session.snapshot().expect("prepared canvas viewport session must retain a snapshot")
    }

    fn assert_point_close(actual: CanvasPoint, expected: CanvasPoint) {
        const POINT_EPSILON: f32 = 0.001;
        assert!((actual.x - expected.x).abs() < POINT_EPSILON, "x: {actual:?} != {expected:?}");
        assert!((actual.y - expected.y).abs() < POINT_EPSILON, "y: {actual:?} != {expected:?}");
    }

    #[test]
    fn zoom_keeps_screen_anchor_stable() {
        let mut session = prepared_session();
        let anchor = CanvasPoint::new(620.0, 410.0);
        let before = snapshot(&session).screen_to_content(anchor);

        session.apply(CanvasViewportAction::ZoomBy { factor: 1.25, screen_anchor: anchor });

        let after = snapshot(&session).screen_to_content(anchor);
        assert_point_close(after, before);
    }

    #[test]
    fn actions_without_a_snapshot_do_not_create_a_viewport() {
        let mut session = CanvasViewportSession::default();
        let screen_anchor = CanvasPoint::new(620.0, 410.0);

        session.apply(CanvasViewportAction::PanBy(CanvasPoint::new(20.0, 30.0)));
        session.apply(CanvasViewportAction::ZoomBy { factor: 1.25, screen_anchor });
        session.apply(CanvasViewportAction::SetAxisPosition {
            axis: CanvasAxis::Horizontal,
            position: 50.0,
        });
        session.apply(CanvasViewportAction::Page { axis: CanvasAxis::Vertical, direction: 1.0 });
        session.apply(CanvasViewportAction::ResetView);

        assert!(session.snapshot().is_none());
    }

    #[test]
    fn metric_change_keeps_focus_anchor_at_its_screen_position() {
        let mut session = prepared_session();
        let focus_anchor = CanvasPoint::new(900.0, 600.0);
        session.apply(CanvasViewportAction::PanBy(CanvasPoint::new(80.0, 60.0)));
        let before = snapshot(&session).content_to_screen(focus_anchor);

        session.prepare(
            CanvasContentMetrics {
                content_bounds: Rect::new(-500.0, -400.0, 4_000.0, 4_000.0),
                focus_anchor: Some(focus_anchor),
            },
            viewport(),
            CanvasViewportConfig::for_dpi(1.0),
        );

        let after = snapshot(&session).content_to_screen(focus_anchor);
        assert_point_close(after, before);
    }

    #[test]
    fn metric_change_without_focus_keeps_previous_viewport_center() {
        let mut session = prepared_session();
        session.apply(CanvasViewportAction::PanBy(CanvasPoint::new(80.0, 60.0)));
        let previous_center =
            CanvasPoint::new(viewport().x + viewport().w * 0.5, viewport().y + viewport().h * 0.5);
        let content_at_previous_center = snapshot(&session).screen_to_content(previous_center);

        session.prepare(
            CanvasContentMetrics {
                content_bounds: Rect::new(-500.0, -400.0, 4_000.0, 4_000.0),
                focus_anchor: None,
            },
            viewport(),
            CanvasViewportConfig::for_dpi(1.0),
        );

        let content_at_current_center = snapshot(&session).screen_to_content(previous_center);
        assert_point_close(content_at_current_center, content_at_previous_center);
    }

    #[test]
    fn narrower_viewport_keeps_old_center_content_at_new_center() {
        let mut session = prepared_session();
        let before = snapshot(&session);
        let old_center = CanvasPoint::new(
            before.viewport.x + before.viewport.w * 0.5,
            before.viewport.y + before.viewport.h * 0.5,
        );
        let content_anchor = before.screen_to_content(old_center);

        session.prepare(
            CanvasContentMetrics { content_bounds: before.content_bounds, focus_anchor: None },
            Rect::new(
                before.viewport.x,
                before.viewport.y,
                before.viewport.w - 280.0,
                before.viewport.h,
            ),
            CanvasViewportConfig::for_dpi(1.0),
        );

        let after = snapshot(&session);
        let new_center = CanvasPoint::new(
            after.viewport.x + after.viewport.w * 0.5,
            after.viewport.y + after.viewport.h * 0.5,
        );
        assert_point_close(after.content_to_screen(content_anchor), new_center);
        assert_eq!(after.zoom, before.zoom);
    }

    #[test]
    fn canvas_viewport_sessions_are_independent() {
        let mut first = CanvasViewportSession::default();
        let mut second = CanvasViewportSession::default();
        let metrics = CanvasContentMetrics {
            content_bounds: Rect::new(0.0, 0.0, 2_000.0, 2_000.0),
            focus_anchor: None,
        };
        let config = CanvasViewportConfig::for_dpi(1.0);
        first.prepare(metrics, viewport(), config);
        second.prepare(metrics, viewport(), config);
        first.apply(CanvasViewportAction::ZoomBy {
            factor: 1.25,
            screen_anchor: CanvasPoint::new(620.0, 410.0),
        });

        assert_ne!(
            first.snapshot(),
            second.snapshot(),
            "a zoom in one tab must not modify another tab's canvas session"
        );
    }
}
