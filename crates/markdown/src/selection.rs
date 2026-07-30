//! Selection state and hit-testing for markdown preview.
//!
//! Contains [`ViewPos`] (position within rendered flat lines),
//! [`SelectionState`] (anchor + cursor), and pure helper functions
//! for hit-testing, word boundaries, and highlight generation.

use crate::grapheme_map::byte_at_grapheme_index;
use crate::layout::{FlatLine, grapheme_at_x, grapheme_x};
use core::text::word_class::{CharClass, classify};
use ui::core::geom::Rect;
use ui::core::paint::{DrawCmd, DrawList};

/// Position within the preview's rendered text.
/// `flat_line_idx` indexes into `LazyLayout.flat_lines` (reading-order).
/// `grapheme_pos` is the grapheme cluster offset within the line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewPos {
    pub flat_line_idx: usize,
    pub grapheme_pos: usize,
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
    Some(ViewPos { flat_line_idx: best_line_idx, grapheme_pos })
}

/// Find word boundaries at the given position.
/// Returns `(word_start, word_end)` as [`ViewPos`] on the same line.
/// Uses Unicode-aware character classification matching VS Code behavior:
/// word = contiguous run of the same character class (letter/digit/underscore/other).
/// Find word boundaries at the given position.
///
/// Returns `(word_start, word_end)` as [`ViewPos`] on the same line.
/// The internal algorithm operates at Rust char granularity (matching VS Code
/// behavior).  For ASCII and CJK text where each char is its own grapheme,
/// `grapheme_pos` and char index coincide.
pub fn word_at_pos(flat_lines: &[FlatLine], pos: ViewPos) -> (ViewPos, ViewPos) {
    let Some(line) = flat_lines.get(pos.flat_line_idx) else {
        return (pos, pos);
    };
    let text = &line.text;
    if text.is_empty() {
        return (pos, pos);
    }

    let char_count = text.chars().count();
    let pos_char = pos.grapheme_pos.min(char_count);

    if pos_char >= char_count {
        if char_count == 0 {
            return (pos, pos);
        }
        let last_ch = text.chars().last().unwrap();
        let last_class = classify(last_ch);
        if last_class == CharClass::Whitespace {
            return (pos, pos);
        }
        let mut start = char_count - 1;
        for (i, ch) in text.chars().rev().enumerate() {
            if classify(ch) != last_class {
                start = char_count - i;
                break;
            }
            if char_count - 1 - i == 0 {
                start = 0;
                break;
            }
        }
        return (
            ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: start },
            ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: char_count },
        );
    }

    let chars: Vec<char> = text.chars().collect();
    let clicked_class = classify(chars[pos_char]);

    if clicked_class == CharClass::Whitespace {
        let mut s = pos_char;
        while s > 0 && classify(chars[s - 1]) == CharClass::Whitespace {
            s -= 1;
        }
        let mut e = pos_char;
        while e < chars.len() && classify(chars[e]) == CharClass::Whitespace {
            e += 1;
        }
        return (
            ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: s },
            ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: e },
        );
    }

    let mut start = pos_char;
    while start > 0 && classify(chars[start - 1]) == clicked_class {
        start -= 1;
    }
    let mut end = pos_char;
    while end < chars.len() && classify(chars[end]) == clicked_class {
        end += 1;
    }

    (
        ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: start },
        ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: end },
    )
}

