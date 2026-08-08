//! ScrollbarWidget — 滚动条绘制 + 鼠标交互转译。
//! viewport_height / total_display_rows / scroll_top 由 app 通过 set_input 注入。

use crate::canvas::CanvasAxis;
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityOrientation, AccessibilityRole, Event, EventCtx, LayoutCtx,
    MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use winit::window::CursorIcon;
// ── 设计常量（从旧 scrollbar.rs 合并）──────────────────────────────

/// Hit-test 总宽度（鼠标拾取区域，始终不变）。
pub const SCROLLBAR_RESERVE_PX: f32 = 14.0;

/// Idle 状态下 thumb 的交叉轴视觉尺寸。
pub const SCROLLBAR_THUMB_CROSS_EXTENT_IDLE: f32 = 4.0;

/// Active（hover/drag）状态下 thumb 的交叉轴视觉尺寸。
pub const SCROLLBAR_THUMB_CROSS_EXTENT_ACTIVE: f32 = 14.0;

/// Thumb 沿主轴的最小视觉尺寸。
const MIN_THUMB_PRIMARY_EXTENT_PX: f32 = 25.0;

// ── Px 形态 Layout ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ScrollbarLayoutPx {
    pub thumb_rect: Rect,
    pub show_thumb: bool,
    pub max_scroll: f64,
}

pub fn compute_layout_px(
    bar_rect: Rect,
    dpi: f32,
    viewport_height: f64,
    total_display_rows: usize,
    scroll_top: f64,
    active: bool,
) -> ScrollbarLayoutPx {
    compute_axis_layout_px(
        bar_rect,
        dpi,
        CanvasAxis::Vertical,
        viewport_height,
        total_display_rows,
        scroll_top,
        active,
    )
}

pub fn compute_axis_layout_px(
    bar_rect: Rect,
    dpi: f32,
    axis: CanvasAxis,
    viewport_primary_extent: f64,
    total_primary_extent: usize,
    scroll_primary: f64,
    active: bool,
) -> ScrollbarLayoutPx {
    let min_thumb_primary_extent = MIN_THUMB_PRIMARY_EXTENT_PX * dpi;
    let total = total_primary_extent.max(1) as f64;
    let visible = viewport_primary_extent.max(1.0);
    let max_scroll = (total - visible).max(0.0);
    let show_thumb = total > visible;

    let thumb_ratio = (visible / total).min(1.0) as f32;
    let primary_extent = match axis {
        CanvasAxis::Horizontal => bar_rect.w,
        CanvasAxis::Vertical => bar_rect.h,
    };
    let cross_extent = match axis {
        CanvasAxis::Horizontal => bar_rect.h,
        CanvasAxis::Vertical => bar_rect.w,
    };
    let thumb_primary_extent =
        (primary_extent * thumb_ratio).max(min_thumb_primary_extent).min(primary_extent);

    let scroll_ratio =
        if max_scroll > 0.0 { (scroll_primary / max_scroll).clamp(0.0, 1.0) as f32 } else { 0.0 };

    let thumb_primary_origin = scroll_ratio * (primary_extent - thumb_primary_extent);
    let thumb_cross_extent = if active {
        SCROLLBAR_THUMB_CROSS_EXTENT_ACTIVE * dpi
    } else {
        SCROLLBAR_THUMB_CROSS_EXTENT_IDLE * dpi
    };
    let thumb_cross_origin = (cross_extent - thumb_cross_extent).max(0.0);
    let thumb_rect = match axis {
        CanvasAxis::Horizontal => Rect::new(
            thumb_primary_origin,
            thumb_cross_origin,
            thumb_primary_extent,
            thumb_cross_extent,
        ),
        CanvasAxis::Vertical => Rect::new(
            thumb_cross_origin,
            thumb_primary_origin,
            thumb_cross_extent,
            thumb_primary_extent,
        ),
    };

    ScrollbarLayoutPx { thumb_rect, show_thumb, max_scroll }
}
use std::any::Any;

// ── State（内部持有）────────────────────────────────────────────────

#[derive(Clone)]
enum ScrollbarInteraction {
    Idle,
    Hovered,
    Dragging { drag_start_pointer_primary: f32, drag_start_thumb_primary: f32 },
}

