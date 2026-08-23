//! 编辑上下文——WYSIWYG 展开机制的核心数据类型和纯函数。

use crate::builder::{InlineStyle, StyleSpan};
use crate::projection::{ProjectedText, ProjectionSpanKind, SourceAnchor};
use std::ops::Range;

/// 编辑器光标上下文。传入 LazyLayout 控制哪些 span 展开源码。
#[derive(Clone, Debug)]
pub struct EditContext {
    /// 光标在源码中的字节偏移 (插入点，位于两个字节之间)。
    pub cursor_byte: usize,
    /// IME composing text rendered at the cursor without committing to source.
    pub preedit_text: Option<String>,
    /// IME composing cursor/selection byte range within preedit_text.
    pub preedit_cursor: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub(crate) struct SourceLineContext {
    pub range: Range<usize>,
    pub text_start: usize,
}

/// 判断光标是否在 span 的源码范围内。
/// 使用闭区间右侧 (<= end)：光标在 span 末尾时仍视为"在 span 内"，
/// 让用户能在边界处继续输入同一样式的文本。
pub fn cursor_in_span(span: &StyleSpan, cursor_byte: usize) -> bool {
    span.source_range.start <= cursor_byte && cursor_byte <= span.source_range.end
}

/// 返回 (prefix_marker_len, suffix_marker_len)。
pub(crate) fn span_marker_len(style: &InlineStyle) -> (usize, usize) {
    match style {
        InlineStyle::Bold => (2, 2),
        InlineStyle::Italic => (1, 1),
        InlineStyle::Strikethrough => (2, 2),
        InlineStyle::InlineCode => (1, 1),
        InlineStyle::Link { .. } => (1, 0),
        InlineStyle::SourceMarker => (0, 0),
    }
}

/// A materialized span in a rendered line, mapping visual text back to source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedSpan {
    pub start: usize,
    pub len: usize,
    pub style: InlineStyle,
    pub source_range: std::ops::Range<usize>,
}

/// A fully materialized line with source-byte mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedLine {
    pub text: String,
    pub spans: Vec<MaterializedSpan>,
    visual_grapheme_to_source_byte: Vec<usize>,
}

impl MaterializedLine {
    pub fn visual_grapheme_to_source_byte(&self, visual_grapheme: usize) -> Option<usize> {
        self.visual_grapheme_to_source_byte.get(visual_grapheme).copied()
    }

    /// Full visual-grapheme-index to source-byte mapping (including one-past-end sentinel).
    /// Callers slice this for wrapped-text segments.
    pub(crate) fn source_map(&self) -> &[usize] {
        &self.visual_grapheme_to_source_byte
    }

    pub fn source_byte_to_visual_grapheme(&self, source_byte: usize) -> Option<usize> {
        // Find the largest grapheme boundary ≤ source_byte.
        match self.visual_grapheme_to_source_byte.binary_search(&source_byte) {
            Ok(i) => Some(i.min(self.visual_grapheme_to_source_byte.len().saturating_sub(2))),
            Err(i) => Some(
                i.saturating_sub(1)
                    .min(self.visual_grapheme_to_source_byte.len().saturating_sub(2)),
            ),
        }
    }
}

// ===== Block-level marker support =====

/// Describes the block-level syntax marker for an active editing block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveBlockMarker {
    /// The marker text to render (e.g. "# ", "## ", "- ", "> ", "- [ ] ").
    pub marker_text: String,
    /// Source byte range of the marker (for cursor hit-testing within the marker).
    pub marker_source_range: std::ops::Range<usize>,
}

/// Returns the active block marker if `cursor_byte` falls within the block's source_range.
pub fn active_block_marker(
    block: &crate::builder::BlockNode,
    cursor_byte: usize,
) -> Option<ActiveBlockMarker> {
    if cursor_byte < block.block_range.start || cursor_byte > block.block_range.end {
        return None;
    }
    let block_start = block.block_range.start;
    match &block.kind {
        crate::builder::BlockKind::Heading { level } => {
            let marker = "#".repeat(*level as usize) + " ";
            let marker_len = marker.len();
            Some(ActiveBlockMarker {
                marker_text: marker,
                marker_source_range: block_start..block_start + marker_len,
            })
        }
        crate::builder::BlockKind::ListItem { bullet, .. } => {
            let marker = list_bullet_marker(bullet);
            let marker_len = marker.len();
            Some(ActiveBlockMarker {
                marker_text: marker,
                marker_source_range: block_start..block_start + marker_len,
            })
        }
        crate::builder::BlockKind::BlockQuote => Some(ActiveBlockMarker {
            marker_text: "> ".to_string(),
            marker_source_range: block_start..block_start + 2,
        }),
        _ => None,
    }
}

