//! Render pipeline: shape visible lines, generate vertices.
//!
//! Extracts the heavy shaping/rendering logic from App.

use crate::cursor_motion::{CursorRenderState, find_visual_line_index};
use crate::document_presentation::DocumentPresentation;
use crate::frame_cache::{FrameCache, LineCache};
use crate::render_cache::{CachedLine, GlyphInstance};
use crate::render_state::{GpuState, TextState};
use appkit_core::document::DocumentModel;
use render::GlyphVertex;
use std::ops::Range;
use textora_core::highlight::HighlightKind;
use ui::decorations::*;
use ui::gutter::ATLAS_SIZE;
use ui::gutter::*;
use ui::layout::*;
use ui::render_geom::AdvanceCacheEntry;
use ui::typography::{TextSpacingMode, TypographyCluster, TypographyInput};

use render::{GlyphKey, GlyphRenderer};

use crate::reshape_worker::content_hash_for_layout;

const PERF_LOG_ENV: &str = "EDIT_PLUS_PERF_LOG";
const PERF_LOG_THRESHOLD_US_ENV: &str = "EDIT_PLUS_PERF_LOG_THRESHOLD_US";
const DEFAULT_PERF_LOG_THRESHOLD_US: u128 = 1_000;
const MARKDOWN_LANGUAGE_ID: &str = "markdown";
const MAX_SYNC_SHAPE_BYTES: usize = 4_000;

#[derive(Clone, Copy)]
struct ShapeMissMapStats {
    hash_matches: bool,
    visual_break_count: usize,
    visual_line_count: u16,
    byte_offset: usize,
    byte_length: u32,
    is_placeholder: bool,
}

impl ShapeMissMapStats {
    fn from_entry(entry: &crate::snap_tree::DisplayLineEntry, content_hash: u64) -> Self {
        let is_placeholder = entry.visual_breaks.is_empty()
            || (entry.visual_breaks.len() == 1 && entry.visual_breaks[0].pixel_width == 0.0);
        Self {
            hash_matches: entry.content_hash == content_hash,
            visual_break_count: entry.visual_breaks.len(),
            visual_line_count: entry.visual_line_count,
            byte_offset: entry.byte_offset,
            byte_length: entry.byte_length,
            is_placeholder,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShapedLineScope {
    FullLine,
    VisualSubset,
}

impl ShapedLineScope {
    fn from_subset_flag(shape_subset_only: bool) -> Self {
        if shape_subset_only { Self::VisualSubset } else { Self::FullLine }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RenderCacheMetadata {
    visual_line_count: u16,
    subset_start: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderViewportState {
    skip_visual: usize,
    sub_line_offset: f32,
    start_doc: usize,
    viewport_visual_rows: usize,
    candidate_lines: usize,
    cursor_doc_line: usize,
    cursor_relative_line: Option<usize>,
}

impl RenderViewportState {
    fn is_filled_after(self, relative_line: usize, rendered_rows: usize) -> bool {
        rendered_rows >= self.viewport_visual_rows
            && self.cursor_relative_line.is_none_or(|cursor_line| relative_line >= cursor_line)
    }
}

fn visual_line_count_from_len(visual_line_count: usize) -> u16 {
    visual_line_count.min(u16::MAX as usize) as u16
}

fn resolve_visible_line_range(
    presentation: &DocumentPresentation,
    line_height: f32,
    total_lines: usize,
    cursor_doc_line: usize,
) -> RenderViewportState {
    let anchor = presentation.display.viewport.scroll_anchor;
    let skip_visual = (anchor.pixel_offset / line_height) as usize;
    let sub_line_offset = -(anchor.pixel_offset - skip_visual as f32 * line_height);
    let start_doc = anchor.doc_line.min(total_lines.saturating_sub(1));
    let viewport_visual_rows = presentation.display.viewport.viewport_height.ceil() as usize + 2;

    let candidate_lines = total_lines.saturating_sub(start_doc);
    let cursor_relative_line =
        cursor_doc_line.checked_sub(start_doc).filter(|line| *line < viewport_visual_rows * 4);
    RenderViewportState {
        skip_visual,
        sub_line_offset,
        start_doc,
        viewport_visual_rows,
        candidate_lines,
        cursor_doc_line,
        cursor_relative_line,
    }
}

fn highlights_for_line<'a>(
    document: &DocumentModel,
    presentation: &'a mut DocumentPresentation,
    line_index: usize,
) -> &'a [textora_core::highlight::Highlight<HighlightKind>] {
    use stdext::arena::scratch_arena;
    use textora_core::highlight::Highlighter as CoreHighlighter;

    let Some(language) = document.language else {
        return &[];
    };

    let arena = scratch_arena(None);
    let mut highlighter = CoreHighlighter::new(&document.tb, language);
    presentation.highlighter_cache.parse_line(
        &arena,
        &mut highlighter,
        line_index as textora_core::helpers::CoordType,
        |line| document.line_index.offsets.get(line as usize).copied().unwrap_or(0),
    )
}

fn should_store_shaped_line_in_render_cache(
    scope: ShapedLineScope,
    preedit_on_cursor_line: bool,
    cache_exists: bool,
) -> bool {
    if preedit_on_cursor_line || cache_exists {
        return false;
    }
    matches!(scope, ShapedLineScope::FullLine | ShapedLineScope::VisualSubset)
}

fn render_cache_metadata_for_shaped_line(
    scope: ShapedLineScope,
    shaped_visual_line_count: usize,
    doc_visual_line_count: u16,
    skip_visual_local: usize,
    subset_start_visual_line: usize,
) -> RenderCacheMetadata {
    match scope {
        ShapedLineScope::FullLine => RenderCacheMetadata {
            visual_line_count: visual_line_count_from_len(shaped_visual_line_count),
            subset_start: skip_visual_local,
        },
        ShapedLineScope::VisualSubset => RenderCacheMetadata {
            visual_line_count: doc_visual_line_count,
            subset_start: subset_start_visual_line,
        },
    }
}

fn cached_line_rows_to_advance(
    scope: ShapedLineScope,
    cached_visual_line_count: u16,
    cached_visual_lines_len: usize,
    skip_visual: usize,
    relative_skip: usize,
) -> usize {
    match scope {
        ShapedLineScope::FullLine => cached_visual_lines_len.saturating_sub(relative_skip),
        ShapedLineScope::VisualSubset => {
            (cached_visual_line_count as usize).saturating_sub(skip_visual)
        }
    }
}

fn perf_logging_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(PERF_LOG_ENV).is_ok_and(|value| {
            matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
        })
    })
}

pub(crate) fn document_uses_verbatim_source_spacing(document: &DocumentModel) -> bool {
    document
        .language
        .is_some_and(|language| !language.id.eq_ignore_ascii_case(MARKDOWN_LANGUAGE_ID))
}

pub(crate) fn spacing_mode_for_line(
    global_mode: TextSpacingMode,
    source_uses_verbatim_spacing: bool,
    line_bytes: &[u8],
) -> TextSpacingMode {
    if source_uses_verbatim_spacing
        || global_mode == TextSpacingMode::Verbatim
        || line_bytes.contains(&b'\t')
    {
        TextSpacingMode::Verbatim
    } else {
        TextSpacingMode::Natural
    }
}

fn precise_cluster_advance(
    cluster_bytes: &[u8],
    shaped_advance: f32,
    char_width: f32,
) -> (bool, f32) {
    if is_whitespace_cluster(cluster_bytes) {
        (true, ws_cluster_advance(cluster_bytes, char_width))
    } else {
        (false, shaped_advance.max(0.0))
    }
}

pub(crate) fn protected_ranges_for_line(
    protected_ranges: &[std::ops::Range<usize>],
    line_offset: usize,
    line_length: usize,
) -> Vec<std::ops::Range<usize>> {
    let line_end = line_offset + line_length;
    let first = protected_ranges.partition_point(|range| range.end <= line_offset);
    let last = first + protected_ranges[first..].partition_point(|range| range.start < line_end);
    protected_ranges[first..last]
        .iter()
        .map(|range| {
            range.start.max(line_offset) - line_offset..range.end.min(line_end) - line_offset
        })
        .collect()
}

fn perf_log_threshold_us() -> u128 {
    static THRESHOLD_US: std::sync::OnceLock<u128> = std::sync::OnceLock::new();
    *THRESHOLD_US.get_or_init(|| {
        std::env::var(PERF_LOG_THRESHOLD_US_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_PERF_LOG_THRESHOLD_US)
    })
}

fn should_perf_log(elapsed_us: u128) -> bool {
    perf_logging_enabled() && elapsed_us >= perf_log_threshold_us()
}

/// Context grouping read-only configuration and environment variables for the render pipeline.
/// Maximum bytes per line to compute word-wrap.
/// Lines exceeding this get a single visual line without wrapping.

/// Compute the available viewport width for text rendering,
/// subtracting the left margin and scrollbar reserve.
pub fn render_viewport_width(
    screen_w: f32,
    left_margin: f32,
    metrics: &ui::settings::UiMetrics,
    word_wrap: bool,
) -> f32 {
    let physical_w = screen_w - left_margin - metrics.scrollbar_reserve;
    const NO_WRAP_SENTINEL: f32 = 1_000_000.0;
    if word_wrap { physical_w.max(1.0) } else { NO_WRAP_SENTINEL }
}

fn cursor_relative_offset_for_line(
    cursor_offset: usize,
    current_line_offset: usize,
) -> Option<u32> {
    cursor_offset.checked_sub(current_line_offset).map(|offset| offset as u32)
}

struct GutterLine {
    doc_index: usize,
    top: f32,
    active: bool,
}

fn emit_gutter_line_number(
    metrics: &ui::settings::UiMetrics,
    ctx: &RenderContext,
    text: &mut TextState,
    gpu: &GpuState,
    line: GutterLine,
    all_vertices: &mut Vec<GlyphVertex>,
) {
    let _line_num_buf = format_line_num(line.doc_index + 1);
    let line_num_str = line_num_str(&_line_num_buf);
    if let Ok(line_num_shaped) = text.shaper.shape(line_num_str) {
        let ln_font_size = text.shaper.font_size() * ui::constants::LN_FONT_SCALE;
        let ln_verts = generate_line_number_vertices(
            ctx,
            &line_num_shaped,
            &mut text.atlas,
            &text.atlas_texture,
            &gpu.ctx.queue,
            &mut text.shaper,
            ln_font_size,
            line.top,
            line.active,
            metrics.gutter_padding,
            metrics.line_height,
        );
        all_vertices.extend(ln_verts);
    }
}

fn build_cached_advance_cache_entries(
    cached: &CachedLine,
    doc_line_idx: usize,
    relative_skip: usize,
    left_margin: f32,
) -> Vec<AdvanceCacheEntry> {
    let mut entries = Vec::new();
    let mut vl_grapheme_start = 0usize;

    for (vl_idx, &(vl_start, vl_end, _)) in cached.visual_lines.iter().enumerate() {
        let grapheme_count = cached.cluster_data[vl_start..vl_end].len();
        if vl_idx < relative_skip {
            vl_grapheme_start += grapheme_count;
            continue;
        }

        let vl_byte_start = cached.cluster_data.get(vl_start).map(|cd| cd.0).unwrap_or(0);
        let mut clusters_for_vl = Vec::with_capacity(grapheme_count);
        let mut px = left_margin;
        for (grapheme_idx, cd) in cached.cluster_data[vl_start..vl_end].iter().enumerate() {
            px += cd.2;
            clusters_for_vl.push((cd.1.saturating_sub(vl_byte_start), px, grapheme_idx as u32));
        }
        entries.push(AdvanceCacheEntry {
            doc_line: doc_line_idx,
            vl_byte_start,
            vl_grapheme_start,
            clusters: clusters_for_vl,
        });
        vl_grapheme_start += grapheme_count;
    }

    entries
}

struct SourceLineShapeOptions<'a> {
    text: &'a str,
    bytes: &'a [u8],
    cluster_byte_offset: usize,
    highlights: &'a [(usize, HighlightKind)],
    has_font_overrides: bool,
    display_breaks: Option<&'a [crate::snap_tree::VisualBreak]>,
    spacing: TextSpacingMode,
    protected_ranges: &'a [Range<usize>],
    viewport_width: f32,
}

