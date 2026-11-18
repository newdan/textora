use super::*;

impl<S: BlockSource> PreviewEngine<S> {
    /// 扩展被 Markdown 布局剥离的行尾空格，令其光标仍归属原文本行。
    pub(in crate::view) fn hit_test_line_right(&self, flat_line: &crate::layout::FlatLine) -> f32 {
        let text_right = flat_line.rect.x
            + crate::layout::grapheme_x(
                flat_line,
                crate::grapheme_map::grapheme_count(&flat_line.text),
            );
        let trailing_space_advance = flat_line
            .source_projection
            .as_ref()
            .and_then(|projection| projection.boundaries.last())
            .map_or(0.0, |anchor| {
                let trailing_space_end =
                    self.edit_source.as_deref().map_or(anchor.byte, |source| {
                        source.get(anchor.byte..).map_or(anchor.byte, |suffix| {
                            anchor.byte
                                + suffix
                                    .chars()
                                    .take_while(|character| matches!(character, ' ' | '\t'))
                                    .map(char::len_utf8)
                                    .sum::<usize>()
                        })
                    });
                self.trailing_stripped_space_advance(flat_line.flat_idx, trailing_space_end)
            });
        (flat_line.rect.x + flat_line.rect.w).max(text_right) + trailing_space_advance
    }

    pub(in crate::view) fn flat_line_start_anchor(
        &self,
        flat_line: &crate::layout::FlatLine,
    ) -> Option<SourceAnchor> {
        if let Some(source_range) = &flat_line.atomic_source_range {
            return Some(SourceAnchor::downstream(source_range.start));
        }

        self.anchor_from_flat_line_and_visual_grapheme(flat_line.flat_idx, 0)
    }

    pub(in crate::view) fn flat_line_end_anchor(
        &self,
        flat_line: &crate::layout::FlatLine,
    ) -> Option<SourceAnchor> {
        if let Some(source_range) = &flat_line.atomic_source_range {
            return Some(SourceAnchor::downstream(source_range.end));
        }

        self.anchor_from_flat_line_and_visual_grapheme(
            flat_line.flat_idx,
            crate::grapheme_map::grapheme_count(&flat_line.text),
        )
    }

    pub(in crate::view) fn source_anchor_for_flat_line_hit(
        &self,
        flat_line: &crate::layout::FlatLine,
        doc_x: f32,
        doc_y: f32,
    ) -> Option<SourceAnchor> {
        if let Some(source_range) = &flat_line.atomic_source_range {
            let vertical_midpoint = flat_line.rect.y + flat_line.rect.h * 0.5;
            return Some(SourceAnchor::downstream(if doc_y < vertical_midpoint {
                source_range.start
            } else {
                source_range.end
            }));
        }

        let line_x = doc_x - flat_line.rect.x;
        let visual_grapheme = crate::layout::grapheme_at_x(flat_line, line_x);
        self.anchor_from_flat_line_and_visual_grapheme(flat_line.flat_idx, visual_grapheme)
    }

    /// 屏幕坐标 → 源码字节偏移。用于鼠标点击。
    /// 输入 y/offset_y 为插件渲染空间坐标；内部加回 scroll_y 以匹配
    /// flat_line 的文档绝对 y（与 render_line_with_offset 的投影互逆）。
    pub fn hit_test_byte(&self, x: f32, y: f32, offset_x: f32, offset_y: f32) -> Option<usize> {
        self.hit_test_anchor(x, y, offset_x, offset_y).map(|anchor| anchor.byte)
    }

