use super::*;

pub(in crate::layout::block) struct LayoutTextLine {
    pub(in crate::layout::block) projected: crate::projection::ProjectedText,
    pub(in crate::layout::block) styles: Option<Vec<StyleSpan>>,
    pub(in crate::layout::block) source_context: Option<SourceLineContext>,
}

pub(in crate::layout::block) struct ListItemTextLines {
    pub(in crate::layout::block) lines: Vec<LayoutTextLine>,
}

pub(in crate::layout::block) fn list_item_text_lines_for_layout(
    block: &BlockNode,
    ctx: &LayoutCtx<'_>,
) -> ListItemTextLines {
    if !block.children.is_empty() && block.text_lines.is_empty() && block.text_styles.is_empty() {
        return ListItemTextLines { lines: Vec::new() };
    }

    ListItemTextLines { lines: list_item_projected_lines(block, ctx.doc) }
}

pub(in crate::layout::block) fn cursor_in_nested_list_item(
    block: &BlockNode,
    cursor_byte: usize,
) -> bool {
    block.children.iter().any(|child| {
        let cursor_in_child =
            child.block_range.start <= cursor_byte && cursor_byte <= child.block_range.end;
        let child_is_list_item = matches!(child.kind, BlockKind::ListItem { .. });
        (child_is_list_item && cursor_in_child) || cursor_in_nested_list_item(child, cursor_byte)
    })
}

pub(in crate::layout::block) fn source_line_context_for_layout(
    block: &BlockNode,
    doc: &dyn textora_core::document::DocView,
    line_idx: usize,
    line_text: &str,
) -> Option<SourceLineContext> {
    match &block.source_range {
        BlockSource::Fragmented(ranges) => ranges
            .get(line_idx)
            .map(|range| SourceLineContext { range: range.clone(), text_start: range.start }),
        BlockSource::Continuous(range) => {
            let mut line_start = range.start;
            for (idx, current_line) in block.lines(doc).iter().enumerate() {
                let current_len = source_line_len(block, idx, current_line);
                if idx == line_idx {
                    let marker_len =
                        if idx == 0 { crate::edit::block_marker_len(block) } else { 0 };
                    return Some(SourceLineContext {
                        range: line_start..line_start + current_len,
                        text_start: line_start + marker_len,
                    });
                }
                line_start += current_len + 1;
            }

            Some(SourceLineContext {
                range: line_start..line_start + line_text.len(),
                text_start: line_start,
            })
        }
    }
}

pub(in crate::layout::block) fn source_line_len(
    block: &BlockNode,
    line_idx: usize,
    line_text: &str,
) -> usize {
    let content_len = if let Some(spans) = block.text_styles.get(line_idx)
        && !spans.is_empty()
    {
        let mut source_len = 0usize;
        let mut folded_pos = 0usize;
        for span in spans {
            source_len += span.start - folded_pos;
            source_len += span.source_range.end - span.source_range.start;
            folded_pos = span.start + span.len;
        }
        source_len + line_text.len() - folded_pos
    } else {
        line_text.len()
    };

    if line_idx == 0 { content_len + crate::edit::block_marker_len(block) } else { content_len }
}

pub(in crate::layout::block) fn list_item_projected_lines(
    block: &BlockNode,
    doc: &dyn textora_core::document::DocView,
) -> Vec<LayoutTextLine> {
    let projected_lines = if block.text_lines.len() == 1 && block.text_lines[0].is_empty() {
        // Empty items have no text event; the parser's block end can include CRLF.
        // Their insertion anchor belongs at the end of the physical marker line.
        let block_source = doc.doc_text_in_range(block.block_range.clone());
        let marker_line_length = block_source.lines().next().map_or(0, str::len);
        vec![crate::projection::ProjectedText::direct(
            "",
            block.block_range.start + marker_line_length,
        )]
    } else if block.projected_lines.is_empty() {
        block
            .lines(doc)
            .into_iter()
            .enumerate()
            .map(|(line_idx, line)| {
                let source_context = source_line_context_for_layout(block, doc, line_idx, &line);
                crate::projection::ProjectedText::direct(
                    &line,
                    source_context.as_ref().map_or(0, |context| context.text_start),
                )
            })
            .collect()
    } else {
        block.projected_lines.clone()
    };

    projected_lines
        .into_iter()
        .enumerate()
        .map(|(line_idx, projected)| {
            let source_extent = projected.source_extent();
            let visual_range = 0..projected.text.len();
            LayoutTextLine {
                styles: styles_for_projected_visual_range(
                    block.text_styles.get(line_idx).map(Vec::as_slice).unwrap_or(&[]),
                    visual_range,
                    &projected,
                ),
                source_context: Some(SourceLineContext {
                    text_start: source_extent.start,
                    range: source_extent,
                }),
                projected,
            }
        })
        .collect()
}

