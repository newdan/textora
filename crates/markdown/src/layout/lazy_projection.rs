use super::super::source_line_map::{HiddenBlockSeparator, SourceLineMap};
use super::*;
use crate::projection::{ProjectionError, SourceAnchor, SourceProjectionIndex};

impl<S: BlockSource> LazyLayout<S> {
    /// Get the document block index for a given laid-out block index.
    pub fn laid_to_doc(&self, idx: usize) -> Option<usize> {
        self.laid_to_doc.get(idx).copied()
    }

    pub(crate) fn set_source_generation(&mut self, source_generation: u32) {
        if self.source_generation == source_generation {
            return;
        }
        self.source_generation = source_generation;
        self.source_projection_index = None;
        self.source_projection_error = None;
    }

    pub(crate) fn rebuild_source_projection_index(&mut self) -> Result<(), ProjectionError> {
        self.publish_flat_line_projection_index()
    }

    pub(crate) fn validate_editable_projections(&self) -> Result<(), ProjectionError> {
        if self.edit_ctx.is_none() {
            return Ok(());
        }

        for line in &self.flat_lines {
            if line.requires_source_projection && line.source_projection.is_none() {
                return Err(ProjectionError::MissingEditableProjection {
                    flat_line_idx: line.flat_idx,
                });
            }
        }
        Ok(())
    }

    pub(in crate::layout::types) fn retained_visual_lines(
        &self,
    ) -> Vec<(VisualLineProjection, f32)> {
        let mut visual_lines = Vec::new();

        for (block_idx, projections) in self.retained_block_projections.iter().enumerate() {
            let y_correction = self.y_delta.get(block_idx).copied().unwrap_or(0.0);
            for projection in projections {
                visual_lines.push((projection.projection.clone(), projection.y + y_correction));
            }
        }

        visual_lines
    }

    pub(in crate::layout::types) fn publish_source_projection_index(
        &mut self,
        visual_lines: Vec<VisualLineProjection>,
    ) -> Result<(), ProjectionError> {
        let next_revision = self
            .layout_revision
            .checked_add(1)
            .expect("source projection layout revision must not overflow");
        match SourceProjectionIndex::build(self.source_generation, next_revision, visual_lines) {
            Ok(index) => {
                self.layout_revision = next_revision;
                self.source_projection_error = None;
                self.source_projection_index = Some(index);
                Ok(())
            }
            Err(error) => {
                self.source_projection_index = None;
                self.source_projection_error = Some(error.clone());
                Err(error)
            }
        }
    }
    /// Build the flat line array from the laid-out blocks.
    /// Must be called after layout/precision changes.
    ///
    /// Phase 4: only iterates blocks in `viewport_range` (visible + buffer).
    /// Blocks outside this range that are None are skipped safely.
    pub fn build_flat_lines(&mut self, _doc_view: &dyn core::document::DocView) {
        let (code_viewport, changed_indices) = crate::layout::code_viewport::update(
            &mut self.laid_out,
            &self.precise,
            self.edit_ctx.as_ref().map(|context| context.cursor_byte),
            self.code_viewport,
        );
        self.code_viewport = code_viewport;
        for index in changed_indices {
            self.reconciled_flat_lines[index] = None;
        }
        let mut lines = Vec::new();
        let mut flat_idx = 0usize;

        let vr = if self.viewport_range.is_empty() {
            0..self.laid_out.len()
        } else {
            self.viewport_range.clone()
        };

        let mut current_doc_bi = 0;
        let mut current_line_base = 0;

        for bi in vr {
            let block = match self.laid_out.get(bi).and_then(|b| b.as_ref()) {
                Some(b) => b,
                None => continue,
            };
            let doc_bi = self.laid_to_doc.get(bi).copied().unwrap_or(0);

            while current_doc_bi < doc_bi {
                if let Some(b) = self.source.blocks().get(current_doc_bi) {
                    current_line_base +=
                        b.line_count() + Self::count_all_descendant_lines(&b.children);
                }
                current_doc_bi += 1;
            }

            let y_delta = self.y_delta.get(bi).copied().unwrap_or(0.0);
            if let Some(mut reconciled_lines) =
                self.reconciled_flat_lines.get_mut(bi).and_then(Option::take)
            {
                for line in &mut reconciled_lines {
                    line.flat_idx = flat_idx;
                    line.rect.y += y_delta;
                    if let Some(projection) = &mut line.source_projection {
                        projection.flat_line_idx = flat_idx;
                    }
                    flat_idx += 1;
                }
                lines.append(&mut reconciled_lines);
                continue;
            }

            let doc_block = self.source.blocks().get(doc_bi);

            Self::flatten_block_into(
                block,
                y_delta,
                &mut lines,
                &mut flat_idx,
                current_line_base,
                doc_block,
                self.source_text.as_deref(),
            );
        }
        self.flat_lines = lines;

        let (projected_empty_lines, hidden_block_separators) = self.collect_empty_line_layout();
        self.projected_empty_lines = projected_empty_lines;
        self.hidden_block_separators = hidden_block_separators;
        let _ = self.publish_flat_line_projection_index();
    }

