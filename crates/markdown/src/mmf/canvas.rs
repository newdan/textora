pub(crate) use super::layout::EMPTY_TITLE_PLACEHOLDER;
use super::layout::{HitMap, LayoutConstants, LayoutNode, LayoutTree};
use super::model::Node;
use crate::mindmap_view::MindmapFocus;
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use ui::canvas::{CanvasPoint, CanvasViewportSnapshot};
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::plugin::CanvasDragPreview;
use ui::tapered_path::{
    TAPERED_PATH_FEATHER_PX, TaperedMesh, TaperedPathInput, tessellate_tapered_path,
};
use ui::theme::MindmapRenderTheme;

/// 连线 alpha
const CONNECTOR_ALPHA: f32 = 0.3;
const CONNECTOR_REFERENCE_WIDTH_DP: f32 = 8.0;
const CONNECTOR_TAIL_WIDTH_DP: f32 = 1.0;
const FIRST_LEVEL_CONNECTOR_HEAD_WIDTH_DP: f32 = 5.0;
const SECOND_LEVEL_CONNECTOR_HEAD_WIDTH_DP: f32 = 3.0;
const THIRD_LEVEL_CONNECTOR_HEAD_WIDTH_DP: f32 = 2.0;
const ZERO_DISTANCE_EPSILON: f32 = 0.01;
const CONNECTOR_MAX_CORNER_RADIUS: f32 = 24.0;
const MIN_CONNECTOR_CORNER_RADIUS: f32 = 0.5;
const CONNECTOR_ARC_SAMPLE_COUNT: usize = 8;
pub(crate) const CARET_WIDTH: f32 = 2.0;
const DRAG_INSERTION_LINE_HEIGHT: f32 = 2.0;
/// 拖拽预览源节点深度查找失败时的回退深度：默认配色中 1.0 缩放档。
const FALLBACK_PREVIEW_DEPTH: u8 = 2;
const PREEDIT_UNDERLINE_HEIGHT: f32 = 1.5;
const PREEDIT_UNDERLINE_OFFSET: f32 = 2.0;
const PLACEHOLDER_ALPHA_MULTIPLIER: f32 = 0.45;
const HOVER_FILL_ALPHA_MULTIPLIER: f32 = 1.15;
/// 分支染色节点背景：分支色以低透明度叠加在画布底色上。
pub(crate) const BRANCH_TINT_FILL_ALPHA: f32 = 0.18;
/// 选中节点高亮填充的透明度，颜色取节点自身 accent。
const SELECTION_FILL_ALPHA: f32 = 0.2;
const DEFAULT_CARD_BORDER_WIDTH: f32 = 1.0;
const HOVERED_CARD_BORDER_WIDTH: f32 = 2.0;
const CONTROL_RING_BORDER_WIDTH: f32 = 2.0;
const CONTROL_CIRCLE_RADIUS_RATIO: f32 = 0.5;
const CONTROL_RING_INSET_DP: f32 = 2.0;
const CONTROL_FONT_SIZE_MULTIPLIER: f32 = 1.0;
const MIN_CONTROL_LABEL_GLYPH_ADVANCE: f32 = 1.0;
const CONTROL_LABEL_SUBPIXEL_PHASE_COUNT: u8 = 4;
const CONTROL_LABEL_SUBPIXEL_STEP: f32 = 0.25;
const EXPANDED_CONTROL_BAR_WIDTH_DP: f32 = 12.0;
const EXPANDED_CONTROL_BAR_HEIGHT_DP: f32 = 2.0;
pub(crate) const CONTROL_HOVER_LIGHTEN_FACTOR: f32 = 0.35;
pub(crate) const CONTROL_HOVER_DARKEN_FACTOR: f32 = 0.75;

