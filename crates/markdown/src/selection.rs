//! Selection state and hit-testing for markdown preview.
//!
//! Contains [`ViewPos`] (position within rendered flat lines),
//! [`SelectionState`] (anchor + cursor), and pure helper functions
//! for hit-testing, word boundaries, and highlight generation.

use crate::grapheme_map::{byte_at_grapheme_index, grapheme_slices};
use crate::layout::{FlatLine, grapheme_at_x, grapheme_x};
use crate::projection::ProjectionOwnerId;
use textora_core::text::word_class::{CharClass, classify};
use ui::core::geom::Rect;
use ui::core::paint::{DrawCmd, DrawList};

/// Index in the current reading-order array of rendered lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FlatLineIdx(usize);

impl FlatLineIdx {
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

/// Grapheme boundary index within the rendered text of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphemeIdx(usize);

impl GraphemeIdx {
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn get(self) -> usize {
        self.0
    }
}

/// Position within the preview's rendered text.
/// `flat_line_idx` indexes into `LazyLayout.flat_lines` (reading-order).
/// `grapheme_pos` is the grapheme cluster offset within the line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewPos {
    pub flat_line_idx: FlatLineIdx,
    pub grapheme_pos: GraphemeIdx,
}

impl ViewPos {
    pub(crate) const fn from_indices(flat_line_idx: usize, grapheme_pos: usize) -> Self {
        Self {
            flat_line_idx: FlatLineIdx::new(flat_line_idx),
            grapheme_pos: GraphemeIdx::new(grapheme_pos),
        }
    }

    pub(crate) const fn indices(self) -> (usize, usize) {
        (self.flat_line_idx.get(), self.grapheme_pos.get())
    }
}

// ── Pure functions ────────────────────────────────────────────────

/// Hit-test a screen pixel position to a [`ViewPos`] in the rendered text.
/// Snaps to the nearest line for clicks in empty space.
pub fn hit_test(
    flat_lines: &[FlatLine],
    scroll_y: f32,
    px: f32,
    py: f32,
    offset_x: f32,
    offset_y: f32,
) -> Option<ViewPos> {
    if flat_lines.is_empty() {
        return None;
    }
    let doc_x = px - offset_x;
    let doc_y = py - offset_y + scroll_y;

    let mut best_line_idx = 0;
    let mut min_dist = f32::MAX;

    for (i, line) in flat_lines.iter().enumerate() {
        let rect = &line.rect;
        let dx = if doc_x < rect.x {
            rect.x - doc_x
        } else if doc_x > rect.x + rect.w {
            doc_x - (rect.x + rect.w)
        } else {
            0.0
        };
        let dy = if doc_y < rect.y {
            rect.y - doc_y
        } else if doc_y > rect.y + rect.h {
            doc_y - (rect.y + rect.h)
        } else {
            0.0
        };
        let dist = dx * dx + dy * dy;
        if dist < min_dist {
            min_dist = dist;
            best_line_idx = i;
        }
        if dist == 0.0 {
            break;
        }
    }

    let line = &flat_lines[best_line_idx];
    let rel_x = doc_x - line.rect.x;
    let grapheme_pos = if rel_x <= 0.0 { 0 } else { grapheme_at_x(line, rel_x) };
    Some(ViewPos::from_indices(best_line_idx, grapheme_pos))
}

/// Find the run of graphemes in the same word class at the given position.
/// A cluster takes the class of its first scalar, so combining marks and joiners
/// inherit the class of the cluster they belong to.
pub fn word_at_pos(flat_lines: &[FlatLine], pos: ViewPos) -> (ViewPos, ViewPos) {
    let Some(line) = flat_lines.get(pos.flat_line_idx.get()) else {
        return (pos, pos);
    };
    let classes: Vec<_> = grapheme_slices(&line.text)
        .map(|grapheme| {
            classify(grapheme.chars().next().expect("a grapheme slice contains a scalar"))
        })
        .collect();
    let Some(&last_class) = classes.last() else {
        return (pos, pos);
    };

    let clicked = pos.grapheme_pos.get();
    if clicked >= classes.len() && last_class == CharClass::Whitespace {
        return (pos, pos);
    }
    let clicked = clicked.min(classes.len() - 1);
    let clicked_class = classes[clicked];
    let start = classes[..clicked]
        .iter()
        .rposition(|&class| class != clicked_class)
        .map_or(0, |index| index + 1);
    let end =
        clicked + classes[clicked..].iter().take_while(|&&class| class == clicked_class).count();
    (
        ViewPos { grapheme_pos: GraphemeIdx::new(start), ..pos },
        ViewPos { grapheme_pos: GraphemeIdx::new(end), ..pos },
    )
}

