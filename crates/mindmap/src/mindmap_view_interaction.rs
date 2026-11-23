use super::*;

impl MindmapView {
    pub(in crate::mindmap_view) fn drag_request_hits_title(
        &self,
        request: &CanvasDragRequest,
    ) -> bool {
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

    pub(in crate::mindmap_view) fn calculate_drag_preview(
        &self,
        request: &CanvasDragRequest,
    ) -> Option<MindmapDragPreview> {
        let MindmapDocumentState::Ready {
            generation,
            tree,
            layout: Some(layout),
            drag_preview_cards,
            ..
        } = &self.document_state
        else {
            return None;
        };
        let nodes = collect_nodes_dfs(&tree.root);
        let source_index =
            nodes.iter().position(|node| node.subtree_source_range == request.source_range)?;
        let source_node = *nodes.get(source_index)?;
        let source_layout = layout_node_for_source(layout, source_index)?;
        let card = &drag_preview_cards[source_layout.node_idx];
        let label = drag_preview_label(&source_node.title, card.descendant_count);
        let source_subtree_indices = source_index..=source_index + card.descendant_count;
        let source_subtree_rects = layout
            .nodes
            .iter()
            .filter(|node| source_subtree_indices.contains(&node.source_node_index))
            .map(layout_rect)
            .collect();
        let pointer = self.screen_to_content(request.pointer_x, request.pointer_y)?;
        let pressed = self.screen_to_content(request.pressed_x, request.pressed_y)?;
        let preview_rect = Rect::new(
            pointer.x - (pressed.x - source_layout.x),
            pointer.y - (pressed.y - source_layout.y),
            card.width,
            source_layout.h,
        );
        let preview_center_y = preview_rect.y + preview_rect.h * 0.5;
        let source_rect = layout_rect(source_layout);
        let candidate = nearest_drag_candidate(layout, source_subtree_indices, preview_rect);
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
            canvas.target_rect = Some(layout_rect(candidate));
            let candidate_index = candidate.source_node_index;
            let same_level = preview_rect.x <= candidate.x + candidate.w;
            let (anchor_index, candidate_target) = if same_level {
                if preview_center_y < candidate.y + candidate.h * 0.5 {
                    (candidate_index, mmf::edit::MoveSubtreeTarget::BeforeSibling)
                } else {
                    (candidate_index, mmf::edit::MoveSubtreeTarget::AfterSibling)
                }
            } else if let Some(child_index) =
                next_child_at_or_below_y(tree, layout, candidate_index, preview_center_y)
            {
                (child_index, mmf::edit::MoveSubtreeTarget::BeforeChild)
            } else {
                (candidate_index, mmf::edit::MoveSubtreeTarget::LastChild)
            };
            let target_parent_index = match candidate_target {
                mmf::edit::MoveSubtreeTarget::BeforeSibling
                | mmf::edit::MoveSubtreeTarget::AfterSibling => find_parent(tree, anchor_index)
                    .and_then(|parent| nodes.iter().position(|node| std::ptr::eq(*node, parent))),
                mmf::edit::MoveSubtreeTarget::BeforeChild => Some(candidate_index),
                mmf::edit::MoveSubtreeTarget::LastChild => Some(anchor_index),
            };
            if let Some(parent) =
                target_parent_index.and_then(|index| layout_node_for_source(layout, index))
            {
                canvas.guide_to = Some((parent.x + parent.w, parent.y + parent.h * 0.5));
            }
            let valid = request.source_generation == *generation
                && source_index != 0
                && !is_noop_sibling_target(tree, source_index, anchor_index, candidate_target);
            if valid {
                let anchor_node = nodes[anchor_index];
                anchor_range = Some(anchor_node.subtree_source_range.clone());
                target = Some(candidate_target);
                canvas.is_valid = true;
                if let Some(insertion_line) =
                    insertion_line(layout, tree, anchor_index, candidate_target, &self.constants)
                {
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

    pub(in crate::mindmap_view) fn semantic_hit_target(
        &self,
        x: f32,
        y: f32,
    ) -> Option<EditHitTarget> {
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

    pub(in crate::mindmap_view) fn cursor_screen_pos(
        &self,
        byte_offset: usize,
    ) -> Option<(f32, f32, f32, f32)> {
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

    pub(in crate::mindmap_view) fn move_edit_target(
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

    pub(in crate::mindmap_view) fn node_navigation_target(
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

    pub(in crate::mindmap_view) fn title_navigation_target(
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

    pub(in crate::mindmap_view) fn title_selection_navigation_target(
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

    pub(in crate::mindmap_view) fn title_caret_navigation(
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

    pub(in crate::mindmap_view) fn adjacent_node_target(
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

    pub(in crate::mindmap_view) fn visible_dfs_neighbor(
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
}

fn nearest_drag_candidate(
    layout: &LayoutTree,
    source_subtree_indices: std::ops::RangeInclusive<usize>,
    preview_rect: Rect,
) -> Option<&mmf::layout::LayoutNode> {
    layout
        .nodes
        .iter()
        .filter(|node| {
            !source_subtree_indices.contains(&node.source_node_index)
                && (node.source_node_index != 0 || preview_rect.x > node.x + node.w)
        })
        .min_by(|left, right| {
            drag_distance(left, preview_rect)
                .total_cmp(&drag_distance(right, preview_rect))
                .then_with(|| left.source_node_index.cmp(&right.source_node_index))
        })
}

pub(in crate::mindmap_view) fn next_child_at_or_below_y(
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

pub(in crate::mindmap_view) fn drag_distance(
    node: &mmf::layout::LayoutNode,
    preview_rect: Rect,
) -> f32 {
    let dx = preview_rect.x - (node.x + node.w);
    let dy = preview_rect.y + preview_rect.h * 0.5 - (node.y + node.h * 0.5);
    dx * dx + dy * dy
}

pub(in crate::mindmap_view) fn is_noop_sibling_target(
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

pub(in crate::mindmap_view) fn insertion_line(
    layout: &LayoutTree,
    tree: &Tree,
    anchor_index: usize,
    target: mmf::edit::MoveSubtreeTarget,
    constants: &LayoutConstants,
) -> Option<((f32, f32), (f32, f32))> {
    let anchor = layout_node_for_source(layout, anchor_index)?;
    let sibling_indices = find_siblings(tree, anchor_index)?;
    let sibling_position = sibling_indices.iter().position(|index| *index == anchor_index)?;
    let sibling_gap = find_parent(tree, anchor_index)
        .and_then(|parent| {
            collect_nodes_dfs(&tree.root).iter().position(|node| std::ptr::eq(*node, parent))
        })
        .and_then(|parent_index| layout_node_for_source(layout, parent_index))
        .map(|parent| constants.sibling_gap_for_parent_depth(parent.depth))
        .unwrap_or(constants.sibling_gap);
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
