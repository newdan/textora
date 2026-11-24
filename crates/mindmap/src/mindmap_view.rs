pub(in crate::mindmap_view) use crate::mmf::canvas::MindmapRenderProjection;
pub(in crate::mindmap_view) use crate::mmf::layout::{
    EXPANDED_CONTROL_RIGHT_OFFSET_DP, HitMap, LayoutConstants, LayoutTree, ProjectedTitle,
    measured_card_width_for_depth,
};
pub(in crate::mindmap_view) use crate::mmf::model::MindmapFocus;
pub(in crate::mindmap_view) use crate::mmf::{self, MmfDiagnostic, Tree};
pub(in crate::mindmap_view) use shaping::Shaper;
pub(in crate::mindmap_view) use std::borrow::Cow;
pub(in crate::mindmap_view) use std::ops::Range;
pub(in crate::mindmap_view) use std::path::Path;
pub(in crate::mindmap_view) use textora_core::document::DocView;
pub(in crate::mindmap_view) use ui::canvas::{
    CanvasPoint, CanvasViewportConfig, CanvasViewportInput, CanvasViewportSnapshot,
    resolve_viewport,
};
pub(in crate::mindmap_view) use ui::core::geom::Rect;
pub(in crate::mindmap_view) use ui::core::paint::DrawList;
pub(in crate::mindmap_view) use ui::core::widget::{KeyCode, Modifiers};
pub(in crate::mindmap_view) use ui::plugin::{
    CanvasContentMetrics, CanvasDragPhase, CanvasDragPreview, CanvasDragRequest,
    CanvasDragResponse, CanvasView, EditHitTarget, EditIntent, EditPlan, EditPolicy, EditRequest,
    KeyIntentMapper, MoveDirection, PluginFactory, ViewPlugin, ViewPluginCapabilities,
};
pub(in crate::mindmap_view) use ui::theme::{
    DEFAULT_MINDMAP_COLOR_SCHEME_ID, MindmapRenderTheme, Theme, find_mindmap_color_scheme,
};

use ui::inspector_panel::{
    GroupId, InspectorItemTarget, InspectorPanelDescriptor, InspectorPresetGroup,
    InspectorPresetItem, ItemId, PanelId,
};
use ui::plugin::InspectorPanelProvider;
use ui::theme::built_in_mindmap_color_schemes;

const APPEARANCE_PANEL_ID: &str = "mindmap.appearance";
const COLOR_PRESETS_GROUP_ID: &str = "color_presets";

pub(in crate::mindmap_view) fn mindmap_render_theme<'a>(
    theme: &'a Theme,
    scheme_id: Option<&'a str>,
) -> MindmapRenderTheme<'a> {
    let scheme = scheme_id
        .and_then(find_mindmap_color_scheme)
        .or_else(|| find_mindmap_color_scheme(DEFAULT_MINDMAP_COLOR_SCHEME_ID))
        .expect("the default mmap scheme is registered");
    MindmapRenderTheme::new(scheme, &theme.mindmap.geometry)
}

pub(in crate::mindmap_view) enum MindmapDocumentState {
    Uninitialized,
    Ready {
        generation: u32,
        source: String,
        tree: Box<Tree>,
        layout: Option<Box<LayoutTree>>,
        drag_preview_cards: Vec<DragPreviewCard>,
        hit_map: Option<Box<HitMap>>,
        connector_mesh_cache: Option<mmf::canvas::ConnectorMeshCache>,
    },
    Invalid {
        generation: u32,
        diagnostic: MmfDiagnostic,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::mindmap_view) enum MindmapDragState {
    Idle,
    Preview(MindmapDragPreview),
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::mindmap_view) struct MindmapDragPreview {
    pub(in crate::mindmap_view) source_range: Range<usize>,
    pub(in crate::mindmap_view) anchor_range: Option<Range<usize>>,
    pub(in crate::mindmap_view) target: Option<mmf::edit::MoveSubtreeTarget>,
    pub(in crate::mindmap_view) canvas: CanvasDragPreview,
}

pub(in crate::mindmap_view) struct DragPreviewCard {
    width: f32,
    descendant_count: usize,
}

pub struct MindmapView {
    pub(in crate::mindmap_view) document_state: MindmapDocumentState,
    pub(in crate::mindmap_view) cursor_byte: Option<usize>,
    pub(in crate::mindmap_view) selection_anchor_byte: Option<usize>,
    pub(in crate::mindmap_view) selection_cursor_byte: Option<usize>,
    pub(in crate::mindmap_view) preedit: Option<(String, Option<(usize, usize)>)>,
    pub(in crate::mindmap_view) cursor_visible: bool,
    pub(in crate::mindmap_view) clear_focus_at: Option<usize>,
    pub(in crate::mindmap_view) canvas_viewport: Option<CanvasViewportSnapshot>,
    pub(in crate::mindmap_view) canvas_pointer: Option<CanvasPoint>,
    layout_font_key: Option<(u32, u64)>,
    pub(in crate::mindmap_view) cached_dpi: f32,
    pub(in crate::mindmap_view) constants: LayoutConstants,
    pub(in crate::mindmap_view) same_level_threshold_ratio: f32,
    pub(in crate::mindmap_view) drag_state: MindmapDragState,
}

impl Default for MindmapView {
    fn default() -> Self {
        Self::new()
    }
}

impl MindmapView {
    pub fn new() -> Self {
        Self {
            document_state: MindmapDocumentState::Uninitialized,
            cursor_byte: None,
            selection_anchor_byte: None,
            selection_cursor_byte: None,
            preedit: None,
            cursor_visible: true,
            clear_focus_at: None,
            canvas_viewport: None,
            canvas_pointer: None,
            layout_font_key: None,
            cached_dpi: 0.0,
            constants: LayoutConstants::default(),
            same_level_threshold_ratio: ui::theme::MindmapGeometry::default()
                .same_level_threshold_ratio,
            drag_state: MindmapDragState::Idle,
        }
    }