/// Find line boundaries at the given position.
/// Returns `(line_start, line_end)` — `grapheme_pos` 0 and grapheme count.
pub fn line_range_at_pos(flat_lines: &[FlatLine], pos: ViewPos) -> (ViewPos, ViewPos) {
    let g_count = flat_lines.get(pos.flat_line_idx.get()).map_or(0, grapheme_count_on_line);
    (
        ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: GraphemeIdx::new(0) },
        ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: GraphemeIdx::new(g_count) },
    )
}

/// Grapheme count on a [`FlatLine`], read from its canonical source projection.
fn grapheme_count_on_line(line: &FlatLine) -> usize {
    line.source_projection.as_ref().map_or_else(
        || crate::grapheme_map::grapheme_count(&line.text),
        |projection| projection.boundaries.len().saturating_sub(1),
    )
}

/// Visual text byte position at a given grapheme index within a [`FlatLine`].
fn visual_byte_at_grapheme_on_line(line: &FlatLine, grapheme_idx: usize) -> usize {
    byte_at_grapheme_index(&line.text, grapheme_idx)
}

// ── SelectionState ────────────────────────────────────────────────

/// Anchor + cursor pair for text selection in the preview.
pub struct SelectionState {
    pub anchor: Option<ViewPos>,
    pub cursor: Option<ViewPos>,
}

impl Default for SelectionState {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectionState {
    pub fn new() -> Self {
        Self { anchor: None, cursor: None }
    }

    /// Returns the normalized selection range (`start <= end`) or `None`.
    pub fn range(&self) -> Option<(ViewPos, ViewPos)> {
        let anchor = self.anchor?;
        let cursor = self.cursor?;
        if anchor == cursor {
            return None;
        }
        if (anchor.flat_line_idx, anchor.grapheme_pos)
            <= (cursor.flat_line_idx, cursor.grapheme_pos)
        {
            Some((anchor, cursor))
        } else {
            Some((cursor, anchor))
        }
    }

    /// Whether there is an active selection.
    pub fn has_selection(&self) -> bool {
        self.anchor.is_some() && self.cursor.is_some() && self.anchor != self.cursor
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.anchor = None;
        self.cursor = None;
    }

    /// Select all rendered text.
    pub fn select_all(&mut self, flat_lines: &[FlatLine]) {
        if flat_lines.is_empty() {
            return;
        }
        self.anchor = Some(ViewPos::from_indices(0, 0));
        let last_idx = flat_lines.len() - 1;
        let last_g = flat_lines.last().map_or(0, grapheme_count_on_line);
        self.cursor = Some(ViewPos::from_indices(last_idx, last_g));
    }

    /// Extract the selected text from the flat lines.
    pub fn selected_text(&self, flat_lines: &[FlatLine]) -> Option<String> {
        let (start, end) = self.range()?;
        let end_idx = end.flat_line_idx.get().min(flat_lines.len().saturating_sub(1));

        let mut result = String::new();
        for (idx, line) in
            flat_lines.iter().enumerate().take(end_idx + 1).skip(start.flat_line_idx.get())
        {
            let text = &line.text;
            let g_start =
                if idx == start.flat_line_idx.get() { start.grapheme_pos.get() } else { 0 };
            let g_end = if idx == end.flat_line_idx.get() {
                end.grapheme_pos.get()
            } else {
                grapheme_count_on_line(line)
            };
            let byte_start = visual_byte_at_grapheme_on_line(line, g_start);
            let byte_end = visual_byte_at_grapheme_on_line(line, g_end);
            if byte_start < byte_end {
                result.push_str(&text[byte_start..byte_end]);
            }
            let is_last = idx == end_idx;
            if !is_last {
                result.push('\n');
            }
        }
        Some(result)
    }

    /// Copy visible Markdown text while retaining source paragraph breaks.
    /// Wrapped visual rows share a source line and therefore add no newline.
    pub fn selected_reading_text(&self, flat_lines: &[FlatLine], source: &str) -> Option<String> {
        let (start, end) = self.range()?;
        let end_idx = end.flat_line_idx.get().min(flat_lines.len().checked_sub(1)?);
        let mut copied = String::new();
        let mut previous_line: Option<&FlatLine> = None;

        for (idx, line) in
            flat_lines.iter().enumerate().take(end_idx + 1).skip(start.flat_line_idx.get())
        {
            if let Some(previous) = previous_line {
                append_reading_separator(&mut copied, previous, line, source);
            }

            let first_grapheme =
                if idx == start.flat_line_idx.get() { start.grapheme_pos.get() } else { 0 };
            let last_grapheme = if idx == end.flat_line_idx.get() {
                end.grapheme_pos.get()
            } else {
                grapheme_count_on_line(line)
            };
            let first_byte = visual_byte_at_grapheme_on_line(line, first_grapheme);
            let last_byte = visual_byte_at_grapheme_on_line(line, last_grapheme);
            if first_byte < last_byte {
                copied.push_str(&line.text[first_byte..last_byte]);
            }
            previous_line = Some(line);
        }
        Some(copied)
    }

