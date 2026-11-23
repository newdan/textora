pub(crate) use super::layout::EMPTY_TITLE_PLACEHOLDER;
pub(in crate::mmf::canvas) use super::layout::{HitMap, LayoutConstants, LayoutNode, LayoutTree};
pub(in crate::mmf::canvas) use super::model::Node;
pub(in crate::mmf::canvas) use crate::mindmap_view::MindmapFocus;
pub(in crate::mmf::canvas) use std::borrow::Cow;
pub(in crate::mmf::canvas) use std::ops::Range;
pub(in crate::mmf::canvas) use std::sync::Arc;
pub(in crate::mmf::canvas) use ui::canvas::{CanvasPoint, CanvasViewportSnapshot};
pub(in crate::mmf::canvas) use ui::core::geom::Rect;
pub(in crate::mmf::canvas) use ui::core::paint::DrawList;
pub(in crate::mmf::canvas) use ui::plugin::CanvasDragPreview;
pub(in crate::mmf::canvas) use ui::tapered_path::{
    TAPERED_PATH_FEATHER_PX, TaperedMesh, TaperedPathInput, tessellate_tapered_path,
};
pub(in crate::mmf::canvas) use ui::theme::MindmapRenderTheme;

pub(in crate::mmf::canvas) const CONNECTOR_REFERENCE_WIDTH_DP: f32 = 8.0;
pub(in crate::mmf::canvas) const CONNECTOR_TAIL_WIDTH_DP: f32 = 1.0;
pub(in crate::mmf::canvas) const FIRST_LEVEL_CONNECTOR_HEAD_WIDTH_DP: f32 = 5.0;
pub(in crate::mmf::canvas) const SECOND_LEVEL_CONNECTOR_HEAD_WIDTH_DP: f32 = 3.0;
pub(in crate::mmf::canvas) const THIRD_LEVEL_CONNECTOR_HEAD_WIDTH_DP: f32 = 2.0;
pub(in crate::mmf::canvas) const ZERO_DISTANCE_EPSILON: f32 = 0.01;
pub(in crate::mmf::canvas) const CONNECTOR_MAX_CORNER_RADIUS: f32 = 24.0;
pub(in crate::mmf::canvas) const MIN_CONNECTOR_CORNER_RADIUS: f32 = 0.5;
pub(in crate::mmf::canvas) const CONNECTOR_ARC_SAMPLE_COUNT: usize = 8;
pub(crate) const CARET_WIDTH: f32 = 2.0;
pub(in crate::mmf::canvas) const DRAG_INSERTION_LINE_HEIGHT: f32 = 2.0;
/// 拖拽预览源节点深度查找失败时的回退深度：默认配色中 1.0 缩放档。
pub(in crate::mmf::canvas) const FALLBACK_PREVIEW_DEPTH: u8 = 2;
pub(in crate::mmf::canvas) const PREEDIT_UNDERLINE_HEIGHT: f32 = 1.5;
pub(in crate::mmf::canvas) const PREEDIT_UNDERLINE_OFFSET: f32 = 2.0;
pub(in crate::mmf::canvas) const PLACEHOLDER_ALPHA_MULTIPLIER: f32 = 0.45;
pub(in crate::mmf::canvas) const HOVER_FILL_ALPHA_MULTIPLIER: f32 = 1.15;
/// 分支染色节点背景：分支色以低透明度叠加在画布底色上。
pub(crate) const BRANCH_TINT_FILL_ALPHA: f32 = 0.18;
/// 选中节点高亮填充的透明度，颜色取节点自身 accent。
pub(in crate::mmf::canvas) const SELECTION_FILL_ALPHA: f32 = 0.2;
pub(in crate::mmf::canvas) const DEFAULT_CARD_BORDER_WIDTH: f32 = 1.0;
pub(in crate::mmf::canvas) const HOVERED_CARD_BORDER_WIDTH: f32 = 2.0;
pub(in crate::mmf::canvas) const CONTROL_RING_BORDER_WIDTH: f32 = 2.0;
pub(in crate::mmf::canvas) const CONTROL_CIRCLE_RADIUS_RATIO: f32 = 0.5;
pub(in crate::mmf::canvas) const CONTROL_RING_INSET_DP: f32 = 2.0;
pub(in crate::mmf::canvas) const CONTROL_FONT_SIZE_MULTIPLIER: f32 = 1.0;
pub(in crate::mmf::canvas) const MIN_CONTROL_LABEL_GLYPH_ADVANCE: f32 = 1.0;
pub(in crate::mmf::canvas) const CONTROL_LABEL_SUBPIXEL_PHASE_COUNT: u8 = 4;
pub(in crate::mmf::canvas) const CONTROL_LABEL_SUBPIXEL_STEP: f32 = 0.25;
pub(in crate::mmf::canvas) const EXPANDED_CONTROL_BAR_WIDTH_DP: f32 = 12.0;
pub(in crate::mmf::canvas) const EXPANDED_CONTROL_BAR_HEIGHT_DP: f32 = 2.0;
pub(crate) const CONTROL_HOVER_LIGHTEN_FACTOR: f32 = 0.35;
pub(crate) const CONTROL_HOVER_DARKEN_FACTOR: f32 = 0.75;

