use crate::mmf::canvas::MindmapRenderProjection;
use crate::mmf::layout::{
    EXPANDED_CONTROL_RIGHT_OFFSET_DP, HitMap, LayoutConstants, LayoutTree, ProjectedTitle,
    descendant_count, measured_card_width,
};
use crate::mmf::utils::{collect_nodes_dfs, find_parent, find_siblings};
use crate::mmf::{self, MmfDiagnostic, Tree};
use core::document::{DocView, DocViewMut};
use shaping::Shaper;
use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;
use ui::canvas::{
    CanvasPoint, CanvasViewportConfig, CanvasViewportInput, CanvasViewportSnapshot,
    resolve_viewport,
};
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::core::widget::{KeyCode, Modifiers};
use ui::plugin::{
    CanvasContentMetrics, CanvasDragPhase, CanvasDragPreview, CanvasDragRequest,
    CanvasDragResponse, EditHitTarget, EditIntent, EditPlan, EditPolicy, EditRequest,
    KeyIntentMapper, MoveDirection, PluginFactory, PluginMessage, PluginQuery, PluginResponse,
    ViewPlugin,
};
use ui::theme::{
    DEFAULT_MINDMAP_COLOR_SCHEME_ID, MindmapRenderTheme, Theme, find_mindmap_color_scheme,
    resolve_mindmap_theme_selection,
};

const PREVIEW_TEXT_FALLBACK_ADVANCE_PER_BYTE: f32 = 7.0;

fn mindmap_render_theme<'a>(
    theme: &'a Theme,
    scheme_id: Option<&'a str>,
) -> MindmapRenderTheme<'a> {
    let scheme = scheme_id
        .and_then(find_mindmap_color_scheme)
        .or_else(|| find_mindmap_color_scheme(DEFAULT_MINDMAP_COLOR_SCHEME_ID))
        .expect("the default mmap scheme is registered");
    MindmapRenderTheme::new(scheme, &theme.mindmap.geometry)
}

