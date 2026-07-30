//! Render pipeline: shape visible lines, generate vertices.
//!
//! Extracts the heavy shaping/rendering logic from App.

use crate::cursor_motion::{LineCache, find_visual_line_index};
use crate::document_presentation::DocumentPresentation;
use crate::render_cache::{CachedLine, GlyphInstance};
use crate::render_state::{GpuState, TextState};
use appkit_core::content_hash;
use appkit_core::document::DocumentModel;
use core::highlight::HighlightKind;
use render::GlyphVertex;
use ui::decorations::*;
use ui::gutter::ATLAS_SIZE;
use ui::gutter::*;
use ui::layout::*;
use ui::render_geom::AdvanceCacheEntry;

use render::{GlyphKey, GlyphRenderer};

const PERF_LOG_ENV: &str = "EDIT_PLUS_PERF_LOG";
const PERF_LOG_THRESHOLD_US_ENV: &str = "EDIT_PLUS_PERF_LOG_THRESHOLD_US";
const DEFAULT_PERF_LOG_THRESHOLD_US: u128 = 1_000;

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
}

fn visual_line_count_from_len(visual_line_count: usize) -> u16 {
    visual_line_count.min(u16::MAX as usize) as u16
}

fn compute_render_viewport_state_from_presentation(
    dv: &DocumentModel,
    presentation: &DocumentPresentation,
    line_height: f32,
) -> RenderViewportState {
    let anchor = presentation.display.viewport.scroll_anchor;
    let skip_visual = (anchor.pixel_offset / line_height) as usize;
    let sub_line_offset = -(anchor.pixel_offset - skip_visual as f32 * line_height);
    let total_lines = dv.line_count();
    let start_doc = anchor.doc_line.min(total_lines.saturating_sub(1));
    let viewport_visual_rows = presentation.display.viewport.viewport_height.ceil() as usize + 2;

    RenderViewportState { skip_visual, sub_line_offset, start_doc, viewport_visual_rows }
}