    pub(in crate::mindmap_view) fn ensure_layout(
        &mut self,
        shaper: &mut Shaper,
        projected_title: Option<ProjectedTitle<'_>>,
    ) {
        let font_key = (shaper.font_size().to_bits(), shaper.attrs_hash());
        if self.layout_font_key != Some(font_key) {
            self.layout_font_key = Some(font_key);
            self.clear_layout();
        }
        let MindmapDocumentState::Ready { tree, layout, drag_preview_cards, hit_map, .. } =
            &mut self.document_state
        else {
            return;
        };
        if layout.is_some() {
            return;
        }

        let computed_layout =
            mmf::layout::compute_layout(tree, shaper, &self.constants, projected_title);
        let computed_hit_map = mmf::layout::build_hit_map(
            tree,
            &computed_layout,
            shaper,
            &self.constants,
            projected_title,
        );
        *drag_preview_cards = measure_drag_preview_cards(
            tree,
            &computed_layout,
            &self.constants,
            shaper,
            projected_title,
        );
        *layout = Some(Box::new(computed_layout));
        *hit_map = Some(Box::new(computed_hit_map));
    }

    pub(in crate::mindmap_view) fn clear_layout(&mut self) {
        if let MindmapDocumentState::Ready {
            layout,
            drag_preview_cards,
            hit_map,
            connector_mesh_cache,
            ..
        } = &mut self.document_state
        {
            *layout = None;
            drag_preview_cards.clear();
            *hit_map = None;
            *connector_mesh_cache = None;
        }
    }

    pub(in crate::mindmap_view) fn ensure_connector_mesh_cache(&mut self, zoom: f32) {
        let constants = &self.constants;
        let MindmapDocumentState::Ready { layout: Some(layout), connector_mesh_cache, .. } =
            &mut self.document_state
        else {
            return;
        };
        if connector_mesh_cache.as_ref().is_some_and(|cache| cache.matches_zoom(zoom)) {
            return;
        }
        *connector_mesh_cache =
            Some(mmf::canvas::ConnectorMeshCache::build(layout, constants, zoom));
    }

    pub(in crate::mindmap_view) fn update_layout_constants(
        &mut self,
        theme: &Theme,
        dpi_scale: f32,
    ) {
        let geometry = &theme.mindmap.geometry;
        self.same_level_threshold_ratio = geometry.same_level_threshold_ratio;
        let next_constants = LayoutConstants {
            card_height: geometry.card_height * dpi_scale,
            card_padding_x: geometry.card_padding_x * dpi_scale,
            card_padding_y: geometry.card_padding_y * dpi_scale,
            root_child_gap: geometry.root_child_gap * dpi_scale,
            nested_child_gap: geometry.nested_child_gap * dpi_scale,
            sibling_gap: geometry.sibling_gap * dpi_scale,
            card_radius: geometry.card_radius * dpi_scale,
            connector_width: geometry.connector_width * dpi_scale,
            expanded_control_right_offset: EXPANDED_CONTROL_RIGHT_OFFSET_DP * dpi_scale,
            depth_font_scales: geometry.depth_font_scales.clone(),
        };
        if self.cached_dpi != dpi_scale || self.constants != next_constants {
            self.cached_dpi = dpi_scale;
            self.constants = next_constants;
            self.clear_layout();
        }
    }

    pub(in crate::mindmap_view) fn current_content_bounds(
        &self,
        selection_outline_gap: f32,
    ) -> Option<Rect> {
        let MindmapDocumentState::Ready { layout: Some(layout), .. } = &self.document_state else {
            return None;
        };
        let mut content_bounds = layout.content_bounds(selection_outline_gap);
        let Some(preview) = self.active_drag_preview() else {
            return Some(content_bounds);
        };
        include_rect(&mut content_bounds, preview.source_rect);
        include_rect(&mut content_bounds, preview.preview_rect);
        if let Some(target_rect) = preview.target_rect {
            include_rect(&mut content_bounds, target_rect);
        }
        include_point(&mut content_bounds, preview.guide_from);
        if let Some(guide_to) = preview.guide_to {
            include_point(&mut content_bounds, guide_to);
        }
        if let Some((from, to)) = preview.insertion_line {
            include_point(&mut content_bounds, from);
            include_point(&mut content_bounds, to);
        }
        Some(content_bounds)
    }

