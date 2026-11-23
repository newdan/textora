pub(in crate::layout::ascii_diagram) use std::collections::{BTreeMap, BTreeSet};
pub(in crate::layout::ascii_diagram) use std::ops::Range;
pub(in crate::layout::ascii_diagram) use std::sync::LazyLock;

pub(in crate::layout::ascii_diagram) use regex::Regex;
pub(in crate::layout::ascii_diagram) use unicode_width::UnicodeWidthChar;

pub(in crate::layout::ascii_diagram) use super::types::LaidOutLine;
use crate::grapheme_map::grapheme_slices;

pub(in crate::layout::ascii_diagram) const MINIMUM_BOX_DRAWING_CHARACTERS: usize = 6;
pub(in crate::layout::ascii_diagram) const MINIMUM_BOX_DRAWING_LINES: usize = 2;
pub(in crate::layout::ascii_diagram) static STANDALONE_COMBINING_MARK_GRAPHEME: LazyLock<Regex> =
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
    pub(in crate::layout::ascii_diagram) render_column_shift: usize,
    pub(in crate::layout::ascii_diagram) left_extension_columns: usize,
}

impl AsciiDiagramCell {
    pub(crate) fn render_column(&self) -> usize {
        self.column + self.render_column_shift
    }

    pub(crate) fn left_extension_columns(&self) -> usize {
        self.left_extension_columns
    }

    pub(in crate::layout::ascii_diagram) fn shift_render_column_by(&mut self, column_count: usize) {
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
pub(in crate::layout::ascii_diagram) struct AsciiDiagramEntry {
    pub(in crate::layout::ascii_diagram) source_range: Range<usize>,
    pub(in crate::layout::ascii_diagram) content_anchor: usize,
    pub(in crate::layout::ascii_diagram) diagram: AsciiDiagram,
}

/// Crate-private render metadata keyed by the source anchor of a code block's first line.
///
/// This keeps [`super::types::LaidOutBlock`] source-compatible for external struct literals.
#[derive(Clone, Debug, Default)]
pub(crate) struct AsciiDiagramRegistry {
    pub(in crate::layout::ascii_diagram) entries: BTreeMap<usize, AsciiDiagramEntry>,
    pub(in crate::layout::ascii_diagram) content_anchors: BTreeMap<usize, usize>,
    pub(in crate::layout::ascii_diagram) selection_range: Option<Range<usize>>,
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

pub(in crate::layout::ascii_diagram) fn diagram_content_anchor(
    lines: &[LaidOutLine],
) -> Option<usize> {
    lines.first()?.source_projection.as_ref().map(|projection| projection.source_extent.start)
}

pub(in crate::layout::ascii_diagram) fn ranges_intersect(
    left: &Range<usize>,
    right: &Range<usize>,
) -> bool {
    left.start < right.end && right.start < left.end
}

pub(in crate::layout::ascii_diagram) fn selection_intersects(
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

pub(in crate::layout::ascii_diagram) fn grid_row(line: &str) -> (AsciiDiagramRow, usize, bool) {
    let mut cells = Vec::new();
    let mut column = 0usize;
    let mut box_count = 0usize;
    let mut has_corner = false;

    for grapheme in grapheme_slices(line) {
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

pub(in crate::layout::ascii_diagram) fn open_timeline_anchor_indices(
    row: &AsciiDiagramRow,
) -> Vec<usize> {
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

pub(in crate::layout::ascii_diagram) fn open_timeline_member_indices(
    row: &AsciiDiagramRow,
) -> Vec<usize> {
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

pub(in crate::layout::ascii_diagram) fn has_open_timeline_structure(
    rows: &[AsciiDiagramRow],
) -> bool {
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

pub(in crate::layout::ascii_diagram) fn grapheme_column_width(grapheme: &str) -> usize {
    if STANDALONE_COMBINING_MARK_GRAPHEME.is_match(grapheme) {
        return 0;
    }

    grapheme.chars().next().and_then(UnicodeWidthChar::width).unwrap_or(0)
}

#[path = "ascii_diagram_align.rs"]
mod ascii_diagram_align;
#[allow(unused_imports)]
pub(in crate::layout::ascii_diagram) use ascii_diagram_align::*;

#[cfg(test)]
#[path = "ascii_diagram_tests.rs"]
mod tests;