enum LineShape {
    Empty,
    Wrapped { shaped: shaping::ShapedRun, visual_lines: Vec<(usize, usize, f32)>, char_width: f32 },
}

fn shape_and_wrap_line(
    shaper: &mut shaping::Shaper,
    options: SourceLineShapeOptions<'_>,
) -> Result<LineShape, shaping::ShapeError> {
    let mut shaped = shape_source_run(
        shaper,
        options.text,
        options.cluster_byte_offset,
        options.highlights,
        options.has_font_overrides,
    )?;
    if shaped.clusters.is_empty() {
        return Ok(LineShape::Empty);
    }
    for cluster in &mut shaped.clusters {
        cluster.byte_range.start += options.cluster_byte_offset;
        cluster.byte_range.end += options.cluster_byte_offset;
    }

    let char_width = shaper.col_width();
    let typography_clusters: Vec<TypographyCluster> = shaped
        .clusters
        .iter()
        .map(|cluster| TypographyCluster::new(cluster.byte_range.clone(), shaper.font_size()))
        .collect();
    let typography_input = TypographyInput::new(
        std::str::from_utf8(options.bytes).unwrap_or_default(),
        options.spacing,
        &typography_clusters,
        options.protected_ranges,
    );
    let gaps = ui::typography::calculate_gaps(&typography_input);

    let visual_lines = options.display_breaks.and_then(|breaks| {
        let visual_lines = map_display_breaks_to_clusters(&shaped, breaks);
        visual_lines.iter().all(|&(start, end, _)| start <= end).then_some(visual_lines)
    });
    if let Some(visual_lines) = visual_lines {
        let shaped = ui::layout::typography::apply_gaps_to_existing_visual_lines(
            &shaped,
            &gaps,
            &visual_lines,
        );
        return Ok(LineShape::Wrapped { shaped, visual_lines, char_width });
    }

    let layout = ui::layout::typography::layout_shaped_run_with_gaps(
        &shaped,
        &gaps,
        options.bytes,
        char_width,
        options.viewport_width,
        0.5,
        0.0,
    );
    Ok(LineShape::Wrapped { shaped: layout.shaped, visual_lines: layout.visual_lines, char_width })
}

fn map_display_breaks_to_clusters(
    shaped: &shaping::ShapedRun,
    breaks: &[crate::snap_tree::VisualBreak],
) -> Vec<(usize, usize, f32)> {
    breaks
        .iter()
        .map(|visual_break| {
            let start = shaped.clusters.partition_point(|cluster| {
                cluster.byte_range.start < visual_break.byte_start as usize
            });
            let end = shaped.clusters.partition_point(|cluster| {
                cluster.byte_range.start < visual_break.byte_end as usize
            });
            (start, end, visual_break.pixel_width)
        })
        .collect()
}

fn full_line_display_breaks(
    entry: &crate::snap_tree::DisplayLineEntry,
    layout_hash: u64,
) -> Option<&[crate::snap_tree::VisualBreak]> {
    let placeholder = entry.visual_breaks.len() == 1
        && entry.visual_breaks[0].pixel_width == 0.0
        && entry.byte_length > 0;
    (!placeholder && entry.content_hash == layout_hash && !entry.visual_breaks.is_empty())
        .then_some(entry.visual_breaks.as_slice())
}