    pub(in crate::mindmap_view) fn focus_anchor(&self) -> Option<CanvasPoint> {
        let node_index = match self.render_focus() {
            MindmapFocus::None => return None,
            MindmapFocus::NodeSelected { node_index }
            | MindmapFocus::TitleEditing { node_index, .. }
            | MindmapFocus::TitleTextSelected { node_index, .. } => node_index,
        };
        let MindmapDocumentState::Ready { layout: Some(layout), .. } = &self.document_state else {
            return None;
        };
        let node = layout_node_for_source(layout, node_index)?;
        Some(CanvasPoint::new(node.x + node.w * 0.5, node.y + node.h * 0.5))
    }

    pub(in crate::mindmap_view) fn render_invalid_or_empty(
        &self,
        bounds: Rect,
        render_theme: &MindmapRenderTheme<'_>,
        shaper: &mut Shaper,
    ) -> DrawList {
        let mut draw_list = DrawList::new();
        draw_list.fill(bounds, render_theme.canvas.background);
        if let MindmapDocumentState::Invalid { diagnostic, .. } = &self.document_state {
            draw_invalid_canvas(&mut draw_list, bounds, render_theme, shaper, diagnostic);
        }
        draw_list
    }

    pub(in crate::mindmap_view) fn clear_layout_for_preedit(&mut self) {
        if self.preedit.is_some() {
            self.clear_layout();
        }
    }

    pub(in crate::mindmap_view) fn needs_source_update(&self, generation: u32) -> bool {
        match &self.document_state {
            MindmapDocumentState::Uninitialized => true,
            MindmapDocumentState::Ready { generation: cached_generation, .. }
            | MindmapDocumentState::Invalid { generation: cached_generation, .. } => {
                *cached_generation != generation
            }
        }
    }

    pub(in crate::mindmap_view) fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor_byte?;
        let cursor = self.selection_cursor_byte?;
        (anchor != cursor).then(|| anchor.min(cursor)..anchor.max(cursor))
    }

    pub(in crate::mindmap_view) fn derive_focus(
        &self,
        cursor_byte: usize,
        selection: Option<Range<usize>>,
    ) -> MindmapFocus {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return MindmapFocus::None;
        };

        if let Some(selection) = selection {
            if let Some(node_index) = tree.subtree_index(&selection) {
                return MindmapFocus::NodeSelected { node_index };
            }
            if let Some(node_index) = tree.nodes().position(|node| {
                selection.start >= node.title_byte_range.start
                    && selection.end <= node.title_byte_range.end
            }) {
                return MindmapFocus::TitleTextSelected { node_index, range: selection };
            }
            return MindmapFocus::None;
        }