/// Compute the marker text for a list bullet (without the content text).
fn list_bullet_marker(bullet: &crate::builder::ListBullet) -> String {
    match bullet {
        crate::builder::ListBullet::Bullet => "- ".to_string(),
        crate::builder::ListBullet::Ordered(n) => format!("{}. ", n),
        crate::builder::ListBullet::TaskList(checked) => {
            let checkbox = if *checked { "[x]" } else { "[ ]" };
            format!("- {} ", checkbox)
        }
    }
}

/// Return the length (in bytes) of a block's syntax marker.
/// e.g., "# " = 2, "## " = 3, "- " = 2, "> " = 2.
///
/// Computed statically without allocation — this is called in the hot
/// `build_flat_lines` path on every cursor move.
pub fn block_marker_len(block: &crate::builder::BlockNode) -> usize {
    match &block.kind {
        crate::builder::BlockKind::Heading { level } => *level as usize + 1,
        crate::builder::BlockKind::ListItem { bullet, .. } => match bullet {
            crate::builder::ListBullet::Bullet => 2,
            crate::builder::ListBullet::Ordered(n) => n.ilog10() as usize + 3, // "n. "
            crate::builder::ListBullet::TaskList(_) => 6, // "- [ ] " or "- [x] "
        },
        crate::builder::BlockKind::BlockQuote => 2,
        _ => 0,
    }
}

pub(crate) fn materialize_block_marker(
    base: ProjectedText,
    marker: &ActiveBlockMarker,
) -> ProjectedText {
    base.prepend_direct(&marker.marker_text, marker.marker_source_range.clone())
}

pub(crate) fn materialize_projected_line(
    base: &ProjectedText,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
    source_line: Option<&SourceLineContext>,
) -> ProjectedText {
    let mut projected = base.clone();

    if let Some(cursor_span) =
        edit_ctx.and_then(|ctx| spans.iter().find(|span| cursor_in_span(span, ctx.cursor_byte)))
        && let Some(expansion) = inline_expansion(base, cursor_span, source)
    {
        let start_grapheme =
            crate::grapheme_map::grapheme_index_at_byte(&projected.text, cursor_span.start);
        let end_grapheme = crate::grapheme_map::grapheme_index_at_byte(
            &projected.text,
            cursor_span.start + cursor_span.len,
        );
        projected = projected.replace_graphemes_with_direct(
            end_grapheme,
            end_grapheme,
            &source[expansion.suffix.clone()],
            expansion.suffix,
        );
        projected = projected.replace_graphemes_with_direct(
            start_grapheme,
            start_grapheme,
            &source[expansion.prefix.clone()],
            expansion.prefix,
        );
    }

    let Some(ctx) = edit_ctx else {
        return projected;
    };
    let Some(preedit_text) = ctx.preedit_text.as_deref().filter(|text| !text.is_empty()) else {
        return projected;
    };
    let Some(source_line) = source_line else {
        return projected;
    };
    if ctx.cursor_byte < source_line.range.start || ctx.cursor_byte > source_line.range.end {
        return projected;
    }

    let insertion_grapheme = projected
        .boundaries
        .iter()
        .position(|anchor| anchor.byte >= ctx.cursor_byte)
        .unwrap_or_else(|| projected.grapheme_count());
    projected.insert_virtual(insertion_grapheme, preedit_text, ctx.cursor_byte)
}

#[derive(Clone, Debug)]
struct InlineExpansion {
    prefix: Range<usize>,
    content: Range<usize>,
    suffix: Range<usize>,
}

fn inline_expansion(
    base: &ProjectedText,
    span: &StyleSpan,
    source: &str,
) -> Option<InlineExpansion> {
    let start_grapheme = crate::grapheme_map::grapheme_index_at_byte(&base.text, span.start);
    let end_grapheme =
        crate::grapheme_map::grapheme_index_at_byte(&base.text, span.start + span.len);
    let content_start = base.boundaries.get(start_grapheme)?.byte;
    let projected_content_end = base.boundaries.get(end_grapheme)?.byte;
    let content_end = if projected_content_end == span.source_range.end {
        inferred_inline_suffix_start(&span.style, &source[span.source_range.clone()])
            .map_or(projected_content_end, |relative_start| {
                span.source_range.start + relative_start
            })
    } else {
        projected_content_end
    };

    if span.source_range.start > content_start
        || content_start > content_end
        || content_end > span.source_range.end
        || span.source_range.end > source.len()
        || !source.is_char_boundary(span.source_range.start)
        || !source.is_char_boundary(content_start)
        || !source.is_char_boundary(content_end)
        || !source.is_char_boundary(span.source_range.end)
    {
        return None;
    }

    Some(InlineExpansion {
        prefix: span.source_range.start..content_start,
        content: content_start..content_end,
        suffix: content_end..span.source_range.end,
    })
}