/// Find line boundaries at the given position.
/// Returns `(line_start, line_end)` — `grapheme_pos` 0 and grapheme count.
pub fn line_range_at_pos(flat_lines: &[FlatLine], pos: ViewPos) -> (ViewPos, ViewPos) {
    let g_count = flat_lines.get(pos.flat_line_idx).map_or(0, grapheme_count_on_line);
    (
        ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: 0 },
        ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: g_count },
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
        self.anchor = Some(ViewPos { flat_line_idx: 0, grapheme_pos: 0 });
        let last_idx = flat_lines.len() - 1;
        let last_g = flat_lines.last().map_or(0, grapheme_count_on_line);
        self.cursor = Some(ViewPos { flat_line_idx: last_idx, grapheme_pos: last_g });
    }

    /// Extract the selected text from the flat lines.
    pub fn selected_text(&self, flat_lines: &[FlatLine]) -> Option<String> {
        let (start, end) = self.range()?;
        let end_idx = end.flat_line_idx.min(flat_lines.len().saturating_sub(1));

        let mut result = String::new();
        for (idx, line) in flat_lines.iter().enumerate().take(end_idx + 1).skip(start.flat_line_idx)
        {
            let text = &line.text;
            let g_start = if idx == start.flat_line_idx { start.grapheme_pos } else { 0 };
            let g_end = if idx == end.flat_line_idx {
                end.grapheme_pos
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
        let end_idx = end.flat_line_idx.min(flat_lines.len().saturating_sub(1));

        for (idx, line) in flat_lines.iter().enumerate().take(end_idx + 1).skip(start.flat_line_idx)
        {
            let line_y = line.rect.y - scroll_y + offset_y;
            let line_h = line.rect.h;
            let viewport_top = offset_y;
            let viewport_bottom = offset_y + viewport_h;
            if line_y + line_h < viewport_top || line_y > viewport_bottom {
                continue;
            }

            let g_start = if idx == start.flat_line_idx { start.grapheme_pos } else { 0 };
            let g_end = if idx == end.flat_line_idx {
                end.grapheme_pos
            } else {
                grapheme_count_on_line(line)
            };

            let x0 = line.rect.x + offset_x + grapheme_x(line, g_start);
            let is_end_line = idx == end.flat_line_idx;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{FlatLine, types::VisualLineProjection};
    use crate::projection::{ProjectionOwnerId, SourceAnchor};
    use ui::core::geom::Rect;

    fn make_flat_line(text: &str) -> FlatLine {
        FlatLine {
            flat_idx: 0,
            rect: Rect::new(0.0, 0.0, 200.0, 20.0),
            text: text.to_string(),
            font_size: 14.0,
            shaped: None,
            requires_source_projection: false,
            source_projection: None,
        }
    }

    #[test]
    fn selection_selected_text_does_not_split_combining_grapheme() {
        let flat_lines = vec![make_flat_line("xe\u{0301}y")];
        let mut sel = SelectionState::new();
        sel.anchor = Some(ViewPos { flat_line_idx: 0, grapheme_pos: 1 });
        sel.cursor = Some(ViewPos { flat_line_idx: 0, grapheme_pos: 2 });

        assert_eq!(sel.selected_text(&flat_lines), Some("e\u{0301}".to_string()));
    }

    #[test]
    fn selection_selected_text_does_not_split_zwj_emoji() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let text = format!("x{emoji}y");
        let flat_lines = vec![make_flat_line(&text)];
        let mut sel = SelectionState::new();
        sel.anchor = Some(ViewPos { flat_line_idx: 0, grapheme_pos: 1 });
        sel.cursor = Some(ViewPos { flat_line_idx: 0, grapheme_pos: 2 });

        assert_eq!(sel.selected_text(&flat_lines), Some(emoji.to_string()));
    }

    #[test]
    fn selection_select_all_counts_graphemes_not_chars() {
        // "e\u{0301}" is 2 chars but 1 grapheme, so grapheme_count = 3
        let flat_lines = vec![make_flat_line("xe\u{0301}y")];
        let mut sel = SelectionState::new();
        sel.select_all(&flat_lines);

        let (start, end) = sel.range().unwrap();
        assert_eq!(start.flat_line_idx, 0);
        assert_eq!(start.grapheme_pos, 0);
        assert_eq!(end.flat_line_idx, 0);
        assert_eq!(end.grapheme_pos, 3); // x, é, y
    }

    #[test]
    fn selection_line_range_returns_grapheme_count() {
        let flat_lines = vec![make_flat_line("xe\u{0301}y")];
        let (_line_start, line_end) =
            line_range_at_pos(&flat_lines, ViewPos { flat_line_idx: 0, grapheme_pos: 0 });
        assert_eq!(line_end.grapheme_pos, 3); // grapheme count, not 4 (char count)
    }

    #[test]
    fn selection_line_range_uses_projection_boundaries() {
        let flat_lines = vec![FlatLine {
            flat_idx: 0,
            rect: Rect::new(0.0, 0.0, 200.0, 20.0),
            text: "x".to_string(),
            font_size: 14.0,
            shaped: None,
            requires_source_projection: true,
            source_projection: Some(VisualLineProjection {
                flat_line_idx: 0,
                owner: ProjectionOwnerId::Block { block_start: 4, logical_line: 0 },
                boundaries: vec![SourceAnchor::downstream(4), SourceAnchor::downstream(5)],
                source_extent: 4..5,
                collapsed: Vec::new(),
            }),
        }];

        let (_line_start, line_end) =
            line_range_at_pos(&flat_lines, ViewPos { flat_line_idx: 0, grapheme_pos: 0 });

        assert_eq!(line_end.grapheme_pos, 1);
    }
}