        tree.nodes()
            .position(|node| {
                cursor_byte >= node.title_byte_range.start
                    && cursor_byte <= node.title_byte_range.end
            })
            .map(|node_index| MindmapFocus::TitleEditing { node_index, cursor_byte })
            .unwrap_or(MindmapFocus::None)
    }

    pub(in crate::mindmap_view) fn current_focus(&self, cursor_byte: usize) -> MindmapFocus {
        self.derive_focus(cursor_byte, self.selection_range())
    }

    pub(in crate::mindmap_view) fn render_focus(&self) -> MindmapFocus {
        let cursor_byte =
            self.cursor_byte.or_else(|| self.selection_range().map(|range| range.end));
        if cursor_byte == self.clear_focus_at {
            return MindmapFocus::None;
        }
        cursor_byte.map(|cursor_byte| self.current_focus(cursor_byte)).unwrap_or(MindmapFocus::None)
    }

    pub(in crate::mindmap_view) fn active_projected_title(&self) -> Option<(usize, String)> {
        let focus = self.render_focus();
        self.preedit_projection(&focus).map(|(node_index, title, _, _)| (node_index, title))
    }

    pub(in crate::mindmap_view) fn build_render_projection(&self) -> MindmapRenderProjection<'_> {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return MindmapRenderProjection {
                focus: MindmapFocus::None,
                projected_titles: Vec::new(),
                preedit_text: "",
                cursor_visible: self.cursor_visible,
                caret: None,
                composition_caret: None,
                preedit_range: None,
                collapsed_descendant_counts: Vec::new(),
                canvas_pointer: self.canvas_pointer,
                drag_preview: None,
            };
        };
        let focus = self.render_focus();
        let mut projected_titles: Vec<Cow<'_, str>> =
            tree.nodes().map(|node| Cow::Borrowed(node.title.as_str())).collect();
        let preedit = self.preedit.as_ref().filter(|(text, _)| !text.is_empty());
        let mut caret = self.caret_for_focus(&focus);
        let mut composition_caret = None;
        let mut preedit_range = None;
        let collapsed_descendant_counts = tree
            .nodes()
            .enumerate()
            .map(|(node_index, node)| {
                (node_index != 0
                    && node.props.as_ref().is_some_and(|props| props.collapsed)
                    && !node.children.is_empty())
                .then(|| tree.subtree_indices(node_index).map_or(0, |range| range.len() - 1))
            })
            .collect();

        if let Some((node_index, projected_title, range, projected_caret)) =
            self.preedit_projection(&focus)
        {
            projected_titles[node_index] = Cow::Owned(projected_title);
            preedit_range = Some((node_index, range));
            composition_caret = projected_caret.map(|offset| (node_index, offset));
            caret = composition_caret.filter(|_| self.cursor_visible);
        }

        for (node_index, node) in tree.nodes().enumerate() {
            if node.title.is_empty()
                && preedit_range.as_ref().is_none_or(|(active, _)| *active != node_index)
            {
                projected_titles[node_index] = Cow::Borrowed(mmf::canvas::EMPTY_TITLE_PLACEHOLDER);
            }
        }

        MindmapRenderProjection {
            focus,
            projected_titles,
            preedit_text: preedit.map(|(text, _)| text.as_str()).unwrap_or(""),
            cursor_visible: self.cursor_visible,
            caret,
            composition_caret,
            preedit_range,
            collapsed_descendant_counts,
            canvas_pointer: self.canvas_pointer,
            drag_preview: self.active_drag_preview(),
        }
    }

    pub(in crate::mindmap_view) fn active_drag_preview(&self) -> Option<&CanvasDragPreview> {
        match &self.drag_state {
            MindmapDragState::Idle => None,
            MindmapDragState::Preview(preview) => Some(&preview.canvas),
        }
    }

    pub(in crate::mindmap_view) fn caret_for_focus(
        &self,
        focus: &MindmapFocus,
    ) -> Option<(usize, usize)> {
        if !self.cursor_visible || self.preedit.as_ref().is_some_and(|(text, _)| !text.is_empty()) {
            return None;
        }
        let MindmapFocus::TitleEditing { node_index, cursor_byte } = focus else {
            return None;
        };
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return None;
        };
        let node = tree.node(*node_index)?;
        let cursor = cursor_byte.checked_sub(node.title_byte_range.start)?;
        Some((*node_index, clamp_to_char_boundary(&node.title, cursor)))
    }

    pub(in crate::mindmap_view) fn preedit_projection(
        &self,
        focus: &MindmapFocus,
    ) -> Option<(usize, String, Range<usize>, Option<usize>)> {
        let (preedit, preedit_cursor) =
            self.preedit.as_ref().filter(|(text, _)| !text.is_empty())?;
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return None;
        };
        let (node_index, replacement) = match focus {
            MindmapFocus::NodeSelected { node_index } => {
                let node = tree.node(*node_index)?;
                (*node_index, 0..node.title.len())
            }
            MindmapFocus::TitleEditing { node_index, cursor_byte } => {
                let node = tree.node(*node_index)?;
                let cursor = cursor_byte.checked_sub(node.title_byte_range.start)?;
                let cursor = clamp_to_char_boundary(&node.title, cursor);
                (*node_index, cursor..cursor)
            }
            MindmapFocus::TitleTextSelected { node_index, range } => {
                let node = tree.node(*node_index)?;
                let start = clamp_to_char_boundary(
                    &node.title,
                    range.start.saturating_sub(node.title_byte_range.start),
                );
                let end = clamp_to_char_boundary(
                    &node.title,
                    range.end.saturating_sub(node.title_byte_range.start),
                );
                (*node_index, start.min(end)..start.max(end))
            }
            MindmapFocus::None => return None,
        };
        let node = tree.node(node_index)?;
        let mut projected_title = String::with_capacity(node.title.len() + preedit.len());
        projected_title.push_str(&node.title[..replacement.start]);
        projected_title.push_str(preedit);
        projected_title.push_str(&node.title[replacement.end..]);
        let preedit_range = replacement.start..replacement.start + preedit.len();
        let caret = matches!(
            focus,
            MindmapFocus::NodeSelected { .. }
                | MindmapFocus::TitleEditing { .. }
                | MindmapFocus::TitleTextSelected { .. }
        )
        .then(|| {
            let cursor = preedit_cursor.map(|(_, cursor)| cursor).unwrap_or(preedit.len());
            replacement.start + clamp_to_char_boundary(preedit, cursor)
        });
        Some((node_index, projected_title, preedit_range, caret))
    }

    pub(in crate::mindmap_view) fn screen_to_content(&self, x: f32, y: f32) -> Option<CanvasPoint> {
        Some(self.canvas_viewport?.screen_to_content(CanvasPoint::new(x, y)))
    }
}

impl MindmapView {
    #[cfg(test)]
    pub(in crate::mindmap_view) fn ready_tree(&self) -> &Tree {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            panic!("test requires ready mmap state");
        };
        tree
    }

    #[cfg(test)]
    pub(in crate::mindmap_view) fn ready_hit_map(&self) -> &HitMap {
        let MindmapDocumentState::Ready { hit_map: Some(hit_map), .. } = &self.document_state
        else {
            panic!("test requires ready mmap state");
        };
        hit_map
    }
}

pub(in crate::mindmap_view) fn layout_node_for_source(
    layout: &LayoutTree,
    source_node_index: usize,
) -> Option<&mmf::layout::LayoutNode> {
    layout.nodes.iter().find(|node| node.source_node_index.0 == source_node_index)
}