fn inferred_inline_suffix_start(style: &InlineStyle, span_source: &str) -> Option<usize> {
    match style {
        InlineStyle::Bold => span_source
            .ends_with("**")
            .then(|| span_source.len() - 2)
            .or_else(|| span_source.ends_with("__").then(|| span_source.len() - 2)),
        InlineStyle::Italic => span_source
            .ends_with('*')
            .then(|| span_source.len() - 1)
            .or_else(|| span_source.ends_with('_').then(|| span_source.len() - 1)),
        InlineStyle::Strikethrough => span_source.ends_with("~~").then(|| span_source.len() - 2),
        InlineStyle::InlineCode => span_source.ends_with('`').then(|| span_source.len() - 1),
        InlineStyle::Link { .. } => {
            if span_source.starts_with('<') && span_source.ends_with('>') {
                return Some(span_source.len() - 1);
            }
            span_source
                .find("](")
                .or_else(|| span_source.find("]["))
                .or_else(|| span_source.rfind(']'))
        }
        InlineStyle::SourceMarker => None,
    }
}

pub(crate) fn materialized_spans_for_projected_line(
    base: &ProjectedText,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
) -> Vec<MaterializedSpan> {
    let projected = materialize_projected_line(base, spans, source, edit_ctx, None);
    let preedit_visual_range = projected.spans.iter().find_map(|span| {
        matches!(span.kind, ProjectionSpanKind::Virtual { .. }).then(|| span.visual_range.clone())
    });
    materialized_spans(base, spans, source, edit_ctx, preedit_visual_range)
}

impl ProjectedText {
    pub(crate) fn slice_visual_line(
        &self,
        flat_line_idx: usize,
        visual_byte_range: Range<usize>,
    ) -> Result<crate::layout::types::VisualLineProjection, crate::projection::ProjectionError>
    {
        let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(&self.text);
        self.slice_visual_line_indexed(&visual_grapheme_bytes, flat_line_idx, visual_byte_range)
    }

    pub(crate) fn slice_visual_line_indexed(
        &self,
        visual_grapheme_bytes: &[usize],
        flat_line_idx: usize,
        visual_byte_range: Range<usize>,
    ) -> Result<crate::layout::types::VisualLineProjection, crate::projection::ProjectionError>
    {
        if visual_byte_range.start > visual_byte_range.end
            || visual_byte_range.end > self.text.len()
            || !self.text.is_char_boundary(visual_byte_range.start)
            || !self.text.is_char_boundary(visual_byte_range.end)
        {
            return Err(crate::projection::ProjectionError::InvalidSourceBoundary {
                byte: visual_byte_range.start,
            });
        }
        if visual_grapheme_bytes.len() != self.boundaries.len() {
            return Err(crate::projection::ProjectionError::BoundaryCountMismatch {
                expected: self.boundaries.len(),
                actual: visual_grapheme_bytes.len(),
            });
        }

        let start_grapheme =
            grapheme_index_at_visual_byte(visual_grapheme_bytes, visual_byte_range.start);
        let end_grapheme =
            grapheme_index_at_visual_byte(visual_grapheme_bytes, visual_byte_range.end);
        let boundaries = self.boundaries[start_grapheme..=end_grapheme].to_vec();
        let mut source_extent = boundaries
            .first()
            .expect("a projected visual line always has a sentinel boundary")
            .byte
            ..boundaries
                .last()
                .expect("a projected visual line always has a sentinel boundary")
                .byte;
        let mut collapsed = Vec::new();

        for span in &self.spans {
            let collapsed_outside_visual_line = span.visual_range.end < visual_byte_range.start
                || span.visual_range.start > visual_byte_range.end;
            if !matches!(span.kind, ProjectionSpanKind::Collapsed) || collapsed_outside_visual_line
            {
                continue;
            }

            let upstream_grapheme = grapheme_index_at_visual_byte(
                visual_grapheme_bytes,
                span.visual_range.start.max(visual_byte_range.start),
            ) - start_grapheme;
            let downstream_grapheme = grapheme_index_at_visual_byte(
                visual_grapheme_bytes,
                span.visual_range.end.min(visual_byte_range.end),
            ) - start_grapheme;
            source_extent.start = source_extent.start.min(span.source_range.start);
            source_extent.end = source_extent.end.max(span.source_range.end);
            collapsed.push(crate::layout::types::CollapsedBoundary {
                source_range: span.source_range.clone(),
                upstream_grapheme,
                downstream_grapheme,
            });
        }

        Ok(crate::layout::types::VisualLineProjection {
            flat_line_idx,
            owner: crate::projection::ProjectionOwnerId::Block {
                block_start: self.source_extent().start,
                logical_line: flat_line_idx,
            },
            boundaries,
            source_extent,
            collapsed,
        })
    }
}

