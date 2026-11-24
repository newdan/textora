use super::*;

pub(in crate::mmf::canvas) fn connector_intersects_viewport(
    layout_node: &LayoutNode,
    viewport: Rect,
) -> bool {
    let connector_top = layout_node.connector_from.1.min(layout_node.connector_to.1);
    let connector_bottom = layout_node.connector_from.1.max(layout_node.connector_to.1);
    let viewport_bottom = viewport.y + viewport.h;

    connector_bottom >= viewport.y && connector_top <= viewport_bottom
}

pub(in crate::mmf::canvas) fn draw_connector(
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

pub(in crate::mmf::canvas) fn connector_head_width(depth: u8, reference_width: f32) -> f32 {
    let width_scale = reference_width / CONNECTOR_REFERENCE_WIDTH_DP;
    let head_width = match depth {
        1 => FIRST_LEVEL_CONNECTOR_HEAD_WIDTH_DP,
        2 => SECOND_LEVEL_CONNECTOR_HEAD_WIDTH_DP,
        3 => THIRD_LEVEL_CONNECTOR_HEAD_WIDTH_DP,
        _ => CONNECTOR_TAIL_WIDTH_DP,
    };
    head_width * width_scale
}

pub(in crate::mmf::canvas) fn connector_tail_width(reference_width: f32) -> f32 {
    CONNECTOR_TAIL_WIDTH_DP * reference_width / CONNECTOR_REFERENCE_WIDTH_DP
}

pub(in crate::mmf::canvas) fn connector_centerline(
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

pub(in crate::mmf::canvas) fn append_arc(
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

pub(in crate::mmf::canvas) fn tapered_connector_mesh(
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

pub(in crate::mmf::canvas) fn push_point(points: &mut Vec<(f32, f32)>, point: (f32, f32)) {
    if points.last().is_some_and(|last| distance(*last, point) <= ZERO_DISTANCE_EPSILON) {
        return;
    }
    points.push(point);
}

pub(in crate::mmf::canvas) fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_text(
    dl: &mut DrawList,
    layout: &LayoutTree,
    visible_node_indices: &[VisibleNodeIdx],
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    node_at: &NodeLookup<'_>,
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
) {
    let font_family = shaper.font_family().map(str::to_owned);
    let font_weight = shaper.font_weight();
    let font_style = shaper.font_style();
    for &i in visible_node_indices {
        let ln = &layout.nodes[i.0];
        let Some(node) = node_at(ln.source_node_index.0) else {
            continue;
        };
        let title = projection.projected_title(ln.source_node_index.0);
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

pub(in crate::mmf::canvas) fn measure_text(text: &str, shaper: &mut shaping::Shaper) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    shaper.shape(text).map(|run| run.width).unwrap_or(text.len() as f32 * shaper.font_size() * 0.5)
}

pub(in crate::mmf::canvas) fn placeholder_color(mut color: [f32; 4]) -> [f32; 4] {
    color[3] *= PLACEHOLDER_ALPHA_MULTIPLIER;
    color
}

pub(in crate::mmf::canvas) fn with_alpha(color: [f32; 4], alpha: f32) -> [f32; 4] {
    [color[0], color[1], color[2], color[3] * alpha]
}

pub(in crate::mmf::canvas) fn source_subtree_opacity(
    preview: Option<&CanvasDragPreview>,
    rect: Rect,
    theme: &MindmapRenderTheme<'_>,
) -> f32 {
    preview
        .filter(|preview| is_source_subtree_node(preview, rect))
        .map(|_| theme.geometry.drag_source_alpha)
        .unwrap_or(1.0)
}

pub(in crate::mmf::canvas) fn is_source_subtree_node(
    preview: &CanvasDragPreview,
    rect: Rect,
) -> bool {
    rect == preview.source_rect || preview.source_subtree_rects.contains(&rect)
}

pub(in crate::mmf::canvas) fn render_node_selection(
    dl: &mut DrawList,
    layout: &LayoutTree,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    node_at: &NodeLookup<'_>,
    projection: &MindmapRenderProjection<'_>,
    viewport: CanvasViewportSnapshot,
) {
    let Some(node_index) = projection.selected_node_index() else {
        return;
    };
    let Some(layout_node) = layout_node_for_source(layout, node_index) else {
        return;
    };
    let Some(source_node) = node_at(layout_node.source_node_index.0) else {
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
#[allow(clippy::too_many_arguments)]
pub(in crate::mmf::canvas) fn render_drag_preview(
    dl: &mut DrawList,
    preview: &CanvasDragPreview,
    layout: &LayoutTree,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    node_at: &NodeLookup<'_>,
    viewport: CanvasViewportSnapshot,
) {
    let preview_rect = viewport.content_rect_to_screen(preview.preview_rect);
    let source_layout_node =
        layout.nodes.iter().find(|node| layout_rect(node) == preview.source_rect);
    let (fill, border) = if preview.is_valid {
        let style = source_layout_node
            .and_then(|layout_node| {
                node_at(layout_node.source_node_index.0)
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

pub(in crate::mmf::canvas) fn render_drag_target_feedback(
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

pub(in crate::mmf::canvas) fn drag_guide_head_width(
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

pub(in crate::mmf::canvas) fn layout_rect(node: &LayoutNode) -> Rect {
    Rect::new(node.x, node.y, node.w, node.h)
}

pub(in crate::mmf::canvas) fn layout_node_for_source(
    layout: &LayoutTree,
    source_node_index: usize,
) -> Option<&LayoutNode> {
    layout.nodes.iter().find(|node| node.source_node_index.0 == source_node_index)
}

pub(in crate::mmf::canvas) fn hit_geometry_for_source(
    hit_map: &HitMap,
    source_node_index: usize,
) -> Option<&crate::mmf::layout::NodeHitGeometry> {
    hit_map.nodes.iter().find(|geometry| geometry.source_node_index == source_node_index)
}

pub(in crate::mmf::canvas) fn render_title_selection(
    dl: &mut DrawList,
    layout: &LayoutTree,
    hit_map: &HitMap,
    visible_node_indices: &[VisibleNodeIdx],
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
            .any(|index| layout.nodes[index.0].source_node_index.0 == node_index)
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

pub(in crate::mmf::canvas) fn render_preedit_underline(
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

pub(in crate::mmf::canvas) fn render_caret(
    dl: &mut DrawList,
    layout: &LayoutTree,
    hit_map: &HitMap,
    theme: &MindmapRenderTheme<'_>,
    node_at: &NodeLookup<'_>,
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
    let Some(layout_node) = layout.nodes.iter().find(|node| node.source_node_index.0 == node_index)
    else {
        return;
    };
    let Some(node) = node_at(node_index) else {
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

pub(in crate::mmf::canvas) fn grapheme_edge_at_or_before(
    boundaries: &[usize],
    byte_offset: usize,
) -> usize {
    match boundaries.binary_search(&byte_offset) {
        Ok(index) => index,
        Err(next_index) => next_index.saturating_sub(1),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    dl: &mut DrawList,
    layout: &LayoutTree,
    viewport: CanvasViewportSnapshot,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    node_at: &NodeLookup<'_>,
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
        node_at,
        hit_map,
        projection,
        connector_mesh_cache,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_cards_and_connectors(
    dl: &mut DrawList,
    layout: &LayoutTree,
    viewport: CanvasViewportSnapshot,
    theme: &MindmapRenderTheme<'_>,
    constants: &LayoutConstants,
    shaper: &mut shaping::Shaper,
    node_at: &NodeLookup<'_>,
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
        node_at,
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
            node_at,
            theme,
            shaper,
            projection,
            viewport,
            projection.drag_preview.is_some(),
        );
    }
    render_node_selection(dl, layout, theme, constants, node_at, projection, viewport);
    if let Some(preview) = projection.drag_preview {
        render_drag_preview(dl, preview, layout, theme, constants, shaper, node_at, viewport);
        render_drag_target_feedback(dl, preview, layout, theme, constants, viewport);
    }
    if let Some(hit_map) = hit_map {
        render_title_selection(dl, layout, hit_map, &visible, theme, projection, viewport);
    }
    render_text(dl, layout, &visible, theme, constants, shaper, node_at, projection, viewport);
    if let Some(hit_map) = hit_map {
        render_preedit_underline(dl, hit_map, theme, projection, viewport);
        render_caret(dl, layout, hit_map, theme, node_at, projection, viewport);
    }
}
