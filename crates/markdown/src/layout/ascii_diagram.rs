use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

use super::types::LaidOutLine;

const MINIMUM_BOX_DRAWING_CHARACTERS: usize = 6;
const MINIMUM_BOX_DRAWING_LINES: usize = 2;
static STANDALONE_COMBINING_MARK_GRAPHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{M}+$").expect("Unicode mark pattern is valid"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BoxConnections {
    pub(crate) left: bool,
    pub(crate) right: bool,
    pub(crate) up: bool,
    pub(crate) down: bool,
}

impl BoxConnections {
    pub(crate) const LEFT_RIGHT: Self = Self { left: true, right: true, up: false, down: false };
    pub(crate) const UP_DOWN: Self = Self { left: false, right: false, up: true, down: true };
    pub(crate) const RIGHT_DOWN: Self = Self { left: false, right: true, up: false, down: true };
    pub(crate) const ALL: Self = Self { left: true, right: true, up: true, down: true };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsciiDiagramCell {
    pub(crate) text: String,
    pub(crate) column: usize,
    pub(crate) column_width: usize,
    pub(crate) box_connections: Option<BoxConnections>,
    render_column_shift: usize,
    left_extension_columns: usize,
}

impl AsciiDiagramCell {
    pub(crate) fn render_column(&self) -> usize {
        self.column + self.render_column_shift
    }

    pub(crate) fn left_extension_columns(&self) -> usize {
        self.left_extension_columns
    }

