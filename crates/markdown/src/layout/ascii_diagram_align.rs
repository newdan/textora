use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::layout::ascii_diagram) struct RectangleEdge {
    pub(in crate::layout::ascii_diagram) row_index: usize,
    pub(in crate::layout::ascii_diagram) left_cell_index: usize,
    pub(in crate::layout::ascii_diagram) right_cell_index: usize,
    pub(in crate::layout::ascii_diagram) left_column: usize,
    pub(in crate::layout::ascii_diagram) right_column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::layout::ascii_diagram) struct RectangleCandidate {
    pub(in crate::layout::ascii_diagram) top: RectangleEdge,
    pub(in crate::layout::ascii_diagram) bottom: RectangleEdge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::layout::ascii_diagram) enum BorderSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::layout::ascii_diagram) struct BorderMember {
    pub(in crate::layout::ascii_diagram) row_index: usize,
    pub(in crate::layout::ascii_diagram) cell_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::layout::ascii_diagram) struct VerticalBorderTrack {
    pub(in crate::layout::ascii_diagram) rectangle_index: usize,
    pub(in crate::layout::ascii_diagram) side: BorderSide,
    pub(in crate::layout::ascii_diagram) members: Vec<BorderMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::layout::ascii_diagram) struct OpenVerticalTrack {
    pub(in crate::layout::ascii_diagram) members: Vec<BorderMember>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::layout::ascii_diagram) struct RowTrackAssignment {
    pub(in crate::layout::ascii_diagram) track_index: usize,
    pub(in crate::layout::ascii_diagram) cell_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::layout::ascii_diagram) struct CandidatePosition {
    pub(in crate::layout::ascii_diagram) expected_position: usize,
    pub(in crate::layout::ascii_diagram) cell_position: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::layout::ascii_diagram) struct AdjacentTrackPair {
    pub(in crate::layout::ascii_diagram) left_track_index: usize,
    pub(in crate::layout::ascii_diagram) right_track_index: usize,
}

pub(in crate::layout::ascii_diagram) type SupportedAdjacentGaps =
    BTreeMap<AdjacentTrackPair, BTreeSet<usize>>;

pub(in crate::layout::ascii_diagram) struct TrackSupportEvidence {
    pub(in crate::layout::ascii_diagram) columns: Vec<BTreeSet<usize>>,
    pub(in crate::layout::ascii_diagram) spans: Vec<BTreeSet<usize>>,
    pub(in crate::layout::ascii_diagram) adjacent_gaps: SupportedAdjacentGaps,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::layout::ascii_diagram) struct AssignmentEvidence {
    pub(in crate::layout::ascii_diagram) directly_supported: bool,
    pub(in crate::layout::ascii_diagram) atomic_partners: BTreeSet<CandidatePosition>,
}

pub(in crate::layout::ascii_diagram) fn corner_spans(
    row_index: usize,
    row: &AsciiDiagramRow,
    left_corner: &str,
    right_corner: &str,
) -> Vec<RectangleEdge> {
    let mut left_stack = Vec::new();
    let mut spans = Vec::new();

    for (cell_index, cell) in row.cells.iter().enumerate() {
        if cell.text == left_corner {
            left_stack.push((cell_index, cell.column));
            continue;
        }
        if cell.text != right_corner {
            continue;
        }
        let Some((left_cell_index, left_column)) = left_stack.pop() else {
            continue;
        };
        if left_column < cell.column {
            spans.push(RectangleEdge {
                row_index,
                left_cell_index,
                right_cell_index: cell_index,
                left_column,
                right_column: cell.column,
            });
        }
    }

    spans.sort_by_key(|span| span.left_column);
    spans
}

pub(in crate::layout::ascii_diagram) fn horizontal_overlap(
    left: RectangleEdge,
    right: RectangleEdge,
) -> usize {
    left.right_column
        .min(right.right_column)
        .saturating_sub(left.left_column.max(right.left_column))
}