fn highlights_for_line<'a>(
    document: &DocumentModel,
    presentation: &'a mut DocumentPresentation,
    line_index: usize,
) -> &'a [core::highlight::Highlight<HighlightKind>] {
    use core::highlight::Highlighter as CoreHighlighter;
    use stdext::arena::scratch_arena;

    let Some(language) = document.language else {
        return &[];
    };

    let arena = scratch_arena(None);
    let mut highlighter = CoreHighlighter::new(&document.tb, language);
    presentation.highlighter_cache.parse_line(
        &arena,
        &mut highlighter,
        line_index as core::helpers::CoordType,
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

/// Render a line number for a doc line that has no shaped text content,
/// so that placeholder rows still show a line number instead of a ghost gap.
fn render_line_number_placeholder(
    metrics: &ui::settings::UiMetrics,
    ctx: &RenderContext,
    text: &mut TextState,
    gpu: &GpuState,
    is_active_line: bool,
    doc_line_idx: usize,
    visual_line: usize,
    all_vertices: &mut Vec<GlyphVertex>,
    sub_line_offset: f32,
) {
    let _line_num_buf = format_line_num(doc_line_idx + 1);
    let line_num_str = line_num_str(&_line_num_buf);
    if let Ok(line_num_shaped) = text.shaper.shape(line_num_str) {
        let ln_y = visual_line as f32 * metrics.line_height + sub_line_offset;
        let ln_font_size = text.shaper.font_size() * ui::constants::LN_FONT_SCALE;
        let ln_verts = generate_line_number_vertices(
            ctx,
            &line_num_shaped,
            &mut text.atlas,
            &text.atlas_texture,
            &gpu.ctx.queue,
            &mut text.shaper,
            ln_font_size,
            ln_y,
            is_active_line,
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

pub fn shape_visible_lines(
    metrics: &ui::settings::UiMetrics,
    min_punctuation_width_ratio: f32,
    ctx: &ui::gutter::RenderContext,
    dv: &mut DocumentModel,
    presentation: &mut DocumentPresentation,
    text: &mut TextState,
    gpu: &GpuState,
    advance_cache: &mut Vec<AdvanceCacheEntry>,
    cluster_pool: &mut Vec<Vec<(usize, f32, u32)>>,
    first_line: &mut LineCache,
    last_line: &mut LineCache,
    tree_dirty: &mut bool,
    word_wrap: bool,
) -> Vec<GlyphVertex> {
    let mut _cache_hits = 0usize;
    let mut _cache_misses = 0usize;
    // ── detailed perf probes ──
    let mut _wi_setup_us = 0u128;
    let mut _wi_skip_us = 0u128;
    let mut _vertex_hit_us = 0u128;
    let mut _vertex_miss_us = 0u128;
    let mut _rc_populate_us = 0u128;
    let mut _ln_shape_us = 0u128;
    let mut _cursor_us = 0u128;
    let mut _skip_lines = 0usize;
    let mut _shaped_lines = 0usize;
    let mut _hit_lines = 0usize;
    let mut _doc_read_us = 0u128;
    let mut _dm_check_us = 0u128;
    let mut _dm_sync_us = 0u128;
    let mut _tree_break_hits = 0usize;
    let mut _vtx_atlas_hits = 0usize;
    let mut _vtx_skip_raster = 0usize;
    let mut _vtx_raster_actual = 0usize;
    let mut _hl_nonzero = 0usize;

    let mut _lines_visited = 0usize;
    let mut _atlas_get_us = 0u128;
    let mut _font_hash_us = 0u128;
    // ── end probes ──
    let _perf_t0 = std::time::Instant::now();
    let mut _perf_cache_hits = 0usize;
    let mut _perf_cache_misses = 0usize;
    let mut _perf_hl_time_us = 0u128;
    let mut _perf_shape_time_us = 0u128;
    let mut _perf_vertex_time_us = 0u128;
    let mut _perf_rc_populate_us = 0u128;

    // Notify WrapIndex of viewport width for dirty tracking
    presentation.display.display_map.set_viewport_size(
        render_viewport_width(ctx.screen_w, ctx.left_margin, metrics, word_wrap),
        metrics.font_size,
    );

    let lh = metrics.line_height;
    let viewport_state = compute_render_viewport_state_from_presentation(dv, presentation, lh);
    let skip_visual = viewport_state.skip_visual;
    let sub_line_offset = viewport_state.sub_line_offset;

    // Drain cluster Vecs into pool for reuse, then clear advance cache
    for entry in advance_cache.drain(..) {
        if !entry.clusters.is_empty() {
            cluster_pool.push(entry.clusters);
        }
    }

    let mut all_vertices = Vec::new();
    let mut visual_line_counter: usize = 0;
    let mut _miss_count: usize = 0;
    let mut tree_dirty_local = false;

    // Start doc line from anchor — while loop renders until viewport is full,
    // not limited by a pre-computed range based on placeholder VL estimates.
    let total_lines = dv.line_count();
    let start_doc = viewport_state.start_doc;

    let cursor_doc_line_pre = dv.cursor_line_cached();
    if cursor_doc_line_pre < start_doc {
        presentation.cursor_render_state.cursor_visual_line = None;
    }

    let vp_height = viewport_state.viewport_visual_rows;

    // Cursor line relative index — bound generously so the break guard still
    // fires when cursor is far below the viewport (avoids wasted iterations).
    let cursor_rel_line = if cursor_doc_line_pre >= start_doc
        && cursor_doc_line_pre.saturating_sub(start_doc) < vp_height * 4
    {
        Some(cursor_doc_line_pre - start_doc)
    } else {
        None
    };

    let mut _wi_key_us = 0u128;
    let _perf_setup_us = _perf_t0.elapsed().as_micros();
    let mut _perf_hit_block_us = 0u128;
    let mut _perf_empty_us = 0u128;
    // Iterate from start_doc to end of doc, break when viewport fills.
    // Bound is the entire remaining doc (not a pre-computed range based on
    // placeholder VL estimates) so that the loop never starves before the
    // viewport is full.
    let bound = total_lines.saturating_sub(start_doc);
    for i in 0..bound {
        let doc_idx = start_doc + i;
        let is_active_line = {
            let mut active = false;
            let line_offset = dv.line_byte_offset(doc_idx).unwrap_or(0);
            let line_len = dv.line_byte_length(doc_idx).unwrap_or(0);
            let line_end = line_offset + line_len;

            if dv.cursor().offset.to_usize() >= line_offset
                && dv.cursor().offset.to_usize() <= line_end
            {
                active = true;
            }
            if let Some(anchor) = dv.cursor().selection_anchor {
                let sel_start = anchor.min(dv.cursor().offset.to_usize());
                let sel_end = anchor.max(dv.cursor().offset.to_usize());
                if line_offset <= sel_end && line_end >= sel_start {
                    active = true;
                }
            }
            active
        };
        let (_offset, length) = if doc_idx < total_lines {
            (dv.line_byte_offset(doc_idx), dv.line_byte_length(doc_idx))
        } else {
            (None, None)
        };
        let length = if let Some(l) = length {
            l
        } else {
            // Line out of range — render placeholder to avoid ghost gaps.
            let doc_line_idx = doc_idx;
            let sub_line_offset = sub_line_offset;
            if ctx.gutter_width > 0.0 && (i > 0 || skip_visual == 0) {
                render_line_number_placeholder(
                    metrics,
                    ctx,
                    text,
                    gpu,
                    is_active_line,
                    doc_line_idx,
                    visual_line_counter,
                    &mut all_vertices,
                    sub_line_offset,
                );
            }
            advance_cache.push(AdvanceCacheEntry {
                doc_line: doc_line_idx,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: Vec::new(),
            });
            visual_line_counter += 1;
            let cursor_line_done = cursor_rel_line.is_none() || i >= cursor_rel_line.unwrap();
            if visual_line_counter >= vp_height && cursor_line_done {
                break;
            }
            continue;
        };
        // Fetch line bytes early for unified empty-line detection
        let doc_line_idx = doc_idx;
        let line_bytes_early = dv.doc_line_bytes(doc_line_idx).map(|c| c.into_owned());

        // Unified empty-line check: covers length==0, bytes-empty, and edge cases
        let is_empty = length == 0 || line_bytes_early.as_ref().is_none_or(|b| b.is_empty());

        if is_empty {
            let _empty_t0 = std::time::Instant::now();
            let doc_offset = dv.line_byte_offset(doc_line_idx).unwrap_or(0);
            if cursor_doc_line_pre == doc_line_idx {
                presentation.cursor_render_state.cursor_visual_line = Some(visual_line_counter);
                presentation.cursor_render_state.cursor_visual_line_in_doc = 0;
            }
            if dv.cursor().offset.to_usize() == doc_offset {
                presentation.cursor_render_state.cursor_pixel_x = ctx.left_margin;
                if presentation.cursor_render_state.sticky_x_dirty {
                    presentation.cursor_render_state.sticky_x =
                        presentation.cursor_render_state.cursor_pixel_x;
                    presentation.cursor_render_state.sticky_x_dirty = false;
                }
            }
            if ctx.gutter_width > 0.0 && (i > 0 || skip_visual == 0) {
                render_line_number_placeholder(
                    metrics,
                    ctx,
                    text,
                    gpu,
                    is_active_line,
                    doc_line_idx,
                    visual_line_counter,
                    &mut all_vertices,
                    sub_line_offset,
                );
            }
            advance_cache.push(AdvanceCacheEntry {
                doc_line: doc_line_idx,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: Vec::new(),
            });
            visual_line_counter += 1;
            _perf_empty_us += _empty_t0.elapsed().as_micros();
            let cursor_line_done = cursor_rel_line.is_none() || i >= cursor_rel_line.unwrap();
            if visual_line_counter >= vp_height && cursor_line_done {
                break;
            }
            continue;
        }

        // ── Check RenderCache ──
        // Fast content_hash: use byte offset + length (stable even without reading bytes)
        let viewport_width =
            render_viewport_width(ctx.screen_w, ctx.left_margin, metrics, word_wrap);
        let content_hash_fast = {
            let off = dv.line_byte_offset(doc_line_idx).unwrap_or(0);
            let len = dv.line_byte_length(doc_line_idx).unwrap_or(0);
            content_hash::content_hash(off, len as u32, viewport_width, metrics.font_size)
        };
        // IME preedit: skip cache for cursor line (positions need shifting)
        let preedit_on_cursor_line =
            ctx.preedit_advance_px > 0.0 && doc_line_idx == cursor_doc_line_pre;
        let cache_hit = if preedit_on_cursor_line {
            None
        } else {
            presentation
                .display
                .render_cache
                .get(doc_line_idx)
                .filter(|c| {
                    c.content_hash == content_hash_fast
                        && c.visual_lines.len() == c.visual_line_instance_starts.len()
                })
                .cloned()
        };
        // Evict stale entries (hash mismatch e.g. font size changed) so the
        // populate path below can re-insert with the current shaping params.
        if cache_hit.is_none() && !preedit_on_cursor_line {
            presentation.display.render_cache.invalidate(doc_line_idx);
        }
        if let Some(cached) = cache_hit {
            let skip_vl_c = if i == 0 { skip_visual } else { 0 };
            let vl_needed_for_cache = vp_height.saturating_sub(visual_line_counter);

            let is_full_line = cached.subset_start == 0
                && cached.visual_lines.len() == cached.visual_line_count as usize;
            let is_perfect_subset = skip_vl_c == cached.subset_start
                && cached.visual_lines.len() >= vl_needed_for_cache;

            if is_full_line || is_perfect_subset {
                _perf_cache_hits += 1;
                let _hit_block_t0 = std::time::Instant::now();
                // Cache hit: emit vertices from GlyphInstances
                let sub_line_offset_c = sub_line_offset;
                let relative_skip = skip_vl_c.saturating_sub(cached.subset_start);

                // IME preedit: compute cursor x threshold for cache-hit path
                let preedit_cursor_vl_and_x: Option<(usize, f32)> = if ctx.preedit_advance_px > 0.0
                    && doc_line_idx == cursor_doc_line_pre
                {
                    let cursor_col = dv.cursor_column();
                    let bounds_c: Vec<(usize, usize)> = cached
                        .visual_lines
                        .iter()
                        .map(|&(vs, ve, _)| {
                            let bs =
                                cached.cluster_data.get(vs).map(|cd| cd.0).unwrap_or(usize::MAX);
                            let be = if ve > 0 {
                                cached.cluster_data.get(ve - 1).map(|cd| cd.1).unwrap_or(usize::MAX)
                            } else {
                                usize::MAX
                            };
                            (bs, be)
                        })
                        .collect();
                    let cvl = find_visual_line_index(&bounds_c, cursor_col);
                    let (cs, ce, _) = cached.visual_lines[cvl];
                    let cluster_end =
                        cached.cluster_data.get(ce.saturating_sub(1)).map(|cd| cd.1).unwrap_or(0);
                    let cx = compute_cursor_pixel_x_cached(
                        &cached.cluster_data,
                        cs,
                        ce,
                        cursor_col,
                        cluster_end,
                        ctx.left_margin,
                    )
                    .unwrap_or(ctx.left_margin);
                    Some((cvl, cx))
                } else {
                    None
                };

                for vl_idx in 0..cached.visual_lines.len() {
                    if vl_idx < relative_skip {
                        continue;
                    }
                    let line_y_c = (visual_line_counter + vl_idx - relative_skip) as f32
                        * metrics.line_height
                        + sub_line_offset_c;
                    // IME preedit: determine shift for this visual line
                    let preedit_shift = preedit_cursor_vl_and_x.and_then(|(cvl, cx)| {
                        if vl_idx == cvl {
                            // Cursor's visual line: shift instances at/after cursor x
                            Some((cx, ctx.preedit_advance_px))
                        } else if vl_idx > cvl {
                            // Visual lines after cursor: shift all instances
                            Some((0.0, ctx.preedit_advance_px))
                        } else {
                            None
                        }
                    });
                    let verts = cached.emit_vertices_for_visual_line(
                        vl_idx,
                        line_y_c,
                        metrics.line_height,
                        ctx.tab_bar_height,
                        ctx.screen_w,
                        ctx.screen_h,
                        ctx.theme.editor.foreground,
                        ctx.theme,
                        preedit_shift,
                    );
                    all_vertices.extend(verts);

                    // Line number for first VL
                    if vl_idx == 0 && ctx.gutter_width > 0.0 && skip_vl_c == 0 {
                        let _line_num_buf = format_line_num(doc_line_idx + 1);
                        let line_num_str = line_num_str(&_line_num_buf);
                        if let Ok(line_num_shaped) = text.shaper.shape(line_num_str) {
                            let ln_font_size =
                                text.shaper.font_size() * ui::constants::LN_FONT_SCALE;
                            let ln_verts = generate_line_number_vertices(
                                ctx,
                                &line_num_shaped,
                                &mut text.atlas,
                                &text.atlas_texture,
                                &gpu.ctx.queue,
                                &mut text.shaper,
                                ln_font_size,
                                line_y_c,
                                is_active_line,
                                metrics.gutter_padding,
                                metrics.line_height,
                            );
                            all_vertices.extend(ln_verts);
                        }
                    }
                }
                advance_cache.extend(build_cached_advance_cache_entries(
                    &cached,
                    doc_line_idx,
                    relative_skip,
                    ctx.left_margin,
                ));

                // Compute cursor position from cached data (cache-hit frames, no shaping needed)
                {
                    let cursor_doc_line = dv.cursor_line_cached();
                    if cursor_doc_line >= start_doc
                        && cursor_doc_line.saturating_sub(start_doc) < vp_height * 4
                    {
                        let cursor_vis_line = cursor_doc_line - start_doc;
                        if cursor_vis_line == i {
                            let cursor_col = dv.cursor_column();
                            // Find which visual line the cursor is on using cached data.
                            // Uses find_visual_line_index to unify boundary-handling with
                            // the non-cache path — non-last lines use "offset < end"
                            // (boundary belongs to the right/target line), preventing
                            // soft-wrap clicks from landing one visual line too high.
                            let bounds: Vec<(usize, usize)> = cached
                                .visual_lines
                                .iter()
                                .map(|&(vl_start, vl_end, _)| {
                                    let byte_start = cached
                                        .cluster_data
                                        .get(vl_start)
                                        .map(|cd| cd.0)
                                        .unwrap_or(usize::MAX);
                                    let byte_end = if vl_end > 0 {
                                        cached
                                            .cluster_data
                                            .get(vl_end - 1)
                                            .map(|cd| cd.1)
                                            .unwrap_or(usize::MAX)
                                    } else {
                                        usize::MAX
                                    };
                                    (byte_start, byte_end)
                                })
                                .collect();
                            let cursor_vl_in_doc = find_visual_line_index(&bounds, cursor_col);
                            let cursor_vl_in_doc_all = cursor_vl_in_doc + cached.subset_start;
                            if doc_line_idx == cursor_doc_line {
                                // Prefer click_hint VL over find_visual_line_index to avoid
                                // boundary ambiguity (byte=5 can be VL0-end or VL1-start).
                                let hint_matches = presentation
                                    .cursor_render_state
                                    .click_hint
                                    .is_some_and(|(ho, _hv)| {
                                        ho == dv
                                            .byte_to_unichar_offset(dv.cursor().offset.to_usize())
                                    });
                                if hint_matches {
                                    // click_hint resolves VL-in-doc boundary ambiguity;
                                    // cursor_visual_line (Y) computed from render-loop state.
                                    presentation.cursor_render_state.cursor_visual_line_in_doc =
                                        cursor_vl_in_doc_all;
                                    presentation.cursor_render_state.cursor_visual_line = Some(
                                        (visual_line_counter + cursor_vl_in_doc_all)
                                            .saturating_sub(skip_vl_c),
                                    );
                                } else {
                                    presentation.cursor_render_state.cursor_visual_line = Some(
                                        (visual_line_counter + cursor_vl_in_doc_all)
                                            .saturating_sub(skip_vl_c),
                                    );
                                    presentation.cursor_render_state.cursor_visual_line_in_doc =
                                        cursor_vl_in_doc_all;
                                }
                            }
                            if relative_skip > 0 && cursor_vl_in_doc < relative_skip {
                            } else {
                                let vli = cursor_vl_in_doc;
                                if vli < cached.visual_lines.len() {
                                    let (cvl_start, cvl_end, _) = cached.visual_lines[vli];
                                    let cluster_end = cached
                                        .cluster_data
                                        .get(cvl_end.saturating_sub(1))
                                        .map(|cd| cd.1)
                                        .unwrap_or(0);
                                    if let Some(px) = compute_cursor_pixel_x_cached(
                                        &cached.cluster_data,
                                        cvl_start,
                                        cvl_end,
                                        cursor_col,
                                        cluster_end,
                                        ctx.left_margin,
                                    ) {
                                        presentation.cursor_render_state.cursor_pixel_x = px;
                                    }
                                }
                                if dv.cursor().offset.to_usize()
                                    >= dv.line_byte_offset(doc_line_idx).unwrap_or(0)
                                    && presentation.cursor_render_state.sticky_x_dirty
                                {
                                    presentation.cursor_render_state.sticky_x =
                                        presentation.cursor_render_state.cursor_pixel_x;
                                    presentation.cursor_render_state.sticky_x_dirty = false;
                                }
                            }
                        }
                    }
                }

                let cached_scope = if cached.visual_lines.len() == cached.visual_line_count as usize
                    && cached.subset_start == 0
                {
                    ShapedLineScope::FullLine
                } else {
                    ShapedLineScope::VisualSubset
                };
                visual_line_counter += cached_line_rows_to_advance(
                    cached_scope,
                    cached.visual_line_count,
                    cached.visual_lines.len(),
                    skip_vl_c,
                    relative_skip,
                );
                _perf_hit_block_us += _hit_block_t0.elapsed().as_micros();
                let cl_done = cursor_rel_line.is_none() || i >= cursor_rel_line.unwrap();
                if visual_line_counter >= vp_height && cl_done {
                    break;
                }
                continue;
            }
        }
        // ── End RenderCache check ──

        // Use pre-fetched line_bytes from unified empty-line check above
        let line_bytes = line_bytes_early.expect("checked non-empty above");

        // Cache key: (offset, length, font_size_bits) — collision-free
        let _font_size_bits = text.shaper.font_size().to_bits();
        // Check cache first; shape only on miss.

        let doc_line_idx_miss = doc_idx;
        let line_len = dv.line_byte_length(doc_line_idx_miss).unwrap_or(0);
        let max_sync_bytes = 4000; // Synchronous shape threshold

        let map_stats = presentation
            .display
            .display_map
            .get_entry(doc_line_idx_miss)
            .map(|entry| ShapeMissMapStats::from_entry(entry, content_hash_fast));
        let is_placeholder = map_stats.map(|stats| stats.is_placeholder).unwrap_or(true);

        if is_placeholder && line_len > max_sync_bytes {
            // Line is too long for sync shape and hasn't been shaped yet. Yield to async background shaping.
            let vl_count = presentation
                .display
                .display_map
                .get_entry(doc_line_idx_miss)
                .map(|e| e.visual_line_count)
                .unwrap_or(1) as usize;
            // Render line number at minimum, so placeholder lines don't appear as ghosts.
            // Text content will appear once async shaping (submit_reshape_ahead) completes.
            if ctx.gutter_width > 0.0 && (i > 0 || skip_visual == 0) {
                let _line_num_buf = format_line_num(doc_line_idx_miss + 1);
                let line_num_str = line_num_str(&_line_num_buf);
                if let Ok(line_num_shaped) = text.shaper.shape(line_num_str) {
                    let sub_line_offset = sub_line_offset;
                    let ln_y = (visual_line_counter) as f32 * metrics.line_height + sub_line_offset;
                    let ln_font_size = text.shaper.font_size() * ui::constants::LN_FONT_SCALE;
                    let ln_verts = generate_line_number_vertices(
                        ctx,
                        &line_num_shaped,
                        &mut text.atlas,
                        &text.atlas_texture,
                        &gpu.ctx.queue,
                        &mut text.shaper,
                        ln_font_size,
                        ln_y,
                        is_active_line,
                        metrics.gutter_padding,
                        metrics.line_height,
                    );
                    all_vertices.extend(ln_verts);
                }
            }
            visual_line_counter += vl_count;

            // Advance cache placeholder so indices stay aligned
            advance_cache.push(AdvanceCacheEntry {
                doc_line: doc_line_idx_miss,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: Vec::new(),
            });

            let cursor_line_done = cursor_rel_line.is_none() || i >= cursor_rel_line.unwrap();
            if visual_line_counter >= vp_height && cursor_line_done {
                break;
            }
            continue;
        }

        _miss_count += 1;
        _perf_cache_misses += 1;

        let _hl_t0 = std::time::Instant::now();
        // --- Subset Shaping Optimization for Long Lines ---
        let mut line_str_to_shape = std::str::from_utf8(&line_bytes).unwrap_or("");
        let mut byte_offset_for_clusters = 0;
        let mut shape_subset_only = false;
        let mut visible_tree_breaks = None;
        let mut doc_vl_count = 1;
        let mut subset_start_visual_line = 0usize;
        let mut subset_end_visual_line = 0usize;
        let mut visible_visual_lines_needed = 0usize;
        let mut cursor_visual_lines_needed = 0usize;

        if !is_placeholder
            && line_len > max_sync_bytes
            && let Some(entry) = presentation.display.display_map.get_entry(doc_line_idx_miss)
            && entry.content_hash == content_hash_fast
            && !entry.visual_breaks.is_empty()
        {
            let skip = if i == 0 { skip_visual } else { 0 };
            let vl_needed = vp_height.saturating_sub(visual_line_counter);
            visible_visual_lines_needed = vl_needed;

            let mut cursor_needed = 0;
            if Some(i) == cursor_rel_line {
                let cursor_offset = dv.cursor().offset.to_usize();
                let current_line_offset =
                    dv.line_byte_offset(doc_line_idx_miss).unwrap_or(entry.byte_offset);
                if let Some(rel_offset) =
                    cursor_relative_offset_for_line(cursor_offset, current_line_offset)
                {
                    let cursor_vl = entry
                        .visual_breaks
                        .partition_point(|b| b.byte_start <= rel_offset)
                        .saturating_sub(1);
                    if cursor_vl >= skip {
                        cursor_needed = cursor_vl - skip + 1;
                    }
                }
            }
            cursor_visual_lines_needed = cursor_needed;

            let needed = vl_needed.max(cursor_needed);
            let end_idx = (skip + needed + 1).min(entry.visual_breaks.len());
            subset_start_visual_line = skip;
            subset_end_visual_line = end_idx;

            if skip < end_idx {
                let start_byte = entry.visual_breaks[skip].byte_start as usize;
                let end_byte = entry.visual_breaks[end_idx - 1].byte_end as usize;

                line_str_to_shape =
                    std::str::from_utf8(&line_bytes[start_byte..end_byte]).unwrap_or("");
                byte_offset_for_clusters = start_byte;
                shape_subset_only = true;
                visible_tree_breaks = Some(entry.visual_breaks[skip..end_idx].to_vec());
            }
            doc_vl_count = entry.visual_breaks.len();
        }

        _perf_hl_time_us += _hl_t0.elapsed().as_micros();
        let _shape_t0 = std::time::Instant::now();

        // Fetch highlight spans once; reuse for both bold/italic check and span building.
        let hl_spans = highlights_for_line(dv, presentation, doc_line_idx_miss);
        let has_bold_italic = hl_spans.iter().any(|h| {
            matches!(
                h.kind,
                HighlightKind::MarkupBold
                    | HighlightKind::MarkupItalic
                    | HighlightKind::MarkupHeading
            )
        });

        let mut shaped = if has_bold_italic {
            // Build per-span weight/style overrides from highlight spans.
            // When shape_subset_only, adjust byte offsets to be relative to the substring.
            let subset_start = byte_offset_for_clusters;
            let subset_end = subset_start + line_str_to_shape.len();
            let mut span_attrs: Vec<(usize, shaping::Weight, shaping::Style)> = Vec::new();
            let mut last_end = 0usize;
            for h in hl_spans.iter() {
                // Skip highlights entirely outside the visible subset
                if h.start >= subset_end {
                    break;
                }
                // Clip highlights that start before the subset
                let start = h.start.saturating_sub(subset_start);
                if start >= line_str_to_shape.len() {
                    continue;
                }
                // Fill gap with base weight/style
                if start > last_end {
                    span_attrs.push((
                        last_end,
                        text.shaper.font_weight(),
                        text.shaper.font_style(),
                    ));
                }
                let (w, s) = match h.kind {
                    HighlightKind::MarkupBold => (shaping::Weight::BOLD, text.shaper.font_style()),
                    HighlightKind::MarkupItalic => {
                        (text.shaper.font_weight(), shaping::Style::Italic)
                    }
                    HighlightKind::MarkupHeading => {
                        (shaping::Weight::BOLD, text.shaper.font_style())
                    }
                    _ => (text.shaper.font_weight(), text.shaper.font_style()),
                };
                span_attrs.push((start, w, s));
                last_end = start;
            }
            // Fill trailing gap
            if last_end < line_str_to_shape.len() {
                span_attrs.push((last_end, text.shaper.font_weight(), text.shaper.font_style()));
            }
            // Deduplicate consecutive identical attrs
            span_attrs.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);

            match text.shaper.shape_with_highlights(line_str_to_shape, &span_attrs) {
                Ok(s) => s,
                Err(_) => continue,
            }
        } else {
            match text.shaper.shape_fast(line_str_to_shape) {
                Ok(s) => s,
                Err(_) => match text.shaper.shape(line_str_to_shape) {
                    Ok(s) => s,
                    Err(_) => continue,
                },
            }
        };

        if shape_subset_only {
            for c in &mut shaped.clusters {
                c.byte_range.start += byte_offset_for_clusters;
                c.byte_range.end += byte_offset_for_clusters;
            }
        }

        if shaped.clusters.is_empty() {
            // Shaper produced no glyphs (e.g. control chars / whitespace only) — still render line number
            if ctx.gutter_width > 0.0 && (i > 0 || skip_visual == 0) {
                render_line_number_placeholder(
                    metrics,
                    ctx,
                    text,
                    gpu,
                    is_active_line,
                    doc_line_idx,
                    visual_line_counter,
                    &mut all_vertices,
                    sub_line_offset,
                );
            }
            advance_cache.push(AdvanceCacheEntry {
                doc_line: doc_line_idx,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: Vec::new(),
            });
            visual_line_counter += 1;
            continue;
        }

        // Monospace column width measured from font
        let char_width = text.shaper.col_width();

        // Word wrap: normal word wrap
        let viewport_width =
            render_viewport_width(ctx.screen_w, ctx.left_margin, metrics, word_wrap);
        let tree_breaks: Option<Vec<(usize, usize, f32)>> = if shape_subset_only {
            let breaks = visible_tree_breaks.unwrap();
            let visual_lines: Vec<_> = breaks
                .iter()
                .map(|b| {
                    let start = shaped
                        .clusters
                        .partition_point(|c| c.byte_range.start < b.byte_start as usize);
                    let end = shaped
                        .clusters
                        .partition_point(|c| c.byte_range.start < b.byte_end as usize);
                    (start, end, b.pixel_width)
                })
                .collect();
            Some(visual_lines)
        } else {
            presentation.display.display_map.get_entry(doc_line_idx_miss).and_then(|entry| {
                // Exclude placeholder entries (single break with pixel_width=0 on non-empty lines)
                let is_placeholder = entry.visual_breaks.len() == 1
                    && entry.visual_breaks[0].pixel_width == 0.0
                    && entry.byte_length > 0;
                if entry.content_hash == content_hash_fast
                    && !entry.visual_breaks.is_empty()
                    && !is_placeholder
                {
                    let visual_lines: Vec<_> = entry
                        .visual_breaks
                        .iter()
                        .map(|b| {
                            let start = shaped
                                .clusters
                                .partition_point(|c| c.byte_range.start < b.byte_start as usize);
                            let end = shaped
                                .clusters
                                .partition_point(|c| c.byte_range.start < b.byte_end as usize);
                            (start, end, b.pixel_width)
                        })
                        .collect();
                    if visual_lines.iter().all(|&(s, e, _)| s <= e) {
                        Some(visual_lines)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
        };

        let visual_lines = if let Some(lines) = tree_breaks {
            lines
        } else {
            compute_visual_lines(&shaped.clusters, &line_bytes, char_width, viewport_width, 0.5)
        };

        // Apply punctuation minimum-width padding after visual lines are computed.
        // Line-break points use original advances; padding only affects rendering & cursor.
        let em_width = text.shaper.font_size();
        let min_ratio = min_punctuation_width_ratio;
        if min_ratio > 0.0 {
            apply_punctuation_padding(&mut shaped.clusters, &line_bytes, em_width, min_ratio);
        }
        // Defer WrapIndex update until after the loop
        let doc_line = doc_idx;

        // skip_visual: only for the first visible doc line
        let skip_visual_local = if shape_subset_only {
            0
        } else if i == 0 {
            skip_visual
        } else {
            0
        };
        let skip_visual_local = skip_visual_local.min(visual_lines.len());

        // B4: Cache visual line + cluster data for first/last visible doc lines
        if i == 0 {
            first_line.visual_lines = visual_lines.clone();
            first_line.clusters = shaped
                .clusters
                .iter()
                .map(|c| {
                    let (_, adv) = line_bytes
                        .get(c.byte_range.clone())
                        .map(|b| cluster_advance(b, c.advance, char_width))
                        .unwrap_or((false, c.advance.max(1.0)));
                    (c.byte_range.start, c.byte_range.end, adv)
                })
                .collect();
            first_line.doc_offset = dv.line_byte_offset(start_doc).unwrap_or(0);
        }
        // Always overwrite — ends up with last visible doc line's data
        {
            last_line.visual_lines = visual_lines.clone();
            last_line.clusters = shaped
                .clusters
                .iter()
                .map(|c| {
                    let (_, adv) = line_bytes
                        .get(c.byte_range.clone())
                        .map(|b| cluster_advance(b, c.advance, char_width))
                        .unwrap_or((false, c.advance.max(1.0)));
                    (c.byte_range.start, c.byte_range.end, adv)
                })
                .collect();
        }

        // Compute cursor pixel x-position from shaped cluster advances
        let cursor_doc_line = dv.cursor_line_cached();
        let range = presentation
            .display
            .viewport
            .visible_doc_range_from_anchor(&presentation.display.display_map, lh);
        let cursor_vis_line = if cursor_doc_line >= range.start && cursor_doc_line < range.end {
            Some(cursor_doc_line - range.start)
        } else {
            None
        };
        if cursor_vis_line == Some(i) && !shaped.clusters.is_empty() {
            let cursor_col = dv.cursor_column();

            // First, find which visual line the cursor is on
            let bounds: Vec<(usize, usize)> = visual_lines
                .iter()
                .map(|&(vl_start, vl_end, _)| {
                    let byte_start = shaped.clusters[vl_start].byte_range.start;
                    let byte_end = shaped.clusters[vl_end - 1].byte_range.end;
                    (byte_start, byte_end)
                })
                .collect();
            let mut cursor_vl_in_doc_all = find_visual_line_index(&bounds, cursor_col);

            // End affinity: if cursor lands at a VL boundary after End,
            // prefer the left VL so the caret stays on the current line.
            if dv.cursor().last_command_was_end && cursor_vl_in_doc_all > 0 {
                let prev_end = bounds[cursor_vl_in_doc_all - 1].1;
                if cursor_col == prev_end {
                    cursor_vl_in_doc_all -= 1;
                }
            }
            // Check if cursor is in the skipped area (above visible portion of first doc line)
            let actual_skip = if i == 0 { skip_visual } else { 0 };
            let cursor_in_skipped = actual_skip > 0 && cursor_vl_in_doc_all < actual_skip;

            if doc_line == cursor_doc_line && !cursor_in_skipped {
                let full_vl_in_doc = if shape_subset_only {
                    cursor_vl_in_doc_all + actual_skip
                } else {
                    cursor_vl_in_doc_all
                };
                // Prefer click_hint VL over find_visual_line_index to avoid
                // boundary ambiguity (byte=5 can be VL0-end or VL1-start).
                let hint_matches =
                    presentation.cursor_render_state.click_hint.is_some_and(|(ho, _hv)| {
                        ho == dv.byte_to_unichar_offset(dv.cursor().offset.to_usize())
                    });
                if hint_matches {
                    // click_hint resolves VL-in-doc boundary ambiguity;
                    // cursor_visual_line (Y) computed from render-loop state.
                    presentation.cursor_render_state.cursor_visual_line_in_doc = full_vl_in_doc;
                    presentation.cursor_render_state.cursor_visual_line =
                        Some((visual_line_counter + full_vl_in_doc).saturating_sub(actual_skip));
                } else {
                    presentation.cursor_render_state.cursor_visual_line =
                        Some((visual_line_counter + full_vl_in_doc).saturating_sub(actual_skip));
                    presentation.cursor_render_state.cursor_visual_line_in_doc = full_vl_in_doc;
                }
            }

            if !cursor_in_skipped {
                // Cursor is in the visible area — compute pixel position via byte_to_x
                // (same code path as selection highlighting, for consistency)
                let vl_idx = cursor_vl_in_doc_all;
                if vl_idx < visual_lines.len() {
                    let (vl_start, _vl_end, _) = visual_lines[vl_idx];
                    let vl_byte_start = shaped.clusters[vl_start].byte_range.start;
                    let cursor_local = dv.cursor_column().saturating_sub(vl_byte_start);
                    // Build temporary cluster array for byte_to_x
                    let mut cluster_xs: smallvec::SmallVec<[(usize, f32, u32); 64]> =
                        smallvec::SmallVec::new();
                    let mut x = ctx.left_margin;
                    for c in &shaped.clusters[vl_start.._vl_end] {
                        let (_, adv) = cluster_advance(
                            &line_bytes[c.byte_range.clone()],
                            c.advance,
                            char_width,
                        );
                        x += adv;
                        // grapheme_idx unused — only byte_to_x consumes this data
                        cluster_xs.push((c.byte_range.end - vl_byte_start, x, 0u32));
                    }
                    let px = ui::render_geom::byte_to_x(
                        cursor_local,
                        &cluster_xs,
                        ctx.left_margin,
                        false,
                    );
                    presentation.cursor_render_state.cursor_pixel_x = px;
                }
            }

            if cursor_doc_line == doc_line {
                // Sync sticky_x from cursor_pixel_x after horizontal move
                if presentation.cursor_render_state.sticky_x_dirty {
                    presentation.cursor_render_state.sticky_x =
                        presentation.cursor_render_state.cursor_pixel_x;
                    presentation.cursor_render_state.sticky_x_dirty = false;
                }
            }
        }

        // Populate advance cache for hit-testing
        let doc_line_idx = range.start + i;
        let entries = build_advance_cache_entries(
            &visual_lines,
            skip_visual_local,
            &shaped,
            &line_bytes,
            char_width,
            doc_line_idx,
            cluster_pool,
            ctx.left_margin,
        );
        advance_cache.extend(entries);

        // Render line number in gutter (first VL of each doc line)
        let doc_line_idx = range.start + i;
        if ctx.gutter_width > 0.0
            && skip_visual_local == 0
            && (!shape_subset_only || (i == 0 && skip_visual == 0) || i > 0)
        {
            let _line_num_buf = format_line_num(doc_line_idx + 1);
            let line_num_str = line_num_str(&_line_num_buf);
            if let Ok(line_num_shaped) = text.shaper.shape(line_num_str) {
                let sub_line_offset = sub_line_offset;
                let ln_y = (visual_line_counter) as f32 * metrics.line_height + sub_line_offset;
                let ln_font_size = text.shaper.font_size() * ui::constants::LN_FONT_SCALE;
                let ln_verts = generate_line_number_vertices(
                    ctx,
                    &line_num_shaped,
                    &mut text.atlas,
                    &text.atlas_texture,
                    &gpu.ctx.queue,
                    &mut text.shaper,
                    ln_font_size,
                    ln_y,
                    is_active_line,
                    metrics.gutter_padding,
                    metrics.line_height,
                );
                all_vertices.extend(ln_verts);
            }
        }

        let shape_elapsed_us = _shape_t0.elapsed().as_micros();
        _perf_shape_time_us += shape_elapsed_us;
        if should_perf_log(shape_elapsed_us) {
            let (
                entry_present,
                hash_matches,
                visual_break_count,
                visual_line_count,
                byte_offset,
                byte_length,
            ) = map_stats
                .map(|stats| {
                    (
                        true,
                        stats.hash_matches,
                        stats.visual_break_count,
                        stats.visual_line_count,
                        stats.byte_offset,
                        stats.byte_length,
                    )
                })
                .unwrap_or((false, false, 0, 0, 0, 0));
            eprintln!(
                "[perf:shape_miss] doc_line={} line_len={} shape={}us subset={} subset_len={} subset_vl={}..{} visible_needed={} cursor_needed={} entry={} hash_match={} breaks={} entry_vl={} entry_offset={} entry_len={} placeholder={} has_style={} doc_vl_count={}",
                doc_line_idx_miss,
                line_len,
                shape_elapsed_us,
                shape_subset_only,
                line_str_to_shape.len(),
                subset_start_visual_line,
                subset_end_visual_line,
                visible_visual_lines_needed,
                cursor_visual_lines_needed,
                entry_present,
                hash_matches,
                visual_break_count,
                visual_line_count,
                byte_offset,
                byte_length,
                is_placeholder,
                has_bold_italic,
                doc_vl_count,
            );
        }
        let _vtx_t0 = std::time::Instant::now();
        // Render each visual line
        let sub_line_offset = sub_line_offset;
        for (vl_idx, &(vl_start, vl_end, _vl_width)) in visual_lines.iter().enumerate() {
            if vl_idx < skip_visual_local {
                continue;
            }
            let line_y = (visual_line_counter + vl_idx - skip_visual_local) as f32
                * metrics.line_height
                + sub_line_offset;
            let y_base =
                line_y + metrics.line_height * ui::constants::BASELINE_RATIO + ctx.tab_bar_height;

            // Get highlight spans for this document line.
            let highlight_spans: Vec<(usize, HighlightKind)> =
                highlights_for_line(dv, presentation, doc_line_idx)
                    .iter()
                    .map(|highlight| (highlight.start, highlight.kind))
                    .collect();

            let mut x_cursor = ctx.left_margin;

            for cluster in &shaped.clusters[vl_start..vl_end] {
                let glyph_id = cluster.glyph_id as u16;
                let font_id = cluster.font_id;
                let font_size = text.shaper.font_size();

                let cluster_bytes = &line_bytes[cluster.byte_range.clone()];
                let (is_ws, advance) = cluster_advance(cluster_bytes, cluster.advance, char_width);
                if is_ws {
                    x_cursor += advance;
                    continue;
                }

                let (int_x, phase) = render::split_subpixel(x_cursor + cluster.x_offset);
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
                    highlight_color_for_offset(
                        &highlight_spans,
                        cluster.byte_range.start,
                        ctx.theme,
                    )
                };

                // IME preedit: shift text after cursor on the cursor's doc line
                let render_x = if ctx.preedit_advance_px > 0.0
                    && doc_line_idx == cursor_doc_line
                    && cluster.byte_range.start >= ctx.preedit_cursor_col
                {
                    int_x + ctx.preedit_advance_px
                } else {
                    int_x
                };

                let verts = GlyphRenderer::generate_vertices(
                    &[(slot, render_x, y_base)],
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
        if shape_subset_only {
            visual_line_counter +=
                doc_vl_count.saturating_sub(if i == 0 { skip_visual } else { 0 });
        } else {
            visual_line_counter += visual_lines.len().saturating_sub(skip_visual_local);
        }

        _perf_vertex_time_us += _vtx_t0.elapsed().as_micros();
        let _rc_t0 = std::time::Instant::now();
        // ── Populate RenderCache ──
        // Skip caching during IME preedit on the cursor line (positions are shifted)
        let cache_doc_line = doc_idx;
        let shaped_line_scope = ShapedLineScope::from_subset_flag(shape_subset_only);
        let cache_exists = presentation.display.render_cache.get(cache_doc_line).is_some();
        if should_store_shaped_line_in_render_cache(
            shaped_line_scope,
            preedit_on_cursor_line,
            cache_exists,
        ) {
            let content_hash = content_hash_fast;
            let mut all_instances: Vec<GlyphInstance> = Vec::new();
            let mut vl_instance_starts: Vec<usize> = Vec::new();

            // Cache highlight spans for per-glyph coloring
            let cache_hl_spans: Vec<(usize, HighlightKind)> =
                highlights_for_line(dv, presentation, cache_doc_line)
                    .iter()
                    .map(|highlight| (highlight.start, highlight.kind))
                    .collect();

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

                    let (c_ws, c_adv) = cluster_advance(
                        &line_bytes[cluster.byte_range.clone()],
                        cluster.advance,
                        char_width,
                    );

                    if c_ws {
                        x_c += c_adv;
                        continue;
                    }

                    let (int_x, phase) = render::split_subpixel(x_c + cluster.x_offset);
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
                            highlight_kind: if cache_hl_spans.is_empty() {
                                0
                            } else {
                                // Highlight spans: [(start0, kind0), (start1, kind1), ..., sentinel]
                                // A cluster belongs to the last span whose start <= byte_start.
                                cache_hl_spans
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
                        cluster_advance(&line_bytes[c.byte_range.clone()], c.advance, char_width);
                    (c.byte_range.start, c.byte_range.end, adv)
                })
                .collect();

            let cache_metadata = render_cache_metadata_for_shaped_line(
                shaped_line_scope,
                visual_lines.len(),
                visual_line_count_from_len(doc_vl_count),
                skip_visual_local,
                subset_start_visual_line,
            );

            if shaped_line_scope == ShapedLineScope::FullLine {
                let breaks: smallvec::SmallVec<[crate::snap_tree::VisualBreak; 1]> = visual_lines
                    .iter()
                    .map(|&(start, end, width)| {
                        let byte_start = shaped
                            .clusters
                            .get(start)
                            .map(|c| c.byte_range.start as u32)
                            .unwrap_or(0);
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
                    byte_offset: dv.line_byte_offset(cache_doc_line).unwrap_or(0),
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
                    tree_dirty_local = true;
                }
            }

            presentation.display.render_cache.insert(
                cache_doc_line,
                CachedLine {
                    instances: all_instances,
                    line_number_glyphs: vec![],
                    atlas_generation: 0,
                    visual_line_count: cache_metadata.visual_line_count,
                    content_hash,
                    visual_lines: visual_lines.clone(),
                    visual_line_instance_starts: vl_instance_starts,
                    cluster_data,
                    subset_start: cache_metadata.subset_start,
                },
            );
        }
        // ── End RenderCache population ──

        let cursor_line_done = cursor_rel_line.is_none() || i >= cursor_rel_line.unwrap();
        if visual_line_counter >= vp_height && cursor_line_done {
            break;
        }
    }

    if tree_dirty_local {
        presentation.display.display_map.rebuild_tree();
        *tree_dirty = true;
    }

    // Post-shape update

    let _perf_total = _perf_t0.elapsed().as_micros();
    if should_perf_log(_perf_total) {
        eprintln!(
            "[perf:shape] total={}us setup={}us hits={} misses={} hit_block={}us empty={}us hl={}us shape={}us vtx={}us rc_pop={}us",
            _perf_total,
            _perf_setup_us,
            _perf_cache_hits,
            _perf_cache_misses,
            _perf_hit_block_us,
            _perf_empty_us,
            _perf_hl_time_us,
            _perf_shape_time_us,
            _perf_vertex_time_us,
            _perf_rc_populate_us,
        );
    }
    all_vertices
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

/// Generate vertices for the search bar text overlay.

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
        let advance = cluster.advance.max(1.0);

        let (int_x, phase) = render::split_subpixel(x_cursor);
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