/// Mindmap 的纯渲染输入；不拥有或修改任何文档状态。
pub(crate) struct MindmapRenderProjection<'a> {
    pub(crate) focus: MindmapFocus,
    pub(crate) projected_titles: Vec<Cow<'a, str>>,
    pub(crate) preedit_text: &'a str,
    pub(crate) preedit_cursor: Option<(usize, usize)>,
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

    fn editing_node_index(&self) -> Option<usize> {
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
    zoom_bits: u32,
    meshes_by_layout_node: Vec<Option<Arc<TaperedMesh>>>,
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

fn get_node_style(
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

fn pointer_hits_node_or_control(
    node_rect: Rect,
    control_bounds: Option<Rect>,
    pointer: Option<CanvasPoint>,
) -> bool {
    pointer.is_some_and(|point| {
        node_rect.contains(point.x, point.y)
            || control_bounds.is_some_and(|bounds| bounds.contains(point.x, point.y))
    })
}

fn render_cards_with_hover(
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

fn render_controls(
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

fn control_is_hovered(
    layout: &LayoutTree,
    control: &super::layout::ControlHitGeometry,
    pointer: Option<CanvasPoint>,
) -> bool {
    let node_rect = layout_node_for_source(layout, control.source_node_index)
        .map(layout_rect)
        .unwrap_or(Rect::ZERO);
    pointer_hits_node_or_control(node_rect, Some(control.bounds), pointer)
}

fn render_collapsed_control(
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

fn render_expanded_control(
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

fn is_dark_background(background: [f32; 4]) -> bool {
    let [r, g, b, _] = background;
    // Rec. 601 luma
    0.299 * r + 0.587 * g + 0.114 * b < 0.5
}

fn control_color(
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

fn render_control_chrome(
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

struct ControlLabelPlacement {
    text_x: f32,
    baseline: f32,
}

struct ControlLabelPhasePlacement {
    text_x: f32,
    visual_center_y: f32,
    horizontal_error: f32,
}

fn centered_control_label_position(
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

fn best_control_label_phase_position(
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

fn control_label_visual_center(
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

fn control_is_visible(
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

fn connector_intersects_viewport(layout_node: &LayoutNode, viewport: Rect) -> bool {
    let connector_top = layout_node.connector_from.1.min(layout_node.connector_to.1);
    let connector_bottom = layout_node.connector_from.1.max(layout_node.connector_to.1);
    let viewport_bottom = viewport.y + viewport.h;

    connector_bottom >= viewport.y && connector_top <= viewport_bottom
}

fn draw_connector(
    dl: &mut DrawList,
    ln: &LayoutNode,
    color: [f32; 4],
    width: f32,
    viewport: CanvasViewportSnapshot,
) {
    let from =
        viewport.content_to_screen(CanvasPoint::new(ln.connector_from.0, ln.connector_from.1));
    let to = viewport.content_to_screen(CanvasPoint::new(ln.connector_to.0, ln.connector_to.1));
    let head_width = connector_head_width(ln.depth, width) * viewport.zoom;
    let tail_width = connector_tail_width(width) * viewport.zoom;
    let turn_x =
        ln.connector_turn_x.expect("non-root mindmap connector must receive a layout turn axis");
    let turn_x = viewport.content_to_screen(CanvasPoint::new(turn_x, 0.0)).x;
    let points =
        connector_centerline((from.x, from.y), (to.x, to.y), turn_x, head_width.max(tail_width));
    if let Some(mesh) = tapered_connector_mesh(&points, head_width, tail_width, 1.0) {
        dl.tapered_mesh(mesh, [0.0, 0.0], color);
    }
}

fn connector_head_width(depth: u8, reference_width: f32) -> f32 {
    let width_scale = reference_width / CONNECTOR_REFERENCE_WIDTH_DP;
    let head_width = match depth {
        1 => FIRST_LEVEL_CONNECTOR_HEAD_WIDTH_DP,
        2 => SECOND_LEVEL_CONNECTOR_HEAD_WIDTH_DP,
        3 => THIRD_LEVEL_CONNECTOR_HEAD_WIDTH_DP,
        _ => CONNECTOR_TAIL_WIDTH_DP,
    };
    head_width * width_scale
}

fn connector_tail_width(reference_width: f32) -> f32 {
    CONNECTOR_TAIL_WIDTH_DP * reference_width / CONNECTOR_REFERENCE_WIDTH_DP
}

fn connector_centerline(
    from: (f32, f32),
    to: (f32, f32),
    turn_x: f32,
    width: f32,
) -> Vec<(f32, f32)> {
    let (fx, fy) = from;
    let (tx, ty) = to;
    let vertical_distance = ty - fy;
    let horizontal_room = (turn_x - fx).abs().min((tx - turn_x).abs());
    let corner_radius = horizontal_room
        .min(vertical_distance.abs() * 0.5)
        .min(CONNECTOR_MAX_CORNER_RADIUS * width / CONNECTOR_REFERENCE_WIDTH_DP);
    if corner_radius <= MIN_CONNECTOR_CORNER_RADIUS {
        return vec![(fx, fy), (tx, ty)];
    }

    let mut points = Vec::with_capacity(CONNECTOR_ARC_SAMPLE_COUNT * 2 + 4);
    push_point(&mut points, (fx, fy));
    push_point(&mut points, (turn_x - corner_radius, fy));

    if vertical_distance > 0.0 {
        append_arc(
            &mut points,
            (turn_x - corner_radius, fy + corner_radius),
            corner_radius,
            -std::f32::consts::FRAC_PI_2,
            0.0,
        );
        push_point(&mut points, (turn_x, ty - corner_radius));
        append_arc(
            &mut points,
            (turn_x + corner_radius, ty - corner_radius),
            corner_radius,
            std::f32::consts::PI,
            std::f32::consts::FRAC_PI_2,
        );
    } else {
        append_arc(
            &mut points,
            (turn_x - corner_radius, fy - corner_radius),
            corner_radius,
            std::f32::consts::FRAC_PI_2,
            0.0,
        );
        push_point(&mut points, (turn_x, ty + corner_radius));
        append_arc(
            &mut points,
            (turn_x + corner_radius, ty + corner_radius),
            corner_radius,
            std::f32::consts::PI,
            std::f32::consts::PI + std::f32::consts::FRAC_PI_2,
        );
    }
    push_point(&mut points, (tx, ty));
    points
}

fn append_arc(
    points: &mut Vec<(f32, f32)>,
    center: (f32, f32),
    radius: f32,
    start_angle: f32,
    end_angle: f32,
) {
    let angle_delta = end_angle - start_angle;
    for step in 1..=CONNECTOR_ARC_SAMPLE_COUNT {
        let progress = step as f32 / CONNECTOR_ARC_SAMPLE_COUNT as f32;
        let angle = start_angle + angle_delta * progress;
        push_point(points, (center.0 + angle.cos() * radius, center.1 + angle.sin() * radius));
    }
}

fn tapered_connector_mesh(
    centerline: &[(f32, f32)],
    head_width: f32,
    tail_width: f32,
    scale: f32,
) -> Option<Arc<TaperedMesh>> {
    let centerline = centerline.iter().map(|&(x, y)| [x, y]).collect::<Vec<_>>();
    tessellate_tapered_path(TaperedPathInput {
        centerline: &centerline,
        head_width,
        tail_width,
        scale,
        feather_width: TAPERED_PATH_FEATHER_PX,
    })
    .map(Arc::new)
}

fn push_point(points: &mut Vec<(f32, f32)>, point: (f32, f32)) {
    if points.last().is_some_and(|last| distance(*last, point) <= ZERO_DISTANCE_EPSILON) {
        return;
    }
    points.push(point);
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    (dx * dx + dy * dy).sqrt()
}

/// 节点文本在屏幕上的字号：基础字号 × 深度缩放 × 视口缩放。
/// 与 Task 5/6 的布局、命中度量使用同一深度缩放，保证渲染与几何一致。
pub(crate) fn node_font_size(
    base_font_size: f32,
    depth: u8,
    zoom: f32,
    constants: &LayoutConstants,
) -> f32 {
    base_font_size * constants.font_scale_for_depth(depth) * zoom
}

pub(crate) fn render_text(
    dl: &mut DrawList,
    layout: &LayoutTree,
    visible_node_indices: &[usize],
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    nodes: &[&Node],
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
) {
    let font_family = shaper.font_family().map(str::to_owned);
    let font_weight = shaper.font_weight();
    let font_style = shaper.font_style();
    for &i in visible_node_indices {
        let ln = &layout.nodes[i];
        let Some(node) = nodes.get(ln.source_node_index) else {
            continue;
        };
        let title = projection.projected_title(ln.source_node_index);
        let style = get_node_style(node, ln, theme);
        let color = if node.title.is_empty() && title == EMPTY_TITLE_PLACEHOLDER {
            placeholder_color(style.text)
        } else {
            style.text
        };
        let opacity = source_subtree_opacity(projection.drag_preview, layout_rect(ln), theme);

        let text_origin = viewport.content_to_screen(CanvasPoint::new(
            ln.x + constants.card_padding_x,
            ln.y + constants.card_padding_y,
        ));
        let font_size = node_font_size(shaper.font_size(), ln.depth, viewport.zoom, constants);
        let baseline_y = text_origin.y + font_size;

        if !title.is_empty() {
            dl.text_shaped_with_font(
                text_origin.x,
                baseline_y,
                font_size,
                with_alpha(color, opacity),
                title,
                font_family.clone(),
                font_weight,
                font_style,
                false,
                shaper,
            );
        }
    }
}

fn measure_text(text: &str, shaper: &mut shaping::Shaper) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    shaper.shape(text).map(|run| run.width).unwrap_or(text.len() as f32 * shaper.font_size() * 0.5)
}

fn placeholder_color(mut color: [f32; 4]) -> [f32; 4] {
    color[3] *= PLACEHOLDER_ALPHA_MULTIPLIER;
    color
}

fn with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], color[3] * alpha]
}

fn source_subtree_opacity(
    preview: Option<&CanvasDragPreview>,
    rect: Rect,
    theme: &MindmapRenderTheme<'_>,
) -> f32 {
    preview
        .filter(|preview| is_source_subtree_node(preview, rect))
        .map(|_| theme.geometry.drag_source_alpha)
        .unwrap_or(1.0)
}

fn is_source_subtree_node(preview: &CanvasDragPreview, rect: Rect) -> bool {
    rect == preview.source_rect || preview.source_subtree_rects.contains(&rect)
}

fn render_node_selection(
    dl: &mut DrawList,
    layout: &LayoutTree,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    nodes: &[&Node],
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
) {
    let Some(node_index) = projection.selected_node_index() else {
        return;
    };
    let Some(layout_node) = layout_node_for_source(layout, node_index) else {
        return;
    };
    let Some(source_node) = nodes.get(layout_node.source_node_index) else {
        return;
    };
    let rect = viewport.content_rect_to_screen(layout_rect(layout_node));
    let geometry = &theme.geometry;
    let opacity = source_subtree_opacity(projection.drag_preview, layout_rect(layout_node), theme);
    // 选中高亮跟随节点自身颜色：分支节点即分支色，根节点/语义节点即其 accent
    let highlight = get_node_style(source_node, layout_node, theme).accent;
    dl.fill_rounded(
        rect,
        with_alpha(highlight, SELECTION_FILL_ALPHA * opacity),
        constants.card_radius * viewport.zoom,
    );
    dl.stroke_rounded(
        rect,
        with_alpha(highlight, opacity),
        constants.card_radius * viewport.zoom,
        viewport.zoom,
    );
    let outline_gap = geometry.selection_outline_gap * viewport.zoom;
    let outer_rect = Rect::new(
        rect.x - outline_gap,
        rect.y - outline_gap,
        rect.w + outline_gap * 2.0,
        rect.h + outline_gap * 2.0,
    );
    dl.stroke_rounded(
        outer_rect,
        with_alpha(highlight, opacity),
        (constants.card_radius + geometry.selection_outline_gap) * viewport.zoom,
        geometry.selection_outline_width * viewport.zoom,
    );
}
fn render_drag_preview(
    dl: &mut DrawList,
    preview: &CanvasDragPreview,
    layout: &LayoutTree,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    nodes: &[&Node],
    viewport: CanvasViewportSnapshot,
) {
    let preview_rect = viewport.content_rect_to_screen(preview.preview_rect);
    let source_layout_node =
        layout.nodes.iter().find(|node| layout_rect(node) == preview.source_rect);
    let (fill, border) = if preview.is_valid {
        let style = source_layout_node
            .and_then(|layout_node| {
                nodes
                    .get(layout_node.source_node_index)
                    .map(|node| get_node_style(node, layout_node, theme))
            })
            .unwrap_or_else(|| theme.node.default.clone());
        (
            with_alpha(style.fill, theme.geometry.drag_preview_alpha),
            with_alpha(style.border, theme.geometry.drag_preview_alpha),
        )
    } else {
        (
            with_alpha(theme.canvas.drag_invalid, theme.geometry.drag_preview_alpha),
            theme.canvas.drag_invalid,
        )
    };
    dl.fill_rounded(preview_rect, fill, constants.card_radius * viewport.zoom);
    dl.stroke_rounded(preview_rect, border, constants.card_radius * viewport.zoom, viewport.zoom);
    if !preview.label.is_empty() {
        let text_color =
            if preview.is_valid { theme.node.default.text } else { theme.canvas.background };
        // 源节点查找失败（如无效拖拽）时回退到深度 2——默认配色中 1.0 缩放档，
        // 避免预览文本相对拖拽源出现突兀的字号跳变。
        let source_depth =
            source_layout_node.map(|node| node.depth).unwrap_or(FALLBACK_PREVIEW_DEPTH);
        let font_size = node_font_size(shaper.font_size(), source_depth, viewport.zoom, constants);
        let text_origin = viewport.content_to_screen(CanvasPoint::new(
            preview.preview_rect.x + constants.card_padding_x,
            preview.preview_rect.y + constants.card_padding_y,
        ));
        dl.text_shaped(
            text_origin.x,
            text_origin.y + font_size,
            font_size,
            with_alpha(text_color, theme.geometry.drag_preview_alpha),
            &preview.label,
            shaper,
        );
    }
}

fn render_drag_target_feedback(
    dl: &mut DrawList,
    preview: &CanvasDragPreview,
    layout: &LayoutTree,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    viewport: CanvasViewportSnapshot,
) {
    let target_color =
        if preview.is_valid { theme.canvas.focus_ring } else { theme.canvas.drag_invalid };
    if let Some(target_rect) = preview.target_rect {
        dl.stroke_rounded(
            viewport.content_rect_to_screen(target_rect),
            target_color,
            constants.card_radius * viewport.zoom,
            theme.geometry.selection_outline_width * viewport.zoom,
        );
    }
    if let Some(guide_to) = preview.guide_to {
        let guide_color =
            if preview.is_valid { theme.canvas.connector_hover } else { theme.canvas.drag_invalid };
        let parent_anchor = viewport.content_to_screen(CanvasPoint::new(guide_to.0, guide_to.1));
        let child_preview = viewport
            .content_to_screen(CanvasPoint::new(preview.guide_from.0, preview.guide_from.1));
        let turn_x = (parent_anchor.x + child_preview.x) * 0.5;
        let points = connector_centerline(
            (parent_anchor.x, parent_anchor.y),
            (child_preview.x, child_preview.y),
            turn_x,
            constants.connector_width * viewport.zoom,
        );
        let head_width =
            drag_guide_head_width(preview, layout, constants.connector_width) * viewport.zoom;
        let tail_width = connector_tail_width(constants.connector_width) * viewport.zoom;
        if let Some(mesh) = tapered_connector_mesh(&points, head_width, tail_width, 1.0) {
            dl.tapered_mesh(mesh, [0.0, 0.0], guide_color);
        }
    }
    if !preview.is_valid {
        return;
    }
    if let Some(((from_x, from_y), (to_x, _))) = preview.insertion_line {
        dl.fill_rounded(
            viewport.content_rect_to_screen(Rect::new(
                from_x,
                from_y - DRAG_INSERTION_LINE_HEIGHT * 0.5,
                to_x - from_x,
                DRAG_INSERTION_LINE_HEIGHT,
            )),
            target_color,
            DRAG_INSERTION_LINE_HEIGHT * viewport.zoom * 0.5,
        );
    }
}

fn drag_guide_head_width(
    preview: &CanvasDragPreview,
    layout: &LayoutTree,
    reference_width: f32,
) -> f32 {
    let target_parent_depth = preview.target_rect.and_then(|target_rect| {
        layout
            .nodes
            .iter()
            .find(|layout_node| layout_rect(layout_node) == target_rect)
            .map(|layout_node| layout_node.depth)
    });
    let Some(target_parent_depth) = target_parent_depth else {
        return connector_tail_width(reference_width);
    };

    connector_head_width(target_parent_depth.saturating_add(1), reference_width)
}

fn layout_rect(node: &LayoutNode) -> Rect {
    Rect::new(node.x, node.y, node.w, node.h)
}

fn layout_node_for_source(layout: &LayoutTree, source_node_index: usize) -> Option<&LayoutNode> {
    layout.nodes.iter().find(|node| node.source_node_index == source_node_index)
}

fn hit_geometry_for_source(
    hit_map: &HitMap,
    source_node_index: usize,
) -> Option<&super::layout::NodeHitGeometry> {
    hit_map.nodes.iter().find(|geometry| geometry.source_node_index == source_node_index)
}

fn render_title_selection(
    dl: &mut DrawList,
    layout: &LayoutTree,
    hit_map: &HitMap,
    visible_node_indices: &[usize],
    theme: &MindmapRenderTheme<'_>,
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
) {
    let MindmapFocus::TitleTextSelected { node_index, ref range } = projection.focus else {
        return;
    };
    if !projection.preedit_text.is_empty()
        || !visible_node_indices
            .iter()
            .any(|index| layout.nodes[*index].source_node_index == node_index)
    {
        return;
    }
    let Some(geometry) = hit_geometry_for_source(hit_map, node_index) else {
        return;
    };
    let start = range.start.saturating_sub(geometry.title_byte_range.start);
    let end = range.end.saturating_sub(geometry.title_byte_range.start);
    let start_index = grapheme_edge_at_or_before(&geometry.grapheme_byte_offsets, start);
    let end_index = grapheme_edge_at_or_before(&geometry.grapheme_byte_offsets, end);
    let Some(start_x) = geometry.grapheme_edges.get(start_index).copied() else {
        return;
    };
    let Some(end_x) = geometry.grapheme_edges.get(end_index).copied() else {
        return;
    };
    if end_x <= start_x {
        return;
    }
    dl.fill(
        viewport.content_rect_to_screen(Rect::new(
            start_x,
            geometry.title_rect.y,
            end_x - start_x,
            geometry.title_rect.h,
        )),
        theme.canvas.selection,
    );
}

fn render_preedit_underline(
    dl: &mut DrawList,
    hit_map: &HitMap,
    theme: &MindmapRenderTheme<'_>,
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
) {
    let Some((node_index, preedit_range)) = &projection.preedit_range else {
        return;
    };
    let Some(geometry) = hit_geometry_for_source(hit_map, *node_index) else {
        return;
    };
    let start_index =
        grapheme_edge_at_or_before(&geometry.grapheme_byte_offsets, preedit_range.start);
    let end_index = grapheme_edge_at_or_before(&geometry.grapheme_byte_offsets, preedit_range.end);
    let Some(start_x) = geometry.grapheme_edges.get(start_index).copied() else {
        return;
    };
    let Some(end_x) = geometry.grapheme_edges.get(end_index).copied() else {
        return;
    };
    if end_x <= start_x {
        return;
    }
    dl.fill(
        viewport.content_rect_to_screen(Rect::new(
            start_x,
            geometry.title_rect.y + geometry.title_rect.h - PREEDIT_UNDERLINE_OFFSET,
            end_x - start_x,
            PREEDIT_UNDERLINE_HEIGHT,
        )),
        theme.canvas.focus_ring,
    );
}

fn render_caret(
    dl: &mut DrawList,
    layout: &LayoutTree,
    hit_map: &HitMap,
    theme: &MindmapRenderTheme<'_>,
    nodes: &[&Node],
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
) {
    if !projection.cursor_visible {
        return;
    }
    let Some((node_index, byte_offset)) = projection.caret() else {
        return;
    };
    let Some(geometry) = hit_geometry_for_source(hit_map, node_index) else {
        return;
    };
    let Some(layout_node) = layout.nodes.iter().find(|node| node.source_node_index == node_index)
    else {
        return;
    };
    let Some(node) = nodes.get(node_index) else {
        return;
    };
    let edge_index = grapheme_edge_at_or_before(&geometry.grapheme_byte_offsets, byte_offset);
    let Some(x) = geometry.grapheme_edges.get(edge_index).copied() else {
        return;
    };
    dl.fill(
        viewport.content_rect_to_screen(Rect::new(
            x - CARET_WIDTH * 0.5,
            geometry.title_rect.y,
            CARET_WIDTH,
            geometry.title_rect.h,
        )),
        get_node_style(node, layout_node, theme).text,
    );
}

fn grapheme_edge_at_or_before(boundaries: &[usize], byte_offset: usize) -> usize {
    match boundaries.binary_search(&byte_offset) {
        Ok(index) => index,
        Err(next_index) => next_index.saturating_sub(1),
    }
}

pub(crate) fn render(
    dl: &mut DrawList,
    layout: &LayoutTree,
    viewport: CanvasViewportSnapshot,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    nodes: &[&Node],
    hit_map: Option<&HitMap>,
    projection: &MindmapRenderProjection<'_>,
    connector_mesh_cache: Option<&ConnectorMeshCache>,
) {
    render_cards_and_connectors(
        dl,
        layout,
        viewport,
        theme,
        constants,
        shaper,
        nodes,
        hit_map,
        projection,
        connector_mesh_cache,
    );
}

pub(crate) fn render_cards_and_connectors(
    dl: &mut DrawList,
    layout: &LayoutTree,
    viewport: CanvasViewportSnapshot,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    nodes: &[&Node],
    hit_map: Option<&HitMap>,
    projection: &MindmapRenderProjection<'_>,
    connector_mesh_cache: Option<&ConnectorMeshCache>,
) {
    let layout_viewport = viewport.screen_rect_to_content(viewport.viewport);
    let visible = layout.visible_node_indices(layout_viewport, 0.0);
    render_connectors(
        dl,
        layout,
        layout_viewport,
        theme,
        constants,
        projection.drag_preview,
        viewport,
        connector_mesh_cache,
    );
    render_cards_with_hover(
        dl,
        layout,
        &visible,
        theme,
        constants,
        nodes,
        projection.drag_preview,
        viewport,
        hit_map,
        projection.canvas_pointer,
        projection.editing_node_index(),
    );
    if let Some(hit_map) = hit_map {
        render_controls(
            dl,
            layout,
            hit_map,
            nodes,
            theme,
            shaper,
            projection,
            viewport,
            projection.drag_preview.is_some(),
        );
    }
    render_node_selection(dl, layout, theme, constants, nodes, projection, viewport);
    if let Some(preview) = projection.drag_preview {
        render_drag_preview(dl, preview, layout, theme, constants, shaper, nodes, viewport);
        render_drag_target_feedback(dl, preview, layout, theme, constants, viewport);
    }
    if let Some(hit_map) = hit_map {
        render_title_selection(dl, layout, hit_map, &visible, theme, projection, viewport);
    }
    render_text(dl, layout, &visible, theme, constants, shaper, nodes, projection, viewport);
    if let Some(hit_map) = hit_map {
        render_preedit_underline(dl, hit_map, theme, projection, viewport);
        render_caret(dl, layout, hit_map, theme, nodes, projection, viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::canvas::{
        CanvasViewPosition, CanvasViewportConfig, CanvasViewportInput, resolve_viewport,
    };
    use ui::core::paint::DrawCmd;
    use ui::theme::{MindmapRenderTheme, Theme, ThemeDefinition};

    fn render_theme(theme: &Theme) -> MindmapRenderTheme<'_> {
        MindmapRenderTheme {
            canvas: &theme.mindmap.canvas,
            node: &theme.mindmap.node,
            semantic: &theme.mindmap.semantic,
            geometry: &theme.mindmap.geometry,
        }
    }

    fn test_viewport(viewport: Rect, content_bounds: Rect) -> CanvasViewportSnapshot {
        test_viewport_at_zoom(viewport, content_bounds, 1.0)
    }

    fn test_viewport_at_zoom(
        viewport: Rect,
        content_bounds: Rect,
        zoom: f32,
    ) -> CanvasViewportSnapshot {
        resolve_viewport(CanvasViewportInput::positioned(
            viewport,
            content_bounds,
            CanvasViewPosition { zoom, scroll: CanvasPoint::ZERO },
            CanvasViewportConfig {
                base_content_padding: 0.0,
                min_screen_padding: 0.0,
                min_initial_fit_zoom: 1.0,
            },
        ))
    }

    fn single_node_layout() -> LayoutTree {
        LayoutTree {
            nodes: vec![LayoutNode {
                x: 10.0,
                y: 20.0,
                w: 80.0,
                h: 44.0,
                node_idx: 0,
                source_node_index: 0,
                depth: 0,
                connector_from: (10.0, 42.0),
                connector_to: (10.0, 42.0),
                connector_turn_x: None,
                branch_index: None,
            }],
            y_sorted_indices: vec![0],
            total_w: 90.0,
            total_h: 64.0,
        }
    }

    #[test]
    fn node_font_size_scales_with_depth_and_zoom() {
        let constants = LayoutConstants::default();

        let root = node_font_size(14.0, 0, 2.0, &constants);
        let level2 = node_font_size(14.0, 2, 2.0, &constants);
        assert!(root > level2);
        assert_eq!(level2, 14.0 * 1.0 * 2.0);
    }

    fn empty_node(heading_level: u8) -> Node {
        Node {
            title: "".into(),
            children: vec![],
            props: None,
            note: None,
            source_range: 0..0,
            subtree_source_range: 0..0,
            title_byte_range: 0..0,
            heading_marker_range: 0..0,
            child_insertion_byte: 0,
            heading_level,
            property_source: None,
            heading_source_end: 0,
        }
    }

    fn plain_projection<'a>(
        titles: impl IntoIterator<Item = &'a str>,
    ) -> MindmapRenderProjection<'a> {
        MindmapRenderProjection {
            focus: MindmapFocus::None,
            projected_titles: titles.into_iter().map(Cow::Borrowed).collect(),
            preedit_text: "",
            preedit_cursor: None,
            cursor_visible: true,
            caret: None,
            composition_caret: None,
            preedit_range: None,
            collapsed_descendant_counts: Vec::new(),
            canvas_pointer: None,
            drag_preview: None,
        }
    }

    fn tapered_mesh_commands(
        draw_list: &DrawList,
    ) -> Vec<&std::sync::Arc<ui::tapered_path::TaperedMesh>> {
        draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TaperedMesh { mesh, .. } => Some(mesh),
                _ => None,
            })
            .collect()
    }

    fn label_visual_center(
        layout: &ui::core::text_layout::UiTextLayout,
        x: f32,
        y_baseline: f32,
        shaper: &mut shaping::Shaper,
    ) -> CanvasPoint {
        let mut weighted_x = 0.0;
        let mut weighted_y = 0.0;
        let mut total_weight = 0.0;
        let mut x_cursor = x;

        for cluster in &layout.shaped.clusters {
            let (_, subpixel_phase) = render::split_subpixel(x_cursor);
            let subpixel_x = subpixel_phase as f32 * 0.25;
            if let Some(bitmap) = shaper.rasterize_glyph(
                cluster.font_id,
                cluster.glyph_id as u16,
                layout.font_size,
                (subpixel_x, 0.0),
            ) && bitmap.width > 0
                && bitmap.height > 0
            {
                let glyph_left = (x_cursor + bitmap.left as f32).round();
                let glyph_top = (y_baseline - bitmap.top as f32).round();
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

        assert!(total_weight > 0.0, "control label should contain visible glyph pixels");
        CanvasPoint::new(weighted_x / total_weight, weighted_y / total_weight)
    }

    fn assert_label_visual_center_is_centered(label_center: CanvasPoint, control_bounds: Rect) {
        let control_center_x = control_bounds.x + control_bounds.w * 0.5;
        let control_center_y = control_bounds.y + control_bounds.h * 0.5;
        const MAX_CENTER_OFFSET_PX: f32 = 0.5;

        assert!(
            (label_center.x - control_center_x).abs() <= MAX_CENTER_OFFSET_PX,
            "label visual weight should be horizontally centered: label={label_center:?}, control={control_bounds:?}"
        );
        assert!(
            (label_center.y - control_center_y).abs() <= MAX_CENTER_OFFSET_PX,
            "label visual weight should be vertically centered: label={label_center:?}, control={control_bounds:?}"
        );
    }

    fn rendered_control_bounds(
        draw_list: &DrawList,
        logical_bounds: Rect,
        background: [f32; 4],
    ) -> Rect {
        draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, color, .. }
                    if *color == background
                        && (rect.w - logical_bounds.w).abs() < f32::EPSILON
                        && (rect.h - logical_bounds.h).abs() < f32::EPSILON
                        && (rect.x - logical_bounds.x).abs() < 1.0
                        && (rect.y - logical_bounds.y).abs() < 1.0 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("control background")
    }

    fn assert_expanded_control_bar_is_centered(
        draw_list: &DrawList,
        control_bounds: Rect,
        connector_color: [f32; 4],
        zoom: f32,
    ) {
        let expected_width = EXPANDED_CONTROL_BAR_WIDTH_DP * zoom;
        let expected_height = EXPANDED_CONTROL_BAR_HEIGHT_DP * zoom;
        let expected_center = CanvasPoint::new(
            control_bounds.x + control_bounds.w * 0.5,
            control_bounds.y + control_bounds.h * 0.5,
        );
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, color, radius }
                if *color == connector_color
                    && *radius == 0.0
                    && (rect.w - expected_width).abs() < f32::EPSILON
                    && (rect.h - expected_height).abs() < f32::EPSILON
                    && (rect.x + rect.w * 0.5 - expected_center.x).abs() < f32::EPSILON
                    && (rect.y + rect.h * 0.5 - expected_center.y).abs() < f32::EPSILON
        )));
        assert!(!draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, .. } if layout.text == "-"
        )));
    }

    fn expanded_control_bar_count(draw_list: &DrawList, connector_color: [f32; 4]) -> usize {
        draw_list
            .cmds
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    DrawCmd::FillRect { rect, color, radius }
                        if *color == connector_color
                            && *radius == 0.0
                            && (rect.w - EXPANDED_CONTROL_BAR_WIDTH_DP).abs() < f32::EPSILON
                            && (rect.h - EXPANDED_CONTROL_BAR_HEIGHT_DP).abs() < f32::EPSILON
                )
            })
            .count()
    }

    #[test]
    fn card_border_uses_configured_radius_and_one_pixel_width() {
        let mut dl = DrawList::new();
        let constants = LayoutConstants { card_radius: 10.0, ..LayoutConstants::default() };
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let node = Node {
            title: "".into(),
            children: vec![],
            props: None,
            note: None,
            source_range: 0..0,
            subtree_source_range: 0..0,
            title_byte_range: 0..0,
            heading_marker_range: 0..0,
            child_insertion_byte: 0,
            heading_level: 1,
            property_source: None,
            heading_source_end: 0,
        };
        let nodes = vec![&node];

        let render_theme = render_theme(&theme);
        render_cards(
            &mut dl,
            &layout,
            &[0],
            &render_theme,
            &constants,
            &nodes,
            None,
            test_viewport(Rect::new(0.0, 0.0, 200.0, 200.0), Rect::new(0.0, 0.0, 200.0, 200.0)),
        );

        let stroke = dl
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                DrawCmd::StrokeRect { radius, line_width, .. } => Some((*radius, *line_width)),
                _ => None,
            })
            .expect("card render should emit a border stroke");
        assert_eq!(stroke, (10.0, DEFAULT_CARD_BORDER_WIDTH));
    }

    #[test]
    fn hovered_card_border_is_wider_than_default_border() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let node = Node { title: "Branch".into(), ..empty_node(2) };
        let nodes = vec![&node];
        let viewport =
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0));

        let mut default_draw_list = DrawList::new();
        let render_theme = render_theme(&theme);
        render_cards(
            &mut default_draw_list,
            &layout,
            &[0],
            &render_theme,
            &constants,
            &nodes,
            None,
            viewport,
        );
        let default_border_width =
            default_draw_list.cmds.iter().find_map(|command| match command {
                DrawCmd::StrokeRect { line_width, .. } => Some(*line_width),
                _ => None,
            });

        let mut hovered_draw_list = DrawList::new();
        render_cards_with_hover(
            &mut hovered_draw_list,
            &layout,
            &[0],
            &render_theme,
            &constants,
            &nodes,
            None,
            viewport,
            None,
            Some(CanvasPoint::new(50.0, 40.0)),
            None,
        );
        let hovered_border_width =
            hovered_draw_list.cmds.iter().find_map(|command| match command {
                DrawCmd::StrokeRect { line_width, .. } => Some(*line_width),
                _ => None,
            });

        assert!(
            hovered_border_width.expect("hovered card border")
                > default_border_width.expect("default card border")
        );
    }

    #[test]
    fn title_editing_card_uses_hover_visuals_without_pointer_hover() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let node = Node { title: "Branch".into(), ..empty_node(1) };
        let nodes = vec![&node];
        let viewport =
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0));
        let mut projection = plain_projection(["Branch"]);
        projection.focus = MindmapFocus::TitleEditing { node_index: 0, cursor_byte: 0 };
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            viewport,
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &projection,
            None,
        );

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { line_width, .. }
                if (*line_width - HOVERED_CARD_BORDER_WIDTH).abs() < f32::EPSILON
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { color, .. }
                if *color == with_alpha(
                    theme.mindmap.node.root.fill,
                    HOVER_FILL_ALPHA_MULTIPLIER,
                )
        )));
    }

    #[test]
    fn render_translates_screen_viewport_to_layout_space() {
        let mut dl = DrawList::new();
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let node = Node {
            title: "".into(),
            children: vec![],
            props: None,
            note: None,
            source_range: 0..0,
            subtree_source_range: 0..0,
            title_byte_range: 0..0,
            heading_marker_range: 0..0,
            child_insertion_byte: 0,
            heading_level: 1,
            property_source: None,
            heading_source_end: 0,
        };
        let nodes = vec![&node];
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let projection = plain_projection([""]);

        let render_theme = render_theme(&theme);
        render(
            &mut dl,
            &layout,
            test_viewport(Rect::new(10.0, 400.0, 200.0, 100.0), Rect::new(0.0, 0.0, 200.0, 100.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &projection,
            None,
        );

        assert!(dl.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, .. } if (rect.y - 420.0).abs() < f32::EPSILON
        )));
    }

    #[test]
    fn collapsed_title_omits_descendant_suffix_at_zoom_levels() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let mut node = Node { title: "Branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: true,
            tags: Vec::new(),
            color: None,
        });
        let nodes = vec![&node];
        let projection = MindmapRenderProjection {
            collapsed_descendant_counts: vec![Some(3)],
            ..plain_projection(["Branch"])
        };

        for zoom in [2.0, 0.5] {
            let viewport = resolve_viewport(CanvasViewportInput::positioned(
                Rect::new(0.0, 0.0, 1_000.0, 400.0),
                Rect::new(0.0, 0.0, 400.0, 200.0),
                CanvasViewPosition { zoom, scroll: CanvasPoint::ZERO },
                CanvasViewportConfig {
                    base_content_padding: 0.0,
                    min_screen_padding: 0.0,
                    min_initial_fit_zoom: 1.0,
                },
            ));
            let mut draw_list = DrawList::new();
            let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

            let render_theme = render_theme(&theme);
            render_text(
                &mut draw_list,
                &layout,
                &[0],
                &render_theme,
                &constants,
                &mut shaper,
                &nodes,
                &projection,
                viewport,
            );

            assert!(draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::TextLayout { layout, .. } if layout.text == "Branch"
            )));
            assert!(!draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::TextLayout { layout, .. } if layout.text == " · 3"
            )));
        }
    }

    #[test]
    fn controls_are_visible_without_pointer_and_show_collapsed_descendant_count() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let mut node = Node { title: "Branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: true,
            tags: Vec::new(),
            color: None,
        });
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let projection = MindmapRenderProjection {
            collapsed_descendant_counts: vec![Some(4)],
            ..plain_projection(["Branch"])
        };
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, .. } if layout.text == "4"
        )));
        let visual_control_bounds =
            rendered_control_bounds(&draw_list, control_bounds, theme.mindmap.canvas.background);
        let expected_ring = visual_control_bounds.shrink(
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
        );
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { rect, color, radius, .. }
                if *rect == expected_ring
                    && *color == theme.mindmap.canvas.connector
                    && (radius - expected_ring.w.min(expected_ring.h) * CONTROL_CIRCLE_RADIUS_RATIO).abs()
                        < f32::EPSILON
        )));
        let label_center = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TextLayout { layout, x, y_baseline, .. } if layout.text == "4" => {
                    Some(label_visual_center(layout, *x, *y_baseline, &mut shaper))
                }
                _ => None,
            })
            .expect("collapsed control label");
        assert_label_visual_center_is_centered(label_center, control_bounds);
    }

    #[test]
    fn collapsed_control_keeps_chrome_and_count_within_constrained_rect() {
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let control_rect = Rect::new(100.0, 40.0, 25.0, 25.0);
        let viewport =
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0));
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_collapsed_control(
            &mut draw_list,
            control_rect,
            "12345",
            shaper.font_size(),
            &render_theme,
            theme.mindmap.canvas.connector,
            &mut shaper,
            viewport,
        );

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, color, .. }
                if *rect == control_rect && *color == theme.mindmap.canvas.background
        )));
        assert!(draw_list.cmds.windows(3).any(|commands| matches!(
            commands,
            [
                DrawCmd::PushClip(clip_rect),
                DrawCmd::TextLayout { layout, .. },
                DrawCmd::PopClip,
            ] if *clip_rect == control_rect && layout.text == "12345"
        )));
    }

    #[test]
    fn expanded_control_draws_a_centered_geometry_bar_without_text() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let node = Node { title: "Branch".into(), children: vec![empty_node(3)], ..empty_node(2) };
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let mut projection = plain_projection(["Branch"]);
        projection.canvas_pointer = Some(CanvasPoint::new(50.0, 42.0));
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        let visual_control_bounds =
            rendered_control_bounds(&draw_list, control_bounds, theme.mindmap.canvas.background);
        assert_expanded_control_bar_is_centered(
            &draw_list,
            visual_control_bounds,
            theme.mindmap.canvas.connector_hover,
            1.0,
        );
    }

    #[test]
    fn expanded_control_bar_stays_within_a_five_dp_control_rect() {
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let control_rect = Rect::new(90.0, 20.0, 5.0, 5.0);
        let viewport =
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0));
        let mut draw_list = DrawList::new();

        let render_theme = render_theme(&theme);
        render_expanded_control(
            &mut draw_list,
            control_rect,
            &render_theme,
            theme.mindmap.canvas.connector_hover,
            viewport,
        );

        let bar_rect = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, color, radius }
                    if *color == theme.mindmap.canvas.connector_hover && *radius == 0.0 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("expanded control bar should be drawn");

        assert!(bar_rect.x >= control_rect.x);
        assert!(bar_rect.y >= control_rect.y);
        assert!(bar_rect.x + bar_rect.w <= control_rect.x + control_rect.w);
        assert!(bar_rect.y + bar_rect.h <= control_rect.y + control_rect.h);
        assert!(bar_rect.w <= 5.0);
        assert!(bar_rect.h <= 5.0);
    }

    #[test]
    fn collapsed_control_label_stays_centered_at_canvas_zoom() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let mut node = Node { title: "Branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: true,
            tags: Vec::new(),
            color: None,
        });
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let projection = MindmapRenderProjection {
            collapsed_descendant_counts: vec![Some(3)],
            ..plain_projection(["Branch"])
        };
        let viewport = test_viewport_at_zoom(
            Rect::new(0.0, 0.0, 600.0, 300.0),
            Rect::new(0.0, 0.0, 400.0, 200.0),
            1.75,
        );
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            viewport,
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        let control_rect = viewport.content_rect_to_screen(control_bounds);
        let label_center = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TextLayout { layout, x, y_baseline, .. } if layout.text == "3" => {
                    Some(label_visual_center(layout, *x, *y_baseline, &mut shaper))
                }
                _ => None,
            })
            .expect("collapsed control label");
        assert_label_visual_center_is_centered(label_center, control_rect);
    }

    #[test]
    fn collapse_control_uses_connector_color_for_light_theme_consistency() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_light());
        let layout = single_node_layout();
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let mut node = Node { title: "Branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: true,
            tags: Vec::new(),
            color: None,
        });
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let projection = MindmapRenderProjection {
            collapsed_descendant_counts: vec![Some(3)],
            ..plain_projection(["Branch"])
        };
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { color, .. } if *color == theme.mindmap.canvas.connector
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, color, .. }
                if layout.text == "3" && *color == theme.mindmap.canvas.connector
        )));
    }

    #[test]
    fn collapsed_control_uses_branch_color_when_available() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let branch_color = theme.mindmap.canvas.branch_color(0).expect("palette is non-empty");
        let layout = LayoutTree {
            nodes: vec![LayoutNode {
                x: 10.0,
                y: 20.0,
                w: 80.0,
                h: 44.0,
                node_idx: 0,
                source_node_index: 0,
                depth: 1,
                connector_from: (90.0, 42.0),
                connector_to: (10.0, 42.0),
                connector_turn_x: Some(50.0),
                branch_index: Some(0),
            }],
            y_sorted_indices: vec![0],
            total_w: 90.0,
            total_h: 64.0,
        };
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let mut node = Node { title: "Branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: true,
            tags: Vec::new(),
            color: None,
        });
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let projection = MindmapRenderProjection {
            collapsed_descendant_counts: vec![Some(3)],
            ..plain_projection(["Branch"])
        };
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { color, .. } if *color == branch_color
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, color, .. }
                if layout.text == "3" && *color == branch_color
        )));
    }

    #[test]
    fn expanded_geometry_bar_is_centered_at_canvas_zoom() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_light());
        let layout = single_node_layout();
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let node = Node { title: "Branch".into(), children: vec![empty_node(3)], ..empty_node(2) };
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let mut projection = plain_projection(["Branch"]);
        projection.canvas_pointer = Some(CanvasPoint::new(50.0, 42.0));
        let viewport = test_viewport_at_zoom(
            Rect::new(0.0, 0.0, 600.0, 300.0),
            Rect::new(0.0, 0.0, 400.0, 200.0),
            1.75,
        );
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            viewport,
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        let visual_control_bounds = rendered_control_bounds(
            &draw_list,
            viewport.content_rect_to_screen(control_bounds),
            theme.mindmap.canvas.background,
        );
        assert_expanded_control_bar_is_centered(
            &draw_list,
            visual_control_bounds,
            theme.mindmap.canvas.connector_hover,
            viewport.zoom,
        );
    }

    #[test]
    fn expanded_control_is_hidden_without_hover() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = single_node_layout();
        let node = Node { title: "Branch".into(), children: vec![empty_node(3)], ..empty_node(2) };
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: Rect::new(90.0, 20.0, 24.0, 24.0),
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let projection = plain_projection(["Branch"]);
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        assert_eq!(expanded_control_bar_count(&draw_list, theme.mindmap.canvas.connector), 0);
    }

    #[test]
    fn collapsed_control_hover_uses_hover_color_for_ring_and_label() {
        const CONTROL_HOVER_COLOR: [f32; 4] = [0.2, 0.6, 0.3, 1.0];

        let constants = LayoutConstants::default();
        let mut theme = Theme::from_definition(&ThemeDefinition::default_dark());
        theme.mindmap.canvas.connector_hover = CONTROL_HOVER_COLOR;
        let layout = single_node_layout();
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let mut node = Node { title: "Branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: true,
            tags: Vec::new(),
            color: None,
        });
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let mut projection = plain_projection(["Branch"]);
        projection.collapsed_descendant_counts = vec![Some(4)];
        projection.canvas_pointer = Some(CanvasPoint::new(108.0, 38.0));
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        let visual_control_bounds =
            rendered_control_bounds(&draw_list, control_bounds, theme.mindmap.canvas.background);
        let ring_rect = visual_control_bounds.shrink(
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
        );
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { rect, color, .. }
                if *rect == ring_rect && *color == CONTROL_HOVER_COLOR
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, color, .. }
                if layout.text == "4" && *color == CONTROL_HOVER_COLOR
        )));
    }

    #[test]
    fn collapsed_control_hover_uses_lightened_branch_color() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let branch_color = theme.mindmap.canvas.branch_color(0).expect("palette is non-empty");
        let expected_hover_color = lighten_color(branch_color, CONTROL_HOVER_LIGHTEN_FACTOR);
        let layout = LayoutTree {
            nodes: vec![LayoutNode {
                x: 10.0,
                y: 20.0,
                w: 80.0,
                h: 44.0,
                node_idx: 0,
                source_node_index: 0,
                depth: 1,
                connector_from: (90.0, 42.0),
                connector_to: (10.0, 42.0),
                connector_turn_x: Some(50.0),
                branch_index: Some(0),
            }],
            y_sorted_indices: vec![0],
            total_w: 90.0,
            total_h: 64.0,
        };
        let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
        let mut node = Node { title: "Branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: true,
            tags: Vec::new(),
            color: None,
        });
        let nodes = vec![&node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![super::super::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: control_bounds,
            }],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let mut projection = MindmapRenderProjection {
            collapsed_descendant_counts: vec![Some(4)],
            ..plain_projection(["Branch"])
        };
        projection.canvas_pointer = Some(CanvasPoint::new(108.0, 38.0));
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        let visual_control_bounds =
            rendered_control_bounds(&draw_list, control_bounds, theme.mindmap.canvas.background);
        let ring_rect = visual_control_bounds.shrink(
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
            CONTROL_RING_INSET_DP,
        );
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { rect, color, .. }
                if *rect == ring_rect && *color == expected_hover_color
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, color, .. }
                if layout.text == "4" && *color == expected_hover_color
        )));
    }

    #[test]
    fn hovering_one_card_shows_only_its_expanded_control() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = LayoutTree {
            nodes: vec![
                LayoutNode {
                    x: 10.0,
                    y: 20.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 0,
                    source_node_index: 0,
                    depth: 0,
                    connector_from: (10.0, 42.0),
                    connector_to: (10.0, 42.0),
                    connector_turn_x: None,
                    branch_index: None,
                },
                LayoutNode {
                    x: 150.0,
                    y: 20.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 1,
                    source_node_index: 1,
                    depth: 1,
                    connector_from: (90.0, 42.0),
                    connector_to: (150.0, 42.0),
                    connector_turn_x: Some(120.0),
                    branch_index: None,
                },
            ],
            y_sorted_indices: vec![0, 1],
            total_w: 230.0,
            total_h: 64.0,
        };
        let first_node =
            Node { title: "First".into(), children: vec![empty_node(3)], ..empty_node(1) };
        let second_node =
            Node { title: "Second".into(), children: vec![empty_node(3)], ..empty_node(2) };
        let nodes = vec![&first_node, &second_node];
        let hit_map = HitMap {
            nodes: Vec::new(),
            controls: vec![
                super::super::layout::ControlHitGeometry {
                    source_node_index: 0,
                    bounds: Rect::new(90.0, 20.0, 24.0, 24.0),
                },
                super::super::layout::ControlHitGeometry {
                    source_node_index: 1,
                    bounds: Rect::new(230.0, 20.0, 24.0, 24.0),
                },
            ],
            node_rects: Vec::new(),
            title_char_edges: Vec::new(),
        };
        let mut projection = plain_projection(["First", "Second"]);
        projection.canvas_pointer = Some(CanvasPoint::new(40.0, 42.0));
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            Some(&hit_map),
            &projection,
            None,
        );

        let expanded_control_count =
            expanded_control_bar_count(&draw_list, theme.mindmap.canvas.connector_hover);
        assert_eq!(expanded_control_count, 1);
    }

    #[test]
    fn node_selection_uses_node_accent_without_drawing_a_caret() {
        let mut dl = DrawList::new();
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = LayoutTree {
            nodes: vec![
                LayoutNode {
                    x: 0.0,
                    y: 0.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 0,
                    source_node_index: 0,
                    depth: 0,
                    connector_from: (0.0, 22.0),
                    connector_to: (0.0, 22.0),
                    connector_turn_x: None,
                    branch_index: None,
                },
                LayoutNode {
                    x: 160.0,
                    y: 0.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 1,
                    source_node_index: 1,
                    depth: 0,
                    connector_from: (80.0, 22.0),
                    connector_to: (160.0, 22.0),
                    connector_turn_x: None,
                    branch_index: None,
                },
            ],
            y_sorted_indices: vec![0, 1],
            total_w: 240.0,
            total_h: 44.0,
        };
        let root = empty_node(1);
        let child = empty_node(2);
        let nodes = vec![&root, &child];
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let projection = MindmapRenderProjection {
            focus: MindmapFocus::NodeSelected { node_index: 1 },
            projected_titles: vec![Cow::Borrowed(""), Cow::Borrowed("")],
            preedit_text: "",
            preedit_cursor: None,
            cursor_visible: true,
            caret: None,
            composition_caret: None,
            preedit_range: None,
            collapsed_descendant_counts: Vec::new(),
            canvas_pointer: None,
            drag_preview: None,
        };

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut dl,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 400.0, 100.0), Rect::new(0.0, 0.0, 400.0, 100.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &projection,
            None,
        );

        let selected_accent = theme.mindmap.node.root.accent;
        assert!(dl.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { color, .. }
                if *color == with_alpha(selected_accent, SELECTION_FILL_ALPHA)
        )));
        let highlight_ring_widths = dl
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::StrokeRect { color, line_width, .. } if *color == selected_accent => {
                    Some(*line_width)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(highlight_ring_widths.len() >= 2);
        assert!(
            highlight_ring_widths
                .iter()
                .any(|width| (*width - theme.mindmap.geometry.selection_outline_width).abs()
                    < f32::EPSILON)
        );
        assert!(!dl.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, .. } if (rect.w - CARET_WIDTH).abs() < f32::EPSILON
        )));
    }

    #[test]
    fn drag_preview_draws_valid_insertion_feedback_and_invalid_color_without_insertion() {
        const CONNECTOR_HOVER_TEST_COLOR: [f32; 4] = [0.2, 0.6, 0.3, 1.0];

        let constants = LayoutConstants::default();
        let mut theme = Theme::from_definition(&ThemeDefinition::default_dark());
        theme.mindmap.canvas.connector_hover = CONNECTOR_HOVER_TEST_COLOR;
        let layout = LayoutTree {
            nodes: vec![
                LayoutNode {
                    x: 0.0,
                    y: 0.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 0,
                    source_node_index: 0,
                    depth: 0,
                    connector_from: (0.0, 22.0),
                    connector_to: (0.0, 22.0),
                    connector_turn_x: None,
                    branch_index: None,
                },
                LayoutNode {
                    x: 160.0,
                    y: 0.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 1,
                    source_node_index: 1,
                    depth: 1,
                    connector_from: (80.0, 22.0),
                    connector_to: (160.0, 22.0),
                    connector_turn_x: Some(120.0),
                    branch_index: None,
                },
            ],
            y_sorted_indices: vec![0, 1],
            total_w: 240.0,
            total_h: 44.0,
        };
        let root = empty_node(1);
        let child = empty_node(2);
        let nodes = vec![&root, &child];
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let valid_preview = CanvasDragPreview {
            label: "Child · 3".into(),
            source_rect: Rect::new(160.0, 0.0, 80.0, 44.0),
            source_subtree_rects: Vec::new(),
            preview_rect: Rect::new(320.0, 40.0, 80.0, 44.0),
            guide_from: (320.0, 62.0),
            guide_to: Some((240.0, 22.0)),
            insertion_line: Some(((160.0, 44.0), (240.0, 44.0))),
            target_rect: Some(Rect::new(160.0, 0.0, 80.0, 44.0)),
            is_valid: true,
        };
        let projection = MindmapRenderProjection {
            drag_preview: Some(&valid_preview),
            ..plain_projection(["", ""])
        };
        let mut valid_draw_list = DrawList::new();

        let render_theme = render_theme(&theme);
        render_cards_and_connectors(
            &mut valid_draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 500.0, 200.0), Rect::new(0.0, 0.0, 500.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &projection,
            None,
        );

        assert!(valid_draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { color, .. }
                if *color == with_alpha(
                    theme.mindmap.node.depth[0].fill,
                    theme.mindmap.geometry.drag_preview_alpha,
                )
        )));
        let insertion_line_marks = valid_draw_list
            .cmds
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    DrawCmd::FillRect { rect, color, .. }
                        if *color == theme.mindmap.canvas.focus_ring
                            && (rect.h - 2.0).abs() < f32::EPSILON
                )
            })
            .count();
        assert_eq!(insertion_line_marks, 1, "only the insertion line uses the focus ring");

        assert!(valid_draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, color, .. }
                if *color == theme.mindmap.canvas.focus_ring
                    && (rect.w - 80.0).abs() < f32::EPSILON
                    && (rect.h - 2.0).abs() < f32::EPSILON
        )));
        assert!(valid_draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, .. } if layout.text == "Child · 3"
        )));
        assert!(!valid_draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, .. } if layout.text == "+" || layout.text == "-"
        )));

        let mut zoomed_draw_list = DrawList::new();
        let zoomed_viewport = resolve_viewport(CanvasViewportInput::positioned(
            Rect::new(0.0, 0.0, 1_000.0, 400.0),
            Rect::new(0.0, 0.0, 500.0, 200.0),
            CanvasViewPosition { zoom: 2.0, scroll: CanvasPoint::ZERO },
            CanvasViewportConfig {
                base_content_padding: 0.0,
                min_screen_padding: 0.0,
                min_initial_fit_zoom: 1.0,
            },
        ));
        render_cards_and_connectors(
            &mut zoomed_draw_list,
            &layout,
            zoomed_viewport,
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &projection,
            None,
        );
        assert!(zoomed_draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, color, radius }
                if *color == theme.mindmap.canvas.focus_ring
                    && (rect.w - 160.0).abs() < f32::EPSILON
                    && (rect.h - 4.0).abs() < f32::EPSILON
                    && (radius - 2.0).abs() < f32::EPSILON
        )));

        let drag_guide_meshes = valid_draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TaperedMesh { mesh, color, .. }
                    if *color == theme.mindmap.canvas.connector_hover =>
                {
                    Some(mesh)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(drag_guide_meshes.len(), 1, "drag guide must use one dynamic tapered mesh");
        assert!(drag_guide_meshes[0].vertices.len() < 1_000);

        let invalid_preview = CanvasDragPreview { is_valid: false, ..valid_preview };
        let invalid_projection = MindmapRenderProjection {
            drag_preview: Some(&invalid_preview),
            ..plain_projection(["", ""])
        };
        let mut invalid_draw_list = DrawList::new();

        render_cards_and_connectors(
            &mut invalid_draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 500.0, 200.0), Rect::new(0.0, 0.0, 500.0, 200.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &invalid_projection,
            None,
        );

        assert!(invalid_draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { color, .. }
                if *color == with_alpha(
                    theme.mindmap.canvas.drag_invalid,
                    theme.mindmap.geometry.drag_preview_alpha,
                )
        )));
        let invalid_guide_meshes = invalid_draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TaperedMesh { mesh, color, .. }
                    if *color == theme.mindmap.canvas.drag_invalid =>
                {
                    Some(mesh)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(invalid_guide_meshes.len(), 1, "invalid preview should retain its guide line");
        assert!(!invalid_draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, color, .. }
                if *color == theme.mindmap.canvas.focus_ring
                    && (rect.h - 2.0).abs() < f32::EPSILON
        )));
    }

    #[test]
    fn drag_preview_dims_every_source_subtree_visual() {
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let root_rect = Rect::new(0.0, 0.0, 80.0, 44.0);
        let source_rect = Rect::new(160.0, 0.0, 80.0, 44.0);
        let descendant_rect = Rect::new(320.0, 0.0, 80.0, 44.0);
        let layout = LayoutTree {
            nodes: vec![
                LayoutNode {
                    x: root_rect.x,
                    y: root_rect.y,
                    w: root_rect.w,
                    h: root_rect.h,
                    node_idx: 0,
                    source_node_index: 0,
                    depth: 0,
                    connector_from: (0.0, 22.0),
                    connector_to: (0.0, 22.0),
                    connector_turn_x: None,
                    branch_index: None,
                },
                LayoutNode {
                    x: source_rect.x,
                    y: source_rect.y,
                    w: source_rect.w,
                    h: source_rect.h,
                    node_idx: 1,
                    source_node_index: 1,
                    depth: 1,
                    connector_from: (80.0, 22.0),
                    connector_to: (160.0, 22.0),
                    connector_turn_x: Some(120.0),
                    branch_index: None,
                },
                LayoutNode {
                    x: descendant_rect.x,
                    y: descendant_rect.y,
                    w: descendant_rect.w,
                    h: descendant_rect.h,
                    node_idx: 2,
                    source_node_index: 2,
                    depth: 2,
                    connector_from: (240.0, 22.0),
                    connector_to: (320.0, 22.0),
                    connector_turn_x: Some(280.0),
                    branch_index: None,
                },
            ],
            y_sorted_indices: vec![0, 1, 2],
            total_w: 400.0,
            total_h: 44.0,
        };
        let root = Node { title: "root".into(), ..empty_node(1) };
        let source = Node { title: "source".into(), ..empty_node(2) };
        let descendant = Node { title: "descendant".into(), ..empty_node(3) };
        let nodes = vec![&root, &source, &descendant];
        let preview = CanvasDragPreview {
            label: String::new(),
            source_rect,
            source_subtree_rects: vec![source_rect, descendant_rect],
            preview_rect: Rect::new(480.0, 0.0, 80.0, 44.0),
            guide_from: (480.0, 22.0),
            guide_to: None,
            insertion_line: None,
            target_rect: None,
            is_valid: true,
        };
        let projection = MindmapRenderProjection {
            focus: MindmapFocus::NodeSelected { node_index: 1 },
            drag_preview: Some(&preview),
            ..plain_projection(["root", "source", "descendant"])
        };
        let source_opacity = theme.mindmap.geometry.drag_source_alpha;
        let render_theme = render_theme(&theme);
        let root_style = get_node_style(&root, &layout.nodes[0], &render_theme);
        let source_style = get_node_style(&source, &layout.nodes[1], &render_theme);
        let descendant_style = get_node_style(&descendant, &layout.nodes[2], &render_theme);
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");

        render_cards_and_connectors(
            &mut draw_list,
            &layout,
            test_viewport(Rect::new(0.0, 0.0, 600.0, 100.0), Rect::new(0.0, 0.0, 600.0, 100.0)),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &projection,
            None,
        );

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, color, .. }
                if *rect == root_rect && *color == root_style.fill
        )));
        for (rect, style) in [(source_rect, &source_style), (descendant_rect, &descendant_style)] {
            assert!(draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::FillRect { rect: command_rect, color, .. }
                    if *command_rect == rect && *color == with_alpha(style.fill, source_opacity)
            )));
            assert!(draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::StrokeRect { rect: command_rect, color, .. }
                    if *command_rect == rect && *color == with_alpha(style.border, source_opacity)
            )));
        }
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { rect, color, .. }
                if *rect == source_rect
                    && *color == with_alpha(source_style.accent, SELECTION_FILL_ALPHA * source_opacity)
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { rect, color, .. }
                if *rect == source_rect
                    && *color == with_alpha(source_style.accent, source_opacity)
        )));
        for (node_index, style) in [(1, &source_style), (2, &descendant_style)] {
            let text_x = layout.nodes[node_index].x + constants.card_padding_x;
            assert!(draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::TextLayout { x, color, .. }
                    if *x == text_x && *color == with_alpha(style.text, source_opacity)
            )));
        }
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TaperedMesh { color, .. }
                if *color == theme.mindmap.canvas.connector
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TaperedMesh { color, .. }
                if *color == with_alpha(theme.mindmap.canvas.connector, source_opacity)
        )));
    }

    #[test]
    fn render_draws_long_connector_crossing_viewport_without_visible_cards() {
        let mut dl = DrawList::new();
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let layout = LayoutTree {
            nodes: vec![
                LayoutNode {
                    x: 0.0,
                    y: 0.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 0,
                    source_node_index: 0,
                    depth: 0,
                    connector_from: (0.0, 22.0),
                    connector_to: (0.0, 22.0),
                    connector_turn_x: None,
                    branch_index: None,
                },
                LayoutNode {
                    x: 160.0,
                    y: 1_000.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 1,
                    source_node_index: 1,
                    depth: 1,
                    connector_from: (80.0, 22.0),
                    connector_to: (160.0, 1_022.0),
                    connector_turn_x: Some(120.0),
                    branch_index: None,
                },
            ],
            y_sorted_indices: vec![0, 1],
            total_w: 240.0,
            total_h: 1_044.0,
        };
        let root = empty_node(1);
        let child = empty_node(2);
        let nodes = vec![&root, &child];
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let projection = plain_projection(["", ""]);

        let render_theme = render_theme(&theme);
        render(
            &mut dl,
            &layout,
            resolve_viewport(CanvasViewportInput::positioned(
                Rect::new(0.0, 450.0, 400.0, 100.0),
                Rect::new(0.0, 0.0, 400.0, 1_044.0),
                CanvasViewPosition { zoom: 1.0, scroll: CanvasPoint::new(0.0, 450.0) },
                CanvasViewportConfig {
                    base_content_padding: 0.0,
                    min_screen_padding: 0.0,
                    min_initial_fit_zoom: 1.0,
                },
            )),
            &render_theme,
            &constants,
            &mut shaper,
            &nodes,
            None,
            &projection,
            None,
        );

        assert!(
            !dl.cmds.iter().any(|command| matches!(command, DrawCmd::StrokeRect { .. })),
            "both endpoint cards should remain outside the card viewport"
        );
        assert_eq!(
            tapered_mesh_commands(&dl).len(),
            1,
            "connector crossing the viewport should be rendered"
        );
    }

    #[test]
    fn connector_emits_one_length_independent_tapered_mesh_command() {
        let connector_node = |connector_to: (f32, f32), turn_x: f32| LayoutNode {
            x: connector_to.0,
            y: connector_to.1 - 22.0,
            w: 80.0,
            h: 44.0,
            node_idx: 1,
            source_node_index: 1,
            depth: 1,
            connector_from: (0.0, 0.0),
            connector_to,
            connector_turn_x: Some(turn_x),
            branch_index: None,
        };
        let mut short = DrawList::new();
        let mut long = DrawList::new();
        let short_node = connector_node((120.0, 60.0), 60.0);
        let long_node = connector_node((1_200.0, 60.0), 600.0);
        let viewport =
            test_viewport(Rect::new(0.0, 0.0, 2_000.0, 200.0), Rect::new(0.0, 0.0, 2_000.0, 200.0));

        draw_connector(&mut short, &short_node, [1.0; 4], CONNECTOR_REFERENCE_WIDTH_DP, viewport);
        draw_connector(&mut long, &long_node, [1.0; 4], CONNECTOR_REFERENCE_WIDTH_DP, viewport);

        let short_meshes = tapered_mesh_commands(&short);
        let long_meshes = tapered_mesh_commands(&long);
        assert_eq!(short_meshes.len(), 1);
        assert_eq!(long_meshes.len(), 1);
        assert_eq!(short_meshes[0].vertices.len(), long_meshes[0].vertices.len());
        assert!(!short.cmds.iter().any(|command| matches!(command, DrawCmd::FillRect { .. })));
    }

    #[test]
    fn sibling_connectors_draw_independent_single_meshes() {
        let parent_joint = (0.0, 100.0);
        let layout = LayoutTree {
            nodes: vec![
                LayoutNode {
                    x: 200.0,
                    y: 28.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 1,
                    source_node_index: 1,
                    depth: 1,
                    connector_from: parent_joint,
                    connector_to: (200.0, 50.0),
                    connector_turn_x: Some(100.0),
                    branch_index: None,
                },
                LayoutNode {
                    x: 200.0,
                    y: 78.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 2,
                    source_node_index: 2,
                    depth: 1,
                    connector_from: parent_joint,
                    connector_to: (200.0, 100.0),
                    connector_turn_x: Some(100.0),
                    branch_index: None,
                },
                LayoutNode {
                    x: 200.0,
                    y: 128.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 3,
                    source_node_index: 3,
                    depth: 1,
                    connector_from: parent_joint,
                    connector_to: (200.0, 150.0),
                    connector_turn_x: Some(100.0),
                    branch_index: None,
                },
            ],
            y_sorted_indices: vec![0, 1, 2],
            total_w: 280.0,
            total_h: 172.0,
        };
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let mut draw_list = DrawList::new();

        let render_theme = render_theme(&theme);
        render_connectors(
            &mut draw_list,
            &layout,
            Rect::new(0.0, 0.0, 300.0, 200.0),
            &render_theme,
            &constants,
            None,
            test_viewport(Rect::new(0.0, 0.0, 300.0, 200.0), Rect::new(0.0, 0.0, 300.0, 200.0)),
            None,
        );

        let connector_meshes = tapered_mesh_commands(&draw_list);
        assert_eq!(connector_meshes.len(), layout.nodes.len());
        assert_eq!(connector_meshes.len(), 3);
        assert!(draw_list.cmds.iter().all(|command| {
            !matches!(command, DrawCmd::FillRect { color, .. } if *color == theme.mindmap.canvas.connector)
        }));
    }

    #[test]
    fn connector_uses_yesterdays_rounded_elbows() {
        let parent_connector = (0.0, 0.0);
        let child_connector = (200.0, 100.0);
        let points = connector_centerline(
            parent_connector,
            child_connector,
            100.0,
            CONNECTOR_REFERENCE_WIDTH_DP,
        );
        let expected_corner_radius = 24.0;

        assert_eq!(points.first().copied(), Some(parent_connector));
        assert_eq!(points[1], (100.0 - expected_corner_radius, 0.0));
        assert_eq!(points[9], (100.0, expected_corner_radius));
        assert_eq!(points[10], (100.0, 100.0 - expected_corner_radius));
        assert_eq!(points[18], (100.0 + expected_corner_radius, 100.0));
        assert_eq!(points.last().copied(), Some(child_connector));
    }

    #[test]
    fn connector_centerline_uses_the_supplied_turn_axis() {
        let turn_x = 40.0;
        let points = connector_centerline((0.0, 0.0), (120.0, 120.0), turn_x, 8.0);

        assert!(points.windows(2).any(|segment| {
            (segment[0].0 - turn_x).abs() < ZERO_DISTANCE_EPSILON
                && (segment[1].0 - turn_x).abs() < ZERO_DISTANCE_EPSILON
                && (segment[0].1 - segment[1].1).abs() > ZERO_DISTANCE_EPSILON
        }));
    }

    #[test]
    fn connector_head_width_shrinks_by_depth() {
        let expected_head_widths = [(1, 5.0), (2, 3.0), (3, 2.0), (4, 1.0), (5, 1.0)];
        for (depth, expected_width) in expected_head_widths {
            let head_width = connector_head_width(depth, CONNECTOR_REFERENCE_WIDTH_DP);
            assert!(
                (head_width - expected_width).abs() <= 0.1,
                "depth {depth} should start at {expected_width}dp, got {head_width}"
            );
        }
    }
    fn branch_layout_node(depth: u8, branch_index: Option<usize>) -> LayoutNode {
        LayoutNode {
            x: 160.0,
            y: 0.0,
            w: 80.0,
            h: 44.0,
            node_idx: 1,
            source_node_index: 1,
            depth,
            connector_from: (80.0, 22.0),
            connector_to: (160.0, 22.0),
            connector_turn_x: Some(120.0),
            branch_index,
        }
    }

    #[test]
    fn branch_color_tints_default_style_node_colors() {
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let node = Node { title: "branch".into(), ..empty_node(2) };
        let layout_node = branch_layout_node(1, Some(1));

        let render_theme = render_theme(&theme);
        let style = get_node_style(&node, &layout_node, &render_theme);

        let expected = theme.mindmap.canvas.branch_color(1).expect("palette is non-empty");
        assert_eq!(style.border, expected);
        assert_eq!(style.text, expected);
        assert_eq!(style.accent, expected);
        // 背景为分支色的低透明度淡色叠加
        assert_eq!(style.fill, with_alpha(expected, BRANCH_TINT_FILL_ALPHA));
    }

    #[test]
    fn root_is_not_branch_tinted() {
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let node = Node { title: "root".into(), ..empty_node(1) };
        let layout_node = branch_layout_node(0, None);

        let render_theme = render_theme(&theme);
        let style = get_node_style(&node, &layout_node, &render_theme);

        assert_eq!(style.border, theme.mindmap.node.root.border);
        assert_eq!(style.accent, theme.mindmap.node.root.accent);
    }

    #[test]
    fn named_color_overrides_branch_tint() {
        const NAMED_BORDER: [f32; 4] = [0.1, 0.2, 0.3, 1.0];

        let mut theme = Theme::from_definition(&ThemeDefinition::default_dark());
        theme.mindmap.semantic.named.insert(
            "sky".into(),
            ui::theme::MindmapNodeStyle {
                fill: [0.0, 0.0, 0.0, 1.0],
                border: NAMED_BORDER,
                text: [0.9, 0.9, 0.9, 1.0],
                accent: NAMED_BORDER,
            },
        );
        let mut node = Node { title: "branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: None,
            owner: None,
            collapsed: false,
            tags: Vec::new(),
            color: Some("sky".into()),
        });
        let layout_node = branch_layout_node(1, Some(0));

        let render_theme = render_theme(&theme);
        let style = get_node_style(&node, &layout_node, &render_theme);

        assert_eq!(style.border, NAMED_BORDER);
    }

    #[test]
    fn status_overrides_branch_tint() {
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let mut node = Node { title: "branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: None,
            status: Some("todo".into()),
            owner: None,
            collapsed: false,
            tags: Vec::new(),
            color: None,
        });
        let layout_node = branch_layout_node(1, Some(0));

        let render_theme = render_theme(&theme);
        let style = get_node_style(&node, &layout_node, &render_theme);

        assert_eq!(style.border, theme.mindmap.semantic.status.todo.border);
        assert_eq!(style.fill, theme.mindmap.semantic.status.todo.fill);
    }

    #[test]
    fn priority_overrides_branch_tint() {
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let mut node = Node { title: "branch".into(), ..empty_node(2) };
        node.props = Some(super::super::model::NodeProps {
            id: None,
            priority: Some("p0".into()),
            status: None,
            owner: None,
            collapsed: false,
            tags: Vec::new(),
            color: None,
        });
        let layout_node = branch_layout_node(1, Some(0));

        let render_theme = render_theme(&theme);
        let style = get_node_style(&node, &layout_node, &render_theme);

        assert_eq!(style.accent, theme.mindmap.semantic.priority.p0.accent);
        assert_eq!(style.border, theme.mindmap.semantic.priority.p0.border);
        let branch_color = theme.mindmap.canvas.branch_color(0).expect("palette is non-empty");
        assert_ne!(style.border, branch_color, "priority 覆盖必须压制分支染色");
    }

    #[test]
    fn connectors_use_branch_color_with_default_fallback() {
        let parent_joint = (0.0, 100.0);
        let layout = LayoutTree {
            nodes: vec![
                LayoutNode {
                    x: 200.0,
                    y: 28.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 1,
                    source_node_index: 1,
                    depth: 1,
                    connector_from: parent_joint,
                    connector_to: (200.0, 50.0),
                    connector_turn_x: Some(100.0),
                    branch_index: Some(0),
                },
                LayoutNode {
                    x: 200.0,
                    y: 78.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 2,
                    source_node_index: 2,
                    depth: 1,
                    connector_from: parent_joint,
                    connector_to: (200.0, 100.0),
                    connector_turn_x: Some(100.0),
                    branch_index: Some(1),
                },
                LayoutNode {
                    x: 200.0,
                    y: 128.0,
                    w: 80.0,
                    h: 44.0,
                    node_idx: 3,
                    source_node_index: 3,
                    depth: 1,
                    connector_from: parent_joint,
                    connector_to: (200.0, 150.0),
                    connector_turn_x: Some(100.0),
                    branch_index: None,
                },
            ],
            y_sorted_indices: vec![0, 1, 2],
            total_w: 280.0,
            total_h: 172.0,
        };
        let constants = LayoutConstants::default();
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let mut draw_list = DrawList::new();

        let render_theme = render_theme(&theme);
        render_connectors(
            &mut draw_list,
            &layout,
            Rect::new(0.0, 0.0, 300.0, 200.0),
            &render_theme,
            &constants,
            None,
            test_viewport(Rect::new(0.0, 0.0, 300.0, 200.0), Rect::new(0.0, 0.0, 300.0, 200.0)),
            None,
        );

        for branch_index in [0, 1] {
            let branch_color =
                theme.mindmap.canvas.branch_color(branch_index).expect("palette is non-empty");
            assert!(
                draw_list.cmds.iter().any(|command| matches!(
                    command,
                    DrawCmd::TaperedMesh { color, .. } if *color == branch_color
                )),
                "branch {branch_index} connector should use its palette color"
            );
        }
        assert!(
            draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::TaperedMesh { color, .. } if *color == theme.mindmap.canvas.connector
            )),
            "branch-less connector should fall back to the default connector color"
        );
    }
}