pub(in crate::layout::ascii_diagram) fn rectangle_candidates(
    rows: &[AsciiDiagramRow],
) -> Vec<RectangleCandidate> {
    let mut rectangles = Vec::new();
    let mut open_top_edges = Vec::new();

    for (row_index, row) in rows.iter().enumerate() {
        for bottom in corner_spans(row_index, row, "└", "┘") {
            let Some(nearest_open_row) = open_top_edges
                .iter()
                .filter(|top| horizontal_overlap(**top, bottom) > 0)
                .map(|top| top.row_index)
                .max()
            else {
                continue;
            };

            let mut best_open_index = None;
            let mut best_overlap = 0usize;
            let mut best_is_ambiguous = false;
            for (open_index, top) in open_top_edges.iter().enumerate() {
                if top.row_index != nearest_open_row {
                    continue;
                }
                let overlap = horizontal_overlap(*top, bottom);
                if overlap == 0 {
                    continue;
                }
                if overlap > best_overlap {
                    best_open_index = Some(open_index);
                    best_overlap = overlap;
                    best_is_ambiguous = false;
                } else if overlap == best_overlap {
                    best_is_ambiguous = true;
                }
            }

            let Some(best_open_index) = best_open_index.filter(|_| !best_is_ambiguous) else {
                continue;
            };
            let top = open_top_edges.remove(best_open_index);
            rectangles.push(RectangleCandidate { top, bottom });
        }

        open_top_edges.extend(corner_spans(row_index, row, "┌", "┐"));
    }

    rectangles.sort_by_key(|rectangle| (rectangle.top.row_index, rectangle.top.left_column));
    rectangles
}

pub(in crate::layout::ascii_diagram) fn is_vertical_edge(cell: &AsciiDiagramCell) -> bool {
    cell.box_connections.is_some_and(|connections| connections.up && connections.down)
}

pub(in crate::layout::ascii_diagram) fn active_rectangle_indices(
    row_index: usize,
    candidates: &[RectangleCandidate],
) -> Vec<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, rectangle)| {
            rectangle.top.row_index < row_index && row_index < rectangle.bottom.row_index
        })
        .map(|(rectangle_index, _)| rectangle_index)
        .collect()
}

pub(in crate::layout::ascii_diagram) fn border_track_index(
    rectangle_index: usize,
    side: BorderSide,
) -> usize {
    rectangle_index * 2
        + match side {
            BorderSide::Left => 0,
            BorderSide::Right => 1,
        }
}

pub(in crate::layout::ascii_diagram) fn track_source_column(
    track: &VerticalBorderTrack,
    candidates: &[RectangleCandidate],
) -> usize {
    let rectangle = candidates[track.rectangle_index];
    match track.side {
        BorderSide::Left => rectangle.top.left_column.min(rectangle.bottom.left_column),
        BorderSide::Right => rectangle.top.right_column.max(rectangle.bottom.right_column),
    }
}

pub(in crate::layout::ascii_diagram) fn expected_track_indices(
    row_index: usize,
    candidates: &[RectangleCandidate],
    tracks: &[VerticalBorderTrack],
) -> Vec<usize> {
    let mut track_indices = active_rectangle_indices(row_index, candidates)
        .into_iter()
        .flat_map(|rectangle_index| {
            [
                border_track_index(rectangle_index, BorderSide::Left),
                border_track_index(rectangle_index, BorderSide::Right),
            ]
        })
        .collect::<Vec<_>>();
    track_indices.sort_by_key(|track_index| {
        let track = &tracks[*track_index];
        (
            track_source_column(track, candidates),
            usize::from(track.side == BorderSide::Right),
            track.rectangle_index,
        )
    });
    track_indices
}

pub(in crate::layout::ascii_diagram) fn vertical_edge_cell_indices(
    row: &AsciiDiagramRow,
) -> Vec<usize> {
    row.cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| is_vertical_edge(cell))
        .map(|(cell_index, _)| cell_index)
        .collect()
}

pub(in crate::layout::ascii_diagram) fn record_confident_row(
    row_index: usize,
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    tracks: &mut [VerticalBorderTrack],
    support: &mut TrackSupportEvidence,
) {
    for (&track_index, &cell_index) in expected_track_indices.iter().zip(cell_indices) {
        support.columns[track_index].insert(row.cells[cell_index].column);
        tracks[track_index].members.push(BorderMember { row_index, cell_index });
    }

    for rectangle_index in tracks.iter().map(|track| track.rectangle_index).collect::<BTreeSet<_>>()
    {
        let left_track_index = border_track_index(rectangle_index, BorderSide::Left);
        let right_track_index = border_track_index(rectangle_index, BorderSide::Right);
        let left_position =
            expected_track_indices.iter().position(|index| *index == left_track_index);
        let right_position =
            expected_track_indices.iter().position(|index| *index == right_track_index);
        let (Some(left_position), Some(right_position)) = (left_position, right_position) else {
            continue;
        };
        let left_column = row.cells[cell_indices[left_position]].column;
        let right_column = row.cells[cell_indices[right_position]].column;
        support.spans[rectangle_index].insert(right_column.saturating_sub(left_column));
    }

    record_adjacent_track_gaps(
        row,
        expected_track_indices,
        cell_indices,
        &mut support.adjacent_gaps,
    );
}

