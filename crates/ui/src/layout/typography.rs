//! Boundary spacing and final geometry for shaped prose runs.
//!
//! This module deliberately keeps source text and shaped glyphs separate. A
//! spacing candidate is a UTF-8 insertion boundary; it never creates a
//! character or a cursor stop.

use crate::typography::GapCandidate;
use shaping::{GlyphCluster, ShapedRun};

const GAP_HALF_DIVISOR: f32 = 2.0;

/// Final geometry produced from one raw shaped run.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedLayout {
    /// The raw clusters with active, same-line gaps applied exactly once.
    pub shaped: ShapedRun,
    /// `(cluster_start, cluster_end, width)` visual lines.
    pub visual_lines: Vec<(usize, usize, f32)>,
}

/// Build final shaped geometry and visual lines from boundary spacing.
#[allow(
    clippy::too_many_arguments,
    reason = "layout inputs mirror the existing visual-line API and keep geometry explicit"
)]
pub fn layout_shaped_run_with_gaps(
    raw: &ShapedRun,
    gaps: &[GapCandidate],
    line_bytes: &[u8],
    char_width: f32,
    viewport_width: f32,
    min_fill_ratio: f32,
    first_line_indent: f32,
) -> ShapedLayout {
    let normalized_gaps = map_gap_candidates(raw, gaps);
    let gap_before = gap_advances_before_clusters(raw.clusters.len(), &normalized_gaps);
    let visual_lines = super::compute_visual_lines_with_gap_advances(
        &raw.clusters,
        line_bytes,
        char_width,
        viewport_width,
        min_fill_ratio,
        first_line_indent,
        &gap_before,
        true,
    );

    let shaped = apply_mapped_gaps(raw, &normalized_gaps, &visual_lines);
    ShapedLayout { shaped, visual_lines }
}

/// Apply spacing using the final visual lines supplied by a layout cache or worker.
/// Line ranges index `raw.clusters`; candidates crossing their boundaries are cancelled.
pub fn apply_gaps_to_existing_visual_lines(
    raw: &ShapedRun,
    gaps: &[GapCandidate],
    visual_lines: &[(usize, usize, f32)],
) -> ShapedRun {
    let normalized_gaps = map_gap_candidates(raw, gaps);
    apply_mapped_gaps(raw, &normalized_gaps, visual_lines)
}

fn apply_mapped_gaps(
    raw: &ShapedRun,
    gaps: &[MappedGap],
    visual_lines: &[(usize, usize, f32)],
) -> ShapedRun {
    let mut shaped = raw.clone();
    shaped.width = raw.width + apply_same_line_gaps(&mut shaped.clusters, gaps, visual_lines);
    shaped
}