fn shape_source_run(
    shaper: &mut shaping::Shaper,
    line_str_to_shape: &str,
    byte_offset_for_clusters: usize,
    highlight_spans: &[(usize, HighlightKind)],
    has_bold_italic: bool,
) -> Result<shaping::ShapedRun, shaping::ShapeError> {
    if !has_bold_italic {
        return shaper.shape_fast(line_str_to_shape).or_else(|_| shaper.shape(line_str_to_shape));
    }
    // Build per-span weight/style overrides from highlight spans.
    // When shape_subset_only, adjust byte offsets to be relative to the substring.
    let subset_start = byte_offset_for_clusters;
    let subset_end = subset_start + line_str_to_shape.len();
    let mut span_attrs: Vec<(usize, shaping::Weight, shaping::Style)> = Vec::new();
    let mut last_end = 0usize;
    for &(highlight_start, kind) in highlight_spans {
        // Skip highlights entirely outside the visible subset
        if highlight_start >= subset_end {
            break;
        }
        // Clip highlights that start before the subset
        let start = highlight_start.saturating_sub(subset_start);
        if start >= line_str_to_shape.len() {
            continue;
        }
        // Fill gap with base weight/style
        if start > last_end {
            span_attrs.push((last_end, shaper.font_weight(), shaper.font_style()));
        }
        let (w, s) = match kind {
            HighlightKind::MarkupBold => (shaping::Weight::BOLD, shaper.font_style()),
            HighlightKind::MarkupItalic => (shaper.font_weight(), shaping::Style::Italic),
            HighlightKind::MarkupHeading => (shaping::Weight::BOLD, shaper.font_style()),
            _ => (shaper.font_weight(), shaper.font_style()),
        };
        span_attrs.push((start, w, s));
        last_end = start;
    }
    // Fill trailing gap
    if last_end < line_str_to_shape.len() {
        span_attrs.push((last_end, shaper.font_weight(), shaper.font_style()));
    }
    // Deduplicate consecutive identical attrs
    span_attrs.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);

    shaper.shape_with_highlights(line_str_to_shape, &span_attrs)
}

struct LineCacheInput<'a> {
    doc_index: usize,
    doc_offset: usize,
    bytes: &'a [u8],
    spacing: TextSpacingMode,
    preedit_on_cursor_line: bool,
}

struct LineCacheLookup {
    layout_hash: u64,
    protected_ranges: Vec<Range<usize>>,
    cached: Option<CachedLine>,
}

fn lookup_or_prepare_cached_line(
    line: LineCacheInput<'_>,
    presentation: &mut DocumentPresentation,
    viewport_width: f32,
    font_size: f32,
    font_family: &str,
) -> LineCacheLookup {
    let protected_ranges = protected_ranges_for_line(
        &presentation.source_spacing.protected_ranges,
        line.doc_offset,
        line.bytes.len(),
    );
    let layout_hash = content_hash_for_layout(
        line.bytes,
        line.doc_offset,
        viewport_width,
        font_size,
        line.spacing,
        &protected_ranges,
        presentation.source_spacing.semantic_version,
        font_family,
    );
    let render_cache = &mut presentation.display.render_cache;
    let cached = if line.preedit_on_cursor_line {
        None
    } else {
        let matching = render_cache
            .get(line.doc_index)
            .filter(|cached| {
                cached.content_hash == layout_hash
                    && cached.visual_lines.len() == cached.visual_line_instance_starts.len()
            })
            .cloned();
        if matching.is_none() {
            render_cache.invalidate(line.doc_index);
        }
        matching
    };
    LineCacheLookup { layout_hash, protected_ranges, cached }
}

#[derive(Clone, Copy)]
pub struct SourceLayoutOptions<'a> {
    pub metrics: &'a ui::settings::UiMetrics,
    pub text_spacing_mode: TextSpacingMode,
    pub render_context: &'a RenderContext<'a>,
    pub word_wrap: bool,
}

pub struct ShapedVisibleLines {
    pub vertices: Vec<GlyphVertex>,
    pub tree_dirty: bool,
}

#[derive(Clone, Copy)]
struct LinePlacement {
    doc_index: usize,
    screen_row: usize,
    skipped_rows: usize,
    sub_line_offset: f32,
    active: bool,
}

struct CachedLineEmission {
    relative_skip: usize,
    rows_advanced: usize,
}

fn emit_cached_line_and_gutter_vertices(
    cached: &CachedLine,
    placement: LinePlacement,
    options: SourceLayoutOptions<'_>,
    rows_needed: usize,
    text: &mut TextState,
    gpu: &GpuState,
    vertices: &mut Vec<GlyphVertex>,
) -> Option<CachedLineEmission> {
    let is_full_line =
        cached.subset_start == 0 && cached.visual_lines.len() == cached.visual_line_count as usize;
    let usable_subset =
        placement.skipped_rows == cached.subset_start && cached.visual_lines.len() >= rows_needed;
    if !is_full_line && !usable_subset {
        return None;
    }

    let metrics = options.metrics;
    let ctx = options.render_context;
    let relative_skip = placement.skipped_rows.saturating_sub(cached.subset_start);
    for row in relative_skip..cached.visual_lines.len() {
        let line_y = (placement.screen_row + row - relative_skip) as f32 * metrics.line_height
            + placement.sub_line_offset;
        vertices.extend(cached.emit_vertices_for_visual_line(
            row,
            line_y,
            metrics.line_height,
            ctx.tab_bar_height,
            ctx.screen_w,
            ctx.screen_h,
            ctx.theme.editor.foreground,
            ctx.theme,
            None,
        ));
        if row == 0 && ctx.gutter_width > 0.0 && placement.skipped_rows == 0 {
            emit_gutter_line_number(
                metrics,
                ctx,
                text,
                gpu,
                GutterLine {
                    doc_index: placement.doc_index,
                    top: line_y,
                    active: placement.active,
                },
                vertices,
            );
        }
    }
    let scope = ShapedLineScope::from_subset_flag(!is_full_line);
    let rows_advanced = cached_line_rows_to_advance(
        scope,
        cached.visual_line_count,
        cached.visual_lines.len(),
        placement.skipped_rows,
        relative_skip,
    );
    Some(CachedLineEmission { relative_skip, rows_advanced })
}

fn update_cursor_from_cached_line(
    document: &mut DocumentModel,
    cursor: &mut CursorRenderState,
    cached: &CachedLine,
    placement: LinePlacement,
    relative_skip: usize,
    left_margin: f32,
) {
    let cursor_col = document.cursor_column();
    let bounds = cached
        .visual_lines
        .iter()
        .map(|&(start, end, _)| {
            let first_byte = cached.cluster_data.get(start).map_or(usize::MAX, |cluster| cluster.0);
            let last_byte = end
                .checked_sub(1)
                .and_then(|last| cached.cluster_data.get(last))
                .map_or(usize::MAX, |cluster| cluster.1);
            (first_byte, last_byte)
        })
        .collect::<Vec<_>>();
    let cursor_row = find_visual_line_index(&bounds, cursor_col);
    let full_row = cursor_row + cached.subset_start;
    cursor.cursor_visual_line_in_doc = full_row;
    cursor.cursor_visual_line =
        Some((placement.screen_row + full_row).saturating_sub(placement.skipped_rows));
    if cursor_row < relative_skip {
        return;
    }

    if let Some(&(start, end, _)) = cached.visual_lines.get(cursor_row) {
        let cluster_end =
            cached.cluster_data.get(end.saturating_sub(1)).map_or(0, |cluster| cluster.1);
        if let Some(cursor_x) = compute_cursor_pixel_x_cached(
            &cached.cluster_data,
            start,
            end,
            cursor_col,
            cluster_end,
            left_margin,
        ) {
            cursor.cursor_pixel_x = cursor_x;
        }
    }
    if cursor.sticky_x_dirty
        && document.cursor().offset.to_usize()
            >= document.line_byte_offset(placement.doc_index).unwrap_or(0)
    {
        cursor.sticky_x = cursor.cursor_pixel_x;
        cursor.sticky_x_dirty = false;
    }
}

