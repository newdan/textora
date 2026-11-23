use super::super::block::layout_block_at_content_origin;
use super::super::reconcile::BlockReconcilePlan;
use super::super::shaping::populate_style_segments;
use super::super::{apply_deltas, flatten_blocks};
use super::*;
use crate::style::MarkdownStyle;

pub(in crate::layout::types) const VIEWPORT_BUFFER_RATIO: f32 = 0.5;

/// 该文档块是否展开为多个 LaidOutBlock（与 flatten_blocks 的展开规则一致）：
/// 单块重排路径遇到这类块时必须整组处理，不能只取首个输出。
pub(in crate::layout::types) fn expands_into_multiple_laid_out_blocks(
    block: &crate::builder::BlockNode,
) -> bool {
    matches!(
        block.kind,
        crate::builder::BlockKind::Container
            | crate::builder::BlockKind::TableRow_
            | crate::builder::BlockKind::TableCell_ { .. }
    )
}

pub(in crate::layout::types) fn laid_indices_by_document_block(
    document_block_count: usize,
    laid_to_doc: &[usize],
) -> Vec<Vec<usize>> {
    let mut laid_indices = vec![Vec::new(); document_block_count];
    for (laid_index, &document_block_index) in laid_to_doc.iter().enumerate() {
        if let Some(block_indices) = laid_indices.get_mut(document_block_index) {
            block_indices.push(laid_index);
        }
    }
    laid_indices
}

pub(in crate::layout::types) fn take_flat_line_groups<S: BlockSource>(
    layout: &mut LazyLayout<S>,
) -> Vec<Option<Vec<FlatLine>>> {
    let laid_out_count = layout.laid_out.len();
    let mut flat_line_groups: Vec<Option<Vec<FlatLine>>> =
        std::iter::repeat_with(|| None).take(laid_out_count).collect();
    let published_range = if layout.viewport_range.is_empty() {
        0..laid_out_count
    } else {
        layout.viewport_range.clone()
    };
    let mut published_lines = std::mem::take(&mut layout.flat_lines).into_iter();

    for laid_index in published_range {
        let Some(block) = layout.laid_out.get(laid_index).and_then(Option::as_ref) else {
            continue;
        };
        let expected_line_count = flat_line_count(block);
        let mut block_lines = Vec::with_capacity(expected_line_count);
        for _ in 0..expected_line_count {
            let Some(line) = published_lines.next() else {
                return std::iter::repeat_with(|| None).take(laid_out_count).collect();
            };
            block_lines.push(line);
        }
        flat_line_groups[laid_index] = Some(block_lines);
    }

    if published_lines.next().is_some() {
        return std::iter::repeat_with(|| None).take(laid_out_count).collect();
    }
    flat_line_groups
}

pub(in crate::layout::types) fn flat_line_count(block: &LaidOutBlock) -> usize {
    match &block.kind {
        LaidOutBlockKind::Text { lines }
        | LaidOutBlockKind::CodeBlock { lines, .. }
        | LaidOutBlockKind::MetadataBlock { lines } => lines.len(),
        LaidOutBlockKind::BlockQuote { blocks } => blocks.iter().map(flat_line_count).sum(),
        LaidOutBlockKind::ListItem { blocks, lines, .. } => {
            lines.len() + blocks.iter().map(flat_line_count).sum::<usize>()
        }
        LaidOutBlockKind::Table { header, rows, .. } => {
            let header_line_count = header.iter().map(Vec::len).sum::<usize>();
            let body_line_count =
                rows.iter().flat_map(|row| row.iter()).map(Vec::len).sum::<usize>();
            header_line_count + body_line_count
        }
        LaidOutBlockKind::HorizontalRule | LaidOutBlockKind::Embedded { .. } => 1,
    }
}

pub(in crate::layout::types) fn block_tree_contains_code_block(
    block: &crate::builder::BlockNode,
) -> bool {
    matches!(
        block.kind,
        crate::builder::BlockKind::CodeBlock { .. } | crate::builder::BlockKind::DisplayMath
    ) || block
        .text_styles
        .iter()
        .flatten()
        .any(|span| matches!(span.style, crate::builder::InlineStyle::Math))
        || block.children.iter().any(block_tree_contains_code_block)
}

pub(in crate::layout::types) fn block_tree_contains_embedded(
    block: &crate::builder::BlockNode,
) -> bool {
    matches!(block.kind, crate::builder::BlockKind::DisplayMath)
        || matches!(&block.kind, crate::builder::BlockKind::CodeBlock { language: Some(language) } if crate::layout::block::is_mermaid_language(language))
        || block
            .text_styles
            .iter()
            .flatten()
            .any(|span| span.style == crate::builder::InlineStyle::Math)
        || block.children.iter().any(block_tree_contains_embedded)
}

pub(in crate::layout::types) fn block_contains_source_byte(
    block: &crate::builder::BlockNode,
    source_byte: usize,
) -> bool {
    block.block_range.start <= source_byte && source_byte <= block.block_range.end
}