    pub(in crate::layout::types) fn collect_empty_line_layout(
        &self,
    ) -> (Vec<ProjectedEmptyLine>, Vec<HiddenBlockSeparator>) {
        let empty_lines = self
            .flat_lines
            .iter()
            .filter_map(|line| {
                let projection = line.source_projection.as_ref()?;
                let ProjectionOwnerId::EmptyLine { source_byte } = projection.owner else {
                    return None;
                };
                Some(ProjectedEmptyLine {
                    owner: projection.owner,
                    source_byte,
                    y_top: line.rect.y,
                    height: line.rect.h,
                })
            })
            .collect();
        let hidden_separators = self.source_text.as_deref().map_or_else(Vec::new, |source| {
            let mut source_ranges = self
                .flat_lines
                .iter()
                .filter_map(|line| {
                    line.source_projection
                        .as_ref()
                        .map(|projection| projection.source_extent.clone())
                        .or_else(|| line.atomic_source_range.clone())
                })
                .collect::<Vec<_>>();
            source_ranges.sort_by_key(|range| range.start);
            SourceLineMap::from_source(source).hidden_separators_for_rendered_lines(&source_ranges)
        });
        (empty_lines, hidden_separators)
    }

    pub(in crate::layout::types) fn add_hidden_separator_collapsed_ranges(
        visual_lines: &mut [VisualLineProjection],
        hidden_separators: impl Iterator<Item = HiddenBlockSeparator>,
    ) {
        for separator in hidden_separators {
            let Some(previous_line) = visual_lines
                .iter_mut()
                .rev()
                .find(|projection| projection.source_extent.end <= separator.source_range.start)
            else {
                continue;
            };
            if let Some(last_boundary) = previous_line.boundaries.last_mut()
                && last_boundary.byte >= separator.source_range.start
            {
                *last_boundary = SourceAnchor::downstream(separator.previous_anchor);
                previous_line.source_extent.end = separator.previous_anchor;
            }
            let boundary = previous_line.boundaries.len().saturating_sub(1);
            previous_line.collapsed.push(CollapsedBoundary {
                source_range: separator.source_range,
                upstream_grapheme: boundary,
                downstream_grapheme: boundary,
            });
        }
    }

    pub(in crate::layout::types) fn add_inter_line_collapsed_ranges(
        visual_lines: &mut [VisualLineProjection],
    ) {
        for line_idx in 0..visual_lines.len().saturating_sub(1) {
            let (left, right) = visual_lines.split_at_mut(line_idx + 1);
            let previous = &mut left[line_idx];
            let next = &right[0];
            let Some(previous_boundary) = previous.boundaries.last() else {
                continue;
            };
            let Some(next_boundary) = next.boundaries.first() else {
                continue;
            };
            if previous_boundary.byte >= next_boundary.byte {
                continue;
            }

            previous.collapsed.push(CollapsedBoundary {
                source_range: previous_boundary.byte..next_boundary.byte,
                upstream_grapheme: previous.boundaries.len().saturating_sub(1),
                downstream_grapheme: previous.boundaries.len().saturating_sub(1),
            });
        }
    }