#[derive(Clone)]
pub struct ScrollbarState {
    interaction: ScrollbarInteraction,
}

impl Default for ScrollbarState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollbarState {
    pub fn new() -> Self {
        Self { interaction: ScrollbarInteraction::Idle }
    }

    fn is_dragging(&self) -> bool {
        matches!(self.interaction, ScrollbarInteraction::Dragging { .. })
    }

    fn is_hovered(&self) -> bool {
        !matches!(self.interaction, ScrollbarInteraction::Idle)
    }

    fn set_hovered(&mut self, hovered: bool) {
        if self.is_dragging() {
            return;
        }
        self.interaction =
            if hovered { ScrollbarInteraction::Hovered } else { ScrollbarInteraction::Idle };
    }

    fn begin_drag(&mut self, drag_start_pointer_primary: f32, drag_start_thumb_primary: f32) {
        self.interaction =
            ScrollbarInteraction::Dragging { drag_start_pointer_primary, drag_start_thumb_primary };
    }

    fn drag_start(&self) -> Option<(f32, f32)> {
        match self.interaction {
            ScrollbarInteraction::Dragging {
                drag_start_pointer_primary,
                drag_start_thumb_primary,
            } => Some((drag_start_pointer_primary, drag_start_thumb_primary)),
            ScrollbarInteraction::Idle | ScrollbarInteraction::Hovered => None,
        }
    }

    fn end_drag(&mut self) -> bool {
        if !self.is_dragging() {
            return false;
        }
        self.interaction = ScrollbarInteraction::Hovered;
        true
    }

    fn cancel_interaction(&mut self) -> bool {
        if matches!(self.interaction, ScrollbarInteraction::Idle) {
            return false;
        }
        self.interaction = ScrollbarInteraction::Idle;
        true
    }
}

// ── Action ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollbarAction {
    /// 点击轨道上方空白区域 → 向上翻页
    PageUp,
    /// 点击轨道下方空白区域 → 向下翻页
    PageDown,
    /// 开始拖拽 thumb
    StartDrag,
    /// 拖拽中，新的 scroll_top 值
    DragTo(f64),
    /// 结束拖拽
    EndDrag,
    /// hover 状态变化
    HoverChanged(bool),
}

// ── Widget ─────────────────────────────────────────────────────────

/// 滚动条每帧输入（Copy，方便按值传递）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarInput {
    pub viewport_height_px: f64,
    pub total_display_rows: usize,
    pub scroll_top_rows: f64,
}

impl Default for ScrollbarInput {
    fn default() -> Self {
        Self { viewport_height_px: 0.0, total_display_rows: 0, scroll_top_rows: 0.0 }
    }
}

pub struct ScrollbarWidget {
    rect: Rect,
    axis: CanvasAxis,
    state: ScrollbarState,
    /// 由 app 每帧注入
    pub input: ScrollbarInput,
    accessibility_id: Option<AccessibilityId>,
    accessibility_label: Option<String>,
}

impl Default for ScrollbarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollbarWidget {
    pub fn new() -> Self {
        Self::vertical()
    }

    pub fn vertical() -> Self {
        Self {
            rect: Rect::ZERO,
            axis: CanvasAxis::Vertical,
            state: ScrollbarState::new(),
            input: ScrollbarInput::default(),
            accessibility_id: None,
            accessibility_label: None,
        }
    }

    pub fn horizontal() -> Self {
        Self {
            rect: Rect::ZERO,
            axis: CanvasAxis::Horizontal,
            state: ScrollbarState::new(),
            input: ScrollbarInput::default(),
            accessibility_id: None,
            accessibility_label: None,
        }
    }

    /// App 每帧调用，注入滚动信息。
    /// 非有限 scroll_top_rows 会被规范化为 0.0。
    pub fn set_input(&mut self, mut input: ScrollbarInput) {
        if !input.scroll_top_rows.is_finite() {
            input.scroll_top_rows = 0.0;
        }
        self.input = input;
    }

    pub fn is_dragging(&self) -> bool {
        self.state.is_dragging()
    }

    pub fn set_accessibility_id(&mut self, id: Option<AccessibilityId>) {
        self.accessibility_id = id;
    }

