use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::render) fn render_line_with_embedded(
    line: &LaidOutLine,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    ox: f32,
    oy: f32,
    shaper: Option<&mut shaping::Shaper>,
    embedded_images: Option<&EmbeddedRegistry>,
) {
    let image_ascent = line
        .styles
        .iter()
        .filter_map(|span| {
            matches!(span.style, InlineStyle::Math)
                .then(|| embedded_images?.image_for(&span.source_range).map(|entry| entry.baseline))
                .flatten()
        })
        .fold(line.font_size, f32::max);
    let ly = line.rect.y - scroll_y + oy + image_ascent - line.font_size;
    let base_color = line.color_override.unwrap_or(style.text_color);
    let line_x = line.rect.x + ox;

    let font_family = if line.is_code {
        style.code_font_family.clone()
    } else {
        style.body_font_family.first().cloned()
    };

    let paint = LinePaint { line, style, line_x, ly, base_color, font_family };
    if line.styles.is_empty() {
        paint.draw_plain(dl, shaper);
    } else if line.style_segments.is_empty() {
        paint.draw_unpositioned_styles(dl, shaper, embedded_images);
    } else {
        paint.draw_positioned_styles(dl, shaper, embedded_images);
    }
}

struct LinePaint<'a> {
    line: &'a LaidOutLine,
    style: &'a MarkdownStyle,
    line_x: f32,
    ly: f32,
    base_color: [f32; 4],
    font_family: Option<String>,
}