pub(in crate::mindmap_view) fn hit_geometry_for_source(
    hit_map: &HitMap,
    source_node_index: usize,
) -> Option<&mmf::layout::NodeHitGeometry> {
    hit_map.nodes.iter().find(|geometry| geometry.source_node_index == source_node_index)
}

fn measure_drag_preview_cards(
    tree: &Tree,
    layout: &LayoutTree,
    constants: &LayoutConstants,
    shaper: &mut Shaper,
    projected_title: Option<ProjectedTitle<'_>>,
) -> Vec<DragPreviewCard> {
    layout
        .nodes
        .iter()
        .map(|layout_node| {
            let source_index = layout_node.source_node_index.0;
            let node = tree.node(source_index).expect("layout points to a source node");
            let descendants =
                tree.subtree_indices(source_index).expect("layout points to an indexed subtree");
            let descendant_count = descendants.len() - 1;
            let title_is_projected =
                projected_title.is_some_and(|title| title.node_index == source_index);
            let width = if descendant_count > 0 || title_is_projected {
                let label = drag_preview_label(&node.title, descendant_count);
                measured_card_width_for_depth(&label, constants, shaper, layout_node.depth)
                    .max(layout_node.w)
            } else {
                layout_node.w
            };
            DragPreviewCard { width, descendant_count }
        })
        .collect()
}

fn drag_preview_label(title: &str, descendant_count: usize) -> String {
    match descendant_count {
        0 => title.to_owned(),
        count => format!("{title} · {count}"),
    }
}

pub(in crate::mindmap_view) fn visible_source_indices(tree: &Tree) -> impl Iterator<Item = usize> {
    std::iter::successors(Some(0), |&index| {
        let node = tree.node(index)?;
        let next = if index != 0 && node.props.as_ref().is_some_and(|props| props.collapsed) {
            tree.subtree_indices(index)?.end
        } else {
            index + 1
        };
        tree.node(next).map(|_| next)
    })
}

pub(in crate::mindmap_view) fn layout_rect(node: &mmf::layout::LayoutNode) -> Rect {
    Rect::new(node.x, node.y, node.w, node.h)
}

pub(in crate::mindmap_view) fn include_rect(bounds: &mut Rect, rect: Rect) {
    include_point(bounds, (rect.x, rect.y));
    include_point(bounds, (rect.x + rect.w, rect.y + rect.h));
}

pub(in crate::mindmap_view) fn include_point(bounds: &mut Rect, point: (f32, f32)) {
    let min_x = bounds.x.min(point.0);
    let min_y = bounds.y.min(point.1);
    let max_x = (bounds.x + bounds.w).max(point.0);
    let max_y = (bounds.y + bounds.h).max(point.1);
    *bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
}