fn grapheme_index_at_visual_byte(visual_grapheme_bytes: &[usize], visual_byte: usize) -> usize {
    match visual_grapheme_bytes.binary_search(&visual_byte) {
        Ok(index) => index,
        Err(insertion_index) => insertion_index.saturating_sub(1),
    }
}

fn project_folded_line(
    line_text: &str,
    spans: &[StyleSpan],
    source_line: Option<&SourceLineContext>,
) -> ProjectedText {
    let mut char_anchors = Vec::with_capacity(line_text.chars().count() + 1);
    let mut folded_byte = 0usize;
    let mut source_byte = spans
        .first()
        .map(|span| span.source_range.start.saturating_sub(span.start))
        .or_else(|| source_line.map(|line| line.text_start))
        .unwrap_or(0);

    for span in spans {
        if span.start > folded_byte {
            push_char_anchors(&mut char_anchors, &line_text[folded_byte..span.start], source_byte);
        }

        let folded = &line_text[span.start..span.start + span.len];
        let folded_source_start = span.source_range.start + span_marker_len(&span.style).0;
        push_char_anchors(&mut char_anchors, folded, folded_source_start);

        source_byte = span.source_range.end;
        folded_byte = span.start + span.len;
    }

    if folded_byte < line_text.len() {
        let trailing = &line_text[folded_byte..];
        push_char_anchors(&mut char_anchors, trailing, source_byte);
        source_byte += trailing.len();
    }

    char_anchors.push(SourceAnchor::downstream(source_byte));
    ProjectedText::from_char_anchors(line_text.to_string(), char_anchors, Vec::new())
}

fn push_char_anchors(anchors: &mut Vec<SourceAnchor>, text: &str, source_start: usize) {
    anchors.extend(
        text.char_indices()
            .map(|(relative_byte, _)| SourceAnchor::downstream(source_start + relative_byte)),
    );
}

fn materialized_spans(
    base: &ProjectedText,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
    preedit_visual_range: Option<Range<usize>>,
) -> Vec<MaterializedSpan> {
    let cursor_span =
        edit_ctx.and_then(|ctx| spans.iter().find(|span| cursor_in_span(span, ctx.cursor_byte)));
    let mut output = Vec::with_capacity(spans.len());
    let mut visual_delta = 0isize;

    for span in spans {
        let materialized_start = span
            .start
            .checked_add_signed(visual_delta)
            .expect("materialized style offsets must remain non-negative after source expansion");
        if Some(span.source_range.clone()) != cursor_span.map(|cursor| cursor.source_range.clone())
        {
            output.push(MaterializedSpan {
                start: materialized_start,
                len: span.len,
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            });
            continue;
        }

        let Some(expansion) = inline_expansion(base, span, source) else {
            output.push(MaterializedSpan {
                start: materialized_start,
                len: span.len,
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            });
            continue;
        };

        let prefix_len = expansion.prefix.len();
        let suffix_len = expansion.suffix.len();
        push_non_empty_materialized_span(
            &mut output,
            materialized_start,
            prefix_len,
            InlineStyle::SourceMarker,
            expansion.prefix,
        );
        push_non_empty_materialized_span(
            &mut output,
            materialized_start + prefix_len,
            span.len,
            span.style.clone(),
            expansion.content,
        );
        push_non_empty_materialized_span(
            &mut output,
            materialized_start + prefix_len + span.len,
            suffix_len,
            InlineStyle::SourceMarker,
            expansion.suffix,
        );
        visual_delta += (prefix_len + suffix_len) as isize;
    }

    if let Some(preedit_visual_range) = preedit_visual_range {
        shift_materialized_spans_for_preedit(&mut output, preedit_visual_range);
    }
    output
}

fn push_non_empty_materialized_span(
    spans: &mut Vec<MaterializedSpan>,
    start: usize,
    len: usize,
    style: InlineStyle,
    source_range: Range<usize>,
) {
    if len > 0 {
        spans.push(MaterializedSpan { start, len, style, source_range });
    }
}

fn shift_materialized_spans_for_preedit(
    spans: &mut [MaterializedSpan],
    preedit_visual_range: Range<usize>,
) {
    let inserted_len = preedit_visual_range.len();
    for span in spans {
        let span_end = span.start + span.len;
        if span.start >= preedit_visual_range.start {
            span.start += inserted_len;
        } else if span_end >= preedit_visual_range.start {
            span.len += inserted_len;
        }
    }
}