enum MindmapDocumentState {
    Uninitialized,
    Ready {
        generation: u32,
        source: String,
        tree: Box<Tree>,
        layout: Option<Box<LayoutTree>>,
        hit_map: Option<Box<HitMap>>,
        connector_mesh_cache: Option<mmf::canvas::ConnectorMeshCache>,
    },
    Invalid {
        generation: u32,
        diagnostic: MmfDiagnostic,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MindmapFocus {
    None,
    NodeSelected { node_index: usize },
    TitleEditing { node_index: usize, cursor_byte: usize },
    TitleTextSelected { node_index: usize, range: Range<usize> },
}

#[derive(Clone, Debug, PartialEq)]
enum MindmapDragState {
    Idle,
    Preview(MindmapDragPreview),
}

#[derive(Clone, Debug, PartialEq)]
struct MindmapDragPreview {
    source_range: Range<usize>,
    anchor_range: Option<Range<usize>>,
    target: Option<mmf::edit::MoveSubtreeTarget>,
    canvas: CanvasDragPreview,
}

struct DragCandidate<'a> {
    node_index: usize,
    node: &'a mmf::Node,
    layout_node: &'a mmf::layout::LayoutNode,
}

pub struct MindmapView {
    document_state: MindmapDocumentState,
    cursor_byte: Option<usize>,
    selection_anchor_byte: Option<usize>,
    selection_cursor_byte: Option<usize>,
    preedit: Option<(String, Option<(usize, usize)>)>,
    cursor_visible: bool,
    clear_focus_at: Option<usize>,
    canvas_viewport: Option<CanvasViewportSnapshot>,
    canvas_pointer: Option<CanvasPoint>,
    cached_font_size: f32,
    cached_dpi: f32,
    constants: LayoutConstants,
    same_level_threshold_ratio: f32,
    drag_state: MindmapDragState,
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
            cached_font_size: 0.0,
            cached_dpi: 0.0,
            constants: LayoutConstants::default(),
            same_level_threshold_ratio: ui::theme::MindmapGeometry::default()
                .same_level_threshold_ratio,
            drag_state: MindmapDragState::Idle,
        }
    }

    fn ensure_layout(&mut self, shaper: &mut Shaper, projected_title: Option<ProjectedTitle<'_>>) {
        self.cached_font_size = shaper.font_size();
        let MindmapDocumentState::Ready { tree, layout, hit_map, .. } = &mut self.document_state
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
        *layout = Some(Box::new(computed_layout));
        *hit_map = Some(Box::new(computed_hit_map));
    }

    fn clear_layout(&mut self) {
        if let MindmapDocumentState::Ready { layout, hit_map, connector_mesh_cache, .. } =
            &mut self.document_state
        {
            *layout = None;
            *hit_map = None;
            *connector_mesh_cache = None;
        }
    }

    fn ensure_connector_mesh_cache(&mut self, zoom: f32) {
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

    fn update_layout_constants(&mut self, theme: &Theme, dpi_scale: f32) {
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

    fn current_content_bounds(&self, selection_outline_gap: f32) -> Option<Rect> {
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

    fn focus_anchor(&self) -> Option<CanvasPoint> {
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

    fn render_invalid_or_empty(
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

    fn clear_layout_for_preedit(&mut self) {
        if self.preedit.is_some() {
            self.clear_layout();
        }
    }

    fn needs_source_update(&self, generation: u32) -> bool {
        match &self.document_state {
            MindmapDocumentState::Uninitialized => true,
            MindmapDocumentState::Ready { generation: cached_generation, .. }
            | MindmapDocumentState::Invalid { generation: cached_generation, .. } => {
                *cached_generation != generation
            }
        }
    }

    fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor_byte?;
        let cursor = self.selection_cursor_byte?;
        (anchor != cursor).then(|| anchor.min(cursor)..anchor.max(cursor))
    }

    fn derive_focus(&self, cursor_byte: usize, selection: Option<Range<usize>>) -> MindmapFocus {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return MindmapFocus::None;
        };
        let nodes = collect_nodes_dfs(&tree.root);

        if let Some(selection) = selection {
            if let Some(node_index) =
                nodes.iter().position(|node| node.subtree_source_range == selection)
            {
                return MindmapFocus::NodeSelected { node_index };
            }
            if let Some(node_index) = nodes.iter().position(|node| {
                selection.start >= node.title_byte_range.start
                    && selection.end <= node.title_byte_range.end
            }) {
                return MindmapFocus::TitleTextSelected { node_index, range: selection };
            }
            return MindmapFocus::None;
        }

        nodes
            .iter()
            .position(|node| {
                cursor_byte >= node.title_byte_range.start
                    && cursor_byte <= node.title_byte_range.end
            })
            .map(|node_index| MindmapFocus::TitleEditing { node_index, cursor_byte })
            .unwrap_or(MindmapFocus::None)
    }

    fn current_focus(&self, cursor_byte: usize) -> MindmapFocus {
        self.derive_focus(cursor_byte, self.selection_range())
    }

    fn render_focus(&self) -> MindmapFocus {
        let cursor_byte =
            self.cursor_byte.or_else(|| self.selection_range().map(|range| range.end));
        if cursor_byte == self.clear_focus_at {
            return MindmapFocus::None;
        }
        cursor_byte.map(|cursor_byte| self.current_focus(cursor_byte)).unwrap_or(MindmapFocus::None)
    }

    fn active_projected_title(&self) -> Option<(usize, String)> {
        let focus = self.render_focus();
        self.preedit_projection(&focus).map(|(node_index, title, _, _)| (node_index, title))
    }

    fn build_render_projection(&self) -> MindmapRenderProjection<'_> {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return MindmapRenderProjection {
                focus: MindmapFocus::None,
                projected_titles: Vec::new(),
                preedit_text: "",
                preedit_cursor: None,
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
        let nodes = collect_nodes_dfs(&tree.root);
        let mut projected_titles: Vec<Cow<'_, str>> =
            nodes.iter().map(|node| Cow::Borrowed(node.title.as_str())).collect();
        let preedit = self.preedit.as_ref().filter(|(text, _)| !text.is_empty());
        let mut caret = self.caret_for_focus(&focus);
        let mut composition_caret = None;
        let mut preedit_range = None;
        let collapsed_descendant_counts = nodes
            .iter()
            .enumerate()
            .map(|(node_index, node)| {
                (node_index != 0
                    && node.props.as_ref().is_some_and(|props| props.collapsed)
                    && !node.children.is_empty())
                .then(|| descendant_count(node))
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

        for (node_index, node) in nodes.iter().enumerate() {
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
            preedit_cursor: preedit.and_then(|(_, cursor)| *cursor),
            cursor_visible: self.cursor_visible,
            caret,
            composition_caret,
            preedit_range,
            collapsed_descendant_counts,
            canvas_pointer: self.canvas_pointer,
            drag_preview: self.active_drag_preview(),
        }
    }

    fn active_drag_preview(&self) -> Option<&CanvasDragPreview> {
        match &self.drag_state {
            MindmapDragState::Idle => None,
            MindmapDragState::Preview(preview) => Some(&preview.canvas),
        }
    }

    fn caret_for_focus(&self, focus: &MindmapFocus) -> Option<(usize, usize)> {
        if !self.cursor_visible || self.preedit.as_ref().is_some_and(|(text, _)| !text.is_empty()) {
            return None;
        }
        let MindmapFocus::TitleEditing { node_index, cursor_byte } = focus else {
            return None;
        };
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return None;
        };
        let node = *collect_nodes_dfs(&tree.root).get(*node_index)?;
        let cursor = cursor_byte.checked_sub(node.title_byte_range.start)?;
        Some((*node_index, clamp_to_char_boundary(&node.title, cursor)))
    }

    fn preedit_projection(
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
                let node = *collect_nodes_dfs(&tree.root).get(*node_index)?;
                (*node_index, 0..node.title.len())
            }
            MindmapFocus::TitleEditing { node_index, cursor_byte } => {
                let node = *collect_nodes_dfs(&tree.root).get(*node_index)?;
                let cursor = cursor_byte.checked_sub(node.title_byte_range.start)?;
                let cursor = clamp_to_char_boundary(&node.title, cursor);
                (*node_index, cursor..cursor)
            }
            MindmapFocus::TitleTextSelected { node_index, range } => {
                let node = *collect_nodes_dfs(&tree.root).get(*node_index)?;
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
        let node = *collect_nodes_dfs(&tree.root).get(node_index)?;
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

    fn screen_to_content(&self, x: f32, y: f32) -> Option<CanvasPoint> {
        Some(self.canvas_viewport?.screen_to_content(CanvasPoint::new(x, y)))
    }

    fn drag_request_hits_title(&self, request: &CanvasDragRequest) -> bool {
        let MindmapDocumentState::Ready { hit_map: Some(hit_map), .. } = &self.document_state
        else {
            return false;
        };
        let pointer = self.screen_to_content(request.pointer_x, request.pointer_y);
        let pressed = self.screen_to_content(request.pressed_x, request.pressed_y);
        [pointer, pressed]
            .into_iter()
            .flatten()
            .any(|point| hit_map.nodes.iter().any(|node| node.contains_title(point.x, point.y)))
    }

    fn calculate_drag_preview(&self, request: &CanvasDragRequest) -> Option<MindmapDragPreview> {
        let MindmapDocumentState::Ready { generation, tree, layout: Some(layout), .. } =
            &self.document_state
        else {
            return None;
        };
        let nodes = collect_nodes_dfs(&tree.root);
        let source_index =
            nodes.iter().position(|node| node.subtree_source_range == request.source_range)?;
        let source_node = *nodes.get(source_index)?;
        let source_layout = layout_node_for_source(layout, source_index)?;
        let label = format!("{} · {}", source_node.title, descendant_count(source_node));
        let preview_width = measured_preview_card_width(
            &label,
            source_layout.w,
            &self.constants,
            self.cached_font_size,
            source_layout.depth,
        );
        let source_subtree_rects = collect_nodes_dfs(source_node)
            .iter()
            .filter_map(|node| nodes.iter().position(|candidate| std::ptr::eq(*candidate, *node)))
            .filter_map(|index| layout_node_for_source(layout, index))
            .map(layout_rect)
            .collect();
        let pointer = self.screen_to_content(request.pointer_x, request.pointer_y)?;
        let pressed = self.screen_to_content(request.pressed_x, request.pressed_y)?;
        let preview_rect = Rect::new(
            pointer.x - (pressed.x - source_layout.x),
            pointer.y - (pressed.y - source_layout.y),
            preview_width,
            source_layout.h,
        );
        let preview_center_y = preview_rect.y + preview_rect.h * 0.5;
        let source_rect = layout_rect(source_layout);
        let candidate =
            nearest_drag_candidate(layout, tree, source_node, source_index, preview_rect);
        let mut anchor_range = None;
        let mut target = None;
        let mut canvas = CanvasDragPreview {
            label,
            source_rect,
            source_subtree_rects,
            preview_rect,
            guide_from: (preview_rect.x, preview_rect.y + preview_rect.h * 0.5),
            guide_to: None,
            insertion_line: None,
            target_rect: None,
            is_valid: false,
        };

        if let Some(candidate) = candidate {
            let anchor_rect = layout_rect(candidate.layout_node);
            canvas.target_rect = Some(anchor_rect);
            let source_is_root = source_index == 0;
            let anchor_is_source_descendant = collect_nodes_dfs(source_node)
                .into_iter()
                .any(|node| std::ptr::eq(node, candidate.node));
            let same_level = preview_rect.x <= candidate.layout_node.x + candidate.layout_node.w;
            let candidate_parent_index = candidate.node_index;
            let (anchor_index, candidate_target) = if same_level {
                if preview_center_y < candidate.layout_node.y + candidate.layout_node.h * 0.5 {
                    (candidate.node_index, mmf::edit::MoveSubtreeTarget::BeforeSibling)
                } else {
                    (candidate.node_index, mmf::edit::MoveSubtreeTarget::AfterSibling)
                }
            } else if let Some(child_index) =
                next_child_at_or_below_y(tree, layout, candidate.node_index, preview_center_y)
            {
                (child_index, mmf::edit::MoveSubtreeTarget::BeforeChild)
            } else {
                (candidate.node_index, mmf::edit::MoveSubtreeTarget::LastChild)
            };
            let target_parent_index = match candidate_target {
                mmf::edit::MoveSubtreeTarget::BeforeSibling
                | mmf::edit::MoveSubtreeTarget::AfterSibling => find_parent(tree, anchor_index)
                    .and_then(|parent| nodes.iter().position(|node| std::ptr::eq(*node, parent))),
                mmf::edit::MoveSubtreeTarget::BeforeChild => Some(candidate_parent_index),
                mmf::edit::MoveSubtreeTarget::LastChild => Some(anchor_index),
            };
            if let Some(parent) =
                target_parent_index.and_then(|index| layout_node_for_source(layout, index))
            {
                canvas.guide_to = Some((parent.x + parent.w, parent.y + parent.h * 0.5));
            }
            let valid = request.source_generation == *generation
                && !source_is_root
                && !anchor_is_source_descendant
                && !is_noop_sibling_target(tree, source_index, anchor_index, candidate_target);
            if valid {
                let anchor_node = nodes[anchor_index];
                anchor_range = Some(anchor_node.subtree_source_range.clone());
                target = Some(candidate_target);
                canvas.is_valid = true;
                if let Some(insertion_line) = insertion_line(
                    layout,
                    tree,
                    anchor_index,
                    candidate_target,
                    self.constants.sibling_gap,
                ) {
                    canvas.insertion_line = Some(insertion_line);
                }
            }
        }

        Some(MindmapDragPreview {
            source_range: request.source_range.clone(),
            anchor_range,
            target,
            canvas,
        })
    }

    fn semantic_hit_target(&self, x: f32, y: f32) -> Option<EditHitTarget> {
        let MindmapDocumentState::Ready { tree, hit_map: Some(hit_map), .. } = &self.document_state
        else {
            return None;
        };
        let pointer = self.screen_to_content(x, y)?;
        let canvas_x = pointer.x;
        let canvas_y = pointer.y;
        let nodes = collect_nodes_dfs(&tree.root);

        for control in &hit_map.controls {
            if control.bounds.contains(canvas_x, canvas_y) {
                let node = nodes.get(control.source_node_index)?;
                return Some(EditHitTarget::CanvasControl {
                    source_range: node.subtree_source_range.clone(),
                });
            }
        }

        for geometry in &hit_map.nodes {
            if geometry.contains_title(canvas_x, canvas_y) {
                if geometry.title_byte_range.is_empty() {
                    return Some(EditHitTarget::TextCaret {
                        byte_offset: geometry.title_byte_range.start,
                        selection_scope: Some(geometry.title_byte_range.clone()),
                    });
                }
                if self.preedit.is_some() {
                    return match self.render_focus() {
                        MindmapFocus::TitleEditing { cursor_byte, .. } => {
                            Some(EditHitTarget::TextCaret {
                                byte_offset: cursor_byte,
                                selection_scope: Some(geometry.title_byte_range.clone()),
                            })
                        }
                        _ => Some(EditHitTarget::SourceObject {
                            source_range: geometry.subtree_source_range.clone(),
                        }),
                    };
                }
                let boundary_index = nearest_grapheme_boundary(&geometry.grapheme_edges, canvas_x);
                let byte_offset = geometry.title_byte_range.start
                    + geometry.grapheme_byte_offsets.get(boundary_index).copied().unwrap_or(0);
                return Some(EditHitTarget::TextCaret {
                    byte_offset,
                    selection_scope: Some(geometry.title_byte_range.clone()),
                });
            }
            if geometry.card_rect.contains(canvas_x, canvas_y) {
                return Some(EditHitTarget::SourceObject {
                    source_range: geometry.subtree_source_range.clone(),
                });
            }
        }

        Some(EditHitTarget::ClearFocus)
    }

    fn cursor_screen_pos(&self, byte_offset: usize) -> Option<(f32, f32, f32, f32)> {
        let MindmapDocumentState::Ready { hit_map: Some(hit_map), .. } = &self.document_state
        else {
            return None;
        };
        let viewport = self.canvas_viewport?;
        let projection = self.build_render_projection();
        let node_index =
            projection.composition_caret().map(|(node_index, _)| node_index).or_else(|| {
                hit_map
                    .nodes
                    .iter()
                    .find(|geometry| {
                        byte_offset >= geometry.title_byte_range.start
                            && byte_offset <= geometry.title_byte_range.end
                    })
                    .map(|geometry| geometry.source_node_index)
            })?;
        let geometry = hit_geometry_for_source(hit_map, node_index)?;
        if projection.composition_caret().is_none()
            && !matches!(
                projection.focus,
                MindmapFocus::TitleEditing { .. } | MindmapFocus::TitleTextSelected { .. }
            )
        {
            return None;
        }
        let relative_byte = projection
            .composition_caret()
            .filter(|(caret_node_index, _)| *caret_node_index == node_index)
            .map(|(_, projected_byte)| projected_byte)
            .unwrap_or(byte_offset - geometry.title_byte_range.start);
        let edge_index =
            grapheme_boundary_at_or_before(&geometry.grapheme_byte_offsets, relative_byte);
        let x = geometry.grapheme_edges.get(edge_index).copied()?;
        let screen_rect = viewport.content_rect_to_screen(Rect::new(
            x,
            geometry.title_rect.y,
            mmf::canvas::CARET_WIDTH,
            geometry.title_rect.h,
        ));
        Some((
            screen_rect.x - viewport.viewport.x,
            screen_rect.y - viewport.viewport.y,
            screen_rect.w,
            screen_rect.h,
        ))
    }

    fn move_edit_target(
        &self,
        current_byte: usize,
        direction: MoveDirection,
    ) -> Option<EditHitTarget> {
        if self.clear_focus_at == Some(current_byte) {
            return None;
        }
        match self.current_focus(current_byte) {
            MindmapFocus::None => None,
            MindmapFocus::NodeSelected { node_index } => {
                self.node_navigation_target(node_index, direction)
            }
            MindmapFocus::TitleEditing { node_index, cursor_byte } => {
                self.title_navigation_target(node_index, cursor_byte, direction)
            }
            MindmapFocus::TitleTextSelected { node_index, range } => {
                self.title_selection_navigation_target(node_index, range, direction)
            }
        }
    }

    fn node_navigation_target(
        &self,
        node_index: usize,
        direction: MoveDirection,
    ) -> Option<EditHitTarget> {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return None;
        };
        let nodes = collect_nodes_dfs(&tree.root);
        let node = *nodes.get(node_index)?;
        let target_index = match direction {
            MoveDirection::Up | MoveDirection::Down => {
                self.visible_dfs_neighbor(node_index, direction)
            }
            MoveDirection::Left => find_parent(tree, node_index).and_then(|parent| {
                nodes.iter().position(|candidate| std::ptr::eq(*candidate, parent))
            }),
            MoveDirection::Right => node.children.first().and_then(|child| {
                nodes.iter().position(|candidate| std::ptr::eq(*candidate, child))
            }),
            MoveDirection::LineStart | MoveDirection::LineEnd => Some(node_index),
        }
        .unwrap_or(node_index);
        let target = *nodes.get(target_index)?;
        Some(EditHitTarget::SourceObject { source_range: target.subtree_source_range.clone() })
    }

    fn title_navigation_target(
        &self,
        node_index: usize,
        cursor_byte: usize,
        direction: MoveDirection,
    ) -> Option<EditHitTarget> {
        match direction {
            MoveDirection::Left
            | MoveDirection::Right
            | MoveDirection::LineStart
            | MoveDirection::LineEnd => {
                self.title_caret_navigation(node_index, cursor_byte, direction)
            }
            MoveDirection::Up | MoveDirection::Down => {
                self.adjacent_node_target(node_index, direction)
            }
        }
    }

    fn title_selection_navigation_target(
        &self,
        node_index: usize,
        range: Range<usize>,
        direction: MoveDirection,
    ) -> Option<EditHitTarget> {
        match direction {
            MoveDirection::Left | MoveDirection::LineStart => {
                Some(EditHitTarget::TextCaret { byte_offset: range.start, selection_scope: None })
            }
            MoveDirection::Right | MoveDirection::LineEnd => {
                Some(EditHitTarget::TextCaret { byte_offset: range.end, selection_scope: None })
            }
            MoveDirection::Up | MoveDirection::Down => {
                self.adjacent_node_target(node_index, direction)
            }
        }
    }

    fn title_caret_navigation(
        &self,
        node_index: usize,
        cursor_byte: usize,
        direction: MoveDirection,
    ) -> Option<EditHitTarget> {
        if self.preedit.is_some() {
            return Some(EditHitTarget::TextCaret {
                byte_offset: cursor_byte,
                selection_scope: None,
            });
        }
        let MindmapDocumentState::Ready { hit_map: Some(hit_map), .. } = &self.document_state
        else {
            return None;
        };
        let geometry = hit_geometry_for_source(hit_map, node_index)?;
        if geometry.title_byte_range.is_empty() {
            return Some(EditHitTarget::TextCaret {
                byte_offset: geometry.title_byte_range.start,
                selection_scope: Some(geometry.title_byte_range.clone()),
            });
        }
        let relative_byte = cursor_byte.checked_sub(geometry.title_byte_range.start)?;
        let current_index =
            grapheme_boundary_at_or_before(&geometry.grapheme_byte_offsets, relative_byte);
        let target_index = match direction {
            MoveDirection::Left => current_index.saturating_sub(1),
            MoveDirection::Right => {
                (current_index + 1).min(geometry.grapheme_byte_offsets.len().saturating_sub(1))
            }
            MoveDirection::LineStart => 0,
            MoveDirection::LineEnd => geometry.grapheme_byte_offsets.len().saturating_sub(1),
            MoveDirection::Up | MoveDirection::Down => return None,
        };
        Some(EditHitTarget::TextCaret {
            byte_offset: geometry.title_byte_range.start
                + geometry.grapheme_byte_offsets.get(target_index).copied()?,
            selection_scope: Some(geometry.title_byte_range.clone()),
        })
    }

    fn adjacent_node_target(
        &self,
        node_index: usize,
        direction: MoveDirection,
    ) -> Option<EditHitTarget> {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            return None;
        };
        let target_index = self.visible_dfs_neighbor(node_index, direction)?;
        let target = *collect_nodes_dfs(&tree.root).get(target_index)?;
        Some(EditHitTarget::SourceObject { source_range: target.subtree_source_range.clone() })
    }

    fn visible_dfs_neighbor(
        &self,
        source_node_index: usize,
        direction: MoveDirection,
    ) -> Option<usize> {
        let MindmapDocumentState::Ready { tree, layout, .. } = &self.document_state else {
            return None;
        };
        let visible_source_indices = if let Some(layout) = layout {
            layout.nodes.iter().map(|node| node.source_node_index).collect::<Vec<_>>()
        } else {
            visible_source_indices(&tree.root)
        };
        let visible_position =
            visible_source_indices.iter().position(|index| *index == source_node_index)?;
        let target_position = match direction {
            MoveDirection::Up => visible_position.checked_sub(1)?,
            MoveDirection::Down => visible_position.checked_add(1)?,
            MoveDirection::Left
            | MoveDirection::Right
            | MoveDirection::LineStart
            | MoveDirection::LineEnd => return None,
        };
        visible_source_indices.get(target_position).copied()
    }

    #[cfg(test)]
    fn ready_tree(&self) -> &Tree {
        let MindmapDocumentState::Ready { tree, .. } = &self.document_state else {
            panic!("test requires ready mmap state");
        };
        tree
    }

    #[cfg(test)]
    fn ready_hit_map(&self) -> &HitMap {
        let MindmapDocumentState::Ready { hit_map: Some(hit_map), .. } = &self.document_state
        else {
            panic!("test requires ready mmap state");
        };
        hit_map
    }
}

fn layout_node_for_source(
    layout: &LayoutTree,
    source_node_index: usize,
) -> Option<&mmf::layout::LayoutNode> {
    layout.nodes.iter().find(|node| node.source_node_index == source_node_index)
}

fn hit_geometry_for_source(
    hit_map: &HitMap,
    source_node_index: usize,
) -> Option<&mmf::layout::NodeHitGeometry> {
    hit_map.nodes.iter().find(|geometry| geometry.source_node_index == source_node_index)
}

fn measured_preview_card_width(
    label: &str,
    minimum_width: f32,
    constants: &LayoutConstants,
    font_size: f32,
    depth: u8,
) -> f32 {
    let measured_width = Shaper::new()
        .ok()
        .map(|mut shaper| {
            if font_size.is_finite() && font_size > 0.0 {
                shaper.set_font_size(font_size * constants.font_scale_for_depth(depth));
            }
            measured_card_width(label, constants, &mut shaper) - 2.0 * constants.card_padding_x
        })
        .unwrap_or(label.len() as f32 * PREVIEW_TEXT_FALLBACK_ADVANCE_PER_BYTE);
    (measured_width + 2.0 * constants.card_padding_x).max(minimum_width)
}

fn visible_source_indices(root: &mmf::Node) -> Vec<usize> {
    fn visit(node: &mmf::Node, source_node_index: usize, output: &mut Vec<usize>) -> usize {
        output.push(source_node_index);
        let subtree_size = 1 + descendant_count(node);
        if source_node_index != 0 && node.props.as_ref().is_some_and(|props| props.collapsed) {
            return subtree_size;
        }
        let mut child_index = source_node_index + 1;
        for child in &node.children {
            child_index += visit(child, child_index, output);
        }
        subtree_size
    }

    let mut indices = Vec::new();
    visit(root, 0, &mut indices);
    indices
}

fn layout_rect(node: &mmf::layout::LayoutNode) -> Rect {
    Rect::new(node.x, node.y, node.w, node.h)
}

fn include_rect(bounds: &mut Rect, rect: Rect) {
    include_point(bounds, (rect.x, rect.y));
    include_point(bounds, (rect.x + rect.w, rect.y + rect.h));
}

fn include_point(bounds: &mut Rect, point: (f32, f32)) {
    let min_x = bounds.x.min(point.0);
    let min_y = bounds.y.min(point.1);
    let max_x = (bounds.x + bounds.w).max(point.0);
    let max_y = (bounds.y + bounds.h).max(point.1);
    *bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
}

fn nearest_drag_candidate<'a>(
    layout: &'a LayoutTree,
    tree: &'a Tree,
    source_node: &'a mmf::Node,
    source_index: usize,
    preview_rect: Rect,
) -> Option<DragCandidate<'a>> {
    let nodes = collect_nodes_dfs(&tree.root);
    let source_subtree = collect_nodes_dfs(source_node);
    layout
        .nodes
        .iter()
        .filter_map(|layout_node| {
            let node_index = layout_node.source_node_index;
            let node = *nodes.get(node_index)?;
            let is_source_subtree =
                source_subtree.iter().any(|subtree_node| std::ptr::eq(*subtree_node, node));
            let is_legal_root = node_index != 0
                || (source_index != 0 && preview_rect.x > layout_node.x + layout_node.w);
            (!is_source_subtree && is_legal_root).then_some(DragCandidate {
                node_index,
                node,
                layout_node,
            })
        })
        .min_by(|left, right| {
            drag_distance(left.layout_node, preview_rect)
                .total_cmp(&drag_distance(right.layout_node, preview_rect))
                .then_with(|| left.node_index.cmp(&right.node_index))
        })
}