pub(in crate::layout::types) fn ranges_intersect_block(
    source_range: &Range<usize>,
    block: &crate::builder::BlockNode,
) -> bool {
    source_range.start < block.block_range.end && block.block_range.start < source_range.end
}

pub(in crate::layout::types) fn signed_offset(current: usize, previous: usize) -> Option<isize> {
    if current >= previous {
        isize::try_from(current - previous).ok()
    } else {
        isize::try_from(previous - current).ok()?.checked_neg()
    }
}

pub(in crate::layout::types) fn shift_source_byte(
    source_byte: usize,
    source_byte_delta: isize,
) -> usize {
    source_byte.checked_add_signed(source_byte_delta).expect(
        "an unchanged block's source anchors must remain valid after block-start translation",
    )
}

pub(in crate::layout::types) fn shift_source_range(
    source_range: &mut Range<usize>,
    source_byte_delta: isize,
) {
    source_range.start = shift_source_byte(source_range.start, source_byte_delta);
    source_range.end = shift_source_byte(source_range.end, source_byte_delta);
}

pub(in crate::layout::types) fn shift_projection_source(
    projection: &mut VisualLineProjection,
    source_byte_delta: isize,
) {
    projection.owner = match projection.owner {
        ProjectionOwnerId::Block { block_start, logical_line } => ProjectionOwnerId::Block {
            block_start: shift_source_byte(block_start, source_byte_delta),
            logical_line,
        },
        ProjectionOwnerId::TableCell { table_start, row, column, logical_line } => {
            ProjectionOwnerId::TableCell {
                table_start: shift_source_byte(table_start, source_byte_delta),
                row,
                column,
                logical_line,
            }
        }
        ProjectionOwnerId::EmptyLine { source_byte } => ProjectionOwnerId::EmptyLine {
            source_byte: shift_source_byte(source_byte, source_byte_delta),
        },
    };
    for boundary in &mut projection.boundaries {
        boundary.byte = shift_source_byte(boundary.byte, source_byte_delta);
    }
    shift_source_range(&mut projection.source_extent, source_byte_delta);
    for collapsed_boundary in &mut projection.collapsed {
        shift_source_range(&mut collapsed_boundary.source_range, source_byte_delta);
    }
}

pub(in crate::layout::types) fn shift_laid_out_line(
    line: &mut LaidOutLine,
    source_byte_delta: isize,
    y_delta: f32,
) {
    line.rect.y += y_delta;
    for style in &mut line.styles {
        shift_source_range(&mut style.source_range, source_byte_delta);
    }
    if let Some(projection) = &mut line.source_projection {
        shift_projection_source(projection, source_byte_delta);
    }
}

pub(in crate::layout::types) fn shift_laid_out_lines(
    lines: &mut [LaidOutLine],
    source_byte_delta: isize,
    y_delta: f32,
) {
    for line in lines {
        shift_laid_out_line(line, source_byte_delta, y_delta);
    }
}

pub(in crate::layout::types) fn shift_laid_out_block(
    block: &mut LaidOutBlock,
    source_byte_delta: isize,
    y_delta: f32,
) {
    block.rect.y += y_delta;
    match &mut block.kind {
        LaidOutBlockKind::Text { lines }
        | LaidOutBlockKind::CodeBlock { lines, .. }
        | LaidOutBlockKind::MetadataBlock { lines } => {
            shift_laid_out_lines(lines, source_byte_delta, y_delta);
        }
        LaidOutBlockKind::BlockQuote { blocks } => {
            for nested_block in blocks {
                shift_laid_out_block(nested_block, source_byte_delta, y_delta);
            }
        }
        LaidOutBlockKind::ListItem { blocks, lines, .. } => {
            shift_laid_out_lines(lines, source_byte_delta, y_delta);
            for nested_block in blocks {
                shift_laid_out_block(nested_block, source_byte_delta, y_delta);
            }
        }
        LaidOutBlockKind::Table { header, rows, .. } => {
            for cell_lines in header {
                shift_laid_out_lines(cell_lines, source_byte_delta, y_delta);
            }
            for row in rows {
                for cell_lines in row {
                    shift_laid_out_lines(cell_lines, source_byte_delta, y_delta);
                }
            }
        }
        LaidOutBlockKind::HorizontalRule => {}
        LaidOutBlockKind::Embedded { source_range } => {
            source_range.start = source_range
                .start
                .checked_add_signed(source_byte_delta)
                .expect("reused embedded source start stays valid");
            source_range.end = source_range
                .end
                .checked_add_signed(source_byte_delta)
                .expect("reused embedded source end stays valid");
        }
    }
}

// ===== Second impl block for LazyLayout =====

impl<S: BlockSource> LazyLayout<S> {
    // ── Construction ──