pub(in crate::mindmap_view) fn clamp_to_char_boundary(text: &str, byte_offset: usize) -> usize {
    let mut boundary = byte_offset.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

pub(in crate::mindmap_view) fn draw_invalid_canvas(
    draw_list: &mut DrawList,
    bounds: Rect,
    render_theme: &MindmapRenderTheme<'_>,
    shaper: &mut Shaper,
    diagnostic: &MmfDiagnostic,
) {
    const MESSAGE_X_OFFSET: f32 = 24.0;
    const MESSAGE_Y_OFFSET: f32 = 48.0;
    const MESSAGE_LINE_HEIGHT: f32 = 28.0;
    const MESSAGE_FONT_SIZE: f32 = 16.0;
    const INSTRUCTION: &str = "使用视图切换按钮进入源码修复";

    let x = bounds.x + MESSAGE_X_OFFSET;
    let first_baseline = bounds.y + MESSAGE_Y_OFFSET;
    let position = format!("{}:{}", diagnostic.line, diagnostic.column);
    draw_list.text_shaped(
        x,
        first_baseline,
        MESSAGE_FONT_SIZE,
        render_theme.canvas.focus_ring,
        &diagnostic.message,
        shaper,
    );
    draw_list.text_shaped(
        x,
        first_baseline + MESSAGE_LINE_HEIGHT,
        MESSAGE_FONT_SIZE,
        render_theme.canvas.focus_ring,
        &position,
        shaper,
    );
    draw_list.text_shaped(
        x,
        first_baseline + 2.0 * MESSAGE_LINE_HEIGHT,
        MESSAGE_FONT_SIZE,
        render_theme.canvas.focus_ring,
        INSTRUCTION,
        shaper,
    );
}

pub(in crate::mindmap_view) fn nearest_grapheme_boundary(edges: &[f32], x: f32) -> usize {
    edges
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| (x - **left).abs().total_cmp(&(x - **right).abs()))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

pub(in crate::mindmap_view) fn grapheme_boundary_at_or_before(
    boundaries: &[usize],
    byte_offset: usize,
) -> usize {
    match boundaries.binary_search(&byte_offset) {
        Ok(index) => index,
        Err(next_index) => next_index.saturating_sub(1),
    }
}

impl EditPolicy for MindmapView {
    fn plan_edit(&self, request: &EditRequest) -> EditPlan {
        let MindmapDocumentState::Ready { generation, source, tree, .. } = &self.document_state
        else {
            return EditPlan::Consume;
        };
        if request.source_generation != *generation {
            return EditPlan::Consume;
        }
        mmf::edit::plan_mindmap_edit(tree, source, request)
    }
}

impl KeyIntentMapper for MindmapView {
    fn map_key(&self, key: &KeyCode, modifiers: &Modifiers) -> Option<EditIntent> {
        let primary = modifiers.cmd || modifiers.ctrl;
        match (key, primary) {
            (KeyCode::Char('['), true) => Some(EditIntent::PromoteObject),
            (KeyCode::Char(']'), true) => Some(EditIntent::DemoteObject),
            (KeyCode::Escape, false) => Some(EditIntent::SelectObject),
            _ => None,
        }
    }
}

impl InspectorPanelProvider for MindmapView {
    fn inspector_descriptor(&self, source_generation: u32) -> Option<InspectorPanelDescriptor> {
        let (selected_item, notice, selection_disabled) = match &self.document_state {
            MindmapDocumentState::Ready { tree, generation, .. }
                if *generation == source_generation =>
            {
                match tree.global_props.get("theme").map(String::as_str) {
                    None => (Some(ItemId(DEFAULT_MINDMAP_COLOR_SCHEME_ID.into())), None, false),
                    Some(id) if find_mindmap_color_scheme(id).is_some() => {
                        (Some(ItemId(id.into())), None, false)
                    }
                    Some(id) => (None, Some(format!("找不到主题：{id}，已使用默认主题")), false),
                }
            }
            MindmapDocumentState::Invalid { generation, .. }
                if *generation == source_generation =>
            {
                (None, Some("请先修复文件元数据".into()), true)
            }
            _ => return None,
        };
        let items = built_in_mindmap_color_schemes()
            .iter()
            .map(|scheme| InspectorPresetItem {
                id: ItemId(scheme.id.into()),
                display_name: scheme.display_name.into(),
                canvas_background: scheme.canvas.background,
                root_fill: scheme.node.root.fill,
                branch_colors: scheme.canvas.branch_palette.clone(),
                disabled: false,
            })
            .collect();
        Some(InspectorPanelDescriptor {
            id: PanelId(APPEARANCE_PANEL_ID.into()),
            title: "风格".into(),
            presets: InspectorPresetGroup {
                id: GroupId(COLOR_PRESETS_GROUP_ID.into()),
                title: "配色方案".into(),
                items,
                selected_item,
                fallback_summary_item: Some(ItemId(DEFAULT_MINDMAP_COLOR_SCHEME_ID.into())),
            },
            notice,
            selection_disabled,
        })
    }

    fn plan_inspector_action(
        &self,
        panel_id: &PanelId,
        target: &InspectorItemTarget,
        source_generation: u32,
    ) -> EditPlan {
        if panel_id.0 != APPEARANCE_PANEL_ID || target.group_id.0 != COLOR_PRESETS_GROUP_ID {
            return EditPlan::Consume;
        }
        self.plan_theme_selection(&target.item_id.0, source_generation)
    }
}

impl MindmapView {
    fn plan_theme_selection(&self, theme_id: &str, source_generation: u32) -> EditPlan {
        let MindmapDocumentState::Ready { generation, source, tree, .. } = &self.document_state
        else {
            return EditPlan::Consume;
        };
        if source_generation != *generation || find_mindmap_color_scheme(theme_id).is_none() {
            return EditPlan::Consume;
        }
        mmf::edit::plan_set_mindmap_theme(
            tree,
            source,
            theme_id,
            source_generation,
            self.cursor_byte.unwrap_or(source.len()),
        )
    }
}

impl ViewPlugin for MindmapView {
    fn name(&self) -> &str {
        "mindmap"
    }

    fn mode(&self) -> ui::plugin::PluginMode {
        ui::plugin::PluginMode::Canvas
    }

    fn render(
        &mut self,
        doc: &dyn DocView,
        bounds: Rect,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let Some(metrics) = self.prepare_canvas(doc, theme, shaper, dpi_scale) else {
            let render_theme = mindmap_render_theme(theme, None);
            return self.render_invalid_or_empty(bounds, &render_theme, shaper);
        };
        let viewport = resolve_viewport(CanvasViewportInput::initial(
            bounds,
            metrics.content_bounds,
            CanvasViewportConfig::for_dpi(dpi_scale),
        ));
        self.render_canvas(doc, &viewport, theme, shaper, dpi_scale)
    }

    fn sync_document(
        &mut self,
        generation: u32,
        reason: ui::plugin::SyncReason,
        load_source: &mut dyn FnMut() -> String,
    ) -> ui::plugin::SyncOutcome {
        if reason == ui::plugin::SyncReason::Normal && !self.needs_source_update(generation) {
            return ui::plugin::SyncOutcome::Unchanged;
        }
        let text = load_source();
        self.clear_focus_at = None;
        self.drag_state = MindmapDragState::Idle;
        self.canvas_viewport = None;
        self.canvas_pointer = None;
        self.document_state = match mmf::parser::parse(&text) {
            Ok(tree) => MindmapDocumentState::Ready {
                generation,
                source: text,
                tree: Box::new(tree),
                layout: None,
                drag_preview_cards: Vec::new(),
                hit_map: None,
                connector_mesh_cache: None,
            },
            Err(diagnostic) => MindmapDocumentState::Invalid { generation, diagnostic },
        };
        ui::plugin::SyncOutcome::Updated
    }
}

impl ui::plugin::ViewPluginCapabilities for MindmapView {
    fn inspector_panel_provider(&self) -> Option<&dyn InspectorPanelProvider> {
        Some(self)
    }

    fn selection_mut(&mut self) -> Option<&mut dyn ui::plugin::SelectionView> {
        Some(self)
    }

    fn needs_cursor_blink_wakeup(&self) -> bool {
        true
    }

    fn allows_editing(&self) -> bool {
        true
    }

    fn editable(&self) -> Option<&dyn ui::plugin::EditableView> {
        Some(self)
    }

    fn editable_mut(&mut self) -> Option<&mut dyn ui::plugin::EditableView> {
        Some(self)
    }

    fn canvas(&self) -> Option<&dyn CanvasView> {
        Some(self)
    }

    fn canvas_mut(&mut self) -> Option<&mut dyn CanvasView> {
        Some(self)
    }
}

impl ui::plugin::EditableView for MindmapView {
    fn hit_test_edit_target(
        &self,
        x: f32,
        y: f32,
        _offset_x: f32,
        _offset_y: f32,
    ) -> Option<Option<ui::plugin::EditHitTarget>> {
        Some(self.semantic_hit_target(x, y))
    }

    fn move_edit_target(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        _target_x: Option<f32>,
    ) -> Option<ui::plugin::EditHitTarget> {
        self.move_edit_target(current_byte, direction)
    }

    fn cursor_screen_rect(&self, cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
        self.cursor_screen_pos(cursor_byte)
    }

    fn set_cursor_byte(&mut self, cursor_byte: usize) {
        if self.clear_focus_at != Some(cursor_byte) {
            self.clear_focus_at = None;
        }
        self.cursor_byte = Some(cursor_byte);
        self.clear_layout_for_preedit();
    }

    fn set_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        let next_preedit = (!text.is_empty()).then_some((text, cursor));
        if self.preedit != next_preedit {
            self.preedit = next_preedit;
            self.clear_layout();
        }
    }

    fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }

    fn clear_edit_focus(&mut self) {
        self.clear_focus_at = self.cursor_byte;
        self.drag_state = MindmapDragState::Idle;
        self.clear_layout_for_preedit();
    }

    fn key_intent_mapper(&self) -> Option<&dyn KeyIntentMapper> {
        Some(self)
    }
}