fn next_child_at_or_below_y(
    tree: &Tree,
    layout: &LayoutTree,
    parent_index: usize,
    pointer_y: f32,
) -> Option<usize> {
    let nodes = collect_nodes_dfs(&tree.root);
    layout
        .nodes
        .iter()
        .filter(|child| {
            find_parent(tree, child.source_node_index)
                .is_some_and(|parent| std::ptr::eq(parent, nodes[parent_index]))
        })
        .find(|child| pointer_y < child.y + child.h * 0.5)
        .map(|child| child.source_node_index)
}

fn drag_distance(node: &mmf::layout::LayoutNode, preview_rect: Rect) -> f32 {
    let dx = preview_rect.x - (node.x + node.w);
    let dy = preview_rect.y + preview_rect.h * 0.5 - (node.y + node.h * 0.5);
    dx * dx + dy * dy
}

fn is_noop_sibling_target(
    tree: &Tree,
    source_index: usize,
    anchor_index: usize,
    target: mmf::edit::MoveSubtreeTarget,
) -> bool {
    if target == mmf::edit::MoveSubtreeTarget::BeforeChild && source_index == anchor_index {
        return true;
    }
    let Some(siblings) = find_siblings(tree, source_index) else {
        return false;
    };
    let Some(source_position) = siblings.iter().position(|index| *index == source_index) else {
        return false;
    };
    let Some(anchor_position) = siblings.iter().position(|index| *index == anchor_index) else {
        return false;
    };
    match target {
        mmf::edit::MoveSubtreeTarget::BeforeSibling => {
            source_position.checked_add(1) == Some(anchor_position)
        }
        mmf::edit::MoveSubtreeTarget::AfterSibling => {
            anchor_position.checked_add(1) == Some(source_position)
        }
        mmf::edit::MoveSubtreeTarget::BeforeChild => false,
        mmf::edit::MoveSubtreeTarget::LastChild => false,
    }
}

fn insertion_line(
    layout: &LayoutTree,
    tree: &Tree,
    anchor_index: usize,
    target: mmf::edit::MoveSubtreeTarget,
    sibling_gap: f32,
) -> Option<((f32, f32), (f32, f32))> {
    let anchor = layout_node_for_source(layout, anchor_index)?;
    let sibling_indices = find_siblings(tree, anchor_index)?;
    let sibling_position = sibling_indices.iter().position(|index| *index == anchor_index)?;
    let y = match target {
        mmf::edit::MoveSubtreeTarget::BeforeSibling | mmf::edit::MoveSubtreeTarget::BeforeChild => {
            sibling_position
                .checked_sub(1)
                .and_then(|previous_position| sibling_indices.get(previous_position))
                .and_then(|index| layout_node_for_source(layout, *index))
                .map(|previous| (previous.y + previous.h + anchor.y) * 0.5)
                .unwrap_or(anchor.y - sibling_gap * 0.5)
        }
        mmf::edit::MoveSubtreeTarget::AfterSibling => sibling_position
            .checked_add(1)
            .and_then(|next_position| sibling_indices.get(next_position))
            .and_then(|index| layout_node_for_source(layout, *index))
            .map(|next| (anchor.y + anchor.h + next.y) * 0.5)
            .unwrap_or(anchor.y + anchor.h + sibling_gap * 0.5),
        mmf::edit::MoveSubtreeTarget::LastChild => return None,
    };
    Some(((anchor.x, y), (anchor.x + anchor.w, y)))
}