    /// Lightweight init: runs estimation pass only (no shaping, no text allocation).
    ///
    /// `laid_out` starts as all `None`. Call `ensure_visible()` followed by
    /// `build_flat_lines()` before accessing flat lines. For full layout
    /// (editing mode), use `from_doc()` or call `ensure_all_blocks()`.
    pub fn new(
        source: S,
        style: &MarkdownStyle,
        viewport_w: f32,
        doc_view: &dyn textora_core::document::DocView,
    ) -> Self {
        let laid_out_est = crate::layout::block::layout_doc_for_estimation(
            source.blocks(),
            style,
            viewport_w,
            doc_view,
        );
        let n = laid_out_est.blocks.len();
        let estimated_heights: Vec<f32> = laid_out_est.blocks.iter().map(|b| b.rect.h).collect();
        let estimated_positions: Vec<f32> = laid_out_est.blocks.iter().map(|b| b.rect.y).collect();
        let total_height = laid_out_est.total_height;
        let laid_to_doc = flatten_blocks(source.blocks());
        // flatten_blocks 镜像 layout_block 的输出基数（Container 与顶层
        // TableRow_/TableCell_ 均按子块展开），两者必须一致。若未来新增块类型
        // 打破该不变量，退化为不可映射（所有槽位跳过物化，渲染为空），
        // 绝不 panic、也不把输出静默错位到错误的文档块。
        let laid_to_doc = if laid_to_doc.len() == n {
            laid_to_doc
        } else {
            debug_assert!(false, "flatten_blocks / layout_doc mismatch");
            vec![usize::MAX; n]
        };

        Self {
            source,
            estimated_heights,
            estimated_positions,
            total_height,
            y_delta: vec![0.0f32; n],
            laid_out: vec![None; n],
            precise: vec![false; n],
            flat_lines: Vec::new(),
            reconciled_flat_lines: std::iter::repeat_with(|| None).take(n).collect(),
            retained_block_projections: vec![Vec::new(); n],
            source_projection_index: None,
            source_projection_error: None,
            source_generation: 0,
            layout_revision: 0,
            laid_to_doc,
            viewport_range: 0..0,
            cached_viewport_w: viewport_w,
            source_text: None,
            projected_empty_lines: Vec::new(),
            hidden_block_separators: Vec::new(),
            projection_flat_line_indices: Vec::new(),
            projected_empty_line_geometry: std::collections::BTreeMap::new(),
            edit_ctx: None,
            code_viewport: None,
            selection_range: None,
            ascii_diagrams: crate::layout::ascii_diagram::AsciiDiagramRegistry::default(),
            embedded_images: crate::layout::embedded::EmbeddedRegistry::default(),
        }
    }

    pub(crate) fn reuse_unchanged_blocks_from(&mut self, mut previous: Self) -> usize {
        let Some(previous_source_text) = previous.source_text.as_deref() else {
            return 0;
        };
        let Some(current_source_text) = self.source_text.as_deref() else {
            return 0;
        };
        let reconcile_plan = BlockReconcilePlan::between(
            previous.source.blocks(),
            previous_source_text,
            self.source.blocks(),
            current_source_text,
        );
        let previous_laid_indices =
            laid_indices_by_document_block(previous.source.blocks().len(), &previous.laid_to_doc);
        let mut previous_flat_line_groups = take_flat_line_groups(&mut previous);
        let mut current_block_ordinals = vec![0usize; self.source.blocks().len()];
        let mut reused_block_count = 0usize;

        for current_laid_index in 0..self.laid_to_doc.len() {
            let current_doc_index = self.laid_to_doc[current_laid_index];
            let Some(previous_doc_index) =
                reconcile_plan.old_index_for_unchanged_new_block(current_doc_index)
            else {
                continue;
            };
            let current_ordinal = current_block_ordinals[current_doc_index];
            current_block_ordinals[current_doc_index] += 1;
            let Some(&previous_laid_index) = previous_laid_indices
                .get(previous_doc_index)
                .and_then(|indices| indices.get(current_ordinal))
            else {
                continue;
            };
            if previous.precise.get(previous_laid_index).copied().unwrap_or(false) {
                continue;
            }
            let Some(current_source_block) = self.source.blocks().get(current_doc_index) else {
                continue;
            };
            let Some(previous_source_block) = previous.source.blocks().get(previous_doc_index)
            else {
                continue;
            };
            let current_cursor_intersects = self.edit_ctx.as_ref().is_some_and(|edit_context| {
                block_contains_source_byte(current_source_block, edit_context.cursor_byte)
            });
            let previous_cursor_intersects =
                previous.edit_ctx.as_ref().is_some_and(|edit_context| {
                    block_contains_source_byte(previous_source_block, edit_context.cursor_byte)
                });
            let current_selection_intersects = self
                .selection_range
                .as_ref()
                .is_some_and(|selection| ranges_intersect_block(selection, current_source_block));
            let previous_selection_intersects = previous
                .selection_range
                .as_ref()
                .is_some_and(|selection| ranges_intersect_block(selection, previous_source_block));
            if current_cursor_intersects
                || previous_cursor_intersects
                || current_selection_intersects
                || previous_selection_intersects
            {
                continue;
            }
            if block_tree_contains_code_block(current_source_block) {
                continue;
            }
            let Some(source_byte_delta) = signed_offset(
                current_source_block.block_range.start,
                previous_source_block.block_range.start,
            ) else {
                continue;
            };
            let Some(mut reused_block) =
                previous.laid_out.get_mut(previous_laid_index).and_then(Option::take)
            else {
                continue;
            };
            let y_delta = self.estimated_positions[current_laid_index]
                - previous.estimated_positions[previous_laid_index];
            shift_laid_out_block(&mut reused_block, source_byte_delta, y_delta);
            if let Some(mut reused_flat_lines) =
                previous_flat_line_groups.get_mut(previous_laid_index).and_then(Option::take)
            {
                let previous_y_correction =
                    previous.y_delta.get(previous_laid_index).copied().unwrap_or(0.0);
                for line in &mut reused_flat_lines {
                    line.rect.y += y_delta - previous_y_correction;
                    if let Some(projection) = &mut line.source_projection {
                        shift_projection_source(projection, source_byte_delta);
                    }
                }
                self.reconciled_flat_lines[current_laid_index] = Some(reused_flat_lines);
            }
            self.retain_block_projections(current_laid_index, &reused_block);
            self.laid_out[current_laid_index] = Some(reused_block);
            reused_block_count += 1;
        }

        reused_block_count
    }