pub(in crate::layout::block) fn styles_for_projected_visual_range(
    styles: &[StyleSpan],
    visual_range: std::ops::Range<usize>,
    projected: &crate::projection::ProjectedText,
) -> Option<Vec<StyleSpan>> {
    (!styles.is_empty()).then(|| {
        styles
            .iter()
            .filter_map(|span| {
                let span_end = span.start + span.len;
                let clipped_start = span.start.max(visual_range.start);
                let clipped_end = span_end.min(visual_range.end);
                (clipped_start < clipped_end).then(|| StyleSpan {
                    start: clipped_start - visual_range.start,
                    len: clipped_end - clipped_start,
                    style: span.style.clone(),
                    source_range: projected_style_source_range(
                        projected,
                        span,
                        clipped_start..clipped_end,
                    ),
                })
            })
            .collect()
    })
}

pub(in crate::layout::block) fn projected_style_source_range(
    projected: &crate::projection::ProjectedText,
    style_span: &StyleSpan,
    visual_range: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let style_end = style_span.start + style_span.len;
    let source_start = if visual_range.start == style_span.start {
        style_span.source_range.start
    } else {
        projected.boundaries
            [crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_range.start)]
        .byte
        .get()
    };
    let source_end = if visual_range.end == style_end {
        style_span.source_range.end
    } else {
        projected.boundaries
            [crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_range.end)]
        .byte
        .get()
    };
    source_start..source_end
}

pub(in crate::layout::block) fn marker_source_range_for_projected_line(
    projected: &crate::projection::ProjectedText,
    marker: &crate::edit::ActiveBlockMarker,
) -> std::ops::Range<usize> {
    let content_start = projected
        .boundaries
        .first()
        .expect("a projected line always has a sentinel boundary")
        .byte
        .get();
    let marker_len = marker.marker_text.len();
    content_start.saturating_sub(marker_len)..content_start
}

pub(in crate::layout::block) fn source_marker_for_projected_line(
    projected: &crate::projection::ProjectedText,
    marker: &crate::edit::ActiveBlockMarker,
    source: Option<&str>,
) -> Option<crate::edit::ActiveBlockMarker> {
    if !marker.marker_text.starts_with('>') {
        return Some(marker.clone());
    }
    let content_start = projected.boundaries.first()?.byte.get();
    let preceding_source = source?.get(..content_start)?;
    let line_start = preceding_source.rfind('\n').map_or(0, |newline| newline + 1);
    let line_prefix = &preceding_source[line_start..];
    let marker_offset = line_prefix.rfind('>')?;
    let marker_text = &line_prefix[marker_offset..];
    if !marker_text[1..].bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
        return None;
    }
    Some(crate::edit::ActiveBlockMarker {
        marker_text: marker_text.to_owned(),
        marker_source_range: line_start + marker_offset..content_start,
    })
}

pub(in crate::layout::block) fn materialize_marker_for_projected_line(
    projected: crate::projection::ProjectedText,
    marker: &crate::edit::ActiveBlockMarker,
    marker_source_range: std::ops::Range<usize>,
) -> crate::projection::ProjectedText {
    crate::edit::materialize_block_marker(
        projected,
        &crate::edit::ActiveBlockMarker {
            marker_text: marker.marker_text.clone(),
            marker_source_range,
        },
    )
}