    pub fn set_accessibility_label(&mut self, label: Option<String>) {
        self.accessibility_label = label;
    }

    fn maximum_scroll(&self) -> f64 {
        let total = self.input.total_display_rows.max(1) as f64;
        let visible = self.input.viewport_height_px.max(1.0);
        (total - visible).max(0.0)
    }

    /// 当前是否处于 active 状态（hover 或 drag）。
    fn active(&self) -> bool {
        self.state.is_hovered()
    }

    fn pointer_in_bar(&self, px: f32, py: f32) -> bool {
        self.rect.w > 0.0 && self.rect.h > 0.0 && self.rect.contains(px, py)
    }

    fn pointer_primary(&self, px: f32, py: f32) -> f32 {
        match self.axis {
            CanvasAxis::Horizontal => px,
            CanvasAxis::Vertical => py,
        }
    }

    fn thumb_primary_origin(&self, thumb_rect: Rect) -> f32 {
        match self.axis {
            CanvasAxis::Horizontal => thumb_rect.x,
            CanvasAxis::Vertical => thumb_rect.y,
        }
    }

    fn thumb_primary_extent(&self, thumb_rect: Rect) -> f32 {
        match self.axis {
            CanvasAxis::Horizontal => thumb_rect.w,
            CanvasAxis::Vertical => thumb_rect.h,
        }
    }

    fn primary_extent(&self) -> f32 {
        match self.axis {
            CanvasAxis::Horizontal => self.rect.w,
            CanvasAxis::Vertical => self.rect.h,
        }
    }

    /// 按需计算 px layout（每次调用时以当前 rect 和 state 重新计算）。
    fn compute_layout(&self, dpi: f32) -> ScrollbarLayoutPx {
        compute_axis_layout_px(
            self.rect,
            dpi,
            self.axis,
            self.input.viewport_height_px,
            self.input.total_display_rows,
            self.input.scroll_top_rows,
            self.active(),
        )
    }
}