    /// Set the full source text for WYSIWYG span expansion.
    pub fn set_edit_source(&mut self, source_text: Option<String>) {
        self.source_text = source_text;
    }

    /// Set the WYSIWYG edit context (cursor position for span expansion).
    pub fn set_edit_ctx(&mut self, edit_ctx: Option<crate::edit::EditContext>) {
        self.edit_ctx = edit_ctx;
    }

    pub fn set_selection_range(&mut self, selection_range: Option<std::ops::Range<usize>>) {
        if self.selection_range != selection_range {
            let changed_block_starts = self
                .source
                .blocks()
                .iter()
                .filter(|block| block_tree_contains_embedded(block))
                .filter(|block| {
                    self.selection_range
                        .as_ref()
                        .is_some_and(|range| ranges_intersect_block(range, block))
                        || selection_range
                            .as_ref()
                            .is_some_and(|range| ranges_intersect_block(range, block))
                })
                .map(|block| block.block_range.start)
                .collect::<Vec<_>>();
            self.invalidate_lines_for_source_bytes(changed_block_starts);
        }
        self.selection_range = selection_range;
        self.ascii_diagrams.set_selection_range(self.selection_range.clone());
    }

    pub(crate) fn ascii_diagrams(&self) -> &crate::layout::ascii_diagram::AsciiDiagramRegistry {
        &self.ascii_diagrams
    }

    pub(crate) fn embedded_images(&self) -> &crate::layout::embedded::EmbeddedRegistry {
        &self.embedded_images
    }

    pub(in crate::layout::types) fn discard_ascii_diagrams_for_laid_index(
        &mut self,
        laid_idx: usize,
    ) {
        let Some(&doc_idx) = self.laid_to_doc.get(laid_idx) else {
            return;
        };
        let Some(block) = self.source.blocks().get(doc_idx) else {
            return;
        };
        self.ascii_diagrams.remove_source_range(&block.block_range);
        self.embedded_images.remove_source_range(&block.block_range);
    }

    /// Invalidate laid-out blocks whose source text contains any of the given bytes.
    /// Used after cursor movement to only relayout affected blocks (span expansion).
    /// 含旧/新光标字节的块必须无条件失效——即使它在渲染窗口外，否则其
    /// active marker 会在光标离开后永久残留（失效只清标记，不会立即重排屏外块）。
    pub fn invalidate_lines_for_source_bytes(&mut self, bytes: impl IntoIterator<Item = usize>) {
        let source_bytes: Vec<usize> = bytes.into_iter().collect();
        if source_bytes.is_empty() {
            return;
        }

        // Invalidate the laid-out owner, including when the cursor is inside a
        // descendant such as a table cell or nested list item.
        let blocks = self.source.blocks();
        let mut invalidated_indices = Vec::new();
        for (laid_idx, &doc_idx) in self.laid_to_doc.iter().enumerate() {
            let Some(block) = blocks.get(doc_idx) else {
                continue;
            };
            if source_bytes.iter().any(|&byte| block_contains_source_byte(block, byte)) {
                invalidated_indices.push(laid_idx);
            }
        }

        // Release the source borrow before clearing cached layout entries.
        for laid_idx in invalidated_indices {
            if let Some(precise) = self.precise.get_mut(laid_idx) {
                *precise = false;
            }
            self.discard_ascii_diagrams_for_laid_index(laid_idx);
            if let Some(slot) = self.laid_out.get_mut(laid_idx) {
                *slot = None;
            }
            if let Some(slot) = self.retained_block_projections.get_mut(laid_idx) {
                slot.clear();
            }
            self.source_projection_index = None;
        }
    }