    /// Generate [`DrawList`] commands for selection highlight rectangles.
    pub fn highlights(
        &self,
        flat_lines: &[FlatLine],
        scroll_y: f32,
        offset_x: f32,
        offset_y: f32,
        viewport_h: f32,
        sel_color: [f32; 4],
    ) -> DrawList {
        let mut dl = DrawList::new();
        let (start, end) = match self.range() {
            Some(r) => r,
            None => return dl,
        };
        let end_idx = end.flat_line_idx.get().min(flat_lines.len().saturating_sub(1));

        for (idx, line) in
            flat_lines.iter().enumerate().take(end_idx + 1).skip(start.flat_line_idx.get())
        {
            let line_y = line.rect.y - scroll_y + offset_y;
            let line_h = line.rect.h;
            let viewport_top = offset_y;
            let viewport_bottom = offset_y + viewport_h;
            if line_y + line_h < viewport_top || line_y > viewport_bottom {
                continue;
            }

            let g_start =
                if idx == start.flat_line_idx.get() { start.grapheme_pos.get() } else { 0 };
            let g_end = if idx == end.flat_line_idx.get() {
                end.grapheme_pos.get()
            } else {
                grapheme_count_on_line(line)
            };

            let x0 = line.rect.x + offset_x + grapheme_x(line, g_start);
            let is_end_line = idx == end.flat_line_idx.get();
            let x1 = if !is_end_line && g_end >= grapheme_count_on_line(line) {
                line.rect.x + offset_x + line.rect.w
            } else {
                line.rect.x + offset_x + grapheme_x(line, g_end)
            };
            let w = (x1 - x0).max(0.0);
            if w > 0.0 {
                dl.cmds.push(DrawCmd::FillRect {
                    rect: Rect::new(x0, line_y, w, line_h),
                    color: sel_color,
                    radius: 0.0,
                });
            }
        }
        dl
    }
}

fn append_reading_separator(
    copied: &mut String,
    previous: &FlatLine,
    next: &FlatLine,
    source: &str,
) {
    let previous_projection = previous.source_projection.as_ref();
    let next_projection = next.source_projection.as_ref();
    if previous_projection.zip(next_projection).is_some_and(|(previous, next)| {
        matches!(
            (previous.owner, next.owner),
            (
                ProjectionOwnerId::TableCell { table_start: first_table, row: first_row, column: first_column, .. },
                ProjectionOwnerId::TableCell { table_start: second_table, row: second_row, column: second_column, .. }
            ) if first_table == second_table && first_row == second_row && first_column != second_column
        )
    }) {
        copied.push('\t');
        return;
    }

    let previous_end = previous_projection
        .map(|projection| projection.source_extent.end)
        .or_else(|| previous.atomic_source_range.as_ref().map(|range| range.end));
    let next_start = next_projection
        .map(|projection| projection.source_extent.start)
        .or_else(|| next.atomic_source_range.as_ref().map(|range| range.start));
    let Some(source_gap) = previous_end
        .zip(next_start)
        .filter(|(end, start)| end <= start)
        .and_then(|(end, start)| source.get(end..start))
    else {
        copied.push('\n');
        return;
    };

    for _ in 0..longest_break_run(source_gap) {
        copied.push('\n');
    }
}

fn longest_break_run(source_gap: &str) -> usize {
    let mut longest_break_run = 0;
    let mut current_break_run = 0;
    for byte in source_gap.bytes() {
        if byte == b'\n' {
            current_break_run += 1;
            longest_break_run = longest_break_run.max(current_break_run);
        } else if byte != b'\r' {
            current_break_run = 0;
        }
    }
    longest_break_run
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{FlatLine, types::VisualLineProjection};
    use crate::projection::{ProjectionOwnerId, SourceAnchor, SourceByte};
    use ui::core::geom::Rect;

    fn make_flat_line(text: &str) -> FlatLine {
        FlatLine {
            flat_idx: 0,
            rect: Rect::new(0.0, 0.0, 200.0, 20.0),
            text: text.to_string(),
            font_size: 14.0,
            is_code: false,
            shaped: None,
            requires_source_projection: false,
            source_projection: None,
            atomic_source_range: None,
        }
    }

    #[test]
    fn word_at_pos_uses_grapheme_offsets_after_combining_marks() {
        let flat_lines = [make_flat_line("e\u{0301} foo")];
        let (start, end) = word_at_pos(&flat_lines, ViewPos::from_indices(0, 2));
        let selection = SelectionState { anchor: Some(start), cursor: Some(end) };

        assert_eq!(selection.selected_text(&flat_lines).as_deref(), Some("foo"));
        assert_eq!(
            (start.grapheme_pos, end.grapheme_pos),
            (GraphemeIdx::new(2), GraphemeIdx::new(5))
        );
    }

    #[test]
    fn word_at_pos_preserves_character_classes_at_grapheme_boundaries() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let line = format!("a中_2{emoji}!  e\u{0301}lan");
        let flat_lines = [make_flat_line(&line)];
        let cases = [
            (1, 0, 4, "a中_2"),
            (4, 4, 6, "👨\u{200D}👩\u{200D}👧!"),
            (7, 6, 8, "  "),
            (8, 8, 12, "e\u{0301}lan"),
            (12, 8, 12, "e\u{0301}lan"),
        ];
        for (clicked, expected_start, expected_end, text) in cases {
            let (start, end) = word_at_pos(&flat_lines, ViewPos::from_indices(0, clicked));
            let selection = SelectionState { anchor: Some(start), cursor: Some(end) };
            assert_eq!(selection.selected_text(&flat_lines).as_deref(), Some(text));
            assert_eq!(
                (start, end),
                (ViewPos::from_indices(0, expected_start), ViewPos::from_indices(0, expected_end))
            );
        }

        let flat_lines = [make_flat_line("word  ")];
        let end = ViewPos::from_indices(0, 6);
        assert_eq!(word_at_pos(&flat_lines, end), (end, end));
    }

