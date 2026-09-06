//! Source line boundaries and hidden separators; visual geometry belongs to layout rows.
use std::ops::Range;

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

pub type SourceLineEntry = SourceLineSpan;

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
        let lines = collect_lines(source);
        let empty_runs = collect_empty_runs(&lines);
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

    pub fn empty_run_position(&self, index: usize) -> Option<EmptyRunPosition> {
        self.empty_runs.get(index).copied().flatten()
    }

    /// 隐藏分隔只负责源码折叠；可编辑空段落的几何来自实际布局行。
    pub(crate) fn hidden_separators_for_rendered_lines(
        &self,
        source_ranges: &[Range<usize>],
    ) -> Vec<HiddenBlockSeparator> {
        let mut separators = Vec::new();
        for (index, line) in self.lines.iter().enumerate().skip(1) {
            let Some(run) = self.empty_runs[index] else { continue };
            if run.index_in_run != 0 || self.lines[index - 1].is_blank {
                continue;
            }
            let next_range = source_ranges.partition_point(|range| range.start < line.start);
            let has_own_layout =
                source_ranges.get(next_range).is_some_and(|range| range.start <= line.end);
            if has_own_layout {
                continue;
            }
            let next_start = self.lines.get(index + 1).map_or(line.end, |next| next.start);
            separators.push(HiddenBlockSeparator {
                source_range: line.start..next_start,
                previous_anchor: self.lines[index - 1].end,
            });
        }
        separators
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

    #[test]
    fn source_line_map_preserves_lf_and_crlf_byte_anchors() {
        for newline in ["\n", "\r\n"] {
            let source = format!("前{newline}{newline}{newline}后");
            let map = SourceLineMap::from_source(&source);
            let anchor = "前".len() + newline.len() * 2;
            let line = map.line_at_byte(anchor).expect("second blank line must exist");
            assert_eq!(line.start, anchor);
            assert_eq!(
                map.empty_run_position(line.index),
                Some(EmptyRunPosition { index_in_run: 1, run_length: 2 })
            );
        }
    }

    #[test]
    fn explicitly_laid_out_empty_content_is_not_a_hidden_separator() {
        let map = SourceLineMap::from_source("a\n\nb");
        assert!(map.hidden_separators_for_rendered_lines(&[0..1, 2..2, 3..4]).is_empty());
    }

    #[test]
    fn hidden_separator_folds_to_preceding_source_boundary() {
        let map = SourceLineMap::from_source("a\n\n\nb");
        assert_eq!(
            map.hidden_separators_for_rendered_lines(&[0..1, 3..3, 4..5]),
            vec![HiddenBlockSeparator { source_range: 2..3, previous_anchor: 1 }]
        );
    }
}
