//! Layout output types and LazyLayout implementation.

use crate::builder::StyleSpan;
use crate::projection::{
    ProjectionError, ProjectionLineIdx, ProjectionOwnerId, SourceAnchor, SourceProjectionIndex,
};
use crate::selection::FlatLineIdx;
use crate::source_snapshot::{MarkdownSource, SourceSnapshot};
use shaping::Weight;
use std::ops::Range;
use std::sync::Arc;
use ui::core::geom::Rect;

use super::BlockSource;
use super::source_line_map::HiddenBlockSeparator;

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
    pub owner: ProjectionOwnerId,
    pub boundaries: Vec<SourceAnchor>,
    pub source_extent: Range<usize>,
    pub collapsed: Vec<CollapsedBoundary>,
}

#[derive(Clone, Debug)]
pub(in crate::layout::types) struct RetainedVisualLineProjection {
    pub(in crate::layout::types) projection: VisualLineProjection,
    /// Uncorrected document y coordinate. The owning block's current `y_delta`
    /// is applied only when publishing the projection index.
    pub(in crate::layout::types) y: f32,
}

/// Snapshot of an explicitly laid out empty paragraph, for projection lookup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ProjectedEmptyLine {
    pub owner: ProjectionOwnerId,
    pub source_byte: usize,
    pub y_top: f32,
    pub height: f32,
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
    /// Estimated content height of each laid-out block, excluding boundary gaps.
    pub estimated_heights: Vec<f32>,
    /// Estimated content top, including the resolved gaps before this block.
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
    pub(in crate::layout::types) flat_line_revision: u64,
    /// Reconciled uncorrected flat-line groups waiting for their next publication.
    pub(in crate::layout::types) reconciled_flat_lines: Vec<Option<Vec<FlatLine>>>,
    /// Lightweight source projections retained after shaped blocks are evicted.
    pub(in crate::layout::types) retained_block_projections: Vec<Vec<RetainedVisualLineProjection>>,
    /// Canonical generation-safe mapping between source anchors and visual positions.
    pub(crate) source_projection_index: Option<SourceProjectionIndex>,
    /// Most recent validation failure while publishing the canonical source projection index.
    pub(crate) source_projection_error: Option<ProjectionError>,
    /// Generation of the source text used to build the canonical projection index.
    pub(in crate::layout::types) source_generation: u32,
    /// Monotonic revision of successfully published projection indexes.
    pub(in crate::layout::types) layout_revision: u64,
    /// Maps laid_out index → doc.blocks index.
    /// Needed because Container blocks expand into multiple laid-out blocks.
    pub(in crate::layout::types) laid_to_doc: Vec<usize>,
    /// Currently visible block range (for eviction tracking).
    pub(in crate::layout::types) viewport_range: std::ops::Range<usize>,
    /// Cached viewport width from construction — used by precise_block_at
    /// when the block hasn't been materialized yet.
    pub(in crate::layout::types) cached_viewport_w: f32,
    /// Full source text for WYSIWYG span expansion via materialize_line.
    pub(in crate::layout::types) source_snapshot: Option<SourceSnapshot>,
    /// Empty paragraph geometry derived from the same rows exposed in `flat_lines`.
    pub(in crate::layout::types) projected_empty_lines: Vec<ProjectedEmptyLine>,
    /// Hidden source ranges represented as collapsed boundaries in the index.
    pub(in crate::layout::types) hidden_block_separators: Vec<HiddenBlockSeparator>,
    /// Projection line → rendered line. Retained rows outside the viewport have no rendered line.
    pub(in crate::layout::types) projection_flat_line_indices: Vec<Option<FlatLineIdx>>,
    /// Rendered line → projection line. Atomic rows have no editable projection.
    pub(in crate::layout::types) flat_line_projection_indices: Vec<Option<ProjectionLineIdx>>,
    /// Projection visual-line id → geometry for an explicitly laid out empty paragraph.
    pub(in crate::layout::types) projected_empty_line_geometry:
        std::collections::BTreeMap<ProjectionLineIdx, ProjectedEmptyLine>,
    /// WYSIWYG edit context; None means pure preview (no cursor span expansion).
    pub(in crate::layout::types) edit_ctx: Option<crate::edit::EditContext>,
    pub(crate) code_viewport: Option<super::code_viewport::CodeViewport>,
    /// Non-empty source selection range used for block-level editing fallbacks.
    pub(in crate::layout::types) selection_range: Option<std::ops::Range<usize>>,
    /// Crate-private rendering sidecar for ASCII diagram grid metadata.
    pub(in crate::layout::types) ascii_diagrams: crate::layout::ascii_diagram::AsciiDiagramRegistry,
    pub(in crate::layout::types) embedded_images: crate::layout::embedded::EmbeddedRegistry,
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
    Embedded {
        source_range: Range<usize>,
    },
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
    /// Source range owned by a non-text visual block such as a horizontal rule.
    pub(crate) atomic_source_range: Option<Range<usize>>,
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

#[path = "lazy_pass.rs"]
mod lazy_pass;
#[path = "lazy_projection.rs"]
mod lazy_projection;

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