/// 生成一行的完整物化表示，包含源码字节映射。
///
/// 与 `materialize_text()` 不同，该函数返回 `MaterializedLine`，
/// 其中包含通过 `visual_grapheme_to_source_byte` 和 `source_byte_to_visual_grapheme`
/// 方法使用的视觉 grapheme 到源码字节的映射表。
pub fn materialize_line(
    line_text: &str,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
) -> MaterializedLine {
    materialize_line_with_source_context(line_text, spans, source, edit_ctx, None)
}

pub(crate) fn materialize_line_with_source_context(
    line_text: &str,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
    source_line: Option<&SourceLineContext>,
) -> MaterializedLine {
    let base = project_folded_line(line_text, spans, source_line);
    let projected = materialize_projected_line(&base, spans, source, edit_ctx, source_line);
    let preedit_visual_range = projected.spans.iter().find_map(|span| {
        matches!(span.kind, ProjectionSpanKind::Virtual { .. }).then(|| span.visual_range.clone())
    });

    MaterializedLine {
        text: projected.text,
        spans: materialized_spans(&base, spans, source, edit_ctx, preedit_visual_range),
        visual_grapheme_to_source_byte: projected
            .boundaries
            .iter()
            .map(|anchor| anchor.byte)
            .collect(),
    }
}