    #[test]
    fn selection_selected_text_does_not_split_combining_grapheme() {
        let flat_lines = vec![make_flat_line("xe\u{0301}y")];
        let mut sel = SelectionState::new();
        sel.anchor = Some(ViewPos::from_indices(0, 1));
        sel.cursor = Some(ViewPos::from_indices(0, 2));

        assert_eq!(sel.selected_text(&flat_lines), Some("e\u{0301}".to_string()));
    }

    #[test]
    fn selection_selected_text_does_not_split_zwj_emoji() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let text = format!("x{emoji}y");
        let flat_lines = vec![make_flat_line(&text)];
        let mut sel = SelectionState::new();
        sel.anchor = Some(ViewPos::from_indices(0, 1));
        sel.cursor = Some(ViewPos::from_indices(0, 2));

        assert_eq!(sel.selected_text(&flat_lines), Some(emoji.to_string()));
    }

    #[test]
    fn selection_select_all_counts_graphemes_not_chars() {
        // "e\u{0301}" is 2 chars but 1 grapheme, so grapheme_count = 3
        let flat_lines = vec![make_flat_line("xe\u{0301}y")];
        let mut sel = SelectionState::new();
        sel.select_all(&flat_lines);

        let (start, end) = sel.range().unwrap();
        assert_eq!(start.flat_line_idx, FlatLineIdx::new(0));
        assert_eq!(start.grapheme_pos.get(), 0);
        assert_eq!(end.flat_line_idx, FlatLineIdx::new(0));
        assert_eq!(end.grapheme_pos.get(), 3); // x, é, y
    }

    #[test]
    fn selection_line_range_returns_grapheme_count() {
        let flat_lines = vec![make_flat_line("xe\u{0301}y")];
        let (_line_start, line_end) = line_range_at_pos(&flat_lines, ViewPos::from_indices(0, 0));
        assert_eq!(line_end.grapheme_pos.get(), 3); // grapheme count, not 4 (char count)
    }

    #[test]
    fn selection_line_range_uses_projection_boundaries() {
        let flat_lines = vec![FlatLine {
            flat_idx: 0,
            rect: Rect::new(0.0, 0.0, 200.0, 20.0),
            text: "x".to_string(),
            font_size: 14.0,
            is_code: false,
            shaped: None,
            requires_source_projection: true,
            source_projection: Some(VisualLineProjection {
                owner: ProjectionOwnerId::Block { block_start: 4, logical_line: 0 },
                boundaries: vec![
                    SourceAnchor::downstream(SourceByte::new(4)),
                    SourceAnchor::downstream(SourceByte::new(5)),
                ],
                source_extent: 4..5,
                collapsed: Vec::new(),
            }),
            atomic_source_range: None,
        }];

        let (_line_start, line_end) = line_range_at_pos(&flat_lines, ViewPos::from_indices(0, 0));

        assert_eq!(line_end.grapheme_pos.get(), 1);
    }
}