fn clamp_to_char_boundary(text: &str, byte_offset: usize) -> usize {
    let mut boundary = byte_offset.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn draw_invalid_canvas(
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

fn nearest_grapheme_boundary(edges: &[f32], x: f32) -> usize {
    edges
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| (x - **left).abs().total_cmp(&(x - **right).abs()))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn grapheme_boundary_at_or_before(boundaries: &[usize], byte_offset: usize) -> usize {
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

impl ViewPlugin for MindmapView {
    fn name(&self) -> &str {
        "mindmap"
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
            let nodes = collect_nodes_dfs(&tree.root);
            draw_list.clip(viewport.viewport, |canvas_draw_list| {
                mmf::canvas::render(
                    canvas_draw_list,
                    layout,
                    *viewport,
                    &render_theme,
                    &self.constants,
                    shaper,
                    &nodes,
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

    fn handle_message(&mut self, message: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        match message {
            PluginMessage::UpdateSource { text, generation } => {
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
                        hit_map: None,
                        connector_mesh_cache: None,
                    },
                    Err(diagnostic) => MindmapDocumentState::Invalid { generation, diagnostic },
                };
                true
            }
            PluginMessage::SetCursorByte(cursor_byte) => {
                if self.clear_focus_at != Some(cursor_byte) {
                    self.clear_focus_at = None;
                }
                self.cursor_byte = Some(cursor_byte);
                self.clear_layout_for_preedit();
                true
            }
            PluginMessage::SetSelAnchorByte(anchor) => {
                self.selection_anchor_byte = anchor;
                self.clear_layout_for_preedit();
                true
            }
            PluginMessage::SetSelCursorByte(cursor) => {
                self.selection_cursor_byte = cursor;
                self.clear_layout_for_preedit();
                true
            }
            PluginMessage::ClearSelection => {
                self.selection_anchor_byte = None;
                self.selection_cursor_byte = None;
                self.clear_layout_for_preedit();
                true
            }
            PluginMessage::SetSelCursor(position) => {
                self.selection_cursor_byte =
                    position.map(|(line, column)| doc.line_byte_offset(line) + column);
                self.clear_layout_for_preedit();
                true
            }
            PluginMessage::SetSelAnchor(position) => {
                self.selection_anchor_byte =
                    position.map(|(line, column)| doc.line_byte_offset(line) + column);
                self.clear_layout_for_preedit();
                true
            }
            PluginMessage::SetPreedit { text, cursor } => {
                let next_preedit = (!text.is_empty()).then_some((text, cursor));
                if self.preedit != next_preedit {
                    self.preedit = next_preedit;
                    self.clear_layout();
                }
                true
            }
            PluginMessage::SetCursorVisible(visible) => {
                self.cursor_visible = visible;
                true
            }
            PluginMessage::SetCanvasPointer(pointer) => {
                self.canvas_pointer = pointer;
                true
            }
            PluginMessage::ClearEditFocus => {
                self.clear_focus_at = self.cursor_byte;
                self.drag_state = MindmapDragState::Idle;
                self.clear_layout_for_preedit();
                true
            }
            _ => false,
        }
    }

    fn query(&self, query: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        match query {
            PluginQuery::NeedsSourceUpdate(generation) => {
                PluginResponse::Bool(self.needs_source_update(generation))
            }
            PluginQuery::HitTestEditTarget { x, y, .. } => {
                PluginResponse::EditHitTarget(self.semantic_hit_target(x, y))
            }
            PluginQuery::MoveEditTarget { current_byte, direction, .. } => {
                PluginResponse::EditHitTarget(self.move_edit_target(current_byte, direction))
            }
            PluginQuery::CursorScreenPos(byte_offset) => {
                PluginResponse::CursorScreenRect(self.cursor_screen_pos(byte_offset))
            }
            PluginQuery::PlanCanvasControl { source_range, source_generation } => {
                let MindmapDocumentState::Ready { generation, source, tree, .. } =
                    &self.document_state
                else {
                    return PluginResponse::EditPlan(EditPlan::Consume);
                };
                if source_generation != *generation {
                    return PluginResponse::EditPlan(EditPlan::Consume);
                }
                PluginResponse::EditPlan(mmf::edit::plan_toggle_collapsed(
                    tree,
                    source,
                    source_range,
                    source_generation,
                ))
            }
            PluginQuery::MindmapThemeSelection => {
                let selection = match &self.document_state {
                    MindmapDocumentState::Invalid { .. } => {
                        ui::theme::MindmapThemeSelection::InvalidMetadata
                    }
                    MindmapDocumentState::Ready { tree, .. } => resolve_mindmap_theme_selection(
                        tree.global_props.get("theme").map(|s| s.as_str()),
                    ),
                    MindmapDocumentState::Uninitialized => {
                        ui::theme::MindmapThemeSelection::Default
                    }
                };
                PluginResponse::MindmapThemeSelection(selection)
            }
            PluginQuery::PlanMindmapTheme { theme_id, source_generation } => {
                let MindmapDocumentState::Ready { generation, source, tree, .. } =
                    &self.document_state
                else {
                    return PluginResponse::EditPlan(EditPlan::Consume);
                };
                if source_generation != *generation {
                    return PluginResponse::EditPlan(EditPlan::Consume);
                }
                if find_mindmap_color_scheme(&theme_id).is_none() {
                    return PluginResponse::EditPlan(EditPlan::Consume);
                }
                PluginResponse::EditPlan(mmf::edit::plan_set_mindmap_theme(
                    tree,
                    source,
                    &theme_id,
                    source_generation,
                    self.cursor_byte.unwrap_or(source.len()),
                ))
            }
            _ => PluginResponse::None,
        }
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
                    EditPlan::UseDefault
                    | EditPlan::SetSelection(_)
                    | EditPlan::MoveCursor(_)
                    | EditPlan::Consume => CanvasDragResponse::Ignore,
                }
            }
        }
    }

    fn shows_cursor(&self) -> bool {
        false
    }

    fn needs_cursor_blink_wakeup(&self) -> bool {
        true
    }

    fn shows_gutter(&self) -> bool {
        false
    }

    fn allows_editing(&self) -> bool {
        true
    }

    fn handles_own_rendering(&self) -> bool {
        true
    }

    fn edit_policy(&self) -> &dyn EditPolicy {
        self
    }

    fn key_intent_mapper(&self) -> Option<&dyn KeyIntentMapper> {
        Some(self)
    }

    fn is_canvas(&self) -> bool {
        true
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

    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MindmapView::new())
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, sync::Arc};

    use super::*;
    use core::document::{DocView, DocViewMut};
    use ui::canvas::{
        CanvasPoint, CanvasViewPosition, CanvasViewportConfig, CanvasViewportInput,
        resolve_viewport,
    };
    use ui::core::paint::DrawCmd;
    use ui::theme::{MindmapThemeSelection, ThemeDefinition};

    const DROP_BEYOND_NODE_RIGHT_EDGE: f32 = 8.0;
    const ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE: f32 = 1.0;
    const ONE_PIXEL_BEFORE_TITLE_LEFT_EDGE: f32 = 1.0;

    struct MindmapTestDoc {
        text: String,
        lines: Vec<String>,
    }

    impl MindmapTestDoc {
        fn new(source: &str) -> Self {
            Self { text: source.to_owned(), lines: source.split('\n').map(str::to_owned).collect() }
        }
    }

    impl DocView for MindmapTestDoc {
        fn line_count(&self) -> usize {
            self.lines.len()
        }

        fn doc_line_text(&self, line: usize) -> Cow<'_, str> {
            Cow::Borrowed(self.lines.get(line).map(String::as_str).unwrap_or(""))
        }

        fn doc_text_in_range(&self, range: Range<usize>) -> Cow<'_, str> {
            Cow::Borrowed(&self.text[range])
        }

        fn line_byte_offset(&self, line: usize) -> usize {
            self.lines.iter().take(line).map(|line| line.len() + 1).sum()
        }

        fn line_byte_length(&self, line: usize) -> usize {
            self.lines.get(line).map(String::len).unwrap_or(0)
        }

        fn scroll_y(&self) -> f32 {
            0.0
        }

        fn viewport_height(&self) -> f32 {
            800.0
        }
    }

    impl DocViewMut for MindmapTestDoc {
        fn set_scroll_y(&mut self, _scroll_y: f32) {}

        fn replace_range(&mut self, range: Range<usize>, text: &str) {
            self.text.replace_range(range, text);
            self.lines = self.text.split('\n').map(str::to_owned).collect();
        }
    }

    fn view_with_source(source: &str) -> (MindmapView, MindmapTestDoc) {
        let mut view = MindmapView::new();
        let mut doc = MindmapTestDoc::new(source);
        view.handle_message(
            PluginMessage::UpdateSource { text: source.to_owned(), generation: 1 },
            &mut doc,
        );
        (view, doc)
    }

    fn render_test_view(view: &mut MindmapView, doc: &MindmapTestDoc) {
        let mut shaper = Shaper::new().expect("test shaper");
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let _ = view.render(doc, Rect::new(0.0, 0.0, 1200.0, 800.0), &theme, &mut shaper, 1.0);
    }

    fn render_test_draw_list(view: &mut MindmapView, doc: &MindmapTestDoc) -> DrawList {
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        render_test_draw_list_with_theme(view, doc, &theme)
    }

    fn render_test_draw_list_with_theme(
        view: &mut MindmapView,
        doc: &MindmapTestDoc,
        theme: &Theme,
    ) -> DrawList {
        let mut shaper = Shaper::new().expect("test shaper");
        view.render(doc, Rect::new(0.0, 0.0, 1200.0, 800.0), theme, &mut shaper, 1.0)
    }

    /// 渲染时无元数据的文档回退到内置默认配色方案，测试断言的颜色必须与之保持一致。
    fn default_scheme_canvas() -> &'static ui::theme::MindmapCanvasTheme {
        &ui::theme::find_mindmap_color_scheme(ui::theme::DEFAULT_MINDMAP_COLOR_SCHEME_ID)
            .expect("default scheme is registered")
            .canvas
    }

    /// 与渲染路径一致：浅色画布上悬停加深，深色画布上悬停提亮。
    fn hovered_control_color(
        canvas: &ui::theme::MindmapCanvasTheme,
        branch_color: [f32; 4],
    ) -> [f32; 4] {
        let [r, g, b, _] = canvas.background;
        let is_dark_background = 0.299 * r + 0.587 * g + 0.114 * b < 0.5;
        if is_dark_background {
            crate::mmf::canvas::lighten_color(
                branch_color,
                crate::mmf::canvas::CONTROL_HOVER_LIGHTEN_FACTOR,
            )
        } else {
            crate::mmf::canvas::darken_color(
                branch_color,
                crate::mmf::canvas::CONTROL_HOVER_DARKEN_FACTOR,
            )
        }
    }

    fn first_tapered_mesh(draw_list: &DrawList) -> Arc<ui::tapered_path::TaperedMesh> {
        draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TaperedMesh { mesh, .. } => Some(Arc::clone(mesh)),
                _ => None,
            })
            .expect("mmap fixture must render at least one connector")
    }

    #[test]
    fn mmap_static_connector_mesh_cache_survives_render_and_pan_but_not_zoom() {
        let source = "# Root\n## Child\n";
        let (mut view, doc) = view_with_source(source);
        let theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let metrics = view
            .prepare_canvas(&doc, &theme, &mut shaper, 1.0)
            .expect("fixture must prepare canvas");
        let viewport = resolve_viewport(CanvasViewportInput::positioned(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            metrics.content_bounds,
            CanvasViewPosition { zoom: 1.0, scroll: CanvasPoint::ZERO },
            CanvasViewportConfig::DEFAULT,
        ));

        let first =
            first_tapered_mesh(&view.render_canvas(&doc, &viewport, &theme, &mut shaper, 1.0));
        let second =
            first_tapered_mesh(&view.render_canvas(&doc, &viewport, &theme, &mut shaper, 1.0));
        assert!(Arc::ptr_eq(&first, &second));

        let mut panned = viewport;
        panned.scroll = CanvasPoint::new(20.0, 10.0);
        let after_pan =
            first_tapered_mesh(&view.render_canvas(&doc, &panned, &theme, &mut shaper, 1.0));
        assert!(Arc::ptr_eq(&first, &after_pan));

        let mut zoomed = viewport;
        zoomed.zoom = 2.0;
        let after_zoom =
            first_tapered_mesh(&view.render_canvas(&doc, &zoomed, &theme, &mut shaper, 1.0));
        assert!(!Arc::ptr_eq(&first, &after_zoom));
    }

    #[test]
    fn mmap_layout_invalidation_drops_connector_mesh_cache() {
        let (mut view, doc) = view_with_source("# Root\n## Child\n");
        render_test_view(&mut view, &doc);

        let MindmapDocumentState::Ready { connector_mesh_cache, .. } = &view.document_state else {
            panic!("fixture must be ready");
        };
        assert!(connector_mesh_cache.is_some());

        view.clear_layout();

        let MindmapDocumentState::Ready { connector_mesh_cache, .. } = &view.document_state else {
            panic!("fixture must remain ready");
        };
        assert!(connector_mesh_cache.is_none());
    }

    #[test]
    fn mmap_non_geometry_render_changes_keep_connector_mesh_cache() {
        let source = "# Root\n## Child\n";
        let (mut view, mut doc) = view_with_source(source);
        let first = first_tapered_mesh(&render_test_draw_list(&mut view, &doc));

        view.handle_message(PluginMessage::SetCursorVisible(false), &mut doc);
        view.handle_message(
            PluginMessage::SetCanvasPointer(Some(CanvasPoint::new(200.0, 120.0))),
            &mut doc,
        );
        let after_pointer = first_tapered_mesh(&render_test_draw_list(&mut view, &doc));
        assert!(Arc::ptr_eq(&first, &after_pointer));

        let mut recolored_theme = Theme::from_definition(&ThemeDefinition::default_dark());
        recolored_theme.mindmap.canvas.connector = [0.8, 0.2, 0.1, 0.3];
        let after_color = first_tapered_mesh(&render_test_draw_list_with_theme(
            &mut view,
            &doc,
            &recolored_theme,
        ));
        assert!(Arc::ptr_eq(&first, &after_color));
    }

    fn draw_list_has_expanded_control_bar(
        draw_list: &DrawList,
        control_screen_rect: Rect,
        connector_color: [f32; 4],
    ) -> bool {
        draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                DrawCmd::FillRect { rect, color, radius }
                    if *color == connector_color
                        && *radius == 0.0
                        && rect.x >= control_screen_rect.x
                        && rect.y >= control_screen_rect.y
                        && rect.x + rect.w <= control_screen_rect.x + control_screen_rect.w
                        && rect.y + rect.h <= control_screen_rect.y + control_screen_rect.h
            )
        })
    }

    fn rendered_viewport(view: &MindmapView) -> CanvasViewportSnapshot {
        view.canvas_viewport.expect("test view must render a canvas viewport")
    }

    fn screen_point(view: &MindmapView, content_x: f32, content_y: f32) -> CanvasPoint {
        rendered_viewport(view).content_to_screen(CanvasPoint::new(content_x, content_y))
    }

    fn screen_rect(view: &MindmapView, content_rect: Rect) -> Rect {
        rendered_viewport(view).content_rect_to_screen(content_rect)
    }

    fn node_by_title<'a>(tree: &'a Tree, title: &str) -> &'a mmf::Node {
        collect_nodes_dfs(&tree.root)
            .into_iter()
            .find(|node| node.title == title)
            .expect("test node title should exist")
    }

    fn drag_request(
        view: &MindmapView,
        phase: CanvasDragPhase,
        source_range: Range<usize>,
        pointer_x: f32,
        pointer_y: f32,
        source_generation: u32,
    ) -> CanvasDragRequest {
        let pointer = screen_point(view, pointer_x, pointer_y);
        drag_request_at_screen(view, phase, source_range, pointer.x, pointer.y, source_generation)
    }

    fn drag_request_at_screen(
        view: &MindmapView,
        phase: CanvasDragPhase,
        source_range: Range<usize>,
        pointer_x: f32,
        pointer_y: f32,
        source_generation: u32,
    ) -> CanvasDragRequest {
        let source_index = collect_nodes_dfs(&view.ready_tree().root)
            .iter()
            .position(|node| node.subtree_source_range == source_range)
            .expect("source must have DFS index");
        let source_geometry = &view.ready_hit_map().nodes[source_index];
        let source_card = source_geometry.card_rect;
        let pressed = screen_point(
            view,
            source_geometry.title_rect.x - ONE_PIXEL_BEFORE_TITLE_LEFT_EDGE,
            source_card.y + source_card.h * 0.5,
        );
        CanvasDragRequest {
            phase,
            source_range,
            pointer_x,
            pointer_y,
            pressed_x: pressed.x,
            pressed_y: pressed.y,
            offset_x: 0.0,
            offset_y: 0.0,
            source_generation,
        }
    }

    fn drag_request_after_node(
        view: &MindmapView,
        source_range: Range<usize>,
        anchor_title: &str,
        source_generation: u32,
    ) -> CanvasDragRequest {
        let pointer_x = pointer_past_node_right_edge(view, &source_range, anchor_title);
        let anchor = node_by_title(view.ready_tree(), anchor_title);
        let anchor_index = collect_nodes_dfs(&view.ready_tree().root)
            .iter()
            .position(|node| node.subtree_source_range == anchor.subtree_source_range)
            .expect("anchor must have DFS index");
        let geometry = &view.ready_hit_map().nodes[anchor_index];
        drag_request(
            view,
            CanvasDragPhase::Update,
            source_range,
            pointer_x,
            geometry.card_rect.y + geometry.card_rect.h + DROP_BEYOND_NODE_RIGHT_EDGE,
            source_generation,
        )
    }

    fn pointer_past_node_right_edge(
        view: &MindmapView,
        source_range: &Range<usize>,
        anchor_title: &str,
    ) -> f32 {
        let nodes = collect_nodes_dfs(&view.ready_tree().root);
        let source_index = nodes
            .iter()
            .position(|node| node.subtree_source_range == *source_range)
            .expect("source must have DFS index");
        let anchor_index = nodes
            .iter()
            .position(|node| node.title == anchor_title)
            .expect("anchor must have DFS index");
        let source_card = view.ready_hit_map().nodes[source_index].card_rect;
        let anchor_card = view.ready_hit_map().nodes[anchor_index].card_rect;
        anchor_card.x + anchor_card.w + source_card.w * 0.5 + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE
    }

    fn drag_request_on_sibling_right_edge(
        view: &MindmapView,
        source_range: Range<usize>,
        anchor_title: &str,
        source_generation: u32,
    ) -> CanvasDragRequest {
        let anchor = node_by_title(view.ready_tree(), anchor_title);
        let anchor_index = collect_nodes_dfs(&view.ready_tree().root)
            .iter()
            .position(|node| node.subtree_source_range == anchor.subtree_source_range)
            .expect("anchor must have DFS index");
        let geometry = &view.ready_hit_map().nodes[anchor_index];
        drag_request(
            view,
            CanvasDragPhase::Update,
            source_range,
            geometry.card_rect.x + geometry.card_rect.w,
            geometry.card_rect.y + geometry.card_rect.h + 8.0,
            source_generation,
        )
    }

    fn apply_transaction_to_text(
        source: &str,
        transaction: &ui::plugin::EditTransaction,
    ) -> String {
        let mut text = source.to_owned();
        let mut replacements = transaction.replacements.clone();
        replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.range.start));
        for replacement in replacements {
            text.replace_range(replacement.range, &replacement.text);
        }
        text
    }

    #[test]
    fn canvas_snapshot_round_trips_hit_cursor_and_drag_at_zoom_and_scroll() {
        const SCROLL: CanvasPoint = CanvasPoint::new(60.0, 45.0);
        let viewport = Rect::new(120.0, 80.0, 60.0, 40.0);
        let source = "# Root\n## A\n## B\n## C\n";
        let (mut view, mut doc) = view_with_source(source);
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let title_start = node_by_title(view.ready_tree(), "B").title_byte_range.start;
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        view.handle_message(PluginMessage::SetCursorByte(title_start), &mut doc);

        for zoom in [0.5, 2.0] {
            let metrics = view
                .prepare_canvas(&doc, &theme, &mut shaper, 1.0)
                .expect("mindmap must report canvas content metrics");
            let snapshot = resolve_viewport(CanvasViewportInput::positioned(
                viewport,
                metrics.content_bounds,
                CanvasViewPosition { zoom, scroll: SCROLL },
                CanvasViewportConfig::for_dpi(1.0),
            ));
            assert!(snapshot.scroll.x > 0.0, "zoom {zoom} must retain horizontal scroll");
            assert!(snapshot.scroll.y > 0.0, "zoom {zoom} must retain vertical scroll");
            let _ = view.render_canvas(&doc, &snapshot, &theme, &mut shaper, 1.0);

            let geometry = &view.ready_hit_map().nodes[2];
            let title_point = CanvasPoint::new(
                geometry.title_rect.x + 1.0,
                geometry.title_rect.y + geometry.title_rect.h * 0.5,
            );
            let screen_title_point = snapshot.content_to_screen(title_point);
            assert!(matches!(
                view.semantic_hit_target(screen_title_point.x, screen_title_point.y),
                Some(EditHitTarget::TextCaret { byte_offset, .. })
                    if byte_offset == title_start
            ));

            let expected_cursor = snapshot.content_rect_to_screen(Rect::new(
                geometry.grapheme_edges[0],
                geometry.title_rect.y,
                mmf::canvas::CARET_WIDTH,
                geometry.title_rect.h,
            ));
            assert!(matches!(
                view.query(PluginQuery::CursorScreenPos(title_start), &doc),
                PluginResponse::CursorScreenRect(Some((x, y, width, height)))
                    if (x - (expected_cursor.x - snapshot.viewport.x)).abs() < f32::EPSILON
                        && (y - (expected_cursor.y - snapshot.viewport.y)).abs() < f32::EPSILON
                        && (width - expected_cursor.w).abs() < f32::EPSILON
                        && (height - expected_cursor.h).abs() < f32::EPSILON
            ));

            let pointer_content = CanvasPoint::new(
                geometry.card_rect.x + geometry.card_rect.w + 20.0,
                geometry.card_rect.y + geometry.card_rect.h + 8.0,
            );
            let pointer_screen = snapshot.content_to_screen(pointer_content);
            // 预览卡宽按源节点（B 为 depth 1）深度缩放字号测量，与 render_drag_preview 一致。
            let mut preview_shaper = Shaper::new().expect("test shaper should initialize");
            preview_shaper
                .set_font_size(view.cached_font_size * view.constants.font_scale_for_depth(1));
            let expected_preview_width =
                measured_card_width("B · 0", &view.constants, &mut preview_shaper)
                    .max(geometry.card_rect.w);
            let expected_preview_center_x = pointer_content.x
                + (geometry.card_rect.x + expected_preview_width * 0.5
                    - (geometry.title_rect.x - ONE_PIXEL_BEFORE_TITLE_LEFT_EDGE));
            let response = view.handle_canvas_drag(
                drag_request_at_screen(
                    &view,
                    CanvasDragPhase::Update,
                    source_range.clone(),
                    pointer_screen.x,
                    pointer_screen.y,
                    1,
                ),
                &doc,
            );
            // 预览中心经 screen round-trip 换算，深度缩放高度引入的浮点尾差需容忍。
            assert!(matches!(response,
                CanvasDragResponse::Preview(preview)
                    if (preview.preview_rect.x + preview.preview_rect.w * 0.5 - expected_preview_center_x)
                        .abs() < f32::EPSILON
                        && (preview.preview_rect.y + preview.preview_rect.h * 0.5 - pointer_content.y)
                            .abs() < 0.01
            ));

            view.handle_message(
                PluginMessage::SetPreedit { text: "你".into(), cursor: Some((0, 3)) },
                &mut doc,
            );
            let preedit_metrics = view
                .prepare_canvas(&doc, &theme, &mut shaper, 1.0)
                .expect("mindmap must report content metrics while composing");
            let preedit_snapshot = resolve_viewport(CanvasViewportInput::positioned(
                viewport,
                preedit_metrics.content_bounds,
                CanvasViewPosition { zoom, scroll: SCROLL },
                CanvasViewportConfig::for_dpi(1.0),
            ));
            let _ = view.render_canvas(&doc, &preedit_snapshot, &theme, &mut shaper, 1.0);
            let composition_geometry = &view.ready_hit_map().nodes[2];
            let composition_rect = preedit_snapshot.content_rect_to_screen(Rect::new(
                composition_geometry.grapheme_edges[1],
                composition_geometry.title_rect.y,
                mmf::canvas::CARET_WIDTH,
                composition_geometry.title_rect.h,
            ));
            assert!(matches!(
                view.query(PluginQuery::CursorScreenPos(title_start), &doc),
                PluginResponse::CursorScreenRect(Some((x, y, width, height)))
                    if (x - (composition_rect.x - preedit_snapshot.viewport.x)).abs()
                        < f32::EPSILON
                        && (y - (composition_rect.y - preedit_snapshot.viewport.y)).abs()
                            < f32::EPSILON
                        && (width - composition_rect.w).abs() < f32::EPSILON
                        && (height - composition_rect.h).abs() < f32::EPSILON
            ));
            let composition_hit = preedit_snapshot.content_to_screen(CanvasPoint::new(
                composition_geometry.grapheme_edges[2],
                composition_geometry.title_rect.y + composition_geometry.title_rect.h * 0.5,
            ));
            assert!(matches!(
                view.semantic_hit_target(composition_hit.x, composition_hit.y),
                Some(EditHitTarget::TextCaret { byte_offset, .. })
                    if byte_offset == title_start
            ));
            view.handle_message(
                PluginMessage::SetPreedit { text: String::new(), cursor: None },
                &mut doc,
            );
        }
    }

    #[test]
    fn canvas_prepare_extends_content_bounds_for_drag_feedback() {
        let (mut view, doc) = view_with_source("# Root\n## Child\n");
        view.drag_state = MindmapDragState::Preview(MindmapDragPreview {
            source_range: 0..0,
            anchor_range: None,
            target: None,
            canvas: CanvasDragPreview {
                label: String::new(),
                source_rect: Rect::new(-220.0, -180.0, 40.0, 30.0),
                source_subtree_rects: Vec::new(),
                preview_rect: Rect::new(-320.0, -260.0, 60.0, 40.0),
                guide_from: (-320.0, -240.0),
                guide_to: Some((480.0, 720.0)),
                insertion_line: Some(((-400.0, 650.0), (620.0, 650.0))),
                target_rect: Some(Rect::new(560.0, 680.0, 50.0, 30.0)),
                is_valid: true,
            },
        });
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut shaper = Shaper::new().expect("test shaper should initialize");

        let metrics = view
            .prepare_canvas(&doc, &theme, &mut shaper, 1.0)
            .expect("mindmap drag feedback must report content metrics");

        assert!(metrics.content_bounds.x <= -400.0);
        assert!(metrics.content_bounds.y <= -260.0);
        assert!(metrics.content_bounds.x + metrics.content_bounds.w >= 620.0);
        assert!(metrics.content_bounds.y + metrics.content_bounds.h >= 720.0);
    }

    #[test]
    fn canvas_render_clips_edge_drag_feedback_to_the_viewport() {
        let (mut view, doc) = view_with_source("# Root\n## Child\n");
        view.drag_state = MindmapDragState::Preview(MindmapDragPreview {
            source_range: 0..0,
            anchor_range: None,
            target: None,
            canvas: CanvasDragPreview {
                label: String::new(),
                source_rect: Rect::new(0.0, 0.0, 60.0, 44.0),
                source_subtree_rects: Vec::new(),
                preview_rect: Rect::new(-500.0, -400.0, 60.0, 44.0),
                guide_from: (-470.0, -378.0),
                guide_to: Some((0.0, 22.0)),
                insertion_line: Some(((-520.0, -350.0), (-420.0, -350.0))),
                target_rect: Some(Rect::new(-500.0, -400.0, 60.0, 44.0)),
                is_valid: false,
            },
        });
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        let metrics = view
            .prepare_canvas(&doc, &theme, &mut shaper, 1.0)
            .expect("mindmap must prepare canvas metrics");
        let viewport = resolve_viewport(CanvasViewportInput::positioned(
            Rect::new(120.0, 80.0, 160.0, 100.0),
            metrics.content_bounds,
            CanvasViewPosition { zoom: 1.0, scroll: CanvasPoint::new(60.0, 45.0) },
            CanvasViewportConfig::for_dpi(1.0),
        ));

        let draw_list = view.render_canvas(&doc, &viewport, &theme, &mut shaper, 1.0);
        let clip_start = draw_list
            .cmds
            .iter()
            .position(
                |command| matches!(command, DrawCmd::PushClip(rect) if *rect == viewport.viewport),
            )
            .expect("canvas content must start a viewport clip");
        let clip_end = draw_list
            .cmds
            .iter()
            .rposition(|command| matches!(command, DrawCmd::PopClip))
            .expect("canvas content must end the viewport clip");
        let mut expected_drag_invalid = default_scheme_canvas().drag_invalid;
        expected_drag_invalid[3] *= theme.mindmap.geometry.drag_preview_alpha;
        let drag_feedback = draw_list
            .cmds
            .iter()
            .position(|command| {
                matches!(
                    command,
                    DrawCmd::FillRect { color, .. } if *color == expected_drag_invalid
                )
            })
            .expect("edge drag feedback must still be emitted inside the clip");

        assert!(clip_start < drag_feedback && drag_feedback < clip_end);
    }

    #[test]
    fn drag_preview_uses_nearest_left_node_and_same_level_after_marker() {
        let source = "# Root\n## A\n## B\n## C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let source_geometry = &view.ready_hit_map().nodes[2];
        let source_card = source_geometry.card_rect;
        let source_press_x =
            source_geometry.title_rect.x - ONE_PIXEL_BEFORE_TITLE_LEFT_EDGE - source_card.x;
        let anchor = view.ready_hit_map().nodes[3].card_rect;
        let pointer_x = anchor.x + anchor.w + source_press_x;
        let expected_insertion_y = anchor.y + anchor.h + view.constants.sibling_gap * 0.5;

        let response = view.handle_canvas_drag(
            drag_request(&view, CanvasDragPhase::Update, source_range, pointer_x, 120.0, 1),
            &doc,
        );
        assert!(matches!(response, CanvasDragResponse::Preview(preview)
            if preview.is_valid
                && preview.target_rect == Some(anchor)
                && preview.insertion_line == Some(((anchor.x, expected_insertion_y),
                    (anchor.x + anchor.w, expected_insertion_y)))));
    }

    #[test]
    fn drag_preview_projects_only_the_source_subtree_rectangles() {
        let source = "# Root\n## Source\n### SourceChild\n## Sibling\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
        let expected_source_subtree_rects =
            vec![view.ready_hit_map().nodes[1].card_rect, view.ready_hit_map().nodes[2].card_rect];

        let response = view
            .handle_canvas_drag(drag_request_after_node(&view, source_range, "Sibling", 1), &doc);

        let CanvasDragResponse::Preview(preview) = response else {
            panic!("dragging a source subtree should produce a preview");
        };
        assert_eq!(preview.source_subtree_rects, expected_source_subtree_rects);
    }

    #[test]
    fn drag_preview_reorders_root_children_with_pointer_inside_card_band() {
        let source = "# Root\n## A\n## B\n## C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "C").subtree_source_range.clone();
        let root = view.ready_hit_map().nodes[0].card_rect;
        let anchor = view.ready_hit_map().nodes[2].card_rect;

        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            anchor.x + anchor.w * 0.5,
            anchor.y,
            1,
        );
        let response = view.handle_canvas_drag(request.clone(), &doc);

        let CanvasDragResponse::Preview(preview) = response else {
            panic!("same-level drag should produce a preview");
        };
        assert!(preview.is_valid);
        assert!(preview.insertion_line.is_some());
        assert_eq!(preview.guide_to, Some((root.x + root.w, root.y + root.h * 0.5)));

        let drop_request = CanvasDragRequest { phase: CanvasDragPhase::Drop, ..request };
        let drop_response = view.handle_canvas_drag(drop_request, &doc);
        let CanvasDragResponse::Apply(transaction) = drop_response else {
            panic!("dragging a root child before a sibling should create a transaction");
        };
        assert_eq!(apply_transaction_to_text(source, &transaction), "# Root\n## A\n## C\n## B\n");
    }

    #[test]
    fn cross_level_sibling_preview_connects_to_the_target_parent() {
        let source = "# Root\n## SourceParent\n### Source\n## Target\n### A\n### B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
        let source_geometry = &view.ready_hit_map().nodes[2];
        let source_card = source_geometry.card_rect;
        let target_parent = view.ready_hit_map().nodes[3].card_rect;
        let anchor = view.ready_hit_map().nodes[5].card_rect;

        let response = view.handle_canvas_drag(
            drag_request(
                &view,
                CanvasDragPhase::Update,
                source_range,
                anchor.x + anchor.w + source_geometry.title_rect.x
                    - source_card.x
                    - ONE_PIXEL_BEFORE_TITLE_LEFT_EDGE,
                anchor.y,
                1,
            ),
            &doc,
        );

        let CanvasDragResponse::Preview(preview) = response else {
            panic!("cross-level sibling target should produce a preview");
        };
        assert!(preview.is_valid);
        assert_eq!(
            preview.guide_to,
            Some((target_parent.x + target_parent.w, target_parent.y + target_parent.h * 0.5,))
        );
        assert!(preview.insertion_line.is_some());
    }

    #[test]
    fn same_level_insertion_line_is_centered_between_neighbor_cards() {
        let source = "# Root\n## A\n## B\n## C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "C").subtree_source_range.clone();
        let previous = view.ready_hit_map().nodes[1].card_rect;
        let anchor = view.ready_hit_map().nodes[2].card_rect;
        let expected_y = (previous.y + previous.h + anchor.y) * 0.5;

        let response = view.handle_canvas_drag(
            drag_request(
                &view,
                CanvasDragPhase::Update,
                source_range,
                anchor.x + anchor.w * 0.5,
                anchor.y,
                1,
            ),
            &doc,
        );

        assert!(matches!(
            response,
            CanvasDragResponse::Preview(CanvasDragPreview {
                insertion_line: Some(((from_x, line_y), (to_x, _))),
                is_valid: true,
                ..
            }) if (from_x - anchor.x).abs() < f32::EPSILON
                && (to_x - (anchor.x + anchor.w)).abs() < f32::EPSILON
                && (line_y - expected_y).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn drag_with_left_edge_before_sibling_right_edge_stays_same_level() {
        let source = "# Root\n## A\n## B\n### B1\n## C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "C").subtree_source_range.clone();
        let anchor = view.ready_hit_map().nodes[2].card_rect;
        let expected_anchor_range =
            node_by_title(view.ready_tree(), "B").subtree_source_range.clone();

        let response = view.handle_canvas_drag(
            drag_request(
                &view,
                CanvasDragPhase::Update,
                source_range,
                anchor.x + anchor.w + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE,
                anchor.y,
                1,
            ),
            &doc,
        );

        assert!(matches!(
            response,
            CanvasDragResponse::Preview(CanvasDragPreview { is_valid: true, .. })
        ));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                anchor_range: Some(candidate_range),
                target: Some(mmf::edit::MoveSubtreeTarget::BeforeSibling),
                ..
            }) if candidate_range == expected_anchor_range
        ));
    }

    #[test]
    fn drag_with_left_edge_past_sibling_right_edge_targets_its_first_child() {
        let source = "# Root\n## A\n## B\n### B1\n## C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "C").subtree_source_range.clone();
        let anchor = view.ready_hit_map().nodes[2].card_rect;
        let source_card = view.ready_hit_map().nodes[4].card_rect;
        let child_range = node_by_title(view.ready_tree(), "B1").subtree_source_range.clone();

        let response = view.handle_canvas_drag(
            drag_request(
                &view,
                CanvasDragPhase::Update,
                source_range,
                anchor.x + anchor.w + source_card.w * 0.5 + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE,
                anchor.y,
                1,
            ),
            &doc,
        );

        assert!(matches!(
            response,
            CanvasDragResponse::Preview(CanvasDragPreview { is_valid: true, .. })
        ));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                anchor_range: Some(anchor_range),
                target: Some(mmf::edit::MoveSubtreeTarget::BeforeChild),
                ..
            }) if anchor_range == child_range
        ));
    }

    #[test]
    fn drag_parent_target_uses_the_dragged_card_left_edge() {
        let source = "# Root\n## A\n## B\n### B1\n## C with a deliberately wide title\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);

        let source_range = node_by_title(view.ready_tree(), "C with a deliberately wide title")
            .subtree_source_range
            .clone();
        let source_card = view.ready_hit_map().nodes[4].card_rect;
        let parent_card = view.ready_hit_map().nodes[2].card_rect;
        let parent_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let pressed_x = source_card.x + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE;
        let preview_left = parent_card.x + parent_card.w + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE;
        let pointer_x = pressed_x + preview_left - source_card.x;

        let preview = view
            .calculate_drag_preview(&CanvasDragRequest {
                phase: CanvasDragPhase::Update,
                source_range,
                pointer_x,
                pointer_y: parent_card.y + parent_card.h * 0.5,
                pressed_x,
                pressed_y: source_card.y,
                offset_x: 0.0,
                offset_y: 0.0,
                source_generation: 1,
            })
            .expect("laid out mindmap should calculate a drag preview");

        assert_eq!(preview.canvas.preview_rect.x, preview_left);
        assert_eq!(preview.canvas.guide_from.0, preview_left);
        assert_eq!(
            preview.canvas.guide_to,
            Some((parent_card.x + parent_card.w, parent_card.y + parent_card.h * 0.5))
        );
        assert_eq!(preview.anchor_range, Some(parent_range));
        assert_eq!(preview.target, Some(mmf::edit::MoveSubtreeTarget::LastChild));
    }

    #[test]
    fn drag_child_insertion_uses_preview_card_center_not_grab_position() {
        let source = "# Root\n## Source\n## Parent\n### First\n### Last\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);

        let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
        let source_card = view.ready_hit_map().nodes[1].card_rect;
        let parent_card = view.ready_hit_map().nodes[2].card_rect;
        let first_card = view.ready_hit_map().nodes[3].card_rect;
        let last_range = node_by_title(view.ready_tree(), "Last").subtree_source_range.clone();
        let pressed_x = source_card.x + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE;
        let preview_left = parent_card.x + parent_card.w + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE;
        let pointer_x = pressed_x + preview_left - source_card.x;
        let preview_top = first_card.y;
        let preview_for_pressed_y = |pressed_y| {
            view.calculate_drag_preview(&CanvasDragRequest {
                phase: CanvasDragPhase::Update,
                source_range: source_range.clone(),
                pointer_x,
                pointer_y: pressed_y + preview_top - source_card.y,
                pressed_x,
                pressed_y,
                offset_x: 0.0,
                offset_y: 0.0,
                source_generation: 1,
            })
            .expect("laid out mindmap should calculate a drag preview")
        };

        let top_grab_preview = preview_for_pressed_y(source_card.y);
        let bottom_grab_preview = preview_for_pressed_y(source_card.y + source_card.h - 1.0);

        // 深度缩放高度使浮点抵消不再精确，逐分量容忍微小尾差。
        let top_rect = top_grab_preview.canvas.preview_rect;
        let bottom_rect = bottom_grab_preview.canvas.preview_rect;
        assert!(
            (top_rect.x - bottom_rect.x).abs() < 0.01
                && (top_rect.y - bottom_rect.y).abs() < 0.01
                && (top_rect.w - bottom_rect.w).abs() < 0.01
                && (top_rect.h - bottom_rect.h).abs() < 0.01,
            "preview rect must not depend on grab position: {top_rect:?} vs {bottom_rect:?}"
        );
        assert_eq!(top_grab_preview.anchor_range, bottom_grab_preview.anchor_range);
        assert_eq!(top_grab_preview.anchor_range, Some(last_range));
        assert_eq!(top_grab_preview.target, Some(mmf::edit::MoveSubtreeTarget::BeforeChild));
        assert_eq!(bottom_grab_preview.target, top_grab_preview.target);
    }

    #[test]
    fn drag_to_parent_level_card_right_edge_promotes_node_to_that_level() {
        let source = "# Root\n## A\n### A1\n### A2\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "A1").subtree_source_range.clone();
        let anchor = view.ready_hit_map().nodes[4].card_rect;
        let anchor_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();

        let response = view.handle_canvas_drag(
            drag_request(
                &view,
                CanvasDragPhase::Update,
                source_range,
                anchor.x + anchor.w,
                anchor.y,
                1,
            ),
            &doc,
        );

        assert!(matches!(
            response,
            CanvasDragResponse::Preview(CanvasDragPreview { is_valid: true, .. })
        ));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                anchor_range: Some(ref candidate_range),
                target: Some(mmf::edit::MoveSubtreeTarget::BeforeSibling),
                ..
            }) if *candidate_range == anchor_range
        ));
    }

    #[test]
    fn drag_inside_nested_target_card_moves_source_to_the_target_level() {
        let source = "# Root\n## A\n### A1\n### A2\n## B\n### B1\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "A1").subtree_source_range.clone();
        let anchor = view.ready_hit_map().nodes[5].card_rect;
        let anchor_range = node_by_title(view.ready_tree(), "B1").subtree_source_range.clone();
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range.clone(),
            anchor.x + anchor.w - ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE,
            anchor.y,
            1,
        );

        assert!(matches!(
            view.handle_canvas_drag(request.clone(), &doc),
            CanvasDragResponse::Preview(CanvasDragPreview { is_valid: true, .. })
        ));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                anchor_range: Some(ref candidate_range),
                target: Some(mmf::edit::MoveSubtreeTarget::BeforeSibling),
                ..
            }) if *candidate_range == anchor_range
        ));

        let drop_request = CanvasDragRequest { phase: CanvasDragPhase::Drop, ..request };
        let CanvasDragResponse::Apply(transaction) = view.handle_canvas_drag(drop_request, &doc)
        else {
            panic!("dropping inside a target card should apply a same-level move");
        };
        assert_eq!(
            apply_transaction_to_text(source, &transaction),
            "# Root\n## A\n### A2\n## B\n### A1\n### B1\n"
        );
    }

    #[test]
    fn drag_ignores_requests_over_a_title_rect() {
        let source = "# Root\n## A\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let title = &view.ready_hit_map().nodes[2].title_rect;

        let response = view.handle_canvas_drag(
            drag_request(
                &view,
                CanvasDragPhase::Update,
                source_range,
                title.x + title.w * 0.5,
                title.y + title.h * 0.5,
                1,
            ),
            &doc,
        );

        assert!(matches!(response, CanvasDragResponse::Ignore));
    }

    #[test]
    fn drag_ignores_requests_started_on_a_title_rect() {
        let source = "# Root\n## A\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let source_title = view.ready_hit_map().nodes[2].title_rect;
        let target_card = view.ready_hit_map().nodes[1].card_rect;
        let pointer = screen_point(
            &view,
            target_card.x + target_card.w + 20.0,
            target_card.y + target_card.h + 8.0,
        );
        let pressed = screen_point(
            &view,
            source_title.x + source_title.w * 0.5,
            source_title.y + source_title.h * 0.5,
        );
        let request = CanvasDragRequest {
            pressed_x: pressed.x,
            pressed_y: pressed.y,
            ..drag_request_at_screen(
                &view,
                CanvasDragPhase::Start,
                source_range,
                pointer.x,
                pointer.y,
                1,
            )
        };

        let response = view.handle_canvas_drag(request, &doc);

        assert!(
            matches!(response, CanvasDragResponse::Ignore),
            "drag started over a title must be ignored, got {response:?}"
        );
    }

    #[test]
    fn drag_preview_marks_root_and_stale_generation_invalid() {
        let source = "# Root\n## A\n### A1\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let root_range = node_by_title(view.ready_tree(), "Root").subtree_source_range.clone();
        let stale_request = drag_request_after_node(
            &view,
            node_by_title(view.ready_tree(), "B").subtree_source_range.clone(),
            "A1",
            2,
        );

        for request in [drag_request_after_node(&view, root_range, "A1", 1), stale_request] {
            assert!(matches!(
                view.handle_canvas_drag(request, &doc),
                CanvasDragResponse::Preview(CanvasDragPreview { is_valid: false, .. })
            ));
        }
    }

    #[test]
    fn drag_preview_skips_a_nearer_descendant_for_a_farther_legal_anchor() {
        let source = "# Root\n## A\n### A1\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "A").subtree_source_range.clone();
        let descendant_card = &view.ready_hit_map().nodes[2].card_rect;
        let legal_anchor_card = &view.ready_hit_map().nodes[3].card_rect;
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            descendant_card.x + descendant_card.w + 20.0,
            legal_anchor_card.y + legal_anchor_card.h + 8.0,
            1,
        );
        let expected_target = view.ready_hit_map().nodes[3].card_rect;

        let response = view.handle_canvas_drag(request, &doc);

        assert!(matches!(response, CanvasDragResponse::Preview(preview)
            if preview.is_valid && preview.target_rect == Some(expected_target)));
    }

    #[test]
    fn drag_past_root_right_edge_moves_node_to_root_level() {
        let source = "# Root\n## A\n### A1\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "A1").subtree_source_range.clone();
        let root = view.ready_hit_map().nodes[0].card_rect;
        let source_card = view.ready_hit_map().nodes[2].card_rect;
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            root.x + root.w + source_card.w * 0.5 + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE,
            root.y,
            1,
        );

        let response = view.handle_canvas_drag(request.clone(), &doc);
        assert!(matches!(
            response,
            CanvasDragResponse::Preview(CanvasDragPreview {
                is_valid: true,
                target_rect: Some(target_rect),
                ..
            }) if target_rect == root
        ));
        let drop_request = CanvasDragRequest { phase: CanvasDragPhase::Drop, ..request };
        let CanvasDragResponse::Apply(transaction) = view.handle_canvas_drag(drop_request, &doc)
        else {
            panic!("dragging past the root should create a root-level child");
        };
        assert_eq!(apply_transaction_to_text(source, &transaction), "# Root\n## A\n## A1\n## B\n");
    }

    #[test]
    fn drag_wide_deep_node_past_root_right_edge_moves_it_to_root_level() {
        let source = "# Root\n## A\n### A node with a deliberately wide title\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range =
            node_by_title(view.ready_tree(), "A node with a deliberately wide title")
                .subtree_source_range
                .clone();
        let root = view.ready_hit_map().nodes[0].card_rect;
        let source_card = view.ready_hit_map().nodes[2].card_rect;
        let source_press_x = view.ready_hit_map().nodes[2].title_rect.x
            - ONE_PIXEL_BEFORE_TITLE_LEFT_EDGE
            - source_card.x;
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            root.x + root.w + ONE_PIXEL_BEYOND_NODE_RIGHT_EDGE + source_press_x,
            root.y,
            1,
        );

        let response = view.handle_canvas_drag(request.clone(), &doc);
        assert!(matches!(
            response,
            CanvasDragResponse::Preview(CanvasDragPreview {
                is_valid: true,
                target_rect: Some(target_rect),
                ..
            }) if target_rect == root
        ));
        let drop_request = CanvasDragRequest { phase: CanvasDragPhase::Drop, ..request };
        let CanvasDragResponse::Apply(transaction) = view.handle_canvas_drag(drop_request, &doc)
        else {
            panic!("dragging a wide node past the root should create a root-level child");
        };
        assert_eq!(
            apply_transaction_to_text(source, &transaction),
            "# Root\n## A\n## A node with a deliberately wide title\n## B\n"
        );
    }

    #[test]
    fn drag_preview_marks_adjacent_same_level_positions_invalid() {
        let source = "# Root\n## A\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let request = drag_request_on_sibling_right_edge(&view, source_range, "A", 1);

        let response = view.handle_canvas_drag(request, &doc);

        assert!(matches!(
            response,
            CanvasDragResponse::Preview(CanvasDragPreview {
                is_valid: false,
                insertion_line: None,
                ..
            })
        ));
    }

    #[test]
    fn drag_preview_uses_last_child_when_candidate_is_not_a_sibling() {
        let source = "# Root\n## A\n### A1\n## B\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let request = drag_request_after_node(&view, source_range, "A1", 1);

        let response = view.handle_canvas_drag(request, &doc);

        assert!(matches!(response, CanvasDragResponse::Preview(preview)
            if preview.is_valid && preview.insertion_line.is_none() && preview.target_rect.is_some()));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                target: Some(mmf::edit::MoveSubtreeTarget::LastChild),
                ..
            })
        ));
    }

    #[test]
    fn drag_into_parent_inserts_before_the_next_direct_child() {
        let source = "# Root\n## Source\n## Parent\n### First\n### Last\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
        let parent = view.ready_hit_map().nodes[2].card_rect;
        let first = view.ready_hit_map().nodes[3].card_rect;
        let last = view.ready_hit_map().nodes[4].card_rect;
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            pointer_past_node_right_edge(
                &view,
                &node_by_title(view.ready_tree(), "Source").subtree_source_range,
                "Parent",
            ),
            (first.y + first.h * 0.5 + last.y + last.h * 0.5) * 0.5,
            1,
        );

        let response = view.handle_canvas_drag(request, &doc);

        assert!(matches!(response, CanvasDragResponse::Preview(preview)
            if preview.is_valid
                && preview.target_rect == Some(parent)
                && preview.guide_to == Some((parent.x + parent.w, parent.y + parent.h * 0.5))
                && preview.insertion_line
                    == Some(((last.x, (first.y + first.h + last.y) * 0.5),
                        (last.x + last.w, (first.y + first.h + last.y) * 0.5)))));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                target: Some(mmf::edit::MoveSubtreeTarget::BeforeChild),
                ..
            })
        ));
    }

    #[test]
    fn dragging_a_direct_child_before_itself_is_invalid() {
        let source = "# Root\n## Parent\n### Source\n### Last\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
        let source_card = view.ready_hit_map().nodes[2].card_rect;
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            pointer_past_node_right_edge(
                &view,
                &node_by_title(view.ready_tree(), "Source").subtree_source_range,
                "Parent",
            ),
            source_card.y,
            1,
        );

        assert!(matches!(
            view.handle_canvas_drag(request, &doc),
            CanvasDragResponse::Preview(CanvasDragPreview { is_valid: false, .. })
        ));
    }

    #[test]
    fn drag_into_parent_after_the_last_direct_child_uses_last_child_target() {
        let source = "# Root\n## Source\n## Parent\n### First\n### Last\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
        let parent = view.ready_hit_map().nodes[2].card_rect;
        let last = view.ready_hit_map().nodes[4].card_rect;
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            pointer_past_node_right_edge(
                &view,
                &node_by_title(view.ready_tree(), "Source").subtree_source_range,
                "Parent",
            ),
            last.y + last.h,
            1,
        );

        let response = view.handle_canvas_drag(request, &doc);

        assert!(matches!(response, CanvasDragResponse::Preview(preview)
            if preview.is_valid
                && preview.target_rect == Some(parent)
                && preview.insertion_line.is_none()));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                target: Some(mmf::edit::MoveSubtreeTarget::LastChild),
                ..
            })
        ));
    }

    #[test]
    fn drag_into_parent_at_a_child_center_inserts_before_the_next_child() {
        let source = "# Root\n## Source\n## Parent\n### First\n### Last\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "Source").subtree_source_range.clone();
        let parent = view.ready_hit_map().nodes[2].card_rect;
        let first = view.ready_hit_map().nodes[3].card_rect;
        let last = view.ready_hit_map().nodes[4].card_rect;
        let request = drag_request(
            &view,
            CanvasDragPhase::Update,
            source_range,
            pointer_past_node_right_edge(
                &view,
                &node_by_title(view.ready_tree(), "Source").subtree_source_range,
                "Parent",
            ),
            // 严格小于半高的边界判定对浮点尾差敏感，略低于中心以确保落在下半区。
            first.y + first.h * 0.5 + 0.01,
            1,
        );

        let response = view.handle_canvas_drag(request, &doc);

        assert!(matches!(response, CanvasDragResponse::Preview(preview)
            if preview.is_valid
                && preview.target_rect == Some(parent)
                && preview.insertion_line
                    == Some(((last.x, (first.y + first.h + last.y) * 0.5),
                        (last.x + last.w, (first.y + first.h + last.y) * 0.5)))));
        assert!(matches!(
            view.drag_state,
            MindmapDragState::Preview(MindmapDragPreview {
                target: Some(mmf::edit::MoveSubtreeTarget::BeforeChild),
                ..
            })
        ));
    }

    #[test]
    fn drag_drop_applies_the_previewed_same_level_move() {
        let source = "# Root\n## A\n## B\n## C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let request = drag_request_on_sibling_right_edge(&view, source_range, "C", 1);
        assert!(matches!(
            view.handle_canvas_drag(request.clone(), &doc),
            CanvasDragResponse::Preview(CanvasDragPreview { is_valid: true, .. })
        ));
        let drop_request = CanvasDragRequest { phase: CanvasDragPhase::Drop, ..request };

        let response = view.handle_canvas_drag(drop_request, &doc);

        let CanvasDragResponse::Apply(transaction) = response else {
            panic!("valid drop should produce a source transaction");
        };
        assert_eq!(apply_transaction_to_text(source, &transaction), "# Root\n## A\n## C\n## B\n");
    }

    #[test]
    fn cancel_and_source_update_clear_drag_preview() {
        let source = "# Root\n## A\n## B\n";
        let (mut view, mut doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let source_range = node_by_title(view.ready_tree(), "B").subtree_source_range.clone();
        let request = drag_request_after_node(&view, source_range.clone(), "A", 1);
        let _ = view.handle_canvas_drag(request.clone(), &doc);
        assert!(view.build_render_projection().drag_preview.is_some());

        let cancel_request =
            CanvasDragRequest { phase: CanvasDragPhase::Cancel, ..request.clone() };
        assert!(matches!(
            view.handle_canvas_drag(cancel_request, &doc),
            CanvasDragResponse::Ignore
        ));
        assert!(view.build_render_projection().drag_preview.is_none());

        let _ = view.handle_canvas_drag(request, &doc);
        view.handle_message(
            PluginMessage::UpdateSource { text: source.to_owned(), generation: 2 },
            &mut doc,
        );
        assert!(view.build_render_projection().drag_preview.is_none());
    }

    fn laid_out_child_card_padding() -> (MindmapView, MindmapTestDoc, (f32, f32)) {
        let (mut view, doc) = view_with_source("# Root\n## Child\n");
        render_test_view(&mut view, &doc);
        let geometry = &view.ready_hit_map().nodes[1];
        let point = screen_point(&view, geometry.card_rect.x + 2.0, geometry.card_rect.y + 2.0);
        (view, doc, (point.x, point.y))
    }

    #[test]
    fn mindmap_is_an_editable_custom_renderer() {
        let view = MindmapView::new();
        assert!(view.allows_editing());
        assert!(view.handles_own_rendering());
        assert!(!view.shows_cursor());
        assert!(view.needs_cursor_blink_wakeup());
    }

    #[test]
    fn theme_query_distinguishes_default_selected_unknown_and_invalid_metadata() {
        for (source, expected) in [
            ("# Root\n", MindmapThemeSelection::Default),
            (
                "```toml mindmap\ntheme = \"tide\"\n```\n# Root\n",
                MindmapThemeSelection::Selected("tide".into()),
            ),
            (
                "```toml mindmap\ntheme = \"future\"\n```\n# Root\n",
                MindmapThemeSelection::Unknown("future".into()),
            ),
            ("```toml mindmap\ntheme = [\n```\n# Root\n", MindmapThemeSelection::InvalidMetadata),
        ] {
            let (view, doc) = view_with_source(source);
            assert!(matches!(
                view.query(PluginQuery::MindmapThemeSelection, &doc),
                PluginResponse::MindmapThemeSelection(actual) if actual == expected
            ));
        }
    }

    #[test]
    fn theme_plan_query_rejects_unknown_scheme_and_plans_known_scheme() {
        let (view, doc) = view_with_source("# Root\n");
        assert!(matches!(
            view.query(
                PluginQuery::PlanMindmapTheme { theme_id: "future".into(), source_generation: 1 },
                &doc,
            ),
            PluginResponse::EditPlan(EditPlan::Consume)
        ));
        assert!(matches!(
            view.query(
                PluginQuery::PlanMindmapTheme {
                    theme_id: "tide".into(),
                    source_generation: 1,
                },
                &doc,
            ),
            PluginResponse::EditPlan(EditPlan::Apply(transaction))
                if transaction.source_generation == 1
        ));
    }

    fn rendered_card_rects(draw_list: &DrawList) -> Vec<Rect> {
        draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::FillRect { rect, radius, .. } if *radius > 0.0 => Some(*rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn file_theme_rendering_is_fixed_across_application_theme_modes() {
        let source = "```toml mindmap\ntheme = \"dawn\"\n```\n# Root\n## Child\n";
        let (mut dark_view, dark_doc) = view_with_source(source);
        let (mut light_view, light_doc) = view_with_source(source);
        let dark_app_theme = Theme::from_definition(&ThemeDefinition::default_dark());
        let light_app_theme = Theme::from_definition(&ThemeDefinition::default_light());
        let dark_draw =
            render_test_draw_list_with_theme(&mut dark_view, &dark_doc, &dark_app_theme);
        let light_draw =
            render_test_draw_list_with_theme(&mut light_view, &light_doc, &light_app_theme);
        let dawn =
            ui::theme::find_mindmap_color_scheme("dawn").expect("dawn is a built-in mmap scheme");

        for draw_list in [&dark_draw, &light_draw] {
            assert!(draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::FillRect { color, .. } if *color == dawn.canvas.background
            )));
            let branch = dawn.canvas.branch_color(0).expect("dawn has branch colors");
            assert!(draw_list.cmds.iter().any(|command| matches!(
                command,
                DrawCmd::TaperedMesh { color, .. } if *color == branch
            )));
        }
        assert_eq!(rendered_card_rects(&dark_draw), rendered_card_rects(&light_draw));
    }

    #[test]
    fn generation_zero_requests_initial_source_update_only_once() {
        let mut view = MindmapView::new();
        let mut doc = MindmapTestDoc::new("# Root\n");

        assert!(matches!(
            view.query(PluginQuery::NeedsSourceUpdate(0), &doc),
            PluginResponse::Bool(true)
        ));

        view.handle_message(
            PluginMessage::UpdateSource { text: "# Root\n".into(), generation: 0 },
            &mut doc,
        );
        assert!(matches!(
            view.query(PluginQuery::NeedsSourceUpdate(0), &doc),
            PluginResponse::Bool(false)
        ));
        assert!(matches!(view.document_state, MindmapDocumentState::Ready { .. }));
    }

    #[test]
    fn exact_subtree_selection_derives_node_selected_focus() {
        let source = "# Root\n## Child\n";
        let (view, _doc) = view_with_source(source);
        let child_range = view.ready_tree().root.children[0].subtree_source_range.clone();
        let focus = view.derive_focus(child_range.end, Some(child_range.clone()));
        assert!(matches!(focus, MindmapFocus::NodeSelected { node_index: 1 }));
    }

    #[test]
    fn card_padding_hit_returns_source_object() {
        let (view, _doc, point) = laid_out_child_card_padding();
        let expected = view.ready_tree().root.children[0].subtree_source_range.clone();
        let response = view.semantic_hit_target(point.0, point.1);
        assert!(matches!(
            response,
            Some(EditHitTarget::SourceObject { source_range })
                if source_range == expected
        ));
    }

    #[test]
    fn blank_space_to_the_right_of_title_selects_node() {
        let (mut view, doc) = view_with_source("# Root\n## 知识库\n## 一个很长的同级标题\n");
        render_test_view(&mut view, &doc);
        let geometry = &view.ready_hit_map().nodes[1];
        let title_visual_end = *geometry
            .grapheme_edges
            .last()
            .expect("non-empty title must have a final grapheme edge");
        let content_point = CanvasPoint::new(
            title_visual_end + 1.0,
            geometry.title_rect.y + geometry.title_rect.h * 0.5,
        );
        assert!(
            geometry.card_rect.contains(content_point.x, content_point.y),
            "regression point must be inside the node card"
        );

        let point = screen_point(&view, content_point.x, content_point.y);
        let expected = view.ready_tree().root.children[0].subtree_source_range.clone();
        let response = view.semantic_hit_target(point.x, point.y);
        assert!(matches!(
            response,
            Some(EditHitTarget::SourceObject { source_range }) if source_range == expected
        ));
    }

    #[test]
    fn empty_title_projects_placeholder_without_changing_source_range() {
        let (mut view, mut doc) = view_with_source("# Root\n##\n");
        render_test_view(&mut view, &doc);
        let empty_title_byte = view.ready_tree().root.children[0].title_byte_range.start;
        view.handle_message(PluginMessage::SetCursorByte(empty_title_byte), &mut doc);

        let projection = view.build_render_projection();
        assert_eq!(projection.projected_title(1), mmf::canvas::EMPTY_TITLE_PLACEHOLDER);
        let title_range = &view.ready_tree().root.children[0].title_byte_range;
        assert_eq!(title_range.start, title_range.end);
    }

    #[test]
    fn empty_placeholder_navigation_stays_at_the_zero_length_source_range() {
        let (mut view, mut doc) = view_with_source("# Root\n##\n");
        render_test_view(&mut view, &doc);
        let title_start = view.ready_tree().root.children[0].title_byte_range.start;
        view.handle_message(PluginMessage::SetCursorByte(title_start), &mut doc);

        assert!(matches!(
            view.move_edit_target(title_start, MoveDirection::Right),
            Some(EditHitTarget::TextCaret { byte_offset, .. }) if byte_offset == title_start
        ));
    }

    #[test]
    fn node_selection_projects_card_highlight_without_text_caret() {
        let (mut view, mut doc) = view_with_source("# Root\n## Child\n");
        let range = view.ready_tree().root.children[0].subtree_source_range.clone();
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(range.start)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(range.end)), &mut doc);
        view.handle_message(PluginMessage::SetCursorByte(range.end), &mut doc);

        let projection = view.build_render_projection();
        assert_eq!(projection.selected_node_index(), Some(1));
        assert!(projection.caret().is_none());
    }

    #[test]
    fn selected_node_preedit_replaces_visual_title() {
        let (mut view, mut doc) = view_with_source("# Root\n## Original\n");
        let range = view.ready_tree().root.children[0].subtree_source_range.clone();
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(range.start)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(range.end)), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut doc,
        );

        assert_eq!(view.build_render_projection().projected_title(1), "ni");
    }

    #[test]
    fn selected_node_preedit_exposes_the_projected_composition_caret() {
        let (mut view, mut doc) = view_with_source("# Root\n## Child\n");
        let child = &view.ready_tree().root.children[0];
        let selection = child.subtree_source_range.clone();
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(selection.start)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(selection.end)), &mut doc);
        view.handle_message(PluginMessage::SetCursorByte(selection.end), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "你".into(), cursor: Some((0, 3)) },
            &mut doc,
        );
        render_test_view(&mut view, &doc);

        let projection = view.build_render_projection();
        assert_eq!(projection.composition_caret(), Some((1, "你".len())));
        let geometry = &view.ready_hit_map().nodes[1];
        let expected_x = screen_point(&view, geometry.grapheme_edges[1], geometry.title_rect.y).x;
        assert!(matches!(
            view.query(PluginQuery::CursorScreenPos(selection.end), &doc),
            PluginResponse::CursorScreenRect(Some((x, _, _, _)))
                if (x - expected_x).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn preedit_cursor_screen_position_uses_projected_grapheme_geometry() {
        let (mut view, mut doc) = view_with_source("# Root\n## Child\n");
        let title_start = view.ready_tree().root.children[0].title_byte_range.start;
        view.handle_message(PluginMessage::SetCursorByte(title_start + 1), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "你".into(), cursor: Some((0, 3)) },
            &mut doc,
        );
        render_test_view(&mut view, &doc);

        let projection = view.build_render_projection();
        assert_eq!(projection.caret(), Some((1, 4)));
        let geometry = &view.ready_hit_map().nodes[1];
        let expected_x = screen_point(&view, geometry.grapheme_edges[2], geometry.title_rect.y).x;
        assert!(matches!(
            view.query(PluginQuery::CursorScreenPos(title_start + 1), &doc),
            PluginResponse::CursorScreenRect(Some((x, _, _, _))) if (x - expected_x).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn selected_title_preedit_projects_a_caret_and_local_candidate_position() {
        let (mut view, mut doc) = view_with_source("# Root\n## Child\n");
        let title_start = view.ready_tree().root.children[0].title_byte_range.start;
        let selection = title_start + 1..title_start + 4;
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(selection.start)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(selection.end)), &mut doc);
        view.handle_message(PluginMessage::SetCursorByte(selection.end), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "你".into(), cursor: Some((0, 3)) },
            &mut doc,
        );
        let mut shaper = Shaper::new().expect("test shaper");
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let _ = view.render(&doc, Rect::new(100.0, 80.0, 800.0, 600.0), &theme, &mut shaper, 1.0);

        let projection = view.build_render_projection();
        assert_eq!(projection.caret(), Some((1, 4)));
        let geometry = &view.ready_hit_map().nodes[1];
        let expected = screen_rect(
            &view,
            Rect::new(
                geometry.grapheme_edges[2],
                geometry.title_rect.y,
                mmf::canvas::CARET_WIDTH,
                geometry.title_rect.h,
            ),
        );
        let viewport = rendered_viewport(&view);
        assert!(matches!(
            view.query(PluginQuery::CursorScreenPos(selection.end), &doc),
            PluginResponse::CursorScreenRect(Some((x, y, _, _)))
                if (x - (expected.x - viewport.viewport.x)).abs() < f32::EPSILON
                    && (y - (expected.y - viewport.viewport.y)).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn hidden_blink_keeps_selected_title_preedit_candidate_position_projected() {
        let (mut view, mut doc) = view_with_source("# Root\n## Child\n");
        let title_start = view.ready_tree().root.children[0].title_byte_range.start;
        let selection = title_start + 1..title_start + 2;
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(selection.start)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(selection.end)), &mut doc);
        view.handle_message(PluginMessage::SetCursorByte(selection.end), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "你".into(), cursor: Some((0, 3)) },
            &mut doc,
        );
        view.handle_message(PluginMessage::SetCursorVisible(false), &mut doc);
        render_test_view(&mut view, &doc);

        assert!(view.build_render_projection().caret().is_none());
        let geometry = &view.ready_hit_map().nodes[1];
        let expected_x = screen_point(&view, geometry.grapheme_edges[2], geometry.title_rect.y).x;
        assert!(matches!(
            view.query(PluginQuery::CursorScreenPos(selection.end), &doc),
            PluginResponse::CursorScreenRect(Some((x, _, _, _)))
                if (x - expected_x).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn cursor_screen_position_is_relative_to_plugin_bounds() {
        let (mut view, doc) = view_with_source("# Root\n");
        let title_start = view.ready_tree().root.title_byte_range.start;
        view.cursor_byte = Some(title_start);
        let mut shaper = Shaper::new().expect("test shaper");
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let _ = view.render(&doc, Rect::new(120.0, 80.0, 800.0, 600.0), &theme, &mut shaper, 1.0);

        let geometry = &view.ready_hit_map().nodes[0];
        let expected = screen_rect(
            &view,
            Rect::new(
                geometry.grapheme_edges[0],
                geometry.title_rect.y,
                mmf::canvas::CARET_WIDTH,
                geometry.title_rect.h,
            ),
        );
        let viewport = rendered_viewport(&view);
        assert!(matches!(
            view.query(PluginQuery::CursorScreenPos(title_start), &doc),
            PluginResponse::CursorScreenRect(Some((x, y, _, _)))
                if (x - (expected.x - viewport.viewport.x)).abs() < f32::EPSILON
                    && (y - (expected.y - viewport.viewport.y)).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn preedit_title_hit_keeps_the_source_caret_in_range() {
        let (mut view, mut doc) = view_with_source("# Root\n## A\n");
        let title_start = view.ready_tree().root.children[0].title_byte_range.start;
        view.handle_message(PluginMessage::SetCursorByte(title_start), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "你".into(), cursor: Some((0, 3)) },
            &mut doc,
        );
        render_test_view(&mut view, &doc);
        let geometry = &view.ready_hit_map().nodes[1];

        let point = screen_point(&view, geometry.grapheme_edges[2], geometry.title_rect.y);
        let response = view.semantic_hit_target(point.x, point.y);
        assert!(matches!(
            response,
            Some(EditHitTarget::TextCaret { byte_offset, .. })
                if byte_offset == title_start
        ));
    }

    #[test]
    fn down_from_a_title_moves_to_the_next_visible_dfs_node() {
        let (mut view, mut doc) = view_with_source("# Root\n## Parent\n### Child\n## Next\n");
        let tree = view.ready_tree();
        let child_start = tree.root.children[0].children[0].title_byte_range.start;
        let next_range = tree.root.children[1].subtree_source_range.clone();
        view.handle_message(PluginMessage::SetCursorByte(child_start), &mut doc);

        assert_eq!(
            view.move_edit_target(child_start, MoveDirection::Down),
            Some(EditHitTarget::SourceObject { source_range: next_range })
        );
    }

    #[test]
    fn invalid_source_discards_previous_layout() {
        let (mut view, mut doc) = view_with_source("# Root\n");
        render_test_view(&mut view, &doc);
        view.handle_message(
            PluginMessage::UpdateSource { text: String::new(), generation: 2 },
            &mut doc,
        );

        assert!(matches!(view.document_state, MindmapDocumentState::Invalid { .. }));
    }

    #[test]
    fn invalid_mmap_reports_no_canvas_metrics_and_valid_recovery_restores_them() {
        const INVALID_SOURCE: &str = "## Orphan\n";
        const VALID_SOURCE: &str = "# Root\n## Recovered\n";

        let (mut view, mut doc) = view_with_source("# Root\n");
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut shaper = Shaper::new().expect("test shaper should initialize");
        view.handle_message(
            PluginMessage::UpdateSource { text: INVALID_SOURCE.to_owned(), generation: 2 },
            &mut doc,
        );

        assert!(
            view.prepare_canvas(&doc, &theme, &mut shaper, 1.0).is_none(),
            "an invalid mmap must not expose stale canvas metrics or scrollbars"
        );

        view.handle_message(
            PluginMessage::UpdateSource { text: VALID_SOURCE.to_owned(), generation: 3 },
            &mut doc,
        );
        let metrics = view
            .prepare_canvas(&doc, &theme, &mut shaper, 1.0)
            .expect("a corrected mmap must restore valid canvas metrics");
        assert!(metrics.content_bounds.w > 0.0 && metrics.content_bounds.h > 0.0);
    }

    #[test]
    fn clear_edit_focus_detaches_from_a_final_title_without_a_newline() {
        let (mut view, mut doc) = view_with_source("# Root");
        let title_end = view.ready_tree().root.title_byte_range.end;
        view.handle_message(PluginMessage::SetCursorByte(title_end), &mut doc);
        assert!(matches!(view.render_focus(), MindmapFocus::TitleEditing { .. }));

        view.handle_message(PluginMessage::ClearEditFocus, &mut doc);

        assert_eq!(view.render_focus(), MindmapFocus::None);
        assert_eq!(view.move_edit_target(title_end, MoveDirection::Left), None);
    }

    #[test]
    fn collapse_control_hit_precedes_card_hit_testing() {
        let (mut view, doc) = view_with_source("# Root\n## Branch\n### Child\n");
        render_test_view(&mut view, &doc);
        let control =
            view.ready_hit_map().controls.first().expect("branch should expose a collapse control");
        let point = screen_point(
            &view,
            control.bounds.x + control.bounds.w * 0.5,
            control.bounds.y + control.bounds.h * 0.5,
        );

        assert!(matches!(
            view.semantic_hit_target(point.x, point.y),
            Some(EditHitTarget::CanvasControl { .. })
        ));
    }

    #[test]
    fn nested_control_does_not_mask_parent_or_child_card_edges() {
        const CARD_EDGE_INSET_DP: f32 = 0.01;

        let (mut view, doc) = view_with_source("# Root\n## Branch\n### Child\n");
        render_test_view(&mut view, &doc);
        let branch = &view.ready_hit_map().nodes[1];
        let child = &view.ready_hit_map().nodes[2];
        let control =
            view.ready_hit_map().controls.first().expect("branch should expose a collapse control");
        let branch_range = view.ready_tree().root.children[0].subtree_source_range.clone();
        let child_range =
            view.ready_tree().root.children[0].children[0].subtree_source_range.clone();
        let branch_edge = screen_point(
            &view,
            branch.card_rect.right() - CARD_EDGE_INSET_DP,
            branch.card_rect.y + branch.card_rect.h * 0.5,
        );
        let child_edge = screen_point(
            &view,
            child.card_rect.x + CARD_EDGE_INSET_DP,
            child.card_rect.y + child.card_rect.h * 0.5,
        );
        let control_center = screen_point(
            &view,
            control.bounds.x + control.bounds.w * 0.5,
            control.bounds.y + control.bounds.h * 0.5,
        );

        assert_eq!(
            view.semantic_hit_target(branch_edge.x, branch_edge.y),
            Some(EditHitTarget::SourceObject { source_range: branch_range })
        );
        assert_eq!(
            view.semantic_hit_target(child_edge.x, child_edge.y),
            Some(EditHitTarget::SourceObject { source_range: child_range })
        );
        assert!(matches!(
            view.semantic_hit_target(control_center.x, control_center.y),
            Some(EditHitTarget::CanvasControl { .. })
        ));
    }

    #[test]
    fn collapsed_projection_records_full_descendant_count_and_control_plan() {
        let source =
            "# Root\n## Child\n```toml node\ncollapsed = true\n```\n### A\n#### B\n### C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let projection = view.build_render_projection();
        assert_eq!(projection.collapsed_descendant_counts[1], Some(3));
        assert!(view.ready_hit_map().controls.iter().all(|control| control.source_node_index != 0));

        let child_range = view.ready_tree().root.children[0].subtree_source_range.clone();
        assert!(matches!(
            view.query(
                PluginQuery::PlanCanvasControl { source_range: child_range, source_generation: 1 },
                &doc,
            ),
            PluginResponse::EditPlan(EditPlan::Apply(_))
        ));
    }

    #[test]
    fn collapse_end_to_end() {
        let source = "# Root\n## Parent\n### Child\n#### Grandchild\n";
        let (mut view, mut doc) = view_with_source(source);
        render_test_view(&mut view, &doc);

        let parent = node_by_title(view.ready_tree(), "Parent");
        let parent_range = parent.subtree_source_range.clone();
        let parent_control = view
            .ready_hit_map()
            .controls
            .iter()
            .find(|control| control.source_node_index == 1)
            .expect("parent must expose a collapse control");
        let parent_control_point = screen_point(
            &view,
            parent_control.bounds.x + parent_control.bounds.w * 0.5,
            parent_control.bounds.y + parent_control.bounds.h * 0.5,
        );
        assert_eq!(
            view.semantic_hit_target(parent_control_point.x, parent_control_point.y),
            Some(EditHitTarget::CanvasControl { source_range: parent_range.clone() })
        );

        let root_range = view.ready_tree().root.subtree_source_range.clone();
        assert!(view.ready_hit_map().controls.iter().all(|control| control.source_node_index != 0));
        assert!(matches!(
            view.query(
                PluginQuery::PlanCanvasControl { source_range: root_range, source_generation: 1 },
                &doc,
            ),
            PluginResponse::EditPlan(EditPlan::Consume)
        ));

        let PluginResponse::EditPlan(EditPlan::Apply(transaction)) = view.query(
            PluginQuery::PlanCanvasControl { source_range: parent_range, source_generation: 1 },
            &doc,
        ) else {
            panic!("parent control must produce a toggle transaction");
        };
        let next_source = apply_transaction_to_text(source, &transaction);
        assert!(next_source.contains("collapsed = true"));
        doc = MindmapTestDoc::new(&next_source);
        view.handle_message(
            PluginMessage::UpdateSource { text: next_source, generation: 2 },
            &mut doc,
        );
        render_test_view(&mut view, &doc);

        let visible_source_indices = view
            .ready_hit_map()
            .nodes
            .iter()
            .map(|geometry| geometry.source_node_index)
            .collect::<Vec<_>>();
        assert_eq!(visible_source_indices, vec![0, 1]);
        let draw_list = render_test_draw_list(&mut view, &doc);
        let rendered_text = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(rendered_text.contains(&"Parent"));
        assert!(rendered_text.contains(&"2"));
        assert!(!rendered_text.contains(&" · 2"));

        let drag_request = drag_request(
            &view,
            CanvasDragPhase::Update,
            view.ready_tree()
                .root
                .children
                .first()
                .expect("parent node")
                .subtree_source_range
                .clone(),
            700.0,
            120.0,
            2,
        );
        let CanvasDragResponse::Preview(preview) =
            view.handle_canvas_drag(drag_request.clone(), &doc)
        else {
            panic!("dragging the visible parent must produce a preview");
        };
        assert_eq!(preview.label, "Parent · 2");

        let child_range = view
            .ready_tree()
            .root
            .children
            .first()
            .and_then(|parent| parent.children.first())
            .expect("child node")
            .subtree_source_range
            .clone();
        let grandchild_range = view
            .ready_tree()
            .root
            .children
            .first()
            .and_then(|parent| parent.children.first())
            .and_then(|child| child.children.first())
            .expect("grandchild node")
            .subtree_source_range
            .clone();
        let calculated_preview =
            view.calculate_drag_preview(&drag_request).expect("parent preview must calculate");
        assert!(calculated_preview.canvas.is_valid, "parent drag preview must be valid");
        let anchor_range = calculated_preview
            .anchor_range
            .as_ref()
            .expect("valid parent drag preview must have an anchor");
        assert_ne!(anchor_range, &child_range, "collapsed Child must not be a drag anchor");
        assert_ne!(
            anchor_range, &grandchild_range,
            "collapsed Grandchild must not be a drag anchor"
        );
    }

    #[test]
    fn root_collapsed_property_does_not_render_a_descendant_count() {
        let source = "# Root\n```toml node\ncollapsed = true\n```\n## Child\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);

        assert_eq!(view.build_render_projection().collapsed_descendant_counts[0], None);
        assert_eq!(view.ready_hit_map().nodes.len(), 2);
        assert!(view.ready_hit_map().controls.iter().all(|control| control.source_node_index != 0));
    }

    #[test]
    fn drag_preview_label_contains_full_descendant_count() {
        let source = "# Root\n## Child\n### A\n#### B\n### C\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let child_range = view.ready_tree().root.children[0].subtree_source_range.clone();
        let request = drag_request(&view, CanvasDragPhase::Update, child_range, 700.0, 120.0, 1);
        let CanvasDragResponse::Preview(preview) = view.handle_canvas_drag(request, &doc) else {
            panic!("drag should produce a preview");
        };
        assert_eq!(preview.label, "Child · 3");
    }

    #[test]
    fn drag_preview_width_covers_unicode_label_suffix() {
        let source = "# Root\n## 子节点\n### 后代\n";
        let (mut view, doc) = view_with_source(source);
        render_test_view(&mut view, &doc);
        let child = view
            .ready_hit_map()
            .nodes
            .iter()
            .find(|geometry| geometry.source_node_index == 1)
            .expect("child geometry");
        let source_width = child.card_rect.w;
        let child_range = view.ready_tree().root.children[0].subtree_source_range.clone();
        let request = drag_request(&view, CanvasDragPhase::Update, child_range, 700.0, 120.0, 1);
        let CanvasDragResponse::Preview(preview) = view.handle_canvas_drag(request, &doc) else {
            panic!("drag should produce a preview");
        };

        assert_eq!(preview.label, "子节点 · 1");
        assert!(preview.preview_rect.w >= source_width);
    }

    #[test]
    fn preview_card_width_scales_with_source_depth() {
        let constants = LayoutConstants::default();
        assert!(
            constants.font_scale_for_depth(0) > constants.font_scale_for_depth(2),
            "test requires root depth to render at a larger font size"
        );
        let label = "Branch · 3";
        let root_width = measured_preview_card_width(label, 0.0, &constants, 16.0, 0);
        let nested_width = measured_preview_card_width(label, 0.0, &constants, 16.0, 2);
        assert!(
            root_width > nested_width,
            "root preview width {root_width} should exceed depth-2 width {nested_width}"
        );
    }

    #[test]
    fn card_hover_highlights_fill_border_and_control_symbol() {
        let (mut view, mut doc) = view_with_source("# Root\n## Branch\n### Child\n");
        render_test_view(&mut view, &doc);
        let branch = view
            .ready_hit_map()
            .nodes
            .iter()
            .find(|geometry| geometry.source_node_index == 1)
            .expect("branch geometry");
        let branch_card = branch.card_rect;
        let branch_center = screen_point(
            &view,
            branch_card.x + branch_card.w * 0.5,
            branch_card.y + branch_card.h * 0.5,
        );
        view.handle_message(PluginMessage::SetCanvasPointer(Some(branch_center)), &mut doc);
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let draw_list = render_test_draw_list_with_theme(&mut view, &doc, &theme);
        let branch_screen_rect =
            view.canvas_viewport.expect("rendered").content_rect_to_screen(branch_card);
        let control_screen_rect = view
            .canvas_viewport
            .expect("rendered")
            .content_rect_to_screen(view.ready_hit_map().controls[0].bounds);
        let branch_color = default_scheme_canvas().branch_color(0).expect("palette is non-empty");
        let control_hover_color = hovered_control_color(default_scheme_canvas(), branch_color);
        // hover 边框跟随节点样式色（分支色），不再使用固定 hover 色
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { rect, color, .. }
                if *rect == branch_screen_rect && *color == branch_color
        )));
        let hovered_fill_alpha = draw_list.cmds.iter().find_map(|command| match command {
            DrawCmd::FillRect { rect, color, .. } if *rect == branch_screen_rect => Some(color[3]),
            _ => None,
        });
        let hovered_fill_alpha = hovered_fill_alpha.expect("hovered branch fill");
        assert!(hovered_fill_alpha > crate::mmf::canvas::BRANCH_TINT_FILL_ALPHA);
        assert!(hovered_fill_alpha <= 1.0);
        assert!(draw_list_has_expanded_control_bar(
            &draw_list,
            control_screen_rect,
            control_hover_color,
        ));
    }

    #[test]
    fn control_hover_highlights_its_card_and_control() {
        let (mut view, mut doc) = view_with_source("# Root\n## Branch\n### Child\n");
        render_test_view(&mut view, &doc);
        let control = view.ready_hit_map().controls[0].bounds;
        let branch_card = view
            .ready_hit_map()
            .nodes
            .iter()
            .find(|geometry| geometry.source_node_index == 1)
            .expect("branch geometry")
            .card_rect;
        let pointer = screen_point(&view, control.x + control.w * 0.5, control.y + control.h * 0.5);
        view.handle_message(PluginMessage::SetCanvasPointer(Some(pointer)), &mut doc);
        let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let draw_list = render_test_draw_list_with_theme(&mut view, &doc, &theme);
        let branch_screen_rect =
            view.canvas_viewport.expect("rendered").content_rect_to_screen(branch_card);
        let control_screen_rect =
            view.canvas_viewport.expect("rendered").content_rect_to_screen(control);
        let branch_color = default_scheme_canvas().branch_color(0).expect("palette is non-empty");
        let control_hover_color = hovered_control_color(default_scheme_canvas(), branch_color);
        let branch_fill_alpha = draw_list.cmds.iter().find_map(|command| match command {
            DrawCmd::FillRect { rect, color, .. } if *rect == branch_screen_rect => Some(color[3]),
            _ => None,
        });
        let branch_fill_alpha = branch_fill_alpha.expect("hovered branch fill");
        assert!(branch_fill_alpha > crate::mmf::canvas::BRANCH_TINT_FILL_ALPHA);
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { rect, color, .. }
                if *rect == branch_screen_rect && *color == branch_color
        )));
        assert!(draw_list_has_expanded_control_bar(
            &draw_list,
            control_screen_rect,
            control_hover_color,
        ));
    }
}
