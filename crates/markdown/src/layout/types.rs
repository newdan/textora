//! Layout output types and LazyLayout implementation.

use crate::builder::StyleSpan;
use crate::projection::{ProjectionError, ProjectionOwnerId, SourceAnchor, SourceProjectionIndex};
use crate::style::MarkdownStyle;
use shaping::Weight;
use std::ops::Range;
use std::sync::Arc;
use ui::core::geom::Rect;

use super::block::{layout_block, layout_doc_with_shaper};
use super::reconcile::BlockReconcilePlan;
use super::shaping::populate_style_segments;
use super::source_line_map::{HiddenBlockSeparator, RenderedLineLayout, SourceLineMap};
use super::{BlockSource, apply_deltas, flatten_blocks};

/// Ratio of viewport height used as buffer above and below for lazy materialization.
const VIEWPORT_BUFFER_RATIO: f32 = 0.5;
fn count_newlines_before_byte(source_bytes: &[u8], byte: usize) -> usize {
    let mut newline_count = 0usize;
    let mut cursor = byte.min(source_bytes.len());
    while cursor > 0 && source_bytes[cursor - 1] == b'\n' {
        newline_count += 1;
        cursor -= 1;
    }
    newline_count
}

fn first_non_newline_at_or_after(source_bytes: &[u8], byte: usize) -> usize {
    let mut cursor = byte.min(source_bytes.len());
    while cursor < source_bytes.len() && source_bytes[cursor] == b'\n' {
        cursor += 1;
    }
    cursor
}