pub(in crate::layout::block) fn estimated_visual_line_ranges(
    text: &str,
    max_w: f32,
    font_size: f32,
    first_line_indent: f32,
) -> Vec<std::ops::Range<usize>> {
    if text.is_empty() {
        let empty_text_range = 0..0;
        return vec![empty_text_range];
    }
    let char_w = font_size * 0.55;
    let graphemes_per_line = (max_w / char_w).max(1.0) as usize;
    let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(text);
    let grapheme_count = visual_grapheme_bytes.len().saturating_sub(1);
    let mut ranges = Vec::with_capacity(grapheme_count.div_ceil(graphemes_per_line));
    let mut grapheme_start = 0;
    while grapheme_start < grapheme_count {
        let line_capacity = if grapheme_start == 0 {
            ((max_w - first_line_indent) / char_w).max(1.0) as usize
        } else {
            graphemes_per_line
        };
        let grapheme_end = (grapheme_start + line_capacity).min(grapheme_count);
        ranges.push(visual_grapheme_bytes[grapheme_start]..visual_grapheme_bytes[grapheme_end]);
        grapheme_start = grapheme_end;
    }
    ranges
}

pub(in crate::layout::block) fn paragraph_first_line_indent(
    block: &BlockNode,
    ctx: &LayoutCtx,
    font_size: f32,
    raw_lines: &[std::borrow::Cow<'_, str>],
) -> f32 {
    if ctx.style.paragraph_first_line_indent <= 0.0
        || !matches!(block.kind, BlockKind::Paragraph)
        || ctx.indent != 0.0
        || ctx.list_depth != 0
        || ctx.font_size_override.is_some()
        || raw_lines.first().and_then(|line| line.chars().next()).is_some_and(char::is_whitespace)
    {
        return 0.0;
    }

    // CommonMark omits up to three leading ASCII spaces from paragraph text.
    // Inspect the owning source line too, so existing manual whitespace wins.
    let mut lower_line = 0;
    let mut upper_line = ctx.doc.line_count();
    while lower_line < upper_line {
        let middle_line = lower_line + (upper_line - lower_line) / 2;
        if ctx.doc.line_byte_offset(middle_line) <= block.block_range.start {
            lower_line = middle_line + 1;
        } else {
            upper_line = middle_line;
        }
    }
    if lower_line > 0
        && ctx.doc.doc_line_text(lower_line - 1).chars().next().is_some_and(char::is_whitespace)
    {
        return 0.0;
    }

    ctx.style.paragraph_first_line_indent.min((ctx.available_width() - font_size).max(0.0))
}

pub(crate) fn layout_text_block(
    block: &BlockNode,
    ctx: &mut LayoutCtx,
    font_size: f32,
    color: [f32; 4],
    font_weight: Weight,
) {
    let line_h = if matches!(block.kind, BlockKind::Heading { .. })
        || font_size >= ctx.style.heading_font_sizes[1]
    {
        font_size * HEADING_LINE_HEIGHT_RATIO
    } else {
        ctx.style.line_height
    };

    let raw_lines = collect_text_lines(block, ctx.doc);
    let first_line_indent = paragraph_first_line_indent(block, ctx, font_size, &raw_lines);
    let raw_styles = &block.text_styles;
    let mut laid_out_lines = Vec::new();
    let mut ly = ctx.y;

    for (line_idx, raw) in raw_lines.iter().enumerate() {
        let line_indent = if line_idx == 0 { first_line_indent } else { 0.0 };
        if ctx.shaper.is_none() {
            // Estimate line count from raw text only — the active block marker
            // must NOT participate, or the count would be wrong for content
            // near the wrap boundary. The marker is prepended downstream by
            // the precision path (shaper available), which handles wrapping
            // and source-map adjustment via prepend_marker_to_line.
            let mut line_styles = raw_styles.get(line_idx).cloned().unwrap_or_default();
            let mut source_projection =
                block.projected_lines.get(line_idx).cloned().unwrap_or_else(|| {
                    crate::projection::ProjectedText::direct(raw, block.block_range.start)
                });
            crate::layout::embedded::expose_failed_math(
                &mut source_projection,
                &mut line_styles,
                ctx.doc,
                font_size,
                color,
                true,
                ctx.selection_range,
            );
            let estimated_ranges = estimated_visual_line_ranges(
                &source_projection.text,
                ctx.available_width(),
                font_size,
                line_indent,
            );
            let visual_grapheme_bytes =
                crate::grapheme_map::grapheme_byte_boundaries(&source_projection.text);
            for (i, visual_range) in estimated_ranges.into_iter().enumerate() {
                let visual_indent = if i == 0 { line_indent } else { 0.0 };
                let text = if i == 0 { source_projection.text.clone() } else { String::new() };
                laid_out_lines.push(LaidOutLine {
                    // Put full raw text and styles in the first line so flat_lines
                    // have content. Wrapped lines (i>0) get empty — precision fills them.
                    text,
                    styles: if i == 0 { line_styles.clone() } else { vec![] },
                    rect: ui::core::geom::Rect::new(
                        ctx.indent + visual_indent,
                        ly,
                        ctx.available_width() - visual_indent,
                        line_h,
                    ),
                    font_size,
                    is_code: false,
                    font_weight,
                    color_override: Some(if ctx.color_fade > 0.0 {
                        crate::style::blend_toward_bg(
                            color,
                            ctx.style.background_color,
                            ctx.color_fade,
                        )
                    } else {
                        color
                    }),
                    doc_line_idx: line_idx,
                    style_segments: vec![],
                    shaped: None,
                    text_layout: None,
                    highlight_spans: vec![],
                    source_projection: Some(
                        source_projection
                            .slice_visual_line_indexed(&visual_grapheme_bytes, i, visual_range)
                            .expect("estimated wrapped lines must preserve source projections"),
                    ),
                });
                ly += line_h;
            }
            continue;
        }

        let line_styles = raw_styles.get(line_idx).map(|s| s.as_slice()).unwrap_or(&[]);
        let source_context = source_line_context_for_layout(block, ctx.doc, line_idx, raw);
        let base_projection = block.projected_lines.get(line_idx).cloned().unwrap_or_else(|| {
            crate::projection::ProjectedText::direct(
                raw,
                source_context.as_ref().map_or(0, |context| context.text_start),
            )
        });
        let mut projected = if let Some(source_text) = ctx.source_text {
            crate::edit::materialize_projected_line(
                &base_projection,
                line_styles,
                source_text,
                ctx.edit_ctx,
                source_context.as_ref(),
            )
        } else {
            base_projection.clone()
        };
        if let Some(source_text) = ctx.source_text {
            append_trailing_whitespace_projection(&mut projected, source_text);
        }
        let marker =
            (line_idx == 0).then_some(ctx.active_block_marker.as_ref()).flatten().and_then(
                |marker| source_marker_for_projected_line(&projected, marker, ctx.source_text),
            );
        let marker = marker.as_ref();
        let marker_source_range =
            marker.map(|marker| marker_source_range_for_projected_line(&projected, marker));
        let materialized_spans = if let Some(source_text) = ctx.source_text {
            crate::edit::materialized_spans_for_projected_line(
                &base_projection,
                &projected,
                line_styles,
                source_text,
                ctx.edit_ctx,
            )
        } else {
            crate::edit::materialize_line(raw, line_styles, "", None).spans
        };
        let mut materialized_styles: Vec<StyleSpan> = materialized_spans
            .iter()
            .map(|span| StyleSpan {
                start: span.start,
                len: span.len,
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            })
            .collect();
        if let Some(marker) = marker {
            projected = materialize_marker_for_projected_line(
                projected,
                marker,
                marker_source_range
                    .clone()
                    .expect("an active marker must have a derived source range"),
            );
            let marker_len = marker.marker_text.len();
            for style in &mut materialized_styles {
                style.start += marker_len;
            }
            materialized_styles.insert(
                0,
                StyleSpan {
                    start: 0,
                    len: marker_len,
                    style: crate::builder::InlineStyle::SourceMarker,
                    source_range: marker_source_range
                        .clone()
                        .expect("an active marker must have a derived source range"),
                },
            );
        }
        crate::layout::embedded::expose_failed_math(
            &mut projected,
            &mut materialized_styles,
            ctx.doc,
            font_size,
            color,
            !ctx.allow_embedded || ctx.shaper.is_none(),
            ctx.selection_range,
        );
        let mut projected_verbatim_ranges = projected_verbatim_ranges(&projected);
        if block.text_lines.is_empty() && raw.contains('\t') {
            projected_verbatim_ranges.push(0..projected.text.len());
        }
        let wrapped = ctx.wrap_text_with_first_line_indent_styled_and_protected(
            &projected.text,
            font_size,
            font_weight,
            ctx.available_width(),
            line_indent,
            &materialized_styles,
            &projected_verbatim_ranges,
            color,
        );
        let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(&projected.text);

        let shaped_input = ctx.last_wrap_shaped.first().and_then(|s| s.as_ref());
        let font_family_str = ctx.style.body_font_family.first().map(|s| s.as_str());
        for (visual_index, w) in wrapped.iter().enumerate() {
            let visual_indent = if visual_index == 0 { line_indent } else { 0.0 };
            let seg_start = w.byte_start;
            let seg_end = w.byte_end;
            // Extract style spans that fall within this wrapped segment
            let mut seg_styles = Vec::new();
            for span in &materialized_styles {
                let span_end = span.start + span.len;
                if span_end <= seg_start || span.start >= seg_end {
                    continue;
                }
                let clamp_start = span.start.max(seg_start) - seg_start;
                let clamp_end = span_end.min(seg_end) - seg_start;
                seg_styles.push(StyleSpan {
                    start: clamp_start,
                    len: clamp_end - clamp_start,
                    style: span.style.clone(),
                    source_range: span.source_range.clone(),
                });
            }
            let seg_text_layout = if let Some(full) = shaped_input {
                crate::layout::shaping::segment_text_layout(
                    full,
                    seg_start,
                    seg_end,
                    &w.text,
                    font_size,
                    font_family_str,
                    font_weight,
                )
            } else {
                None
            };
            let source_projection = projected
                .slice_visual_line_indexed(&visual_grapheme_bytes, 0, seg_start..seg_end)
                .expect("wrapped visual lines must end at projection grapheme boundaries");
            let row_height =
                line_height_for_math(&seg_styles, line_h, font_size, &ctx.embedded_images);
            laid_out_lines.push(LaidOutLine {
                text: w.text.clone(),
                rect: ui::core::geom::Rect::new(
                    ctx.indent + visual_indent,
                    ly,
                    ctx.available_width() - visual_indent,
                    row_height,
                ),
                font_size,
                is_code: false,
                font_weight,
                color_override: Some(if ctx.color_fade > 0.0 {
                    crate::style::blend_toward_bg(color, ctx.style.background_color, ctx.color_fade)
                } else {
                    color
                }),
                doc_line_idx: line_idx,
                styles: seg_styles.clone(),
                style_segments: vec![],
                shaped: seg_text_layout.as_ref().map(|layout| layout.shaped.clone()),
                text_layout: seg_text_layout,
                highlight_spans: vec![],
                source_projection: Some(source_projection),
            });
            ly += row_height;
        }
    }

    if block.is_editable_empty_paragraph() {
        for line in &mut laid_out_lines {
            if let Some(projection) = &mut line.source_projection {
                projection.owner = crate::projection::ProjectionOwnerId::EmptyLine {
                    source_byte: block.block_range.start,
                };
            }
        }
    }

    let total_h = if laid_out_lines.is_empty() { line_h } else { ly - ctx.y };
    ctx.push_block(LaidOutBlockKind::Text { lines: laid_out_lines }, total_h);
}

pub(in crate::layout::block) fn append_trailing_whitespace_projection(
    projected: &mut crate::projection::ProjectedText,
    source_text: &str,
) {
    if projected.text.trim().is_empty() {
        return;
    }
    let Some(last_boundary) = projected.boundaries.last() else {
        return;
    };
    let source_line_end = source_text[last_boundary.byte.get()..]
        .find('\n')
        .map_or(source_text.len(), |newline_offset| last_boundary.byte.get() + newline_offset);
    let trailing_range = last_boundary.byte.get()..source_line_end;
    let Some(trailing_text) = source_text.get(trailing_range.clone()) else {
        return;
    };
    if trailing_text.is_empty() || !trailing_text.chars().all(char::is_whitespace) {
        return;
    }
    let visual_end = projected.text.len();
    projected.spans.push(crate::projection::ProjectionSpan {
        source_range: trailing_range,
        visual_range: visual_end..visual_end,
        kind: crate::projection::ProjectionSpanKind::Collapsed,
    });
}

pub(in crate::layout::block) fn projected_verbatim_ranges(
    projected: &crate::projection::ProjectedText,
) -> Vec<std::ops::Range<usize>> {
    projected
        .spans
        .iter()
        .filter(|span| matches!(span.kind, crate::projection::ProjectionSpanKind::Verbatim))
        .map(|span| span.visual_range.clone())
        .collect()
}

/// Layout a line of text with style spans, adjusting spans for wrapped segments.
/// Returns LaidOutLine list with correct style information.
///
/// Every wrapped segment receives a slice of the canonical source projection.
#[allow(clippy::too_many_arguments)]
pub(in crate::layout::block) fn layout_line_with_styles(
    line_styles: &[StyleSpan],
    wrapped: &[crate::layout::types::WrappedLine],
    projected: &crate::projection::ProjectedText,
    font_size: f32,
    line_h: f32,
    x: f32,
    y_start: f32,
    width: f32,
    color: [f32; 4],
    font_family: Option<&str>,
    full_shaped: Option<&shaping::ShapedRun>,
    doc_line_idx: usize,
    embedded_images: &crate::layout::embedded::EmbeddedRegistry,
) -> Vec<LaidOutLine> {
    let mut result = Vec::new();
    let mut ly = y_start;
    let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(&projected.text);
    for w in wrapped {
        let seg_start = w.byte_start;
        let seg_end = w.byte_end;
        let mut seg_styles = Vec::new();
        for span in line_styles {
            let span_end = span.start + span.len;
            if span_end <= seg_start || span.start >= seg_end {
                continue;
            }
            let clamp_start = span.start.max(seg_start) - seg_start;
            let clamp_end = span_end.min(seg_end) - seg_start;
            seg_styles.push(StyleSpan {
                start: clamp_start,
                len: clamp_end - clamp_start,
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            });
        }
        let source_projection = projected
            .slice_visual_line_indexed(&visual_grapheme_bytes, 0, seg_start..seg_end)
            .expect("wrapped visual lines must end at projection grapheme boundaries");
        let seg_text_layout = full_shaped.and_then(|shaped| {
            crate::layout::shaping::segment_text_layout(
                shaped,
                seg_start,
                seg_end,
                &w.text,
                font_size,
                font_family,
                Weight::NORMAL,
            )
        });
        let row_height = line_height_for_math(&seg_styles, line_h, font_size, embedded_images);
        // Shaping deferred to render phase (only visible lines)
        result.push(LaidOutLine {
            text: w.text.clone(),
            rect: ui::core::geom::Rect::new(x, ly, width, row_height),
            font_size,
            is_code: false,
            font_weight: Weight::NORMAL,
            color_override: Some(color),
            doc_line_idx,
            styles: seg_styles,
            style_segments: vec![],
            shaped: seg_text_layout.as_ref().map(|layout| layout.shaped.clone()),
            text_layout: seg_text_layout,
            highlight_spans: vec![],
            source_projection: Some(source_projection),
        });
        ly += row_height;
    }
    result
}

pub(in crate::layout::block) fn line_height_for_math(
    styles: &[StyleSpan],
    default_height: f32,
    font_size: f32,
    embedded_images: &crate::layout::embedded::EmbeddedRegistry,
) -> f32 {
    let mut ascent = font_size;
    let mut descent = (default_height - font_size).max(0.0);
    for span in styles {
        if !matches!(span.style, crate::builder::InlineStyle::Math) {
            continue;
        }
        let Some(image) = embedded_images.image_for(&span.source_range) else {
            continue;
        };
        ascent = ascent.max(image.baseline);
        descent = descent.max(image.image.height() as f32 - image.baseline);
    }
    (ascent + descent).max(default_height)
}