    pub(in crate::layout::types) fn add_document_edge_collapsed_ranges(
        visual_lines: &mut [VisualLineProjection],
        source_len: usize,
    ) {
        let Some(first_line) = visual_lines.first_mut() else {
            return;
        };
        let Some(first_boundary) = first_line.boundaries.first() else {
            return;
        };
        if first_boundary.byte > 0 {
            first_line.collapsed.push(CollapsedBoundary {
                source_range: 0..first_boundary.byte,
                upstream_grapheme: 0,
                downstream_grapheme: 0,
            });
        }

        let Some(last_line) = visual_lines.last_mut() else {
            return;
        };
        let Some(last_boundary) = last_line.boundaries.last() else {
            return;
        };
        if last_boundary.byte < source_len {
            let terminal_grapheme = last_line.boundaries.len().saturating_sub(1);
            last_line.collapsed.push(CollapsedBoundary {
                source_range: last_boundary.byte..source_len,
                upstream_grapheme: terminal_grapheme,
                downstream_grapheme: terminal_grapheme,
            });
        }
    }

    pub(in crate::layout::types) fn publish_flat_line_projection_index(
        &mut self,
    ) -> Result<(), ProjectionError> {
        self.validate_editable_projections()?;
        let mut projection_candidates = self
            .retained_visual_lines()
            .into_iter()
            .map(|(projection, y)| {
                let flat_line_idx = self.flat_lines.iter().find_map(|line| {
                    line.source_projection
                        .as_ref()
                        .filter(|current| Self::same_visual_line_identity(current, &projection))
                        .map(|_| line.flat_idx)
                });
                (projection, flat_line_idx, y)
            })
            .collect::<Vec<_>>();
        for line in &self.flat_lines {
            let Some(projection) = line.source_projection.clone() else {
                continue;
            };
            let retained = projection_candidates
                .iter()
                .any(|(candidate, _, _)| Self::same_visual_line_identity(candidate, &projection));
            if !retained {
                projection_candidates.push((projection, Some(line.flat_idx), line.rect.y));
            }
        }
        let mut visual_lines = projection_candidates
            .iter_mut()
            .map(|(projection, _, _)| projection)
            .collect::<Vec<_>>();
        if let Some(source_text) = self.source_text.as_deref() {
            for projection in &mut visual_lines {
                // A zero-width projection is an insertion anchor, not a line-terminal boundary.
                if projection.source_extent.is_empty() {
                    continue;
                }
                let Some(last_boundary) = projection.boundaries.last_mut() else {
                    continue;
                };
                let previous_boundary = last_boundary.byte;
                last_boundary.byte =
                    Self::source_line_terminal_boundary(source_text, previous_boundary);
                if projection.source_extent.end == previous_boundary {
                    projection.source_extent.end = last_boundary.byte;
                }
            }
        }
        projection_candidates.sort_by(|left, right| {
            left.0
                .source_extent
                .start
                .cmp(&right.0.source_extent.start)
                .then_with(|| left.0.source_extent.end.cmp(&right.0.source_extent.end))
                .then_with(|| left.2.total_cmp(&right.2))
        });
        self.projection_flat_line_indices.clear();
        self.projected_empty_line_geometry.clear();
        let mut visual_lines = Vec::with_capacity(projection_candidates.len());
        for (visual_line_idx, (mut projection, flat_line_idx, _)) in
            projection_candidates.into_iter().enumerate()
        {
            projection.flat_line_idx = visual_line_idx;
            if let ProjectionOwnerId::EmptyLine { source_byte } = projection.owner
                && let Some(empty_line) = self
                    .projected_empty_lines
                    .iter()
                    .copied()
                    .find(|empty_line| empty_line.source_byte == source_byte)
            {
                self.projected_empty_line_geometry.insert(visual_line_idx, empty_line);
            }
            self.projection_flat_line_indices.push(flat_line_idx);
            visual_lines.push(projection);
        }
        Self::add_hidden_separator_collapsed_ranges(
            &mut visual_lines,
            self.hidden_block_separators.iter().cloned(),
        );
        Self::add_inter_line_collapsed_ranges(&mut visual_lines);
        if let Some(source_text) = self.source_text.as_deref() {
            Self::add_document_edge_collapsed_ranges(&mut visual_lines, source_text.len());
        }
        self.publish_source_projection_index(visual_lines)
    }

    pub(in crate::layout::types) fn same_visual_line_identity(
        left: &VisualLineProjection,
        right: &VisualLineProjection,
    ) -> bool {
        left.owner == right.owner
            && left.boundaries == right.boundaries
            && left.source_extent == right.source_extent
            && left.collapsed == right.collapsed
    }