pub(in crate::layout::ascii_diagram) fn record_adjacent_track_gaps(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    supported_adjacent_gaps: &mut SupportedAdjacentGaps,
) {
    for (expected_pair, cell_pair) in expected_track_indices.windows(2).zip(cell_indices.windows(2))
    {
        let left_column = row.cells[cell_pair[0]].column;
        let right_column = row.cells[cell_pair[1]].column;
        let Some(gap) = right_column.checked_sub(left_column) else {
            continue;
        };
        if gap == 0 {
            continue;
        }
        supported_adjacent_gaps
            .entry(AdjacentTrackPair {
                left_track_index: expected_pair[0],
                right_track_index: expected_pair[1],
            })
            .or_default()
            .insert(gap);
    }
}

pub(in crate::layout::ascii_diagram) fn directly_supported_candidates(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    supported_columns: &[BTreeSet<usize>],
) -> BTreeMap<CandidatePosition, AssignmentEvidence> {
    let mut candidates = BTreeMap::new();
    for (expected_position, track_index) in expected_track_indices.iter().copied().enumerate() {
        for (cell_position, cell_index) in cell_indices.iter().copied().enumerate() {
            if supported_columns[track_index].contains(&row.cells[cell_index].column) {
                candidates.entry(CandidatePosition { expected_position, cell_position }).or_insert(
                    AssignmentEvidence {
                        directly_supported: true,
                        ..AssignmentEvidence::default()
                    },
                );
            }
        }
    }
    candidates
}

pub(in crate::layout::ascii_diagram) fn expected_rectangle_side_positions(
    expected_track_indices: &[usize],
    tracks: &[VerticalBorderTrack],
) -> Vec<[Option<usize>; 2]> {
    let rectangle_count = tracks.iter().map(|track| track.rectangle_index + 1).max().unwrap_or(0);
    let mut positions = vec![[None, None]; rectangle_count];
    for (expected_position, track_index) in expected_track_indices.iter().copied().enumerate() {
        let track = &tracks[track_index];
        let side_index = match track.side {
            BorderSide::Left => 0,
            BorderSide::Right => 1,
        };
        positions[track.rectangle_index][side_index] = Some(expected_position);
    }
    positions
}

pub(in crate::layout::ascii_diagram) fn record_atomic_candidate_pair(
    candidates: &mut BTreeMap<CandidatePosition, AssignmentEvidence>,
    left: CandidatePosition,
    right: CandidatePosition,
) {
    candidates.entry(left).or_default().atomic_partners.insert(right);
    candidates.entry(right).or_default().atomic_partners.insert(left);
}

pub(in crate::layout::ascii_diagram) fn add_span_supported_candidates(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    tracks: &[VerticalBorderTrack],
    supported_spans: &[BTreeSet<usize>],
    supported_adjacent_gaps: &SupportedAdjacentGaps,
    candidates: &mut BTreeMap<CandidatePosition, AssignmentEvidence>,
) {
    let side_positions = expected_rectangle_side_positions(expected_track_indices, tracks);
    let cell_positions_by_column = cell_indices
        .iter()
        .enumerate()
        .map(|(cell_position, cell_index)| (row.cells[*cell_index].column, cell_position))
        .collect::<BTreeMap<_, _>>();

    for (rectangle_index, spans) in supported_spans.iter().enumerate() {
        let [Some(left_expected), Some(right_expected)] = side_positions[rectangle_index] else {
            continue;
        };
        for (left_cell_position, left_cell_index) in cell_indices.iter().copied().enumerate() {
            let left_column = row.cells[left_cell_index].column;
            for span in spans {
                let Some(right_column) = left_column.checked_add(*span) else {
                    continue;
                };
                let Some(right_cell_position) = cell_positions_by_column.get(&right_column) else {
                    continue;
                };
                if left_expected >= right_expected || left_cell_position >= *right_cell_position {
                    continue;
                }
                let left = CandidatePosition {
                    expected_position: left_expected,
                    cell_position: left_cell_position,
                };
                let right = CandidatePosition {
                    expected_position: right_expected,
                    cell_position: *right_cell_position,
                };
                if should_exclude_tree_branch_conflicting_span_candidate(
                    row,
                    expected_track_indices,
                    cell_indices,
                    left,
                    right,
                    supported_adjacent_gaps,
                    candidates,
                ) {
                    continue;
                }
                record_atomic_candidate_pair(candidates, left, right);
            }
        }
    }
}

