//! Geometry helpers for selection highlighting and cursor rendering.
//!
//! Pure functions — testable without GPU or App state.

use render::GlyphVertex;

/// Per-visual-line data for hit-testing, selection rendering, and cursor movement.
///
/// Byte coordinate semantics (unified convention):
/// - `vl_byte_start`: byte offset of this VL's start *relative to doc line start* (line-local).
/// - `vl_grapheme_start`: 0-based grapheme index of this VL's first grapheme (within doc line).
/// - `clusters[i].0` (cluster_end_byte): byte offset of this cluster's end
///   *relative to VL start* (vl-local).
/// - `clusters[i].2` (grapheme_idx): 0-based grapheme cluster index within this visual line.
///
/// For the i-th cluster:
///   absolute byte range in doc = [line_byte_offset(doc_line) + vl_byte_start + prev_cluster_end,
///                                 line_byte_offset(doc_line) + vl_byte_start + clusters[i].0)
#[derive(Clone, Debug)]
pub struct AdvanceCacheEntry {
    pub doc_line: usize,
    pub vl_byte_start: usize,
    pub vl_grapheme_start: usize,
    pub clusters: Vec<(usize, f32, u32)>, // (cluster_end_byte_vl_local, pixel_x, grapheme_idx)
}

/// Map a byte offset (vl-local, relative to visual-line start) to its pixel x position.
///
/// `clusters`: sorted `(cluster_end_byte_vl_local, pixel_x, grapheme_idx)` tuples.
/// `byte_offset`: vl-local byte offset.
/// `left_margin`: starting x pixel where text content begins.
/// `is_end`: if true, return the *end* x of the cluster containing `byte_offset`;
///           if false, return the *start* x.
pub fn byte_to_x(
    byte_offset: usize,
    clusters: &[(usize, f32, u32)],
    left_margin: f32,
    is_end: bool,
) -> f32 {
    let mut prev_x = left_margin;
    let mut prev_end: usize = 0;
    for &(c_end, c_x, _) in clusters {
        if if is_end { c_end >= byte_offset } else { c_end > byte_offset } {
            let cluster_bytes = c_end.saturating_sub(prev_end);
            if cluster_bytes == 0 {
                return if is_end { c_x } else { prev_x };
            }
            let fraction = (byte_offset.saturating_sub(prev_end)) as f32 / cluster_bytes as f32;
            return prev_x + (c_x - prev_x) * fraction;
        }
        prev_x = c_x;
        prev_end = c_end;
    }
    clusters.last().map(|&(_, x, _)| x).unwrap_or(left_margin)
}

/// Map a grapheme index (vl-local) to its pixel x position.
///
/// `target_unichar`: 0-based grapheme index within this visual line.
/// `clusters`: sorted `(cluster_end_byte_vl_local, pixel_x, grapheme_idx)` tuples.
/// `left_margin`: starting x pixel where text content begins.
/// `trailing`: if true, return the *end* x of the target grapheme;
///             if false, return the *start* x.
pub fn unichar_to_x(
    target_unichar: u32,
    clusters: &[(usize, f32, u32)],
    left_margin: f32,
    trailing: bool,
) -> f32 {
    let mut prev_x = left_margin;
    for &(_, c_x, g_idx) in clusters {
        if g_idx >= target_unichar {
            return if trailing { c_x } else { prev_x };
        }
        prev_x = c_x;
    }
    clusters.last().map(|&(_, x, _)| x).unwrap_or(left_margin)
}

/// Find the grapheme index whose pixel x is closest to `target_x`.
///
/// `target_x`: pixel x position to query.
/// `clusters`: sorted `(cluster_end_byte_vl_local, pixel_x, grapheme_idx)` tuples.
/// Returns the `grapheme_idx` of the closest cluster. Returns 0 if clusters is
/// empty or `target_x` is before the first cluster.
pub fn x_to_unichar(target_x: f32, clusters: &[(usize, f32, u32)], left_margin: f32) -> u32 {
    if clusters.is_empty() {
        return 0;
    }
    // Before the first cluster
    if target_x <= clusters[0].1 {
        return clusters[0].2;
    }
    let mut prev_x = left_margin;
    let mut prev_g: u32 = 0;
    for &(_, c_x, g_idx) in clusters {
        let mid = (prev_x + c_x) / 2.0;
        if target_x <= mid {
            return prev_g;
        }
        prev_x = c_x;
        prev_g = g_idx;
    }
    // Past the last cluster
    prev_g
}