    /// Full layout: runs estimation + stores all laid-out blocks.
    /// Use for editing mode or when all blocks must be accessible.
    /// After calling this, `laid_out` is fully populated.
    pub fn from_doc(
        source: S,
        style: &MarkdownStyle,
        viewport_w: f32,
        doc_view: &dyn textora_core::document::DocView,
    ) -> Self {
        let mut this = Self::new(source, style, viewport_w, doc_view);
        this.ensure_all_blocks(style, viewport_w, None, None, doc_view);
        this.build_flat_lines(doc_view);
        this
    }

    // ── Visible-block materialization ──

    /// Binary search: find the first laid-out block that intersects `y`.
    /// Uses `estimated_positions + y_delta` (real positions) + `estimated_heights`.
    /// Returns index in `0..estimated_heights.len()`.
    pub fn block_at_y(&self, y: f32) -> usize {
        let n = self.estimated_heights.len();
        if n == 0 {
            return 0;
        }
        let mut lo = 0;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let mid_top =
                self.estimated_positions[mid] + self.y_delta.get(mid).copied().unwrap_or(0.0);
            let mid_bottom = mid_top + self.estimated_heights[mid];
            if mid_bottom <= y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.min(n - 1)
    }

    /// Materialize blocks in the visible range and evict those outside it.
    /// Must be called before `build_flat_lines()` when using viewport-driven mode.
    ///
    /// `buffer_ratio` controls how much extra content to materialize above/below
    /// the viewport (default 0.5 = half a screen).
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_visible(
        &mut self,
        scroll_y: f32,
        viewport_h: f32,
        style: &MarkdownStyle,
        viewport_w: f32,
        shaper: &mut shaping::Shaper,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn textora_core::document::DocView,
    ) {
        let buffer = viewport_h * VIEWPORT_BUFFER_RATIO;
        let visible_top = (scroll_y - buffer).max(0.0);
        let visible_bottom = scroll_y + viewport_h + buffer;

        let start_block = self.block_at_y(visible_top);
        let end_block = self
            .block_at_y(visible_bottom)
            .min(self.estimated_heights.len())
            .saturating_add(1)
            .min(self.estimated_heights.len());

        let new_range = start_block..end_block;

        // Evict blocks that left the visible range.
        if new_range != self.viewport_range {
            self.evict_outside(&new_range);
        }

        // Materialize blocks newly entering the visible range.
        // Collect per-block deltas for batch application.
        let mut deltas: Vec<(usize, f32)> = Vec::new();
        for i in new_range.clone() {
            if self.laid_out[i].is_some() {
                continue;
            }
            self.discard_ascii_diagrams_for_laid_index(i);
            let doc_idx = match self.laid_to_doc.get(i).copied() {
                Some(di) if di < self.source.blocks().len() => di,
                _ => continue,
            };
            let src_block = &self.source.blocks()[doc_idx];
            if expands_into_multiple_laid_out_blocks(src_block) {
                let group_deltas =
                    self.relayout_multi_output_group(i, style, Some(shaper), highlighter, doc);
                deltas.extend(group_deltas);
                continue;
            }

            let base_y = self.estimated_positions[i];
            let mut ctx = crate::layout::context::LayoutCtx::new(
                doc,
                style,
                viewport_w,
                Some(shaper),
                highlighter,
                self.source_text.as_deref(),
                self.edit_ctx.as_ref(),
            );
            ctx.selection_range = self.selection_range.as_ref();
            ctx.ascii_diagrams.set_selection_range(self.selection_range.clone());
            ctx.y = base_y;
            ctx.indent = 0.0;
            layout_block_at_content_origin(src_block, &mut ctx);
            self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
            self.embedded_images.extend(std::mem::take(&mut ctx.embedded_images));
            if let Some(mut new_block) = ctx.output.into_iter().next() {
                populate_style_segments(&mut new_block, shaper, style);
                let old_height = self.estimated_heights[i];
                let old_bottom = base_y + old_height;
                let new_bottom = new_block.rect.y + new_block.rect.h;
                let delta = new_bottom - old_bottom;
                self.estimated_heights[i] = new_bottom - base_y;

                self.retain_block_projections(i, &new_block);
                self.laid_out[i] = Some(new_block);
                self.precise[i] = true;

                // Collect delta for batch application.
                if delta.abs() > 0.5 {
                    deltas.push((i, delta));
                }
                self.total_height += delta;
            }
        }

        // Batch-apply deltas to y_delta (cumulative convention).
        if !deltas.is_empty() {
            apply_deltas(&mut self.y_delta, &deltas);
        }

        self.viewport_range = new_range;
        let _ = self.rebuild_source_projection_index();
    }