impl CanvasView for MindmapView {
    fn hit_test_canvas_control(&self, position: CanvasPoint) -> Option<std::ops::Range<usize>> {
        self.canvas_control_source_range(position)
    }

    fn plan_canvas_control(
        &self,
        source_range: std::ops::Range<usize>,
        source_generation: u32,
    ) -> EditPlan {
        let MindmapDocumentState::Ready { generation, source, tree, .. } = &self.document_state
        else {
            return EditPlan::Consume;
        };
        if source_generation != *generation {
            return EditPlan::Consume;
        }
        mmf::edit::plan_toggle_collapsed(tree, source, source_range, source_generation)
    }

    fn set_canvas_pointer(&mut self, pointer: Option<CanvasPoint>) -> bool {
        self.canvas_pointer = pointer;
        true
    }

    fn prepare_canvas(
        &mut self,
        _doc: &dyn DocView,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> Option<CanvasContentMetrics> {
        self.update_layout_constants(theme, dpi_scale);
        let active_title = self.active_projected_title();
        let projected_title = active_title
            .as_ref()
            .map(|(node_index, text)| ProjectedTitle { node_index: *node_index, text });
        self.ensure_layout(shaper, projected_title);
        let content_bounds =
            self.current_content_bounds(theme.mindmap.geometry.selection_outline_gap)?;
        Some(CanvasContentMetrics { content_bounds, focus_anchor: self.focus_anchor() })
    }

    fn render_canvas(
        &mut self,
        _doc: &dyn DocView,
        viewport: &CanvasViewportSnapshot,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        self.update_layout_constants(theme, dpi_scale);
        self.canvas_viewport = Some(*viewport);
        self.ensure_connector_mesh_cache(viewport.zoom);
        let mut draw_list = DrawList::new();

        let file_scheme_id = match &self.document_state {
            MindmapDocumentState::Ready { tree, .. } => {
                tree.global_props.get("theme").map(|s| s.as_str())
            }
            _ => None,
        };
        let render_theme = mindmap_render_theme(theme, file_scheme_id);

        draw_list.fill(viewport.viewport, render_theme.canvas.background);
        let MindmapDocumentState::Invalid { diagnostic, .. } = &self.document_state else {
            let MindmapDocumentState::Ready {
                tree,
                layout: Some(layout),
                hit_map: Some(hit_map),
                connector_mesh_cache,
                ..
            } = &self.document_state
            else {
                return draw_list;
            };
            let projection = self.build_render_projection();
            draw_list.clip(viewport.viewport, |canvas_draw_list| {
                mmf::canvas::render(
                    canvas_draw_list,
                    layout,
                    *viewport,
                    &render_theme,
                    &self.constants,
                    shaper,
                    &|index| tree.node(index),
                    Some(hit_map),
                    &projection,
                    connector_mesh_cache.as_ref(),
                );
            });
            return draw_list;
        };
        draw_invalid_canvas(&mut draw_list, viewport.viewport, &render_theme, shaper, diagnostic);
        draw_list
    }

    fn handle_canvas_drag(
        &mut self,
        request: CanvasDragRequest,
        _doc: &dyn DocView,
    ) -> CanvasDragResponse {
        match request.phase {
            CanvasDragPhase::Cancel => {
                self.drag_state = MindmapDragState::Idle;
                CanvasDragResponse::Ignore
            }
            CanvasDragPhase::Start | CanvasDragPhase::Update => {
                if self.drag_request_hits_title(&request) {
                    self.drag_state = MindmapDragState::Idle;
                    return CanvasDragResponse::Ignore;
                }
                let Some(preview) = self.calculate_drag_preview(&request) else {
                    self.drag_state = MindmapDragState::Idle;
                    return CanvasDragResponse::Ignore;
                };
                let response = CanvasDragResponse::Preview(preview.canvas.clone());
                self.drag_state = MindmapDragState::Preview(preview);
                response
            }
            CanvasDragPhase::Drop => {
                let MindmapDragState::Preview(stored_preview) = &self.drag_state else {
                    return CanvasDragResponse::Ignore;
                };
                if stored_preview.source_range != request.source_range
                    || self.drag_request_hits_title(&request)
                {
                    self.drag_state = MindmapDragState::Idle;
                    return CanvasDragResponse::Ignore;
                }
                let Some(current_preview) = self.calculate_drag_preview(&request) else {
                    self.drag_state = MindmapDragState::Idle;
                    return CanvasDragResponse::Ignore;
                };
                let is_current_candidate = current_preview.canvas.is_valid
                    && current_preview.anchor_range == stored_preview.anchor_range
                    && current_preview.target == stored_preview.target;
                self.drag_state = MindmapDragState::Idle;
                if !is_current_candidate {
                    return CanvasDragResponse::Ignore;
                }
                let (Some(anchor_range), Some(target)) =
                    (current_preview.anchor_range, current_preview.target)
                else {
                    return CanvasDragResponse::Ignore;
                };
                let MindmapDocumentState::Ready { tree, source, .. } = &self.document_state else {
                    return CanvasDragResponse::Ignore;
                };
                match mmf::edit::plan_move_subtree(
                    tree,
                    source,
                    request.source_range,
                    anchor_range,
                    target,
                    request.source_generation,
                ) {
                    EditPlan::Apply(transaction) => CanvasDragResponse::Apply(transaction),
                    // `ApplyDefault` is only produced by the app's default plan
                    // resolver and never by `plan_move_subtree`.
                    EditPlan::ApplyDefault(..)
                    | EditPlan::UseDefault
                    | EditPlan::SetSelection(_)
                    | EditPlan::MoveCursor(_)
                    | EditPlan::Consume => CanvasDragResponse::Ignore,
                }
            }
        }
    }
}

pub struct MindmapPluginFactory;

impl PluginFactory for MindmapPluginFactory {
    fn name(&self) -> &str {
        "mindmap"
    }

    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|path| path.to_str()).is_some_and(|path| path.ends_with(".mmap.md"))
    }

    fn create(&self) -> Box<dyn ViewPluginCapabilities> {
        Box::new(MindmapView::new())
    }
}