#[derive(Clone, Copy)]
struct ShapedLineView<'a> {
    shaped: &'a shaping::ShapedRun,
    visual_lines: &'a [(usize, usize, f32)],
    bytes: &'a [u8],
    highlights: &'a [(usize, HighlightKind)],
    char_width: f32,
}

struct FreshLineEmission<'a> {
    line: ShapedLineView<'a>,
    placement: LinePlacement,
    show_line_number: bool,
    preedit_on_cursor_line: bool,
}

fn emit_line_and_gutter_vertices(
    emission: FreshLineEmission<'_>,
    options: SourceLayoutOptions<'_>,
    text: &mut TextState,
    gpu: &GpuState,
    all_vertices: &mut Vec<GlyphVertex>,
) {
    let placement = emission.placement;
    let ctx = options.render_context;
    let metrics = options.metrics;
    let skip_visual_local = placement.skipped_rows;
    let ShapedLineView {
        shaped,
        visual_lines,
        bytes: line_bytes,
        highlights: highlight_spans,
        char_width,
    } = emission.line;
    if emission.show_line_number && ctx.gutter_width > 0.0 {
        emit_gutter_line_number(
            metrics,
            ctx,
            text,
            gpu,
            GutterLine {
                doc_index: placement.doc_index,
                top: placement.screen_row as f32 * metrics.line_height + placement.sub_line_offset,
                active: placement.active,
            },
            all_vertices,
        );
    }

    for (vl_idx, &(vl_start, vl_end, _vl_width)) in visual_lines.iter().enumerate() {
        if vl_idx < skip_visual_local {
            continue;
        }
        let line_y = (placement.screen_row + vl_idx - skip_visual_local) as f32
            * metrics.line_height
            + placement.sub_line_offset;
        let y_base =
            line_y + metrics.line_height * ui::constants::BASELINE_RATIO + ctx.tab_bar_height;

        let mut x_cursor = ctx.left_margin;

        for cluster in &shaped.clusters[vl_start..vl_end] {
            let glyph_id = cluster.glyph_id as u16;
            let font_id = cluster.font_id;
            let font_size = text.shaper.font_size();

            let cluster_bytes = &line_bytes[cluster.byte_range.clone()];
            let (is_ws, advance) =
                precise_cluster_advance(cluster_bytes, cluster.advance, char_width);
            if is_ws {
                x_cursor += advance;
                continue;
            }

            // Apply composition displacement before choosing the raster phase and pixel origin.
            let preedit_shift = if emission.preedit_on_cursor_line
                && cluster.byte_range.start >= ctx.preedit_cursor_col
            {
                ctx.preedit_advance_px
            } else {
                0.0
            };
            let (int_x, phase) = crate::text_rasterize::glyph_position(
                &mut text.shaper,
                font_id,
                x_cursor + cluster.x_offset + preedit_shift,
            );
            let Some(slot) = crate::text_rasterize::resolve_glyph(
                font_id,
                glyph_id,
                font_size,
                phase,
                &mut text.shaper,
                &mut text.atlas,
                &text.atlas_texture,
                &gpu.ctx.queue,
            ) else {
                x_cursor += advance;
                continue;
            };

            // Look up highlight color for this cluster's byte range.
            let color = if highlight_spans.is_empty() {
                ctx.theme.editor.foreground
            } else {
                highlight_color_for_offset(highlight_spans, cluster.byte_range.start, ctx.theme)
            };

            let verts = GlyphRenderer::generate_vertices(
                &[(slot, int_x, y_base)],
                ATLAS_SIZE,
                ATLAS_SIZE,
                ctx.screen_w,
                ctx.screen_h,
                color,
            );
            all_vertices.extend(verts);
            x_cursor += advance;
        }
    }
}

struct CacheLineCommit {
    doc_index: usize,
    doc_offset: usize,
    layout_hash: u64,
    atlas_generation: u64,
    scope: ShapedLineScope,
    metadata: RenderCacheMetadata,
}

fn cache_shaped_line(
    line: ShapedLineView<'_>,
    commit: CacheLineCommit,
    presentation: &mut DocumentPresentation,
    text: &mut TextState,
    ctx: &RenderContext<'_>,
) -> bool {
    let ShapedLineView {
        shaped,
        visual_lines,
        bytes: line_bytes,
        highlights: highlight_spans,
        char_width,
    } = line;
    let CacheLineCommit {
        doc_index: cache_doc_line,
        layout_hash: content_hash,
        atlas_generation,
        scope: shaped_line_scope,
        metadata: cache_metadata,
        ..
    } = commit;
    let mut all_instances: Vec<GlyphInstance> = Vec::new();
    let mut vl_instance_starts: Vec<usize> = Vec::new();

    for &(vl_start, vl_end, _vl_width) in visual_lines.iter() {
        vl_instance_starts.push(all_instances.len());
        let mut x_c = ctx.left_margin;

        for cluster in &shaped.clusters[vl_start..vl_end] {
            let c_font_id = cluster.font_id;
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            c_font_id.hash(&mut h);
            let c_fid = h.finish() as usize;
            let c_fs = text.shaper.font_size();

            let (c_ws, c_adv) = precise_cluster_advance(
                &line_bytes[cluster.byte_range.clone()],
                cluster.advance,
                char_width,
            );

            if c_ws {
                x_c += c_adv;
                continue;
            }

            let (int_x, phase) = crate::text_rasterize::glyph_position(
                &mut text.shaper,
                c_font_id,
                x_c + cluster.x_offset,
            );
            let c_key = GlyphKey {
                glyph_id: cluster.glyph_id,
                font_id: c_fid,
                font_size: (c_fs * 64.0) as u32,
                subpixel_phase: phase,
            };

            if let Some(c_slot) = text.atlas.get(&c_key) {
                let aw = ATLAS_SIZE as f32;
                let ah = ATLAS_SIZE as f32;
                all_instances.push(GlyphInstance {
                    x: int_x,
                    y: 0.0,
                    bearing_x: c_slot.bearing_x,
                    bearing_y: c_slot.bearing_y,
                    width: c_slot.width as f32,
                    height: c_slot.height as f32,
                    uv: [
                        c_slot.x as f32 / aw,
                        c_slot.y as f32 / ah,
                        (c_slot.x + c_slot.width) as f32 / aw,
                        (c_slot.y + c_slot.height) as f32 / ah,
                    ],
                    atlas_page: c_slot.page,
                    highlight_kind: if highlight_spans.is_empty() {
                        0
                    } else {
                        // Highlight spans: [(start0, kind0), (start1, kind1), ..., sentinel]
                        // A cluster belongs to the last span whose start <= byte_start.
                        highlight_spans
                            .iter()
                            .rev()
                            .find(|(start, _)| *start <= cluster.byte_range.start)
                            .map(|(_, kind)| *kind as u8)
                            .unwrap_or(0)
                    },
                });
            }
            x_c += c_adv;
        }
    }

    let cluster_data: Vec<_> = shaped
        .clusters
        .iter()
        .map(|c| {
            let (_is_ws, adv) =
                precise_cluster_advance(&line_bytes[c.byte_range.clone()], c.advance, char_width);
            (c.byte_range.start, c.byte_range.end, adv)
        })
        .collect();

    let mut tree_dirty = false;
    if shaped_line_scope == ShapedLineScope::FullLine {
        let breaks: smallvec::SmallVec<[crate::snap_tree::VisualBreak; 1]> = visual_lines
            .iter()
            .map(|&(start, end, width)| {
                let byte_start =
                    shaped.clusters.get(start).map(|c| c.byte_range.start as u32).unwrap_or(0);
                let byte_end_val = shaped
                    .clusters
                    .get(end.saturating_sub(1))
                    .map(|c| c.byte_range.end as u32)
                    .unwrap_or(0);
                crate::snap_tree::VisualBreak {
                    byte_start,
                    byte_end: byte_end_val,
                    pixel_width: width,
                }
            })
            .collect();
        let entry = crate::snap_tree::DisplayLineEntry {
            visual_line_count: cache_metadata.visual_line_count,
            visual_breaks: breaks,
            byte_offset: commit.doc_offset,
            byte_length: line_bytes.len() as u32,
            content_hash,
        };
        let old_vl_count = presentation
            .display
            .display_map
            .get_entry(cache_doc_line)
            .map(|e| e.visual_line_count)
            .unwrap_or(1);
        presentation.display.display_map.update_entry_in_place(cache_doc_line, entry);
        if old_vl_count != cache_metadata.visual_line_count {
            tree_dirty = true;
        }
    }

    presentation.display.render_cache.insert(
        cache_doc_line,
        CachedLine {
            instances: all_instances,
            line_number_glyphs: vec![],
            atlas_generation,
            visual_line_count: cache_metadata.visual_line_count,
            content_hash,
            visual_lines: visual_lines.to_vec(),
            visual_line_instance_starts: vl_instance_starts,
            cluster_data,
            subset_start: cache_metadata.subset_start,
        },
    );
    tree_dirty
}

