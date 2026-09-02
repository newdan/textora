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
struct BlankLineLayout {
    role: SourceLineRole,
    y_top: f32,
    /// 可点击、可放置光标的行框高度。
    height: f32,
    /// 推进到下一源码行的距离；可编辑空段落还包含自身的出向块间距。
    advance: f32,
    is_rendered: bool,
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

    pub fn empty_run_position(&self, index: usize) -> Option<EmptyRunPosition> {
        self.empty_runs.get(index).copied().flatten()
    }

    /// 排布渲染行，为空行分类并赋予视觉坐标。
    ///
    /// 隐藏分隔行的高度取**真实块间 gap**（下一块首个渲染行顶边 − 前内容底边，
    /// 减去 run 内可编辑空段落的行高和出向间距），与
    /// `reserve_extra_blank_source_lines` 的追加公式
    /// `(N-1)*(line_height+paragraph_spacing)` 互为镜像。
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
            let mut line_advance = line_height;
            let mut role = SourceLineRole::Other;

            let line = &self.lines[line_idx];

            // Advance rendered_idx past segments that cannot overlap this line.
            // 空行自带的渲染投影（代码块/metadata 块内部空行，通常为零宽）锚定在
            // 行区间内，必须留给空行分类识别，不能在此跳过。
            while rendered_idx < rendered_lines.len() {
                let rendered = &rendered_lines[rendered_idx];
                if rendered.source_range.end > line.start
                    || Self::owns_rendered_line(rendered, line)
                {
                    break;
                }
                rendered_idx += 1;
            }