impl LinePaint<'_> {
    fn draw_plain(self, dl: &mut DrawList, mut shaper: Option<&mut shaping::Shaper>) {
        let Self { line, line_x, ly, base_color, font_family, .. } = self;
        let font_size = line.font_size;
        if !line.highlight_spans.is_empty() {
            // Syntax-highlighted code line: render per-span with highlight colors
            // Use actual HarfBuzz shaped widths to advance cursor (not estimated widths)
            let text_len = line.text.len();
            let mut cursor_x = line_x;
            let mut last_end = 0usize;
            for span in &line.highlight_spans {
                let span_start = span.start.min(text_len);
                let span_end = (span.start + span.len).min(text_len);
                if span_start > last_end {
                    // Gap before this span — render with base color
                    let gap = &line.text[safe_byte_idx(&line.text, last_end)
                        ..safe_byte_idx(&line.text, span_start)];
                    if let Some(ref mut s) = shaper {
                        let w = dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            base_color,
                            gap,
                            font_family.clone(),
                            line.font_weight,
                            Style::Normal,
                            false,
                            s,
                        );
                        cursor_x += w;
                    } else {
                        cursor_x += estimate_text_width(gap, font_size);
                    }
                }
                if span_start < text_len {
                    let segment = &line.text[safe_byte_idx(&line.text, span_start)
                        ..safe_byte_idx(&line.text, span_end)];
                    if let Some(ref mut s) = shaper {
                        let w = dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            span.color,
                            segment,
                            font_family.clone(),
                            line.font_weight,
                            Style::Normal,
                            false,
                            s,
                        );
                        cursor_x += w;
                    } else {
                        cursor_x += estimate_text_width(segment, font_size);
                    }
                }
                last_end = span_end;
            }
            if last_end < text_len {
                let tail = &line.text[safe_byte_idx(&line.text, last_end)..];
                if let Some(ref mut s) = shaper {
                    dl.text_shaped_with_font(
                        cursor_x,
                        ly + font_size,
                        font_size,
                        base_color,
                        tail,
                        font_family,
                        line.font_weight,
                        Style::Normal,
                        false,
                        s,
                    );
                }
            }
        } else if let Some(ref layout) = line.text_layout {
            dl.text_layout(layout.clone(), line_x, ly + font_size, base_color);
        } else if let Some(ref mut s) = shaper {
            dl.text_shaped_with_font(
                line_x,
                ly + font_size,
                font_size,
                base_color,
                &line.text,
                font_family,
                line.font_weight,
                Style::Normal,
                false,
                s,
            );
        }
    }

    fn draw_unpositioned_styles(
        self,
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
        embedded_images: Option<&EmbeddedRegistry>,
    ) {
        let Self { line, style, line_x, ly, base_color, font_family } = self;
        let font_size = line.font_size;
        let text_len = line.text.len();
        // Draw inline code backgrounds
        for span in &line.styles {
            if span.start >= text_len {
                continue;
            }
            if matches!(span.style, InlineStyle::InlineCode) {
                let span_end = (span.start + span.len).min(text_len);
                let segment = &line.text
                    [safe_byte_idx(&line.text, span.start)..safe_byte_idx(&line.text, span_end)];
                let w = measure_text_width_with_font(
                    segment,
                    font_size,
                    font_family.as_deref(),
                    line.font_weight,
                    Style::Normal,
                    shaper.as_deref_mut(),
                );
                let prefix = &line.text[..safe_byte_idx(&line.text, span.start)];
                let prefix_w = measure_text_width_with_font(
                    prefix,
                    font_size,
                    font_family.as_deref(),
                    line.font_weight,
                    Style::Normal,
                    shaper.as_deref_mut(),
                );
                let padding =
                    inline_code_background_padding(&line.text, span.start, span_end, font_size);
                let rect =
                    inline_code_background_rect(line_x + prefix_w, ly, w, font_size, padding);
                draw_inline_code_bg(style, dl, rect);
            }
        }
        // Render text split by style spans using actual shaped widths
        let mut cursor_x = line_x;
        let mut last_end = 0usize;
        for span in &line.styles {
            if span.start >= text_len {
                continue;
            }
            if span.start > last_end {
                let gap = &line.text
                    [safe_byte_idx(&line.text, last_end)..safe_byte_idx(&line.text, span.start)];
                if let Some(ref mut s) = shaper {
                    let w = dl.text_shaped_with_font(
                        cursor_x,
                        ly + font_size,
                        font_size,
                        base_color,
                        gap,
                        font_family.clone(),
                        line.font_weight,
                        Style::Normal,
                        false,
                        s,
                    );
                    cursor_x += w;
                } else {
                    cursor_x += estimate_text_width(gap, font_size);
                }
            }
            let span_end = (span.start + span.len).min(text_len);
            if span.start < text_len {
                if matches!(span.style, InlineStyle::Math)
                    && let Some(entry) =
                        embedded_images.and_then(|images| images.image_for(&span.source_range))
                {
                    let image_width = entry.image.width() as f32;
                    dl.image(
                        entry.image.clone(),
                        Rect::new(
                            cursor_x,
                            ly + font_size - entry.baseline,
                            image_width,
                            entry.image.height() as f32,
                        ),
                    );
                    cursor_x += image_width;
                    last_end = span_end;
                    continue;
                }
                let segment = &line.text
                    [safe_byte_idx(&line.text, span.start)..safe_byte_idx(&line.text, span_end)];
                let color = style_for_span(&span.style, base_color, style);
                let mut w = if let Some(ref mut s) = shaper {
                    if needs_styled_text(&span.style) {
                        let (ws_weight, ws_style) = weight_style_for(&span.style);
                        let italic = is_italic(&span.style);
                        dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            color,
                            segment,
                            font_family.clone(),
                            ws_weight,
                            ws_style,
                            italic,
                            s,
                        )
                    } else {
                        dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            color,
                            segment,
                            font_family.clone(),
                            line.font_weight,
                            Style::Normal,
                            false,
                            s,
                        )
                    }
                } else {
                    estimate_text_width(segment, font_size)
                };
                if is_italic(&span.style) {
                    w += font_size * ITALIC_SHEAR;
                }
                if is_underlined(&span.style) {
                    dl.fill(Rect::new(cursor_x, ly + font_size + 2.0, w, 1.0), color);
                }
                if is_strikethrough(&span.style) {
                    dl.fill(
                        Rect::new(
                            cursor_x,
                            ly + font_size * 0.55,
                            w,
                            strikethrough_thickness(font_size),
                        ),
                        color,
                    );
                }
                cursor_x += w;
            }
            last_end = span_end;
        }
        if last_end < text_len {
            let tail = &line.text[safe_byte_idx(&line.text, last_end)..];
            if let Some(ref mut s) = shaper {
                dl.text_shaped_with_font(
                    cursor_x,
                    ly + font_size,
                    font_size,
                    base_color,
                    tail,
                    font_family,
                    line.font_weight,
                    Style::Normal,
                    false,
                    s,
                );
            }
        }
    }

    fn draw_positioned_styles(
        self,
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
        embedded_images: Option<&EmbeddedRegistry>,
    ) {
        let Self { line, style, line_x, ly, base_color, font_family } = self;
        let font_size = line.font_size;
        let text_len = line.text.len();
        // Use precomputed style_segments for precise positioning
        let segments = &line.style_segments;

        // 1) Draw inline code backgrounds (behind text)
        for seg in segments {
            if matches!(seg.style, InlineStyle::InlineCode) {
                let seg_end = (seg.start + seg.len).min(text_len);
                let padding =
                    inline_code_background_padding(&line.text, seg.start, seg_end, font_size);
                let rect = inline_code_background_rect(
                    line_x + seg.x_offset,
                    ly,
                    seg.width,
                    font_size,
                    padding,
                );
                draw_inline_code_bg(style, dl, rect);
            }
        }

        // 2) Render text in segment order: gap, styled, gap, styled, tail
        let mut last_end = 0usize;

        for seg in segments {
            if seg.start >= text_len {
                continue;
            }
            // Render unstyled gap before this segment
            if seg.start > last_end {
                let gap_x = end_x_for_offset(segments, line_x, last_end, text_len);
                draw_line_segment(
                    line,
                    last_end..seg.start,
                    None,
                    [gap_x, ly + font_size],
                    base_color,
                    font_family.as_deref(),
                    dl,
                    shaper.as_deref_mut(),
                );
            }
            // Render styled segment
            let seg_end = (seg.start + seg.len).min(text_len);
            if seg.start < text_len {
                let color = style_for_span(&seg.style, base_color, style);
                let x = line_x + seg.x_offset;
                let math_image = matches!(seg.style, InlineStyle::Math)
                    .then(|| {
                        line.styles
                            .iter()
                            .find(|span| span.start == seg.start && span.len == seg.len)
                            .and_then(|span| {
                                embedded_images
                                    .and_then(|images| images.image_for(&span.source_range))
                            })
                    })
                    .flatten();
                if let Some(entry) = math_image {
                    dl.image(
                        entry.image.clone(),
                        Rect::new(
                            x,
                            ly + font_size - entry.baseline,
                            entry.image.width() as f32,
                            entry.image.height() as f32,
                        ),
                    );
                } else {
                    draw_line_segment(
                        line,
                        seg.start..seg_end,
                        Some(&seg.style),
                        [x, ly + font_size],
                        color,
                        font_family.as_deref(),
                        dl,
                        shaper.as_deref_mut(),
                    );
                }
                let mut seg_w = seg.width;
                if is_italic(&seg.style) {
                    seg_w += font_size * ITALIC_SHEAR;
                }
                if is_underlined(&seg.style) {
                    dl.fill(Rect::new(x, ly + font_size + 2.0, seg_w, 1.0), color);
                }
                if is_strikethrough(&seg.style) {
                    dl.fill(
                        Rect::new(
                            x,
                            ly + font_size * 0.55,
                            seg_w,
                            strikethrough_thickness(font_size),
                        ),
                        color,
                    );
                }
            }
            last_end = seg_end;
        }

        // Render remaining unstyled tail after last segment
        if last_end < text_len {
            let tail_x = end_x_for_offset(segments, line_x, last_end, text_len);
            draw_line_segment(
                line,
                last_end..text_len,
                None,
                [tail_x, ly + font_size],
                base_color,
                font_family.as_deref(),
                dl,
                shaper,
            );
        }
    }
}