    /// Evict (clear) all blocks outside the given range.
    /// This frees HarfBuzz shaped data and text allocations for distant blocks.
    pub(in crate::layout::types) fn evict_outside(&mut self, keep: &std::ops::Range<usize>) {
        for i in 0..self.laid_out.len() {
            if !keep.contains(&i) {
                self.discard_ascii_diagrams_for_laid_index(i);
                self.laid_out[i] = None;
            }
        }
    }

    /// Materialize all blocks (full layout). Use for editing mode or when
    /// the entire flat_lines array is needed (search, selection across doc).
    pub fn ensure_all_blocks(
        &mut self,
        style: &MarkdownStyle,
        viewport_w: f32,
        mut shaper: Option<&mut shaping::Shaper>,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn textora_core::document::DocView,
    ) {
        let full_range = 0..self.estimated_heights.len();
        if full_range.is_empty() {
            return;
        }
        let has_shaper = shaper.is_some();
        let mut deltas: Vec<(usize, f32)> = Vec::new();

        // Materialize all blocks (in order) using the same layout logic.
        for i in 0..self.estimated_heights.len() {
            if self.laid_out[i].is_some() {
                continue;
            }
            self.discard_ascii_diagrams_for_laid_index(i);
            let doc_idx = match self.laid_to_doc.get(i).copied() {
                Some(di) if di < self.source.blocks().len() => di,
                _ => continue,
            };
            let src_block = &self.source.blocks()[doc_idx];
            if expands_into_multiple_laid_out_blocks(src_block) {
                let group_deltas = self.relayout_multi_output_group(
                    i,
                    style,
                    shaper.as_deref_mut(),
                    highlighter,
                    doc,
                );
                deltas.extend(group_deltas);
                continue;
            }
            let mut ctx = crate::layout::context::LayoutCtx::new(
                doc,
                style,
                viewport_w,
                shaper.as_deref_mut(),
                highlighter,
                self.source_text.as_deref(),
                self.edit_ctx.as_ref(),
            );
            ctx.selection_range = self.selection_range.as_ref();
            ctx.ascii_diagrams.set_selection_range(self.selection_range.clone());
            ctx.y = self.estimated_positions[i];
            let base_y = self.estimated_positions[i];
            let old_height = self.estimated_heights[i];
            let old_bottom = base_y + old_height;

            layout_block_at_content_origin(src_block, &mut ctx);
            self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
            self.embedded_images.extend(std::mem::take(&mut ctx.embedded_images));
            if let Some(mut new_block) = ctx.output.into_iter().next() {
                // Populate style segments when shaper is available (same as ensure_visible).
                if let Some(ref mut s) = shaper {
                    populate_style_segments(&mut new_block, s, style);
                }
                let new_bottom = new_block.rect.y + new_block.rect.h;
                let delta = new_bottom - old_bottom;
                self.estimated_heights[i] = new_bottom - base_y;

                self.retain_block_projections(i, &new_block);
                self.laid_out[i] = Some(new_block);
                self.precise[i] = has_shaper;

                if delta.abs() > 0.5 {
                    deltas.push((i, delta));
                }
                self.total_height += delta;
            }
        }

        // Batch-apply deltas to y_delta (same pattern as ensure_visible).
        if !deltas.is_empty() {
            apply_deltas(&mut self.y_delta, &deltas);
        }

        self.viewport_range = full_range;
        if !self.flat_lines.is_empty() {
            let _ = self.rebuild_source_projection_index();
        }
    }

    /// Ensure y_delta and precise arrays are at least as long as estimated_heights.
    pub(in crate::layout::types) fn ensure_consistent(&mut self) {
        let n = self.estimated_heights.len();
        if self.y_delta.len() < n {
            let last = *self.y_delta.last().unwrap_or(&0.0);
            self.y_delta.resize(n, last);
        }
        if self.precise.len() < n {
            self.precise.resize(n, false);
        }
        if self.laid_out.len() < n {
            self.laid_out.resize_with(n, || None);
        }
        if self.retained_block_projections.len() < n {
            self.retained_block_projections.resize_with(n, Vec::new);
        }
        if self.reconciled_flat_lines.len() < n {
            self.reconciled_flat_lines.resize_with(n, || None);
        }
    }

    /// Collect all materialized blocks into a LaidOutDoc for the rendering pass.
    /// Also returns a y_delta slice mapped to the output block order.
    pub fn materialized_blocks(&self) -> (LaidOutDoc, Vec<f32>) {
        let mut blocks = Vec::new();
        let mut mapped_yd = Vec::new();
        for (i, b) in self.laid_out.iter().enumerate() {
            if let Some(block) = b {
                blocks.push(block.clone());
                mapped_yd.push(self.y_delta.get(i).copied().unwrap_or(0.0));
            }
        }
        (LaidOutDoc { blocks, total_height: self.total_height }, mapped_yd)
    }