/// Compute visual lines using boundary spacing, without producing draw data.
#[allow(
    clippy::too_many_arguments,
    reason = "kept parallel to layout_shaped_run_with_gaps for measurement callers"
)]
pub fn compute_visual_lines_with_gaps(
    clusters: &[GlyphCluster],
    gaps: &[GapCandidate],
    line_bytes: &[u8],
    char_width: f32,
    viewport_width: f32,
    min_fill_ratio: f32,
    first_line_indent: f32,
) -> Vec<(usize, usize, f32)> {
    let raw = ShapedRun { clusters: clusters.to_vec(), width: 0.0 };
    let normalized_gaps = map_gap_candidates(&raw, gaps);
    let gap_before = gap_advances_before_clusters(clusters.len(), &normalized_gaps);
    super::compute_visual_lines_with_gap_advances(
        clusters,
        line_bytes,
        char_width,
        viewport_width,
        min_fill_ratio,
        first_line_indent,
        &gap_before,
        true,
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MappedGap {
    left_cluster: usize,
    right_cluster: usize,
    advance: f32,
}

fn map_gap_candidates(raw: &ShapedRun, gaps: &[GapCandidate]) -> Vec<MappedGap> {
    let mut candidates: Vec<(usize, f32)> = gaps
        .iter()
        .filter(|candidate| candidate.advance.is_finite() && candidate.advance > 0.0)
        .map(|candidate| (candidate.boundary, candidate.advance))
        .collect();
    if !candidates.windows(2).all(|pair| pair[0].0 <= pair[1].0) {
        candidates.sort_unstable_by_key(|(boundary, _)| *boundary);
    }

    let mut unique_candidates: Vec<(usize, f32)> = Vec::with_capacity(candidates.len());
    for (boundary, advance) in candidates {
        if let Some((previous_boundary, previous_advance)) = unique_candidates.last_mut()
            && *previous_boundary == boundary
        {
            *previous_advance = (*previous_advance).max(advance);
        } else {
            unique_candidates.push((boundary, advance));
        }
    }

    let mut mapped: Vec<MappedGap> = Vec::with_capacity(unique_candidates.len());
    let mut cluster_pair_index: usize = 0;
    for (boundary, advance) in unique_candidates {
        while cluster_pair_index + 1 < raw.clusters.len()
            && raw.clusters[cluster_pair_index].byte_range.end < boundary
        {
            cluster_pair_index += 1;
        }
        while cluster_pair_index + 1 < raw.clusters.len()
            && raw.clusters[cluster_pair_index].byte_range.end == boundary
            && raw.clusters[cluster_pair_index + 1].byte_range.start != boundary
        {
            cluster_pair_index += 1;
        }
        if cluster_pair_index + 1 >= raw.clusters.len()
            || raw.clusters[cluster_pair_index].byte_range.end != boundary
            || raw.clusters[cluster_pair_index + 1].byte_range.start != boundary
        {
            continue;
        }
        mapped.push(MappedGap {
            left_cluster: cluster_pair_index,
            right_cluster: cluster_pair_index + 1,
            advance,
        });
    }
    mapped
}

fn gap_advances_before_clusters(cluster_count: usize, gaps: &[MappedGap]) -> Vec<f32> {
    let mut gap_before = vec![0.0f32; cluster_count + 1];
    for gap in gaps {
        gap_before[gap.right_cluster] = gap_before[gap.right_cluster].max(gap.advance);
    }
    gap_before
}

fn apply_same_line_gaps(
    clusters: &mut [GlyphCluster],
    gaps: &[MappedGap],
    visual_lines: &[(usize, usize, f32)],
) -> f32 {
    let line_for_cluster = line_indices(clusters.len(), visual_lines);
    let mut active_width = 0.0;
    for gap in gaps {
        if line_for_cluster[gap.left_cluster].is_none()
            || line_for_cluster[gap.left_cluster] != line_for_cluster[gap.right_cluster]
        {
            continue;
        }
        let half_gap = gap.advance / GAP_HALF_DIVISOR;
        clusters[gap.left_cluster].advance += half_gap;
        let right_range = clusters[gap.right_cluster].byte_range.clone();
        let mut right_cluster = gap.right_cluster;
        let mut last_right_cluster = None;
        while right_cluster < clusters.len() && clusters[right_cluster].byte_range == right_range {
            if line_for_cluster[right_cluster] == line_for_cluster[gap.right_cluster] {
                clusters[right_cluster].x_offset += half_gap;
                last_right_cluster = Some(right_cluster);
            }
            right_cluster += 1;
        }
        if let Some(last_right_cluster) = last_right_cluster {
            clusters[last_right_cluster].advance += half_gap;
        }
        active_width += gap.advance;
    }
    active_width
}

fn line_indices(cluster_count: usize, visual_lines: &[(usize, usize, f32)]) -> Vec<Option<usize>> {
    let mut line_for_cluster = vec![None; cluster_count];
    for (line_index, &(start, end, _)) in visual_lines.iter().enumerate() {
        for (_, line_index_slot) in line_for_cluster
            .iter_mut()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start).min(cluster_count.saturating_sub(start)))
        {
            *line_index_slot = Some(line_index);
        }
    }
    line_for_cluster
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::*;

    fn cluster(start: usize, end: usize, advance: f32) -> GlyphCluster {
        GlyphCluster {
            byte_range: Range { start, end },
            glyph_id: 0,
            font_id: Default::default(),
            advance,
            x_offset: 0.0,
            y_offset: 0.0,
        }
    }

    fn candidate(boundary: usize, advance: f32) -> GapCandidate {
        GapCandidate { boundary, advance }
    }

    #[test]
    fn existing_wrap_boundaries_control_active_gaps() {
        let raw = ShapedRun {
            clusters: vec![cluster(0, 3, 10.0), cluster(3, 6, 10.0), cluster(6, 9, 10.0)],
            width: 30.0,
        };
        let gaps = [candidate(3, 4.0), candidate(6, 4.0)];
        let visual_lines = [(0, 1, 10.0), (1, 3, 24.0)];
        let shaped = apply_gaps_to_existing_visual_lines(&raw, &gaps, &visual_lines);
        assert_eq!(shaped.clusters[0].advance, 10.0);
        assert_eq!(shaped.clusters[1].advance, 12.0);
        assert_eq!(shaped.clusters[2].advance, 12.0);
        assert_eq!(shaped.width, 34.0);
        assert_eq!(shaped.clusters[1].x_offset, 0.0);
        assert_eq!(shaped.clusters[2].x_offset, 2.0);
    }

    #[test]
    fn gap_is_split_between_adjacent_clusters_once() {
        let raw =
            ShapedRun { clusters: vec![cluster(0, 3, 10.0), cluster(3, 6, 10.0)], width: 20.0 };

        let layout = layout_shaped_run_with_gaps(
            &raw,
            &[candidate(3, 4.0)],
            "甲乙".as_bytes(),
            10.0,
            100.0,
            0.5,
            0.0,
        );

        assert_eq!(layout.visual_lines, vec![(0, 2, 24.0)]);
        assert_eq!(layout.shaped.clusters[0].advance, 12.0);
        assert_eq!(layout.shaped.clusters[1].x_offset, 2.0);
        assert_eq!(layout.shaped.width, 24.0);
        assert_eq!(raw.clusters[0].advance, 10.0);
    }

    #[test]
    fn gap_at_soft_wrap_is_cancelled_from_both_geometries() {
        let raw =
            ShapedRun { clusters: vec![cluster(0, 3, 10.0), cluster(3, 6, 10.0)], width: 20.0 };

        let layout = layout_shaped_run_with_gaps(
            &raw,
            &[candidate(3, 4.0)],
            "甲乙".as_bytes(),
            10.0,
            20.0,
            0.5,
            0.0,
        );

        assert_eq!(layout.visual_lines, vec![(0, 1, 10.0), (1, 2, 10.0)]);
        assert_eq!(layout.shaped, raw);
    }

    #[test]
    fn duplicate_glyph_range_receives_one_boundary_gap() {
        let raw = ShapedRun {
            clusters: vec![cluster(0, 3, 0.0), cluster(0, 3, 0.0), cluster(3, 6, 10.0)],
            width: 10.0,
        };

        let layout = layout_shaped_run_with_gaps(
            &raw,
            &[candidate(3, 4.0), candidate(3, 2.0)],
            "甲乙".as_bytes(),
            10.0,
            100.0,
            0.5,
            0.0,
        );

        assert_eq!(layout.visual_lines, vec![(0, 3, 14.0)]);
        assert_eq!(layout.shaped.clusters[0].advance, 0.0);
        assert_eq!(layout.shaped.clusters[1].advance, 2.0);
        assert_eq!(layout.shaped.clusters[2].x_offset, 2.0);
        assert_eq!(layout.shaped.width, 14.0);
    }

    #[test]
    fn right_multi_glyph_cluster_keeps_internal_positions_after_gap() {
        let raw = ShapedRun {
            clusters: vec![
                cluster(0, 3, 10.0),
                cluster(3, 6, 10.0),
                GlyphCluster { x_offset: -10.0, ..cluster(3, 6, 0.0) },
            ],
            width: 20.0,
        };

        let layout = layout_shaped_run_with_gaps(
            &raw,
            &[candidate(3, 4.0)],
            "甲乙".as_bytes(),
            10.0,
            100.0,
            0.5,
            0.0,
        );

        let mut pen = 0.0;
        let positions = layout
            .shaped
            .clusters
            .iter()
            .map(|glyph| {
                let position = pen + glyph.x_offset;
                pen += glyph.advance;
                position
            })
            .collect::<Vec<_>>();
        assert_eq!(positions, [0.0, 14.0, 14.0]);
        assert_eq!(layout.shaped.width, 24.0);
    }

    #[test]
    fn gap_width_is_internal_to_a_range_only() {
        let clusters = vec![cluster(0, 1, 10.0), cluster(1, 2, 10.0)];
        let lines = compute_visual_lines_with_gaps(
            &clusters,
            &[candidate(1, 4.0)],
            b"ab",
            10.0,
            24.0,
            0.5,
            0.0,
        );
        assert_eq!(lines, vec![(0, 2, 24.0)]);
    }

    #[test]
    fn first_line_indent_applies_with_gaps() {
        let clusters = vec![cluster(0, 3, 10.0), cluster(3, 6, 10.0), cluster(6, 9, 10.0)];
        let lines = compute_visual_lines_with_gaps(
            &clusters,
            &[candidate(3, 4.0)],
            "甲乙丙".as_bytes(),
            10.0,
            24.0,
            0.5,
            4.0,
        );
        assert_eq!(lines, vec![(0, 1, 10.0), (1, 3, 20.0)]);
    }

    #[test]
    fn opening_bracket_does_not_end_a_new_layout_line() {
        let clusters = vec![cluster(0, 3, 10.0), cluster(3, 4, 10.0), cluster(4, 7, 10.0)];
        let lines = compute_visual_lines_with_gaps(
            &clusters,
            &[],
            "甲(乙".as_bytes(),
            10.0,
            20.0,
            0.5,
            0.0,
        );
        assert_eq!(lines, vec![(0, 1, 10.0), (1, 3, 20.0)]);
    }

    #[test]
    fn multi_glyph_cluster_is_not_split_at_a_wrap_boundary() {
        let clusters = vec![cluster(0, 3, 6.0), cluster(0, 3, 4.0), cluster(3, 6, 10.0)];
        let lines =
            compute_visual_lines_with_gaps(&clusters, &[], "甲乙".as_bytes(), 10.0, 7.0, 0.5, 0.0);
        assert_eq!(lines, vec![(0, 2, 10.0), (2, 3, 10.0)]);
    }
}