/// Reuse the final layout geometry, including boundary gaps and shaping offsets.
#[allow(clippy::too_many_arguments)]
pub(in crate::render) fn draw_line_segment(
    line: &LaidOutLine,
    byte_range: std::ops::Range<usize>,
    inline_style: Option<&InlineStyle>,
    origin: [f32; 2],
    color: [f32; 4],
    font_family: Option<&str>,
    draw_list: &mut DrawList,
    shaper: Option<&mut shaping::Shaper>,
) {
    let start = safe_byte_idx(&line.text, byte_range.start);
    let end = safe_byte_idx(&line.text, byte_range.end);
    let text = &line.text[start..end];
    let (weight, font_style) = inline_style
        .filter(|style| needs_styled_text(style))
        .map(weight_style_for)
        .unwrap_or((line.font_weight, Style::Normal));
    let italic = inline_style.is_some_and(is_italic);
    if let Some(shaped) = &line.shaped
        && let Some(mut layout) = crate::layout::shaping::segment_text_layout(
            shaped,
            start,
            end,
            text,
            line.font_size,
            font_family,
            weight,
        )
    {
        Arc::make_mut(&mut layout).italic = italic;
        draw_list.text_layout(layout, origin[0], origin[1], color);
        return;
    }
    if let Some(shaper) = shaper {
        draw_list.text_shaped_with_font(
            origin[0],
            origin[1],
            line.font_size,
            color,
            text,
            font_family.map(str::to_string),
            weight,
            font_style,
            italic,
            shaper,
        );
    }
}

pub(in crate::render) fn inline_code_background_padding(
    line_text: &str,
    span_start: usize,
    span_end: usize,
    font_size: f32,
) -> (f32, f32) {
    let padding = font_size * INLINE_CODE_BACKGROUND_HORIZONTAL_PADDING_RATIO;
    let left_padding = if line_text[..safe_byte_idx(line_text, span_start)]
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
    {
        padding
    } else {
        0.0
    };
    let right_padding = if line_text[safe_byte_idx(line_text, span_end)..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        padding
    } else {
        0.0
    };

    (left_padding, right_padding)
}

pub(in crate::render) fn inline_code_background_rect(
    text_x: f32,
    line_y: f32,
    text_width: f32,
    font_size: f32,
    padding: (f32, f32),
) -> Rect {
    let (left_padding, right_padding) = padding;
    Rect::new(
        text_x - left_padding,
        line_y,
        text_width + left_padding + right_padding,
        font_size * INLINE_CODE_BACKGROUND_HEIGHT_RATIO,
    )
}

