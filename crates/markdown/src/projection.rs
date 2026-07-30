use std::collections::BTreeMap;
use std::ops::Range;

use crate::layout::types::VisualLineProjection;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CursorAffinity {
    Upstream,
    Downstream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ProjectionOwnerId {
    Block { block_start: usize, logical_line: usize },
    TableCell { table_start: usize, row: usize, column: usize, logical_line: usize },
    EmptyLine { source_byte: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceAnchor {
    pub byte: usize,
    pub affinity: CursorAffinity,
    virtual_grapheme_ordinal: Option<usize>,
}

impl SourceAnchor {
    pub(crate) const fn upstream(byte: usize) -> Self {
        Self { byte, affinity: CursorAffinity::Upstream, virtual_grapheme_ordinal: None }
    }

    pub(crate) const fn downstream(byte: usize) -> Self {
        Self { byte, affinity: CursorAffinity::Downstream, virtual_grapheme_ordinal: None }
    }

    const fn virtual_boundary(byte: usize, virtual_grapheme_ordinal: usize) -> Self {
        Self {
            byte,
            affinity: CursorAffinity::Downstream,
            virtual_grapheme_ordinal: Some(virtual_grapheme_ordinal),
        }
    }

    const fn with_affinity(self, affinity: CursorAffinity) -> Self {
        Self { affinity, ..self }
    }

    const fn without_virtual_ordinal(self) -> Self {
        Self { virtual_grapheme_ordinal: None, ..self }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionSpanKind {
    Direct,
    Collapsed,
    Virtual { anchor_byte: usize, virtual_grapheme_start: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionSpan {
    pub source_range: Range<usize>,
    pub visual_range: Range<usize>,
    pub kind: ProjectionSpanKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectedText {
    pub text: String,
    pub spans: Vec<ProjectionSpan>,
    pub boundaries: Vec<SourceAnchor>,
}

#[derive(Default)]
pub(crate) struct TextProjectionBuilder {
    text: String,
    spans: Vec<ProjectionSpan>,
    char_anchors: Vec<SourceAnchor>,
    pending_gap_start: Option<usize>,
    truncated_terminal_anchor: Option<SourceAnchor>,
}

impl TextProjectionBuilder {
    pub(crate) fn push_direct(&mut self, text: &str, source_range: Range<usize>) {
        self.truncated_terminal_anchor = None;
        if let Some(gap_start) = self.pending_gap_start.take() {
            let visual_start = self.text.len();
            self.text.push(' ');
            self.char_anchors.push(SourceAnchor::upstream(gap_start));
            self.spans.push(ProjectionSpan {
                source_range: gap_start..source_range.start,
                visual_range: visual_start..self.text.len(),
                kind: ProjectionSpanKind::Collapsed,
            });
        }
        let visual_start = self.text.len();
        self.text.push_str(text);
        self.char_anchors.extend(
            text.char_indices()
                .map(|(offset, _)| SourceAnchor::downstream(source_range.start + offset)),
        );
        self.spans.push(ProjectionSpan {
            source_range,
            visual_range: visual_start..self.text.len(),
            kind: ProjectionSpanKind::Direct,
        });
    }

    pub(crate) fn push_soft_break(&mut self, event_range: Range<usize>) {
        self.pending_gap_start = Some(event_range.start);
    }

    pub(crate) fn trim_trailing_newline(&mut self) {
        let (newline_start, newline) = self
            .text
            .char_indices()
            .last()
            .expect("a projection trimmed with a newline must contain a final character");
        assert_eq!(newline, '\n', "projection trim must match the pending text newline");

        let anchor =
            self.char_anchors.pop().expect("a projection character must have a source anchor");
        let span =
            self.spans.last_mut().expect("a projection character must belong to a source span");
        assert!(matches!(span.kind, ProjectionSpanKind::Direct));
        assert_eq!(span.visual_range.end, self.text.len());

        self.text.truncate(newline_start);
        span.visual_range.end = newline_start;
        span.source_range.end = anchor.byte;
        self.truncated_terminal_anchor = Some(anchor);
    }

    pub(crate) fn finish(mut self, source_end: usize) -> ProjectedText {
        let last_span_source_end = self.spans.last().map(|span| span.source_range.end);
        let terminal_anchor = self.truncated_terminal_anchor.take().unwrap_or_else(|| {
            SourceAnchor::downstream(last_span_source_end.unwrap_or(source_end))
        });
        self.char_anchors.push(terminal_anchor);
        let boundaries = grapheme_boundary_anchors(&self.text, &self.char_anchors);
        ProjectedText { text: self.text, spans: self.spans, boundaries }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionError {
    BoundaryCountMismatch { expected: usize, actual: usize },
    InvalidSourceBoundary { byte: usize },
    NonMonotonicSourceOrder { previous: usize, current: usize },
    UnclassifiedDuplicateBoundary { byte: usize },
    StaleGeneration { expected: u32, actual: u32 },
    StaleLayoutRevision { expected: u64, actual: u64 },
    MissingEditableProjection { flat_line_idx: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VisualPosition {
    pub layout_revision: u64,
    pub flat_line_idx: usize,
    pub grapheme_pos: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceVisualAnchor {
    pub source: SourceAnchor,
    pub visual: VisualPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HorizontalDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineBoundary {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollapsedSourceRange {
    pub source_range: Range<usize>,
    pub upstream: VisualPosition,
    pub downstream: VisualPosition,
}

fn collect_virtual_boundaries(visual_order: &[SourceVisualAnchor]) -> Vec<SourceVisualAnchor> {
    let mut canonical_boundaries = BTreeMap::new();
    for &entry in visual_order {
        let Some(virtual_grapheme_ordinal) = entry.source.virtual_grapheme_ordinal else {
            continue;
        };
        canonical_boundaries.entry((entry.source.byte, virtual_grapheme_ordinal)).or_insert(entry);
    }
    canonical_boundaries.into_values().collect()
}

#[derive(Clone, Debug)]
pub(crate) struct SourceProjectionIndex {
    source_generation: u32,
    layout_revision: u64,
    visual_lines: Vec<VisualLineProjection>,
    reverse: Vec<SourceVisualAnchor>,
    virtual_boundaries: Vec<SourceVisualAnchor>,
    collapsed: Vec<CollapsedSourceRange>,
}

impl SourceProjectionIndex {
    fn canonical_reverse_anchor(
        visual_lines: &[VisualLineProjection],
        line_idx: usize,
        grapheme_pos: usize,
        source: SourceAnchor,
    ) -> SourceAnchor {
        if Self::is_source_progressing_wrap_boundary(visual_lines, line_idx, grapheme_pos, source) {
            return source.with_affinity(CursorAffinity::Upstream);
        }

        source
    }

    fn is_source_progressing_wrap_boundary(
        visual_lines: &[VisualLineProjection],
        line_idx: usize,
        grapheme_pos: usize,
        source: SourceAnchor,
    ) -> bool {
        if source.affinity != CursorAffinity::Downstream {
            return false;
        }

        let Some(line) = visual_lines.get(line_idx) else {
            return false;
        };
        let Some(next_line) = visual_lines.get(line_idx + 1) else {
            return false;
        };
        let Some(previous_boundary) =
            grapheme_pos.checked_sub(1).and_then(|position| line.boundaries.get(position))
        else {
            return false;
        };
        let Some(next_boundary) = next_line.boundaries.get(1) else {
            return false;
        };

        grapheme_pos + 1 == line.boundaries.len()
            && line.flat_line_idx.checked_add(1) == Some(next_line.flat_line_idx)
            && next_line.boundaries.first() == Some(&source)
            && line.source_extent.end == source.byte
            && next_line.source_extent.start == source.byte
            && previous_boundary.byte < source.byte
            && source.byte < next_boundary.byte
    }

    pub(crate) fn build(
        source_generation: u32,
        layout_revision: u64,
        mut visual_lines: Vec<VisualLineProjection>,
    ) -> Result<Self, ProjectionError> {
        visual_lines.sort_by_key(|line| line.flat_line_idx);
        let mut reverse = Vec::new();
        let mut visual_order = Vec::new();
        let mut collapsed = Vec::new();
        let mut previous_source_byte = None;

        for (line_idx, line) in visual_lines.iter().enumerate() {
            if line.boundaries.is_empty() {
                return Err(ProjectionError::MissingEditableProjection {
                    flat_line_idx: line.flat_line_idx,
                });
            }

            for (grapheme_pos, &source) in line.boundaries.iter().enumerate() {
                if let Some(previous) = previous_source_byte
                    && source.byte < previous
                {
                    return Err(ProjectionError::NonMonotonicSourceOrder {
                        previous,
                        current: source.byte,
                    });
                }
                previous_source_byte = Some(source.byte);
                let source =
                    Self::canonical_reverse_anchor(&visual_lines, line_idx, grapheme_pos, source);
                let anchor = SourceVisualAnchor {
                    source,
                    visual: VisualPosition {
                        layout_revision,
                        flat_line_idx: line.flat_line_idx,
                        grapheme_pos,
                    },
                };
                reverse.push(SourceVisualAnchor {
                    source: anchor.source.without_virtual_ordinal(),
                    visual: anchor.visual,
                });
                visual_order.push(anchor);
            }

            for collapsed_boundary in &line.collapsed {
                collapsed.push(CollapsedSourceRange {
                    source_range: collapsed_boundary.source_range.clone(),
                    upstream: VisualPosition {
                        layout_revision,
                        flat_line_idx: line.flat_line_idx,
                        grapheme_pos: collapsed_boundary.upstream_grapheme,
                    },
                    downstream: VisualPosition {
                        layout_revision,
                        flat_line_idx: line.flat_line_idx,
                        grapheme_pos: collapsed_boundary.downstream_grapheme,
                    },
                });
            }
        }

        reverse.sort_by_key(|entry| {
            (
                entry.source.byte,
                entry.source.affinity,
                entry.visual.flat_line_idx,
                entry.visual.grapheme_pos,
            )
        });
        let virtual_boundaries = collect_virtual_boundaries(&visual_order);
        reverse.dedup_by_key(|entry| entry.source);

        Ok(Self {
            source_generation,
            layout_revision,
            visual_lines,
            reverse,
            virtual_boundaries,
            collapsed,
        })
    }

    pub(crate) fn source_anchor_at(
        &self,
        source_generation: u32,
        position: VisualPosition,
    ) -> Result<SourceAnchor, ProjectionError> {
        if source_generation != self.source_generation {
            return Err(ProjectionError::StaleGeneration {
                expected: self.source_generation,
                actual: source_generation,
            });
        }
        if position.layout_revision != self.layout_revision {
            return Err(ProjectionError::StaleLayoutRevision {
                expected: self.layout_revision,
                actual: position.layout_revision,
            });
        }

        let line = self
            .visual_lines
            .binary_search_by_key(&position.flat_line_idx, |line| line.flat_line_idx)
            .ok()
            .and_then(|line_idx| self.visual_lines.get(line_idx))
            .ok_or(ProjectionError::MissingEditableProjection {
                flat_line_idx: position.flat_line_idx,
            })?;
        line.boundaries.get(position.grapheme_pos).copied().ok_or(
            ProjectionError::MissingEditableProjection { flat_line_idx: position.flat_line_idx },
        )
    }

    pub(crate) fn visual_position_for_source(
        &self,
        source_byte: usize,
        affinity: CursorAffinity,
    ) -> Option<VisualPosition> {
        let range_start = self.reverse.partition_point(|entry| entry.source.byte < source_byte);
        let range_end = self.reverse.partition_point(|entry| entry.source.byte <= source_byte);
        if let Some(matching) =
            self.reverse.get(range_start..range_end).filter(|matching| !matching.is_empty())
        {
            return matching
                .iter()
                .find(|entry| entry.source.affinity == affinity)
                .or_else(|| match affinity {
                    CursorAffinity::Upstream => matching.first(),
                    CursorAffinity::Downstream => matching.last(),
                })
                .map(|entry| entry.visual);
        }

        self.collapsed
            .iter()
            .find(|collapsed| {
                collapsed.source_range.contains(&source_byte)
                    || collapsed.source_range.end == source_byte
            })
            .map(|collapsed| match affinity {
                CursorAffinity::Upstream => collapsed.upstream,
                CursorAffinity::Downstream => collapsed.downstream,
            })
    }

    pub(crate) fn virtual_position_for_source(
        &self,
        source_byte: usize,
        virtual_grapheme: usize,
    ) -> Option<VisualPosition> {
        self.virtual_boundaries
            .iter()
            .find(|entry| {
                entry.source.byte == source_byte
                    && entry.source.virtual_grapheme_ordinal == Some(virtual_grapheme)
            })
            .map(|entry| entry.visual)
    }

    pub(crate) fn move_horizontal(
        &self,
        current_byte: usize,
        direction: HorizontalDirection,
    ) -> Option<SourceAnchor> {
        let current = self.visual_position_for_source(current_byte, CursorAffinity::Downstream)?;
        let line_idx = self
            .visual_lines
            .binary_search_by_key(&current.flat_line_idx, |line| line.flat_line_idx)
            .ok()?;

        match direction {
            HorizontalDirection::Previous => {
                self.previous_distinct_boundary(line_idx, current.grapheme_pos, current_byte)
            }
            HorizontalDirection::Next => {
                self.next_distinct_boundary(line_idx, current.grapheme_pos, current_byte)
            }
        }
    }

    pub(crate) fn line_boundary(
        &self,
        current_byte: usize,
        boundary: LineBoundary,
    ) -> Option<SourceAnchor> {
        let position = self.visual_position_for_source(current_byte, CursorAffinity::Downstream)?;
        let line = self
            .visual_lines
            .binary_search_by_key(&position.flat_line_idx, |line| line.flat_line_idx)
            .ok()
            .and_then(|line_idx| self.visual_lines.get(line_idx))?;
        match boundary {
            LineBoundary::Start => line.boundaries.first().copied(),
            LineBoundary::End => line.boundaries.last().copied(),
        }
    }

    fn previous_distinct_boundary(
        &self,
        current_line_idx: usize,
        current_grapheme: usize,
        current_byte: usize,
    ) -> Option<SourceAnchor> {
        let current_line = self.visual_lines.get(current_line_idx)?;
        if let Some(anchor) = current_line.boundaries[..current_grapheme]
            .iter()
            .rev()
            .find(|anchor| anchor.byte != current_byte)
        {
            return Some(*anchor);
        }

        self.visual_lines[..current_line_idx].iter().rev().find_map(|line| {
            line.boundaries.iter().rev().find(|anchor| anchor.byte != current_byte).copied()
        })
    }

    fn next_distinct_boundary(
        &self,
        current_line_idx: usize,
        current_grapheme: usize,
        current_byte: usize,
    ) -> Option<SourceAnchor> {
        let current_line = self.visual_lines.get(current_line_idx)?;
        if let Some(remaining_boundaries) =
            current_line.boundaries.get(current_grapheme.saturating_add(1)..)
            && let Some(anchor) =
                remaining_boundaries.iter().find(|anchor| anchor.byte != current_byte)
        {
            return Some(*anchor);
        }

        self.visual_lines[current_line_idx.saturating_add(1)..].iter().find_map(|line| {
            line.boundaries.iter().find(|anchor| anchor.byte != current_byte).copied()
        })
    }

    pub(crate) fn visual_lines(&self) -> &[VisualLineProjection] {
        &self.visual_lines
    }

    pub(crate) const fn layout_revision(&self) -> u64 {
        self.layout_revision
    }
}

impl ProjectedText {
    pub(crate) fn direct(text: &str, source_start: usize) -> Self {
        let mut boundaries = text
            .char_indices()
            .map(|(relative_byte, _)| SourceAnchor::downstream(source_start + relative_byte))
            .collect::<Vec<_>>();
        boundaries.push(SourceAnchor::downstream(source_start + text.len()));
        let boundaries = grapheme_boundary_anchors(text, &boundaries);

        Self {
            text: text.to_string(),
            spans: vec![ProjectionSpan {
                source_range: source_start..source_start + text.len(),
                visual_range: 0..text.len(),
                kind: ProjectionSpanKind::Direct,
            }],
            boundaries,
        }
    }

    pub(crate) fn grapheme_count(&self) -> usize {
        crate::grapheme_map::grapheme_count(&self.text)
    }

    /// Returns the full source extent represented by this visual text,
    /// including collapsed Markdown syntax between direct text spans.
    pub(crate) fn source_extent(&self) -> Range<usize> {
        let boundary_start =
            self.boundaries.first().expect("a projected text always has a sentinel boundary").byte;
        let boundary_end =
            self.boundaries.last().expect("a projected text always has a sentinel boundary").byte;
        self.spans.iter().fold(boundary_start..boundary_end, |extent, span| {
            extent.start.min(span.source_range.start)..extent.end.max(span.source_range.end)
        })
    }

    /// Splits parser-collapsed softbreaks back into physical source lines.
    ///
    /// Each collapsed span is retained as a zero-width boundary at the end of
    /// the preceding line, so cursor navigation still observes the newline and
    /// continuation indentation it represents.
    pub(crate) fn split_collapsed_source_lines(&self) -> Vec<(Range<usize>, Self)> {
        let mut physical_lines = Vec::new();
        let mut visual_start = 0usize;

        for span in
            self.spans.iter().filter(|span| matches!(span.kind, ProjectionSpanKind::Collapsed))
        {
            let visual_end = span.visual_range.start;
            let mut line = self.slice_visual_text(visual_start..visual_end);
            line.spans.push(ProjectionSpan {
                source_range: span.source_range.clone(),
                visual_range: line.text.len()..line.text.len(),
                kind: ProjectionSpanKind::Collapsed,
            });
            physical_lines.push((visual_start..visual_end, line));
            visual_start = span.visual_range.end;
        }

        physical_lines.push((
            visual_start..self.text.len(),
            self.slice_visual_text(visual_start..self.text.len()),
        ));
        physical_lines
    }

    fn slice_visual_text(&self, visual_range: Range<usize>) -> Self {
        assert!(
            visual_range.start <= visual_range.end
                && visual_range.end <= self.text.len()
                && self.text.is_char_boundary(visual_range.start)
                && self.text.is_char_boundary(visual_range.end),
            "projected text slices must begin and end at visual character boundaries"
        );

        let start_grapheme =
            crate::grapheme_map::grapheme_index_at_byte(&self.text, visual_range.start);
        let end_grapheme =
            crate::grapheme_map::grapheme_index_at_byte(&self.text, visual_range.end);
        let spans = self
            .spans
            .iter()
            .filter_map(|span| {
                let clipped_start = span.visual_range.start.max(visual_range.start);
                let clipped_end = span.visual_range.end.min(visual_range.end);
                if clipped_start >= clipped_end {
                    return None;
                }

                let source_range = match span.kind {
                    ProjectionSpanKind::Direct => {
                        let start =
                            crate::grapheme_map::grapheme_index_at_byte(&self.text, clipped_start);
                        let end =
                            crate::grapheme_map::grapheme_index_at_byte(&self.text, clipped_end);
                        self.boundaries[start].byte..self.boundaries[end].byte
                    }
                    ProjectionSpanKind::Collapsed | ProjectionSpanKind::Virtual { .. } => {
                        span.source_range.clone()
                    }
                };
                Some(ProjectionSpan {
                    source_range,
                    visual_range: clipped_start - visual_range.start
                        ..clipped_end - visual_range.start,
                    kind: span.kind.clone(),
                })
            })
            .collect();

        Self {
            text: self.text[visual_range.clone()].to_string(),
            spans,
            boundaries: self.boundaries[start_grapheme..=end_grapheme].to_vec(),
        }
    }

    pub(crate) fn prepend_direct(mut self, text: &str, source_range: Range<usize>) -> Self {
        if text.is_empty() {
            return self;
        }

        let direct = Self::direct_with_source_range(text, source_range);
        let visual_offset = direct.text.len();
        for span in &mut self.spans {
            span.visual_range.start += visual_offset;
            span.visual_range.end += visual_offset;
        }

        let mut spans = direct.spans;
        spans.extend(self.spans);

        let mut boundaries = direct.boundaries;
        boundaries.pop();
        boundaries.extend(self.boundaries);

        Self { text: format!("{}{}", direct.text, self.text), spans, boundaries }
    }

    pub(crate) fn insert_virtual(
        mut self,
        grapheme_index: usize,
        text: &str,
        anchor_byte: usize,
    ) -> Self {
        if text.is_empty() {
            return self;
        }

        let grapheme_index = grapheme_index.min(self.grapheme_count());
        let visual_byte = crate::grapheme_map::byte_at_grapheme_index(&self.text, grapheme_index);
        let virtual_grapheme_count = crate::grapheme_map::grapheme_count(text);
        let virtual_grapheme_start = self.next_virtual_grapheme_ordinal();

        self.text.insert_str(visual_byte, text);
        self.boundaries.splice(
            grapheme_index..=grapheme_index,
            (0..=virtual_grapheme_count).map(|offset| {
                SourceAnchor::virtual_boundary(anchor_byte, virtual_grapheme_start + offset)
            }),
        );
        insert_virtual_span(
            &mut self.spans,
            visual_byte,
            text.len(),
            anchor_byte,
            virtual_grapheme_start,
        );
        self
    }

    fn next_virtual_grapheme_ordinal(&self) -> usize {
        self.spans
            .iter()
            .filter_map(|span| {
                let ProjectionSpanKind::Virtual { virtual_grapheme_start, .. } = span.kind else {
                    return None;
                };
                let span_text = self.text.get(span.visual_range.clone())?;
                Some(virtual_grapheme_start + crate::grapheme_map::grapheme_count(span_text) + 1)
            })
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn replace_graphemes_with_direct(
        mut self,
        start_grapheme: usize,
        end_grapheme: usize,
        text: &str,
        source_range: Range<usize>,
    ) -> Self {
        let grapheme_count = self.grapheme_count();
        let start_grapheme = start_grapheme.min(grapheme_count);
        let end_grapheme = end_grapheme.clamp(start_grapheme, grapheme_count);
        let visual_start = crate::grapheme_map::byte_at_grapheme_index(&self.text, start_grapheme);
        let visual_end = crate::grapheme_map::byte_at_grapheme_index(&self.text, end_grapheme);
        let replaced_source_range =
            self.boundaries[start_grapheme].byte..self.boundaries[end_grapheme].byte;
        let direct = Self::direct_with_source_range(text, source_range);

        self.text.replace_range(visual_start..visual_end, text);
        self.boundaries.splice(start_grapheme..=end_grapheme, direct.boundaries.iter().copied());
        replace_spans_with_direct(
            &mut self.spans,
            visual_start..visual_end,
            replaced_source_range,
            direct.spans.into_iter().next().expect("direct text has one projection span"),
        );
        self
    }

    pub(crate) fn from_char_anchors(
        text: String,
        char_anchors: Vec<SourceAnchor>,
        spans: Vec<ProjectionSpan>,
    ) -> Self {
        let boundaries = grapheme_boundary_anchors(&text, &char_anchors);
        Self { text, spans, boundaries }
    }

    pub(crate) fn validate(&self, source: &str) -> Result<(), ProjectionError> {
        let expected = self.grapheme_count() + 1;
        if self.boundaries.len() != expected {
            return Err(ProjectionError::BoundaryCountMismatch {
                expected,
                actual: self.boundaries.len(),
            });
        }

        let mut previous = None;
        for anchor in &self.boundaries {
            if anchor.byte > source.len() || !source.is_char_boundary(anchor.byte) {
                return Err(ProjectionError::InvalidSourceBoundary { byte: anchor.byte });
            }
            if let Some(previous_byte) = previous
                && anchor.byte < previous_byte
            {
                return Err(ProjectionError::NonMonotonicSourceOrder {
                    previous: previous_byte,
                    current: anchor.byte,
                });
            }
            previous = Some(anchor.byte);
        }

        Ok(())
    }

    fn direct_with_source_range(text: &str, source_range: Range<usize>) -> Self {
        let mut char_anchors = text
            .char_indices()
            .map(|(relative_byte, _)| SourceAnchor::downstream(source_range.start + relative_byte))
            .collect::<Vec<_>>();
        char_anchors.push(SourceAnchor::downstream(source_range.end));

        Self {
            text: text.to_string(),
            spans: vec![ProjectionSpan {
                source_range,
                visual_range: 0..text.len(),
                kind: ProjectionSpanKind::Direct,
            }],
            boundaries: grapheme_boundary_anchors(text, &char_anchors),
        }
    }
}

fn insert_virtual_span(
    spans: &mut Vec<ProjectionSpan>,
    visual_byte: usize,
    inserted_len: usize,
    anchor_byte: usize,
    virtual_grapheme_start: usize,
) {
    let mut transformed = Vec::with_capacity(spans.len() + 2);
    for mut span in spans.drain(..) {
        if span.visual_range.start >= visual_byte {
            span.visual_range.start += inserted_len;
            span.visual_range.end += inserted_len;
            transformed.push(span);
            continue;
        }
        if span.visual_range.end <= visual_byte {
            transformed.push(span);
            continue;
        }

        let split_source_byte = anchor_byte.clamp(span.source_range.start, span.source_range.end);
        transformed.push(ProjectionSpan {
            source_range: span.source_range.start..split_source_byte,
            visual_range: span.visual_range.start..visual_byte,
            kind: span.kind.clone(),
        });
        transformed.push(ProjectionSpan {
            source_range: split_source_byte..span.source_range.end,
            visual_range: visual_byte + inserted_len..span.visual_range.end + inserted_len,
            kind: span.kind,
        });
    }
    transformed.push(ProjectionSpan {
        source_range: anchor_byte..anchor_byte,
        visual_range: visual_byte..visual_byte + inserted_len,
        kind: ProjectionSpanKind::Virtual { anchor_byte, virtual_grapheme_start },
    });
    transformed.sort_by_key(|span| span.visual_range.start);
    *spans = transformed;
}

fn replace_spans_with_direct(
    spans: &mut Vec<ProjectionSpan>,
    replaced_visual_range: Range<usize>,
    replaced_source_range: Range<usize>,
    mut direct_span: ProjectionSpan,
) {
    let replacement_len = direct_span.visual_range.len();
    let replaced_len = replaced_visual_range.len();
    let visual_delta = replacement_len as isize - replaced_len as isize;
    let mut transformed = Vec::with_capacity(spans.len() + 1);

    for mut span in spans.drain(..) {
        if span.visual_range.end <= replaced_visual_range.start {
            transformed.push(span);
            continue;
        }
        if span.visual_range.start >= replaced_visual_range.end {
            span.visual_range.start = shift_visual_byte(span.visual_range.start, visual_delta);
            span.visual_range.end = shift_visual_byte(span.visual_range.end, visual_delta);
            transformed.push(span);
            continue;
        }
        if span.visual_range.start < replaced_visual_range.start {
            transformed.push(ProjectionSpan {
                source_range: span.source_range.start..replaced_source_range.start,
                visual_range: span.visual_range.start..replaced_visual_range.start,
                kind: span.kind.clone(),
            });
        }
        if span.visual_range.end > replaced_visual_range.end {
            transformed.push(ProjectionSpan {
                source_range: replaced_source_range.end..span.source_range.end,
                visual_range: replaced_visual_range.start + replacement_len
                    ..shift_visual_byte(span.visual_range.end, visual_delta),
                kind: span.kind,
            });
        }
    }

    direct_span.visual_range =
        replaced_visual_range.start..replaced_visual_range.start + replacement_len;
    transformed.push(direct_span);
    transformed.sort_by_key(|span| span.visual_range.start);
    *spans = transformed;
}

fn shift_visual_byte(visual_byte: usize, delta: isize) -> usize {
    visual_byte
        .checked_add_signed(delta)
        .expect("projection visual ranges must remain non-negative")
}

fn grapheme_boundary_anchors(text: &str, char_anchors: &[SourceAnchor]) -> Vec<SourceAnchor> {
    let char_count = text.chars().count();
    assert_eq!(char_anchors.len(), char_count + 1, "char anchors must include one sentinel");
    let char_ordinals = (0..=char_count).collect::<Vec<_>>();
    crate::grapheme_map::build_visual_grapheme_map(text, &char_ordinals)
        .as_slice()
        .iter()
        .map(|&ordinal| char_anchors[ordinal])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::types::VisualLineProjection;

    fn shared_boundary_fixture() -> Vec<VisualLineProjection> {
        vec![
            VisualLineProjection {
                flat_line_idx: 0,
                owner: ProjectionOwnerId::Block { block_start: 0, logical_line: 0 },
                boundaries: (0..5)
                    .map(SourceAnchor::downstream)
                    .chain(std::iter::once(SourceAnchor::upstream(5)))
                    .collect(),
                source_extent: 0..5,
                collapsed: Vec::new(),
            },
            VisualLineProjection {
                flat_line_idx: 1,
                owner: ProjectionOwnerId::Block { block_start: 0, logical_line: 0 },
                boundaries: (5..=9).map(SourceAnchor::downstream).collect(),
                source_extent: 5..9,
                collapsed: Vec::new(),
            },
        ]
    }

    #[test]
    fn reverse_index_uses_requested_affinity_at_shared_wrap_boundary() {
        let lines = shared_boundary_fixture();
        let index = SourceProjectionIndex::build(7, 3, lines).expect("fixture is valid");
        assert_eq!(
            index.visual_position_for_source(5, CursorAffinity::Upstream),
            Some(VisualPosition { layout_revision: 3, flat_line_idx: 0, grapheme_pos: 5 })
        );
        assert_eq!(
            index.visual_position_for_source(5, CursorAffinity::Downstream),
            Some(VisualPosition { layout_revision: 3, flat_line_idx: 1, grapheme_pos: 0 })
        );
    }

    #[test]
    fn index_rejects_stale_generation_queries() {
        let index = SourceProjectionIndex::build(7, 3, shared_boundary_fixture())
            .expect("fixture is valid");
        assert_eq!(
            index.source_anchor_at(
                8,
                VisualPosition { layout_revision: 3, flat_line_idx: 0, grapheme_pos: 0 },
            ),
            Err(ProjectionError::StaleGeneration { expected: 7, actual: 8 })
        );
    }

    #[test]
    fn index_reads_anchor_from_matching_flat_line_index() {
        let index = SourceProjectionIndex::build(
            7,
            3,
            vec![VisualLineProjection {
                flat_line_idx: 4,
                owner: ProjectionOwnerId::Block { block_start: 9, logical_line: 0 },
                boundaries: vec![SourceAnchor::downstream(9)],
                source_extent: 9..9,
                collapsed: Vec::new(),
            }],
        )
        .expect("fixture is valid");

        assert_eq!(
            index.source_anchor_at(
                7,
                VisualPosition { layout_revision: 3, flat_line_idx: 4, grapheme_pos: 0 },
            ),
            Ok(SourceAnchor::downstream(9))
        );
    }

    #[test]
    fn index_classifies_virtual_preedit_boundaries_for_navigation() {
        let projected = ProjectedText::direct("ab", 10).insert_virtual(1, "中文", 11);
        let virtual_line = projected
            .slice_visual_line(0, 0..projected.text.len())
            .expect("the virtual projection must slice at its visual boundaries");

        assert_eq!(
            virtual_line.owner,
            ProjectionOwnerId::Block { block_start: 10, logical_line: 0 }
        );

        let index = SourceProjectionIndex::build(7, 3, vec![virtual_line])
            .expect("virtual preedit is valid");

        assert_eq!(
            index.move_horizontal(11, HorizontalDirection::Previous),
            Some(SourceAnchor::downstream(10))
        );
        assert_eq!(
            index.move_horizontal(11, HorizontalDirection::Next),
            Some(SourceAnchor::downstream(12))
        );
        assert_eq!(
            index.line_boundary(11, LineBoundary::Start),
            Some(SourceAnchor::downstream(10))
        );
        assert_eq!(index.line_boundary(11, LineBoundary::End), Some(SourceAnchor::downstream(12)));
        assert_eq!(
            index.virtual_position_for_source(11, 2),
            Some(VisualPosition { layout_revision: 3, flat_line_idx: 0, grapheme_pos: 3 })
        );
    }

    #[test]
    fn virtual_preedit_cursor_uses_second_grapheme_after_soft_wrap() {
        let projected = ProjectedText::direct("ab", 10).insert_virtual(1, "中文", 11);
        let first_line_end = "a中".len();
        let first_line = projected
            .slice_visual_line(0, 0..first_line_end)
            .expect("the first wrapped line must end at a visual boundary");
        let second_line = projected
            .slice_visual_line(1, first_line_end..projected.text.len())
            .expect("the second wrapped line must start at a visual boundary");

        let index = SourceProjectionIndex::build(7, 3, vec![first_line, second_line])
            .expect("wrapped virtual preedit is valid");

        assert_eq!(
            index.virtual_position_for_source(11, 2),
            Some(VisualPosition { layout_revision: 3, flat_line_idx: 1, grapheme_pos: 1 }),
            "the second preedit grapheme cursor must be at the visual end, not the next line start"
        );
    }

    #[test]
    fn index_rejects_position_from_same_generation_earlier_layout_revision() {
        let previous = SourceProjectionIndex::build(7, 3, shared_boundary_fixture())
            .expect("fixture is valid");
        let stale_position = previous
            .visual_position_for_source(3, CursorAffinity::Downstream)
            .expect("fixture must map source byte 3");
        let reflowed_lines = vec![
            VisualLineProjection {
                flat_line_idx: 0,
                owner: ProjectionOwnerId::Block { block_start: 0, logical_line: 0 },
                boundaries: (0..3)
                    .map(SourceAnchor::downstream)
                    .chain(std::iter::once(SourceAnchor::upstream(3)))
                    .collect(),
                source_extent: 0..3,
                collapsed: Vec::new(),
            },
            VisualLineProjection {
                flat_line_idx: 1,
                owner: ProjectionOwnerId::Block { block_start: 0, logical_line: 0 },
                boundaries: (3..=9).map(SourceAnchor::downstream).collect(),
                source_extent: 3..9,
                collapsed: Vec::new(),
            },
        ];
        let reflowed =
            SourceProjectionIndex::build(7, 4, reflowed_lines).expect("fixture is valid");

        assert_eq!(
            reflowed.source_anchor_at(7, stale_position),
            Err(ProjectionError::StaleLayoutRevision { expected: 4, actual: 3 })
        );
    }

    #[test]
    fn direct_ascii_projection_has_one_boundary_per_grapheme_plus_sentinel() {
        let projected = ProjectedText::direct("abc", 5);
        assert_eq!(projected.text, "abc");
        assert_eq!(
            projected.boundaries.iter().map(|anchor| anchor.byte).collect::<Vec<_>>(),
            vec![5, 6, 7, 8]
        );
        assert_eq!(projected.validate(".....abc"), Ok(()));
    }

    #[test]
    fn virtual_insertion_splits_direct_projection_spans_without_overlap() {
        let projected = ProjectedText::direct("ab", 10).insert_virtual(1, "中文", 11);

        assert_eq!(projected.text, "a中文b");
        assert_eq!(
            projected.boundaries.iter().map(|anchor| anchor.byte).collect::<Vec<_>>(),
            vec![10, 11, 11, 11, 12]
        );
        assert_eq!(projected.spans.len(), 3);
        assert_eq!(projected.spans[0].visual_range, 0..1);
        assert!(matches!(projected.spans[0].kind, ProjectionSpanKind::Direct));
        assert_eq!(projected.spans[1].visual_range, 1.."a中文".len());
        assert!(matches!(
            projected.spans[1].kind,
            ProjectionSpanKind::Virtual { anchor_byte: 11, .. }
        ));
        assert_eq!(projected.spans[2].visual_range, "a中文".len().."a中文b".len());
        assert!(matches!(projected.spans[2].kind, ProjectionSpanKind::Direct));
    }

    #[test]
    fn validation_rejects_non_monotonic_source_boundaries() {
        let projected = ProjectedText {
            text: "ab".to_string(),
            spans: Vec::new(),
            boundaries: vec![
                SourceAnchor::downstream(4),
                SourceAnchor::downstream(3),
                SourceAnchor::downstream(5),
            ],
        };
        assert_eq!(
            projected.validate("....."),
            Err(ProjectionError::NonMonotonicSourceOrder { previous: 4, current: 3 })
        );
    }

    #[test]
    fn validation_rejects_boundary_count_mismatch() {
        let projected = ProjectedText {
            text: "👨\u{200d}👩".to_string(),
            spans: Vec::new(),
            boundaries: vec![SourceAnchor::downstream(0)],
        };
        assert!(matches!(
            projected.validate("👨\u{200d}👩"),
            Err(ProjectionError::BoundaryCountMismatch { .. })
        ));
    }
}
