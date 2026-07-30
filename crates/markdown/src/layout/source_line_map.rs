//! SourceLineMap — 源码行 ↔ 视觉坐标的桥接（方案 2026-07-06 阶段 1a）。
use std::ops::Range;

use crate::projection::ProjectionOwnerId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceLineSpan {
    pub index: usize,
    pub start: usize,
    pub end: usize,
    pub is_blank: bool,
}

impl SourceLineSpan {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.is_blank
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyRunPosition {
    pub index_in_run: usize,
    pub run_length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceLineRole {
    Paragraph,
    Heading,
    ListItem,
    BlockQuote,
    CodeBlock,
    TableCell,
    EditableEmpty,
    HiddenBlockSeparator,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceLineEntry {
    pub index: usize,
    pub start: usize,
    pub end: usize,
    pub is_blank: bool,
    pub role: SourceLineRole,
    pub y_top: f32,
    pub height: f32,
}

impl SourceLineEntry {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.is_blank
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedLineLayout {
    pub source_range: Range<usize>,
    pub y_top: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ProjectedEmptyLine {
    pub owner: ProjectionOwnerId,
    pub source_byte: usize,
    pub y_top: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HiddenBlockSeparator {
    pub source_range: Range<usize>,
    pub previous_anchor: usize,
}

#[derive(Clone, Debug, Default)]
pub struct SourceLineMap {
    lines: Vec<SourceLineEntry>,
    empty_runs: Vec<Option<EmptyRunPosition>>,
}

impl SourceLineMap {
    pub fn from_source(source: &str) -> Self {
        let spans = collect_lines(source);
        let empty_runs = collect_empty_runs(&spans);
        let lines = spans
            .into_iter()
            .map(|span| SourceLineEntry {
                index: span.index,
                start: span.start,
                end: span.end,
                is_blank: span.is_blank,
                role: SourceLineRole::Other,
                y_top: 0.0,
                height: 0.0,
            })
            .collect();
        Self { lines, empty_runs }
    }

    pub fn lines(&self) -> &[SourceLineEntry] {
        &self.lines
    }

    pub fn line_at_index(&self, index: usize) -> Option<SourceLineEntry> {
        self.lines.get(index).copied()
    }

    pub fn line_at_byte(&self, byte: usize) -> Option<SourceLineEntry> {
        let insertion = self.lines.partition_point(|line| line.start <= byte);
        if insertion == 0 {
            return None;
        }
        let candidate = self.lines[insertion - 1];
        if byte <= candidate.end { Some(candidate) } else { None }
    }

    pub fn is_hidden_block_separator(&self, index: usize) -> bool {
        matches!(
            self.empty_runs.get(index).and_then(|slot| slot.as_ref()),
            Some(pos) if pos.index_in_run == 0
        )
    }

    pub fn is_editable_empty(&self, index: usize) -> bool {
        matches!(
            self.empty_runs.get(index).and_then(|slot| slot.as_ref()),
            Some(pos) if pos.index_in_run >= 1
        )
    }

    pub fn empty_run_position(&self, index: usize) -> Option<EmptyRunPosition> {
        self.empty_runs.get(index).copied().flatten()
    }

    pub fn attach_layout(
        &mut self,
        rendered_lines: &[RenderedLineLayout],
        line_height: f32,
        paragraph_spacing: f32,
    ) {
        if self.lines.is_empty() {
            return;
        }

        let mut rendered_idx = 0;
        let mut current_y = 0.0;
        let mut prev_had_block = false;

        for line_idx in 0..self.lines.len() {
            let mut is_rendered = false;
            let mut line_y = current_y;
            let mut line_h = line_height;
            let mut role = SourceLineRole::Other;

            let line = &self.lines[line_idx];

            // Advance rendered_idx past segments that cannot overlap this line.
            while rendered_idx < rendered_lines.len()
                && rendered_lines[rendered_idx].source_range.end <= line.start
            {
                rendered_idx += 1;
            }

            if line.is_empty() {
                if let Some(run_pos) = self.empty_runs[line_idx] {
                    let has_next_block = rendered_idx < rendered_lines.len();

                    if prev_had_block && has_next_block && run_pos.index_in_run == 0 {
                        role = SourceLineRole::HiddenBlockSeparator;
                        line_h = paragraph_spacing;
                    } else {
                        role = SourceLineRole::EditableEmpty;
                        line_h = line_height;
                    }
                }
            } else {
                let first_rendered_idx = rendered_idx;
                while rendered_idx < rendered_lines.len()
                    && rendered_lines[rendered_idx].source_range.start < line.end
                    && rendered_lines[rendered_idx].source_range.end > line.start
                {
                    rendered_idx += 1;
                }

                if first_rendered_idx < rendered_idx {
                    let first = &rendered_lines[first_rendered_idx];
                    let last = &rendered_lines[rendered_idx - 1];
                    is_rendered = true;
                    prev_had_block = true;
                    line_y = first.y_top;
                    line_h = (last.y_top + last.height - line_y).max(first.height);
                    role = SourceLineRole::Paragraph; // Just a dummy for now unless we do full parsing
                }
            }

            self.lines[line_idx].role = role;
            self.lines[line_idx].y_top = line_y;
            self.lines[line_idx].height = line_h;

            if !is_rendered {
                current_y += line_h;
            } else {
                current_y = line_y + line_h;
            }
        }
    }

    pub fn extra_height_before_block(
        &self,
        block_start: usize,
        is_first_block: bool,
        line_height: f32,
        paragraph_spacing: f32,
    ) -> f32 {
        let Some(block_line) = self.line_at_byte(block_start) else { return 0.0 };
        let empty_before =
            (0..block_line.index).rev().take_while(|&idx| self.lines[idx].is_empty()).count();
        if is_first_block {
            empty_before as f32 * line_height
        } else {
            let editable_empty_lines = empty_before.saturating_sub(1);
            if editable_empty_lines == 0 {
                return 0.0;
            }
            editable_empty_lines as f32 * line_height + paragraph_spacing
        }
    }

    pub fn trailing_editable_height(&self) -> f32 {
        self.lines.iter().rev().take_while(|l| l.is_empty()).map(|l| l.height).sum()
    }

    pub fn previous_non_empty(&self, current_index: usize) -> Option<SourceLineEntry> {
        let start = current_index.checked_sub(1)?;
        (0..=start).rev().map(|i| self.lines[i]).find(|line| !line.is_empty())
    }

    pub fn next_non_empty(&self, current_index: usize) -> Option<SourceLineEntry> {
        self.lines.get(current_index + 1..)?.iter().copied().find(|line| !line.is_empty())
    }

    pub fn empty_lines_in_byte_range(
        &self,
        byte_range: Range<usize>,
    ) -> impl Iterator<Item = SourceLineEntry> + '_ {
        self.lines.iter().copied().filter(move |line| {
            line.is_empty() && line.start >= byte_range.start && line.start < byte_range.end
        })
    }

    pub(crate) fn projected_empty_lines(&self) -> impl Iterator<Item = ProjectedEmptyLine> + '_ {
        self.lines.iter().filter_map(|line| {
            (line.role == SourceLineRole::EditableEmpty).then_some(ProjectedEmptyLine {
                owner: ProjectionOwnerId::EmptyLine { source_byte: line.start },
                source_byte: line.start,
                y_top: line.y_top,
                height: line.height,
            })
        })
    }

    pub(crate) fn hidden_block_separators(
        &self,
    ) -> impl Iterator<Item = HiddenBlockSeparator> + '_ {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.role == SourceLineRole::HiddenBlockSeparator)
            .map(|(line_idx, line)| {
                let next_start =
                    self.lines.get(line_idx + 1).map_or(line.end, |next_line| next_line.start);
                HiddenBlockSeparator {
                    source_range: line.start..next_start,
                    previous_anchor: self.lines[line_idx - 1].end,
                }
            })
    }
}

fn collect_lines(source: &str) -> Vec<SourceLineSpan> {
    if source.is_empty() {
        return vec![SourceLineSpan { index: 0, start: 0, end: 0, is_blank: true }];
    }

    let mut lines = Vec::new();
    let mut line_start = 0usize;
    let bytes = source.as_bytes();

    for (byte_idx, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            let is_blank = source[line_start..byte_idx].chars().all(|c| c.is_whitespace());
            lines.push(SourceLineSpan {
                index: lines.len(),
                start: line_start,
                end: byte_idx,
                is_blank,
            });
            line_start = byte_idx + 1;
        }
    }

    let is_blank = source[line_start..source.len()].chars().all(|c| c.is_whitespace());
    lines.push(SourceLineSpan {
        index: lines.len(),
        start: line_start,
        end: source.len(),
        is_blank,
    });
    lines
}

fn collect_empty_runs(lines: &[SourceLineSpan]) -> Vec<Option<EmptyRunPosition>> {
    let mut runs = vec![None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].is_empty() {
            let mut j = i;
            while j < lines.len() && lines[j].is_empty() {
                j += 1;
            }
            let run_length = j - i;
            for k in i..j {
                runs[k] = Some(EmptyRunPosition { index_in_run: k - i, run_length });
            }
            i = j;
        } else {
            i += 1;
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_rendered_line_layout(
        range: std::ops::Range<usize>,
        y_top: f32,
        height: f32,
    ) -> Vec<RenderedLineLayout> {
        vec![RenderedLineLayout { source_range: range, y_top, height }]
    }

    fn two_rendered_line_layout() -> Vec<RenderedLineLayout> {
        vec![
            RenderedLineLayout { source_range: 0..1, y_top: 0.0, height: 24.0 },
            RenderedLineLayout { source_range: 5..6, y_top: 60.0, height: 24.0 },
        ]
    }

    #[test]
    fn trailing_empty_lines_are_all_editable_and_extend_content_height() {
        let source = "heading\n\n\n";
        let mut map = SourceLineMap::from_source(source);
        map.attach_layout(&single_rendered_line_layout(0..7, 0.0, 24.0), 24.0, 12.0);

        assert_eq!(map.line_at_index(1).expect("line 1").role, SourceLineRole::EditableEmpty);
        assert_eq!(map.line_at_index(2).expect("line 2").role, SourceLineRole::EditableEmpty);
        assert_eq!(map.line_at_index(3).expect("line 3").role, SourceLineRole::EditableEmpty);
        assert_eq!(map.trailing_editable_height(), 72.0);
    }

    #[test]
    fn inter_block_run_has_one_hidden_separator_then_editable_lines() {
        let source = "a\n\n\n\nb";
        let mut map = SourceLineMap::from_source(source);
        map.attach_layout(&two_rendered_line_layout(), 24.0, 12.0);

        assert_eq!(
            map.line_at_index(1).expect("separator").role,
            SourceLineRole::HiddenBlockSeparator
        );
        assert_eq!(map.line_at_index(2).expect("editable").role, SourceLineRole::EditableEmpty);
        assert_eq!(map.line_at_index(3).expect("editable").role, SourceLineRole::EditableEmpty);
    }

    #[test]
    fn source_line_height_includes_all_soft_wrapped_segments() {
        let mut map = SourceLineMap::from_source("abcdefghij\n\nnext");
        let rendered = vec![
            RenderedLineLayout { source_range: 0..4, y_top: 0.0, height: 24.0 },
            RenderedLineLayout { source_range: 4..8, y_top: 24.0, height: 24.0 },
            RenderedLineLayout { source_range: 8..10, y_top: 48.0, height: 24.0 },
            RenderedLineLayout { source_range: 12..16, y_top: 84.0, height: 24.0 },
        ];
        map.attach_layout(&rendered, 24.0, 12.0);

        assert_eq!(map.line_at_index(0).expect("wrapped line").height, 72.0);
        assert_eq!(map.line_at_index(1).expect("separator").y_top, 72.0);
    }

    #[test]
    fn adjacent_half_open_range_does_not_overlap_next_source_line() {
        let mut map = SourceLineMap::from_source("a\nb");
        let rendered = vec![
            RenderedLineLayout { source_range: 0..2, y_top: 0.0, height: 24.0 },
            RenderedLineLayout { source_range: 2..3, y_top: 24.0, height: 24.0 },
        ];
        map.attach_layout(&rendered, 24.0, 12.0);

        assert_eq!(map.line_at_index(1).expect("second line").y_top, 24.0);
    }
}