    pub(crate) fn flat_line_idx_for_projection(&self, visual_line_idx: usize) -> Option<usize> {
        self.projection_flat_line_indices.get(visual_line_idx).copied().flatten()
    }

    pub(crate) fn projection_visual_line_idx_for_flat_line(
        &self,
        flat_line_idx: usize,
    ) -> Option<usize> {
        self.projection_flat_line_indices
            .iter()
            .position(|candidate| *candidate == Some(flat_line_idx))
    }

    pub(crate) fn projection_visual_line_idx_for_empty_source_byte(
        &self,
        source_byte: usize,
    ) -> Option<usize> {
        self.projected_empty_line_geometry.iter().find_map(|(&visual_line_idx, empty_line)| {
            (empty_line.source_byte == source_byte).then_some(visual_line_idx)
        })
    }

    pub(in crate::layout::types) fn source_line_terminal_boundary(
        source: &str,
        boundary: usize,
    ) -> usize {
        if boundary > 0 && source.as_bytes().get(boundary - 1) == Some(&b'\n') {
            return if boundary > 1 && source.as_bytes()[boundary - 2] == b'\r' {
                boundary - 2
            } else {
                boundary - 1
            };
        }
        let newline_byte = if boundary == source.len() {
            source.as_bytes().last().is_some_and(|byte| *byte == b'\n').then_some(boundary - 1)
        } else {
            source.as_bytes().get(boundary).is_some_and(|byte| *byte == b'\n').then_some(boundary)
        };
        let Some(newline_byte) = newline_byte else {
            return boundary;
        };
        if newline_byte > 0 && source.as_bytes()[newline_byte - 1] == b'\r' {
            newline_byte - 1
        } else {
            newline_byte
        }
    }

    pub(in crate::layout::types) fn retain_block_projections(
        &mut self,
        block_idx: usize,
        block: &LaidOutBlock,
    ) {
        let mut projections = Vec::new();
        Self::collect_block_projections(block, &mut projections);

        if let Some(slot) = self.retained_block_projections.get_mut(block_idx) {
            *slot = projections;
        }
    }

    pub(in crate::layout::types) fn collect_block_projections(
        block: &LaidOutBlock,
        projections: &mut Vec<RetainedVisualLineProjection>,
    ) {
        match &block.kind {
            LaidOutBlockKind::Text { lines }
            | LaidOutBlockKind::CodeBlock { lines, .. }
            | LaidOutBlockKind::MetadataBlock { lines } => {
                Self::collect_line_projections(lines, projections);
            }
            LaidOutBlockKind::BlockQuote { blocks } => {
                for block in blocks {
                    Self::collect_block_projections(block, projections);
                }
            }
            LaidOutBlockKind::ListItem { lines, blocks, .. } => {
                Self::collect_line_projections(lines, projections);
                for block in blocks {
                    Self::collect_block_projections(block, projections);
                }
            }
            LaidOutBlockKind::Table { header, rows, .. } => {
                for lines in header {
                    Self::collect_line_projections(lines, projections);
                }
                for row in rows {
                    for lines in row {
                        Self::collect_line_projections(lines, projections);
                    }
                }
            }
            LaidOutBlockKind::HorizontalRule | LaidOutBlockKind::Embedded { .. } => {}
        }
    }

    pub(in crate::layout::types) fn collect_line_projections(
        lines: &[LaidOutLine],
        projections: &mut Vec<RetainedVisualLineProjection>,
    ) {
        for line in lines {
            if let Some(projection) = &line.source_projection {
                projections.push(RetainedVisualLineProjection {
                    projection: projection.clone(),
                    y: line.rect.y,
                });
            }
        }
    }

    /// Count total text_lines for all doc blocks that appear before `target`
    /// in the tree walk. Used to compute a unique flat index for each doc block.
    pub fn count_block_lines_before(
        blocks: &[crate::builder::BlockNode],
        target: &crate::builder::BlockNode,
    ) -> usize {
        let mut count = 0usize;
        Self::count_until_target(blocks, target, &mut count);
        count
    }

    /// Walk the doc tree, accumulating line counts until we reach `target`.
    /// Returns true if `target` was found (and counting should stop).
    pub(in crate::layout::types) fn count_until_target(
        blocks: &[crate::builder::BlockNode],
        target: &crate::builder::BlockNode,
        count: &mut usize,
    ) -> bool {
        for block in blocks {
            if std::ptr::eq(block, target) {
                return true;
            }
            *count += block.line_count();
            if Self::count_until_target(&block.children, target, count) {
                return true;
            }
        }
        false
    }