    fn shift_render_column_by(&mut self, column_count: usize) {
        self.render_column_shift += column_count;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsciiDiagramRow {
    pub(crate) cells: Vec<AsciiDiagramCell>,
    /// Source-grid width before cumulative virtual-column insertions.
    pub(crate) column_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsciiDiagram {
    pub(crate) rows: Vec<AsciiDiagramRow>,
    /// Maximum source-grid width before cumulative virtual-column insertions.
    pub(crate) column_count: usize,
}

#[derive(Clone, Debug)]
struct AsciiDiagramEntry {
    source_range: Range<usize>,
    content_anchor: usize,
    diagram: AsciiDiagram,
}

/// Crate-private render metadata keyed by the source anchor of a code block's first line.
///
/// This keeps [`super::types::LaidOutBlock`] source-compatible for external struct literals.
#[derive(Clone, Debug, Default)]
pub(crate) struct AsciiDiagramRegistry {
    entries: BTreeMap<usize, AsciiDiagramEntry>,
    content_anchors: BTreeMap<usize, usize>,
    selection_range: Option<Range<usize>>,
}

impl AsciiDiagramRegistry {
    pub(crate) fn set_selection_range(&mut self, selection_range: Option<Range<usize>>) {
        self.selection_range = selection_range.filter(|range| range.start < range.end);
    }

    pub(crate) fn register(
        &mut self,
        source_range: Range<usize>,
        lines: &[LaidOutLine],
        diagram: AsciiDiagram,
    ) {
        let Some(content_anchor) = diagram_content_anchor(lines) else {
            return;
        };
        self.remove_source_range(&source_range);
        let canonical_anchor = source_range.start;
        self.content_anchors.insert(content_anchor, canonical_anchor);
        self.entries
            .insert(canonical_anchor, AsciiDiagramEntry { source_range, content_anchor, diagram });
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for entry in other.entries.into_values() {
            self.remove_source_range(&entry.source_range);
            let canonical_anchor = entry.source_range.start;
            self.content_anchors.insert(entry.content_anchor, canonical_anchor);
            self.entries.insert(canonical_anchor, entry);
        }
    }

    pub(crate) fn remove_source_range(&mut self, source_range: &Range<usize>) {
        let canonical_anchors: Vec<usize> = self
            .entries
            .iter()
            .filter_map(|(&canonical_anchor, entry)| {
                ranges_intersect(&entry.source_range, source_range).then_some(canonical_anchor)
            })
            .collect();

        for canonical_anchor in canonical_anchors {
            if let Some(entry) = self.entries.remove(&canonical_anchor) {
                self.content_anchors.remove(&entry.content_anchor);
            }
        }
    }

    pub(crate) fn diagram_for(&self, lines: &[LaidOutLine]) -> Option<&AsciiDiagram> {
        let canonical_anchor = self.content_anchors.get(&diagram_content_anchor(lines)?)?;
        let entry = self.entries.get(canonical_anchor)?;
        if selection_intersects(&entry.source_range, self.selection_range.as_ref()) {
            return None;
        }
        Some(&entry.diagram)
    }

    #[cfg(test)]
    pub(crate) fn remove_last_row_for(&mut self, lines: &[LaidOutLine]) {
        if let Some(entry) = diagram_content_anchor(lines)
            .and_then(|anchor| self.content_anchors.get(&anchor))
            .and_then(|canonical_anchor| self.entries.get_mut(canonical_anchor))
        {
            entry.diagram.rows.pop();
        }
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

fn diagram_content_anchor(lines: &[LaidOutLine]) -> Option<usize> {
    lines.first()?.source_projection.as_ref().map(|projection| projection.source_extent.start)
}

fn ranges_intersect(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn selection_intersects(
    source_range: &Range<usize>,
    selection_range: Option<&Range<usize>>,
) -> bool {
    selection_range.is_some_and(|selection_range| ranges_intersect(source_range, selection_range))
}

pub(crate) fn box_connections(text: &str) -> Option<BoxConnections> {
    match text {
        "─" => Some(BoxConnections::LEFT_RIGHT),
        "│" => Some(BoxConnections::UP_DOWN),
        "┌" => Some(BoxConnections::RIGHT_DOWN),
        "┐" => Some(BoxConnections { left: true, down: true, ..BoxConnections::default() }),
        "└" => Some(BoxConnections { right: true, up: true, ..BoxConnections::default() }),
        "┘" => Some(BoxConnections { left: true, up: true, ..BoxConnections::default() }),
        "├" => Some(BoxConnections { right: true, up: true, down: true, left: false }),
        "┤" => Some(BoxConnections { left: true, up: true, down: true, right: false }),
        "┬" => Some(BoxConnections { left: true, right: true, down: true, up: false }),
        "┴" => Some(BoxConnections { left: true, right: true, up: true, down: false }),
        "┼" => Some(BoxConnections::ALL),
        _ => None,
    }
}

pub(crate) fn detect_ascii_diagram(lines: &[String]) -> Option<AsciiDiagram> {
    if lines.iter().filter(|line| !line.is_empty()).count() < 2 {
        return None;
    }

    let mut box_character_count = 0usize;
    let mut box_line_count = 0usize;
    let mut has_corner = false;
    let mut rows = Vec::with_capacity(lines.len());

    for line in lines {
        let (row, row_box_count, row_has_corner) = grid_row(line);
        box_character_count += row_box_count;
        box_line_count += usize::from(row_box_count > 0);
        has_corner |= row_has_corner;
        rows.push(row);
    }

    if !(has_corner || has_open_timeline_structure(&rows))
        || box_character_count < MINIMUM_BOX_DRAWING_CHARACTERS
        || box_line_count < MINIMUM_BOX_DRAWING_LINES
    {
        return None;
    }

    align_vertical_tracks(&mut rows);
    let column_count = rows.iter().map(|row| row.column_count).max().unwrap_or(0);
    Some(AsciiDiagram { rows, column_count })
}

fn grid_row(line: &str) -> (AsciiDiagramRow, usize, bool) {
    let mut cells = Vec::new();
    let mut column = 0usize;
    let mut box_count = 0usize;
    let mut has_corner = false;

    for grapheme in UnicodeSegmentation::graphemes(line, true) {
        let connections = box_connections(grapheme);
        if connections.is_some() {
            box_count += 1;
            has_corner |= matches!(grapheme, "┌" | "┐" | "└" | "┘");
        }
        let column_width = grapheme_column_width(grapheme);
        cells.push(AsciiDiagramCell {
            text: grapheme.to_owned(),
            column,
            column_width,
            box_connections: connections,
            render_column_shift: 0,
            left_extension_columns: 0,
        });
        column += column_width;
    }

    (AsciiDiagramRow { cells, column_count: column }, box_count, has_corner)
}

fn open_timeline_anchor_indices(row: &AsciiDiagramRow) -> Vec<usize> {
    let anchor_indices = row
        .cells
        .iter()
        .enumerate()
        .filter_map(|(cell_index, cell)| {
            matches!(cell.text.as_str(), "├" | "┼" | "┤").then_some(cell_index)
        })
        .collect::<Vec<_>>();

    if anchor_indices.len() < 2
        || row.cells[anchor_indices[0]].text != "├"
        || row.cells[*anchor_indices.last().expect("anchor list contains at least two cells")].text
            != "┤"
        || anchor_indices[1..anchor_indices.len() - 1]
            .iter()
            .any(|cell_index| row.cells[*cell_index].text != "┼")
        || !anchor_indices.windows(2).all(|pair| {
            let left_index = pair[0];
            let right_index = pair[1];
            row.cells[left_index + 1..right_index]
                .iter()
                .all(|cell| cell.text == "─" && cell.column_width == 1)
        })
    {
        return Vec::new();
    }

    anchor_indices
}

fn open_timeline_member_indices(row: &AsciiDiagramRow) -> Vec<usize> {
    row.cells
        .iter()
        .enumerate()
        .filter_map(|(cell_index, cell)| {
            let is_vertical_segment = cell.box_connections.is_some_and(|connections| {
                connections.up && connections.down && !connections.left && !connections.right
            });
            (is_vertical_segment || cell.text == "▼").then_some(cell_index)
        })
        .collect()
}

fn has_open_timeline_structure(rows: &[AsciiDiagramRow]) -> bool {
    rows.iter().enumerate().any(|(row_index, row)| {
        let anchor_count = open_timeline_anchor_indices(row).len();
        if anchor_count < 2 {
            return false;
        }

        row_index
            .checked_sub(1)
            .and_then(|previous_index| rows.get(previous_index))
            .is_some_and(|previous| open_timeline_member_indices(previous).len() == anchor_count)
            || rows
                .get(row_index + 1)
                .is_some_and(|next| open_timeline_member_indices(next).len() == anchor_count)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RectangleEdge {
    row_index: usize,
    left_cell_index: usize,
    right_cell_index: usize,
    left_column: usize,
    right_column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RectangleCandidate {
    top: RectangleEdge,
    bottom: RectangleEdge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BorderSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BorderMember {
    row_index: usize,
    cell_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VerticalBorderTrack {
    rectangle_index: usize,
    side: BorderSide,
    members: Vec<BorderMember>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenVerticalTrack {
    members: Vec<BorderMember>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowTrackAssignment {
    track_index: usize,
    cell_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CandidatePosition {
    expected_position: usize,
    cell_position: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AdjacentTrackPair {
    left_track_index: usize,
    right_track_index: usize,
}

type SupportedAdjacentGaps = BTreeMap<AdjacentTrackPair, BTreeSet<usize>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AssignmentEvidence {
    directly_supported: bool,
    atomic_partners: BTreeSet<CandidatePosition>,
}

fn corner_spans(
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

fn horizontal_overlap(left: RectangleEdge, right: RectangleEdge) -> usize {
    left.right_column
        .min(right.right_column)
        .saturating_sub(left.left_column.max(right.left_column))
}

fn rectangle_candidates(rows: &[AsciiDiagramRow]) -> Vec<RectangleCandidate> {
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

fn is_vertical_edge(cell: &AsciiDiagramCell) -> bool {
    cell.box_connections.is_some_and(|connections| connections.up && connections.down)
}

fn active_rectangle_indices(row_index: usize, candidates: &[RectangleCandidate]) -> Vec<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, rectangle)| {
            rectangle.top.row_index < row_index && row_index < rectangle.bottom.row_index
        })
        .map(|(rectangle_index, _)| rectangle_index)
        .collect()
}

fn border_track_index(rectangle_index: usize, side: BorderSide) -> usize {
    rectangle_index * 2
        + match side {
            BorderSide::Left => 0,
            BorderSide::Right => 1,
        }
}

fn track_source_column(track: &VerticalBorderTrack, candidates: &[RectangleCandidate]) -> usize {
    let rectangle = candidates[track.rectangle_index];
    match track.side {
        BorderSide::Left => rectangle.top.left_column.min(rectangle.bottom.left_column),
        BorderSide::Right => rectangle.top.right_column.max(rectangle.bottom.right_column),
    }
}

fn expected_track_indices(
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

fn vertical_edge_cell_indices(row: &AsciiDiagramRow) -> Vec<usize> {
    row.cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| is_vertical_edge(cell))
        .map(|(cell_index, _)| cell_index)
        .collect()
}

fn record_confident_row(
    row_index: usize,
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    tracks: &mut [VerticalBorderTrack],
    supported_columns: &mut [BTreeSet<usize>],
    supported_spans: &mut [BTreeSet<usize>],
    supported_adjacent_gaps: &mut SupportedAdjacentGaps,
) {
    for (&track_index, &cell_index) in expected_track_indices.iter().zip(cell_indices) {
        supported_columns[track_index].insert(row.cells[cell_index].column);
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
        supported_spans[rectangle_index].insert(right_column.saturating_sub(left_column));
    }

    record_adjacent_track_gaps(row, expected_track_indices, cell_indices, supported_adjacent_gaps);
}

fn record_adjacent_track_gaps(
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

fn directly_supported_candidates(
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

fn expected_rectangle_side_positions(
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

fn record_atomic_candidate_pair(
    candidates: &mut BTreeMap<CandidatePosition, AssignmentEvidence>,
    left: CandidatePosition,
    right: CandidatePosition,
) {
    candidates.entry(left).or_default().atomic_partners.insert(right);
    candidates.entry(right).or_default().atomic_partners.insert(left);
}

fn add_span_supported_candidates(
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

fn should_exclude_tree_branch_conflicting_span_candidate(
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

fn unique_direct_compatible_adjacent_gap_candidate_pairs(
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

fn unique_direct_compatible_adjacent_gap_candidate(
    row: &AsciiDiagramRow,
    cell_indices: &[usize],
    left_expected_position: usize,
    supported_gaps: &BTreeSet<usize>,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> Option<(CandidatePosition, CandidatePosition)> {
    let mut unique_candidate = None;
    for left_cell_position in 0..cell_indices.len() {
        let left_column = row.cells[cell_indices[left_cell_position]].column;
        for right_cell_position in left_cell_position + 1..cell_indices.len() {
            let right_column = row.cells[cell_indices[right_cell_position]].column;
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

fn add_adjacent_gap_supported_candidates(
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

        for left_cell_position in 0..cell_indices.len() {
            let left_column = row.cells[cell_indices[left_cell_position]].column;
            for right_cell_position in left_cell_position + 1..cell_indices.len() {
                let right_column = row.cells[cell_indices[right_cell_position]].column;
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

fn has_tree_branch_junction(row: &AsciiDiagramRow) -> bool {
    row.cells.iter().any(|cell| matches!(cell.text.as_str(), "├" | "┤"))
}

fn candidate_position_conflicts_with_direct_support(
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

fn mutually_unique_candidate_positions(
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

fn candidate_positions_cross(left: CandidatePosition, right: CandidatePosition) -> bool {
    (left.expected_position < right.expected_position && left.cell_position > right.cell_position)
        || (right.expected_position < left.expected_position
            && right.cell_position > left.cell_position)
}

fn positions_without_crossing_candidates(
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

fn supported_row_candidates(
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

fn has_complete_forced_atomic_partner(
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

fn directly_supported_boundary_positions(
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

fn has_direct_axis_competitor(
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

fn unique_reciprocal_atomic_partner(
    position: CandidatePosition,
    candidates: &BTreeMap<CandidatePosition, AssignmentEvidence>,
) -> Option<CandidatePosition> {
    let mut partners = candidates[&position].atomic_partners.iter().copied().filter(|partner| {
        candidates.get(partner).is_some_and(|evidence| evidence.atomic_partners.contains(&position))
    });
    let partner = partners.next()?;
    partners.next().is_none().then_some(partner)
}

fn position_is_compatible(
    position: CandidatePosition,
    selected: &BTreeSet<CandidatePosition>,
) -> bool {
    selected.iter().all(|other| positions_are_compatible(position, *other))
}

fn positions_are_compatible(left: CandidatePosition, right: CandidatePosition) -> bool {
    left == right
        || (left.expected_position != right.expected_position
            && left.cell_position != right.cell_position
            && !candidate_positions_cross(left, right))
}

fn position_crosses_candidate_compatible_with_boundaries(
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

fn include_direct_boundary_anchors(
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
fn uniquely_supported_row_assignments(
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

fn record_uniquely_supported_row(
    row_index: usize,
    row: &AsciiDiagramRow,
    expected_track_indices: &[usize],
    cell_indices: &[usize],
    tracks: &mut [VerticalBorderTrack],
    supported_columns: &[BTreeSet<usize>],
    supported_spans: &[BTreeSet<usize>],
    supported_adjacent_gaps: &SupportedAdjacentGaps,
) {
    for assignment in uniquely_supported_row_assignments(
        row,
        expected_track_indices,
        cell_indices,
        tracks,
        supported_columns,
        supported_spans,
        supported_adjacent_gaps,
    ) {
        tracks[assignment.track_index]
            .members
            .push(BorderMember { row_index, cell_index: assignment.cell_index });
    }
}

fn new_vertical_border_tracks(candidates: &[RectangleCandidate]) -> Vec<VerticalBorderTrack> {
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

fn corner_support(
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

fn vertical_border_tracks(
    rows: &[AsciiDiagramRow],
    candidates: &[RectangleCandidate],
) -> Vec<VerticalBorderTrack> {
    let mut tracks = new_vertical_border_tracks(candidates);
    let (mut supported_columns, mut supported_spans) = corner_support(candidates, &tracks);
    let mut supported_adjacent_gaps = SupportedAdjacentGaps::new();
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
                &mut supported_columns,
                &mut supported_spans,
                &mut supported_adjacent_gaps,
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
            &supported_columns,
            &supported_spans,
            &supported_adjacent_gaps,
        );
    }
    for track in &mut tracks {
        track.members.sort_by_key(|member| (member.row_index, member.cell_index));
        track.members.dedup();
    }
    tracks
}

fn open_track_row_members(
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

fn contiguous_open_track_rows(
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

    for row_index in spine_row_index + 1..rows.len() {
        let Some(member_indices) =
            open_track_row_members(row_index, &rows[row_index], expected_count, rectangle_members)
        else {
            break;
        };
        member_rows.push((row_index, member_indices));
    }

    member_rows
}

fn open_vertical_tracks(
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

fn align_vertical_tracks(rows: &mut [AsciiDiagramRow]) {
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

fn align_row_suffix_to(row: &mut AsciiDiagramRow, cell_index: usize, target_column: usize) {
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

fn grapheme_column_width(grapheme: &str) -> usize {
    if STANDALONE_COMBINING_MARK_GRAPHEME.is_match(grapheme) {
        return 0;
    }

    grapheme.chars().next().and_then(UnicodeWidthChar::width).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN_TIMELINE: &[&str] = &[
        "Day -29          Day -7           Day -1    Today    Day +1",
        "  │                │                │         │         │",
        "  ├────────────────┼────────────────┼─────────┼─────────┤",
        "  │                 │                │         │         │",
        "  │  30天历史行为   │  近期模式      │ 昨日    │ 今日    │ 次日展示",
        "  │  (服务端存储)   │  (7天细粒度)   │ 增量    │ WPS     │ 焦点项",
        "  │                 │                │ 上传    │ 首页    │",
        "  │                 │                │         │         │",
        "  ▼                 ▼                ▼         ▼         ▼",
    ];

    const ELECTRON_GO_ARCHITECTURE: &[&str] = &[
        "Electron (UI Shell)          Go (Agent Core)",
        "┌──────────────────────┐    ┌─────────────────────────────┐",
        "│  Main Process         │    │  WebSocket Server            │",
        "│  ├─ spawn Go 二进制    │◄──►│  ├─ token 认证               │",
        "│  └─ BrowserWindow     │ WS │  └─ 收发 JSON 消息           │",
        "│                       │    │                              │",
        "│  Renderer (Chat UI)   │    │  Agent Loop                  │",
        "│  ├─ 流式对话           │    │  ├─ Orchestrator (主 agent)  │",
        "│  ├─ 工具调用卡片        │    │  └─ Worker (子 agent)       │",
        "│  ├─ 子 agent 进度      │    │                              │",
        "│  └─ 任务面板            │    │  LLM Provider               │",
        "└──────────────────────┘    │  ├─ Anthropic (流式 SSE)      │",
        "                            │  └─ OpenAI 兼容               │",
        "                            │                               │",
        "                            │  工具系统 (8 tools)            │",
        "                            │  条件式 Prompt 构建            │",
        "                            │  Skills 系统                  │",
        "                            │  会话持久化                    │",
        "                            └─────────────────────────────┘",
    ];

    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn detects_open_timeline_without_rectangle_corners() {
        assert!(
            detect_ascii_diagram(&lines(OPEN_TIMELINE)).is_some(),
            "a strongly connected open timeline must use the fixed-grid renderer"
        );
    }

    #[test]
    fn rejects_open_timeline_with_wrong_junction_topology() {
        assert!(detect_ascii_diagram(&lines(&["──┼──┼──", "  │  │"])).is_none());
        assert!(detect_ascii_diagram(&lines(&["├─┬─┤", "│   │"])).is_none());
    }

    #[test]
    fn aligns_open_timeline_tracks_with_cjk_content() {
        const EXPECTED_COLUMNS: [usize; 5] = [2, 20, 37, 47, 57];

        let diagram = detect_ascii_diagram(&lines(OPEN_TIMELINE))
            .expect("a strongly connected open timeline must be detected");

        for row_index in 1..diagram.rows.len() {
            let row = &diagram.rows[row_index];
            let mut member_indices = open_timeline_anchor_indices(row);
            if member_indices.is_empty() {
                member_indices = open_timeline_member_indices(row);
            }
            let render_columns = member_indices
                .iter()
                .map(|cell_index| row.cells[*cell_index].render_column())
                .collect::<Vec<_>>();

            assert_eq!(
                render_columns, EXPECTED_COLUMNS,
                "timeline row {row_index} must use the shared vertical tracks"
            );
        }
    }

    #[test]
    fn leaves_mismatched_open_timeline_row_at_source_columns() {
        let diagram =
            detect_ascii_diagram(&lines(&["  │     │    │", "  ├────┼────┤", "  │    │"]))
                .expect("the first two rows provide a strongly connected open timeline");
        let mismatched_row = &diagram.rows[2];
        let render_columns = open_timeline_member_indices(mismatched_row)
            .iter()
            .map(|cell_index| mismatched_row.cells[*cell_index].render_column())
            .collect::<Vec<_>>();

        assert_eq!(
            render_columns,
            [2, 7],
            "a row with a different member count must not be guessed into the open tracks"
        );
    }

    #[test]
    fn neighboring_spine_terminates_open_track_scan() {
        let diagram = detect_ascii_diagram(&lines(&["│     │    │", "├────┼────┤", "├──┤  │ │ │"]))
            .expect("the first two rows provide a strongly connected open timeline");
        let neighboring_spine_row = &diagram.rows[2];
        let render_columns = open_timeline_member_indices(neighboring_spine_row)
            .iter()
            .map(|cell_index| neighboring_spine_row.cells[*cell_index].render_column())
            .collect::<Vec<_>>();

        assert_eq!(
            render_columns,
            [6, 8, 10],
            "a neighboring spine must terminate the current open track group"
        );
    }

    #[test]
    fn closed_table_divider_does_not_create_open_tracks() {
        let diagram =
            detect_ascii_diagram(&lines(&["┌─────┐", "│  │  │", "├──┼──┤", "│   │ │", "└─────┘"]))
                .expect("a closed table must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[0], 6).render_column(), 6);
        assert_eq!(cell_at_source_column(&diagram.rows[4], 6).render_column(), 6);
        assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 3);
        assert_eq!(cell_at_source_column(&diagram.rows[3], 4).render_column(), 4);
    }

    #[test]
    fn independent_open_timeline_inside_rectangle_still_aligns() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌─────────────┐",
            "│  │      │   │",
            "│  ├──────┤   │",
            "│  │       │  │",
            "└─────────────┘",
        ]))
        .expect("an outer rectangle containing an independent open timeline must be detected");

        for (row_index, source_column) in [(1, 10), (2, 10), (3, 11)] {
            assert_eq!(
                cell_at_source_column(&diagram.rows[row_index], source_column).render_column(),
                11,
                "inner timeline right track must align on row {row_index}"
            );
        }
        for row_index in 0..diagram.rows.len() {
            assert_eq!(
                right_edge_cell(&diagram.rows[row_index]).render_column(),
                15,
                "outer rectangle must remain aligned after inner track correction on row {row_index}"
            );
        }
    }

    fn cell_at_source_column(row: &AsciiDiagramRow, column: usize) -> &AsciiDiagramCell {
        row.cells
            .iter()
            .find(|cell| cell.column == column)
            .expect("fixture must contain a cell at the requested source column")
    }

    fn right_edge_cell(row: &AsciiDiagramRow) -> &AsciiDiagramCell {
        row.cells
            .iter()
            .rev()
            .find(|cell| matches!(cell.text.as_str(), "│" | "┐" | "┘"))
            .expect("fixture row must contain a right edge")
    }

    #[test]
    fn aligns_electron_shell_outer_right_edge_across_tree_branch_rows() {
        const LEFT_FRAME_RIGHT_SOURCES: [(usize, usize); 11] = [
            (1, 23),
            (2, 24),
            (3, 25),
            (4, 24),
            (5, 24),
            (6, 24),
            (7, 25),
            (8, 26),
            (9, 25),
            (10, 26),
            (11, 23),
        ];

        let diagram = detect_ascii_diagram(&lines(ELECTRON_GO_ARCHITECTURE))
            .expect("the architecture fixture must be detected");
        let render_columns = LEFT_FRAME_RIGHT_SOURCES.map(|(row_index, source_column)| {
            cell_at_source_column(&diagram.rows[row_index], source_column).render_column()
        });

        assert_eq!(render_columns, [26; LEFT_FRAME_RIGHT_SOURCES.len()]);
        for row_index in [3, 7, 8, 9] {
            assert_eq!(
                cell_at_source_column(&diagram.rows[row_index], 3).render_column(),
                3,
                "tree branch on row {row_index} must stay at its source column"
            );
        }
    }

    fn candidate_position_for_assignment(
        assignment: RowTrackAssignment,
        expected_track_indices: &[usize],
        cell_indices: &[usize],
    ) -> CandidatePosition {
        CandidatePosition {
            expected_position: expected_track_indices
                .iter()
                .position(|track_index| *track_index == assignment.track_index)
                .expect("assignment track must occur in the expected topology"),
            cell_position: cell_indices
                .iter()
                .position(|cell_index| *cell_index == assignment.cell_index)
                .expect("assignment cell must occur in the actual edge sequence"),
        }
    }

    #[test]
    fn aligning_one_edge_shifts_every_following_cell() {
        let (mut row, _, _) = grid_row("┌─┐  ┌─┐");
        let first_right_index = row
            .cells
            .iter()
            .position(|cell| cell.column == 2)
            .expect("fixture contains the first right corner");

        align_row_suffix_to(&mut row, first_right_index, 4);

        assert_eq!(cell_at_source_column(&row, 2).render_column(), 4);
        assert_eq!(cell_at_source_column(&row, 5).render_column(), 7);
        assert_eq!(cell_at_source_column(&row, 7).render_column(), 9);
    }

    #[test]
    fn inherited_shift_does_not_extend_later_horizontal_edges() {
        let (mut row, _, _) = grid_row("┌─┐  ┌─┐");
        let first_right_index = row
            .cells
            .iter()
            .position(|cell| cell.column == 2)
            .expect("fixture contains the first right corner");

        align_row_suffix_to(&mut row, first_right_index, 4);

        assert_eq!(cell_at_source_column(&row, 2).left_extension_columns(), 2);
        assert_eq!(cell_at_source_column(&row, 5).left_extension_columns(), 0);
        assert_eq!(cell_at_source_column(&row, 7).left_extension_columns(), 0);
    }

    #[test]
    fn later_track_uses_columns_shifted_by_the_previous_track() {
        let (first_row, _, _) = grid_row("│  │  │  │");
        let (mut second_row, _, _) = grid_row("│ │  │ │");

        align_row_suffix_to(&mut second_row, 2, 3);
        assert_eq!(cell_at_source_column(&second_row, 5).render_column(), 6);

        align_row_suffix_to(&mut second_row, 5, 7);
        assert_eq!(cell_at_source_column(&second_row, 7).render_column(), 9);
        assert_eq!(cell_at_source_column(&first_row, 9).render_column(), 9);
    }

    #[test]
    fn pairs_rectangle_corners_when_both_sides_drift_between_top_and_bottom() {
        let rows = lines(&[
            "                                                     ┌───────────────────────┐",
            "                                                    │                        │",
            "                                                   └─────────────────────────┘",
        ])
        .iter()
        .map(|line| grid_row(line).0)
        .collect::<Vec<_>>();

        let candidates = rectangle_candidates(&rows);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].top.left_column, 53);
        assert_eq!(candidates[0].top.right_column, 77);
        assert_eq!(candidates[0].bottom.left_column, 51);
        assert_eq!(candidates[0].bottom.right_column, 77);
    }

    #[test]
    fn closest_open_layer_closes_before_its_outer_rectangle() {
        let rows = lines(&["┌─────────┐", "  ┌─────┐", "  └─────┘", "└─────────┘"])
            .iter()
            .map(|line| grid_row(line).0)
            .collect::<Vec<_>>();

        let candidates = rectangle_candidates(&rows);

        assert_eq!(candidates.len(), 2);
        assert_eq!((candidates[0].top.row_index, candidates[0].bottom.row_index), (0, 3));
        assert_eq!((candidates[1].top.row_index, candidates[1].bottom.row_index), (1, 2));
    }

    #[test]
    fn equal_overlap_on_the_closest_open_row_is_ambiguous() {
        let rows = lines(&["┌───┐ ┌───┐", "  └─────┘"])
            .iter()
            .map(|line| grid_row(line).0)
            .collect::<Vec<_>>();

        assert!(rectangle_candidates(&rows).is_empty());
    }

    #[test]
    fn arrow_after_a_shifted_boundary_inherits_the_prefix_delta() {
        let diagram =
            detect_ascii_diagram(&lines(&["┌────┐  ┌──┐", "│  │ →  │  │", "└────┘  └──┘"]))
                .expect("fixture must be detected");

        let shifted_boundary = cell_at_source_column(&diagram.rows[1], 3);
        let arrow = cell_at_source_column(&diagram.rows[1], 5);
        let following_boundary = cell_at_source_column(&diagram.rows[1], 8);

        assert_eq!(shifted_boundary.render_column(), 5);
        assert_eq!(arrow.render_column(), 7);
        assert_eq!(following_boundary.render_column(), 10);
        assert!(shifted_boundary.render_column() < arrow.render_column());
        assert!(arrow.render_column() < following_boundary.render_column());
    }

    #[test]
    fn many_parallel_nested_tracks_keep_only_uniquely_supported_low_confidence_edges() {
        const TRAILING_SPAN_LEFT_COLUMN: usize = 100;
        const UNIQUE_TRAILING_SPAN: usize = 41;
        const TRAILING_SPAN_RIGHT_COLUMN: usize = TRAILING_SPAN_LEFT_COLUMN + UNIQUE_TRAILING_SPAN;
        let actual_columns = [
            0,
            2,
            8,
            10,
            14,
            16,
            22,
            28,
            31,
            37,
            38,
            TRAILING_SPAN_LEFT_COLUMN,
            TRAILING_SPAN_RIGHT_COLUMN,
        ];
        let mut fixture_characters = vec![' '; TRAILING_SPAN_RIGHT_COLUMN + 1];
        for column in actual_columns {
            fixture_characters[column] = '│';
        }
        let fixture = fixture_characters.into_iter().collect::<String>();
        let (row, _, _) = grid_row(&fixture);
        let cell_indices = vertical_edge_cell_indices(&row);
        let tracks = (0..7)
            .flat_map(|rectangle_index| {
                [BorderSide::Left, BorderSide::Right].map(|side| VerticalBorderTrack {
                    rectangle_index,
                    side,
                    members: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        let expected_track_indices = [0, 2, 3, 1, 4, 6, 7, 5, 8, 10, 11, 9, 12, 13];
        let supported_track_columns = [
            (0, 0),
            (2, 2),
            (3, 8),
            (1, 10),
            (4, 14),
            (6, 16),
            (7, 22),
            (5, 24),
            (8, 28),
            (10, 30),
            (11, 36),
            (9, 38),
        ];
        let mut supported_columns = vec![BTreeSet::new(); tracks.len()];
        for (track_index, column) in supported_track_columns {
            supported_columns[track_index].insert(column);
        }
        let mut supported_spans = vec![BTreeSet::new(); 7];
        supported_spans[5].insert(6);
        supported_spans[6].insert(UNIQUE_TRAILING_SPAN);

        let assignments = uniquely_supported_row_assignments(
            &row,
            &expected_track_indices,
            &cell_indices,
            &tracks,
            &supported_columns,
            &supported_spans,
            &SupportedAdjacentGaps::new(),
        );
        let candidates = supported_row_candidates(
            &row,
            &expected_track_indices,
            &cell_indices,
            &tracks,
            &supported_columns,
            &supported_spans,
            &SupportedAdjacentGaps::new(),
        );
        let assignment_positions = assignments
            .iter()
            .copied()
            .map(|assignment| {
                candidate_position_for_assignment(
                    assignment,
                    &expected_track_indices,
                    &cell_indices,
                )
            })
            .collect::<Vec<_>>();
        let assignment_position_set = assignment_positions.iter().copied().collect::<BTreeSet<_>>();

        assert!(candidates.len() <= expected_track_indices.len() * cell_indices.len());
        assert!(assignments.len() <= expected_track_indices.len().min(cell_indices.len()));
        assert_eq!(assignment_position_set.len(), assignments.len());
        assert!(!assignments.is_empty());
        assert!(assignment_positions.windows(2).all(|pair| {
            pair[0].expected_position < pair[1].expected_position
                && pair[0].cell_position < pair[1].cell_position
        }));
        assert!(assignments.iter().all(|assignment| assignment.track_index != 5));

        let mut span_only_assignment_count = 0usize;
        for position in assignment_positions {
            assert_eq!(
                candidates
                    .keys()
                    .filter(|candidate| {
                        candidate.expected_position == position.expected_position
                    })
                    .count(),
                1
            );
            assert_eq!(
                candidates
                    .keys()
                    .filter(|candidate| candidate.cell_position == position.cell_position)
                    .count(),
                1
            );
            assert!(candidates.keys().all(|candidate| {
                *candidate == position || !candidate_positions_cross(position, *candidate)
            }));

            let evidence = &candidates[&position];
            if evidence.directly_supported {
                continue;
            }
            span_only_assignment_count += 1;
            assert!(evidence.atomic_partners.iter().any(|partner| {
                assignment_position_set.contains(partner)
                    && candidates.get(partner).is_some_and(|partner_evidence| {
                        partner_evidence.atomic_partners.contains(&position)
                    })
            }));
        }
        assert_eq!(span_only_assignment_count, 2);
    }

    #[test]
    fn low_confidence_assignment_has_no_recursive_search_path() {
        let source = include_str!("ascii_diagram.rs");
        let forbidden_symbols = [
            ["collect_supported_", "mappings"].concat(),
            ["BestRow", "Assignments"].concat(),
            ["too_many_", "arguments"].concat(),
        ];
        for forbidden_symbol in forbidden_symbols {
            assert!(!source.contains(&forbidden_symbol));
        }

        let function_name = ["uniquely_supported_row_", "assignments"].concat();
        let signature = format!("fn {function_name}(");
        let function_start =
            source.find(&signature).expect("low-confidence assignment function must exist");
        let function_end_marker = "\n}\n\nfn record_uniquely_supported_row";
        let function_end = source[function_start..]
            .find(function_end_marker)
            .map(|relative_end| function_start + relative_end)
            .expect("low-confidence assignment function must have a bounded source body");
        let function_source = &source[function_start..function_end];

        assert_eq!(function_source.matches(&function_name).count(), 1);
    }

    #[test]
    fn unique_candidate_crossing_ambiguous_candidates_remains_unassigned() {
        let (row, _, _) = grid_row("│││");
        let cell_indices = vertical_edge_cell_indices(&row);
        let tracks = (0..2)
            .map(|rectangle_index| VerticalBorderTrack {
                rectangle_index,
                side: BorderSide::Left,
                members: Vec::new(),
            })
            .collect::<Vec<_>>();
        let mut supported_columns = vec![BTreeSet::new(); tracks.len()];
        supported_columns[0].extend([1, 2]);
        supported_columns[1].insert(0);

        let assignments = uniquely_supported_row_assignments(
            &row,
            &[0, 1],
            &cell_indices,
            &tracks,
            &supported_columns,
            &[BTreeSet::new(), BTreeSet::new()],
            &SupportedAdjacentGaps::new(),
        );

        assert!(assignments.is_empty());
    }

    #[test]
    fn adjacent_gap_candidates_are_scoped_to_their_track_pair() {
        let (row, _, _) = grid_row("├    │    │");
        let cell_indices = vertical_edge_cell_indices(&row);
        let expected_track_indices = [0, 1, 2];
        let supported_adjacent_gaps = BTreeMap::from([(
            AdjacentTrackPair { left_track_index: 1, right_track_index: 2 },
            BTreeSet::from([5]),
        )]);
        let mut candidates = BTreeMap::new();

        add_adjacent_gap_supported_candidates(
            &row,
            &expected_track_indices,
            &cell_indices,
            &supported_adjacent_gaps,
            &mut candidates,
        );

        let left = CandidatePosition { expected_position: 1, cell_position: 0 };
        let right = CandidatePosition { expected_position: 2, cell_position: 1 };
        assert!(candidates[&left].atomic_partners.contains(&right));
        assert!(candidates[&right].atomic_partners.contains(&left));
        assert!(candidates.keys().all(|position| position.expected_position != 0));
    }

    #[test]
    fn adjacent_gap_candidates_ignore_rows_without_tree_branch_junction() {
        let (row, _, _) = grid_row("│    │");
        let cell_indices = vertical_edge_cell_indices(&row);
        let supported_adjacent_gaps = BTreeMap::from([(
            AdjacentTrackPair { left_track_index: 0, right_track_index: 1 },
            BTreeSet::from([5]),
        )]);
        let mut candidates = BTreeMap::new();

        add_adjacent_gap_supported_candidates(
            &row,
            &[0, 1],
            &cell_indices,
            &supported_adjacent_gaps,
            &mut candidates,
        );

        assert!(candidates.is_empty());
    }

    #[test]
    fn adjacent_gap_candidates_reject_endpoints_conflicting_with_direct_support() {
        let (row, _, _) = grid_row(ELECTRON_GO_ARCHITECTURE[3]);
        let cell_indices = vertical_edge_cell_indices(&row);
        let expected_track_indices = [0, 1, 2, 3];
        let supported_adjacent_gaps = BTreeMap::from([
            (AdjacentTrackPair { left_track_index: 1, right_track_index: 2 }, BTreeSet::from([5])),
            (
                AdjacentTrackPair { left_track_index: 2, right_track_index: 3 },
                BTreeSet::from([30, 31, 33]),
            ),
        ]);
        let mut candidates = BTreeMap::from([
            (
                CandidatePosition { expected_position: 0, cell_position: 0 },
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
            (
                CandidatePosition { expected_position: 3, cell_position: 5 },
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
        ]);

        add_adjacent_gap_supported_candidates(
            &row,
            &expected_track_indices,
            &cell_indices,
            &supported_adjacent_gaps,
            &mut candidates,
        );

        let target_left = CandidatePosition { expected_position: 1, cell_position: 2 };
        let target_right = CandidatePosition { expected_position: 2, cell_position: 3 };
        let right_boundary = CandidatePosition { expected_position: 3, cell_position: 5 };
        assert!(candidates[&target_left].atomic_partners.contains(&target_right));
        assert!(candidates[&target_right].atomic_partners.contains(&target_left));
        assert!(candidates[&target_right].atomic_partners.contains(&right_boundary));
        assert!(candidates[&right_boundary].atomic_partners.contains(&target_right));
        assert!(
            !candidates.contains_key(&CandidatePosition { expected_position: 2, cell_position: 0 })
        );
        assert!(
            !candidates.contains_key(&CandidatePosition { expected_position: 2, cell_position: 1 })
        );
    }

    #[test]
    fn tree_branch_span_filter_requires_a_tree_branch_junction() {
        let (tree_row, _, _) = grid_row(ELECTRON_GO_ARCHITECTURE[3]);
        let tree_cell_indices = vertical_edge_cell_indices(&tree_row);
        let (plain_row, _, _) = grid_row("│    │    │    │");
        let plain_cell_indices = vertical_edge_cell_indices(&plain_row);
        let expected_track_indices = [0, 1, 2, 3];
        let supported_adjacent_gaps = BTreeMap::from([(
            AdjacentTrackPair { left_track_index: 1, right_track_index: 2 },
            BTreeSet::from([5]),
        )]);
        let tree_candidates = BTreeMap::from([
            (
                CandidatePosition { expected_position: 0, cell_position: 0 },
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
            (
                CandidatePosition { expected_position: 3, cell_position: 5 },
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
        ]);
        let plain_candidates = BTreeMap::from([
            (
                CandidatePosition { expected_position: 0, cell_position: 0 },
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
            (
                CandidatePosition { expected_position: 3, cell_position: 3 },
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
        ]);

        assert!(should_exclude_tree_branch_conflicting_span_candidate(
            &tree_row,
            &expected_track_indices,
            &tree_cell_indices,
            CandidatePosition { expected_position: 2, cell_position: 0 },
            CandidatePosition { expected_position: 3, cell_position: 3 },
            &supported_adjacent_gaps,
            &tree_candidates,
        ));
        assert!(!should_exclude_tree_branch_conflicting_span_candidate(
            &plain_row,
            &expected_track_indices,
            &plain_cell_indices,
            CandidatePosition { expected_position: 2, cell_position: 0 },
            CandidatePosition { expected_position: 3, cell_position: 2 },
            &supported_adjacent_gaps,
            &plain_candidates,
        ));
    }

    #[test]
    fn ambiguous_adjacent_gap_pairs_remain_unassigned() {
        let (row, _, _) = grid_row("├    │    │");
        let cell_indices = vertical_edge_cell_indices(&row);
        let tracks = [BorderSide::Left, BorderSide::Right]
            .map(|side| VerticalBorderTrack { rectangle_index: 0, side, members: Vec::new() })
            .to_vec();
        let supported_adjacent_gaps = BTreeMap::from([(
            AdjacentTrackPair { left_track_index: 0, right_track_index: 1 },
            BTreeSet::from([5]),
        )]);
        let mut candidates = BTreeMap::new();

        add_adjacent_gap_supported_candidates(
            &row,
            &[0, 1],
            &cell_indices,
            &supported_adjacent_gaps,
            &mut candidates,
        );
        assert!(
            !candidates.is_empty(),
            "tree branch row must generate the equal-gap candidates under test"
        );

        let assignments = uniquely_supported_row_assignments(
            &row,
            &[0, 1],
            &cell_indices,
            &tracks,
            &[BTreeSet::new(), BTreeSet::new()],
            &[BTreeSet::new()],
            &supported_adjacent_gaps,
        );

        assert!(assignments.is_empty(), "two equal gap pairs must remain ambiguous");
    }

    #[test]
    fn direct_candidate_conflicting_with_a_complete_span_pair_remains_unassigned() {
        let (row, _, _) = grid_row("│    │    │");
        let cell_indices = vertical_edge_cell_indices(&row);
        let tracks = [BorderSide::Left, BorderSide::Right]
            .map(|side| VerticalBorderTrack { rectangle_index: 0, side, members: Vec::new() })
            .to_vec();
        let mut supported_columns = vec![BTreeSet::new(); tracks.len()];
        supported_columns[0].insert(5);
        let mut supported_spans = vec![BTreeSet::new()];
        supported_spans[0].insert(10);

        let assignments = uniquely_supported_row_assignments(
            &row,
            &[0, 1],
            &cell_indices,
            &tracks,
            &supported_columns,
            &supported_spans,
            &SupportedAdjacentGaps::new(),
        );

        assert!(assignments.is_empty());
    }

    #[test]
    fn direct_last_boundary_anchor_recovers_its_only_reciprocal_atomic_partner() {
        let partner = CandidatePosition { expected_position: 1, cell_position: 1 };
        let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
        let candidates = BTreeMap::from([
            (
                partner,
                AssignmentEvidence {
                    directly_supported: false,
                    atomic_partners: BTreeSet::from([boundary]),
                },
            ),
            (
                boundary,
                AssignmentEvidence {
                    directly_supported: true,
                    atomic_partners: BTreeSet::from([partner]),
                },
            ),
        ]);
        let mut selected = BTreeSet::new();

        include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

        assert_eq!(selected, BTreeSet::from([partner, boundary]));
    }

    #[test]
    fn direct_boundary_anchor_does_not_choose_between_multiple_atomic_partners() {
        let first_partner = CandidatePosition { expected_position: 0, cell_position: 0 };
        let second_partner = CandidatePosition { expected_position: 1, cell_position: 1 };
        let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
        let candidates = BTreeMap::from([
            (
                first_partner,
                AssignmentEvidence {
                    directly_supported: false,
                    atomic_partners: BTreeSet::from([boundary]),
                },
            ),
            (
                second_partner,
                AssignmentEvidence {
                    directly_supported: false,
                    atomic_partners: BTreeSet::from([boundary]),
                },
            ),
            (
                boundary,
                AssignmentEvidence {
                    directly_supported: true,
                    atomic_partners: BTreeSet::from([first_partner, second_partner]),
                },
            ),
        ]);
        let mut selected = BTreeSet::new();

        include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

        assert_eq!(selected, BTreeSet::from([boundary]));
    }

    #[test]
    fn direct_boundary_anchor_rejects_a_direct_candidate_sharing_its_actual_edge() {
        let competing = CandidatePosition { expected_position: 2, cell_position: 2 };
        let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
        let candidates = BTreeMap::from([
            (
                competing,
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
            (
                boundary,
                AssignmentEvidence { directly_supported: true, ..AssignmentEvidence::default() },
            ),
        ]);
        let mut selected = BTreeSet::new();

        include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

        assert!(selected.is_empty());
    }

    #[test]
    fn boundary_atomic_partner_rejects_crossing_candidate_that_remains_viable() {
        let crossing = CandidatePosition { expected_position: 2, cell_position: 0 };
        let partner = CandidatePosition { expected_position: 1, cell_position: 1 };
        let boundary = CandidatePosition { expected_position: 3, cell_position: 2 };
        let candidates = BTreeMap::from([
            (crossing, AssignmentEvidence::default()),
            (
                partner,
                AssignmentEvidence {
                    directly_supported: false,
                    atomic_partners: BTreeSet::from([boundary]),
                },
            ),
            (
                boundary,
                AssignmentEvidence {
                    directly_supported: true,
                    atomic_partners: BTreeSet::from([partner]),
                },
            ),
        ]);
        let mut selected = BTreeSet::new();

        include_direct_boundary_anchors(&candidates, 4, 3, &mut selected);

        assert_eq!(selected, BTreeSet::from([boundary]));
    }

    #[test]
    fn aligns_wps_architecture_outer_edges_with_nested_and_missing_segments() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌──────────── WPS 客户端 ────────────┐          ┌────── 服务端（增量持续）──────┐",
            "│                                     │          │                              │",
            "│  ┌─ 本地日志（30天滚动）─────────┐  │  每日增量 │  ┌─ 画像更新 Pipeline ───┐  │",
            "│  │ · 文件操作（打开/关闭/保存）  │  │  上传     │  │ · 角色标签更新          │  │",
            "│  │ · 编辑会话（时长/段落/光标）  │  │────────→ │  │ · 模板偏好更新          │  │",
            "│  │ · 模板使用记录               │  │  DSL包   │  │ · 活跃项目检测          │  │",
            "│  │ · 搜索/崩溃日志              │  │ (仅增量)  │  │ · 周期性模式识别        │  │",
            "│  │ · 过去30天全部可触达         │  │          │  └─────────────────────────┘  │",
            "│  └──────────────────────────────┘  │          │                              │",
            "│                                     │          │  ┌─ 行为预测 Agent ──────┐  │",
            "│  ┌─ 本地 Tool（核心）────────────┐  │          │  │ · 读取 30 天滚动窗口    │  │",
            "│  │ ① 扫描 30 天日志目录         │  │          │  │ · 加载用户画像          │  │",
            "│  │ ② 数据清洗 + 去噪 + 聚合    │  │          │  │ · LLM 推理：            │  │",
            "│  │ ③ 结构化 DSL 事件            │  │          │  │   短期意图(续编/模板)    │  │",
            "│  │ ④ 上下文字段填充             │  │          │  │   周期性模式(月报/周报)  │  │",
            "│  │ ⑤ 画像缓存附加               │  │          │  │ · 生成焦点项（≤5 条）   │  │",
            "│  │ ⑥ 增量打包上传               │  │          │  │ · 输出结构化焦点 DSL    │  │",
            "│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │",
            "│                                     │          │                              │",
            "│  ┌─ 焦点渲染 ──────────────────┐  │  每日拉取  │  ┌─ 焦点项存储 + 下发 ───┐  │",
            "│  │ · 首页焦点卡片列表           │  │←──────── │  │ · 存储到用户维度        │  │",
            "│  │ · 排序展示（按优先级）       │  │ 焦点DSL   │  │ · 客户端启动时拉取      │  │",
            "│  │ · 点击执行 action            │  │          │  │ · 附带更新后画像        │  │",
            "│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │",
            "│                                     │          │                              │",
            "│  ┌─ 反馈采集 ──────────────────┐  │          │  ┌─ 反馈回收 ─────────────┐  │",
            "│  │ · 卡片点击（accepted）       │──反馈上报→│  │ · 焦点项点击/忽略统计   │  │",
            "│  │ · 卡片关闭（dismissed）      │  │          │  │ · 纳入画像调整信号      │  │",
            "│  │ · 无操作（ignored）          │  │          │  │ · 周期准确性验证        │  │",
            "│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │",
            "└────────────────────────────────────┘          └──────────────────────────────┘",
        ]))
        .expect("fixture must be detected");

        let client_right_columns = [(0, 37), (1, 38), (12, 36), (30, 37)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert!(client_right_columns.windows(2).all(|pair| pair[0] == pair[1]));

        let server_left_columns = [(0, 48), (3, 50), (12, 47), (19, 49)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert!(server_left_columns.windows(2).all(|pair| pair[0] == pair[1]));

        let server_right_columns = [(0, 80), (3, 82), (12, 79), (30, 79)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert!(server_right_columns.windows(2).all(|pair| pair[0] == pair[1]));

        assert_eq!(cell_at_source_column(&diagram.rows[26], 46).render_column(), 51);
        assert_eq!(cell_at_source_column(&diagram.rows[26], 78).render_column(), 84);
    }

    #[test]
    fn aligns_wps_rolling_window_box_to_its_rightmost_existing_edge() {
        let diagram = detect_ascii_diagram(&lines(&[
            "时间轴（30天滚动窗口）",
            "",
            "Day -29          Day -7           Day -1    Today    Day +1",
            "  │                │                │         │         │",
            "  ├────────────────┼────────────────┼─────────┼─────────┤",
            "  │                 │                │         │         │",
            "  │  30天历史行为   │  近期模式      │ 昨日    │ 今日    │ 次日展示",
            "  │  (服务端存储)   │  (7天细粒度)   │ 增量    │ WPS     │ 焦点项",
            "  │                 │                │ 上传    │ 首页    │",
            "  │                 │                │         │         │",
            "  ▼                 ▼                ▼         ▼         ▼",
            " ┌─────────────────────────────────────────────────────────┐",
            " │  服务端滚动窗口（始终保留最近 30 天行为摘要）             │",
            " │                                                         │",
            " │  每天凌晨：                                              │",
            " │    ① 接收昨日增量 DSL 包 → 追加到滚动窗口               │",
            " │    ② 淘汰 Day-31 旧数据                                 │",
            " │    ③ 更新画像（5个维度全量重算）                        │",
            " │    ④ 短期意图预测（基于 1-7 天近期行为）                │",
            " │    ⑤ 周期性模式检测（基于 7-30 天中长周期）             │",
            " │    ⑥ 合并焦点项 → 排序 → 存储                          │",
            " └─────────────────────────────────────────────────────────┘",
        ]))
        .expect("fixture must be detected");

        let right_columns = (11..=20)
            .map(|row_index| right_edge_cell(&diagram.rows[row_index]).render_column())
            .collect::<Vec<_>>();
        assert!(right_columns.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn missing_outer_side_does_not_claim_an_inner_adjacent_vertical_edge() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌───────────┐",
            "│ ┌─────┐   │",
            "│ │ │   │ │",
            "│ └─────┘   │",
            "└───────────┘",
        ]))
        .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[2], 10).render_column(), 10);
    }

    #[test]
    fn aligns_misaligned_complete_rectangle_right_edges() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌─ 本地日志（30天滚动）─────────┐",
            "│ · 文件操作（打开/关闭/保存）  │",
            "│ · 模板使用记录               │",
            "└──────────────────────────────┘",
        ]))
        .expect("fixture must be detected as an ASCII diagram");

        let source_columns =
            diagram.rows.iter().map(|row| right_edge_cell(row).column).collect::<Vec<_>>();
        let render_columns =
            diagram.rows.iter().map(|row| right_edge_cell(row).render_column()).collect::<Vec<_>>();

        assert_eq!(source_columns, vec![32, 32, 31, 31]);
        assert_eq!(render_columns, vec![32, 32, 32, 32]);
    }

    #[test]
    fn nested_tracks_recompute_outer_target_after_inner_suffix_shift() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌──────────┐",
            "│ ┌──────┐ │",
            "│ │ x  │   │",
            "│ └─────┘  │",
            "└─────────┘",
        ]))
        .expect("fixture must be detected");

        let inner_right_columns = [(1, 9), (2, 7), (3, 8)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert_eq!(inner_right_columns, [9, 9, 9]);

        let outer_right_columns = [(0, 11), (1, 11), (2, 11), (3, 11), (4, 10)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert_eq!(outer_right_columns, [13, 13, 13, 13, 13]);
    }

    #[test]
    fn aligns_discontinuous_rectangle_to_rightmost_existing_edge() {
        let diagram =
            detect_ascii_diagram(&lines(&["┌────┐", "│ a    │", "│       ", "│ c  │", "└────┘"]))
                .expect("fixture must be detected");

        let top_right = cell_at_source_column(&diagram.rows[0], 5);
        let first_existing_side = cell_at_source_column(&diagram.rows[1], 7);
        let second_existing_side = cell_at_source_column(&diagram.rows[3], 5);
        let bottom_right = cell_at_source_column(&diagram.rows[4], 5);

        assert_eq!(top_right.render_column(), 7);
        assert_eq!(first_existing_side.render_column(), 7);
        assert!(is_vertical_edge(cell_at_source_column(&diagram.rows[2], 0)));
        assert_eq!(second_existing_side.render_column(), 7);
        assert_eq!(bottom_right.render_column(), 7);
    }

    #[test]
    fn leaves_ambiguous_discontinuous_right_edges_at_source_columns() {
        let diagram = detect_ascii_diagram(&lines(&["┌─────┐", "│ a  │ │", "└─────┘"]))
            .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 5);
        assert_eq!(cell_at_source_column(&diagram.rows[1], 7).render_column(), 7);
        assert_eq!(cell_at_source_column(&diagram.rows[0], 6).render_column(), 6);
    }

    #[test]
    fn missing_inner_side_does_not_claim_the_outer_right_edge() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌──────────┐",
            "│ ┌────┐   │",
            "│ │ x      │",
            "│ └────┘   │",
            "└──────────┘",
        ]))
        .expect("fixture must be detected");

        let only_right_edge_on_gap_row = right_edge_cell(&diagram.rows[2]);
        assert_eq!(only_right_edge_on_gap_row.column, 11);
        assert_eq!(only_right_edge_on_gap_row.render_column(), 11);
        assert_eq!(diagram.rows[2].cells.iter().filter(|cell| is_vertical_edge(cell)).count(), 3);
    }

    #[test]
    fn missing_left_side_does_not_claim_an_unrelated_right_edge() {
        let diagram = detect_ascii_diagram(&lines(&[
            "  ┌────┐",
            "  │ a  │",
            "          │",
            "  │ c  │",
            "  └────┘",
        ]))
        .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[0], 7).render_column(), 7);
        assert_eq!(cell_at_source_column(&diagram.rows[1], 7).render_column(), 7);
        assert_eq!(cell_at_source_column(&diagram.rows[2], 10).render_column(), 10);
        assert_eq!(cell_at_source_column(&diagram.rows[3], 7).render_column(), 7);
        assert_eq!(cell_at_source_column(&diagram.rows[4], 7).render_column(), 7);
    }

    #[test]
    fn parallel_rectangle_tracks_accumulate_from_left_to_right() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌────┐  ┌──────┐",
            "│a │    │b   │",
            "└───┘   └─────┘",
        ]))
        .expect("fixture must be detected");

        let left_right_columns = [(0, 5), (1, 3), (2, 4)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert_eq!(left_right_columns, [5, 5, 5]);

        let right_left_columns = [(0, 8), (1, 8), (2, 8)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert_eq!(right_left_columns, [10, 10, 10]);

        let right_right_columns = [(0, 15), (1, 13), (2, 14)]
            .map(|(row, source)| cell_at_source_column(&diagram.rows[row], source).render_column());
        assert_eq!(right_right_columns, [17, 17, 17]);
    }

    #[test]
    fn leaves_incomplete_rectangle_but_aligns_closed_rectangle_suffixes() {
        let incomplete = detect_ascii_diagram(&lines(&["┌────┐", "│x  │"]))
            .expect("thresholds still identify the incomplete fixture as a diagram");
        assert_eq!(right_edge_cell(&incomplete.rows[1]).render_column(), 4);

        let sequential_bottoms = detect_ascii_diagram(&lines(&["┌────┐", "│x │", "└──┘", "└───┘"]))
            .expect("thresholds still identify the closed fixture as a diagram");
        assert_eq!(right_edge_cell(&sequential_bottoms.rows[1]).render_column(), 5);

        let row_with_text = detect_ascii_diagram(&lines(&["┌────┐", "│a │x", "└───┘"]))
            .expect("fixture must be detected");
        assert_eq!(cell_at_source_column(&row_with_text.rows[1], 3).render_column(), 5);
        assert_eq!(cell_at_source_column(&row_with_text.rows[1], 4).render_column(), 6);
    }

    #[test]
    fn shifts_horizontal_edge_in_the_row_suffix() {
        let horizontal_gap = detect_ascii_diagram(&lines(&["┌──┐─", "│  │", "└────┘"]))
            .expect("fixture must be detected");
        assert_eq!(cell_at_source_column(&horizontal_gap.rows[0], 3).render_column(), 5);
        assert_eq!(cell_at_source_column(&horizontal_gap.rows[0], 4).render_column(), 6);
        assert_eq!(cell_at_source_column(&horizontal_gap.rows[1], 3).render_column(), 5);
    }

    #[test]
    fn shifts_text_after_a_moved_corner() {
        let target_text = detect_ascii_diagram(&lines(&["┌──┐ X", "│  │", "└────┘"]))
            .expect("fixture must be detected");
        assert_eq!(cell_at_source_column(&target_text.rows[0], 3).render_column(), 5);
        assert_eq!(cell_at_source_column(&target_text.rows[0], 5).render_column(), 7);
        assert_eq!(cell_at_source_column(&target_text.rows[1], 3).render_column(), 5);
    }

    #[test]
    fn aligns_right_edges_when_the_row_suffix_is_whitespace() {
        let whitespace_path = detect_ascii_diagram(&lines(&["┌──┐  ", "│  │", "└────┘"]))
            .expect("fixture must be detected");
        assert_eq!(cell_at_source_column(&whitespace_path.rows[0], 3).render_column(), 5);
        assert_eq!(cell_at_source_column(&whitespace_path.rows[1], 3).render_column(), 5);
        assert_eq!(cell_at_source_column(&whitespace_path.rows[2], 5).render_column(), 5);
    }

    #[test]
    fn shifts_text_after_a_moved_middle_edge() {
        let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│  │ X", "└────┘"]))
            .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 5);
        assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 7);
    }

    #[test]
    fn shifts_box_character_after_a_moved_middle_edge() {
        let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│  │ ─", "└────┘"]))
            .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 5);
        assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 7);
    }

    #[test]
    fn aligns_middle_edge_to_blank_target_column() {
        let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│  │  ", "└────┘"]))
            .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[1], 3).render_column(), 5);
    }

    #[test]
    fn keeps_middle_edge_at_matching_target_column() {
        let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│    │", "└────┘"]))
            .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 5);
    }

    #[test]
    fn keeps_vertically_stacked_rectangles_and_arrow_at_source_columns() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌────┐",
            "│ a  │",
            "└────┘",
            "  ↓",
            "┌────┐",
            "│ b  │",
            "└────┘",
        ]))
        .expect("fixture must be detected");

        for row_index in [0, 1, 2, 4, 5, 6] {
            let right_edge = right_edge_cell(&diagram.rows[row_index]);
            assert_eq!(right_edge.render_column(), right_edge.column);
        }
        let arrow = cell_at_source_column(&diagram.rows[3], 2);
        assert_eq!(arrow.text, "↓");
        assert_eq!(arrow.render_column(), arrow.column);
    }

    #[test]
    fn aligns_corner_tracks_when_outer_rectangle_lacks_middle_edges() {
        let diagram = detect_ascii_diagram(&lines(&[
            "┌─────────┐",
            "  ┌─────┐",
            "  │ x  │",
            "  └────┘",
            "└────────┘",
        ]))
        .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[2], 7).render_column(), 8);
        assert_eq!(cell_at_source_column(&diagram.rows[0], 10).render_column(), 10);
        assert_eq!(cell_at_source_column(&diagram.rows[4], 9).render_column(), 10);
    }

    #[test]
    fn aligns_uniquely_supported_shared_right_edges() {
        let diagram =
            detect_ascii_diagram(&lines(&["┌─┌────┐──┐", "│ │x │", "│ │y  │", "└─└───┘─┘"]))
                .expect("fixture must be detected");

        assert_eq!(cell_at_source_column(&diagram.rows[1], 5).render_column(), 5);
        assert_eq!(cell_at_source_column(&diagram.rows[2], 6).render_column(), 7);
        assert_eq!(cell_at_source_column(&diagram.rows[3], 6).render_column(), 7);
        assert_eq!(cell_at_source_column(&diagram.rows[0], 10).render_column(), 10);
        assert_eq!(cell_at_source_column(&diagram.rows[3], 8).render_column(), 10);
    }

    #[test]
    fn detects_light_box_diagram_with_cjk_label() {
        let diagram = detect_ascii_diagram(&lines(&["┌────┐", "│中文│", "└────┘"]))
            .expect("a multi-line light box must be detected");

        assert_eq!(diagram.rows.len(), 3);
        assert_eq!(diagram.column_count, 6);
        assert!(diagram.rows.iter().all(|row| row.column_count == 6));
        assert_eq!(diagram.rows[1].cells[1].column_width, 2);
    }

    #[test]
    fn assigns_zero_columns_to_standalone_combining_mark_graphemes() {
        assert_eq!(grapheme_column_width("\u{0301}"), 0);
    }

    #[test]
    fn assigns_complete_east_asian_widths_to_graphemes() {
        assert_eq!(grapheme_column_width("A"), 1);
        assert_eq!(grapheme_column_width("｟"), 2);
        assert_eq!(grapheme_column_width("￦"), 2);
        assert_eq!(grapheme_column_width("ᄀ"), 2);
        assert_eq!(grapheme_column_width("e\u{0301}"), 1);
        assert_eq!(grapheme_column_width("\u{0301}"), 0);
    }

    #[test]
    fn enforces_box_drawing_detection_threshold_boundaries() {
        assert!(detect_ascii_diagram(&lines(&["┌─┐", "│  │"])).is_none());
        assert!(detect_ascii_diagram(&lines(&["┌────┐", "plain text"])).is_none());
        assert!(detect_ascii_diagram(&lines(&["┌─┐", "└─┘"])).is_some());
    }

    #[test]
    fn preserves_unknown_box_drawing_character_as_text_cell() {
        let diagram =
            detect_ascii_diagram(&lines(&["┌──┐", "│═ │", "└──┘"])).expect("fixture is a diagram");
        let unknown_cell = diagram.rows[1]
            .cells
            .iter()
            .find(|cell| cell.text == "═")
            .expect("fixture contains unknown box drawing character");

        assert_eq!(unknown_cell.box_connections, None);
        assert_eq!(unknown_cell.column_width, 1);
    }

    #[test]
    fn rejects_normal_code_and_single_box_character() {
        assert!(
            detect_ascii_diagram(&lines(&["let result = value + 1;", "println!(\"{result}\");"]))
                .is_none()
        );
        assert!(detect_ascii_diagram(&lines(&["┌ value", "plain text"])).is_none());
    }

    #[test]
    fn maps_all_supported_light_box_connections() {
        let expected_connections = [
            ("─", BoxConnections::LEFT_RIGHT),
            ("│", BoxConnections::UP_DOWN),
            ("┌", BoxConnections::RIGHT_DOWN),
            ("┐", BoxConnections { left: true, down: true, ..BoxConnections::default() }),
            ("└", BoxConnections { right: true, up: true, ..BoxConnections::default() }),
            ("┘", BoxConnections { left: true, up: true, ..BoxConnections::default() }),
            ("├", BoxConnections { right: true, up: true, down: true, left: false }),
            ("┤", BoxConnections { left: true, up: true, down: true, right: false }),
            ("┬", BoxConnections { left: true, right: true, down: true, up: false }),
            ("┴", BoxConnections { left: true, right: true, up: true, down: false }),
            ("┼", BoxConnections::ALL),
        ];

        for (character, expected) in expected_connections {
            assert_eq!(
                box_connections(character),
                Some(expected),
                "unexpected connections for {character}"
            );
        }
    }
}