    pub(in crate::view) fn hit_test_anchor(
        &self,
        x: f32,
        y: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Option<SourceAnchor> {
        if let Some(composition) = &self.replacement_preedit {
            return composition
                .engine
                .hit_test_anchor(x, y, offset_x, offset_y)
                .map(|anchor| composition.source_anchor(anchor));
        }
        let lazy = self.lazy.as_ref()?;
        let doc_x = x - offset_x;
        let doc_y = y - offset_y + self.scroll_y;

        let same_row = |flat_line: &&crate::layout::FlatLine| {
            doc_y >= flat_line.rect.y && doc_y <= flat_line.rect.y + flat_line.rect.h
        };
        let horizontal_distance = |flat_line: &&crate::layout::FlatLine| {
            let left = flat_line.rect.x;
            let right = self.hit_test_line_right(flat_line);
            if doc_x < left {
                left - doc_x
            } else if doc_x > right {
                doc_x - right
            } else {
                0.0
            }
        };
        let flat_line = lazy
            .flat_lines
            .iter()
            .filter(|flat_line| same_row(flat_line))
            .find(|flat_line| {
                doc_x >= flat_line.rect.x && doc_x <= self.hit_test_line_right(flat_line)
            })
            .or_else(|| {
                lazy.flat_lines.iter().filter(|flat_line| same_row(flat_line)).min_by(
                    |left, right| {
                        horizontal_distance(left)
                            .total_cmp(&horizontal_distance(right))
                            .then_with(|| right.rect.x.total_cmp(&left.rect.x))
                    },
                )
            });

        if let Some(flat_line) = flat_line {
            return self.source_anchor_for_flat_line_hit(flat_line, doc_x, doc_y);
        }

        let above = lazy.flat_lines.iter().rfind(|fl| fl.rect.y + fl.rect.h <= doc_y);
        let below = lazy.flat_lines.iter().find(|fl| fl.rect.y > doc_y);

        if let (Some(above), Some(below)) = (above, below) {
            let above_bottom = above.rect.y + above.rect.h;
            let gap_mid_y = (above_bottom + below.rect.y) * 0.5;
            return if doc_y < gap_mid_y {
                self.flat_line_end_anchor(above)
            } else {
                self.flat_line_start_anchor(below)
            };
        }

        // Atomic blocks keep their allocated document-edge margin clickable,
        // even when the painted content is only a thin horizontal rule.
        if let Some(above) = above {
            if above.atomic_source_range.is_some() && doc_y <= lazy.total_height {
                return self.flat_line_end_anchor(above);
            }
            let gap = doc_y - (above.rect.y + above.rect.h);
            if gap > above.rect.h * HIT_TEST_SNAP_MAX_LINE_HEIGHTS {
                return None; // far below content
            }
            return self.flat_line_end_anchor(above);
        }

        // No line above — click above first line; snap to start of first line.
        if let Some(below) = lazy.flat_lines.first() {
            if below.atomic_source_range.is_some() && doc_y >= 0.0 {
                return self.flat_line_start_anchor(below);
            }
            let gap = below.rect.y - doc_y;
            if gap > below.rect.h * HIT_TEST_SNAP_MAX_LINE_HEIGHTS {
                return None; // far above content
            }
            return self.flat_line_start_anchor(below);
        }

        None
    }

    pub(in crate::view) fn byte_from_flat_line_and_visual_grapheme(
        &self,
        flat_line_idx: usize,
        visual_grapheme: usize,
    ) -> Option<usize> {
        self.anchor_from_flat_line_and_visual_grapheme(flat_line_idx, visual_grapheme)
            .map(|anchor| anchor.byte)
    }

    pub(in crate::view) fn anchor_from_flat_line_and_visual_grapheme(
        &self,
        flat_line_idx: usize,
        visual_grapheme: usize,
    ) -> Option<SourceAnchor> {
        if let Some(composition) = &self.replacement_preedit {
            return composition
                .engine
                .anchor_from_flat_line_and_visual_grapheme(flat_line_idx, visual_grapheme)
                .map(|anchor| composition.source_anchor(anchor));
        }
        let lazy = self.lazy.as_ref()?;
        let index = lazy.source_projection_index.as_ref()?;
        let projection_flat_line_idx =
            lazy.projection_visual_line_idx_for_flat_line(flat_line_idx)?;
        let position = VisualPosition {
            layout_revision: index.layout_revision(),
            flat_line_idx: projection_flat_line_idx,
            grapheme_pos: visual_grapheme,
        };
        index.source_anchor_at(self.source_generation, position).ok()
    }

    pub(in crate::view) fn find_flat_and_grapheme_for_byte(
        &self,
        byte: usize,
    ) -> Option<(usize, usize)> {
        if let Some(composition) = &self.replacement_preedit {
            return composition
                .engine
                .find_flat_and_grapheme_for_byte(composition.preview_byte(byte));
        }
        let position =
            self.cursor_visual_position_for_byte(byte, self.cursor_affinity_for_byte(byte))?;
        let flat_line_idx =
            self.lazy.as_ref()?.flat_line_idx_for_projection(position.flat_line_idx)?;
        Some((flat_line_idx, position.grapheme_pos))
    }

    pub(in crate::view) fn visual_range_for_byte_selection(&self) -> Option<(ViewPos, ViewPos)> {
        let (start_byte, end_byte) = self.byte_selection_range()?;
        let start = self.selection_position_for_byte(start_byte, false)?;
        let end = self.selection_position_for_byte(end_byte, true)?;
        let start = ViewPos { flat_line_idx: start.0, grapheme_pos: start.1 };
        let end = ViewPos { flat_line_idx: end.0, grapheme_pos: end.1 };
        if (start.flat_line_idx, start.grapheme_pos) <= (end.flat_line_idx, end.grapheme_pos) {
            Some((start, end))
        } else {
            Some((end, start))
        }
    }

    pub(in crate::view) fn selection_position_for_byte(
        &self,
        source_byte: usize,
        prefer_previous_rendered_line: bool,
    ) -> Option<(usize, usize)> {
        let lazy = self.lazy.as_ref()?;
        let index = lazy.source_projection_index.as_ref()?;
        let position = index.visual_position_for_source(source_byte, CursorAffinity::Downstream)?;
        if let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(position.flat_line_idx) {
            return Some((flat_line_idx, position.grapheme_pos));
        }

        if prefer_previous_rendered_line {
            for projection_idx in (0..position.flat_line_idx).rev() {
                let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(projection_idx) else {
                    continue;
                };
                let grapheme_pos =
                    index.visual_lines()[projection_idx].boundaries.len().saturating_sub(1);
                return Some((flat_line_idx, grapheme_pos));
            }
            return None;
        }

        for projection_idx in position.flat_line_idx.saturating_add(1)..index.visual_lines().len() {
            if let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(projection_idx) {
                return Some((flat_line_idx, 0));
            }
        }
        None
    }

    /// 视觉方向导航。Uses canonical source projections when translating between visual rows and
    /// source bytes.
    pub fn visual_move(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<usize> {
        self.visual_move_anchor(current_byte, direction, target_x).map(|anchor| anchor.byte)
    }

    pub(in crate::view) fn visual_move_anchor(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<SourceAnchor> {
        use ui::plugin::MoveDirection;
        if let Some(composition) = &self.replacement_preedit {
            return composition
                .engine
                .visual_move_anchor(composition.preview_byte(current_byte), direction, target_x)
                .map(|anchor| composition.source_anchor(anchor));
        }
        let lazy = self.lazy.as_ref()?;
        let source = self.edit_source.as_ref();
        let current_source_line = source.and_then(|text| source_line_at_byte(text, current_byte));

        if matches!(
            direction,
            MoveDirection::Left | MoveDirection::Right | MoveDirection::Up | MoveDirection::Down
        ) && current_source_line.is_some_and(|source_line| {
            source.is_some_and(|source| {
                self.empty_source_line_role(source_line, lazy, source)
                    == EmptySourceLineRole::HiddenBlockSeparator
            })
        }) {
            return self.move_from_hidden_block_separator(current_byte, direction);
        }

        let index = lazy.source_projection_index.as_ref()?;

        match direction {
            MoveDirection::Left => {
                if current_byte == 0 {
                    return Some(SourceAnchor::downstream(0));
                }
                if let Some(anchor) =
                    index.move_horizontal(current_byte, HorizontalDirection::Previous)
                {
                    return Some(anchor);
                }
                Some(SourceAnchor::downstream(0))
            }
            MoveDirection::Right => {
                if let Some(anchor) = index.move_horizontal(current_byte, HorizontalDirection::Next)
                {
                    return Some(anchor);
                }
                Some(SourceAnchor::downstream(current_byte))
            }
            MoveDirection::Up | MoveDirection::Down => self.visual_move_in_projection_sequence(
                lazy,
                index,
                current_byte,
                direction,
                target_x,
            ),
            MoveDirection::LineStart => {
                if let Some(line) = current_source_line
                    && line.is_empty()
                {
                    return Some(SourceAnchor::downstream(line.start));
                }
                index.line_boundary_with_affinity(
                    current_byte,
                    self.cursor_affinity_for_byte(current_byte),
                    LineBoundary::Start,
                )
            }
            MoveDirection::LineEnd => {
                if let Some(line) = current_source_line
                    && line.is_empty()
                {
                    return Some(SourceAnchor::downstream(line.start));
                }
                index.line_boundary_with_affinity(
                    current_byte,
                    self.cursor_affinity_for_byte(current_byte),
                    LineBoundary::End,
                )
            }
        }
    }

    pub(in crate::view) fn visual_move_in_projection_sequence(
        &self,
        lazy: &LazyLayout<S>,
        index: &SourceProjectionIndex,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<SourceAnchor> {
        let current_position =
            self.projection_position_for_vertical_move(lazy, index, current_byte)?;
        let target_visual_line_idx = match vertical_move_target(
            index.visual_lines(),
            current_position.flat_line_idx,
            direction,
        )? {
            VerticalMoveTarget::VisualLine(target) => target,
            VerticalMoveTarget::DocumentStart => return Some(SourceAnchor::downstream(0)),
            VerticalMoveTarget::DocumentEnd => {
                return self
                    .edit_source
                    .as_deref()
                    .map(|source| SourceAnchor::downstream(source.len()));
            }
        };
        let screen_x = match target_x {
            Some(screen_x) => screen_x,
            None => self.projection_screen_x(lazy, current_position)?,
        };
        let target_grapheme =
            self.projection_grapheme_at_screen_x(lazy, target_visual_line_idx, screen_x)?;
        let target_position = VisualPosition {
            layout_revision: index.layout_revision(),
            flat_line_idx: target_visual_line_idx,
            grapheme_pos: target_grapheme,
        };
        index.source_anchor_at(self.source_generation, target_position).ok()
    }

    pub(in crate::view) fn projection_position_for_vertical_move(
        &self,
        lazy: &LazyLayout<S>,
        index: &SourceProjectionIndex,
        current_byte: usize,
    ) -> Option<VisualPosition> {
        let source_line = self
            .edit_source
            .as_deref()
            .and_then(|source| source_line_at_byte(source, current_byte));
        if let Some(source_line) = source_line.filter(|line| line.is_empty())
            && let Some(visual_line_idx) =
                lazy.projection_visual_line_idx_for_empty_source_byte(source_line.start)
        {
            return Some(VisualPosition {
                layout_revision: index.layout_revision(),
                flat_line_idx: visual_line_idx,
                grapheme_pos: 0,
            });
        }

        index.visual_position_for_source(current_byte, self.cursor_affinity_for_byte(current_byte))
    }

    pub(in crate::view) fn projection_screen_x(
        &self,
        lazy: &LazyLayout<S>,
        position: VisualPosition,
    ) -> Option<f32> {
        let flat_line_idx = lazy.flat_line_idx_for_projection(position.flat_line_idx)?;
        let line = lazy.flat_lines.get(flat_line_idx)?;
        Some(line.rect.x + self.grapheme_x_for_line(line, position.grapheme_pos))
    }

    pub(in crate::view) fn projection_grapheme_at_screen_x(
        &self,
        lazy: &LazyLayout<S>,
        visual_line_idx: usize,
        screen_x: f32,
    ) -> Option<usize> {
        let flat_line_idx = lazy.flat_line_idx_for_projection(visual_line_idx)?;
        let line = lazy.flat_lines.get(flat_line_idx)?;
        Some(self.grapheme_at_x_for_line(line, screen_x - line.rect.x))
    }

    pub(in crate::view) fn move_from_hidden_block_separator(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
    ) -> Option<SourceAnchor> {
        let index = self.lazy.as_ref()?.source_projection_index.as_ref()?;
        match direction {
            ui::plugin::MoveDirection::Left => index.line_boundary(current_byte, LineBoundary::End),
            ui::plugin::MoveDirection::Up | ui::plugin::MoveDirection::LineStart => {
                index.line_boundary(current_byte, LineBoundary::Start)
            }
            ui::plugin::MoveDirection::Right | ui::plugin::MoveDirection::Down => {
                index.move_horizontal(current_byte, HorizontalDirection::Next)
            }
            ui::plugin::MoveDirection::LineEnd => {
                index.line_boundary(current_byte, LineBoundary::End)
            }
        }
    }

    pub(in crate::view) fn empty_source_line_role(
        &self,
        source_line: SourceLineAtByte,
        lazy: &LazyLayout<S>,
        _source: &str,
    ) -> EmptySourceLineRole {
        if !source_line.is_empty() {
            return EmptySourceLineRole::EditableLine;
        }
        if self.own_rendered_line(source_line, lazy).is_some() {
            EmptySourceLineRole::EditableLine
        } else {
            EmptySourceLineRole::HiddenBlockSeparator
        }
    }
}

enum VerticalMoveTarget {
    VisualLine(usize),
    DocumentStart,
    DocumentEnd,
}

fn vertical_move_target(
    visual_lines: &[crate::layout::types::VisualLineProjection],
    current_idx: usize,
    direction: ui::plugin::MoveDirection,
) -> Option<VerticalMoveTarget> {
    if let Some(crate::projection::ProjectionOwnerId::TableCell {
        table_start, row, column, ..
    }) = visual_lines.get(current_idx).map(|line| line.owner)
    {
        return table_cell_vertical_target(
            visual_lines,
            current_idx,
            table_start,
            row,
            column,
            direction,
        );
    }

    match direction {
        ui::plugin::MoveDirection::Up => Some(
            current_idx
                .checked_sub(1)
                .map_or(VerticalMoveTarget::DocumentStart, VerticalMoveTarget::VisualLine),
        ),
        ui::plugin::MoveDirection::Down => {
            let target = current_idx.checked_add(1)?;
            Some(if target >= visual_lines.len() {
                VerticalMoveTarget::DocumentEnd
            } else {
                VerticalMoveTarget::VisualLine(target)
            })
        }
        _ => None,
    }
}

fn table_cell_vertical_target(
    visual_lines: &[crate::layout::types::VisualLineProjection],
    current_idx: usize,
    table_start: usize,
    row: usize,
    column: usize,
    direction: ui::plugin::MoveDirection,
) -> Option<VerticalMoveTarget> {
    match direction {
        ui::plugin::MoveDirection::Up => {
            table_cell_vertical_up(visual_lines, current_idx, table_start, row, column)
        }
        ui::plugin::MoveDirection::Down => {
            table_cell_vertical_down(visual_lines, current_idx, table_start, row, column)
        }
        _ => None,
    }
}

fn table_cell_vertical_up(
    visual_lines: &[crate::layout::types::VisualLineProjection],
    current_idx: usize,
    table_start: usize,
    row: usize,
    column: usize,
) -> Option<VerticalMoveTarget> {
    if let Some(prev_idx) = current_idx.checked_sub(1)
        && visual_lines
            .get(prev_idx)
            .is_some_and(|line| is_table_cell(line.owner, table_start, row, column))
    {
        return Some(VerticalMoveTarget::VisualLine(prev_idx));
    }
    if let Some(above_idx) = visual_lines[..current_idx].iter().rposition(|line| {
        matches!(
            line.owner,
            crate::projection::ProjectionOwnerId::TableCell { table_start: t, row: r, column: c, .. }
                if t == table_start && r < row && c == column
        )
    }) {
        return Some(VerticalMoveTarget::VisualLine(above_idx));
    }
    let before_table = visual_lines[..current_idx]
        .iter()
        .rposition(|line| !belongs_to_table(line.owner, table_start));
    Some(before_table.map_or(VerticalMoveTarget::DocumentStart, VerticalMoveTarget::VisualLine))
}

fn table_cell_vertical_down(
    visual_lines: &[crate::layout::types::VisualLineProjection],
    current_idx: usize,
    table_start: usize,
    row: usize,
    column: usize,
) -> Option<VerticalMoveTarget> {
    let next_idx = current_idx.checked_add(1)?;
    if visual_lines
        .get(next_idx)
        .is_some_and(|line| is_table_cell(line.owner, table_start, row, column))
    {
        return Some(VerticalMoveTarget::VisualLine(next_idx));
    }
    let tail = visual_lines.get(next_idx..).unwrap_or(&[]);
    if let Some(offset) = tail.iter().position(|line| {
        matches!(
            line.owner,
            crate::projection::ProjectionOwnerId::TableCell { table_start: t, row: r, column: c, .. }
                if t == table_start && r > row && c == column
        )
    }) {
        return Some(VerticalMoveTarget::VisualLine(next_idx + offset));
    }
    let after_offset = tail.iter().position(|line| !belongs_to_table(line.owner, table_start));
    Some(after_offset.map_or(VerticalMoveTarget::DocumentEnd, |offset| {
        VerticalMoveTarget::VisualLine(next_idx + offset)
    }))
}

fn is_table_cell(
    owner: crate::projection::ProjectionOwnerId,
    table_start: usize,
    row: usize,
    column: usize,
) -> bool {
    matches!(
        owner,
        crate::projection::ProjectionOwnerId::TableCell { table_start: t, row: r, column: c, .. }
            if t == table_start && r == row && c == column
    )
}

fn belongs_to_table(owner: crate::projection::ProjectionOwnerId, table_start: usize) -> bool {
    matches!(
        owner,
        crate::projection::ProjectionOwnerId::TableCell { table_start: t, .. } if t == table_start
    )
}