    /// Helper to create a FlatLine from a LaidOutLine.
    pub(in crate::layout::types) fn push_flat_line(
        line: &LaidOutLine,
        y_correction: f32,
        flat: &mut Vec<FlatLine>,
        flat_idx: &mut usize,
    ) {
        // Prefer explicit shaped data; fall back to the pre-built TextLayout's
        // ShapedRun so that grapheme_x can use real glyph advances for hit-testing
        // and search-highlight positioning instead of the heuristic fallback.
        let shaped =
            line.shaped.clone().or_else(|| line.text_layout.as_ref().map(|tl| tl.shaped.clone()));
        flat.push(FlatLine {
            flat_idx: *flat_idx,
            rect: Rect::new(line.rect.x, line.rect.y + y_correction, line.rect.w, line.rect.h),
            text: line.text.clone(),
            font_size: line.font_size,
            is_code: line.is_code,
            shaped,
            requires_source_projection: true,
            source_projection: line.source_projection.as_ref().map(|projection| {
                let mut projection = projection.clone();
                projection.flat_line_idx = *flat_idx;
                projection
            }),
            atomic_source_range: None,
        });
        *flat_idx += 1;
    }

    /// Recursively flatten a block's lines into the flat array.
    /// `block_line_base` is a unique flat index for this doc block
    /// (total text_lines of all preceding blocks in tree-walk order).
    /// `doc_block` is the source block corresponding to `block` (for source metadata and
    /// recursive descent).
    pub(in crate::layout::types) fn flatten_block_into(
        block: &LaidOutBlock,
        y_correction: f32,
        flat: &mut Vec<FlatLine>,
        flat_idx: &mut usize,
        block_line_base: usize,
        doc_block: Option<&crate::builder::BlockNode>,
        source_text: Option<&str>,
    ) {
        let doc_children = doc_block.map(|block| block.children.as_slice());
        match &block.kind {
            LaidOutBlockKind::Text { lines: text_lines }
            | LaidOutBlockKind::CodeBlock { lines: text_lines, .. }
            | LaidOutBlockKind::MetadataBlock { lines: text_lines } => {
                for line in text_lines.iter() {
                    Self::push_flat_line(line, y_correction, flat, flat_idx);
                }
            }
            LaidOutBlockKind::BlockQuote { blocks: sub_blocks } => {
                let mut child_offset = block_line_base;
                for (i, sub) in sub_blocks.iter().enumerate() {
                    let child = doc_children.and_then(|c| c.get(i));
                    Self::flatten_block_into(
                        sub,
                        y_correction,
                        flat,
                        flat_idx,
                        child_offset,
                        child,
                        source_text,
                    );
                    if let Some(c) = child {
                        child_offset +=
                            c.line_count() + Self::count_all_descendant_lines(&c.children);
                    }
                }
            }
            LaidOutBlockKind::ListItem { lines: list_lines, blocks: sub_blocks, .. } => {
                for line in list_lines.iter() {
                    Self::push_flat_line(line, y_correction, flat, flat_idx);
                }
                let mut child_offset = block_line_base + list_lines.len();
                for (i, sub) in sub_blocks.iter().enumerate() {
                    let child = doc_children.and_then(|c| c.get(i));
                    Self::flatten_block_into(
                        sub,
                        y_correction,
                        flat,
                        flat_idx,
                        child_offset,
                        child,
                        source_text,
                    );
                    if let Some(c) = child {
                        child_offset +=
                            c.line_count() + Self::count_all_descendant_lines(&c.children);
                    }
                }
            }
            LaidOutBlockKind::Table { header, rows, .. } => {
                for cell_lines in header {
                    for line in cell_lines {
                        Self::push_flat_line(line, y_correction, flat, flat_idx);
                    }
                }
                for row in rows {
                    for cell_lines in row {
                        for line in cell_lines {
                            Self::push_flat_line(line, y_correction, flat, flat_idx);
                        }
                    }
                }
            }
            LaidOutBlockKind::HorizontalRule | LaidOutBlockKind::Embedded { .. } => {
                flat.push(FlatLine {
                    flat_idx: *flat_idx,
                    rect: Rect::new(
                        block.rect.x,
                        block.rect.y + y_correction,
                        block.rect.w,
                        block.rect.h,
                    ),
                    text: String::new(),
                    font_size: 14.0, // Non-zero so the cursor is visible
                    is_code: false,
                    shaped: None,
                    requires_source_projection: false,
                    source_projection: None,
                    atomic_source_range: doc_block.and_then(|block| {
                        Self::source_range_without_line_ending(&block.block_range, source_text)
                    }),
                });
                *flat_idx += 1;
            }
        }
    }