pub(in crate::layout::ascii_diagram) fn should_exclude_tree_branch_conflicting_span_candidate(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    left: CandidatePosition,
    right: CandidatePosition,
    supported_adjacent_gaps: &SupportedAdjacentGaps,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> bool {
    if !has_tree_branch_junction(row)
        || !(candidate_position_conflicts_with_direct_support(left, candidates)
            || candidate_position_conflicts_with_direct_support(right, candidates))
        || right.expected_position != left.expected_position + 1
    {
        return false;
    }

    unique_direct_compatible_adjacent_gap_candidate_pairs(
        row,
        expected_track_indices,
        cell_indices,
        supported_adjacent_gaps,
        candidates,
    )
    .into_iter()
    .any(|(adjacent_left, adjacent_right)| {
        [left, right].into_iter().any(|span_position| {
            candidate_positions_cross(span_position, adjacent_left)
                || candidate_positions_cross(span_position, adjacent_right)
        })
    })
}

pub(in crate::layout::ascii_diagram) fn unique_direct_compatible_adjacent_gap_candidate_pairs(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    supported_adjacent_gaps: &SupportedAdjacentGaps,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> Vec<(CandidatePosition, CandidatePosition)> {
    expected_track_indices
        .windows(2)
        .enumerate()
        .filter_map(|(left_expected_position, expected_pair)| {
            let pair = AdjacentTrackPair {
                left_track_index: expected_pair[0],
                right_track_index: expected_pair[1],
            };
            unique_direct_compatible_adjacent_gap_candidate(
                row,
                cell_indices,
                left_expected_position,
                supported_adjacent_gaps.get(&pair)?,
                candidates,
            )
        })
        .collect()
}

pub(in crate::layout::ascii_diagram) fn unique_direct_compatible_adjacent_gap_candidate(
    row: &AsciiDiagramRow,
    cell_indices: &[usize],
    left_expected_position: usize,
    supported_gaps: &BTreeSet<usize>,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> Option<(CandidatePosition, CandidatePosition)> {
    let mut unique_candidate = None;
    for (left_cell_position, &left_cell_index) in cell_indices.iter().enumerate() {
        let left_column = row.cells[left_cell_index].column;
        for (right_offset, &right_cell_index) in
            cell_indices[left_cell_position + 1..].iter().enumerate()
        {
            let right_cell_position = left_cell_position + 1 + right_offset;
            let right_column = row.cells[right_cell_index].column;
            let Some(gap) = right_column.checked_sub(left_column) else {
                continue;
            };
            if !supported_gaps.contains(&gap) {
                continue;
            }
            let left = CandidatePosition {
                expected_position: left_expected_position,
                cell_position: left_cell_position,
            };
            let right = CandidatePosition {
                expected_position: left_expected_position + 1,
                cell_position: right_cell_position,
            };
            if candidate_position_conflicts_with_direct_support(left, candidates)
                || candidate_position_conflicts_with_direct_support(right, candidates)
            {
                continue;
            }
            if unique_candidate.replace((left, right)).is_some() {
                return None;
            }
        }
    }
    unique_candidate
}

pub(in crate::layout::ascii_diagram) fn add_adjacent_gap_supported_candidates(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    supported_adjacent_gaps: &SupportedAdjacentGaps,
    candidates: &mut BTreeMap<CandidatePosition, AssignmentEvidence>,
) {
    if !has_tree_branch_junction(row) {
        return;
    }

    for (left_expected_position, expected_pair) in expected_track_indices.windows(2).enumerate() {
        let pair = AdjacentTrackPair {
            left_track_index: expected_pair[0],
            right_track_index: expected_pair[1],
        };
        let Some(supported_gaps) = supported_adjacent_gaps.get(&pair) else {
            continue;
        };

        for (left_cell_position, &left_cell_index) in cell_indices.iter().enumerate() {
            let left_column = row.cells[left_cell_index].column;
            for (right_offset, &right_cell_index) in
                cell_indices[left_cell_position + 1..].iter().enumerate()
            {
                let right_cell_position = left_cell_position + 1 + right_offset;
                let right_column = row.cells[right_cell_index].column;
                let Some(gap) = right_column.checked_sub(left_column) else {
                    continue;
                };
                if !supported_gaps.contains(&gap) {
                    continue;
                }
                let left = CandidatePosition {
                    expected_position: left_expected_position,
                    cell_position: left_cell_position,
                };
                let right = CandidatePosition {
                    expected_position: left_expected_position + 1,
                    cell_position: right_cell_position,
                };
                if candidate_position_conflicts_with_direct_support(left, candidates)
                    || candidate_position_conflicts_with_direct_support(right, candidates)
                {
                    continue;
                }
                record_atomic_candidate_pair(candidates, left, right);
            }
        }
    }
}

pub(in crate::layout::ascii_diagram) fn has_tree_branch_junction(row: &AsciiDiagramRow) -> bool {
    row.cells.iter().any(|cell| matches!(cell.text.as_str(), "├" | "┤"))
}

pub(in crate::layout::ascii_diagram) fn candidate_position_conflicts_with_direct_support(
    position: CandidatePosition,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> bool {
    candidates.iter().any(|(other, evidence)| {
        *other != position
            && evidence.directly_supported
            && (other.expected_position == position.expected_position
                || other.cell_position == position.cell_position)
    })
}

pub(in crate::layout::ascii_diagram) fn mutually_unique_candidate_positions(
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> BTreeSet<CandidatePosition> {
    let mut expected_counts = BTreeMap::new();
    let mut cell_counts = BTreeMap::new();
    for position in candidates.keys() {
        *expected_counts.entry(position.expected_position).or_insert(0usize) += 1;
        *cell_counts.entry(position.cell_position).or_insert(0usize) += 1;
    }
    candidates
        .keys()
        .filter(|position| {
            expected_counts[&position.expected_position] == 1
                && cell_counts[&position.cell_position] == 1
        })
        .copied()
        .collect()
}

pub(in crate::layout::ascii_diagram) fn candidate_positions_cross(
    left: CandidatePosition,
    right: CandidatePosition,
) -> bool {
    (left.expected_position < right.expected_position && left.cell_position > right.cell_position)
        || (right.expected_position < left.expected_position
            && right.cell_position > left.cell_position)
}

pub(in crate::layout::ascii_diagram) fn positions_without_crossing_candidates(
    positions: &BTreeSet<CandidatePosition>,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> BTreeSet<CandidatePosition> {
    positions
        .iter()
        .filter(|position| {
            !candidates.keys().any(|candidate| {
                **position != *candidate && candidate_positions_cross(**position, *candidate)
            })
        })
        .copied()
        .collect()
}

pub(in crate::layout::ascii_diagram) fn supported_row_candidates(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    tracks: &[VerticalBorderTrack],
    supported_columns: &[BTreeSet<usize>],
    supported_spans: &[BTreeSet<usize>],
    supported_adjacent_gaps: &SupportedAdjacentGaps,
) -> BTreeMap<CandidatePosition, AssignmentEvidence> {
    let mut candidates =
        directly_supported_candidates(row, expected_track_indices, cell_indices, supported_columns);
    add_span_supported_candidates(
        row,
        expected_track_indices,
        cell_indices,
        tracks,
        supported_spans,
        supported_adjacent_gaps,
        &mut candidates,
    );
    add_adjacent_gap_supported_candidates(
        row,
        expected_track_indices,
        cell_indices,
        supported_adjacent_gaps,
        &mut candidates,
    );
    candidates
}

pub(in crate::layout::ascii_diagram) fn has_complete_forced_atomic_partner(
    position: CandidatePosition,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
    forced_positions: &BTreeSet<CandidatePosition>,
) -> bool {
    candidates[&position].atomic_partners.iter().any(|partner| {
        forced_positions.contains(partner)
            && candidates
                .get(partner)
                .is_some_and(|evidence| evidence.atomic_partners.contains(&position))
    })
}

pub(in crate::layout::ascii_diagram) fn directly_supported_boundary_positions(
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
    expected_count: usize,
    cell_count: usize,
) -> BTreeSet<CandidatePosition> {
    if expected_count == 0 || cell_count == 0 {
        return BTreeSet::new();
    }
    [
        CandidatePosition { expected_position: 0, cell_position: 0 },
        CandidatePosition { expected_position: expected_count - 1, cell_position: cell_count - 1 },
    ]
    .into_iter()
    .filter(|position| {
        candidates.get(position).is_some_and(|evidence| evidence.directly_supported)
            && !has_direct_axis_competitor(*position, candidates)
    })
    .collect()
}

pub(in crate::layout::ascii_diagram) fn has_direct_axis_competitor(
    position: CandidatePosition,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> bool {
    candidates.iter().any(|(other, evidence)| {
        *other != position
            && evidence.directly_supported
            && (other.expected_position == position.expected_position
                || other.cell_position == position.cell_position)
    })
}

pub(in crate::layout::ascii_diagram) fn unique_reciprocal_atomic_partner(
    position: CandidatePosition,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> Option<CandidatePosition> {
    let mut partners = candidates[&position].atomic_partners.iter().copied().filter(|partner| {
        candidates.get(partner).is_some_and(|evidence| evidence.atomic_partners.contains(&position))
    });
    let partner = partners.next()?;
    partners.next().is_none().then_some(partner)
}

pub(in crate::layout::ascii_diagram) fn position_is_compatible(
    position: CandidatePosition,
    selected: &BTreeSet<CandidatePosition>,
) -> bool {
    selected.iter().all(|other| positions_are_compatible(position, *other))
}

pub(in crate::layout::ascii_diagram) fn positions_are_compatible(
    left: CandidatePosition,
    right: CandidatePosition,
) -> bool {
    left == right
        || (left.expected_position != right.expected_position
            && left.cell_position != right.cell_position
            && !candidate_positions_cross(left, right))
}

pub(in crate::layout::ascii_diagram) fn position_crosses_candidate_compatible_with_boundaries(
    position: CandidatePosition,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
    boundaries: &BTreeSet<CandidatePosition>,
) -> bool {
    candidates.keys().any(|candidate| {
        *candidate != position
            && boundaries.iter().all(|boundary| positions_are_compatible(*candidate, *boundary))
            && candidate_positions_cross(position, *candidate)
    })
}

pub(in crate::layout::ascii_diagram) fn include_direct_boundary_anchors(
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
    expected_count: usize,
    cell_count: usize,
    selected: &mut BTreeSet<CandidatePosition>,
) {
    let boundaries = directly_supported_boundary_positions(candidates, expected_count, cell_count);
    for boundary in &boundaries {
        if selected.contains(boundary) || position_is_compatible(*boundary, selected) {
            selected.insert(*boundary);
        }
    }

    let active_boundaries = boundaries
        .iter()
        .copied()
        .filter(|boundary| selected.contains(boundary))
        .collect::<BTreeSet<_>>();

    for boundary in boundaries {
        let Some(partner) = unique_reciprocal_atomic_partner(boundary, candidates) else {
            continue;
        };
        if selected.contains(&boundary)
            && position_is_compatible(partner, selected)
            && !position_crosses_candidate_compatible_with_boundaries(
                partner,
                candidates,
                &active_boundaries,
            )
        {
            selected.insert(partner);
        }
    }
}

/// Builds `K <= expected_tracks * cells` candidates once. Direct evidence costs
/// `O(expected_tracks * cells * log S)`. Rectangle-span evidence costs
/// `O(cells * sum(supported_spans_per_rectangle) * log cells)`. Adjacent-gap evidence costs
/// `O(expected_adjacent_pairs * cells^2 * log G)`. Ordered-map insertion adds logarithmic
/// factors. Uniqueness costs `O(K log K)`, complete-set crossing rejection costs `O(K^2)`,
/// and atomic-partner validation costs `O(P log K)` for `P` partner relationships. Storage is
/// `O(K + P)`; filtering never rebuilds or re-uniquifies the candidate set.
pub(in crate::layout::ascii_diagram) fn uniquely_supported_row_assignments(
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    tracks: &[VerticalBorderTrack],
    supported_columns: &[BTreeSet<usize>],
    supported_spans: &[BTreeSet<usize>],
    supported_adjacent_gaps: &SupportedAdjacentGaps,
) -> Vec<RowTrackAssignment> {
    let candidates = supported_row_candidates(
        row,
        expected_track_indices,
        cell_indices,
        tracks,
        supported_columns,
        supported_spans,
        supported_adjacent_gaps,
    );
    let unique_positions = mutually_unique_candidate_positions(&candidates);
    let mut forced_positions =
        positions_without_crossing_candidates(&unique_positions, &candidates);
    include_direct_boundary_anchors(
        &candidates,
        expected_track_indices.len(),
        cell_indices.len(),
        &mut forced_positions,
    );

    forced_positions
        .iter()
        .copied()
        .filter(|position| {
            candidates[position].directly_supported
                || has_complete_forced_atomic_partner(*position, &candidates, &forced_positions)
        })
        .map(|position| RowTrackAssignment {
            track_index: expected_track_indices[position.expected_position],
            cell_index: cell_indices[position.cell_position],
        })
        .collect()
}

pub(in crate::layout::ascii_diagram) fn record_uniquely_supported_row(
    row_index: usize,
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    tracks: &mut [VerticalBorderTrack],
    support: &TrackSupportEvidence,
) {
    for assignment in uniquely_supported_row_assignments(
        row,
        expected_track_indices,
        cell_indices,
        tracks,
        &support.columns,
        &support.spans,
        &support.adjacent_gaps,
    ) {
        tracks[assignment.track_index]
            .members
            .push(BorderMember { row_index, cell_index: assignment.cell_index });
    }
}

pub(in crate::layout::ascii_diagram) fn new_vertical_border_tracks(
    candidates: &[RectangleCandidate],
) -> Vec<VerticalBorderTrack> {
    candidates
        .iter()
        .enumerate()
        .flat_map(|(rectangle_index, rectangle)| {
            [
                VerticalBorderTrack {
                    rectangle_index,
                    side: BorderSide::Left,
                    members: vec![
                        BorderMember {
                            row_index: rectangle.top.row_index,
                            cell_index: rectangle.top.left_cell_index,
                        },
                        BorderMember {
                            row_index: rectangle.bottom.row_index,
                            cell_index: rectangle.bottom.left_cell_index,
                        },
                    ],
                },
                VerticalBorderTrack {
                    rectangle_index,
                    side: BorderSide::Right,
                    members: vec![
                        BorderMember {
                            row_index: rectangle.top.row_index,
                            cell_index: rectangle.top.right_cell_index,
                        },
                        BorderMember {
                            row_index: rectangle.bottom.row_index,
                            cell_index: rectangle.bottom.right_cell_index,
                        },
                    ],
                },
            ]
        })
        .collect()
}

pub(in crate::layout::ascii_diagram) fn corner_support(
    candidates: &[RectangleCandidate],
    tracks: &[VerticalBorderTrack],
) -> (Vec<BTreeSet<usize>>, Vec<BTreeSet<usize>>) {
    let mut supported_columns = vec![BTreeSet::new(); tracks.len()];
    let mut supported_spans = vec![BTreeSet::new(); candidates.len()];
    for (rectangle_index, rectangle) in candidates.iter().enumerate() {
        let left_track_index = border_track_index(rectangle_index, BorderSide::Left);
        let right_track_index = border_track_index(rectangle_index, BorderSide::Right);
        supported_columns[left_track_index]
            .extend([rectangle.top.left_column, rectangle.bottom.left_column]);
        supported_columns[right_track_index]
            .extend([rectangle.top.right_column, rectangle.bottom.right_column]);
        supported_spans[rectangle_index].extend([
            rectangle.top.right_column - rectangle.top.left_column,
            rectangle.bottom.right_column - rectangle.bottom.left_column,
        ]);
    }
    (supported_columns, supported_spans)
}

pub(in crate::layout::ascii_diagram) fn vertical_border_tracks(
    rows: &[AsciiDiagramRow],
    candidates: &[RectangleCandidate],
) -> Vec<VerticalBorderTrack> {
    let mut tracks = new_vertical_border_tracks(candidates);
    let (columns, spans) = corner_support(candidates, &tracks);
    let mut support =
        TrackSupportEvidence { columns, spans, adjacent_gaps: SupportedAdjacentGaps::new() };
    let mut uncertain_rows = Vec::new();

    for (row_index, row) in rows.iter().enumerate() {
        let expected_track_indices = expected_track_indices(row_index, candidates, &tracks);
        if expected_track_indices.is_empty() {
            continue;
        }
        let cell_indices = vertical_edge_cell_indices(row);
        if cell_indices.len() == expected_track_indices.len() {
            record_confident_row(
                row_index,
                row,
                &expected_track_indices,
                &cell_indices,
                &mut tracks,
                &mut support,
            );
        } else {
            uncertain_rows.push((row_index, expected_track_indices, cell_indices));
        }
    }

    for (row_index, expected_track_indices, cell_indices) in uncertain_rows {
        record_uniquely_supported_row(
            row_index,
            &rows[row_index],
            &expected_track_indices,
            &cell_indices,
            &mut tracks,
            &support,
        );
    }
    for track in &mut tracks {
        track.members.sort_by_key(|member| (member.row_index, member.cell_index));
        track.members.dedup();
    }
    tracks
}

pub(in crate::layout::ascii_diagram) fn open_track_row_members(
    row_index: usize,
    row: &AsciiDiagramRow,
    expected_count: usize,
    rectangle_members: &BTreeSet<BorderMember>,
) -> Option<Vec<usize>> {
    // A horizontal spine is a group boundary. This keeps every pure vertical segment
    // reachable only from its nearest spine above and below instead of rescanning a table.
    if !open_timeline_anchor_indices(row).is_empty() {
        return None;
    }

    let member_indices = open_timeline_member_indices(row)
        .into_iter()
        .filter(|cell_index| {
            !rectangle_members.contains(&BorderMember { row_index, cell_index: *cell_index })
        })
        .collect::<Vec<_>>();
    (member_indices.len() == expected_count).then_some(member_indices)
}

pub(in crate::layout::ascii_diagram) fn contiguous_open_track_rows(
    rows: &[AsciiDiagramRow],
    spine_row_index: usize,
    spine_anchor_indices: Vec<usize>,
    rectangle_members: &BTreeSet<BorderMember>,
) -> Vec<(usize, Vec<usize>)> {
    let expected_count = spine_anchor_indices.len();
    let mut member_rows = vec![(spine_row_index, spine_anchor_indices)];

    for row_index in (0..spine_row_index).rev() {
        let Some(member_indices) =
            open_track_row_members(row_index, &rows[row_index], expected_count, rectangle_members)
        else {
            break;
        };
        member_rows.push((row_index, member_indices));
    }
    member_rows.reverse();

    for (offset, row) in rows[spine_row_index + 1..].iter().enumerate() {
        let row_index = spine_row_index + 1 + offset;
        let Some(member_indices) =
            open_track_row_members(row_index, row, expected_count, rectangle_members)
        else {
            break;
        };
        member_rows.push((row_index, member_indices));
    }

    member_rows
}

pub(in crate::layout::ascii_diagram) fn open_vertical_tracks(
    rows: &[AsciiDiagramRow],
    rectangle_members: &BTreeSet<BorderMember>,
) -> Vec<OpenVerticalTrack> {
    let mut tracks = Vec::new();

    for (spine_row_index, row) in rows.iter().enumerate() {
        let spine_anchor_indices = open_timeline_anchor_indices(row);
        if spine_anchor_indices.len() < 2 {
            continue;
        }
        let spine_belongs_to_rectangle = spine_anchor_indices.iter().any(|cell_index| {
            rectangle_members
                .contains(&BorderMember { row_index: spine_row_index, cell_index: *cell_index })
        });
        if spine_belongs_to_rectangle {
            continue;
        }
        let member_rows = contiguous_open_track_rows(
            rows,
            spine_row_index,
            spine_anchor_indices,
            rectangle_members,
        );
        if member_rows.len() < 2 {
            continue;
        }

        for track_position in 0..member_rows[0].1.len() {
            let members = member_rows
                .iter()
                .map(|(row_index, cell_indices)| BorderMember {
                    row_index: *row_index,
                    cell_index: cell_indices[track_position],
                })
                .collect::<Vec<_>>();
            tracks.push(OpenVerticalTrack { members });
        }
    }

    tracks
}

pub(in crate::layout::ascii_diagram) fn align_vertical_tracks(rows: &mut [AsciiDiagramRow]) {
    let rectangles = rectangle_candidates(rows);
    let rectangle_tracks = vertical_border_tracks(rows, &rectangles);
    let rectangle_members = rectangle_tracks
        .iter()
        .flat_map(|track| track.members.iter().copied())
        .collect::<BTreeSet<_>>();
    let open_tracks = open_vertical_tracks(rows, &rectangle_members);
    let mut track_members = rectangle_tracks
        .into_iter()
        .map(|track| track.members)
        .chain(open_tracks.into_iter().map(|track| track.members))
        .collect::<Vec<_>>();
    track_members.sort_by_key(|members| {
        members
            .iter()
            .map(|member| rows[member.row_index].cells[member.cell_index].render_column())
            .min()
            .unwrap_or(usize::MAX)
    });

    for members in track_members {
        let target_column = members
            .iter()
            .map(|member| rows[member.row_index].cells[member.cell_index].render_column())
            .max()
            .expect("every vertical alignment track contains members");
        for member in members {
            align_row_suffix_to(&mut rows[member.row_index], member.cell_index, target_column);
        }
    }
}

pub(in crate::layout::ascii_diagram) fn align_row_suffix_to(
    row: &mut AsciiDiagramRow,
    cell_index: usize,
    target_column: usize,
) {
    let current_column = row.cells[cell_index].render_column();
    let shift = target_column.saturating_sub(current_column);
    if shift == 0 {
        return;
    }

    if row.cells[cell_index].box_connections.is_some_and(|connections| connections.left) {
        row.cells[cell_index].left_extension_columns += shift;
    }
    for cell in &mut row.cells[cell_index..] {
        cell.shift_render_column_by(shift);
    }
}