/// Mindmap 的纯渲染输入；不拥有或修改任何文档状态。
pub(crate) struct MindmapRenderProjection<'a> {
    pub(crate) focus: MindmapFocus,
    pub(crate) projected_titles: Vec<Cow<'a, str>>,
    pub(crate) preedit_text: &'a str,
    pub(crate) cursor_visible: bool,
    /// `(node_index, projected_title_byte_offset)`。
    pub(crate) caret: Option<(usize, usize)>,
    /// IME 候选窗定位使用的投影 caret，不受 blink 相位影响。
    pub(crate) composition_caret: Option<(usize, usize)>,
    /// `(node_index, projected_title_byte_range)`，用于绘制 IME 预编辑下划线。
    pub(crate) preedit_range: Option<(usize, Range<usize>)>,
    /// 完整源码 DFS 序中每个收起节点的后代数量。
    pub(crate) collapsed_descendant_counts: Vec<Option<usize>>,
    /// 最近一次画布指针位置（屏幕坐标）。
    pub(crate) canvas_pointer: Option<CanvasPoint>,
    /// 画布拖拽的纯视觉预览，不参与布局或命中计算。
    pub(crate) drag_preview: Option<&'a CanvasDragPreview>,
}

impl MindmapRenderProjection<'_> {
    pub(crate) fn projected_title(&self, node_index: usize) -> &str {
        self.projected_titles.get(node_index).map(Cow::as_ref).unwrap_or_default()
    }

    pub(crate) fn selected_node_index(&self) -> Option<usize> {
        match self.focus {
            MindmapFocus::NodeSelected { node_index } => Some(node_index),
            _ => None,
        }
    }

    pub(in crate::mmf::canvas) fn editing_node_index(&self) -> Option<usize> {
        match self.focus {
            MindmapFocus::TitleEditing { node_index, .. }
            | MindmapFocus::TitleTextSelected { node_index, .. } => Some(node_index),
            MindmapFocus::None | MindmapFocus::NodeSelected { .. } => None,
        }
    }

    pub(crate) fn caret(&self) -> Option<(usize, usize)> {
        self.caret
    }

    pub(crate) fn composition_caret(&self) -> Option<(usize, usize)> {
        self.composition_caret
    }
}

pub(crate) struct ConnectorMeshCache {
    pub(in crate::mmf::canvas) zoom_bits: u32,
    pub(in crate::mmf::canvas) meshes_by_layout_node: Vec<Option<Arc<TaperedMesh>>>,
}

