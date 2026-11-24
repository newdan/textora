use super::*;
use crate::mmf::layout::SourceNodeIdx;
use ui::canvas::{CanvasViewPosition, CanvasViewportConfig, CanvasViewportInput, resolve_viewport};
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
            node_idx: VisibleNodeIdx(0),
            source_node_index: SourceNodeIdx(0),
            depth: 0,
            connector_from: (10.0, 42.0),
            connector_to: (10.0, 42.0),
            connector_turn_x: None,
            branch_index: None,
        }],
        y_sorted_indices: vec![VisibleNodeIdx(0)],
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

fn plain_projection<'a>(titles: impl IntoIterator<Item = &'a str>) -> MindmapRenderProjection<'a> {
    MindmapRenderProjection {
        focus: MindmapFocus::None,
        projected_titles: titles.into_iter().map(Cow::Borrowed).collect(),
        preedit_text: "",
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
        let subpixel_x = render::subpixel_offset(subpixel_phase);
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
    let nodes = [&node];

    let render_theme = render_theme(&theme);
    render_cards(
        &mut dl,
        &layout,
        &[VisibleNodeIdx(0)],
        &render_theme,
        &constants,
        &|index| nodes.get(index).copied(),
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
    let nodes = [&node];
    let viewport =
        test_viewport(Rect::new(0.0, 0.0, 400.0, 200.0), Rect::new(0.0, 0.0, 400.0, 200.0));

    let mut default_draw_list = DrawList::new();
    let render_theme = render_theme(&theme);
    render_cards(
        &mut default_draw_list,
        &layout,
        &[VisibleNodeIdx(0)],
        &render_theme,
        &constants,
        &|index| nodes.get(index).copied(),
        None,
        viewport,
    );
    let default_border_width = default_draw_list.cmds.iter().find_map(|command| match command {
        DrawCmd::StrokeRect { line_width, .. } => Some(*line_width),
        _ => None,
    });

    let mut hovered_draw_list = DrawList::new();
    render_cards_with_hover(
        &mut hovered_draw_list,
        &layout,
        &[VisibleNodeIdx(0)],
        &render_theme,
        &constants,
        &|index| nodes.get(index).copied(),
        None,
        viewport,
        None,
        Some(CanvasPoint::new(50.0, 40.0)),
        None,
    );
    let hovered_border_width = hovered_draw_list.cmds.iter().find_map(|command| match command {
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
    let nodes = [&node];
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
        &|index| nodes.get(index).copied(),
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
    let nodes = [&node];
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
        &|index| nodes.get(index).copied(),
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
    node.props = Some(crate::mmf::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = [&node];
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
            &[VisibleNodeIdx(0)],
            &render_theme,
            &constants,
            &mut shaper,
            &|index| nodes.get(index).copied(),
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
    node.props = Some(crate::mmf::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
    node.props = Some(crate::mmf::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
    node.props = Some(crate::mmf::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
            node_idx: VisibleNodeIdx(0),
            source_node_index: SourceNodeIdx(0),
            depth: 1,
            connector_from: (90.0, 42.0),
            connector_to: (10.0, 42.0),
            connector_turn_x: Some(50.0),
            branch_index: Some(0),
        }],
        y_sorted_indices: vec![VisibleNodeIdx(0)],
        total_w: 90.0,
        total_h: 64.0,
    };
    let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
    let mut node = Node { title: "Branch".into(), ..empty_node(2) };
    node.props = Some(crate::mmf::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
    node.props = Some(crate::mmf::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
            node_idx: VisibleNodeIdx(0),
            source_node_index: SourceNodeIdx(0),
            depth: 1,
            connector_from: (90.0, 42.0),
            connector_to: (10.0, 42.0),
            connector_turn_x: Some(50.0),
            branch_index: Some(0),
        }],
        y_sorted_indices: vec![VisibleNodeIdx(0)],
        total_w: 90.0,
        total_h: 64.0,
    };
    let control_bounds = Rect::new(90.0, 20.0, 36.0, 36.0);
    let mut node = Node { title: "Branch".into(), ..empty_node(2) };
    node.props = Some(crate::mmf::model::NodeProps {
        id: None,
        priority: None,
        status: None,
        owner: None,
        collapsed: true,
        tags: Vec::new(),
        color: None,
    });
    let nodes = [&node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
                node_idx: VisibleNodeIdx(0),
                source_node_index: SourceNodeIdx(0),
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
                node_idx: VisibleNodeIdx(1),
                source_node_index: SourceNodeIdx(1),
                depth: 1,
                connector_from: (90.0, 42.0),
                connector_to: (150.0, 42.0),
                connector_turn_x: Some(120.0),
                branch_index: None,
            },
        ],
        y_sorted_indices: vec![VisibleNodeIdx(0), VisibleNodeIdx(1)],
        total_w: 230.0,
        total_h: 64.0,
    };
    let first_node = Node { title: "First".into(), children: vec![empty_node(3)], ..empty_node(1) };
    let second_node =
        Node { title: "Second".into(), children: vec![empty_node(3)], ..empty_node(2) };
    let nodes = [&first_node, &second_node];
    let hit_map = HitMap {
        nodes: Vec::new(),
        controls: vec![
            crate::mmf::layout::ControlHitGeometry {
                source_node_index: 0,
                bounds: Rect::new(90.0, 20.0, 24.0, 24.0),
            },
            crate::mmf::layout::ControlHitGeometry {
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
        &|index| nodes.get(index).copied(),
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
                node_idx: VisibleNodeIdx(0),
                source_node_index: SourceNodeIdx(0),
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
                node_idx: VisibleNodeIdx(1),
                source_node_index: SourceNodeIdx(1),
                depth: 0,
                connector_from: (80.0, 22.0),
                connector_to: (160.0, 22.0),
                connector_turn_x: None,
                branch_index: None,
            },
        ],
        y_sorted_indices: vec![VisibleNodeIdx(0), VisibleNodeIdx(1)],
        total_w: 240.0,
        total_h: 44.0,
    };
    let root = empty_node(1);
    let child = empty_node(2);
    let nodes = [&root, &child];
    let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
    let projection = MindmapRenderProjection {
        focus: MindmapFocus::NodeSelected { node_index: 1 },
        projected_titles: vec![Cow::Borrowed(""), Cow::Borrowed("")],
        preedit_text: "",
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
        &|index| nodes.get(index).copied(),
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
    assert!(highlight_ring_widths.iter().any(|width| {
        (*width - theme.mindmap.geometry.selection_outline_width).abs() < f32::EPSILON
    }));
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
                node_idx: VisibleNodeIdx(0),
                source_node_index: SourceNodeIdx(0),
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
                node_idx: VisibleNodeIdx(1),
                source_node_index: SourceNodeIdx(1),
                depth: 1,
                connector_from: (80.0, 22.0),
                connector_to: (160.0, 22.0),
                connector_turn_x: Some(120.0),
                branch_index: None,
            },
        ],
        y_sorted_indices: vec![VisibleNodeIdx(0), VisibleNodeIdx(1)],
        total_w: 240.0,
        total_h: 44.0,
    };
    let root = empty_node(1);
    let child = empty_node(2);
    let nodes = [&root, &child];
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
        &|index| nodes.get(index).copied(),
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
        &|index| nodes.get(index).copied(),
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
        &|index| nodes.get(index).copied(),
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
                node_idx: VisibleNodeIdx(0),
                source_node_index: SourceNodeIdx(0),
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
                node_idx: VisibleNodeIdx(1),
                source_node_index: SourceNodeIdx(1),
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
                node_idx: VisibleNodeIdx(2),
                source_node_index: SourceNodeIdx(2),
                depth: 2,
                connector_from: (240.0, 22.0),
                connector_to: (320.0, 22.0),
                connector_turn_x: Some(280.0),
                branch_index: None,
            },
        ],
        y_sorted_indices: vec![VisibleNodeIdx(0), VisibleNodeIdx(1), VisibleNodeIdx(2)],
        total_w: 400.0,
        total_h: 44.0,
    };
    let root = Node { title: "root".into(), ..empty_node(1) };
    let source = Node { title: "source".into(), ..empty_node(2) };
    let descendant = Node { title: "descendant".into(), ..empty_node(3) };
    let nodes = [&root, &source, &descendant];
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
        &|index| nodes.get(index).copied(),
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
                node_idx: VisibleNodeIdx(0),
                source_node_index: SourceNodeIdx(0),
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
                node_idx: VisibleNodeIdx(1),
                source_node_index: SourceNodeIdx(1),
                depth: 1,
                connector_from: (80.0, 22.0),
                connector_to: (160.0, 1_022.0),
                connector_turn_x: Some(120.0),
                branch_index: None,
            },
        ],
        y_sorted_indices: vec![VisibleNodeIdx(0), VisibleNodeIdx(1)],
        total_w: 240.0,
        total_h: 1_044.0,
    };
    let root = empty_node(1);
    let child = empty_node(2);
    let nodes = [&root, &child];
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
        &|index| nodes.get(index).copied(),
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
        node_idx: VisibleNodeIdx(1),
        source_node_index: SourceNodeIdx(1),
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
                node_idx: VisibleNodeIdx(1),
                source_node_index: SourceNodeIdx(1),
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
                node_idx: VisibleNodeIdx(2),
                source_node_index: SourceNodeIdx(2),
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
                node_idx: VisibleNodeIdx(3),
                source_node_index: SourceNodeIdx(3),
                depth: 1,
                connector_from: parent_joint,
                connector_to: (200.0, 150.0),
                connector_turn_x: Some(100.0),
                branch_index: None,
            },
        ],
        y_sorted_indices: vec![VisibleNodeIdx(0), VisibleNodeIdx(1), VisibleNodeIdx(2)],
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
        node_idx: VisibleNodeIdx(1),
        source_node_index: SourceNodeIdx(1),
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
    node.props = Some(crate::mmf::model::NodeProps {
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
    node.props = Some(crate::mmf::model::NodeProps {
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
    node.props = Some(crate::mmf::model::NodeProps {
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
                node_idx: VisibleNodeIdx(1),
                source_node_index: SourceNodeIdx(1),
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
                node_idx: VisibleNodeIdx(2),
                source_node_index: SourceNodeIdx(2),
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
                node_idx: VisibleNodeIdx(3),
                source_node_index: SourceNodeIdx(3),
                depth: 1,
                connector_from: parent_joint,
                connector_to: (200.0, 150.0),
                connector_turn_x: Some(100.0),
                branch_index: None,
            },
        ],
        y_sorted_indices: vec![VisibleNodeIdx(0), VisibleNodeIdx(1), VisibleNodeIdx(2)],
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