fn update_frame_edge_lines(
    line: ShapedLineView<'_>,
    first_line_offset: Option<usize>,
    first_line: &mut LineCache,
    last_line: &mut LineCache,
) {
    last_line.visual_lines.clear();
    last_line.visual_lines.extend_from_slice(line.visual_lines);
    last_line.clusters.clear();
    last_line.clusters.extend(line.shaped.clusters.iter().map(|cluster| {
        let (_, advance) = line
            .bytes
            .get(cluster.byte_range.clone())
            .map(|bytes| precise_cluster_advance(bytes, cluster.advance, line.char_width))
            .unwrap_or((false, cluster.advance.max(0.0)));
        (cluster.byte_range.start, cluster.byte_range.end, advance)
    }));
    if let Some(doc_offset) = first_line_offset {
        first_line.visual_lines.clone_from(&last_line.visual_lines);
        first_line.clusters.clone_from(&last_line.clusters);
        first_line.doc_offset = doc_offset;
    }
}

fn update_cursor_from_shaped_line(
    document: &mut DocumentModel,
    cursor: &mut CursorRenderState,
    line: ShapedLineView<'_>,
    placement: LinePlacement,
    is_cursor_doc_line: bool,
    scope: ShapedLineScope,
    left_margin: f32,
) {
    let cursor_col = document.cursor_column();
    let bounds = line
        .visual_lines
        .iter()
        .map(|&(start, end, _)| {
            (
                line.shaped.clusters[start].byte_range.start,
                line.shaped.clusters[end - 1].byte_range.end,
            )
        })
        .collect::<Vec<_>>();
    let mut cursor_row = find_visual_line_index(&bounds, cursor_col);

    // After End, a cursor at a shared boundary stays on the previous visual row.
    if document.cursor().last_command_was_end
        && cursor_row > 0
        && cursor_col == bounds[cursor_row - 1].1
    {
        cursor_row -= 1;
    }
    if cursor_row >= placement.skipped_rows {
        if is_cursor_doc_line {
            let doc_row = match scope {
                ShapedLineScope::FullLine => cursor_row,
                ShapedLineScope::VisualSubset => cursor_row + placement.skipped_rows,
            };
            cursor.cursor_visual_line_in_doc = doc_row;
            cursor.cursor_visual_line =
                Some((placement.screen_row + doc_row).saturating_sub(placement.skipped_rows));
        }
        if let Some(cursor_x) = shaped_cursor_x(line, cursor_row, cursor_col, left_margin) {
            cursor.cursor_pixel_x = cursor_x;
        }
    }
    if is_cursor_doc_line && cursor.sticky_x_dirty {
        cursor.sticky_x = cursor.cursor_pixel_x;
        cursor.sticky_x_dirty = false;
    }
}

fn shaped_cursor_x(
    line: ShapedLineView<'_>,
    row: usize,
    byte_column: usize,
    left_margin: f32,
) -> Option<f32> {
    let &(start, end, _) = line.visual_lines.get(row)?;
    let first_byte = line.shaped.clusters[start].byte_range.start;
    let mut cluster_positions: smallvec::SmallVec<[(usize, f32, u32); 64]> =
        smallvec::SmallVec::new();
    let mut x = left_margin;
    for cluster in &line.shaped.clusters[start..end] {
        let (_, advance) = precise_cluster_advance(
            &line.bytes[cluster.byte_range.clone()],
            cluster.advance,
            line.char_width,
        );
        x += advance;
        cluster_positions.push((cluster.byte_range.end - first_byte, x, 0));
    }
    Some(ui::render_geom::byte_to_x(
        byte_column.saturating_sub(first_byte),
        &cluster_positions,
        left_margin,
        false,
    ))
}