impl ConnectorMeshCache {
    pub(crate) fn build(layout: &LayoutTree, constants: &LayoutConstants, zoom: f32) -> Self {
        let meshes_by_layout_node = layout
            .nodes
            .iter()
            .map(|layout_node| {
                if layout_node.depth == 0 {
                    return None;
                }
                let turn_x = layout_node
                    .connector_turn_x
                    .expect("non-root mmap connector must have a turn axis");
                let centerline = connector_centerline(
                    layout_node.connector_from,
                    layout_node.connector_to,
                    turn_x,
                    constants.connector_width,
                );
                tapered_connector_mesh(
                    &centerline,
                    connector_head_width(layout_node.depth, constants.connector_width),
                    connector_tail_width(constants.connector_width),
                    zoom,
                )
            })
            .collect();
        Self { zoom_bits: zoom.to_bits(), meshes_by_layout_node }
    }

    pub(crate) fn matches_zoom(&self, zoom: f32) -> bool {
        self.zoom_bits == zoom.to_bits()
    }

    pub(crate) fn mesh_for_layout_node(
        &self,
        layout_node_index: usize,
    ) -> Option<&Arc<TaperedMesh>> {
        self.meshes_by_layout_node.get(layout_node_index).and_then(Option::as_ref)
    }
}

pub(in crate::mmf::canvas) fn get_node_style(
    node: &Node,
    layout_node: &LayoutNode,
    theme: &MindmapRenderTheme<'_>,
) -> ui::theme::MindmapNodeStyle {
    // 1. Root or depth style
    let mut style = if layout_node.depth == 0 {
        theme.node.root.clone()
    } else if theme.node.depth.is_empty() {
        theme.node.default.clone()
    } else {
        theme.node.depth[(layout_node.depth as usize - 1) % theme.node.depth.len()].clone()
    };

    // 1.5 分支染色：作用于默认外观的 fill/border/text/accent，语义覆盖仍可覆盖之
    if layout_node.depth > 0
        && let Some(branch_color) =
            layout_node.branch_index.and_then(|index| theme.canvas.branch_color(index))
    {
        style.fill = with_alpha(branch_color, BRANCH_TINT_FILL_ALPHA);
        style.border = branch_color;
        style.text = branch_color;
        style.accent = branch_color;
    }

    // 2. Status overrides body (fill, text)
    if let Some(props) = &node.props {
        if let Some(status) = &props.status {
            let status_style = match status.as_str() {
                "todo" => Some(&theme.semantic.status.todo),
                "doing" => Some(&theme.semantic.status.doing),
                "done" => Some(&theme.semantic.status.done),
                "blocked" => Some(&theme.semantic.status.blocked),
                "canceled" => Some(&theme.semantic.status.canceled),
                _ => None,
            };
            if let Some(ss) = status_style {
                style = ss.clone();
            }
        }

        // 3. Priority overrides accent/border
        if let Some(priority) = &props.priority {
            let prio_style = match priority.to_lowercase().as_str() {
                "p0" => Some(&theme.semantic.priority.p0),
                "p1" => Some(&theme.semantic.priority.p1),
                "p2" => Some(&theme.semantic.priority.p2),
                "p3" => Some(&theme.semantic.priority.p3),
                _ => None,
            };
            if let Some(ps) = prio_style {
                style.accent = ps.accent;
                style.border = ps.border;
            }
        }

        // 4. Named color overrides everything
        if let Some(named_style) = props.color.as_ref().and_then(|c| theme.semantic.named.get(c)) {
            style = named_style.clone();
        }
    }

    style
}

/// 渲染可见节点卡片。
#[allow(clippy::too_many_arguments)]
pub fn render_cards(
    dl: &mut DrawList,
    layout: &LayoutTree,
    visible_node_indices: &[usize],
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    nodes: &[&Node],
    preview: Option<&CanvasDragPreview>,
    viewport: CanvasViewportSnapshot,
) {
    render_cards_with_hover(
        dl,
        layout,
        visible_node_indices,
        theme,
        constants,
        nodes,
        preview,
        viewport,
        None,
        None,
        None,
    );
}