#[path = "mindmap_view_interaction.rs"]
mod interaction;
#[allow(unused_imports)]
pub(in crate::mindmap_view) use interaction::*;

#[cfg(test)]
#[path = "mindmap_view_tests.rs"]
mod tests;

impl ui::plugin::SelectionView for MindmapView {
    fn set_selection_cursor(
        &mut self,
        position: Option<ui::plugin::ReadingPosition>,
        document: &dyn DocView,
    ) -> bool {
        ui::plugin::SelectionView::set_selection_cursor_byte(
            self,
            position.map(|position| {
                document.line_byte_offset(position.line_index) + position.grapheme_index
            }),
        )
    }

    fn set_selection_anchor(
        &mut self,
        position: Option<ui::plugin::ReadingPosition>,
        document: &dyn DocView,
    ) -> bool {
        ui::plugin::SelectionView::set_selection_anchor_byte(
            self,
            position.map(|position| {
                document.line_byte_offset(position.line_index) + position.grapheme_index
            }),
        )
    }

    fn set_selection_anchor_byte(&mut self, byte: Option<usize>) -> bool {
        self.selection_anchor_byte = byte;
        self.clear_layout_for_preedit();
        true
    }

    fn set_selection_cursor_byte(&mut self, byte: Option<usize>) -> bool {
        self.selection_cursor_byte = byte;
        self.clear_layout_for_preedit();
        true
    }

    fn clear_selection(&mut self) -> bool {
        self.selection_anchor_byte = None;
        self.selection_cursor_byte = None;
        self.clear_layout_for_preedit();
        true
    }
}