pub fn shape_visible_lines(
    options: SourceLayoutOptions<'_>,
    document: &mut DocumentModel,
    presentation: &mut DocumentPresentation,
    text: &mut TextState,
    gpu: &GpuState,
    frame_cache: &mut FrameCache,
) -> ShapedVisibleLines {
    let mut frame = SourceRenderFrame::new(options, document, presentation, text, gpu, frame_cache);
    // Walk candidate document lines until their shaped rows fill the viewport.
    for relative_line in 0..frame.viewport.candidate_lines {
        let progress = frame.render_line(relative_line);
        if progress == LineProgress::CheckViewport
            && frame.viewport.is_filled_after(relative_line, frame.rendered_rows)
        {
            break;
        }
    }
    frame.finish()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineProgress {
    CheckViewport,
    KeepScanning,
}

struct SourceRenderFrame<'resources, 'options> {
    document: &'resources mut DocumentModel,
    presentation: &'resources mut DocumentPresentation,
    text: &'resources mut TextState,
    gpu: &'resources GpuState,
    frame_cache: &'resources mut FrameCache,
    options: SourceLayoutOptions<'options>,
    viewport: RenderViewportState,
    viewport_width: f32,
    rendered_rows: usize,
    vertices: Vec<GlyphVertex>,
    atlas_generation: u64,
    source_uses_verbatim_spacing: bool,
    tree_dirty: bool,
    timing: SourceShapeTiming,
}

impl<'resources, 'options> SourceRenderFrame<'resources, 'options> {
    fn new(
        options: SourceLayoutOptions<'options>,
        document: &'resources mut DocumentModel,
        presentation: &'resources mut DocumentPresentation,
        text: &'resources mut TextState,
        gpu: &'resources GpuState,
        frame_cache: &'resources mut FrameCache,
    ) -> Self {
        let atlas_generation = text.atlas_generation();
        presentation.display.render_cache.invalidate_stale_atlas(atlas_generation);
        let mut timing = SourceShapeTiming::new();
        let metrics = options.metrics;
        let ctx = options.render_context;
        let viewport_width =
            render_viewport_width(ctx.screen_w, ctx.left_margin, metrics, options.word_wrap);
        presentation.display.display_map.set_viewport_size(viewport_width, metrics.font_size);
        let viewport = resolve_visible_line_range(
            presentation,
            metrics.line_height,
            document.line_count(),
            document.cursor_line_cached(),
        );
        for entry in presentation.display.advance_cache.drain(..) {
            if !entry.clusters.is_empty() {
                frame_cache.cluster_pool.push(entry.clusters);
            }
        }
        if viewport.cursor_doc_line < viewport.start_doc {
            presentation.cursor_render_state.cursor_visual_line = None;
        }
        timing.setup_us = timing.started_at.elapsed().as_micros();
        let source_uses_verbatim_spacing = document_uses_verbatim_source_spacing(document);
        Self {
            document,
            presentation,
            text,
            gpu,
            frame_cache,
            options,
            viewport,
            viewport_width,
            rendered_rows: 0,
            vertices: Vec::new(),
            atlas_generation,
            source_uses_verbatim_spacing,
            tree_dirty: false,
            timing,
        }
    }

    fn render_line(&mut self, relative_line: usize) -> LineProgress {
        let doc_index = self.viewport.start_doc + relative_line;
        let doc_offset = self.document.line_byte_offset(doc_index).unwrap_or(0);
        let length = self.document.line_byte_length(doc_index);
        let placement = LinePlacement {
            doc_index,
            screen_row: self.rendered_rows,
            skipped_rows: if relative_line == 0 { self.viewport.skip_visual } else { 0 },
            sub_line_offset: self.viewport.sub_line_offset,
            active: line_contains_cursor_or_selection(
                self.document,
                doc_offset,
                length.unwrap_or(0),
            ),
        };
        let Some(length) = length else {
            self.emit_placeholder(placement, 1);
            return LineProgress::CheckViewport;
        };
        let bytes = self.document.doc_line_bytes(doc_index).map(|bytes| bytes.into_owned());
        let Some(bytes) = bytes.filter(|bytes| length > 0 && !bytes.is_empty()) else {
            let started_at = std::time::Instant::now();
            self.update_empty_line_cursor(placement, doc_offset);
            self.emit_placeholder(placement, 1);
            self.timing.empty_us += started_at.elapsed().as_micros();
            return LineProgress::CheckViewport;
        };

        let spacing = spacing_mode_for_line(
            self.options.text_spacing_mode,
            self.source_uses_verbatim_spacing,
            &bytes,
        );
        let preedit_on_cursor_line = self.options.render_context.preedit_advance_px > 0.0
            && self.viewport.cursor_doc_line == doc_index;
        let LineCacheLookup { layout_hash, protected_ranges, cached } =
            lookup_or_prepare_cached_line(
                LineCacheInput {
                    doc_index,
                    doc_offset,
                    bytes: &bytes,
                    spacing,
                    preedit_on_cursor_line,
                },
                self.presentation,
                self.viewport_width,
                self.options.metrics.font_size,
                self.text.shaper.font_family().unwrap_or("sans-serif"),
            );
        if let Some(cached) = cached
            && self.render_cached_line(&cached, placement, relative_line)
        {
            return LineProgress::CheckViewport;
        }

        self.render_uncached_line(UncachedLine {
            placement,
            relative_line,
            doc_offset,
            length,
            bytes: &bytes,
            spacing,
            layout_hash,
            protected_ranges: &protected_ranges,
            preedit_on_cursor_line,
        })
    }

    fn render_cached_line(
        &mut self,
        cached: &CachedLine,
        placement: LinePlacement,
        relative_line: usize,
    ) -> bool {
        let started_at = std::time::Instant::now();
        let rows_needed = self.viewport.viewport_visual_rows.saturating_sub(self.rendered_rows);
        let Some(emission) = emit_cached_line_and_gutter_vertices(
            cached,
            placement,
            self.options,
            rows_needed,
            self.text,
            self.gpu,
            &mut self.vertices,
        ) else {
            return false;
        };
        let left_margin = self.options.render_context.left_margin;
        self.presentation.display.advance_cache.extend(build_cached_advance_cache_entries(
            cached,
            placement.doc_index,
            emission.relative_skip,
            left_margin,
        ));
        if self.viewport.cursor_relative_line == Some(relative_line) {
            update_cursor_from_cached_line(
                self.document,
                &mut self.presentation.cursor_render_state,
                cached,
                placement,
                emission.relative_skip,
                left_margin,
            );
        }
        self.timing.cache_hits += 1;
        self.timing.hit_block_us += started_at.elapsed().as_micros();
        self.rendered_rows += emission.rows_advanced;
        true
    }

    fn render_uncached_line(&mut self, line: UncachedLine<'_>) -> LineProgress {
        let doc_index = line.placement.doc_index;
        let map_entry = self.presentation.display.display_map.get_entry(doc_index);
        let map_stats =
            map_entry.map(|entry| ShapeMissMapStats::from_entry(entry, line.layout_hash));
        let is_placeholder = map_stats.is_none_or(|stats| stats.is_placeholder);
        if is_placeholder && line.length > MAX_SYNC_SHAPE_BYTES {
            // Leave long, unresolved rows to background shaping.
            let rows = map_entry.map_or(1, |entry| usize::from(entry.visual_line_count));
            self.emit_placeholder(line.placement, rows);
            return LineProgress::CheckViewport;
        }

        self.timing.cache_misses += 1;
        let subset_started_at = std::time::Instant::now();
        let subset_entry =
            map_entry.filter(|_| !is_placeholder && line.length > MAX_SYNC_SHAPE_BYTES);
        let cursor_offset = subset_entry.and_then(|entry| {
            (self.viewport.cursor_relative_line == Some(line.relative_line))
                .then(|| {
                    cursor_relative_offset_for_line(
                        self.document.cursor().offset.to_usize(),
                        self.document.line_byte_offset(doc_index).unwrap_or(entry.byte_offset),
                    )
                })
                .flatten()
        });
        let plan = LineShapePlan::resolve(
            line.bytes,
            subset_entry,
            line.layout_hash,
            line.placement.skipped_rows,
            self.viewport.viewport_visual_rows.saturating_sub(self.rendered_rows),
            cursor_offset,
        );
        self.timing.highlight_us += subset_started_at.elapsed().as_micros();
        let shape_started_at = std::time::Instant::now();

        let highlights = highlights_for_line(self.document, self.presentation, doc_index)
            .iter()
            .map(|highlight| (highlight.start, highlight.kind))
            .collect::<Vec<_>>();
        let has_font_overrides = highlights.iter().any(|&(_, kind)| {
            matches!(
                kind,
                HighlightKind::MarkupBold
                    | HighlightKind::MarkupItalic
                    | HighlightKind::MarkupHeading
            )
        });
        let display_breaks = plan.visible_breaks.as_deref().or_else(|| {
            self.presentation
                .display
                .display_map
                .get_entry(doc_index)
                .and_then(|entry| full_line_display_breaks(entry, line.layout_hash))
        });
        let Ok(shape) = shape_and_wrap_line(
            &mut self.text.shaper,
            SourceLineShapeOptions {
                text: plan.text,
                bytes: line.bytes,
                cluster_byte_offset: plan.cluster_byte_offset,
                highlights: &highlights,
                has_font_overrides,
                display_breaks,
                spacing: line.spacing,
                protected_ranges: line.protected_ranges,
                viewport_width: self.viewport_width,
            },
        ) else {
            return LineProgress::KeepScanning;
        };
        let LineShape::Wrapped { shaped, visual_lines, char_width } = shape else {
            self.emit_placeholder(line.placement, 1);
            return LineProgress::KeepScanning;
        };
        let shaped_line = ShapedLineView {
            shaped: &shaped,
            visual_lines: &visual_lines,
            bytes: line.bytes,
            highlights: &highlights,
            char_width,
        };
        self.render_fresh_line(
            line,
            shaped_line,
            &plan,
            ShapeMissTiming { started_at: shape_started_at, map_stats, has_font_overrides },
        );
        LineProgress::CheckViewport
    }

    fn render_fresh_line(
        &mut self,
        line: UncachedLine<'_>,
        shaped: ShapedLineView<'_>,
        plan: &LineShapePlan<'_>,
        timing: ShapeMissTiming,
    ) {
        let skip_visual = if plan.is_subset() { 0 } else { line.placement.skipped_rows };
        let skip_visual = skip_visual.min(shaped.visual_lines.len());
        update_frame_edge_lines(
            shaped,
            (line.relative_line == 0).then_some(line.doc_offset),
            &mut self.frame_cache.first_line,
            &mut self.frame_cache.last_line,
        );

        let cursor_doc_line = self.document.cursor_line_cached();
        let range = self.presentation.display.viewport.visible_doc_range_from_anchor(
            &self.presentation.display.display_map,
            self.options.metrics.line_height,
        );
        let left_margin = self.options.render_context.left_margin;
        if range.contains(&cursor_doc_line) && cursor_doc_line - range.start == line.relative_line {
            update_cursor_from_shaped_line(
                self.document,
                &mut self.presentation.cursor_render_state,
                shaped,
                line.placement,
                cursor_doc_line == line.placement.doc_index,
                plan.scope(),
                left_margin,
            );
        }

        let projected_doc_index = range.start + line.relative_line;
        self.presentation.display.advance_cache.extend(build_advance_cache_entries(
            shaped.visual_lines,
            skip_visual,
            shaped.shaped,
            shaped.bytes,
            shaped.char_width,
            projected_doc_index,
            &mut self.frame_cache.cluster_pool,
            left_margin,
        ));

        let elapsed_us = timing.started_at.elapsed().as_micros();
        self.timing.shape_us += elapsed_us;
        timing.log_if_slow(line, plan, elapsed_us);

        let started_at = std::time::Instant::now();
        let show_line_number =
            skip_visual == 0 && (!plan.is_subset() || line.placement.skipped_rows == 0);
        emit_line_and_gutter_vertices(
            FreshLineEmission {
                line: shaped,
                placement: LinePlacement {
                    doc_index: projected_doc_index,
                    skipped_rows: skip_visual,
                    ..line.placement
                },
                show_line_number,
                preedit_on_cursor_line: self.options.render_context.preedit_advance_px > 0.0
                    && projected_doc_index == cursor_doc_line,
            },
            self.options,
            self.text,
            self.gpu,
            &mut self.vertices,
        );
        self.rendered_rows += match plan.scope() {
            ShapedLineScope::FullLine => shaped.visual_lines.len().saturating_sub(skip_visual),
            ShapedLineScope::VisualSubset => {
                plan.doc_rows.saturating_sub(line.placement.skipped_rows)
            }
        };
        self.timing.vertex_us += started_at.elapsed().as_micros();
        self.cache_fresh_line(line, shaped, plan, skip_visual);
    }

    fn cache_fresh_line(
        &mut self,
        line: UncachedLine<'_>,
        shaped: ShapedLineView<'_>,
        plan: &LineShapePlan<'_>,
        skip_visual: usize,
    ) {
        let doc_index = line.placement.doc_index;
        let cache_exists = self.presentation.display.render_cache.contains(doc_index);
        if !should_store_shaped_line_in_render_cache(
            plan.scope(),
            line.preedit_on_cursor_line,
            cache_exists,
        ) {
            return;
        }
        let metadata = render_cache_metadata_for_shaped_line(
            plan.scope(),
            shaped.visual_lines.len(),
            visual_line_count_from_len(plan.doc_rows),
            skip_visual,
            plan.start_row,
        );
        self.tree_dirty |= cache_shaped_line(
            shaped,
            CacheLineCommit {
                doc_index,
                doc_offset: line.doc_offset,
                layout_hash: line.layout_hash,
                atlas_generation: self.atlas_generation,
                scope: plan.scope(),
                metadata,
            },
            self.presentation,
            self.text,
            self.options.render_context,
        );
    }

    fn emit_placeholder(&mut self, placement: LinePlacement, rows: usize) {
        let ctx = self.options.render_context;
        let metrics = self.options.metrics;
        if ctx.gutter_width > 0.0 && placement.skipped_rows == 0 {
            emit_gutter_line_number(
                metrics,
                ctx,
                self.text,
                self.gpu,
                GutterLine {
                    doc_index: placement.doc_index,
                    top: placement.screen_row as f32 * metrics.line_height
                        + placement.sub_line_offset,
                    active: placement.active,
                },
                &mut self.vertices,
            );
        }
        self.presentation.display.advance_cache.push(AdvanceCacheEntry {
            doc_line: placement.doc_index,
            vl_byte_start: 0,
            vl_grapheme_start: 0,
            clusters: Vec::new(),
        });
        self.rendered_rows += rows;
    }

    fn update_empty_line_cursor(&mut self, placement: LinePlacement, doc_offset: usize) {
        let cursor = &mut self.presentation.cursor_render_state;
        if self.viewport.cursor_doc_line == placement.doc_index {
            cursor.cursor_visual_line = Some(placement.screen_row);
            cursor.cursor_visual_line_in_doc = 0;
        }
        if self.document.cursor().offset.to_usize() == doc_offset {
            cursor.cursor_pixel_x = self.options.render_context.left_margin;
            if cursor.sticky_x_dirty {
                cursor.sticky_x = cursor.cursor_pixel_x;
                cursor.sticky_x_dirty = false;
            }
        }
    }

    fn finish(self) -> ShapedVisibleLines {
        if self.tree_dirty {
            self.presentation.display.display_map.rebuild_tree();
        }
        self.timing.log_if_slow();
        ShapedVisibleLines { vertices: self.vertices, tree_dirty: self.tree_dirty }
    }
}

fn line_contains_cursor_or_selection(
    document: &DocumentModel,
    start: usize,
    length: usize,
) -> bool {
    let end = start + length;
    let cursor = document.cursor().offset.to_usize();
    if (start..=end).contains(&cursor) {
        return true;
    }
    document
        .cursor()
        .selection_anchor
        .is_some_and(|anchor| start <= anchor.max(cursor) && end >= anchor.min(cursor))
}

#[derive(Clone, Copy)]
struct UncachedLine<'a> {
    placement: LinePlacement,
    relative_line: usize,
    doc_offset: usize,
    length: usize,
    bytes: &'a [u8],
    spacing: TextSpacingMode,
    layout_hash: u64,
    protected_ranges: &'a [Range<usize>],
    preedit_on_cursor_line: bool,
}