impl Widget for ScrollbarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let active = self.active();
        let layout = self.compute_layout(ctx.dpi);

        // VSCode 风格：idle 时只画细 thumb（无 track），active 时画 track + 粗 thumb。
        if active {
            // 轨道背景
            let mut track_color = ctx.theme.editor.scrollbar_track;
            track_color[3] *= 1.0;
            ctx.list.fill(Rect::new(0.0, 0.0, self.rect.w, self.rect.h), track_color);
        }

        if layout.show_thumb {
            let mut thumb_color = ctx.theme.editor.scrollbar_thumb;
            thumb_color[3] *= if active { 1.0 } else { 0.6 };
            ctx.list.fill(layout.thumb_rect, thumb_color);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.pointer_in_bar(px, py)
    }

    fn is_capturing(&self) -> bool {
        self.state.is_dragging()
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let id = self.accessibility_id?;
        let maximum_scroll = self.maximum_scroll();
        if maximum_scroll <= 0.0 || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let orientation = match self.axis {
            CanvasAxis::Horizontal => AccessibilityOrientation::Horizontal,
            CanvasAxis::Vertical => AccessibilityOrientation::Vertical,
        };
        let fallback_name = match self.axis {
            CanvasAxis::Horizontal => "水平滚动条",
            CanvasAxis::Vertical => "垂直滚动条",
        };
        Some(
            AccessibilityNode::new(id, AccessibilityRole::ScrollBar, ctx.screen_bounds(self.rect))
                .with_name(self.accessibility_label.as_deref().unwrap_or(fallback_name))
                .with_numeric_value(
                    self.input.scroll_top_rows.clamp(0.0, maximum_scroll),
                    0.0,
                    maximum_scroll,
                )
                .with_orientation(orientation)
                .with_action(AccessibilityAction::Increment)
                .with_action(AccessibilityAction::Decrement),
        )
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if Some(request.target) != self.accessibility_id || self.maximum_scroll() <= 0.0 {
            return None;
        }
        match request.action {
            AccessibilityAction::Increment => {
                Some(WidgetAction::Scrollbar(ScrollbarAction::PageDown))
            }
            AccessibilityAction::Decrement => {
                Some(WidgetAction::Scrollbar(ScrollbarAction::PageUp))
            }
            _ => None,
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            Event::MouseMove { px, py } => {
                let was_hovered = self.state.is_hovered();
                let in_bar = self.pointer_in_bar(*px, *py);
                self.state.set_hovered(in_bar);

                // Always set cursor hint when over the scrollbar area
                if in_bar || self.state.is_dragging() {
                    ctx.cursor_hint = Some(CursorIcon::Default);
                }

                if let Some((drag_start_pointer_primary, drag_start_thumb_primary)) =
                    self.state.drag_start()
                {
                    let layout = self.compute_layout(ctx.dpi);
                    if layout.show_thumb && layout.max_scroll > 0.0 {
                        let track_range =
                            self.primary_extent() - self.thumb_primary_extent(layout.thumb_rect);
                        if track_range > 0.0 {
                            let pointer_primary = self.pointer_primary(*px, *py);
                            let pointer_delta = pointer_primary - drag_start_pointer_primary;
                            let raw_thumb_primary = drag_start_thumb_primary + pointer_delta;
                            let clamped_thumb_primary = raw_thumb_primary.clamp(0.0, track_range);
                            let ratio = clamped_thumb_primary / track_range;
                            let new_scroll =
                                (ratio as f64 * layout.max_scroll).clamp(0.0, layout.max_scroll);
                            return Some(WidgetAction::Scrollbar(ScrollbarAction::DragTo(
                                new_scroll,
                            )));
                        }
                    }
                } else if was_hovered != self.state.is_hovered() {
                    return Some(WidgetAction::Scrollbar(ScrollbarAction::HoverChanged(
                        self.state.is_hovered(),
                    )));
                }
                None
            }
            Event::PointerLeave => {
                if self.state.is_dragging() || !self.state.is_hovered() {
                    return None;
                }
                self.state.set_hovered(false);
                Some(WidgetAction::Scrollbar(ScrollbarAction::HoverChanged(false)))
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                // Only handle clicks within the scrollbar's bounds
                if !self.pointer_in_bar(*px, *py) {
                    return None;
                }
                let layout = self.compute_layout(ctx.dpi);
                if !layout.show_thumb {
                    return None;
                }
                // Hit-test thumb with完整交叉轴范围，而非视觉上的细 thumb。
                // This matches VSCode behavior: easy-to-hit even when thumb appears thin.
                let pointer_primary = self.pointer_primary(*px, *py);
                let thumb_primary_origin = self.thumb_primary_origin(layout.thumb_rect);
                let thumb_primary_extent = self.thumb_primary_extent(layout.thumb_rect);
                if pointer_primary >= thumb_primary_origin
                    && pointer_primary < thumb_primary_origin + thumb_primary_extent
                {
                    self.state.begin_drag(pointer_primary, thumb_primary_origin);
                    Some(WidgetAction::Scrollbar(ScrollbarAction::StartDrag))
                } else if pointer_primary < thumb_primary_origin {
                    Some(WidgetAction::Scrollbar(ScrollbarAction::PageUp))
                } else {
                    Some(WidgetAction::Scrollbar(ScrollbarAction::PageDown))
                }
            }
            Event::MouseUp { .. } => {
                if self.state.end_drag() {
                    Some(WidgetAction::Scrollbar(ScrollbarAction::EndDrag))
                } else {
                    None
                }
            }
            Event::InteractionCancel => {
                self.state.cancel_interaction().then_some(WidgetAction::Consumed)
            }
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::CanvasAxis;
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::theme::Theme;

    fn test_theme() -> Theme {
        let mut t = crate::theme::test_theme();
        t.editor.scrollbar_track = [0.2, 0.2, 0.2, 0.3];
        t.editor.scrollbar_thumb = [0.5, 0.5, 0.5, 0.6];
        t
    }
    fn make_widget(rect: Rect, dpi: f32, vp_h: f64, total: usize, scroll: f64) -> ScrollbarWidget {
        let mut w = ScrollbarWidget::new();
        w.set_input(ScrollbarInput {
            viewport_height_px: vp_h,
            total_display_rows: total,
            scroll_top_rows: scroll,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi };
        w.set_rect(rect, &mut lc);
        w
    }

    #[test]
    fn accessibility_exposes_scrollbar_range_and_page_actions() {
        let accessibility_id = crate::core::AccessibilityId(70);
        let mut widget = make_widget(Rect::new(0.0, 0.0, 14.0, 200.0), 1.0, 100.0, 400, 50.0);
        widget.set_accessibility_id(Some(accessibility_id));
        widget.set_accessibility_label(Some("文档垂直滚动".into()));
        let node = widget
            .accessibility_node(&crate::core::AccessibilityContext::new(10.0, 20.0))
            .expect("overflowing scrollbar should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::ScrollBar);
        assert_eq!(node.name.as_deref(), Some("文档垂直滚动"));
        assert_eq!(node.numeric_value, Some(50.0));
        assert_eq!(node.numeric_minimum, Some(0.0));
        assert_eq!(node.numeric_maximum, Some(300.0));
        assert_eq!(node.orientation, Some(crate::core::AccessibilityOrientation::Vertical));
        assert_eq!(node.bounds, Rect::new(10.0, 20.0, 14.0, 200.0));
        assert_eq!(
            widget.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                accessibility_id,
                crate::core::AccessibilityAction::Increment,
            )),
            Some(WidgetAction::Scrollbar(ScrollbarAction::PageDown))
        );

        widget.set_input(ScrollbarInput {
            viewport_height_px: 400.0,
            total_display_rows: 100,
            scroll_top_rows: 0.0,
        });
        assert!(widget.accessibility_node(&crate::core::AccessibilityContext::default()).is_none());
    }

    // ── ScrollbarInput ──

    #[test]
    fn set_input_replaces_all_scrollbar_fields() {
        let mut widget = ScrollbarWidget::new();
        widget.set_input(ScrollbarInput {
            viewport_height_px: 50.0,
            total_display_rows: 100,
            scroll_top_rows: 25.0,
        });
        widget.set_input(ScrollbarInput {
            viewport_height_px: 20.0,
            total_display_rows: 40,
            scroll_top_rows: 5.0,
        });
        assert_eq!(
            widget.input,
            ScrollbarInput {
                viewport_height_px: 20.0,
                total_display_rows: 40,
                scroll_top_rows: 5.0,
            }
        );
    }

    #[test]
    fn non_finite_scroll_top_is_normalized_to_zero() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut widget = ScrollbarWidget::new();
            widget.set_input(ScrollbarInput {
                viewport_height_px: 20.0,
                total_display_rows: 40,
                scroll_top_rows: value,
            });
            assert_eq!(widget.input.scroll_top_rows, 0.0);
        }
    }

    // ── paint ──

    #[test]
    fn paint_zero_rect_skips() {
        let w = make_widget(Rect::ZERO, 1.0, 100.0, 200, 0.0);
        let t = test_theme();
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
        assert!(dl.cmds.is_empty());
    }

    #[test]
    fn paint_idle_emits_thumb_only_no_track() {
        // 阶段 D：非 hover/drag 时 track 隐藏，仅留淡 thumb
        let w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
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

        let fills: Vec<_> =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).collect();
        assert_eq!(fills.len(), 1, "idle 状态只有 thumb");
    }

    #[test]
    fn paint_hover_emits_track_and_thumb() {
        // 阶段 D：hover 时 track 显示
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        w.on_event(&Event::MouseMove { px: 6.0, py: 368.0 }, &mut ec);

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

        let fills: Vec<_> =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).collect();
        assert_eq!(fills.len(), 2, "hover 时 track + thumb");
    }

    #[test]
    fn paint_small_file_no_thumb() {
        // 短文件无 thumb：idle 状态下不发任何 fill
        let w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 200.0, 100, 0.0);
        let t = test_theme();
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

        let fills: Vec<_> =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).collect();
        assert_eq!(fills.len(), 0, "短文件 idle 无 fill");
    }

    // ── on_event: hover ──

    #[test]
    fn hover_enter_emits_hover_changed() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let a = w.on_event(&Event::MouseMove { px: 6.0, py: 368.0 }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::HoverChanged(true)));
    }

    #[test]
    fn hover_leave_emits_hover_changed_false() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        // First hover in
        w.on_event(&Event::MouseMove { px: 6.0, py: 368.0 }, &mut ec);
        // Then hover out
        let a = w.on_event(&Event::MouseMove { px: -1188.0, py: -32.0 }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::HoverChanged(false)));
    }

    #[test]
    fn lifecycle_leave_preserves_drag_and_cancel_is_idempotent() {
        let mut widget = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let theme = test_theme();
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        assert_eq!(
            widget.on_event(&Event::MouseMove { px: 6.0, py: 68.0 }, &mut event_ctx),
            Some(WidgetAction::Scrollbar(ScrollbarAction::HoverChanged(true)))
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 6.0, py: 68.0, button: MouseButton::Left },
                &mut event_ctx,
            ),
            Some(WidgetAction::Scrollbar(ScrollbarAction::StartDrag))
        );
        assert_eq!(widget.on_event(&Event::PointerLeave, &mut event_ctx), None);
        assert!(widget.is_capturing());

        assert_eq!(
            widget.on_event(&Event::InteractionCancel, &mut event_ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!widget.is_capturing());
        assert!(!widget.active());
        assert_eq!(widget.on_event(&Event::InteractionCancel, &mut event_ctx), None);
        assert_eq!(
            widget.on_event(&Event::MouseMove { px: 6.0, py: 468.0 }, &mut event_ctx),
            Some(WidgetAction::Scrollbar(ScrollbarAction::HoverChanged(true)))
        );
    }

    #[test]
    fn no_hover_change_on_same_state() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        // First hover in
        w.on_event(&Event::MouseMove { px: 6.0, py: 368.0 }, &mut ec);
        // Same position again
        let a = w.on_event(&Event::MouseMove { px: 6.0, py: 368.0 }, &mut ec);
        assert!(a.is_none());
    }

    // ── on_event: click track ──

    #[test]
    fn click_track_above_thumb_returns_page_up() {
        // scroll down so thumb is in middle
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 50.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let a =
            w.on_event(&Event::MouseDown { px: 6.0, py: 2.0, button: MouseButton::Left }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::PageUp));
    }

    #[test]
    fn click_track_below_thumb_returns_page_down() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 400, 0.0);
        // thumb is near top (small ratio), so most of track is "below"
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let a = w
            .on_event(&Event::MouseDown { px: 6.0, py: 668.0, button: MouseButton::Left }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::PageDown));
    }

    #[test]
    fn click_thumb_starts_drag() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        // thumb_y = 32
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let a =
            w.on_event(&Event::MouseDown { px: 6.0, py: 68.0, button: MouseButton::Left }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::StartDrag));
        assert!(w.is_dragging());
    }

    #[test]
    fn click_no_thumb_returns_none() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 200.0, 100, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let a = w
            .on_event(&Event::MouseDown { px: 6.0, py: 368.0, button: MouseButton::Left }, &mut ec);
        assert!(a.is_none());
    }

    // ── on_event: drag ──

    #[test]
    fn drag_produces_drag_to() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        // Start drag
        w.on_event(&Event::MouseDown { px: 6.0, py: 68.0, button: MouseButton::Left }, &mut ec);
        // Move mouse down
        let a = w.on_event(&Event::MouseMove { px: 6.0, py: 468.0 }, &mut ec);
        let a = a.unwrap();
        match a {
            WidgetAction::Scrollbar(ScrollbarAction::DragTo(scroll)) => {
                assert!(scroll > 0.0, "drag down should increase scroll");
            }
            other => panic!("expected DragTo, got {other:?}"),
        }
    }

    #[test]
    fn mouse_up_ends_drag() {
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        w.on_event(&Event::MouseDown { px: 6.0, py: 68.0, button: MouseButton::Left }, &mut ec);
        assert!(w.is_dragging());
        let a =
            w.on_event(&Event::MouseUp { px: 6.0, py: 468.0, button: MouseButton::Left }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::EndDrag));
        assert!(!w.is_dragging());
    }

    #[test]
    fn vertical_mouse_up_outside_track_keeps_hovered_after_drag() {
        let mut widget = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let theme = test_theme();
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        let start = widget
            .on_event(
                &Event::MouseDown { px: 6.0, py: 68.0, button: MouseButton::Left },
                &mut event_ctx,
            )
            .expect("纵向 thumb 内的左键按下必须开始拖动");
        assert_eq!(start, WidgetAction::Scrollbar(ScrollbarAction::StartDrag));

        let drag = widget
            .on_event(&Event::MouseMove { px: 30.0, py: 900.0 }, &mut event_ctx)
            .expect("拖动移出轨道仍必须产生滚动动作");
        assert!(matches!(drag, WidgetAction::Scrollbar(ScrollbarAction::DragTo(_))));

        let end = widget
            .on_event(
                &Event::MouseUp { px: 30.0, py: 900.0, button: MouseButton::Left },
                &mut event_ctx,
            )
            .expect("拖动结束必须产生 EndDrag");
        assert_eq!(end, WidgetAction::Scrollbar(ScrollbarAction::EndDrag));
        assert!(
            widget.state.is_hovered(),
            "纵向拖动移出轨道后抬起必须保持 active，直到下一次 MouseMove 更新 hover"
        );
    }

    // ── hit ──

    #[test]
    fn hit_delegates_to_rect() {
        let w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        assert!(w.hit(6.0, 368.0));
        assert!(!w.hit(-1088.0, 368.0));
    }

    // ── downcast ──

    #[test]
    fn as_any_downcast_works() {
        let w = ScrollbarWidget::new();
        let r: &dyn Widget = &w;
        assert!(r.as_any().downcast_ref::<ScrollbarWidget>().is_some());
    }

    #[test]
    fn as_any_mut_downcast_works() {
        let mut w = ScrollbarWidget::new();
        let r: &mut dyn Widget = &mut w;
        assert!(r.as_any_mut().downcast_mut::<ScrollbarWidget>().is_some());
    }

    // ── DPI ──

    #[test]
    fn dpi_affects_thumb_size() {
        let w1 = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 20000, 0.0);
        let w2 = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 2.0, 100.0, 20000, 0.0);
        assert!(w2.compute_layout(2.0).thumb_rect.h > w1.compute_layout(1.0).thumb_rect.h);
    }
    #[test]
    fn is_capturing_true_during_drag() {
        // 阶段 C：dragging 中 is_capturing()==true，dock 据此优先派事件
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        assert!(!w.is_capturing(), "idle 时不捕获");
        w.on_event(&Event::MouseDown { px: 6.0, py: 68.0, button: MouseButton::Left }, &mut ec);
        assert!(w.is_capturing(), "拖拽中应捕获");
        w.on_event(&Event::MouseUp { px: 6.0, py: 68.0, button: MouseButton::Left }, &mut ec);
        assert!(!w.is_capturing(), "MouseUp 后释放捕获");
    }

    #[test]
    fn full_drag_flow_start_drag_to_end() {
        // Bug 1.4: Full drag flow through scrollbar widget
        let mut w = make_widget(Rect::new(1188.0, 32.0, 12.0, 744.0), 1.0, 100.0, 200, 0.0);
        let t = test_theme();
        let mut ec = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };

        // Phase 1: Start drag by clicking on thumb
        let a =
            w.on_event(&Event::MouseDown { px: 6.0, py: 68.0, button: MouseButton::Left }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::StartDrag));
        assert!(w.is_dragging(), "Should be dragging after StartDrag");

        // Phase 2: Drag moves scroll position
        let a = w.on_event(&Event::MouseMove { px: 6.0, py: 468.0 }, &mut ec);
        let a = a.unwrap();
        match a {
            WidgetAction::Scrollbar(ScrollbarAction::DragTo(scroll)) => {
                assert!(scroll > 0.0, "Drag down should increase scroll, got {}", scroll);
            }
            other => panic!("Expected DragTo, got {:?}", other),
        }

        // Phase 3: End drag
        let a =
            w.on_event(&Event::MouseUp { px: 6.0, py: 468.0, button: MouseButton::Left }, &mut ec);
        let a = a.unwrap();
        assert_eq!(a, WidgetAction::Scrollbar(ScrollbarAction::EndDrag));
        assert!(!w.is_dragging(), "Should not be dragging after EndDrag");
    }

    // ── compute_layout_px 纯函数测试（从旧 scrollbar.rs 合并）──

    #[test]
    fn thumb_proportional_to_ratio() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 200, 0.0, false);
        assert!((lay.thumb_rect.h - 372.0).abs() < 0.5);
        assert!(lay.show_thumb);
    }

    #[test]
    fn min_thumb_height_respected() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 20000, 0.0, false);
        assert!(lay.thumb_rect.h >= 25.0);
    }

    #[test]
    fn thumb_not_exceeding_bar_height() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 20000, 0.0, false);
        assert!(lay.thumb_rect.h <= bar.h);
    }

    #[test]
    fn small_file_no_thumb() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 1.0, 200.0, 100, 0.0, false);
        assert!(!lay.show_thumb);
    }

    #[test]
    fn scroll_middle_maps_thumb_y() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 400.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 400, 150.0, false);
        assert!((lay.thumb_rect.h - 100.0).abs() < 0.5);
        assert!((lay.thumb_rect.y - 150.0).abs() < 0.5);
    }

    #[test]
    fn scroll_bottom_thumb_at_bottom() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 400.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 400, 300.0, false);
        assert!((lay.thumb_rect.y - 300.0).abs() < 0.5);
    }

    #[test]
    fn dpi_scales_min_thumb() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 744.0);
        let lay = compute_layout_px(bar, 2.0, 100.0, 20000, 0.0, false);
        assert!(lay.thumb_rect.h >= 50.0);
    }

    #[test]
    fn empty_total_uses_1() {
        let bar = Rect::new(1188.0, 32.0, 12.0, 400.0);
        let lay = compute_layout_px(bar, 1.0, 100.0, 0, 0.0, false);
        assert!((lay.thumb_rect.h - 400.0).abs() < 0.5);
    }

    #[test]
    fn horizontal_thumb_uses_width_as_primary_extent() {
        let layout = compute_axis_layout_px(
            Rect::new(0.0, 0.0, 400.0, 14.0),
            1.0,
            CanvasAxis::Horizontal,
            100.0,
            400,
            150.0,
            false,
        );

        assert_eq!(layout.thumb_rect.w, 100.0);
        assert_eq!(layout.thumb_rect.x, 150.0);
        assert_eq!(layout.max_scroll, 300.0);
    }

    #[test]
    fn horizontal_drag_emits_start_drag_drag_to_and_end_drag() {
        const EXPECTED_SCROLL_POSITION: f64 = 200.0;
        const SCROLL_POSITION_EPSILON: f64 = 0.001;

        let mut widget = ScrollbarWidget::horizontal();
        widget.set_input(ScrollbarInput {
            viewport_height_px: 100.0,
            total_display_rows: 400,
            scroll_top_rows: 0.0,
        });
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        widget.set_rect(Rect::new(0.0, 0.0, 400.0, 14.0), &mut layout_ctx);
        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        let start = widget
            .on_event(
                &Event::MouseDown { px: 50.0, py: 7.0, button: MouseButton::Left },
                &mut event_ctx,
            )
            .expect("横向 thumb 内的左键按下必须开始拖动");
        assert_eq!(start, WidgetAction::Scrollbar(ScrollbarAction::StartDrag));

        let drag = widget
            .on_event(&Event::MouseMove { px: 250.0, py: 7.0 }, &mut event_ctx)
            .expect("横向拖动必须产生新的滚动位置");
        match drag {
            WidgetAction::Scrollbar(ScrollbarAction::DragTo(value)) => {
                assert!(
                    (value - EXPECTED_SCROLL_POSITION).abs() < SCROLL_POSITION_EPSILON,
                    "横向拖动应滚动至 {EXPECTED_SCROLL_POSITION}，实际为 {value}"
                );
            }
            other => panic!("横向拖动应产生 DragTo，实际为 {other:?}"),
        }

        let end = widget
            .on_event(
                &Event::MouseUp { px: 250.0, py: 7.0, button: MouseButton::Left },
                &mut event_ctx,
            )
            .expect("横向拖动结束必须产生 EndDrag");
        assert_eq!(end, WidgetAction::Scrollbar(ScrollbarAction::EndDrag));
    }
}