/// 拼接一行的布局用文本（向后兼容的包装器）。
pub fn materialize_text(
    line_text: &str,
    spans: &[StyleSpan],
    source: &str,
    edit_ctx: Option<&EditContext>,
) -> String {
    materialize_line(line_text, spans, source, edit_ctx).text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::InlineStyle;
    use crate::layout::types::{CollapsedBoundary, VisualLineProjection};
    use crate::projection::{ProjectionError, ProjectionOwnerId, TextProjectionBuilder};

    fn make_span(
        start: usize,
        len: usize,
        source_start: usize,
        source_end: usize,
        style: InlineStyle,
    ) -> StyleSpan {
        StyleSpan { start, len, style, source_range: source_start..source_end }
    }

    fn slice_visual_line_linear_reference(
        projected: &ProjectedText,
        flat_line_idx: usize,
        visual_byte_range: Range<usize>,
    ) -> Result<VisualLineProjection, ProjectionError> {
        if visual_byte_range.start > visual_byte_range.end
            || visual_byte_range.end > projected.text.len()
            || !projected.text.is_char_boundary(visual_byte_range.start)
            || !projected.text.is_char_boundary(visual_byte_range.end)
        {
            return Err(ProjectionError::InvalidSourceBoundary { byte: visual_byte_range.start });
        }

        let start_grapheme =
            crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_byte_range.start);
        let end_grapheme =
            crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_byte_range.end);
        let boundaries = projected.boundaries[start_grapheme..=end_grapheme].to_vec();
        let mut source_extent = boundaries
            .first()
            .expect("a projected visual line always has a sentinel boundary")
            .byte
            ..boundaries
                .last()
                .expect("a projected visual line always has a sentinel boundary")
                .byte;
        let mut collapsed = Vec::new();

        for span in &projected.spans {
            let collapsed_outside_visual_line = span.visual_range.end < visual_byte_range.start
                || span.visual_range.start > visual_byte_range.end;
            if !matches!(span.kind, ProjectionSpanKind::Collapsed) || collapsed_outside_visual_line
            {
                continue;
            }

            let upstream_grapheme = crate::grapheme_map::grapheme_index_at_byte(
                &projected.text,
                span.visual_range.start.max(visual_byte_range.start),
            ) - start_grapheme;
            let downstream_grapheme = crate::grapheme_map::grapheme_index_at_byte(
                &projected.text,
                span.visual_range.end.min(visual_byte_range.end),
            ) - start_grapheme;
            source_extent.start = source_extent.start.min(span.source_range.start);
            source_extent.end = source_extent.end.max(span.source_range.end);
            collapsed.push(CollapsedBoundary {
                source_range: span.source_range.clone(),
                upstream_grapheme,
                downstream_grapheme,
            });
        }

        Ok(VisualLineProjection {
            flat_line_idx,
            owner: ProjectionOwnerId::Block {
                block_start: projected.source_extent().start,
                logical_line: flat_line_idx,
            },
            boundaries,
            source_extent,
            collapsed,
        })
    }

    #[test]
    fn indexed_visual_line_slice_matches_linear_unicode_reference() {
        let unicode_segments = ["e\u{0301}", "👩‍💻", "❤\u{fe0f}", "中文"];
        let mut builder = TextProjectionBuilder::default();
        let mut source_byte = 0usize;

        for (segment_index, segment) in unicode_segments.iter().enumerate() {
            if segment_index > 0 {
                builder.push_soft_break(source_byte..source_byte + 1);
                source_byte += 1;
            }
            let source_end = source_byte + segment.len();
            builder.push_direct(segment, source_byte..source_end);
            source_byte = source_end;
        }

        let projected = builder.finish(source_byte);
        let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(&projected.text);
        let character_boundaries: Vec<usize> = (0..=projected.text.len())
            .filter(|byte| projected.text.is_char_boundary(*byte))
            .collect();

        for &visual_start in &character_boundaries {
            for &visual_end in character_boundaries.iter().filter(|&&byte| byte >= visual_start) {
                let visual_range = visual_start..visual_end;
                let linear =
                    slice_visual_line_linear_reference(&projected, 7, visual_range.clone());
                let indexed = projected.slice_visual_line_indexed(
                    &visual_grapheme_bytes,
                    7,
                    visual_range.clone(),
                );
                assert_eq!(
                    indexed, linear,
                    "indexed projection diverged for visual range {visual_range:?}"
                );
            }
        }
    }

    #[test]
    fn cursor_in_span_inclusive_end() {
        // source: "hello **world** here"
        // Bold span: line "world" at [6..11], source "**world**" at [6..15]
        let span = make_span(6, 5, 6, 15, InlineStyle::Bold);
        assert!(cursor_in_span(&span, 6)); // 光标在起始边界
        assert!(cursor_in_span(&span, 10)); // 光标在中间
        assert!(cursor_in_span(&span, 15)); // 光标在结束边界 (闭区间)
        assert!(!cursor_in_span(&span, 5)); // 光标在 span 前
        assert!(!cursor_in_span(&span, 16)); // 光标在 span 后
    }

    #[test]
    fn materialize_no_edit_ctx_returns_line_text() {
        let source = "hello **world** here";
        let line_text = "hello world here";
        let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
        let result = materialize_text(line_text, &spans, source, None);
        assert_eq!(result, "hello world here");
    }

    #[test]
    fn materialize_unfolds_cursor_span() {
        let source = "hello **world** here";
        let line_text = "hello world here";
        let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
        // 光标在 Bold span 内 (source byte 10)
        let ctx = EditContext { cursor_byte: 10, preedit_text: None, preedit_cursor: None };
        let result = materialize_text(line_text, &spans, source, Some(&ctx));
        assert_eq!(result, "hello **world** here");
    }

    #[test]
    fn materialize_no_matching_span_returns_line_text() {
        let source = "hello **world** here";
        let line_text = "hello world here";
        let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
        // 光标不在任何 span 内
        let ctx = EditContext { cursor_byte: 2, preedit_text: None, preedit_cursor: None };
        let result = materialize_text(line_text, &spans, source, Some(&ctx));
        assert_eq!(result, "hello world here");
    }

    #[test]
    fn materialize_multiple_spans_unfolds_only_cursor() {
        let source = "**bold** and *italic*";
        let line_text = "bold and italic";
        let spans = vec![
            make_span(0, 4, 0, 8, InlineStyle::Bold),
            make_span(9, 6, 13, 21, InlineStyle::Italic),
        ];
        // 光标在 Bold span 内
        let ctx = EditContext { cursor_byte: 3, preedit_text: None, preedit_cursor: None };
        let result = materialize_text(line_text, &spans, source, Some(&ctx));
        assert_eq!(result, "**bold** and italic");
    }

    #[test]
    fn materialize_cursor_at_italic_span() {
        let source = "**bold** and *italic*";
        let line_text = "bold and italic";
        let spans = vec![
            make_span(0, 4, 0, 8, InlineStyle::Bold),
            make_span(9, 6, 13, 21, InlineStyle::Italic),
        ];
        // 光标在 Italic span 内
        let ctx = EditContext { cursor_byte: 17, preedit_text: None, preedit_cursor: None };
        let result = materialize_text(line_text, &spans, source, Some(&ctx));
        assert_eq!(result, "bold and *italic*");
    }

    #[test]
    fn materialize_empty_line_text() {
        let result = materialize_text("", &[], "", None);
        assert_eq!(result, "");
    }

    #[test]
    fn materialize_cursor_at_boundary() {
        // 光标恰好在 span 的 source_range.end（闭区间包含）
        let source = "**bold** text";
        let line_text = "bold text";
        let spans = vec![make_span(0, 4, 0, 8, InlineStyle::Bold)];
        let ctx = EditContext { cursor_byte: 8, preedit_text: None, preedit_cursor: None }; // source_range.end = 8
        let result = materialize_text(line_text, &spans, source, Some(&ctx));
        assert_eq!(result, "**bold** text");
    }

    #[test]
    fn materialize_inline_code_span() {
        let source = "use `println!` here";
        let line_text = "use println! here";
        let spans = vec![make_span(4, 8, 4, 14, InlineStyle::InlineCode)];
        let ctx = EditContext { cursor_byte: 8, preedit_text: None, preedit_cursor: None };
        let result = materialize_text(line_text, &spans, source, Some(&ctx));
        assert_eq!(result, "use `println!` here");
    }

    #[test]
    fn materialized_line_maps_expanded_bold_markers_to_source_bytes() {
        let source = "hello **world** here";
        let line_text = "hello world here";
        let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
        let ctx = EditContext { cursor_byte: 10, preedit_text: None, preedit_cursor: None };

        let line = materialize_line(line_text, &spans, source, Some(&ctx));

        assert_eq!(line.text, "hello **world** here");
        assert_eq!(line.visual_grapheme_to_source_byte(6), Some(6));
        assert_eq!(line.visual_grapheme_to_source_byte(8), Some(8));
        assert_eq!(line.source_byte_to_visual_grapheme(10), Some(10));
    }

    #[test]
    fn materialized_multiline_bold_keeps_softbreak_collapsed() {
        let source = "**first\n  second**";
        let second_start = source.find("second").expect("fixture contains continuation text");
        let mut projection_builder = TextProjectionBuilder::default();
        projection_builder.push_direct("first", 2..7);
        projection_builder.push_soft_break(7..8);
        projection_builder.push_direct("second", second_start..second_start + "second".len());
        let base = projection_builder.finish(source.len() - 2);
        let spans = vec![make_span(0, "first second".len(), 0, source.len(), InlineStyle::Bold)];
        let ctx =
            EditContext { cursor_byte: second_start, preedit_text: None, preedit_cursor: None };

        let projected = materialize_projected_line(&base, &spans, source, Some(&ctx), None);
        let materialized_spans =
            materialized_spans_for_projected_line(&base, &spans, source, Some(&ctx));

        assert_eq!(projected.text, "**first second**");
        assert!(projected.spans.iter().any(|span| {
            matches!(span.kind, ProjectionSpanKind::Collapsed)
                && span.source_range.start == 7
                && span.source_range.end == second_start
        }));
        assert_eq!(
            materialized_spans
                .iter()
                .map(|span| (&projected.text[span.start..span.start + span.len], &span.style))
                .collect::<Vec<_>>(),
            vec![
                ("**", &InlineStyle::SourceMarker),
                ("first second", &InlineStyle::Bold),
                ("**", &InlineStyle::SourceMarker),
            ]
        );
        projected.validate(source).expect("materialized projection must remain valid");
    }

    #[test]
    fn materialized_line_keeps_folded_text_when_cursor_outside_span() {
        let source = "hello **world** here";
        let line_text = "hello world here";
        let spans = vec![make_span(6, 5, 6, 15, InlineStyle::Bold)];
        let ctx = EditContext { cursor_byte: 2, preedit_text: None, preedit_cursor: None };

        let line = materialize_line(line_text, &spans, source, Some(&ctx));

        assert_eq!(line.text, "hello world here");
        assert_eq!(line.visual_grapheme_to_source_byte(6), Some(8));
        assert_eq!(line.source_byte_to_visual_grapheme(10), Some(8));
    }

    #[test]
    fn materialized_line_maps_combining_grapheme_to_single_source_position() {
        let source = "**e\u{0301}**";
        let line_text = "e\u{0301}";
        // line_text is 2 chars (e + combining acute), 3 bytes total
        let spans = vec![make_span(0, line_text.len(), 0, source.len(), InlineStyle::Bold)];
        let ctx = EditContext { cursor_byte: 3, preedit_text: None, preedit_cursor: None };

        let line = materialize_line(line_text, &spans, source, Some(&ctx));

        assert_eq!(line.text, source);
        // ** markers at grapheme 0 and 1; é at grapheme 2; ** at 3 and 4
        assert_eq!(line.visual_grapheme_to_source_byte(0), Some(0));
        assert_eq!(line.visual_grapheme_to_source_byte(2), Some(2));
        // Next grapheme after é starts at byte 2 + "e\u{0301}".len() = 2 + 3 = 5
        assert_eq!(line.visual_grapheme_to_source_byte(3), Some(2 + "e\u{0301}".len()));
    }

    #[test]
    fn materialized_heading_marker_keeps_absolute_source_anchors() {
        let base = crate::projection::ProjectedText::direct("Title", 2);
        let marker = ActiveBlockMarker { marker_text: "# ".to_string(), marker_source_range: 0..2 };

        let projected = materialize_block_marker(base, &marker);

        assert_eq!(projected.text, "# Title");
        assert_eq!(
            projected.boundaries.iter().map(|anchor| anchor.byte).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5, 6, 7]
        );
    }

    #[test]
    fn preedit_visual_text_anchors_every_boundary_to_cursor_byte() {
        let base = crate::projection::ProjectedText::direct("ab", 10);

        let projected = base.insert_virtual(1, "中文", 11);

        assert_eq!(projected.text, "a中文b");
        assert_eq!(projected.boundaries[1].byte, 11);
        assert_eq!(projected.boundaries[2].byte, 11);
        assert_eq!(projected.boundaries[3].byte, 11);
    }

    // ===== ActiveBlockMarker tests =====

    use crate::builder::{BlockKind, BlockNode, BlockSource, ListBullet};

    fn heading_block(text: &str) -> BlockNode {
        BlockNode {
            kind: BlockKind::Heading { level: 1 },
            children: vec![],
            block_range: 0..text.len(),
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..text.len()),
            text_lines: vec![text[2..].to_string()], // content after "# "
            projected_lines: vec![],
            text_styles: vec![vec![]],
        }
    }

    #[test]
    fn active_marker_for_heading() {
        let block = heading_block("# Title");
        let marker =
            active_block_marker(&block, 3).expect("cursor in heading should produce marker");
        assert_eq!(marker.marker_text, "# ");
        assert_eq!(marker.marker_source_range, 0..2);
    }

    #[test]
    fn active_marker_none_when_cursor_outside_block_range() {
        let block = heading_block("# Title");
        assert!(active_block_marker(&block, 8).is_none(), "cursor past block end");
        assert!(active_block_marker(&block, 100).is_none(), "cursor way past block");
    }

    #[test]
    fn active_marker_for_h2() {
        let block = BlockNode {
            kind: BlockKind::Heading { level: 2 },
            children: vec![],
            block_range: 0..9,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..9),
            text_lines: vec!["SubTitle".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        let marker = active_block_marker(&block, 5).expect("cursor in h2");
        assert_eq!(marker.marker_text, "## ");
        assert_eq!(marker.marker_source_range, 0..3);
    }

    #[test]
    fn active_marker_for_list_item_bullet() {
        let block = BlockNode {
            kind: BlockKind::ListItem {
                bullet: ListBullet::Bullet,
                tight: true,
                blank_line_before: false,
            },
            children: vec![],
            block_range: 0..7,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..7),
            text_lines: vec!["item".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        let marker = active_block_marker(&block, 3).expect("cursor in list");
        assert_eq!(marker.marker_text, "- ");
        assert_eq!(marker.marker_source_range, 0..2);
    }

    #[test]
    fn active_marker_for_ordered_list_item() {
        let block = BlockNode {
            kind: BlockKind::ListItem {
                bullet: ListBullet::Ordered(3),
                tight: true,
                blank_line_before: false,
            },
            children: vec![],
            block_range: 0..6,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..6),
            text_lines: vec!["item".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        let marker = active_block_marker(&block, 2).expect("cursor in ordered list");
        assert_eq!(marker.marker_text, "3. ");
        assert_eq!(marker.marker_source_range, 0..3);
    }

    #[test]
    fn active_marker_for_task_list() {
        let block = BlockNode {
            kind: BlockKind::ListItem {
                bullet: ListBullet::TaskList(false),
                tight: true,
                blank_line_before: false,
            },
            children: vec![],
            block_range: 0..9,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..9),
            text_lines: vec!["todo".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        let marker = active_block_marker(&block, 3).expect("cursor in task list");
        assert_eq!(marker.marker_text, "- [ ] ");
        assert_eq!(marker.marker_source_range, 0..6);
    }

    #[test]
    fn active_marker_for_blockquote() {
        let block = BlockNode {
            kind: BlockKind::BlockQuote,
            children: vec![],
            block_range: 0..10,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..10),
            text_lines: vec!["quoted".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        let marker = active_block_marker(&block, 5).expect("cursor in blockquote");
        assert_eq!(marker.marker_text, "> ");
        assert_eq!(marker.marker_source_range, 0..2);
    }

    #[test]
    fn block_marker_len_heading() {
        let h1 = BlockNode {
            kind: BlockKind::Heading { level: 1 },
            children: vec![],
            block_range: 0..7,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..7),
            text_lines: vec!["Title".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        assert_eq!(block_marker_len(&h1), 2);
    }

    #[test]
    fn block_marker_len_h3() {
        let h3 = BlockNode {
            kind: BlockKind::Heading { level: 3 },
            children: vec![],
            block_range: 0..9,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..9),
            text_lines: vec!["Small".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        assert_eq!(block_marker_len(&h3), 4); // "### " = 4 bytes
    }

    #[test]
    fn block_marker_len_paragraph_is_zero() {
        let para = BlockNode {
            kind: BlockKind::Paragraph,
            children: vec![],
            block_range: 0..5,
            code_line_source_starts: None,
            source_range: BlockSource::Continuous(0..5),
            text_lines: vec!["text".to_string()],
            projected_lines: vec![],
            text_styles: vec![vec![]],
        };
        assert_eq!(block_marker_len(&para), 0);
    }
}