struct LineShapePlan<'a> {
    text: &'a str,
    cluster_byte_offset: usize,
    visible_breaks: Option<Vec<crate::snap_tree::VisualBreak>>,
    doc_rows: usize,
    start_row: usize,
    end_row: usize,
    visible_rows_needed: usize,
    cursor_rows_needed: usize,
}

impl<'a> LineShapePlan<'a> {
    fn resolve(
        bytes: &'a [u8],
        entry: Option<&crate::snap_tree::DisplayLineEntry>,
        layout_hash: u64,
        skipped_rows: usize,
        visible_rows: usize,
        cursor_offset: Option<u32>,
    ) -> Self {
        let mut plan = Self {
            text: std::str::from_utf8(bytes).unwrap_or_default(),
            cluster_byte_offset: 0,
            visible_breaks: None,
            doc_rows: 1,
            start_row: 0,
            end_row: 0,
            visible_rows_needed: 0,
            cursor_rows_needed: 0,
        };
        let Some(entry) = entry
            .filter(|entry| entry.content_hash == layout_hash && !entry.visual_breaks.is_empty())
        else {
            return plan;
        };

        let breaks = &entry.visual_breaks;
        let cursor_rows = cursor_offset.map_or(0, |offset| {
            let cursor_row =
                breaks.partition_point(|item| item.byte_start <= offset).saturating_sub(1);
            cursor_row.checked_sub(skipped_rows).map_or(0, |row| row + 1)
        });
        let end_row = (skipped_rows + visible_rows.max(cursor_rows) + 1).min(breaks.len());
        plan.doc_rows = breaks.len();
        plan.start_row = skipped_rows;
        plan.end_row = end_row;
        plan.visible_rows_needed = visible_rows;
        plan.cursor_rows_needed = cursor_rows;
        if skipped_rows < end_row {
            let start_byte = breaks[skipped_rows].byte_start as usize;
            let end_byte = breaks[end_row - 1].byte_end as usize;
            plan.text = std::str::from_utf8(&bytes[start_byte..end_byte]).unwrap_or_default();
            plan.cluster_byte_offset = start_byte;
            plan.visible_breaks = Some(breaks[skipped_rows..end_row].to_vec());
        }
        plan
    }

    fn is_subset(&self) -> bool {
        self.visible_breaks.is_some()
    }

    fn scope(&self) -> ShapedLineScope {
        ShapedLineScope::from_subset_flag(self.is_subset())
    }
}