            if line.is_empty() {
                if let Some(run_pos) = self.empty_runs[line_idx] {
                    let blank = Self::classify_blank_line(
                        line,
                        run_pos,
                        rendered_lines.get(rendered_idx),
                        prev_had_block,
                        current_y,
                        line_height,
                        paragraph_spacing,
                    );
                    role = blank.role;
                    line_y = blank.y_top;
                    line_h = blank.height;
                    line_advance = blank.advance;
                    is_rendered = blank.is_rendered;
                    if blank.is_rendered {
                        prev_had_block = true;
                        rendered_idx += 1;
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
                    line_advance = line_h;
                    role = SourceLineRole::Paragraph; // 非空渲染行不做块类型细分，统一标记为 Paragraph
                }
            }

            self.lines[line_idx].role = role;
            self.lines[line_idx].y_top = line_y;
            self.lines[line_idx].height = line_h;

            if !is_rendered {
                current_y += line_advance;
            } else {
                current_y = line_y + line_h;
            }
        }
    }

    /// 空行自带的渲染投影：投影起点落在该空行的源码区间内
    ///（代码块/metadata 块会为内部空行生成渲染行，通常为零宽投影）。
    fn owns_rendered_line(rendered: &RenderedLineLayout, line: &SourceLineEntry) -> bool {
        line.is_empty()
            && rendered.source_range.start >= line.start
            && rendered.source_range.start <= line.end
    }

    /// 空行分类：块内部空行按渲染行处理；块间 run 首行折叠为隐藏分隔；
    /// 其余为可编辑空行。
    fn classify_blank_line(
        line: &SourceLineEntry,
        run_pos: EmptyRunPosition,
        next_rendered: Option<&RenderedLineLayout>,
        prev_had_block: bool,
        current_y: f32,
        line_height: f32,
        paragraph_spacing: f32,
    ) -> BlankLineLayout {
        if let Some(rendered) = next_rendered.filter(|r| Self::owns_rendered_line(r, line)) {
            return BlankLineLayout {
                role: SourceLineRole::Other,
                y_top: rendered.y_top,
                height: rendered.height,
                advance: rendered.height,
                is_rendered: true,
            };
        }

        if prev_had_block
            && let Some(next) = next_rendered
            && run_pos.index_in_run == 0
        {
            // 隐藏分隔行只消费前块到首个空段落的真实间距；每个可编辑
            // 空段落保留自己的行高和出向 paragraph spacing。
            let real_gap = next.y_top - current_y;
            let editable_in_run = run_pos.run_length.saturating_sub(1) as f32;
            let reserved_editable_extent = editable_in_run * (line_height + paragraph_spacing);
            let separator_height = (real_gap - reserved_editable_extent).max(0.0);
            return BlankLineLayout {
                role: SourceLineRole::HiddenBlockSeparator,
                y_top: current_y,
                height: separator_height,
                advance: separator_height,
                is_rendered: false,
            };
        }

        let is_inter_block_editable_line = prev_had_block && next_rendered.is_some();
        BlankLineLayout {
            role: SourceLineRole::EditableEmpty,
            y_top: current_y,
            height: line_height,
            advance: line_height
                + if is_inter_block_editable_line { paragraph_spacing } else { 0.0 },
            is_rendered: false,
        }
    }

    /// 块前应额外保留的空行高度：首块前的每个空行各占一行；非首块前的
    /// 空行 run 首行折叠为隐藏分隔（其高度由真实块间 gap 提供，见
    /// [`Self::attach_layout`]），其余每个可编辑空段落追加一行高和一份
    /// 出向段落间距。
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
            empty_before.saturating_sub(1) as f32 * (line_height + paragraph_spacing)
        }
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
            RenderedLineLayout { source_range: 5..6, y_top: 108.0, height: 24.0 },
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
        // 每个尾随空行各占一行高，合计延伸内容高度 3 * line_height。
        assert_eq!(map.line_at_index(1).expect("line 1").height, 24.0);
        assert_eq!(map.line_at_index(2).expect("line 2").height, 24.0);
        assert_eq!(map.line_at_index(3).expect("line 3").height, 24.0);
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
    fn inter_block_spacing_hides_only_the_required_separator_line() {
        let cases = [("a\n\nb", 0), ("a\n\n\nb", 1), ("a\n\n\n\nb", 2)];

        for (source, expected_editable_lines) in cases {
            let next_block_start = source.find('b').expect("fixture must contain a second block");
            let rendered_lines = vec![
                RenderedLineLayout { source_range: 0..1, y_top: 0.0, height: 24.0 },
                RenderedLineLayout {
                    source_range: next_block_start..next_block_start + 1,
                    y_top: 24.0 + 12.0 + expected_editable_lines as f32 * (24.0 + 12.0),
                    height: 24.0,
                },
            ];
            let mut map = SourceLineMap::from_source(source);
            map.attach_layout(&rendered_lines, 24.0, 12.0);

            let hidden_separators = map
                .lines()
                .iter()
                .filter(|line| line.role == SourceLineRole::HiddenBlockSeparator)
                .count();
            let editable_lines = map
                .lines()
                .iter()
                .filter(|line| line.role == SourceLineRole::EditableEmpty)
                .count();

            assert_eq!(hidden_separators, 1, "block separator mismatch for {source:?}");
            assert_eq!(
                editable_lines, expected_editable_lines,
                "editable empty-line mismatch for {source:?}"
            );
        }
    }

    #[test]
    fn blank_line_with_own_rendered_projection_is_not_hidden_separator() {
        // active 代码块/metadata 块会为内部空行生成专属渲染行（零宽投影），
        // 这类空行属于块内容，不得折叠为块间分隔。
        let source = "a\n\nb";
        let rendered = vec![
            RenderedLineLayout { source_range: 0..1, y_top: 0.0, height: 24.0 },
            RenderedLineLayout { source_range: 2..2, y_top: 24.0, height: 24.0 },
            RenderedLineLayout { source_range: 3..4, y_top: 48.0, height: 24.0 },
        ];
        let mut map = SourceLineMap::from_source(source);
        map.attach_layout(&rendered, 24.0, 12.0);

        let blank = map.line_at_index(1).expect("blank line");
        assert_ne!(blank.role, SourceLineRole::HiddenBlockSeparator);
        assert_eq!(blank.y_top, 24.0);
        assert_eq!(map.hidden_block_separators().count(), 0);
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
    fn hidden_separator_height_uses_real_inter_block_gap() {
        // 标题后跟 2 个空行：真实块间 gap = heading 底部间距(10.8)
        // + 一个可编辑空段落行高(24) + 其出向段落间距(12)。
        let source = "# T\n\n\npara";
        let line_height = 24.0;
        let heading_bottom = 24.0;
        let real_spacing = 10.8;
        let paragraph_spacing = 12.0;
        let para_top = heading_bottom + real_spacing + line_height + paragraph_spacing;
        let rendered = vec![
            RenderedLineLayout { source_range: 0..2, y_top: 0.0, height: 24.0 },
            RenderedLineLayout { source_range: 6..10, y_top: para_top, height: 24.0 },
        ];
        let mut map = SourceLineMap::from_source(source);
        map.attach_layout(&rendered, line_height, paragraph_spacing);

        let separator = map.line_at_index(1).expect("separator line");
        assert_eq!(separator.role, SourceLineRole::HiddenBlockSeparator);
        assert!(
            (separator.height - real_spacing).abs() < 0.001,
            "separator height {} must equal the real inter-block spacing {real_spacing}",
            separator.height
        );

        let editable = map.line_at_index(2).expect("editable line");
        assert_eq!(editable.role, SourceLineRole::EditableEmpty);
        assert!(
            (editable.y_top - (heading_bottom + real_spacing)).abs() < 0.001,
            "editable line y {} must start right below the real spacing",
            editable.y_top
        );
        assert!(
            (editable.y_top + editable.height + paragraph_spacing - para_top).abs() < 0.001,
            "editable line plus trailing spacing ends at {}, expected next block top {para_top}",
            editable.y_top + editable.height + paragraph_spacing
        );
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