    /// Ensure all top-level blocks whose rect intersects [scroll_y - buffer, scroll_y + vh + buffer]
    /// are precision-laid-out. Returns list of (block_idx, height_delta) for blocks that
    /// transitioned from estimated to precise.
    pub fn ensure_precise_range(
        &mut self,
        scroll_y: f32,
        viewport_h: f32,
        style: &MarkdownStyle,
        shaper: &mut shaping::Shaper,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn textora_core::document::DocView,
    ) -> Vec<(usize, f32)> {
        self.ensure_consistent();
        let buffer = viewport_h * VIEWPORT_BUFFER_RATIO;
        let range_start = (scroll_y - buffer).max(0.0);
        let range_end = scroll_y + viewport_h + buffer;

        let n = self.estimated_heights.len();

        // Collect indices of blocks that need precision pass.
        let mut indices: Vec<usize> = Vec::new();
        for i in 0..n {
            let block_top =
                self.estimated_positions[i] + self.y_delta.get(i).copied().unwrap_or(0.0);
            let block_bottom = block_top + self.estimated_heights[i];

            if block_bottom < range_start {
                continue;
            }
            if block_top > range_end {
                break;
            }
            if self.precise[i] {
                continue;
            }
            indices.push(i);
        }

        let mut deltas: Vec<(usize, f32)> = Vec::new();
        for i in indices {
            let delta = self.precise_block_at(i, style, shaper, highlighter, doc);
            if delta.abs() > 0.5 {
                deltas.push((i, delta));
            }
        }

        if !deltas.is_empty() {
            apply_deltas(&mut self.y_delta, &deltas);
        }
        if !deltas.is_empty() {
            self.build_flat_lines(doc);
        }

        // Update viewport_range so blocks materialized by this method are
        // visible to evict_outside() and build_flat_lines().
        let start_block = self.block_at_y(range_start);
        let end_block = self.block_at_y(range_end).min(n).saturating_add(1).min(n);
        let new_vp_range = start_block..end_block;
        // Only expand — never shrink (ensure_visible handles eviction).
        if new_vp_range.start < self.viewport_range.start
            || new_vp_range.end > self.viewport_range.end
        {
            let merged = self.viewport_range.start.min(new_vp_range.start)
                ..self.viewport_range.end.max(new_vp_range.end);
            self.viewport_range = merged;
        }

        deltas
    }

    /// Re-run precise layout for the viewport range even when blocks were
    /// already shaped by a cheap whole-document pass. This lets callers defer
    /// expensive adornments such as syntax highlighting to visible blocks.
    pub fn refresh_precise_range(
        &mut self,
        scroll_y: f32,
        viewport_h: f32,
        style: &MarkdownStyle,
        shaper: &mut shaping::Shaper,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn textora_core::document::DocView,
    ) -> Vec<(usize, f32)> {
        self.ensure_consistent();
        let buffer = viewport_h * VIEWPORT_BUFFER_RATIO;
        let range_start = (scroll_y - buffer).max(0.0);
        let range_end = scroll_y + viewport_h + buffer;
        let n = self.estimated_heights.len();
        let start_block = self.block_at_y(range_start);
        let end_block = self.block_at_y(range_end).min(n).saturating_add(1).min(n);

        let mut deltas = Vec::new();
        for i in start_block..end_block {
            let delta = self.precise_block_at(i, style, shaper, highlighter, doc);
            if delta.abs() > 0.5 {
                deltas.push((i, delta));
            }
        }

        if !deltas.is_empty() {
            apply_deltas(&mut self.y_delta, &deltas);
            self.build_flat_lines(doc);
        }

        deltas
    }