struct ShapeMissTiming {
    started_at: std::time::Instant,
    map_stats: Option<ShapeMissMapStats>,
    has_font_overrides: bool,
}

impl ShapeMissTiming {
    fn log_if_slow(self, line: UncachedLine<'_>, plan: &LineShapePlan<'_>, elapsed_us: u128) {
        if !should_perf_log(elapsed_us) {
            return;
        }
        let stats = self.map_stats;
        eprintln!(
            "[perf:shape_miss] doc_line={} line_len={} shape={}us subset={} subset_len={} subset_vl={}..{} visible_needed={} cursor_needed={} entry={} hash_match={} breaks={} entry_vl={} entry_offset={} entry_len={} placeholder={} has_style={} doc_vl_count={}",
            line.placement.doc_index,
            line.length,
            elapsed_us,
            plan.is_subset(),
            plan.text.len(),
            plan.start_row,
            plan.end_row,
            plan.visible_rows_needed,
            plan.cursor_rows_needed,
            stats.is_some(),
            stats.is_some_and(|item| item.hash_matches),
            stats.map_or(0, |item| item.visual_break_count),
            stats.map_or(0, |item| item.visual_line_count),
            stats.map_or(0, |item| item.byte_offset),
            stats.map_or(0, |item| item.byte_length),
            stats.is_none_or(|item| item.is_placeholder),
            self.has_font_overrides,
            plan.doc_rows,
        );
    }
}

struct SourceShapeTiming {
    started_at: std::time::Instant,
    setup_us: u128,
    cache_hits: usize,
    cache_misses: usize,
    hit_block_us: u128,
    empty_us: u128,
    highlight_us: u128,
    shape_us: u128,
    vertex_us: u128,
}

impl SourceShapeTiming {
    fn new() -> Self {
        Self {
            started_at: std::time::Instant::now(),
            setup_us: 0,
            cache_hits: 0,
            cache_misses: 0,
            hit_block_us: 0,
            empty_us: 0,
            highlight_us: 0,
            shape_us: 0,
            vertex_us: 0,
        }
    }

    fn log_if_slow(self) {
        let elapsed = self.started_at.elapsed().as_micros();
        if should_perf_log(elapsed) {
            eprintln!(
                "[perf:shape] total={}us setup={}us hits={} misses={} hit_block={}us empty={}us hl={}us shape={}us vtx={}us",
                elapsed,
                self.setup_us,
                self.cache_hits,
                self.cache_misses,
                self.hit_block_us,
                self.empty_us,
                self.highlight_us,
                self.shape_us,
                self.vertex_us,
            );
        }
    }
}

/// Compute cursor pixel_x from cached cluster data.
/// Iterates over clusters, accumulating advances for clusters that end
/// at or before cursor_col. Stops when a cluster's byte_end exceeds cursor_col.
fn compute_cursor_pixel_x_cached(
    cluster_data: &[(usize, usize, f32)],
    vl_start: usize,
    vl_end: usize,
    cursor_col: usize,
    cluster_end: usize,
    left_margin: f32,
) -> Option<f32> {
    if cursor_col > cluster_end {
        return None;
    }
    let mut x = left_margin;
    for cd in &cluster_data[vl_start..vl_end] {
        if cd.1 > cursor_col {
            break;
        }
        x += cd.2;
    }
    Some(x)
}

#[cfg(test)]
// Tests live in a separate file to keep this module readable.
#[path = "render_pipeline_tests.rs"]
mod tests;

/// Generate vertices for IME preedit (composing) text rendered at the cursor position.
pub fn preedit_text_vertices(
    metrics: &ui::settings::UiMetrics,
    preedit_text: &str,
    cursor_x_px: f32,
    cursor_y_px: f32,
    text: &mut TextState,
    gpu: &GpuState,
    screen_w: f32,
    screen_h: f32,
    color: [f32; 4],
) -> Vec<GlyphVertex> {
    use crate::render_state::ATLAS_SIZE;

    let font_size = metrics.font_size;
    let old_font_size = text.shaper.font_size();
    text.shaper.set_font_size(font_size);

    let shaped_result = text.shaper.shape(preedit_text);
    text.shaper.set_font_size(old_font_size);

    let shaped = match shaped_result {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let mut vertices = Vec::new();
    let mut x_cursor = cursor_x_px;
    let y_base = cursor_y_px + metrics.line_height * ui::constants::BASELINE_RATIO;

    // Underline color (same as text but slightly dimmed)
    let underline_color = [color[0], color[1], color[2], color[3] * 0.75];

    for cluster in &shaped.clusters {
        let glyph_id = cluster.glyph_id as u16;
        let font_id = cluster.font_id;
        let advance = cluster.advance.max(0.0);

        let (int_x, phase) = crate::text_rasterize::glyph_position(
            &mut text.shaper,
            font_id,
            x_cursor + cluster.x_offset,
        );
        let Some(slot) = crate::text_rasterize::resolve_glyph(
            font_id,
            glyph_id,
            font_size,
            phase,
            &mut text.shaper,
            &mut text.atlas,
            &text.atlas_texture,
            &gpu.ctx.queue,
        ) else {
            x_cursor += advance;
            continue;
        };

        let left = (int_x + slot.bearing_x) / screen_w * 2.0 - 1.0;
        let top = 1.0 - (y_base - slot.bearing_y) / screen_h * 2.0;
        let right = (int_x + slot.bearing_x + slot.width as f32) / screen_w * 2.0 - 1.0;
        let bottom = 1.0 - (y_base - slot.bearing_y + slot.height as f32) / screen_h * 2.0;
        let uv_left = slot.x as f32 / ATLAS_SIZE as f32;
        let uv_top = slot.y as f32 / ATLAS_SIZE as f32;
        let uv_right = (slot.x + slot.width) as f32 / ATLAS_SIZE as f32;
        let uv_bottom = (slot.y + slot.height) as f32 / ATLAS_SIZE as f32;

        // Glyph vertices
        vertices.push(GlyphVertex { position: [left, top], tex_coords: [uv_left, uv_top], color });
        vertices.push(GlyphVertex {
            position: [right, top],
            tex_coords: [uv_right, uv_top],
            color,
        });
        vertices.push(GlyphVertex {
            position: [left, bottom],
            tex_coords: [uv_left, uv_bottom],
            color,
        });
        vertices.push(GlyphVertex {
            position: [right, top],
            tex_coords: [uv_right, uv_top],
            color,
        });
        vertices.push(GlyphVertex {
            position: [right, bottom],
            tex_coords: [uv_right, uv_bottom],
            color,
        });
        vertices.push(GlyphVertex {
            position: [left, bottom],
            tex_coords: [uv_left, uv_bottom],
            color,
        });

        // Underline vertices (at baseline, 2px thick)
        let ul_thickness = 2.0 / screen_h * 2.0;
        let baseline_ndc = 1.0 - y_base / screen_h * 2.0;
        let ul_top = baseline_ndc + ul_thickness * 0.5;
        let ul_bottom = baseline_ndc - ul_thickness * 0.5;

        vertices.push(GlyphVertex {
            position: [left, ul_top],
            tex_coords: [0.0, 0.0],
            color: underline_color,
        });
        vertices.push(GlyphVertex {
            position: [right, ul_top],
            tex_coords: [0.0, 0.0],
            color: underline_color,
        });
        vertices.push(GlyphVertex {
            position: [left, ul_bottom],
            tex_coords: [0.0, 0.0],
            color: underline_color,
        });
        vertices.push(GlyphVertex {
            position: [right, ul_top],
            tex_coords: [0.0, 0.0],
            color: underline_color,
        });
        vertices.push(GlyphVertex {
            position: [right, ul_bottom],
            tex_coords: [0.0, 0.0],
            color: underline_color,
        });
        vertices.push(GlyphVertex {
            position: [left, ul_bottom],
            tex_coords: [0.0, 0.0],
            color: underline_color,
        });

        x_cursor += advance;
    }

    vertices
}