/// A wrapped text segment with byte range in the original text.
/// Used to correctly map style spans after wrapping via compute_visual_lines.
#[derive(Clone, Debug)]
pub(crate) struct WrappedLine {
    pub text: String,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollapsedBoundary {
    pub source_range: Range<usize>,
    pub upstream_grapheme: usize,
    pub downstream_grapheme: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VisualLineProjection {
    pub flat_line_idx: usize,
    pub owner: ProjectionOwnerId,
    pub boundaries: Vec<SourceAnchor>,
    pub source_extent: Range<usize>,
    pub collapsed: Vec<CollapsedBoundary>,
}

impl VisualLineProjection {
    pub(crate) fn empty(
        flat_line_idx: usize,
        source_byte: usize,
        owner: ProjectionOwnerId,
    ) -> Self {
        Self {
            flat_line_idx,
            owner,
            boundaries: vec![SourceAnchor::downstream(source_byte)],
            source_extent: source_byte..source_byte,
            collapsed: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct RetainedVisualLineProjection {
    projection: VisualLineProjection,
    /// Uncorrected document y coordinate. The owning block's current `y_delta`
    /// is applied only when publishing the projection index.
    y: f32,
}

// ===== Layout output types =====

#[derive(Clone, Debug)]
pub struct LaidOutDoc {
    pub blocks: Vec<LaidOutBlock>,
    pub total_height: f32,
}

/// Lazy two-phase layout: estimation pass for all blocks, precision pass on demand.
///
/// Generic over `S: BlockSource` so that Phase 3 can swap `MarkdownDoc` for
/// `NovelStructure` without rewriting the layout engine.
///
/// Phase 4: viewport-driven layout. `new()` only computes estimated heights
/// (no shaping, no text allocation). `ensure_visible()` materializes visible
/// blocks on demand. `la laid_out` is sparse — only blocks in the visible
/// range are Some. Call `ensure_all_blocks()` for full layout (editing mode).
#[derive(Clone, Debug)]
pub struct LazyLayout<S: BlockSource> {
    /// Source document structure (MarkdownDoc or NovelStructure).
    pub source: S,
    /// Estimated height of each laid-out block (from estimation pass, no shaping).
    pub estimated_heights: Vec<f32>,
    /// Estimated y-position (top) of each laid-out block. Precomputed prefix sum.
    pub estimated_positions: Vec<f32>,
    /// y_delta[i] = cumulative height correction for block i and beyond.
    /// Real visual Y of block i = estimated_positions[i] + y_delta[i].
    pub y_delta: Vec<f32>,
    /// Total document height (estimated + y_delta corrections).
    pub total_height: f32,
    /// Materialized blocks. None until ensure_visible or precision pass fills them.
    /// Indexed by laid-out block index (after Container expansion).
    pub laid_out: Vec<Option<LaidOutBlock>>,
    /// Per laid-out block: has precision layout (with shaping) been done?
    pub precise: Vec<bool>,
    /// Flattened lines in reading order, for selection indexing.
    pub flat_lines: Vec<FlatLine>,
    /// Reconciled uncorrected flat-line groups waiting for their next publication.
    reconciled_flat_lines: Vec<Option<Vec<FlatLine>>>,
    /// Lightweight source projections retained after shaped blocks are evicted.
    retained_block_projections: Vec<Vec<RetainedVisualLineProjection>>,
    /// Canonical generation-safe mapping between source anchors and visual positions.
    pub(crate) source_projection_index: Option<SourceProjectionIndex>,
    /// Most recent validation failure while publishing the canonical source projection index.
    pub(crate) source_projection_error: Option<ProjectionError>,
    /// Generation of the source text used to build the canonical projection index.
    source_generation: u32,
    /// Monotonic revision of successfully published projection indexes.
    layout_revision: u64,
    /// Maps laid_out index → doc.blocks index.
    /// Needed because Container blocks expand into multiple laid-out blocks.
    laid_to_doc: Vec<usize>,
    /// Currently visible block range (for eviction tracking).
    viewport_range: std::ops::Range<usize>,
    /// Cached viewport width from construction — used by precise_block_at
    /// when the block hasn't been materialized yet.
    cached_viewport_w: f32,
    /// Full source text for WYSIWYG span expansion via materialize_line.
    source_text: Option<String>,
    /// Geometry used to materialize editable source-only empty lines.
    source_line_height: f32,
    /// Geometry reserved for a hidden separator between rendered blocks.
    hidden_separator_height: f32,
    /// Synthetic editable empty-line projections. These are deliberately kept
    /// out of `flat_lines`, which is the public rendered-line collection.
    projected_empty_lines: Vec<super::source_line_map::ProjectedEmptyLine>,
    /// Hidden source ranges represented as collapsed boundaries in the index.
    hidden_block_separators: Vec<HiddenBlockSeparator>,
    /// Projection visual-line id → rendered `flat_lines` index. `None` denotes
    /// a zero-grapheme editable empty source line.
    projection_flat_line_indices: Vec<Option<usize>>,
    /// Projection visual-line id → geometry for a synthetic empty source line.
    projected_empty_line_geometry:
        std::collections::BTreeMap<usize, super::source_line_map::ProjectedEmptyLine>,
    /// WYSIWYG edit context; None means pure preview (no cursor span expansion).
    edit_ctx: Option<crate::edit::EditContext>,
    /// Non-empty source selection range used for block-level editing fallbacks.
    selection_range: Option<std::ops::Range<usize>>,
    /// Crate-private rendering sidecar for ASCII diagram grid metadata.
    ascii_diagrams: super::ascii_diagram::AsciiDiagramRegistry,
}

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

    fn retained_visual_lines(&self) -> Vec<(VisualLineProjection, f32)> {
        let mut visual_lines = Vec::new();

        for (block_idx, projections) in self.retained_block_projections.iter().enumerate() {
            let y_correction = self.y_delta.get(block_idx).copied().unwrap_or(0.0);
            for projection in projections {
                visual_lines.push((projection.projection.clone(), projection.y + y_correction));
            }
        }

        visual_lines
    }

    fn publish_source_projection_index(
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
            let doc_children = doc_block.map(|b| b.children.as_slice());

            Self::flatten_block_into(
                block,
                y_delta,
                &mut lines,
                &mut flat_idx,
                current_line_base,
                doc_children,
            );
        }
        self.flat_lines = lines;

        let (projected_empty_lines, hidden_block_separators) =
            self.collect_source_only_empty_line_projections();
        self.projected_empty_lines = projected_empty_lines;
        self.hidden_block_separators = hidden_block_separators;
        if let Some(projected_content_bottom) = self
            .projected_empty_lines
            .iter()
            .map(|line| line.y_top + line.height)
            .max_by(f32::total_cmp)
        {
            self.total_height = self.total_height.max(projected_content_bottom);
        }

        let _ = self.publish_flat_line_projection_index();
    }

    fn collect_source_only_empty_line_projections(
        &self,
    ) -> (Vec<super::source_line_map::ProjectedEmptyLine>, Vec<HiddenBlockSeparator>) {
        let Some(source_text) = self.source_text.as_deref() else {
            return (Vec::new(), Vec::new());
        };
        if source_text.is_empty() || self.source_line_height <= 0.0 {
            return (Vec::new(), Vec::new());
        }

        let rendered_lines = self
            .flat_lines
            .iter()
            .filter_map(|line| {
                let projection = line.source_projection.as_ref()?;
                Some(RenderedLineLayout {
                    source_range: projection.source_extent.clone(),
                    y_top: line.rect.y,
                    height: line.rect.h,
                })
            })
            .collect::<Vec<_>>();
        let mut source_line_map = SourceLineMap::from_source(source_text);
        source_line_map.attach_layout(&rendered_lines, self.source_line_height);

        let mut projected_empty_lines = source_line_map.projected_empty_lines().collect::<Vec<_>>();
        for empty_line in &mut projected_empty_lines {
            let Some(source_line) = source_line_map.line_at_byte(empty_line.source_byte) else {
                continue;
            };
            let Some(run_position) = source_line_map.empty_run_position(source_line.index) else {
                continue;
            };
            let run_ends_at_document_end = source_line.index
                + (run_position.run_length - run_position.index_in_run)
                == source_line_map.lines().len();
            if !run_ends_at_document_end || run_position.run_length < 2 {
                continue;
            }
            if run_position.index_in_run == 0 {
                empty_line.height = self.hidden_separator_height;
            } else {
                empty_line.y_top -= self.source_line_height - self.hidden_separator_height;
            }
        }

        (projected_empty_lines, source_line_map.hidden_block_separators().collect())
    }

    fn add_hidden_separator_collapsed_ranges(
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

    fn add_inter_line_collapsed_ranges(visual_lines: &mut [VisualLineProjection]) {
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

    fn add_document_edge_collapsed_ranges(
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

    fn publish_flat_line_projection_index(&mut self) -> Result<(), ProjectionError> {
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
        projection_candidates.extend(self.projected_empty_lines.iter().copied().map(
            |empty_line| {
                (
                    VisualLineProjection::empty(0, empty_line.source_byte, empty_line.owner),
                    None,
                    empty_line.y_top,
                )
            },
        ));
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
            if flat_line_idx.is_none() {
                let source_byte = projection.source_extent.start;
                if let Some(empty_line) = self
                    .projected_empty_lines
                    .iter()
                    .copied()
                    .find(|empty_line| empty_line.source_byte == source_byte)
                {
                    self.projected_empty_line_geometry.insert(visual_line_idx, empty_line);
                }
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

    fn same_visual_line_identity(
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

    pub(crate) fn projected_empty_line_for_projection(
        &self,
        visual_line_idx: usize,
    ) -> Option<super::source_line_map::ProjectedEmptyLine> {
        self.projected_empty_line_geometry.get(&visual_line_idx).copied()
    }

    pub(crate) fn projection_visual_line_idx_for_empty_source_byte(
        &self,
        source_byte: usize,
    ) -> Option<usize> {
        self.projected_empty_line_geometry.iter().find_map(|(&visual_line_idx, empty_line)| {
            (empty_line.source_byte == source_byte).then_some(visual_line_idx)
        })
    }

    fn source_line_terminal_boundary(source: &str, boundary: usize) -> usize {
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

    fn retain_block_projections(&mut self, block_idx: usize, block: &LaidOutBlock) {
        let mut projections = Vec::new();
        Self::collect_block_projections(block, &mut projections);

        if let Some(slot) = self.retained_block_projections.get_mut(block_idx) {
            *slot = projections;
        }
    }

    fn collect_block_projections(
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
            LaidOutBlockKind::HorizontalRule => {}
        }
    }

    fn collect_line_projections(
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
    fn count_until_target(
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
    fn push_flat_line(
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
        });
        *flat_idx += 1;
    }

    /// Recursively flatten a block's lines into the flat array.
    /// `block_line_base` is a unique flat index for this doc block
    /// (total text_lines of all preceding blocks in tree-walk order).
    /// `doc_children` is the children slice of that doc block (for recursive descent).
    fn flatten_block_into(
        block: &LaidOutBlock,
        y_correction: f32,
        flat: &mut Vec<FlatLine>,
        flat_idx: &mut usize,
        block_line_base: usize,
        doc_children: Option<&[crate::builder::BlockNode]>,
    ) {
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
                    let grandchildren = child.map(|c| c.children.as_slice());
                    Self::flatten_block_into(
                        sub,
                        y_correction,
                        flat,
                        flat_idx,
                        child_offset,
                        grandchildren,
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
                    let grandchildren = child.map(|c| c.children.as_slice());
                    Self::flatten_block_into(
                        sub,
                        y_correction,
                        flat,
                        flat_idx,
                        child_offset,
                        grandchildren,
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
            LaidOutBlockKind::HorizontalRule => {
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
                });
                *flat_idx += 1;
            }
        }
    }

    /// Count all text_lines in a subtree (used for offset calculation).
    fn count_all_descendant_lines(blocks: &[crate::builder::BlockNode]) -> usize {
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

    fn search_block_line_at_byte(
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

    fn search_block_line_at_byte_with_offset(
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
    fn find_line_idx_in_block(block: &crate::builder::BlockNode, byte: usize) -> usize {
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

#[derive(Clone, Debug)]
pub struct LaidOutBlock {
    pub kind: LaidOutBlockKind,
    pub rect: Rect,
}

#[derive(Clone, Debug)]
pub enum LaidOutBlockKind {
    Text {
        lines: Vec<LaidOutLine>,
    },
    CodeBlock {
        lines: Vec<LaidOutLine>,
        language: Option<String>,
    },
    BlockQuote {
        blocks: Vec<LaidOutBlock>,
    },
    ListItem {
        bullet: crate::builder::ListBullet,
        blocks: Vec<LaidOutBlock>,
        lines: Vec<LaidOutLine>,
        level_indent: f32,
        depth: usize,
    },
    Table {
        columns: usize,
        header: Vec<Vec<LaidOutLine>>,
        rows: Vec<Vec<Vec<LaidOutLine>>>,
        column_widths: Vec<f32>,
        /// Header row height in pixels. 0.0 if no header.
        header_height: f32,
        /// Body row heights in pixels, one per row.
        row_heights: Vec<f32>,
    },
    HorizontalRule,
    MetadataBlock {
        lines: Vec<LaidOutLine>,
    },
}

/// A text line with absolute document position, for flat-indexed selection.
///
/// Invariant: `flat_lines` is sorted by `rect.y` ascending (reading order).
#[derive(Clone, Debug)]
pub struct FlatLine {
    /// 0-based index in the flat reading-order array.
    pub flat_idx: usize,
    /// Line rect with y in absolute document coordinates (block y + delta + relative line y).
    pub rect: Rect,
    pub text: String,
    /// Font size in pixels. 0.0 for non-text elements (e.g., HorizontalRule).
    pub font_size: f32,
    /// Whether this line uses the configured code font family.
    pub is_code: bool,
    pub shaped: Option<shaping::ShapedRun>,
    /// Whether this rendered line is editable source content and must retain a projection.
    pub(crate) requires_source_projection: bool,
    /// Source projection for this exact visual-line segment.
    pub(crate) source_projection: Option<VisualLineProjection>,
}

#[derive(Clone, Debug)]
pub struct LaidOutLine {
    pub text: String,
    pub rect: Rect,
    pub font_size: f32,
    pub is_code: bool,
    pub font_weight: Weight,
    pub color_override: Option<[f32; 4]>,
    /// Index of this line in the original source block.
    pub doc_line_idx: usize,
    /// Inline style spans (bold, italic, code, link).
    pub styles: Vec<StyleSpan>,
    /// Precomputed style segments with precise x positions (relative to line start).
    /// Used by render pass to position styled text without width estimation.
    pub style_segments: Vec<StyleSegment>,
    /// Harfbuzz shape result (layout phase produces, render phase consumes).
    pub shaped: Option<shaping::ShapedRun>,
    /// Pre-built TextLayout (created during layout, reused across frames, ID stable).
    /// Render directly takes Arc and passes it to DrawCmd, no need to rebuild.
    pub text_layout: Option<Arc<ui::core::text_layout::UiTextLayout>>,
    /// Syntax highlight spans for code lines. Empty for non-code lines.
    pub highlight_spans: Vec<crate::builder::HighlightSpan>,
    /// Source projection for this exact wrapped segment.
    pub(crate) source_projection: Option<VisualLineProjection>,
}

/// A styled text segment with precomputed position.
#[derive(Clone, Debug)]
pub struct StyleSegment {
    /// Byte range within the line text.
    pub start: usize,
    pub len: usize,
    /// X offset from line start (pixels, precise via shaper).
    pub x_offset: f32,
    /// Precise width of this segment (pixels, via shaper).
    pub width: f32,
    /// The inline style for this segment.
    pub style: crate::builder::InlineStyle,
}

/// 该文档块是否展开为多个 LaidOutBlock（与 flatten_blocks 的展开规则一致）：
/// 单块重排路径遇到这类块时必须整组处理，不能只取首个输出。
fn expands_into_multiple_laid_out_blocks(block: &crate::builder::BlockNode) -> bool {
    matches!(
        block.kind,
        crate::builder::BlockKind::Container
            | crate::builder::BlockKind::TableRow_
            | crate::builder::BlockKind::TableCell_ { .. }
    )
}

fn laid_indices_by_document_block(
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

fn take_flat_line_groups<S: BlockSource>(layout: &mut LazyLayout<S>) -> Vec<Option<Vec<FlatLine>>> {
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

fn flat_line_count(block: &LaidOutBlock) -> usize {
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
        LaidOutBlockKind::HorizontalRule => 1,
    }
}

fn block_tree_contains_code_block(block: &crate::builder::BlockNode) -> bool {
    matches!(block.kind, crate::builder::BlockKind::CodeBlock { .. })
        || block.children.iter().any(block_tree_contains_code_block)
}

fn block_contains_source_byte(block: &crate::builder::BlockNode, source_byte: usize) -> bool {
    block.block_range.start <= source_byte && source_byte <= block.block_range.end
}

fn ranges_intersect_block(source_range: &Range<usize>, block: &crate::builder::BlockNode) -> bool {
    source_range.start < block.block_range.end && block.block_range.start < source_range.end
}

fn signed_offset(current: usize, previous: usize) -> Option<isize> {
    if current >= previous {
        isize::try_from(current - previous).ok()
    } else {
        isize::try_from(previous - current).ok()?.checked_neg()
    }
}

fn shift_source_byte(source_byte: usize, source_byte_delta: isize) -> usize {
    source_byte.checked_add_signed(source_byte_delta).expect(
        "an unchanged block's source anchors must remain valid after block-start translation",
    )
}

fn shift_source_range(source_range: &mut Range<usize>, source_byte_delta: isize) {
    source_range.start = shift_source_byte(source_range.start, source_byte_delta);
    source_range.end = shift_source_byte(source_range.end, source_byte_delta);
}

fn shift_projection_source(projection: &mut VisualLineProjection, source_byte_delta: isize) {
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

fn shift_laid_out_line(line: &mut LaidOutLine, source_byte_delta: isize, y_delta: f32) {
    line.rect.y += y_delta;
    for style in &mut line.styles {
        shift_source_range(&mut style.source_range, source_byte_delta);
    }
    if let Some(projection) = &mut line.source_projection {
        shift_projection_source(projection, source_byte_delta);
    }
}

fn shift_laid_out_lines(lines: &mut [LaidOutLine], source_byte_delta: isize, y_delta: f32) {
    for line in lines {
        shift_laid_out_line(line, source_byte_delta, y_delta);
    }
}

fn shift_laid_out_block(block: &mut LaidOutBlock, source_byte_delta: isize, y_delta: f32) {
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
        doc_view: &dyn core::document::DocView,
    ) -> Self {
        let laid_out_est =
            layout_doc_with_shaper(source.blocks(), style, viewport_w, None, None, doc_view);
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
            source_line_height: 0.0,
            hidden_separator_height: 0.0,
            projected_empty_lines: Vec::new(),
            hidden_block_separators: Vec::new(),
            projection_flat_line_indices: Vec::new(),
            projected_empty_line_geometry: std::collections::BTreeMap::new(),
            edit_ctx: None,
            selection_range: None,
            ascii_diagrams: super::ascii_diagram::AsciiDiagramRegistry::default(),
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

    /// Reserve visual height for extra blank source lines between Markdown blocks.
    ///
    /// The parser drops blank lines from the block tree. In WYSIWYG editing mode,
    /// those source lines still need spatial presence so paragraph insertion is
    /// visible immediately after Enter.
    ///
    /// 2026-07-06：由 [`SourceLineMap`] 提供 run 位置，
    /// 保留旧签名（供 `PreviewEngine::rebuild_layout` 直接调用）。
    pub fn reserve_extra_blank_source_lines(&mut self, line_height: f32, paragraph_spacing: f32) {
        self.source_line_height = line_height;
        self.hidden_separator_height = paragraph_spacing;
        let Some(source_text) = self.source_text.as_deref() else {
            return;
        };
        if self.laid_to_doc.is_empty() {
            return;
        }
        let map = super::source_line_map::SourceLineMap::from_source(source_text);
        let blocks = self.source.blocks();

        // ── Leading blank lines ──
        if let Some(first_block) = self.laid_to_doc.first().and_then(|doc_idx| blocks.get(*doc_idx))
        {
            let first_start = first_block.block_range.start.min(source_text.len());
            let leading_height = map.extra_height_before_block(first_start, true, line_height);
            if leading_height > 0.0 {
                for y_delta in &mut self.y_delta {
                    *y_delta += leading_height;
                }
                self.total_height += leading_height;
            }
        }

        if self.laid_to_doc.len() < 2 {
            return;
        }

        // ── Inter-block blank lines ──
        // 追加量 = (N-1)*line_height：空行 run 首行的间距由真实块间 gap 提供
        // （见 SourceLineMap::attach_layout），此处不再按块类型特判。
        // 嵌套块（列表项/引用块子块）间的空行补偿在 layout_block 内完成，
        // 与此处共用同一公式。
        let mut gap_deltas = Vec::new();
        for laid_idx in 1..self.laid_to_doc.len() {
            let current_doc_idx = self.laid_to_doc[laid_idx];
            let previous_doc_idx = self.laid_to_doc[laid_idx - 1];
            if previous_doc_idx == current_doc_idx {
                continue;
            }
            let Some(current_block) = blocks.get(current_doc_idx) else {
                continue;
            };
            let current_start = current_block.block_range.start.min(source_text.len());
            let extra_height = map.extra_height_before_block(current_start, false, line_height);
            if extra_height > 0.0 {
                gap_deltas.push((laid_idx - 1, extra_height));
                self.total_height += extra_height;
            }
        }

        apply_deltas(&mut self.y_delta, &gap_deltas);
    }

    /// Set the WYSIWYG edit context (cursor position for span expansion).
    pub fn set_edit_ctx(&mut self, edit_ctx: Option<crate::edit::EditContext>) {
        self.edit_ctx = edit_ctx;
    }

    pub fn set_selection_range(&mut self, selection_range: Option<std::ops::Range<usize>>) {
        self.selection_range = selection_range;
        self.ascii_diagrams.set_selection_range(self.selection_range.clone());
    }

    pub(crate) fn ascii_diagrams(&self) -> &super::ascii_diagram::AsciiDiagramRegistry {
        &self.ascii_diagrams
    }

    fn discard_ascii_diagrams_for_laid_index(&mut self, laid_idx: usize) {
        let Some(&doc_idx) = self.laid_to_doc.get(laid_idx) else {
            return;
        };
        let Some(block) = self.source.blocks().get(doc_idx) else {
            return;
        };
        self.ascii_diagrams.remove_source_range(&block.block_range);
    }

    /// Invalidate laid-out blocks whose source text contains any of the given bytes.
    /// Used after cursor movement to only relayout affected blocks (span expansion).
    /// 含旧/新光标字节的块必须无条件失效——即使它在渲染窗口外，否则其
    /// active marker 会在光标离开后永久残留（失效只清标记，不会立即重排屏外块）。
    pub fn invalidate_lines_for_source_bytes(&mut self, bytes: impl IntoIterator<Item = usize>) {
        // Phase 1: collect the unique block line bases for each byte position.
        let mut bases: Vec<usize> = Vec::new();
        for byte in bytes {
            if let Some((block_line_base, _line_idx)) = self.find_block_line_at_byte(byte) {
                bases.push(block_line_base);
            }
        }
        if bases.is_empty() {
            return;
        }
        bases.sort();
        bases.dedup();

        // Phase 2: identify laid_out entries whose doc block matches a base.
        let blocks = self.source.blocks();
        let mut invalidated_indices = Vec::new();
        for (laid_idx, &doc_idx) in self.laid_to_doc.iter().enumerate() {
            if doc_idx >= blocks.len() {
                continue;
            }
            let before = Self::count_block_lines_before(blocks, &blocks[doc_idx]);
            if bases.contains(&before) {
                invalidated_indices.push(laid_idx);
            }
        }

        // Phase 3: invalidate each selected entry after releasing the source borrow.
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
        doc_view: &dyn core::document::DocView,
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
    pub fn ensure_visible(
        &mut self,
        scroll_y: f32,
        viewport_h: f32,
        style: &MarkdownStyle,
        viewport_w: f32,
        shaper: &mut shaping::Shaper,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn core::document::DocView,
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
            let mut ctx = super::context::LayoutCtx::new(
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
            ctx.block_count = i;

            // Restore spacing context from previous block (same logic as layout_block).
            if i > 0
                && let Some(prev_laid) = &self.laid_out[i - 1]
            {
                let prev_doc_kind = self
                    .laid_to_doc
                    .get(i - 1)
                    .and_then(|&prev_doc_idx| self.source.blocks().get(prev_doc_idx))
                    .map(|block| &block.kind);
                ctx.restore_spacing_context(super::context::spacing_kind_of_laid_block(
                    &prev_laid.kind,
                    prev_doc_kind,
                ));
            }

            // Adjust for heading margin collapsing (same as layout_block).
            ctx.presubtract_entry_spacing(&src_block.kind);

            layout_block(src_block, &mut ctx);
            self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
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
    fn evict_outside(&mut self, keep: &std::ops::Range<usize>) {
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
        doc: &dyn core::document::DocView,
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
            let mut ctx = super::context::LayoutCtx::new(
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
            ctx.block_count = i;

            // Restore spacing context (same as layout_block).
            if i > 0
                && let Some(prev_laid) = &self.laid_out[i - 1]
            {
                let prev_doc_kind = self
                    .laid_to_doc
                    .get(i - 1)
                    .and_then(|&prev_doc_idx| self.source.blocks().get(prev_doc_idx))
                    .map(|block| &block.kind);
                ctx.restore_spacing_context(super::context::spacing_kind_of_laid_block(
                    &prev_laid.kind,
                    prev_doc_kind,
                ));
            }

            ctx.presubtract_entry_spacing(&src_block.kind);

            let base_y = self.estimated_positions[i];
            let old_height = self.estimated_heights[i];
            let old_bottom = base_y + old_height;

            layout_block(src_block, &mut ctx);
            self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
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
    fn ensure_consistent(&mut self) {
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
        doc: &dyn core::document::DocView,
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
        doc: &dyn core::document::DocView,
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
    /// 单块重排的静默丢弃而存在;间距上下文按 layout_block 主逻辑恢复。
    fn relayout_multi_output_group(
        &mut self,
        laid_idx: usize,
        style: &MarkdownStyle,
        mut shaper: Option<&mut shaping::Shaper>,
        highlighter: Option<&dyn crate::builder::CodeHighlighter>,
        doc: &dyn core::document::DocView,
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
        let mut ctx = super::context::LayoutCtx::new(
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
        ctx.block_count = group_start;
        // 恢复上一块的间距上下文(与 layout_block 主逻辑一致)。
        if group_start > 0
            && let Some(prev_laid) = &self.laid_out[group_start - 1]
        {
            let prev_doc_kind = self
                .laid_to_doc
                .get(group_start - 1)
                .and_then(|&prev_doc_idx| self.source.blocks().get(prev_doc_idx))
                .map(|block| &block.kind);
            ctx.restore_spacing_context(super::context::spacing_kind_of_laid_block(
                &prev_laid.kind,
                prev_doc_kind,
            ));
        }
        ctx.presubtract_entry_spacing(&src_block.kind);
        layout_block(src_block, &mut ctx);
        self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
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
            let old_bottom = self.estimated_positions[slot] + self.estimated_heights[slot];
            let new_bottom = new_block.rect.y + new_block.rect.h;
            let delta = new_bottom - old_bottom;
            self.estimated_heights[slot] = new_bottom - self.estimated_positions[slot];
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
        doc: &dyn core::document::DocView,
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
        // Use estimated_heights (slot height = spacing + content) rather than
        // rect.h (content-only). layout_block may bake inter-block spacing into
        // rect.y (e.g. list_group_spacing for list→non-list transitions, heading
        // top spacing). Using rect.h omits that spacing, producing a spurious
        // delta on every refresh_precise_range call that accumulates downward.
        let old_height = self.estimated_heights[idx];
        let base_y = self.estimated_positions[idx];
        self.discard_ascii_diagrams_for_laid_index(idx);
        let src_block = &self.source.blocks()[doc_idx];
        let estimated_w = self.laid_out[idx]
            .as_ref()
            .map(|b| b.rect.w + b.rect.x)
            .unwrap_or(self.cached_viewport_w);
        let estimated_indent = self.laid_out[idx].as_ref().map(|b| b.rect.x).unwrap_or(0.0);
        let mut ctx = super::context::LayoutCtx::new(
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
        ctx.block_count = 1;
        // Restore context for spacing decisions (same logic as layout_block).
        if idx > 0
            && let Some(prev_laid) = &self.laid_out[idx - 1]
        {
            let prev_doc_kind = self
                .laid_to_doc
                .get(idx - 1)
                .and_then(|&prev_doc_idx| self.source.blocks().get(prev_doc_idx))
                .map(|block| &block.kind);
            ctx.restore_spacing_context(super::context::spacing_kind_of_laid_block(
                &prev_laid.kind,
                prev_doc_kind,
            ));
        }
        ctx.presubtract_entry_spacing(&src_block.kind);
        layout_block(src_block, &mut ctx);
        self.ascii_diagrams.extend(std::mem::take(&mut ctx.ascii_diagrams));
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

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::MarkdownDoc;
    use crate::parser::parse_markdown;
    use crate::projection::CursorAffinity;
    use crate::test_utils::default_style;

    fn make_doc(md: &str) -> (&str, MarkdownDoc) {
        let parsed = parse_markdown(md);
        (md, MarkdownDoc::build(&parsed, &default_style()))
    }

    fn build_editing_layout(
        source: &str,
        cursor_byte: usize,
        source_generation: u32,
    ) -> LazyLayout<MarkdownDoc> {
        let style = default_style();
        let (_, document) = make_doc(source);
        let document_view = core::document::StringDocView::new(source);
        let mut layout = LazyLayout::new(document, &style, 400.0, &document_view);
        layout.set_source_generation(source_generation);
        layout.set_edit_source(Some(source.to_owned()));
        layout.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte,
            preedit_text: None,
            preedit_cursor: None,
        }));
        layout.reserve_extra_blank_source_lines(style.line_height, style.paragraph_spacing);
        layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
        layout.build_flat_lines(&document_view);
        layout
    }

    fn assert_flat_layout_equivalent(
        actual: &LazyLayout<MarkdownDoc>,
        expected: &LazyLayout<MarkdownDoc>,
        source: &str,
    ) {
        assert_eq!(actual.total_height, expected.total_height);
        assert_eq!(actual.flat_lines.len(), expected.flat_lines.len());
        for (actual_line, expected_line) in actual.flat_lines.iter().zip(&expected.flat_lines) {
            assert_eq!(actual_line.flat_idx, expected_line.flat_idx);
            assert_eq!(actual_line.rect, expected_line.rect);
            assert_eq!(actual_line.text, expected_line.text);
            assert_eq!(actual_line.font_size, expected_line.font_size);
            assert_eq!(actual_line.source_projection, expected_line.source_projection);
        }

        let actual_projection_index = actual
            .source_projection_index
            .as_ref()
            .expect("the reconciled editing layout publishes a projection index");
        let expected_projection_index = expected
            .source_projection_index
            .as_ref()
            .expect("the full editing layout publishes a projection index");
        assert_eq!(
            actual_projection_index.visual_lines(),
            expected_projection_index.visual_lines()
        );
        for source_byte in (0..=source.len()).filter(|byte| source.is_char_boundary(*byte)) {
            for affinity in [CursorAffinity::Upstream, CursorAffinity::Downstream] {
                let actual_position = actual_projection_index
                    .visual_position_for_source(source_byte, affinity)
                    .map(|position| (position.flat_line_idx, position.grapheme_pos));
                let expected_position = expected_projection_index
                    .visual_position_for_source(source_byte, affinity)
                    .map(|position| (position.flat_line_idx, position.grapheme_pos));
                assert_eq!(actual_position, expected_position, "source byte {source_byte}");
            }
        }
    }

    fn reconcile_editing_layout(
        previous: LazyLayout<MarkdownDoc>,
        source: &str,
        cursor_byte: usize,
        source_generation: u32,
    ) -> (LazyLayout<MarkdownDoc>, usize) {
        let style = default_style();
        let (_, document) = make_doc(source);
        let document_view = core::document::StringDocView::new(source);
        let mut layout = LazyLayout::new(document, &style, 400.0, &document_view);
        layout.set_source_generation(source_generation);
        layout.set_edit_source(Some(source.to_owned()));
        layout.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte,
            preedit_text: None,
            preedit_cursor: None,
        }));
        layout.reserve_extra_blank_source_lines(style.line_height, style.paragraph_spacing);
        let reused_block_count = layout.reuse_unchanged_blocks_from(previous);
        layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
        layout.build_flat_lines(&document_view);
        (layout, reused_block_count)
    }

    #[test]
    fn reconciled_layout_reuses_prefix_and_shifted_suffix_equivalently() {
        let old_source = "# Title\n\nalpha\n\nomega 👩‍💻";
        let new_source = "# Title\n\nalpha changed\n\nomega 👩‍💻";
        let previous = build_editing_layout(
            old_source,
            old_source.find("alpha").expect("the old fixture contains the edited paragraph"),
            1,
        );
        let changed_cursor =
            new_source.find("changed").expect("the new fixture contains the inserted text");
        let expected = build_editing_layout(new_source, changed_cursor, 2);
        let style = default_style();
        let (_, new_document) = make_doc(new_source);
        let document_view = core::document::StringDocView::new(new_source);
        let mut reconciled = LazyLayout::new(new_document, &style, 400.0, &document_view);
        reconciled.set_source_generation(2);
        reconciled.set_edit_source(Some(new_source.to_owned()));
        reconciled.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte: changed_cursor,
            preedit_text: None,
            preedit_cursor: None,
        }));
        reconciled.reserve_extra_blank_source_lines(style.line_height, style.paragraph_spacing);

        let reused_block_count = reconciled.reuse_unchanged_blocks_from(previous);

        assert_eq!(reused_block_count, 2);
        assert!(reconciled.laid_out[0].is_some());
        assert!(reconciled.laid_out[1].is_none());
        assert!(reconciled.laid_out[2].is_some());
        assert_eq!(
            reconciled
                .reconciled_flat_lines
                .iter()
                .filter_map(Option::as_ref)
                .map(Vec::len)
                .sum::<usize>(),
            2
        );

        reconciled.ensure_all_blocks(&style, 400.0, None, None, &document_view);
        reconciled.build_flat_lines(&document_view);

        assert_flat_layout_equivalent(&reconciled, &expected, new_source);
    }

    #[test]
    fn reconciled_edit_sequence_matches_full_layout_and_projection_index() {
        let initial_source = "# Title\n\nalpha 👩‍💻\n\n- item\n\n```rust\ncode\n```\n\n尾声";
        let initial_cursor = initial_source
            .find("alpha")
            .expect("the initial fixture contains the edited paragraph");
        let mut reconciled = build_editing_layout(initial_source, initial_cursor, 1);
        let edit_sequence = [
            ("# Title\n\nalpha! 👩‍💻\n\n- item\n\n```rust\ncode\n```\n\n尾声", "alpha!"),
            (
                "# Title\n\nalpha! 👩‍💻\n\ninserted\n\n- item\n\n```rust\ncode\n```\n\n尾声",
                "inserted",
            ),
            ("# Title\n\nalpha! 👩‍💻\n\ninserted\n\n```rust\ncode\n```\n\n尾声", "inserted"),
            (
                "# Title\n\nalpha! 👩‍💻\n\ninserted\n\n- first\n- second\n\n```rust\ncode\n```\n\n尾声",
                "second",
            ),
            (
                "# Title\n\nalpha! 👩‍💻\n\ninserted\n\n- first\n- second\n\n```rust\ncode edited\n```\n\n尾声",
                "code edited",
            ),
            (
                "# Title\n\nalpha! 👩‍💻\n\n- first\n- second\n\n```rust\ncode edited\n```\n\n尾声",
                "alpha!",
            ),
        ];

        for (edit_index, (source, cursor_needle)) in edit_sequence.iter().enumerate() {
            let cursor_byte =
                source.find(cursor_needle).expect("every edit fixture contains its cursor needle");
            let source_generation = u32::try_from(edit_index + 2)
                .expect("the bounded edit sequence generation fits in u32");
            let expected = build_editing_layout(source, cursor_byte, source_generation);
            let (next_reconciled, reused_block_count) =
                reconcile_editing_layout(reconciled, source, cursor_byte, source_generation);

            assert!(
                reused_block_count > 0,
                "edit {edit_index} should preserve at least one surrounding block"
            );
            assert_flat_layout_equivalent(&next_reconciled, &expected, source);
            reconciled = next_reconciled;
        }
    }

    #[test]
    fn apply_deltas_empty_height_deltas_noop() {
        let mut yd = vec![0.0f32; 5];
        let orig = yd.clone();
        apply_deltas(&mut yd, &[]);
        assert_eq!(yd, orig);
    }

    #[test]
    fn apply_deltas_single_block_shifts_subsequent() {
        let mut yd = vec![0.0f32; 5];
        // block 2 grew by 10px -> shifts blocks 3, 4
        apply_deltas(&mut yd, &[(2, 10.0)]);
        assert_eq!(yd[0], 0.0); // block 0 unaffected
        assert_eq!(yd[1], 0.0); // block 1 unaffected
        assert_eq!(yd[2], 0.0); // block 2 unaffected (own delta doesn't shift self)
        assert_eq!(yd[3], 10.0);
        assert_eq!(yd[4], 10.0);
    }

    #[test]
    fn apply_deltas_multiple_blocks_accumulate() {
        let mut yd = vec![0.0f32; 5];
        apply_deltas(&mut yd, &[(1, 5.0), (3, 7.0)]);
        assert_eq!(yd[0], 0.0);
        assert_eq!(yd[1], 0.0);
        assert_eq!(yd[2], 5.0);
        assert_eq!(yd[3], 5.0); // block 3 unaffected by own delta
        assert_eq!(yd[4], 12.0); // 5.0 + 7.0
    }

    #[test]
    fn apply_deltas_negative_delta() {
        let mut yd = vec![0.0f32; 5];
        apply_deltas(&mut yd, &[(2, -3.0)]);
        assert_eq!(yd[0], 0.0);
        assert_eq!(yd[1], 0.0);
        assert_eq!(yd[2], 0.0);
        assert_eq!(yd[3], -3.0);
        assert_eq!(yd[4], -3.0);
    }

    #[test]
    fn apply_deltas_first_block_shifts_all_others() {
        let mut yd = vec![0.0f32; 3];
        apply_deltas(&mut yd, &[(0, 8.0)]);
        assert_eq!(yd[0], 0.0); // block 0 unaffected
        assert_eq!(yd[1], 8.0);
        assert_eq!(yd[2], 8.0);
    }

    #[test]
    fn apply_deltas_single_element() {
        let mut yd = vec![0.0f32; 1];
        apply_deltas(&mut yd, &[(0, 5.0)]);
        // Only block 0, no subsequent blocks to shift
        assert_eq!(yd[0], 0.0);
    }

    #[test]
    fn apply_deltas_very_small_delta() {
        let mut yd = vec![0.0f32; 3];
        apply_deltas(&mut yd, &[(0, 0.001)]);
        assert_eq!(yd[0], 0.0);
        assert!((yd[1] - 0.001).abs() < 1e-10);
        assert!((yd[2] - 0.001).abs() < 1e-10);
    }

    #[test]
    fn lazy_layout_estimation_has_all_blocks() {
        let (src, doc) = make_doc("# Title\n\nparagraph\n\n## Section\n\n- item\n\n```\ncode\n```");
        let style = default_style();
        let lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(!lazy.laid_out.is_empty());
        assert_eq!(lazy.precise.len(), lazy.laid_out.len());
        assert_eq!(lazy.y_delta.len(), lazy.laid_out.len());
        assert!(lazy.precise.iter().all(|p| !*p), "all blocks start as estimated");
        assert!(lazy.y_delta.iter().all(|&d| d == 0.0), "y_delta starts at zero");
        assert!(lazy.total_height > 0.0);
    }

    #[test]
    fn trailing_empty_source_lines_are_included_in_total_height() {
        let source = "paragraph\n\n\n";
        let (source, document) = make_doc(source);
        let style = default_style();
        let document_view = core::document::StringDocView::new(source);
        let mut layout = LazyLayout::new(document, &style, 400.0, &document_view);
        layout.set_edit_source(Some(source.to_string()));
        layout.reserve_extra_blank_source_lines(style.line_height, style.paragraph_spacing);
        layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
        layout.build_flat_lines(&document_view);

        let trailing_empty_bottom = layout
            .projected_empty_lines
            .iter()
            .map(|line| line.y_top + line.height)
            .max_by(f32::total_cmp)
            .expect("fixture must project trailing empty source lines");

        assert!(
            layout.total_height >= trailing_empty_bottom,
            "total height {} must include trailing empty-line bottom {}",
            layout.total_height,
            trailing_empty_bottom,
        );
    }

    #[test]
    fn heading_blank_run_projects_editable_line_inside_real_gap() {
        // `# T` 后跟 2 个空行：可编辑空行必须落在标题底部与段落顶边之间，
        // 不得按 paragraph_spacing 假设而与段落文字重叠（heading_spacing_bottom 更小）。
        let source = "# T\n\n\npara";
        let style = default_style();
        let layout = build_editing_layout(source, source.len(), 1);

        let heading_line = layout
            .flat_lines
            .iter()
            .find(|line| line.text.trim_end() == "T")
            .expect("fixture must render the heading line");
        let para_line = layout
            .flat_lines
            .iter()
            .find(|line| line.text == "para")
            .expect("fixture must render the paragraph line");
        let heading_bottom = heading_line.rect.y + heading_line.rect.h;

        assert_eq!(
            layout.projected_empty_lines.len(),
            1,
            "run of two blank lines must project exactly one editable empty line"
        );
        let empty = layout.projected_empty_lines[0];
        assert!(
            empty.y_top >= heading_bottom - 0.01,
            "editable empty line y {} must not overlap the heading (bottom {heading_bottom})",
            empty.y_top
        );
        assert!(
            empty.y_top + empty.height <= para_line.rect.y + 0.01,
            "editable empty line bottom {} must not overlap the paragraph (top {})",
            empty.y_top + empty.height,
            para_line.rect.y
        );
        // 间距本身由真实 gap 提供：空行 run 额外占用恰好一个行高。
        let gap = para_line.rect.y - heading_bottom;
        let expected_gap = style.heading_spacing_bottom + style.line_height;
        assert!(
            (gap - expected_gap).abs() < 0.01,
            "real gap {gap} must be heading_spacing_bottom + one line height ({expected_gap})"
        );
    }

    #[test]
    fn blank_run_between_paragraphs_reserves_only_editable_line_heights() {
        // para→para 的 2 个空行：追加量 = (N-1)*line_height，间距只算一份真实
        // paragraph_spacing，不再因块类型特判而多加一份。
        let source = "a\n\n\nb";
        let style = default_style();
        let layout = build_editing_layout(source, source.len(), 1);

        let a_line = layout.flat_lines.iter().find(|line| line.text == "a").expect("fixture has a");
        let b_line = layout.flat_lines.iter().find(|line| line.text == "b").expect("fixture has b");
        let gap = b_line.rect.y - (a_line.rect.y + a_line.rect.h);
        let expected = style.paragraph_spacing + style.line_height;
        assert!(
            (gap - expected).abs() < 0.01,
            "inter-block gap {gap} must equal paragraph_spacing + one line height ({expected})"
        );

        let empty = layout
            .projected_empty_lines
            .first()
            .expect("fixture must project one editable empty line");
        assert!(
            (empty.y_top - (a_line.rect.y + a_line.rect.h + style.paragraph_spacing)).abs() < 0.01,
            "editable empty line must sit right below the paragraph spacing, got y {}",
            empty.y_top
        );
    }

    #[test]
    fn loose_list_item_reserves_blank_line_height_between_child_paragraphs() {
        // `- a` 与项内子段落 `b` 之间有 2 个源空行：项内补偿 (N-1)*line_height，
        // 子段落间距 = paragraph_spacing + line_height。
        let source = "- a\n\n\n  b";
        let style = default_style();
        let layout = build_editing_layout(source, source.len(), 1);

        let item_blocks = layout
            .laid_out
            .iter()
            .flatten()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::ListItem { blocks, .. } if blocks.len() == 2 => Some(blocks),
                _ => None,
            })
            .expect("fixture must produce a list item with two child paragraphs");
        let first_bottom = item_blocks[0].rect.y + item_blocks[0].rect.h;
        let second_top = item_blocks[1].rect.y;
        let expected = style.paragraph_spacing + style.line_height;
        assert!(
            (second_top - first_bottom - expected).abs() < 0.01,
            "child paragraph gap {} must equal paragraph_spacing + one line height ({expected})",
            second_top - first_bottom
        );
    }

    #[test]
    fn editable_empty_line_inside_loose_list_stays_between_child_paragraphs() {
        // 项内空行 run 的可编辑空行不得覆盖子段落文字。
        let source = "- a\n\n\n  b";
        let layout = build_editing_layout(source, source.len(), 1);

        let a_line = layout.flat_lines.iter().find(|line| line.text == "a").expect("fixture has a");
        let b_line = layout.flat_lines.iter().find(|line| line.text == "b").expect("fixture has b");
        let a_bottom = a_line.rect.y + a_line.rect.h;

        let empty = layout
            .projected_empty_lines
            .first()
            .expect("fixture must project one editable empty line inside the list item");
        assert!(
            empty.y_top >= a_bottom - 0.01,
            "editable empty line y {} must not overlap item text (bottom {a_bottom})",
            empty.y_top
        );
        assert!(
            empty.y_top + empty.height <= b_line.rect.y + 0.01,
            "editable empty line bottom {} must not overlap child paragraph (top {})",
            empty.y_top + empty.height,
            b_line.rect.y
        );
    }

    #[test]
    fn viewport_eviction_discards_ascii_diagram_sidecars_without_mismatching_same_first_row() {
        let filler = "filler line\n".repeat(100);
        let source =
            format!("```\n┌───┐\n│甲  │\n└───┘\n```\n\n{filler}\n```\n┌───┐\n│乙  │\n└───┘\n```");
        let style = default_style();
        let parsed = parse_markdown(&source);
        let document = MarkdownDoc::build(&parsed, &style);
        let source_view = core::document::StringDocView::new(&source);
        let mut layout = LazyLayout::new(document, &style, 400.0, &source_view);
        let mut shaper = shaping::Shaper::new().expect("sidecar eviction test needs a shaper");

        layout.ensure_visible(0.0, 80.0, &style, 400.0, &mut shaper, None, &source_view);
        layout.ensure_visible(
            (layout.total_height - 80.0).max(0.0),
            80.0,
            &style,
            400.0,
            &mut shaper,
            None,
            &source_view,
        );

        let code_lines = layout
            .laid_out
            .iter()
            .flatten()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::CodeBlock { lines, .. }
                    if lines.iter().any(|line| line.text.contains('乙')) =>
                {
                    Some(lines)
                }
                _ => None,
            })
            .expect("second diagram must be materialized after scrolling into its viewport");
        let diagram = layout
            .ascii_diagrams()
            .diagram_for(code_lines)
            .expect("second diagram must retain its own sidecar");
        let second_row_text: String =
            diagram.rows[1].cells.iter().map(|cell| cell.text.as_str()).collect();

        assert_eq!(second_row_text, "│乙  │");
        assert_eq!(
            layout.ascii_diagrams().entry_count(),
            1,
            "evicted diagrams must not leave stale registry entries behind"
        );
    }

    #[test]
    fn invalidation_discards_ascii_diagram_sidecars_before_relayout() {
        let source = "```\n┌───┐\n│甲  │\n└───┘\n```\n\nparagraph";
        let style = default_style();
        let parsed = parse_markdown(source);
        let document = MarkdownDoc::build(&parsed, &style);
        let source_view = core::document::StringDocView::new(source);
        let mut layout = LazyLayout::new(document, &style, 400.0, &source_view);
        let mut shaper = shaping::Shaper::new().expect("sidecar invalidation test needs a shaper");
        layout.ensure_visible(0.0, 600.0, &style, 400.0, &mut shaper, None, &source_view);

        assert_eq!(layout.ascii_diagrams().entry_count(), 1);

        layout.invalidate_lines_for_source_bytes([source.find('甲').expect("fixture has label")]);

        assert_eq!(
            layout.ascii_diagrams().entry_count(),
            0,
            "invalidated diagrams must release their sidecar before relayout"
        );
    }

    #[test]
    fn precision_pass_marks_block_precise() {
        let (src, doc) = make_doc("# Title\n\nparagraph text here\n\n## Another heading");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));

        let mut shaper = shaping::Shaper::new().unwrap();
        let _deltas = lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );

        // First block (and possibly more) should be precise now
        assert!(lazy.precise[0], "first block should be precise");
        let any_precise = lazy.precise.iter().any(|p| *p);
        assert!(any_precise, "at least one block should be precise");
    }

    #[test]
    fn precision_pass_respects_scroll_offset() {
        let (src, doc) = make_doc("# Block 1\n\n## Block 2\n\n### Block 3\n\nparagraph");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(lazy.laid_out.len() >= 3);

        let second_block_y = lazy.laid_out[1].as_ref().unwrap().rect.y;
        let mut shaper = shaping::Shaper::new().unwrap();
        // Scroll so only block 1+ (index 1 and beyond) are in range
        let _deltas = lazy.ensure_precise_range(
            second_block_y + 10.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );

        // Block 0 is above range_start, should NOT be precise
        // (unless viewport is so large that buffer_above reaches it)
        let any_precise = lazy.precise.iter().any(|p| *p);
        assert!(any_precise, "at least one block should be precise");
    }

    #[test]
    fn lazy_layout_e2e_renders_all_text() {
        use crate::render::render_doc_with_offset;
        let md = "# Title\n\nparagraph with **bold**\n\n- list item\n\n```\ncode\n```";
        let style = default_style();
        let (src, doc) = make_doc(md);
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        let mut shaper = shaping::Shaper::new().unwrap();

        // Precision-pass visible area
        lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );

        // Render
        let mut dl = ui::core::paint::DrawList::new();
        let (visible, visible_yd) = lazy.materialized_blocks();
        render_doc_with_offset(
            &visible,
            &style,
            &mut dl,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &visible_yd,
        );

        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::paint::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        let all_text = texts.concat();
        assert!(all_text.contains("Title"));
        assert!(all_text.contains("paragraph"));
        assert!(all_text.contains("bold"));
        assert!(all_text.contains("list item"));
        assert!(all_text.contains("code"));
    }

    #[test]
    fn lazy_layout_scroll_culling_still_works() {
        use crate::render::render_doc_with_offset;
        let md = "# Top\n\n## Middle\n\n### Bottom";
        let style = default_style();
        let (src, doc) = make_doc(md);
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        let mut shaper = shaping::Shaper::new().unwrap();

        // Only make "Middle" and "Bottom" precise
        let mid_y = lazy.laid_out[1].as_ref().unwrap().rect.y;
        lazy.ensure_precise_range(
            mid_y,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );

        // Scroll past Top
        let scroll_y = lazy.laid_out[0].as_ref().unwrap().rect.y
            + lazy.laid_out[0].as_ref().unwrap().rect.h
            + 10.0;
        let mut dl = ui::core::paint::DrawList::new();
        let (visible, visible_yd) = lazy.materialized_blocks();
        render_doc_with_offset(
            &visible,
            &style,
            &mut dl,
            scroll_y,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &visible_yd,
        );

        let texts: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let ui::core::paint::DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.clone())
                } else {
                    None
                }
            })
            .collect();
        let all_text = texts.concat();
        assert!(!all_text.contains("Top"), "Top should be culled by scroll");
        assert!(
            all_text.contains("Middle") || all_text.contains("Bottom"),
            "visible blocks should render"
        );
    }

    #[test]
    fn lazy_layout_y_delta_propagates_correctly() {
        let md = "# A\n\n## B\n\n### C";
        let style = default_style();
        let (src, doc) = make_doc(md);
        let lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));

        // Before precision: all y_delta = 0
        assert_eq!(lazy.y_delta[0], 0.0);
        assert_eq!(lazy.y_delta[1], 0.0);
        assert_eq!(lazy.y_delta[2], 0.0);

        // Real Y equals rect.y
        let b0_y = lazy.laid_out[0].as_ref().unwrap().rect.y;
        assert!((lazy.laid_out[0].as_ref().unwrap().rect.y + lazy.y_delta[0] - b0_y).abs() < 0.01);
    }

    /// 合成的多输出文档块源：一个顶层块展开为多个 LaidOutBlock
    /// （根 Container，或意外到达顶层的 TableRow_）。
    struct MultiOutputDocSource {
        blocks: Vec<crate::builder::BlockNode>,
    }

    impl BlockSource for MultiOutputDocSource {
        fn blocks(&self) -> &[crate::builder::BlockNode] {
            &self.blocks
        }

        fn headings(&self) -> &[crate::layout::HeadingEntry] {
            &[]
        }
    }

    fn multi_output_source(kind: crate::builder::BlockKind, md: &str) -> MultiOutputDocSource {
        let parsed = parse_markdown(md);
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let wrapper = crate::builder::BlockNode {
            kind,
            children: doc.blocks,
            text_lines: vec![],
            projected_lines: vec![],
            text_styles: vec![],
            source_range: crate::builder::BlockSource::Continuous(0..md.len()),
            block_range: 0..md.len(),
            code_line_source_starts: None,
        };
        MultiOutputDocSource { blocks: vec![wrapper] }
    }

    fn laid_text(layout: &LazyLayout<MultiOutputDocSource>, slot: usize) -> String {
        let block = layout.laid_out[slot].as_ref().expect("slot must be materialized");
        match &block.kind {
            LaidOutBlockKind::Text { lines } => {
                lines.iter().map(|line| line.text.as_str()).collect::<String>()
            }
            other => {
                panic!("slot {slot} must be a text block, got {:?}", std::mem::discriminant(other))
            }
        }
    }

    #[test]
    fn ensure_all_blocks_distributes_multi_output_doc_blocks() {
        // 根 Container 的两个子段落必须分别落入两个槽位；
        // 只取首个输出会把 slot 1 也填成 "alpha"（静默复制/丢弃）。
        let style = default_style();
        let md = "alpha\n\nomega";
        let source = multi_output_source(crate::builder::BlockKind::Container, md);
        let doc_view = core::document::StringDocView::new(md);
        let mut layout = LazyLayout::new(source, &style, 400.0, &doc_view);
        layout.ensure_all_blocks(&style, 400.0, None, None, &doc_view);

        assert_eq!(layout.laid_out.len(), 2);
        assert_eq!(laid_text(&layout, 0), "alpha");
        assert_eq!(laid_text(&layout, 1), "omega");
    }

    #[test]
    fn precise_block_at_distributes_multi_output_doc_blocks() {
        let style = default_style();
        let md = "alpha\n\nomega";
        let source = multi_output_source(crate::builder::BlockKind::Container, md);
        let doc_view = core::document::StringDocView::new(md);
        let mut layout = LazyLayout::new(source, &style, 400.0, &doc_view);
        let mut shaper = shaping::Shaper::new().expect("multi-output test needs a shaper");

        layout.precise_block_at(0, &style, &mut shaper, None, &doc_view);

        assert_eq!(laid_text(&layout, 0), "alpha");
        assert_eq!(laid_text(&layout, 1), "omega");
        assert!(layout.precise.iter().all(|&p| p), "both slots of the group become precise");
    }

    #[test]
    fn top_level_table_row_expands_without_layout_mismatch() {
        // flatten_blocks 必须镜像 layout_block 对顶层 TableRow_ 的子块展开，
        // 否则 LazyLayout::new 的块数不变量被打破。
        let style = default_style();
        let md = "alpha\n\nomega";
        let source = multi_output_source(crate::builder::BlockKind::TableRow_, md);
        let doc_view = core::document::StringDocView::new(md);
        let mut layout = LazyLayout::new(source, &style, 400.0, &doc_view);
        layout.ensure_all_blocks(&style, 400.0, None, None, &doc_view);

        assert_eq!(layout.laid_out.len(), 2);
        assert_eq!(laid_text(&layout, 0), "alpha");
        assert_eq!(laid_text(&layout, 1), "omega");
    }

    #[test]
    fn incremental_relayout_matches_one_shot_block_positions() {
        // 间距上下文恢复 + 入口预扣必须与 layout_block 主逻辑逐项一致:
        // 逐块重排(ensure_all_blocks)的最终落点必须与一次性全文布局相同。
        // 覆盖列表组收尾 bump、tight 缩减、标题 margin collapsing 等入口调整。
        let fixtures = [
            "para\n- a\n- b",
            "- a\n- b\n\npara",
            "para\n\n- a\n\n# H",
            "# H\n\npara\n- x",
            "- a\n\n# H\n\npara",
            "para\n\n---\n\n# H\n\ntail",
            "# A\n# B\n\n> q\n\n```\ncode\n```\n\nend",
        ];
        for md in fixtures {
            let style = default_style();
            let (src, doc) = make_doc(md);
            let doc_view = core::document::StringDocView::new(src);
            let oneshot = crate::layout::block::layout_doc(doc.blocks(), &style, 400.0, &doc_view);

            let (_, lazy_doc) = make_doc(md);
            let mut lazy = LazyLayout::new(lazy_doc, &style, 400.0, &doc_view);
            lazy.ensure_all_blocks(&style, 400.0, None, None, &doc_view);

            assert_eq!(
                lazy.laid_out.len(),
                oneshot.blocks.len(),
                "block count mismatch for {md:?}"
            );
            for (idx, oneshot_block) in oneshot.blocks.iter().enumerate() {
                let laid = lazy.laid_out[idx].as_ref().expect("block must be materialized");
                let final_y = laid.rect.y + lazy.y_delta.get(idx).copied().unwrap_or(0.0);
                assert!(
                    (final_y - oneshot_block.rect.y).abs() < 0.01,
                    "block {idx} y mismatch for {md:?}: incremental {final_y} vs one-shot {}",
                    oneshot_block.rect.y
                );
            }
            assert!(
                (lazy.total_height - oneshot.total_height).abs() < 0.01,
                "total height mismatch for {md:?}: incremental {} vs one-shot {}",
                lazy.total_height,
                oneshot.total_height
            );
        }
    }

    #[test]
    fn precise_block_at_returns_height_delta() {
        let (src, doc) = make_doc("# Title\n\nparagraph text");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        let mut shaper = shaping::Shaper::new().unwrap();

        assert!(!lazy.precise[0]);
        let _delta = lazy.precise_block_at(
            0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );
        assert!(lazy.precise[0], "block should be marked precise");
        // delta may be 0.0 if estimation was exact, but precise_block_at should not panic
        // total_height should have been updated
        assert!(lazy.total_height > 0.0);
        // calling again should still work (idempotent)
        let _delta2 = lazy.precise_block_at(
            0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );
        assert!(lazy.precise[0]);
    }

    #[test]
    fn precise_block_at_out_of_bounds_returns_zero() {
        let (src, doc) = make_doc("hello");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        let mut shaper = shaping::Shaper::new().unwrap();

        let delta = lazy.precise_block_at(
            999,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );
        assert_eq!(delta, 0.0, "out-of-bounds idx should return 0.0");
    }

    #[test]
    fn lazy_layout_empty_document() {
        let (src, doc) = make_doc("");
        let style = default_style();
        let lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(lazy.laid_out.is_empty());
        assert!(lazy.precise.is_empty());
        assert!(lazy.y_delta.is_empty());
        assert_eq!(lazy.total_height, 0.0);
    }

    #[test]
    fn ensure_precise_range_twice_no_double_count() {
        let (src, doc) = make_doc("# A\n\n## B\n\n### C");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        let mut shaper = shaping::Shaper::new().unwrap();

        let _deltas1 = lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );
        let height_after_first = lazy.total_height;
        let y_delta_after_first = lazy.y_delta.clone();

        // Second call should be a no-op (all blocks already precise)
        let deltas2 = lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );
        assert!(deltas2.is_empty(), "second call should produce no deltas");
        assert_eq!(lazy.total_height, height_after_first, "total_height should not change");
        assert_eq!(lazy.y_delta, y_delta_after_first, "y_delta should not change");
    }

    #[test]
    fn first_visible_block_idx_with_y_delta() {
        use crate::render::first_visible_block_idx;
        let md = "# A\n\n## B\n\n### C";
        let style = default_style();
        let (src, doc) = make_doc(md);
        let laid_out = crate::layout::layout_doc(
            &doc.blocks,
            &style,
            400.0,
            &core::document::StringDocView::new(src),
        );
        assert!(laid_out.blocks.len() >= 3);

        // Without y_delta, should find block 0 at scroll_y=0
        assert_eq!(first_visible_block_idx(&laid_out.blocks, &[], 0.0), 0);

        // y_delta[1] = 50 means block 1's real position is shifted down by 50px.
        // This means block 1 extends further down, so scrolling past block 0's
        // original bottom should still find block 1 via backward walk.
        let mut y_delta = vec![0.0f32; laid_out.blocks.len()];
        y_delta[1] = 50.0; // block 1 and later shifted down by 50px

        // scroll_y just past block 0's bottom (without y_delta, block 1 starts here)
        let scroll_y = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 1.0;
        let idx_no_delta = first_visible_block_idx(&laid_out.blocks, &[], scroll_y);
        let idx_with_delta = first_visible_block_idx(&laid_out.blocks, &y_delta, scroll_y);

        // Both should find a valid block
        assert!(idx_no_delta < laid_out.blocks.len());
        assert!(idx_with_delta < laid_out.blocks.len());

        // With y_delta, block 1's real_bottom = rect.y + rect.h + 50,
        // which is higher than without y_delta, so the backward walk
        // may extend visibility further
        assert!(
            idx_with_delta <= idx_no_delta + 1,
            "y_delta should not cause block index to jump too far"
        );
    }

    #[test]
    fn heading_followed_by_code_block_spacing_after_precision() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let md = "### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count\n                          = display_map.tree.total_rows()\n```";
        let parsed = parse_markdown(md);
        let doc = MarkdownDoc::build(&parsed, &style);
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 600.0, &core::document::StringDocView::new(md));

        let deltas = lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(md),
        );

        assert!(
            lazy.laid_out.len() >= 2,
            "expected at least 2 blocks, got {}",
            lazy.laid_out.len()
        );

        let heading = lazy.laid_out[0].as_ref().unwrap();
        let code_block = lazy.laid_out[1].as_ref().unwrap();

        let heading_real_bottom =
            heading.rect.y + lazy.y_delta.first().copied().unwrap_or(0.0) + heading.rect.h;
        let code_real_top = code_block.rect.y + lazy.y_delta.get(1).copied().unwrap_or(0.0);

        let gap = code_real_top - heading_real_bottom;
        assert!(
            gap >= style.heading_spacing_bottom - 1.0,
            "gap {} between heading bottom and code block top after precision should be >= heading_spacing_bottom {}. heading.rect=({},{}), code.rect=({},{}), y_delta={:?}, deltas={:?}",
            gap,
            style.heading_spacing_bottom,
            heading.rect.y,
            heading.rect.h,
            code_block.rect.y,
            code_block.rect.h,
            &lazy.y_delta,
            deltas
        );
    }

    #[test]
    fn large_code_block_then_heading_then_code_block_spacing() {
        // Matches the real document structure: big diagram -> heading -> code block
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        // Simulate a large code block (like the 84-line diagram), then heading, then code block
        let large_block = "```\n".to_string() + &"line\n".repeat(84) + "```";
        let md = format!(
            "{}\n\n### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count\n```",
            large_block
        );
        let parsed = parse_markdown(&md);
        let doc = MarkdownDoc::build(&parsed, &style);
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 600.0, &core::document::StringDocView::new(&md));

        let _deltas = lazy.ensure_precise_range(
            0.0,
            10000.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(&md),
        );

        assert!(
            lazy.laid_out.len() >= 3,
            "expected at least 3 blocks (large code + heading + code), got {}",
            lazy.laid_out.len()
        );

        let heading = lazy.laid_out[1].as_ref().unwrap();
        let code_block = lazy.laid_out[2].as_ref().unwrap();

        let heading_real_bottom =
            heading.rect.y + lazy.y_delta.get(1).copied().unwrap_or(0.0) + heading.rect.h;
        let code_real_top = code_block.rect.y + lazy.y_delta.get(2).copied().unwrap_or(0.0);

        let gap = code_real_top - heading_real_bottom;
        assert!(
            gap >= style.heading_spacing_bottom - 1.0,
            "gap {} between heading bottom and code block top should be >= heading_spacing_bottom {}. heading.rect=({},{}), code.rect.y={}, y_delta[1]={}, y_delta[2]={}",
            gap,
            style.heading_spacing_bottom,
            heading.rect.y,
            heading.rect.h,
            code_block.rect.y,
            lazy.y_delta.get(1).copied().unwrap_or(0.0),
            lazy.y_delta.get(2).copied().unwrap_or(0.0)
        );
    }

    #[test]
    fn precision_block_rect_y_equals_estimation_rect_y() {
        // After precision pass, block.rect.y must stay as the ESTIMATION position,
        // not include y_delta. Otherwise rendering's rect.y + y_delta[i] formula
        // double-counts and creates visual overlaps.
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        // Two paragraphs: first paragraph's height may differ between estimation
        // and precision, which produces a non-zero delta propagated via y_delta.
        let md = "A long paragraph that might wrap differently with HarfBuzz shaping than with the simple byte-based estimation used during the initial layout pass.\n\nSecond paragraph here.";
        let parsed = parse_markdown(md);
        let doc = MarkdownDoc::build(&parsed, &style);
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(md));

        // Snapshot estimation rect.y values
        let est_y0 = lazy.laid_out[0].as_ref().unwrap().rect.y;
        let est_y1 = lazy.laid_out[1].as_ref().unwrap().rect.y;

        // Precision-lay both blocks
        let _deltas = lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(md),
        );

        // rect.y must still equal the estimation position (not include y_delta).
        // If rect.y changed to include y_delta, the rendering formula
        // screen_y = rect.y + y_delta[i] would double-count.
        assert!(
            (lazy.laid_out[0].as_ref().unwrap().rect.y - est_y0).abs() < 0.5,
            "block[0].rect.y changed from {} to {} (delta={}), should stay at estimation position",
            est_y0,
            lazy.laid_out[0].as_ref().unwrap().rect.y,
            lazy.laid_out[0].as_ref().unwrap().rect.y - est_y0,
        );
        assert!(
            (lazy.laid_out[1].as_ref().unwrap().rect.y - est_y1).abs() < 0.5,
            "block[1].rect.y changed from {} to {} (delta={}), should stay at estimation position. y_delta[1]={}",
            est_y1,
            lazy.laid_out[1].as_ref().unwrap().rect.y,
            lazy.laid_out[1].as_ref().unwrap().rect.y - est_y1,
            lazy.y_delta.get(1).copied().unwrap_or(0.0),
        );
    }

    #[test]
    fn no_overlap_heading_code_block_lines() {
        // Check that heading lines don't overlap with code block lines
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let md = "### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count\n                          = display_map.tree.total_rows()\n```";
        let parsed = parse_markdown(md);
        let doc = MarkdownDoc::build(&parsed, &style);
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 600.0, &core::document::StringDocView::new(md));

        let _deltas = lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(md),
        );

        assert!(
            lazy.laid_out.len() >= 2,
            "expected at least 2 blocks, got {}",
            lazy.laid_out.len()
        );

        let heading = lazy.laid_out[0].as_ref().unwrap();
        let code_block = lazy.laid_out[1].as_ref().unwrap();

        // Heading is Text block; get last line's bottom
        if let LaidOutBlockKind::Text { lines } = &heading.kind
            && let Some(last_heading_line) = lines.last()
        {
            let heading_text_bottom = last_heading_line.rect.y + last_heading_line.rect.h;
            let heading_real_bottom =
                heading_text_bottom + lazy.y_delta.first().copied().unwrap_or(0.0);

            // Code block first line
            if let LaidOutBlockKind::CodeBlock { lines: code_lines, .. } = &code_block.kind
                && let Some(first_code_line) = code_lines.first()
            {
                let code_text_top =
                    first_code_line.rect.y + lazy.y_delta.get(1).copied().unwrap_or(0.0);

                let gap = code_text_top - heading_real_bottom;
                // Gap should be heading_spacing_bottom + code_block_padding
                let expected_min = style.heading_spacing_bottom + style.code_block_padding - 2.0;
                assert!(
                    gap >= expected_min,
                    "heading last line bottom to code first line top gap {} should be >= {} (heading_spacing_bottom {} + code_padding {}). heading_line.rect=({},{}), code_line.rect.y={}, y_delta={:?}",
                    gap,
                    expected_min,
                    style.heading_spacing_bottom,
                    style.code_block_padding,
                    last_heading_line.rect.y,
                    last_heading_line.rect.h,
                    first_code_line.rect.y,
                    &lazy.y_delta
                );
            }
        }
    }

    #[test]
    fn flat_lines_update_after_y_delta() {
        let (src, doc) = make_doc("Line 1\nLine 2");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        let initial_y = lazy.flat_lines[0].rect.y;

        // Simulate y_delta change
        if !lazy.y_delta.is_empty() {
            lazy.y_delta[0] = 10.0;
            lazy.build_flat_lines(&core::document::StringDocView::new(src));

            // Y position should have changed
            assert_ne!(lazy.flat_lines[0].rect.y, initial_y);
        }
    }

    // ===== build_flat_lines tests =====

    #[test]
    fn build_flat_lines_text_block() {
        let (src, doc) = make_doc("Hello world\nSecond line");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        assert!(!lazy.flat_lines.is_empty());
        // Text may be wrapped, so just check content contains expected text
        let all_text: String =
            lazy.flat_lines.iter().map(|fl| fl.text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(all_text.contains("Hello world"));
        assert!(all_text.contains("Second line"));
        // Check flat_idx is sequential
        for (i, fl) in lazy.flat_lines.iter().enumerate() {
            assert_eq!(fl.flat_idx, i);
        }
    }

    #[test]
    fn build_flat_lines_blockquote() {
        let (src, doc) = make_doc("> quoted text\n> second line");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        // BlockQuote should be flattened into the lines
        assert!(!lazy.flat_lines.is_empty());
        assert!(lazy.flat_lines.iter().any(|fl| fl.text.contains("quoted text")));
    }

    #[test]
    fn build_flat_lines_table() {
        let (src, doc) = make_doc("| A | B |\n|---|---|\n| 1 | 2 |");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        // Table cells should be flattened
        assert!(!lazy.flat_lines.is_empty());
        // Check that table content is present
        let all_text: String =
            lazy.flat_lines.iter().map(|fl| fl.text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(all_text.contains("A") || all_text.contains("B"));
    }

    #[test]
    fn build_flat_lines_horizontal_rule() {
        let (src, doc) = make_doc("---");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        assert_eq!(lazy.flat_lines.len(), 1);
        assert_eq!(lazy.flat_lines[0].text, "");
        assert_eq!(lazy.flat_lines[0].font_size, 14.0); // HR has non-zero font_size so cursor is visible
        lazy.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte: 0,
            preedit_text: None,
            preedit_cursor: None,
        }));
        lazy.validate_editable_projections()
            .expect("a non-editable horizontal rule must not require a source projection");
    }

    #[test]
    fn build_flat_lines_sorted_by_rect_y() {
        let (src, doc) = make_doc("First\n\nSecond\n\nThird");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        // Verify invariant: flat_lines sorted by rect.y ascending
        for i in 1..lazy.flat_lines.len() {
            assert!(
                lazy.flat_lines[i].rect.y >= lazy.flat_lines[i - 1].rect.y,
                "flat_lines not sorted: line {} y={} < line {} y={}",
                i,
                lazy.flat_lines[i].rect.y,
                i - 1,
                lazy.flat_lines[i - 1].rect.y
            );
        }
    }

    #[test]
    fn build_flat_lines_empty_doc() {
        let (src, doc) = make_doc("");
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        // Empty doc might still have one empty line
        // Just verify it doesn't panic
    }

    /// Regression: flat_line y must match rendering y (within content_height).
    /// Previously, push_flat_line computed y = abs_block_y + line.rect.y,
    /// but line.rect.y was already absolute — causing double-counting.
    #[test]
    fn flat_lines_y_within_content_height() {
        let (src, doc) = make_doc(
            "# Title\n\nFirst paragraph.\n\nSecond paragraph.\n\n> Blockquote text\n\n- List item",
        );
        let style = default_style();
        let lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        let total_h = lazy.total_height;

        for (i, fl) in lazy.flat_lines.iter().enumerate() {
            assert!(
                fl.rect.y < total_h + style.line_height * 2.0, // small tolerance for spacing
                "flat_line {} '{}' y={} exceeds content_height={} — likely double-counted",
                i,
                fl.text,
                fl.rect.y,
                total_h,
            );
        }
    }

    /// Regression: flat_line y for the first line of a text block must
    /// approximately equal block.rect.y (not 2× block.rect.y).
    #[test]
    fn flat_line_y_matches_block_y() {
        let (src, doc) = make_doc("Hello world");
        let style = default_style();
        let lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));

        assert!(!lazy.flat_lines.is_empty());
        assert!(!lazy.laid_out.is_empty());
        assert!(lazy.laid_out[0].is_some());

        let block = lazy.laid_out[0].as_ref().unwrap();
        let first_flat = &lazy.flat_lines[0];
        // first flat_line y should be close to block.rect.y, not 2× block.rect.y
        let expected_y = block.rect.y;
        assert!(
            (first_flat.rect.y - expected_y).abs() < 1.0,
            "flat_line y={} but block y={} — expected approximately equal",
            first_flat.rect.y,
            expected_y,
        );
    }

    /// Novel mode: consecutive paragraphs must have gap = paragraph_spacing (≈0.5× line_height),
    /// NOT ~1.5–2× line_height (regression: each paragraph carried a trailing empty line from
    /// the byte-range newline).
    #[test]
    fn novel_paragraph_spacing_not_doubled() {
        let src = "第一段内容文字。\n\n第二段内容文字。\n";
        let doc_view = core::document::StringDocView::new(src);
        let novel = crate::builder::NovelStructure::scan(&doc_view);
        let style = default_style();
        let expected_gap = style.paragraph_spacing;
        let line_h = style.line_height;

        // Verify we have two paragraph blocks.
        assert_eq!(novel.blocks().len(), 2, "should have 2 paragraph blocks");
        for (i, b) in novel.blocks().iter().enumerate() {
            assert!(
                matches!(b.kind, crate::builder::BlockKind::Paragraph),
                "block {i} should be Paragraph"
            );
            // Each paragraph should produce exactly 1 line (no trailing empty).
            let lines = b.lines(&doc_view);
            assert_eq!(
                lines.len(),
                1,
                "paragraph {i} should have 1 line, got {}: {:?}",
                lines.len(),
                lines
            );
        }

        // Full layout as baseline.
        let full = LazyLayout::from_doc(novel.clone(), &style, 400.0, &doc_view);
        assert!(full.laid_out.len() >= 2);
        assert!(full.laid_out[0].is_some());
        assert!(full.laid_out[1].is_some());
        let b0_full = full.laid_out[0].as_ref().unwrap();
        let b1_full = full.laid_out[1].as_ref().unwrap();
        let gap_full = b1_full.rect.y - (b0_full.rect.y + b0_full.rect.h);
        assert!(
            (gap_full - expected_gap).abs() < 1.0,
            "full layout gap {:.1} should be ~paragraph_spacing {:.1}",
            gap_full,
            expected_gap
        );

        // Viewport-driven path: ensure_visible + build_flat_lines.
        let mut vp = LazyLayout::new(novel.clone(), &style, 400.0, &doc_view);
        let total_h = vp.total_height;
        vp.ensure_visible(
            0.0,
            total_h + 10.0,
            &style,
            400.0,
            &mut shaping::Shaper::new().expect("need shaper"),
            None,
            &doc_view,
        );
        vp.build_flat_lines(&doc_view);

        assert!(vp.laid_out.len() >= 2);
        assert!(vp.laid_out[0].is_some(), "block 0 should be materialized");
        assert!(vp.laid_out[1].is_some(), "block 1 should be materialized");
        let b0_vp = vp.laid_out[0].as_ref().unwrap();
        let b1_vp = vp.laid_out[1].as_ref().unwrap();
        let real_y0 = b0_vp.rect.y + vp.y_delta.first().copied().unwrap_or(0.0);
        let real_y1 = b1_vp.rect.y + vp.y_delta.get(1).copied().unwrap_or(0.0);
        let gap_vp = real_y1 - (real_y0 + b0_vp.rect.h);
        assert!(
            (gap_vp - expected_gap).abs() < 1.0,
            "viewport-driven gap {:.1} should be ~paragraph_spacing {:.1} (line_height={:.1})",
            gap_vp,
            expected_gap,
            line_h
        );

        // Sanity: gap must not be ~2× line_height (the old bug).
        assert!(
            gap_vp < line_h * 1.2,
            "gap {:.1} should NOT be ~2 line heights ({:.1}) — paragraph_spacing={:.1}",
            gap_vp,
            line_h * 2.0,
            expected_gap
        );
    }

    // ===== Active block marker tests =====

    /// Build a LazyLayout with edit context (cursor at given byte).
    fn layout_with_cursor(md: &str, cursor_byte: usize) -> LazyLayout<crate::builder::MarkdownDoc> {
        let src = md;
        let parsed = crate::parser::parse_markdown(src);
        let doc = crate::builder::MarkdownDoc::build(&parsed, &default_style());
        let style = default_style();
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.set_edit_source(Some(src.to_string()));
        lazy.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte,
            preedit_text: None,
            preedit_cursor: None,
        }));
        // Rebuild with edit context for span expansion + active marker
        let mut shaper = shaping::Shaper::new().expect("need shaper");
        lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );
        lazy.build_flat_lines(&core::document::StringDocView::new(src));
        lazy
    }

    fn layout_with_cursor_and_width(
        source: &str,
        cursor_byte: usize,
        width: f32,
    ) -> LazyLayout<crate::builder::MarkdownDoc> {
        let parsed = crate::parser::parse_markdown(source);
        let style = default_style();
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(source);
        let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
        lazy.set_edit_source(Some(source.to_string()));
        lazy.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte,
            preedit_text: None,
            preedit_cursor: None,
        }));
        let mut shaper = shaping::Shaper::new().expect("projection test needs a shaper");
        lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
        lazy.build_flat_lines(&doc_view);
        lazy
    }

    #[test]
    fn every_editable_flat_line_has_projection_after_full_layout() {
        let corpus = [
            "plain paragraph",
            "# wrapped heading wrapped heading wrapped heading",
            "> first\n> second",
            "- first\n  continuation",
            "| a | b |\n| --- | --- |\n| c | d |",
            "```rust\nlet value = 1;\n```",
        ];

        for source in corpus {
            let lazy = layout_with_cursor_and_width(source, 0, 160.0);
            lazy.validate_editable_projections()
                .unwrap_or_else(|error| panic!("missing projection for {source:?}: {error:?}"));
        }
    }

    #[test]
    fn estimated_soft_wrap_preserves_editable_projections() {
        let source =
            "a paragraph that is deliberately long enough to require an estimated soft wrap";
        let style = default_style();
        let parsed = crate::parser::parse_markdown(source);
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(source);
        let estimated = layout_doc_with_shaper(doc.blocks(), &style, 40.0, None, None, &doc_view);
        let mut lazy = LazyLayout::new(doc, &style, 40.0, &doc_view);
        lazy.laid_out = estimated.blocks.into_iter().map(Some).collect();
        lazy.viewport_range = 0..lazy.laid_out.len();
        lazy.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte: 0,
            preedit_text: None,
            preedit_cursor: None,
        }));
        lazy.build_flat_lines(&doc_view);

        assert!(
            lazy.flat_lines
                .iter()
                .any(|line| line.text.is_empty() && line.source_projection.is_some()),
            "estimated soft wrapping must preserve an editable projection for each continuation"
        );
        lazy.validate_editable_projections()
            .expect("editable estimated continuations must have source projections");
    }

    #[test]
    fn local_viewport_rebuild_retains_stable_projection_identity_and_geometry() {
        let source = "first paragraph\n\nsecond paragraph\n\nthird paragraph";
        let mut lazy = layout_with_cursor_and_width(source, 0, 240.0);
        let before = lazy
            .source_projection_index
            .as_ref()
            .expect("full fixture must have an index")
            .visual_lines()
            .iter()
            .map(|line| (line.owner, line.source_extent.clone()))
            .collect::<Vec<_>>();
        lazy.evict_outside(&(1..2));
        lazy.viewport_range = 1..2;
        lazy.build_flat_lines(&core::document::StringDocView::new(source));
        assert_eq!(
            lazy.source_projection_index
                .as_ref()
                .expect("index must survive local viewport rebuild")
                .visual_lines()
                .iter()
                .map(|line| (line.owner, line.source_extent.clone()))
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn wrapped_layout_publishes_upstream_and_downstream_positions_at_shared_boundary() {
        let source = "a plain paragraph that is deliberately long enough to wrap several times";
        let lazy = layout_with_cursor_and_width(source, 0, 80.0);
        let index = lazy
            .source_projection_index
            .as_ref()
            .expect("a real wrapped layout must publish its projection index");
        let visual_lines = index.visual_lines();
        let (previous, next, shared_byte) = visual_lines
            .windows(2)
            .find_map(|lines| {
                let previous = &lines[0];
                let next = &lines[1];
                let shared_byte = previous.boundaries.last()?.byte;
                (shared_byte == next.boundaries.first()?.byte).then_some((
                    previous,
                    next,
                    shared_byte,
                ))
            })
            .expect("fixture must contain an automatic wrap boundary");

        assert_eq!(
            previous.boundaries.last(),
            Some(&SourceAnchor::downstream(shared_byte)),
            "layout must preserve the original source anchor on the preceding visual line",
        );
        assert_eq!(
            next.boundaries.first(),
            Some(&SourceAnchor::downstream(shared_byte)),
            "layout must preserve the original source anchor on the following visual line",
        );
        let upstream = index
            .visual_position_for_source(shared_byte, CursorAffinity::Upstream)
            .expect("the preceding visual line must be addressable upstream");
        let downstream = index
            .visual_position_for_source(shared_byte, CursorAffinity::Downstream)
            .expect("the following visual line must be addressable downstream");
        assert_eq!(upstream.flat_line_idx, previous.flat_line_idx);
        assert_eq!(upstream.grapheme_pos, previous.boundaries.len() - 1);
        assert_eq!(downstream.flat_line_idx, next.flat_line_idx);
        assert_eq!(downstream.grapheme_pos, 0);
    }

    #[test]
    #[should_panic(expected = "source projection layout revision must not overflow")]
    fn source_projection_revision_does_not_wrap_on_overflow() {
        let mut lazy = layout_with_cursor_and_width("plain paragraph", 0, 240.0);
        lazy.layout_revision = u64::MAX;

        let _ = lazy.rebuild_source_projection_index();
    }

    #[test]
    fn active_wrapped_heading_gives_every_segment_explicit_projection() {
        let source = "# a heading long enough to wrap across three visual rows";
        let lazy = layout_with_cursor_and_width(source, 4, 120.0);
        let lines = &lazy.flat_lines;
        assert!(lines.len() >= 3);
        assert!(lines.iter().all(|line| line.source_projection.is_some()));
    }

    #[test]
    fn consecutive_blockquote_projection_jumps_over_second_marker() {
        let source = "> first physical line\n> second physical line";
        let second = source.find("second").expect("fixture must contain second");
        let lazy = layout_with_cursor_and_width(source, second, 180.0);
        let second_line = lazy
            .flat_lines
            .iter()
            .find(|line| line.text.contains("second"))
            .expect("second text must be visible");
        let projection = second_line.source_projection.as_ref().expect("projection required");
        assert!(projection.boundaries.iter().any(|anchor| anchor.byte == second));
        assert!(!projection.boundaries.iter().any(|anchor| anchor.byte == 0));
    }

    #[test]
    fn nested_blockquote_syntax_gap_has_canonical_projection() {
        let source = "> outer\n> > **inner** — continuation";
        let lazy = layout_with_cursor_and_width(source, 8, 140.0);
        let index = lazy
            .source_projection_index
            .as_ref()
            .expect("nested blockquote layout must publish an index");

        for source_byte in 7..14 {
            assert!(
                index.visual_position_for_source(source_byte, CursorAffinity::Downstream).is_some(),
                "nested blockquote syntax byte {source_byte} must resolve canonically"
            );
        }
    }

    #[test]
    fn heading_active_marker_appears_in_flat_line_text() {
        let lazy = layout_with_cursor("# Title", 3); // cursor in "Title"
        assert!(!lazy.flat_lines.is_empty(), "should have flat lines");
        let first_text = &lazy.flat_lines[0].text;
        assert!(
            first_text.starts_with("# "),
            "heading flat line should start with '# ', got: {:?}",
            first_text
        );
        assert!(
            first_text.contains("Title"),
            "heading flat line should contain 'Title', got: {:?}",
            first_text
        );
    }

    #[test]
    fn heading_no_marker_when_cursor_outside_block() {
        // Cursor outside heading block range should NOT show marker
        let parsed = crate::parser::parse_markdown("# Title\n\nparagraph");
        let doc = crate::builder::MarkdownDoc::build(&parsed, &default_style());
        let style = default_style();
        let src = "# Title\n\nparagraph";
        let mut lazy =
            LazyLayout::from_doc(doc, &style, 400.0, &core::document::StringDocView::new(src));
        lazy.set_edit_source(Some(src.to_string()));
        // Cursor in paragraph (byte 10), NOT in heading
        lazy.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte: 10,
            preedit_text: None,
            preedit_cursor: None,
        }));
        let mut shaper = shaping::Shaper::new().expect("need shaper");
        lazy.ensure_precise_range(
            0.0,
            600.0,
            &style,
            &mut shaper,
            None,
            &core::document::StringDocView::new(src),
        );
        lazy.build_flat_lines(&core::document::StringDocView::new(src));

        let first_text = &lazy.flat_lines[0].text;
        assert!(
            !first_text.starts_with("# "),
            "heading flat line should NOT start with '# ' when cursor outside, got: {:?}",
            first_text
        );
        assert!(first_text.contains("Title"), "heading flat line should contain 'Title'");
    }

    #[test]
    fn heading_h2_active_marker_text() {
        let lazy = layout_with_cursor("## Section", 5); // cursor in "Section"
        let first_text = &lazy.flat_lines[0].text;
        assert!(
            first_text.starts_with("## "),
            "H2 flat line should start with '## ', got: {:?}",
            first_text
        );
    }

    #[test]
    fn list_item_active_marker_appears_in_flat_line_text() {
        let lazy = layout_with_cursor("- item", 3); // cursor in "item"
        assert!(!lazy.flat_lines.is_empty());
        let first_text = &lazy.flat_lines[0].text;
        assert!(
            first_text.starts_with("- "),
            "list flat line should start with '- ', got: {:?}",
            first_text
        );
        assert!(
            first_text.contains("item"),
            "list flat line should contain 'item', got: {:?}",
            first_text
        );
    }

    #[test]
    fn blockquote_active_marker_appears_in_flat_line_text() {
        let lazy = layout_with_cursor("> quoted", 5); // cursor in "quoted"
        assert!(!lazy.flat_lines.is_empty());
        let first_text = &lazy.flat_lines[0].text;
        assert!(
            first_text.starts_with("> "),
            "blockquote flat line should start with '> ', got: {:?}",
            first_text
        );
        assert!(
            first_text.contains("quoted"),
            "blockquote flat line should contain 'quoted', got: {:?}",
            first_text
        );
    }

    #[test]
    fn task_list_active_marker_appears_in_flat_line_text() {
        let lazy = layout_with_cursor("- [ ] todo", 7); // cursor in "todo"
        assert!(!lazy.flat_lines.is_empty());
        let first_text = &lazy.flat_lines[0].text;
        assert!(
            first_text.starts_with("- [ ] "),
            "task list flat line should start with '- [ ] ', got: {:?}",
            first_text
        );
        assert!(
            first_text.contains("todo"),
            "task list flat line should contain 'todo', got: {:?}",
            first_text
        );
    }

    #[test]
    fn heading_active_marker_projection_correct() {
        // "# Title" = 7 bytes: [0]='#', [1]=' ', [2]='T', [3]='i', [4]='t', [5]='l', [6]='e'
        let lazy = layout_with_cursor("# Title", 3);
        let fl = &lazy.flat_lines[0];
        let map = fl
            .source_projection
            .as_ref()
            .expect("should have projection")
            .boundaries
            .iter()
            .map(|anchor| anchor.byte)
            .collect::<Vec<_>>();
        // Visual chars: '#' (0), ' ' (1), 'T' (2), 'i' (3), 't' (4), 'l' (5), 'e' (6), sentinel (7)
        assert!(
            map.len() >= 8,
            "projection should have 8+ boundaries (7 chars + sentinel), got {}",
            map.len()
        );
        assert_eq!(map[0], 0, "visual char 0 '#' → source byte 0");
        assert_eq!(map[1], 1, "visual char 1 ' ' → source byte 1");
        assert_eq!(map[2], 2, "visual char 2 'T' → source byte 2");
        assert_eq!(map[3], 3, "visual char 3 'i' → source byte 3");
        assert_eq!(map[6], 6, "visual char 6 'e' → source byte 6");
        // Sentinel should point past the last byte (7 = length of "# Title")
        assert_eq!(map[7], 7, "sentinel should be 7 (len of '# Title')");
    }

    #[test]
    fn list_item_active_marker_projection_correct() {
        // "- item" = 6 bytes: [0]='-', [1]=' ', [2]='i', [3]='t', [4]='e', [5]='m'
        let lazy = layout_with_cursor("- item", 3);
        let fl = &lazy.flat_lines[0];
        let map = fl
            .source_projection
            .as_ref()
            .expect("should have projection")
            .boundaries
            .iter()
            .map(|anchor| anchor.byte)
            .collect::<Vec<_>>();
        assert!(
            map.len() >= 7,
            "projection should have 7+ boundaries (6 chars + sentinel), got {}",
            map.len()
        );
        assert_eq!(map[0], 0, "visual char 0 '-' → source byte 0");
        assert_eq!(map[1], 1, "visual char 1 ' ' → source byte 1");
        assert_eq!(map[2], 2, "visual char 2 'i' → source byte 2");
        assert_eq!(map[6], 6, "sentinel should be 6 (len of '- item')");
    }

    #[test]
    fn heading_with_inline_bold_active_marker_projection() {
        // "# **bold**" = 11 bytes: [0]='#', [1]=' ', [2]='*', [3]='*', [4]='b', [5]='o', [6]='l', [7]='d', [8]='*', [9]='*'
        // Cursor inside bold span (byte 5 = 'o')
        let lazy = layout_with_cursor("# **bold**", 5);
        let fl = &lazy.flat_lines[0];
        let map = fl
            .source_projection
            .as_ref()
            .expect("should have projection")
            .boundaries
            .iter()
            .map(|anchor| anchor.byte)
            .collect::<Vec<_>>();
        // Visual chars: '#', ' ', '**', 'bold', '**' = 10 chars
        // The marker "# " (bytes 0..2) should map correctly
        assert_eq!(map[0], 0, "visual char 0 '#' → source byte 0");
        assert_eq!(map[1], 1, "visual char 1 ' ' → source byte 1");
        // The expanded bold "**bold**" (bytes 2..10)
        assert_eq!(map[2], 2, "visual char 2 '*' → source byte 2");
        assert_eq!(map[3], 3, "visual char 3 '*' → source byte 3");
        assert_eq!(map[4], 4, "visual char 4 'b' → source byte 4");
        assert_eq!(map[8], 8, "visual char 8 '*' → source byte 8");
        assert_eq!(map[9], 9, "visual char 9 '*' → source byte 9");
        assert_eq!(map[10], 10, "sentinel should be 10 (len of '# **bold**')");
    }

    #[test]
    fn ordered_list_active_marker() {
        // "3. item" = 7 bytes: [0]='3', [1]='.', [2]=' ', [3]='i', [4]='t', [5]='e', [6]='m'
        let lazy = layout_with_cursor("3. item", 4);
        let first_text = &lazy.flat_lines[0].text;
        assert!(
            first_text.starts_with("3. "),
            "ordered list should start with '3. ', got: {:?}",
            first_text
        );
    }

    #[test]
    fn activating_ordered_list_materializes_only_the_current_source_number() {
        let source = "1. first\n7. second\n4. third";
        let cursor = source.find("second").expect("fixture must contain the active item");
        let layout = layout_with_cursor(source, cursor);
        let rendered_lines =
            layout.flat_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert_eq!(rendered_lines, ["first", "7. second", "third"]);
    }

    #[test]
    fn moving_cursor_into_ordered_list_invalidates_only_the_current_item() {
        let source = "1. first\n7. second\n4. third";
        let cursor = source.find("second").expect("fixture must contain the active item");
        let style = default_style();
        let parsed = crate::parser::parse_markdown(source);
        let document = crate::builder::MarkdownDoc::build(&parsed, &style);
        let document_view = core::document::StringDocView::new(source);
        let mut layout = LazyLayout::from_doc(document, &style, 400.0, &document_view);
        layout.set_edit_source(Some(source.to_owned()));
        layout.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte: cursor,
            preedit_text: None,
            preedit_cursor: None,
        }));

        layout.invalidate_lines_for_source_bytes([cursor]);
        layout.ensure_all_blocks(&style, 400.0, None, None, &document_view);
        layout.build_flat_lines(&document_view);

        let rendered_lines =
            layout.flat_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        assert_eq!(rendered_lines, ["first", "7. second", "third"]);
    }

    // ===== y-stability: marker prepend must not shift subsequent blocks =====

    /// Build two LazyLayouts of the same markdown, one with cursor outside the
    /// first list item (no marker) and one with cursor inside it (marker active).
    /// Returns `(lazy_outside, lazy_inside)`.
    fn layout_list_pair_outside_and_inside(
        md: &str,
    ) -> (LazyLayout<crate::builder::MarkdownDoc>, LazyLayout<crate::builder::MarkdownDoc>) {
        // Cursor in the SECOND item -> first item gets NO marker
        let outside = layout_with_cursor(md, 10);
        // Cursor in the FIRST item -> first item gets marker
        let inside = layout_with_cursor(md, 3);
        (outside, inside)
    }

    #[test]
    fn list_item_y_stable_when_cursor_moves_into_marker() {
        // "- first\n- second\n" = 17 bytes
        // first item block_range covers bytes 0..8, second item 8..17
        let md = "- first\n- second\n";
        let (outside, inside) = layout_list_pair_outside_and_inside(md);

        assert!(
            outside.laid_out.len() >= 2,
            "expected at least 2 laid-out blocks, got {}",
            outside.laid_out.len()
        );
        assert!(
            inside.laid_out.len() >= 2,
            "expected at least 2 laid-out blocks, got {}",
            inside.laid_out.len()
        );

        // Verify first item gets marker when cursor inside, not when outside
        let first_outside_text = &outside.flat_lines[0].text;
        let first_inside_text = &inside.flat_lines[0].text;
        assert!(
            !first_outside_text.starts_with("- "),
            "first item should NOT have marker when cursor is outside, got: {:?}",
            first_outside_text
        );
        assert!(
            first_inside_text.starts_with("- "),
            "first item should have marker when cursor is inside, got: {:?}",
            first_inside_text
        );

        // The critical assertion: second block's real y must be unchanged.
        let second_outside_y = outside.laid_out[1].as_ref().unwrap().rect.y
            + outside.y_delta.get(1).copied().unwrap_or(0.0);
        let second_inside_y = inside.laid_out[1].as_ref().unwrap().rect.y
            + inside.y_delta.get(1).copied().unwrap_or(0.0);

        assert!(
            (second_outside_y - second_inside_y).abs() < 0.5,
            "second list item y shifted: outside={:.1} vs inside={:.1} (delta={:.1})",
            second_outside_y,
            second_inside_y,
            second_outside_y - second_inside_y,
        );
    }

    #[test]
    fn list_item_height_stable_when_cursor_moves_into_marker() {
        // Same as above but also check that first block's height is unchanged.
        let md = "- first\n- second\n";
        let (outside, inside) = layout_list_pair_outside_and_inside(md);

        let first_outside_h = outside.laid_out[0].as_ref().unwrap().rect.h;
        let first_inside_h = inside.laid_out[0].as_ref().unwrap().rect.h;

        assert!(
            (first_outside_h - first_inside_h).abs() < 0.5,
            "first list item height changed: outside={:.1} vs inside={:.1}",
            first_outside_h,
            first_inside_h,
        );
    }

    #[test]
    fn total_height_stable_when_list_marker_appears() {
        // Total document height must not change when only marker visibility changes.
        let md = "- first\n- second\n";
        let (outside, inside) = layout_list_pair_outside_and_inside(md);

        assert!(
            (outside.total_height - inside.total_height).abs() < 0.5,
            "total_height changed: outside={:.1} vs inside={:.1} (delta={:.1})",
            outside.total_height,
            inside.total_height,
            outside.total_height - inside.total_height,
        );
    }

    #[test]
    fn heading_y_stable_when_cursor_moves_into_marker() {
        // Same property for headings: marker prepend must not shift subsequent blocks.
        let md = "# Title\n\nparagraph";
        // Cursor at byte 10 (in "paragraph") -> heading has no marker
        let outside = layout_with_cursor(md, 10);
        // Cursor at byte 3 (in "Title") -> heading has marker
        let inside = layout_with_cursor(md, 3);

        // Heading text check
        assert!(
            !outside.flat_lines[0].text.starts_with("# "),
            "heading should NOT have marker when cursor outside"
        );
        assert!(
            inside.flat_lines[0].text.starts_with("# "),
            "heading should have marker when cursor inside"
        );

        // Second block y stability
        if outside.laid_out.len() >= 2 && inside.laid_out.len() >= 2 {
            let second_outside_y = outside.laid_out[1].as_ref().unwrap().rect.y
                + outside.y_delta.get(1).copied().unwrap_or(0.0);
            let second_inside_y = inside.laid_out[1].as_ref().unwrap().rect.y
                + inside.y_delta.get(1).copied().unwrap_or(0.0);

            assert!(
                (second_outside_y - second_inside_y).abs() < 0.5,
                "second block y shifted when heading marker appeared: outside={:.1} vs inside={:.1}",
                second_outside_y,
                second_inside_y,
            );
        }
    }

    #[test]
    fn blockquote_y_stable_when_cursor_moves_into_marker() {
        // Use a doc with blockquote + paragraph so we can place cursor
        // outside the blockquote to test marker on/off while checking
        // subsequent block y stability.
        let md = "> quoted\n\nparagraph";
        // "> quoted\n\nparagraph"
        // [0]='>', [1]=' ', [2..8]="quoted", [8]='\n', [9]='\n',
        // [10]='p', [11..18]="aragraph"
        // blockquote block_range: 0..9, paragraph block_range: 9..19

        // Cursor at byte 12 (in "paragraph") -> blockquote has no marker
        let outside = layout_with_cursor(md, 12);
        // Cursor at byte 3 (in "quoted") -> blockquote has marker
        let inside = layout_with_cursor(md, 3);

        // Blockquote text check
        assert!(
            !outside.flat_lines[0].text.starts_with("> "),
            "blockquote should NOT have marker when cursor outside, got: {:?}",
            outside.flat_lines[0].text
        );
        assert!(
            inside.flat_lines[0].text.starts_with("> "),
            "blockquote should have marker when cursor inside, got: {:?}",
            inside.flat_lines[0].text
        );

        // Height stability
        let first_outside_h = outside.laid_out[0].as_ref().unwrap().rect.h;
        let first_inside_h = inside.laid_out[0].as_ref().unwrap().rect.h;
        assert!(
            (first_outside_h - first_inside_h).abs() < 0.5,
            "blockquote height changed: outside={:.1} vs inside={:.1}",
            first_outside_h,
            first_inside_h,
        );

        // Second block y stability
        if outside.laid_out.len() >= 2 && inside.laid_out.len() >= 2 {
            let second_outside_y = outside.laid_out[1].as_ref().unwrap().rect.y
                + outside.y_delta.get(1).copied().unwrap_or(0.0);
            let second_inside_y = inside.laid_out[1].as_ref().unwrap().rect.y
                + inside.y_delta.get(1).copied().unwrap_or(0.0);
            assert!(
                (second_outside_y - second_inside_y).abs() < 0.5,
                "paragraph y shifted when blockquote marker appeared: outside={:.1} vs inside={:.1}",
                second_outside_y,
                second_inside_y,
            );
        }
    }
}