/// Compute selection highlight quads from advance cache data.
///
/// `advance_cache`: per-visual-line data.
/// `sel_range`: `(start_byte, end_byte)` of the selection (end is exclusive).
/// Returns GlyphVertex quads (6 vertices per quad) for semi-transparent highlight rectangles.
#[allow(
    clippy::too_many_arguments,
    reason = "selection geometry consumes independent line-map, viewport, and style inputs"
)]
pub fn compute_selection_highlight_quads(
    advance_cache: &[AdvanceCacheEntry],
    sel_range: (usize, usize),
    line_byte_offsets: &[usize],
    screen_w: f32,
    screen_h: f32,
    line_height: f32,
    sub_line_offset: f32,
    left_margin: f32,
    selection_color: [f32; 4],
    tab_bar_height: f32,
) -> Vec<GlyphVertex> {
    let (sel_start, sel_end) = sel_range;
    if sel_start >= sel_end || advance_cache.is_empty() {
        return Vec::new();
    }

    let mut quads = Vec::new();
    let uv = 0.0f32;
    // VS Code-style selection highlight: semi-transparent blue
    let color = selection_color;
    // left_margin is passed directly from caller

    for (vl_idx, entry) in advance_cache.iter().enumerate() {
        let doc_line = entry.doc_line;
        let vl_byte_start = entry.vl_byte_start;
        let clusters = &entry.clusters;
        if clusters.is_empty() {
            continue;
        }
        // advance_cache stores LOCAL byte offsets (relative to line start).
        // Convert to absolute document offsets for comparison with sel_range.
        let line_abs = line_byte_offsets.get(doc_line).copied().unwrap_or(0);
        let abs_vl_start = line_abs + vl_byte_start;
        // clusters store vl-local end offsets; add vl_byte_start to get line-local absolute end
        let vl_byte_end = clusters.last().map(|&(end, _, _)| end).unwrap_or(0);
        let abs_vl_end = line_abs + vl_byte_start + vl_byte_end;

        // Clamp absolute selection to this visual line's absolute range
        let clip_start = sel_start.max(abs_vl_start);
        let clip_end = sel_end.min(abs_vl_end);
        if clip_start >= clip_end {
            continue;
        }

        // Convert clipped absolute offsets to vl-local for byte_to_x.
        // Use saturating_sub to guard against edge cases where line_abs
        // may not match the entry's actual line offset (defensive).
        let local_clip_start = clip_start.saturating_sub(line_abs).saturating_sub(vl_byte_start);
        let local_clip_end = clip_end.saturating_sub(line_abs).saturating_sub(vl_byte_start);
        let x_start = byte_to_x(local_clip_start, clusters, left_margin, false);
        let x_end = byte_to_x(local_clip_end, clusters, left_margin, true);
        if x_end <= x_start {
            continue;
        }

        let line_y = vl_idx as f32 * line_height + sub_line_offset;
        let top = 1.0 - (line_y + tab_bar_height) / screen_h * 2.0;
        let bottom = 1.0 - (line_y + line_height + tab_bar_height) / screen_h * 2.0;
        let left = x_start / screen_w * 2.0 - 1.0;
        let right = x_end / screen_w * 2.0 - 1.0;

        quads.push(GlyphVertex { position: [left, top], tex_coords: [uv, uv], color });
        quads.push(GlyphVertex { position: [right, top], tex_coords: [uv, uv], color });
        quads.push(GlyphVertex { position: [left, bottom], tex_coords: [uv, uv], color });
        quads.push(GlyphVertex { position: [right, top], tex_coords: [uv, uv], color });
        quads.push(GlyphVertex { position: [right, bottom], tex_coords: [uv, uv], color });
        quads.push(GlyphVertex { position: [left, bottom], tex_coords: [uv, uv], color });
    }

    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_to_x_first_cluster() {
        let clusters = vec![(5usize, 110.0f32, 0u32), (10, 120.0, 1), (15, 130.0, 2)];
        assert!((byte_to_x(3, &clusters, 100.0, false) - 106.0).abs() < 0.01);
        assert!((byte_to_x(7, &clusters, 100.0, false) - 114.0).abs() < 0.01);
        assert!((byte_to_x(7, &clusters, 100.0, true) - 114.0).abs() < 0.01);
    }

    #[test]
    fn byte_to_x_exact_boundary() {
        let clusters = vec![(5usize, 110.0f32, 0u32), (10, 120.0, 1)];
        assert!((byte_to_x(5, &clusters, 100.0, true) - 110.0).abs() < 0.01);
        assert!((byte_to_x(5, &clusters, 100.0, false) - 110.0).abs() < 0.01);
    }

    #[test]
    fn byte_to_x_past_end() {
        let clusters = vec![(5usize, 110.0f32, 0u32)];
        assert!((byte_to_x(100, &clusters, 100.0, false) - 110.0).abs() < 0.01);
        assert!((byte_to_x(100, &clusters, 100.0, true) - 110.0).abs() < 0.01);
    }

    #[test]
    fn byte_to_x_empty_clusters() {
        let clusters: Vec<(usize, f32, u32)> = vec![];
        assert!((byte_to_x(5, &clusters, 100.0, false) - 100.0).abs() < 0.01);
    }

    #[test]
    fn compute_selection_highlight_empty_range() {
        let cache = vec![];
        let result = compute_selection_highlight_quads(
            &cache,
            (5, 5),
            &[],
            800.0,
            600.0,
            14.0,
            0.0,
            0.0,
            [0.0; 4],
            32.0,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn compute_selection_highlight_empty_cache() {
        let cache = vec![];
        let result = compute_selection_highlight_quads(
            &cache,
            (0, 100),
            &[0],
            800.0,
            600.0,
            14.0,
            0.0,
            0.0,
            [0.0; 4],
            32.0,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn byte_to_x_vl_local_at_zero() {
        let clusters = vec![(5usize, 110.0f32, 0u32)];
        assert!((byte_to_x(0, &clusters, 100.0, false) - 100.0).abs() < 0.01);
        assert!((byte_to_x(0, &clusters, 100.0, true) - 100.0).abs() < 0.01);
    }

    #[test]
    fn compute_selection_highlight_quads_multi_vl_left_aligned() {
        let cache = vec![
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 0,
                vl_grapheme_start: 0,
                clusters: vec![(2, 40.0, 0), (5, 80.0, 1)],
            },
            AdvanceCacheEntry {
                doc_line: 0,
                vl_byte_start: 5,
                vl_grapheme_start: 2,
                clusters: vec![(2, 40.0, 0), (5, 80.0, 1)],
            },
        ];
        let offsets = vec![0usize];
        let result = compute_selection_highlight_quads(
            &cache,
            (2, 8),
            &offsets,
            800.0,
            600.0,
            14.0,
            0.0,
            32.0,
            [0.1, 0.2, 0.8, 0.3],
            0.0,
        );
        assert_eq!(result.len(), 12, "should produce 2 quads (12 vertices) for 2-VL selection");
    }

    #[test]
    fn byte_to_x_with_padded_punctuation() {
        let clusters: Vec<(usize, f32, u32)> = vec![(1, 7.5, 0), (2, 16.5, 1), (3, 24.0, 2)];
        let left_margin = 0.0;

        let x = byte_to_x(0, &clusters, left_margin, false);
        assert!((x - 0.0).abs() < 0.01, "byte 0 start: expected 0.0, got {x}");

        let x = byte_to_x(1, &clusters, left_margin, true);
        assert!((x - 7.5).abs() < 0.01, "byte 1 end: expected 7.5, got {x}");

        let x = byte_to_x(1, &clusters, left_margin, false);
        assert!((x - 7.5).abs() < 0.01, "byte 1 start: expected 7.5, got {x}");

        let x = byte_to_x(2, &clusters, left_margin, false);
        assert!((x - 16.5).abs() < 0.01, "byte 2 start: expected 16.5, got {x}");

        let x = byte_to_x(0, &clusters, left_margin, false);
        assert!((x - 0.0).abs() < 0.01);
    }

    // ── unichar_to_x tests ──

    #[test]
    fn unichar_to_x_ascii_trailing() {
        let clusters = vec![(1usize, 10.0f32, 0u32), (2, 20.0, 1), (3, 30.0, 2)];
        let lm = 0.0;
        // trailing=true: end of grapheme N = c_x of cluster with g_idx >= N
        assert!((unichar_to_x(0, &clusters, lm, true) - 10.0).abs() < 0.01);
        assert!((unichar_to_x(1, &clusters, lm, true) - 20.0).abs() < 0.01);
        assert!((unichar_to_x(2, &clusters, lm, true) - 30.0).abs() < 0.01);
    }

    #[test]
    fn unichar_to_x_ascii_leading() {
        let clusters = vec![(1usize, 10.0f32, 0u32), (2, 20.0, 1), (3, 30.0, 2)];
        let lm = 0.0;
        // trailing=false: start of grapheme N = prev_x of cluster with g_idx >= N
        assert!((unichar_to_x(0, &clusters, lm, false) - 0.0).abs() < 0.01);
        assert!((unichar_to_x(1, &clusters, lm, false) - 10.0).abs() < 0.01);
        assert!((unichar_to_x(2, &clusters, lm, false) - 20.0).abs() < 0.01);
    }

    #[test]
    fn unichar_to_x_past_end() {
        let clusters = vec![(1usize, 10.0f32, 0u32), (2, 20.0, 1)];
        let lm = 0.0;
        assert!((unichar_to_x(100, &clusters, lm, true) - 20.0).abs() < 0.01);
        assert!((unichar_to_x(100, &clusters, lm, false) - 20.0).abs() < 0.01);
    }

    #[test]
    fn unichar_to_x_empty() {
        let clusters: Vec<(usize, f32, u32)> = vec![];
        assert!((unichar_to_x(0, &clusters, 42.0, false) - 42.0).abs() < 0.01);
        assert!((unichar_to_x(0, &clusters, 42.0, true) - 42.0).abs() < 0.01);
    }

    #[test]
    fn unichar_to_x_single_cluster() {
        let clusters = vec![(3usize, 30.0f32, 0u32)];
        let lm = 100.0;
        assert!((unichar_to_x(0, &clusters, lm, false) - 100.0).abs() < 0.01);
        assert!((unichar_to_x(0, &clusters, lm, true) - 30.0).abs() < 0.01);
    }

    // ── x_to_unichar tests ──

    #[test]
    fn x_to_unichar_roundtrip() {
        let clusters = vec![(1usize, 10.0f32, 0u32), (2, 20.0, 1), (3, 30.0, 2)];
        // For each grapheme, the trailing x maps back to the same grapheme
        assert_eq!(x_to_unichar(10.0, &clusters, 0.0), 0);
        assert_eq!(x_to_unichar(20.0, &clusters, 0.0), 1);
        assert_eq!(x_to_unichar(30.0, &clusters, 0.0), 2);
    }

    #[test]
    fn x_to_unichar_midpoint() {
        // 3 clusters at x=10, 20, 30
        let clusters = vec![(1usize, 10.0f32, 0u32), (2, 20.0, 1), (3, 30.0, 2)];
        // Between cluster 0 (x=10) and cluster 1 (x=20), midpoint=15
        // x=14 → before midpoint → grapheme 0
        assert_eq!(x_to_unichar(14.0, &clusters, 0.0), 0);
        // x=16 → after midpoint → grapheme 1
        assert_eq!(x_to_unichar(16.0, &clusters, 0.0), 1);
    }

    #[test]
    fn x_to_unichar_before_first() {
        let clusters = vec![(1usize, 10.0f32, 0u32), (2, 20.0, 1)];
        assert_eq!(x_to_unichar(0.0, &clusters, 0.0), 0);
        assert_eq!(x_to_unichar(5.0, &clusters, 0.0), 0);
    }

    #[test]
    fn x_to_unichar_empty() {
        let clusters: Vec<(usize, f32, u32)> = vec![];
        assert_eq!(x_to_unichar(50.0, &clusters, 0.0), 0);
    }

    #[test]
    fn x_to_unichar_past_last() {
        let clusters = vec![(1usize, 10.0f32, 0u32), (2, 20.0, 1)];
        assert_eq!(x_to_unichar(100.0, &clusters, 0.0), 1);
    }

    #[test]
    fn x_to_unichar_single_cluster() {
        let clusters = vec![(3usize, 30.0f32, 0u32)];
        assert_eq!(x_to_unichar(15.0, &clusters, 0.0), 0);
        assert_eq!(x_to_unichar(0.0, &clusters, 0.0), 0);
        assert_eq!(x_to_unichar(100.0, &clusters, 0.0), 0);
    }
}