    /// 一个文档块展开为多个 LaidOutBlock（根 Container，或意外到达顶层的
    /// TableRow_/TableCell_）时的整组重排：一次 layout_block 的全部输出按序
    /// 写入该组各槽位，避免只取首个输出而静默丢弃/复制后续块。
    /// 返回各槽位的高度 delta。该路径在当前 parser 输出下不可达，仅为消除
    /// 单块重排的静默丢弃而存在；首个内容位置由估计布局提供。
    pub(in crate::layout::types) fn relayout_multi_output_group(
        &mut self,
        laid_idx: usize,
        style: &MarkdownStyle,
        mut shaper: Option<&mut shaping::Shaper>,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn textora_core::document::DocView,
    ) -> Vec<(usize, f32)> {
        let Some(doc_idx) = self.laid_to_doc.get(laid_idx).copied() else {
            return Vec::new();
        };
        let Some(src_block) = self.source.blocks().get(doc_idx) else {
            return Vec::new();
        };
        let ordinal = self.laid_to_doc[..laid_idx].iter().filter(|&&d| d == doc_idx).count();
        let group_start = laid_idx - ordinal;
        let base_y = self.estimated_positions[group_start];
        let mut ctx = crate::layout::context::LayoutCtx::new(
            doc,
            style,
            self.cached_viewport_w,
            shaper.as_deref_mut(),
            highlighter,
            self.source_text.as_deref(),
            self.edit_ctx.as_ref(),
        );
        ctx.selection_range = self.selection_range.as_ref();
        ctx.ascii_diagrams.set_selection_range(self.selection_range.clone());
        ctx.y = base_y;
        ctx.indent = 0.0;
        layout_block_at_content_origin(src_block, &mut ctx);
        self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
        self.embedded_images.extend(std::mem::take(&mut ctx.embedded_images));
        let outputs = std::mem::take(&mut ctx.output);

        // ctx 不再使用，释放其对 shaper 的借用。
        let has_shaper = shaper.is_some();
        let mut deltas = Vec::new();
        for (offset, mut new_block) in outputs.into_iter().enumerate() {
            let slot = group_start + offset;
            if slot >= self.estimated_heights.len() {
                break;
            }
            if let Some(ref mut s) = shaper {
                populate_style_segments(&mut new_block, s, style);
            }
            let normalized_y_delta = self.estimated_positions[slot] - new_block.rect.y;
            shift_laid_out_block(&mut new_block, 0, normalized_y_delta);
            let old_height = self.estimated_heights[slot];
            let new_height = new_block.rect.h;
            let delta = new_height - old_height;
            self.estimated_heights[slot] = new_height;
            self.discard_ascii_diagrams_for_laid_index(slot);
            self.retain_block_projections(slot, &new_block);
            self.laid_out[slot] = Some(new_block);
            self.precise[slot] = has_shaper;
            if delta.abs() > 0.5 {
                deltas.push((slot, delta));
            }
            self.total_height += delta;
        }
        deltas
    }

    /// Re-precision-layout a single block with full HarfBuzz shaping.
    /// Returns the height delta. Caller should propagate y_delta if needed.
    /// After calling this, `flat_lines` may be stale — call `build_flat_lines()` if needed.
    pub(crate) fn precise_block_at(
        &mut self,
        idx: usize,
        style: &MarkdownStyle,
        shaper: &mut shaping::Shaper,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn textora_core::document::DocView,
    ) -> f32 {
        self.ensure_consistent();
        if idx >= self.estimated_heights.len() || idx >= self.y_delta.len() {
            return 0.0;
        }
        let doc_idx = self.laid_to_doc[idx];
        if doc_idx >= self.source.blocks().len() {
            return 0.0;
        }
        if expands_into_multiple_laid_out_blocks(&self.source.blocks()[doc_idx]) {
            // 多输出块整组重排；delta 已在组内按槽位计算，此处直接应用并重建
            // flat lines，返回值保持 0 以免调用方重复应用。
            let group_deltas =
                self.relayout_multi_output_group(idx, style, Some(shaper), highlighter, doc);
            if !group_deltas.is_empty() {
                apply_deltas(&mut self.y_delta, &group_deltas);
                self.build_flat_lines(doc);
            }
            return 0.0;
        }
        // Boundary spacing is already resolved in estimated_positions. Precision
        // changes only content height and propagates that difference to later blocks.
        let old_height = self.estimated_heights[idx];
        let base_y = self.estimated_positions[idx];
        self.discard_ascii_diagrams_for_laid_index(idx);
        let src_block = &self.source.blocks()[doc_idx];
        let estimated_w = self.laid_out[idx]
            .as_ref()
            .map(|b| b.rect.w + b.rect.x)
            .unwrap_or(self.cached_viewport_w);
        let estimated_indent = self.laid_out[idx].as_ref().map(|b| b.rect.x).unwrap_or(0.0);
        let mut ctx = crate::layout::context::LayoutCtx::new(
            doc,
            style,
            estimated_w,
            Some(shaper),
            highlighter,
            self.source_text.as_deref(),
            self.edit_ctx.as_ref(),
        );
        ctx.selection_range = self.selection_range.as_ref();
        ctx.ascii_diagrams.set_selection_range(self.selection_range.clone());
        ctx.y = base_y;
        ctx.indent = estimated_indent;
        layout_block_at_content_origin(src_block, &mut ctx);
        self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
        self.embedded_images.extend(std::mem::take(&mut ctx.embedded_images));
        if let Some(mut new_block) = ctx.output.into_iter().next() {
            populate_style_segments(&mut new_block, shaper, style);
            self.retain_block_projections(idx, &new_block);
            self.laid_out[idx] = Some(new_block);
        }
        self.precise[idx] = true;
        // Compute delta as bottom position change, not just height change.
        let old_bottom = base_y + old_height;
        let new_bottom = if let Some(b) = &self.laid_out[idx] {
            b.rect.y + b.rect.h
        } else {
            return 0.0;
        };
        let delta = new_bottom - old_bottom;
        self.estimated_heights[idx] = new_bottom - base_y;
        self.total_height += delta;
        delta
    }
}