pub(in crate::mmf::canvas) fn pointer_hits_node_or_control(
    node_rect: Rect,
    control_bounds: Option<Rect>,
    pointer: Option<CanvasPoint>,
) -> bool {
    pointer.is_some_and(|point| {
        node_rect.contains(point.x, point.y)
            || control_bounds.is_some_and(|bounds| bounds.contains(point.x, point.y))
    })
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mmf::canvas) fn render_cards_with_hover(
    dl: &mut DrawList,
    layout: &LayoutTree,
    visible_node_indices: &[usize],
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    nodes: &[&Node],
    preview: Option<&CanvasDragPreview>,
    viewport: CanvasViewportSnapshot,
    hit_map: Option<&HitMap>,
    pointer: Option<CanvasPoint>,
    editing_node_index: Option<usize>,
) {
    let pointer = pointer.map(|point| viewport.screen_to_content(point));
    for &i in visible_node_indices {
        let ln = &layout.nodes[i];
        let Some(node) = nodes.get(ln.source_node_index) else {
            continue;
        };
        let rect = viewport.content_rect_to_screen(layout_rect(ln));
        let style = get_node_style(node, ln, theme);
        let opacity = source_subtree_opacity(preview, layout_rect(ln), theme);
        let control_bounds = hit_map.and_then(|map| {
            map.controls
                .iter()
                .find(|control| control.source_node_index == ln.source_node_index)
                .map(|control| control.bounds)
        });
        let uses_hover_visuals =
            pointer_hits_node_or_control(layout_rect(ln), control_bounds, pointer)
                || editing_node_index == Some(ln.source_node_index);
        let fill = if uses_hover_visuals {
            with_alpha(style.fill, opacity * HOVER_FILL_ALPHA_MULTIPLIER)
        } else {
            with_alpha(style.fill, opacity)
        };
        // hover/编辑边框跟随节点自身样式色（分支节点即分支色），靠加粗边框与提亮填充反馈
        let border = style.border;
        let border_width =
            if uses_hover_visuals { HOVERED_CARD_BORDER_WIDTH } else { DEFAULT_CARD_BORDER_WIDTH };

        dl.fill_rounded(rect, fill, constants.card_radius * viewport.zoom);
        dl.stroke_rounded(
            rect,
            with_alpha(border, opacity),
            constants.card_radius * viewport.zoom,
            border_width * viewport.zoom,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mmf::canvas) fn render_controls(
    dl: &mut DrawList,
    layout: &LayoutTree,
    hit_map: &HitMap,
    nodes: &[&Node],
    theme: &MindmapRenderTheme<'_>,
    shaper: &mut shaping::Shaper,
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
    hidden_by_preview: bool,
) {
    if hidden_by_preview {
        return;
    }
    let pointer = projection.canvas_pointer.map(|point| viewport.screen_to_content(point));
    let font_size = shaper.font_size() * viewport.zoom * CONTROL_FONT_SIZE_MULTIPLIER;
    for control in &hit_map.controls {
        let Some(node) = nodes.get(control.source_node_index) else {
            continue;
        };
        if !control_is_visible(node, layout, control, pointer) {
            continue;
        }
        let control_rect = viewport.content_rect_to_screen(control.bounds);
        let hovered = control_is_hovered(layout, control, pointer);
        let color = control_color(theme, layout, control, hovered);
        if node.props.as_ref().is_some_and(|props| props.collapsed) {
            let label = projection
                .collapsed_descendant_counts
                .get(control.source_node_index)
                .copied()
                .flatten()
                .map(|count| count.to_string())
                .unwrap_or_default();
            render_collapsed_control(
                dl,
                control_rect,
                &label,
                font_size,
                theme,
                color,
                shaper,
                viewport,
            );
        } else {
            render_expanded_control(dl, control_rect, theme, color, viewport);
        }
    }
}

pub(in crate::mmf::canvas) fn control_is_hovered(
    layout: &LayoutTree,
    control: &super::layout::ControlHitGeometry,
    pointer: Option<CanvasPoint>,
) -> bool {
    let node_rect = layout_node_for_source(layout, control.source_node_index)
        .map(layout_rect)
        .unwrap_or(Rect::ZERO);
    pointer_hits_node_or_control(node_rect, Some(control.bounds), pointer)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mmf::canvas) fn render_collapsed_control(
    dl: &mut DrawList,
    control_rect: Rect,
    label: &str,
    font_size: f32,
    theme: &MindmapRenderTheme<'_>,
    color: [f32; 4],
    shaper: &mut shaping::Shaper,
    viewport: CanvasViewportSnapshot,
) {
    let label_placement = centered_control_label_position(control_rect, label, font_size, shaper);
    render_control_chrome(dl, control_rect, theme, color, viewport);
    dl.clip(control_rect, |clipped| {
        clipped.text_shaped(
            label_placement.text_x,
            label_placement.baseline,
            font_size,
            color,
            label,
            shaper,
        );
    });
}

pub(in crate::mmf::canvas) fn render_expanded_control(
    dl: &mut DrawList,
    control_rect: Rect,
    theme: &MindmapRenderTheme<'_>,
    color: [f32; 4],
    viewport: CanvasViewportSnapshot,
) {
    render_control_chrome(dl, control_rect, theme, color, viewport);
    let bar_width = (EXPANDED_CONTROL_BAR_WIDTH_DP * viewport.zoom).min(control_rect.w);
    let bar_height = (EXPANDED_CONTROL_BAR_HEIGHT_DP * viewport.zoom).min(control_rect.h);
    dl.fill(
        Rect::new(
            control_rect.x + (control_rect.w - bar_width) * 0.5,
            control_rect.y + (control_rect.h - bar_height) * 0.5,
            bar_width,
            bar_height,
        ),
        color,
    );
}

pub(in crate::mmf::canvas) fn is_dark_background(background: [f32; 4]) -> bool {
    let [r, g, b, _] = background;
    // Rec. 601 luma
    0.299 * r + 0.587 * g + 0.114 * b < 0.5
}

pub(in crate::mmf::canvas) fn control_color(
    theme: &MindmapRenderTheme<'_>,
    layout: &LayoutTree,
    control: &super::layout::ControlHitGeometry,
    hovered: bool,
) -> [f32; 4] {
    let branch_color =
        layout_node_for_source(layout, control.source_node_index).and_then(|layout_node| {
            layout_node.branch_index.and_then(|index| theme.canvas.branch_color(index))
        });
    let base_color = branch_color.unwrap_or(theme.canvas.connector);
    if !hovered {
        return base_color;
    }
    match branch_color {
        Some(color) => {
            if is_dark_background(theme.canvas.background) {
                lighten_color(color, CONTROL_HOVER_LIGHTEN_FACTOR)
            } else {
                darken_color(color, CONTROL_HOVER_DARKEN_FACTOR)
            }
        }
        None => theme.canvas.connector_hover,
    }
}

pub(crate) fn lighten_color(c: [f32; 4], factor: f32) -> [f32; 4] {
    [c[0] + (1.0 - c[0]) * factor, c[1] + (1.0 - c[1]) * factor, c[2] + (1.0 - c[2]) * factor, c[3]]
}

pub(crate) fn darken_color(c: [f32; 4], factor: f32) -> [f32; 4] {
    [c[0] * factor, c[1] * factor, c[2] * factor, c[3]]
}

pub(in crate::mmf::canvas) fn render_control_chrome(
    dl: &mut DrawList,
    control_rect: Rect,
    theme: &MindmapRenderTheme<'_>,
    color: [f32; 4],
    viewport: CanvasViewportSnapshot,
) {
    let control_radius = control_rect.w.min(control_rect.h) * CONTROL_CIRCLE_RADIUS_RATIO;
    dl.fill_rounded(control_rect, theme.canvas.background, control_radius);
    let ring_inset = CONTROL_RING_INSET_DP * viewport.zoom;
    let ring_rect = control_rect.shrink(ring_inset, ring_inset, ring_inset, ring_inset);
    dl.stroke_rounded(
        ring_rect,
        color,
        ring_rect.w.min(ring_rect.h) * CONTROL_CIRCLE_RADIUS_RATIO,
        CONTROL_RING_BORDER_WIDTH * viewport.zoom,
    );
}

pub(in crate::mmf::canvas) struct ControlLabelPlacement {
    pub(in crate::mmf::canvas) text_x: f32,
    pub(in crate::mmf::canvas) baseline: f32,
}

pub(in crate::mmf::canvas) struct ControlLabelPhasePlacement {
    pub(in crate::mmf::canvas) text_x: f32,
    pub(in crate::mmf::canvas) visual_center_y: f32,
    pub(in crate::mmf::canvas) horizontal_error: f32,
}

pub(in crate::mmf::canvas) fn centered_control_label_position(
    control_rect: Rect,
    label: &str,
    font_size: f32,
    shaper: &mut shaping::Shaper,
) -> ControlLabelPlacement {
    let original_font_size = shaper.font_size();
    shaper.set_font_size(font_size);
    let shaped = shaper.shape(label).ok();
    shaper.set_font_size(original_font_size);

    if let Some(shaped) = shaped {
        let target = CanvasPoint::new(
            control_rect.x + control_rect.w * 0.5,
            control_rect.y + control_rect.h * 0.5,
        );
        if let Some(best_position) =
            best_control_label_phase_position(&shaped, font_size, target.x, shaper)
        {
            let baseline = (target.y - best_position.visual_center_y).round();
            return ControlLabelPlacement { text_x: best_position.text_x, baseline };
        }
    }

    let label_width = measure_text(label, shaper) * (font_size / shaper.font_size());
    ControlLabelPlacement {
        text_x: control_rect.x + (control_rect.w - label_width) * 0.5,
        baseline: control_rect.y + control_rect.h * 0.5 + font_size * 0.5,
    }
}

pub(in crate::mmf::canvas) fn best_control_label_phase_position(
    shaped: &shaping::ShapedRun,
    font_size: f32,
    target_x: f32,
    shaper: &mut shaping::Shaper,
) -> Option<ControlLabelPhasePlacement> {
    let mut best_position: Option<ControlLabelPhasePlacement> = None;
    for phase in 0..CONTROL_LABEL_SUBPIXEL_PHASE_COUNT {
        let candidate_x = phase as f32 * CONTROL_LABEL_SUBPIXEL_STEP;
        let Some(relative_center) =
            control_label_visual_center(shaped, font_size, candidate_x, 0.0, shaper)
        else {
            continue;
        };
        let integer_translation = (target_x - relative_center.x).round();
        let visual_center_x = relative_center.x + integer_translation;
        let horizontal_error = (visual_center_x - target_x).abs();
        if best_position.as_ref().is_some_and(|best| best.horizontal_error <= horizontal_error) {
            continue;
        }
        best_position = Some(ControlLabelPhasePlacement {
            text_x: candidate_x + integer_translation,
            visual_center_y: relative_center.y,
            horizontal_error,
        });
    }
    best_position
}

pub(in crate::mmf::canvas) fn control_label_visual_center(
    shaped: &shaping::ShapedRun,
    font_size: f32,
    origin_x: f32,
    baseline: f32,
    shaper: &mut shaping::Shaper,
) -> Option<CanvasPoint> {
    let mut weighted_x = 0.0;
    let mut weighted_y = 0.0;
    let mut total_weight = 0.0;
    let mut x_cursor = origin_x;

    for cluster in &shaped.clusters {
        let (_, subpixel_phase) = render::split_subpixel(x_cursor);
        let subpixel_x = subpixel_phase as f32 * CONTROL_LABEL_SUBPIXEL_STEP;
        if let Some(bitmap) = shaper.rasterize_glyph(
            cluster.font_id,
            cluster.glyph_id as u16,
            font_size,
            (subpixel_x, 0.0),
        ) && bitmap.width > 0
            && bitmap.height > 0
        {
            let glyph_left = (x_cursor + bitmap.left as f32).round();
            let glyph_top = (baseline - bitmap.top as f32).round();
            for (pixel_index, alpha) in bitmap.data.iter().copied().enumerate() {
                let weight = alpha as f32;
                let pixel_x = (pixel_index as u32 % bitmap.width) as f32 + 0.5;
                let pixel_y = (pixel_index as u32 / bitmap.width) as f32 + 0.5;
                weighted_x += (glyph_left + pixel_x) * weight;
                weighted_y += (glyph_top + pixel_y) * weight;
                total_weight += weight;
            }
        }
        x_cursor += cluster.advance.max(MIN_CONTROL_LABEL_GLYPH_ADVANCE);
    }

    (total_weight > 0.0)
        .then(|| CanvasPoint::new(weighted_x / total_weight, weighted_y / total_weight))
}

pub(in crate::mmf::canvas) fn control_is_visible(
    node: &Node,
    layout: &LayoutTree,
    control: &super::layout::ControlHitGeometry,
    pointer: Option<CanvasPoint>,
) -> bool {
    if node.props.as_ref().is_some_and(|props| props.collapsed) {
        return true;
    }
    let Some(node_rect) =
        layout_node_for_source(layout, control.source_node_index).map(layout_rect)
    else {
        return false;
    };
    pointer_hits_node_or_control(node_rect, Some(control.bounds), pointer)
}

/// 渲染垂直范围与布局视口相交的连接线。
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_connectors(
    dl: &mut DrawList,
    layout: &LayoutTree,
    layout_viewport: Rect,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    preview: Option<&CanvasDragPreview>,
    viewport: CanvasViewportSnapshot,
    connector_mesh_cache: Option<&ConnectorMeshCache>,
) {
    for (layout_node_index, layout_node) in layout.nodes.iter().enumerate() {
        if layout_node.depth == 0 || !connector_intersects_viewport(layout_node, layout_viewport) {
            continue;
        }
        let child_rect = layout_rect(layout_node);
        let opacity = preview
            .filter(|preview| {
                child_rect != preview.source_rect && is_source_subtree_node(preview, child_rect)
            })
            .map(|_| theme.geometry.drag_source_alpha)
            .unwrap_or(1.0);
        let connector_color = layout_node
            .branch_index
            .and_then(|index| theme.canvas.branch_color(index))
            .unwrap_or(theme.canvas.connector);
        let color = with_alpha(connector_color, opacity);
        if let Some(mesh) =
            connector_mesh_cache.and_then(|cache| cache.mesh_for_layout_node(layout_node_index))
        {
            let translation = viewport.content_to_screen(CanvasPoint::ZERO);
            dl.tapered_mesh(Arc::clone(mesh), [translation.x, translation.y], color);
        } else {
            draw_connector(dl, layout_node, color, constants.connector_width, viewport);
        }
    }
}

#[path = "canvas_overlays.rs"]
mod canvas_overlays;
#[allow(unused_imports)]
pub(in crate::mmf::canvas) use canvas_overlays::*;
#[allow(unused_imports)]
pub(crate) use canvas_overlays::{
    node_font_size, render, render_cards_and_connectors, render_text,
};

#[cfg(test)]
#[path = "canvas_tests.rs"]
mod tests;