    pub(in crate::layout::types) fn source_range_without_line_ending(
        block_range: &Range<usize>,
        source_text: Option<&str>,
    ) -> Option<Range<usize>> {
        let source_text = source_text?;
        let start = block_range.start.min(source_text.len());
        let mut end = block_range.end.min(source_text.len());
        let source_bytes = source_text.as_bytes();

        while end > start && matches!(source_bytes[end - 1], b'\n' | b'\r') {
            end -= 1;
        }

        (start < end).then_some(start..end)
    }

    /// Count all text_lines in a subtree (used for offset calculation).
    pub(in crate::layout::types) fn count_all_descendant_lines(
        blocks: &[crate::builder::BlockNode],
    ) -> usize {
        let mut total = 0usize;
        for block in blocks {
            total += block.line_count();
            total += Self::count_all_descendant_lines(&block.children);
        }
        total
    }

    /// Search the doc tree for the block and line containing `byte`.
    /// Returns `(block_line_base, line_idx_in_block)` matching the deepest
    /// (most specific) block whose source_range contains the byte.
    pub fn find_block_line_at_byte(&self, byte: usize) -> Option<(usize, usize)> {
        Self::search_block_line_at_byte(self.source.blocks(), byte)
    }

    pub(in crate::layout::types) fn search_block_line_at_byte(
        blocks: &[crate::builder::BlockNode],
        byte: usize,
    ) -> Option<(usize, usize)> {
        let mut offset = 0usize;
        for block in blocks {
            if byte < block.block_range.start || byte > block.block_range.end {
                // Skip this entire subtree.
                offset += block.line_count();
                offset += Self::count_all_descendant_lines(&block.children);
                continue;
            }
            // Search children first for a more specific match.
            if let Some((child_offset, line_idx)) = Self::search_block_line_at_byte_with_offset(
                &block.children,
                byte,
                offset + block.line_count(),
            ) {
                return Some((child_offset, line_idx));
            }
            // No child matched; this block is the answer.
            let line_idx = Self::find_line_idx_in_block(block, byte);
            return Some((offset, line_idx));
        }
        None
    }

    pub(in crate::layout::types) fn search_block_line_at_byte_with_offset(
        blocks: &[crate::builder::BlockNode],
        byte: usize,
        start_offset: usize,
    ) -> Option<(usize, usize)> {
        let mut offset = start_offset;
        for block in blocks {
            if byte < block.block_range.start || byte > block.block_range.end {
                offset += block.line_count();
                offset += Self::count_all_descendant_lines(&block.children);
                continue;
            }
            if let Some((child_offset, line_idx)) = Self::search_block_line_at_byte_with_offset(
                &block.children,
                byte,
                offset + block.line_count(),
            ) {
                return Some((child_offset, line_idx));
            }
            let line_idx = Self::find_line_idx_in_block(block, byte);
            return Some((offset, line_idx));
        }
        None
    }

    /// Find which line in a block contains the given byte.
    pub(in crate::layout::types) fn find_line_idx_in_block(
        block: &crate::builder::BlockNode,
        byte: usize,
    ) -> usize {
        if let Some(starts) = &block.code_line_source_starts {
            for (i, &start) in starts.iter().enumerate().rev() {
                if byte >= start {
                    return i;
                }
            }
            return 0;
        }
        for (line_idx, spans) in block.text_styles.iter().enumerate() {
            for span in spans {
                if crate::edit::cursor_in_span(span, byte) {
                    return line_idx;
                }
            }
            if spans.is_empty() && block.line_count() == 1 {
                return 0;
            }
        }
        if !block.block_range.is_empty() && block.text_styles.is_empty() {
            return 0;
        }
        0
    }
}