pub(in crate::render) fn draw_inline_code_bg(style: &MarkdownStyle, dl: &mut DrawList, rect: Rect) {
    let radius = rect.h * INLINE_CODE_BACKGROUND_RADIUS_RATIO;
    dl.fill_rounded(rect, style.inline_code_bg, radius);
}

pub(in crate::render) fn measure_text_width_with_font(
    text: &str,
    font_size: f32,
    font_family: Option<&str>,
    font_weight: Weight,
    font_style: Style,
    shaper: Option<&mut shaping::Shaper>,
) -> f32 {
    let Some(shaper) = shaper else {
        return estimate_text_width(text, font_size);
    };
    if text.is_empty() {
        return 0.0;
    }

    let old_size = shaper.font_size();
    let old_weight = shaper.font_weight();
    let old_style = shaper.font_style();
    let old_family = shaper.font_family().map(str::to_string);

    shaper.set_font_size(font_size);
    shaper.set_font_weight(font_weight);
    shaper.set_font_style(font_style);
    shaper.set_font_family(font_family);
    let width = shaper
        .shape(text)
        .map(|run| run.width)
        .unwrap_or_else(|_| estimate_text_width(text, font_size));

    shaper.set_font_size(old_size);
    shaper.set_font_weight(old_weight);
    shaper.set_font_style(old_style);
    shaper.set_font_family(old_family.as_deref());

    width
}

/// Compute the x position at a given byte offset within a line.
/// Uses the nearest preceding style_segment's position + its width.
pub(in crate::render) fn end_x_for_offset(
    segments: &[crate::layout::StyleSegment],
    line_x: f32,
    byte_offset: usize,
    text_len: usize,
) -> f32 {
    if byte_offset == 0 {
        return line_x;
    }
    // Find the segment that ends at or before byte_offset
    for seg in segments.iter().rev() {
        let seg_end = (seg.start + seg.len).min(text_len);
        if seg_end <= byte_offset {
            return line_x + seg.x_offset + seg.width;
        }
    }
    line_x
}

/// Get the rendering color for an inline style.
pub(in crate::render) fn style_for_span(
    inline: &InlineStyle,
    base_color: [f32; 4],
    style: &MarkdownStyle,
) -> [f32; 4] {
    match inline {
        InlineStyle::Bold => base_color,
        InlineStyle::Italic => base_color,
        InlineStyle::Strikethrough => base_color,
        InlineStyle::InlineCode => style.code_color,
        InlineStyle::Math => base_color,
        InlineStyle::Link { .. } => style.link_color,
        InlineStyle::SourceMarker => {
            blend_toward_bg(base_color, style.background_color, SOURCE_MARKER_FADE_RATIO)
        }
    }
}

pub(in crate::render) fn is_underlined(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Link { .. })
}

pub(in crate::render) fn is_strikethrough(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Strikethrough)
}

pub(in crate::render) fn strikethrough_thickness(font_size: f32) -> f32 {
    (font_size * STRIKETHROUGH_THICKNESS_RATIO).max(MIN_STRIKETHROUGH_THICKNESS)
}

/// Map an inline style to (font_weight, font_style).
/// Note: Italic returns Style::Normal — slant is applied in vertex stage via shear transform.
/// Bold uses SEMIBOLD (600) instead of BOLD (700) because macOS CJK fonts
/// (PingFang SC) max out at Semibold; BOLD queries trigger cross-font fallback.
pub(in crate::render) fn weight_style_for(inline: &InlineStyle) -> (Weight, Style) {
    match inline {
        InlineStyle::Bold => (Weight::SEMIBOLD, Style::Normal),
        InlineStyle::Italic => (Weight::NORMAL, Style::Normal),
        InlineStyle::SourceMarker
        | InlineStyle::Strikethrough
        | InlineStyle::InlineCode
        | InlineStyle::Math
        | InlineStyle::Link { .. } => (Weight::NORMAL, Style::Normal),
    }
}

pub(in crate::render) fn is_italic(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Italic)
}

/// Whether this inline style needs a non-default weight or style.
pub(in crate::render) fn needs_styled_text(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Bold | InlineStyle::Italic)
}

/// Text width estimate, CJK-aware.
///
/// CJK/fullwidth characters occupy roughly 1.0 × font_size;
/// ASCII and other narrow characters occupy roughly 0.55 × font_size.
pub(in crate::render) fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    let mut w = 0.0f32;
    for ch in text.chars() {
        if crate::layout::is_cjk_or_fullwidth(ch) {
            w += font_size;
        } else {
            w += font_size * 0.55;
        }
    }
    w
}
