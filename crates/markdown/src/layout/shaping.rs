//! Shaping and style segment computation.

use shaping::Shaper;
use std::sync::Arc;

use super::types::{LaidOutBlock, LaidOutBlockKind, LaidOutLine, StyleSegment};
use crate::builder::InlineStyle;
use crate::safe_byte_idx;
use crate::style::MarkdownStyle;

const ITALIC_VISUAL_WIDTH_EXTRA_RATIO: f32 = ui::core::text_layout::ITALIC_SHEAR;

#[derive(Clone, Copy)]
struct SegmentShape {
    weight: shaping::Weight,
    style: shaping::Style,
    italic: bool,
}

/// Shape text and build UiTextLayout during layout phase.
/// Returns (shaped_run, text_layout) — both None if no shaper or empty text.
#[allow(dead_code)] // Used in tests; may be needed for on-demand shaping
pub(crate) fn shape_line(
    text: &str,
    font_size: f32,
    weight: shaping::Weight,
    style: shaping::Style,
    font_family: Option<&str>,
    shaper: Option<&mut Shaper>,
) -> (Option<shaping::ShapedRun>, Option<Arc<ui::core::text_layout::UiTextLayout>>) {
    let Some(shaper) = shaper else {
        return (None, None);
    };
    if text.is_empty() {
        return (None, None);
    }
    let old_size = shaper.font_size();
    let old_weight = shaper.font_weight();
    let old_style = shaper.font_style();
    let old_family = shaper.font_family().map(|s| s.to_string());
    shaper.set_font_size(font_size);
    shaper.set_font_weight(weight);
    shaper.set_font_style(style);
    if let Some(family) = font_family {
        shaper.set_font_family(Some(family));
    }
    let shaped = shaper.shape(text).ok();
    shaper.set_font_size(old_size);
    shaper.set_font_weight(old_weight);
    shaper.set_font_style(old_style);
    shaper.set_font_family(old_family.as_deref());

    let text_layout = shaped.as_ref().map(|s| {
        Arc::new(ui::core::text_layout::UiTextLayout::from_shaped(
            text,
            font_size,
            font_family.map(|s| s.to_string()),
            weight,
            style,
            false, /* italic handled at vertex stage */
            s.clone(),
        ))
    });
    (shaped, text_layout)
}

/// Create a UiTextLayout for a byte-range segment of a pre-shaped input line.
/// Avoids re-shaping by extracting the relevant clusters from the full shaped run.
pub(crate) fn segment_text_layout(
    full_shaped: &shaping::ShapedRun,
    seg_byte_start: usize,
    seg_byte_end: usize,
    seg_text: &str,
    font_size: f32,
    font_family: Option<&str>,
    font_weight: shaping::Weight,
) -> Option<std::sync::Arc<ui::core::text_layout::UiTextLayout>> {
    if seg_text.is_empty() {
        return None;
    }
    let mut clusters = Vec::new();
    let mut width: f32 = 0.0;
    for cluster in &full_shaped.clusters {
        // Skip clusters outside the segment
        if cluster.byte_range.end <= seg_byte_start || cluster.byte_range.start >= seg_byte_end {
            continue;
        }
        clusters.push(shaping::GlyphCluster {
            byte_range: (cluster.byte_range.start - seg_byte_start)
                ..(cluster.byte_range.end - seg_byte_start),
            glyph_id: cluster.glyph_id,
            font_id: cluster.font_id,
            advance: cluster.advance,
            x_offset: cluster.x_offset,
            y_offset: cluster.y_offset,
        });
        width += cluster.advance;
    }
    if clusters.is_empty() {
        return None;
    }
    let segment_run = shaping::ShapedRun { clusters, width };
    Some(std::sync::Arc::new(ui::core::text_layout::UiTextLayout::from_shaped(
        seg_text,
        font_size,
        font_family.map(|s| s.to_string()),
        font_weight,
        shaping::Style::Normal,
        false,
        segment_run,
    )))
}

/// Recursively populate style_segments for all Text lines in a block tree.
pub(crate) fn populate_style_segments(
    block: &mut LaidOutBlock,
    shaper: &mut shaping::Shaper,
    style: &MarkdownStyle,
) {
    match &mut block.kind {
        LaidOutBlockKind::Text { lines } => {
            let font_family = style.body_font_family.first().map(|s| s.as_str());
            for line in lines {
                if !line.styles.is_empty() {
                    populate_line_style_segments(line, shaper, font_family);
                }
            }
        }
        LaidOutBlockKind::CodeBlock { .. } => {}
        LaidOutBlockKind::BlockQuote { blocks } => {
            for child in blocks {
                populate_style_segments(child, shaper, style);
            }
        }
        LaidOutBlockKind::ListItem { lines, blocks, .. } => {
            let font_family = style.body_font_family.first().map(|s| s.as_str());
            for line in lines {
                if !line.styles.is_empty() {
                    populate_line_style_segments(line, shaper, font_family);
                }
            }
            for child in blocks {
                populate_style_segments(child, shaper, style);
            }
        }
        LaidOutBlockKind::Table { header, rows, .. } => {
            let font_family = style.body_font_family.first().map(|s| s.as_str());
            for row in header {
                for line in row {
                    if !line.styles.is_empty() {
                        populate_line_style_segments(line, shaper, font_family);
                    }
                }
            }
            for row in rows {
                for cell in row {
                    for line in cell {
                        if !line.styles.is_empty() {
                            populate_line_style_segments(line, shaper, font_family);
                        }
                    }
                }
            }
        }
        LaidOutBlockKind::MetadataBlock { lines } => {
            let font_family = style.body_font_family.first().map(|s| s.as_str());
            for line in lines {
                if !line.styles.is_empty() {
                    populate_line_style_segments(line, shaper, font_family);
                }
            }
        }
        LaidOutBlockKind::HorizontalRule => {}
    }
}

fn populate_line_style_segments(
    line: &mut LaidOutLine,
    shaper: &mut Shaper,
    font_family: Option<&str>,
) {
    line.style_segments = compute_style_segments(
        &line.text,
        &line.styles,
        line.font_size,
        line.font_weight,
        Some(shaper),
        &line.shaped,
        font_family,
    );
    line.shaped = shape_styled_run(
        &line.text,
        &line.styles,
        line.font_size,
        line.font_weight,
        font_family,
        shaper,
    );
}

fn shape_styled_run(
    text: &str,
    styles: &[crate::builder::StyleSpan],
    font_size: f32,
    base_weight: shaping::Weight,
    font_family: Option<&str>,
    shaper: &mut Shaper,
) -> Option<shaping::ShapedRun> {
    let old_size = shaper.font_size();
    let old_weight = shaper.font_weight();
    let old_style = shaper.font_style();
    let old_family = shaper.font_family().map(|family| family.to_string());

    shaper.set_font_size(font_size);
    shaper.set_font_family(font_family);

    let shaped = (|| {
        let mut clusters = Vec::new();
        let mut width = 0.0;
        let mut cursor = 0usize;
        let base_shape =
            SegmentShape { weight: base_weight, style: shaping::Style::Normal, italic: false };

        for span in styles {
            let span_start = safe_byte_idx(text, span.start).max(cursor);
            let span_end = safe_byte_idx(text, (span.start + span.len).min(text.len()));
            if span_end <= span_start {
                continue;
            }

            let gap = shape_segment(text, cursor..span_start, base_shape, font_size, shaper)?;
            width += gap.width;
            clusters.extend(gap.clusters);

            let styled_shape = effective_segment_shape(base_weight, &span.style);
            let styled =
                shape_segment(text, span_start..span_end, styled_shape, font_size, shaper)?;
            width += styled.width;
            clusters.extend(styled.clusters);
            cursor = span_end;
        }

        let tail = shape_segment(text, cursor..text.len(), base_shape, font_size, shaper)?;
        width += tail.width;
        clusters.extend(tail.clusters);

        Some(shaping::ShapedRun { clusters, width })
    })();

    shaper.set_font_size(old_size);
    shaper.set_font_weight(old_weight);
    shaper.set_font_style(old_style);
    shaper.set_font_family(old_family.as_deref());

    shaped
}

fn shape_segment(
    text: &str,
    byte_range: std::ops::Range<usize>,
    shape: SegmentShape,
    font_size: f32,
    shaper: &mut Shaper,
) -> Option<shaping::ShapedRun> {
    if byte_range.is_empty() {
        return Some(shaping::ShapedRun { clusters: Vec::new(), width: 0.0 });
    }

    shaper.set_font_weight(shape.weight);
    shaper.set_font_style(shape.style);
    let mut shaped = shaper.shape(&text[byte_range.clone()]).ok()?;
    if shape.italic {
        let italic_advance = font_size * ITALIC_VISUAL_WIDTH_EXTRA_RATIO;
        if let Some(cluster) = shaped.clusters.last_mut() {
            cluster.advance += italic_advance;
        }
        shaped.width += italic_advance;
    }
    let mut clusters = Vec::with_capacity(shaped.clusters.len());
    for mut cluster in shaped.clusters {
        cluster.byte_range.start += byte_range.start;
        cluster.byte_range.end += byte_range.start;
        clusters.push(cluster);
    }
    Some(shaping::ShapedRun { clusters, width: shaped.width })
}

fn width_at_byte(shaped: &Option<shaping::ShapedRun>, byte_pos: usize) -> Option<f32> {
    let shaped = shaped.as_ref()?;
    let mut cum_w = 0.0f32;
    for cluster in &shaped.clusters {
        if cluster.byte_range.start >= byte_pos {
            break;
        }
        if cluster.byte_range.end <= byte_pos {
            cum_w += cluster.advance;
        } else {
            // Partial overlap — approximate by proportional advance
            let overlap = (byte_pos - cluster.byte_range.start) as f32
                / cluster.byte_range.len().max(1) as f32;
            cum_w += cluster.advance * overlap;
            break;
        }
    }
    Some(cum_w)
}

/// Compute precise pixel positions for style spans within a text line.
/// Uses the shaper to measure actual glyph widths instead of estimating.
fn compute_style_segments(
    text: &str,
    styles: &[crate::builder::StyleSpan],
    font_size: f32,
    base_weight: shaping::Weight,
    shaper: Option<&mut Shaper>,
    pre_shaped: &Option<shaping::ShapedRun>,
    font_family: Option<&str>,
) -> Vec<StyleSegment> {
    if styles.is_empty() {
        return vec![];
    }
    let Some(shaper) = shaper else {
        return vec![];
    };
    let old_size = shaper.font_size();
    let old_weight = shaper.font_weight();
    let old_style = shaper.font_style();
    let old_family = shaper.font_family().map(|family| family.to_string());
    shaper.set_font_size(font_size);
    if let Some(family) = font_family {
        shaper.set_font_family(Some(family));
    }

    let mut segments = Vec::new();
    let mut cursor_x = 0.0;
    let mut last_end = 0usize;

    for span in styles {
        if span.start > last_end {
            cursor_x +=
                unstyled_width_between(text, last_end, span.start, base_weight, shaper, pre_shaped);
        }

        let span_end = (span.start + span.len).min(text.len());
        let segment = &text[safe_byte_idx(text, span.start)..safe_byte_idx(text, span_end)];
        let seg_w = styled_segment_width(segment, font_size, base_weight, &span.style, shaper);

        segments.push(StyleSegment {
            start: span.start,
            len: span.len,
            x_offset: cursor_x,
            width: seg_w,
            style: match &span.style {
                InlineStyle::Bold => InlineStyle::Bold,
                InlineStyle::Italic => InlineStyle::Italic,
                InlineStyle::Strikethrough => InlineStyle::Strikethrough,
                InlineStyle::InlineCode => InlineStyle::InlineCode,
                InlineStyle::Link { url } => InlineStyle::Link { url: url.clone() },
                InlineStyle::SourceMarker => InlineStyle::SourceMarker,
            },
        });
        cursor_x += seg_w;
        last_end = span_end;
    }
    shaper.set_font_size(old_size);
    shaper.set_font_weight(old_weight);
    shaper.set_font_style(old_style);
    shaper.set_font_family(old_family.as_deref());
    segments
}

fn unstyled_width_between(
    text: &str,
    start: usize,
    end: usize,
    base_weight: shaping::Weight,
    shaper: &mut Shaper,
    pre_shaped: &Option<shaping::ShapedRun>,
) -> f32 {
    if end <= start {
        return 0.0;
    }
    match (width_at_byte(pre_shaped, start), width_at_byte(pre_shaped, end)) {
        (Some(start_x), Some(end_x)) => (end_x - start_x).max(0.0),
        _ => {
            let segment = &text[safe_byte_idx(text, start)..safe_byte_idx(text, end)];
            shaper.set_font_weight(base_weight);
            shaper.set_font_style(shaping::Style::Normal);
            shaper.shape(segment).map(|run| run.width).unwrap_or(0.0)
        }
    }
}

fn styled_segment_width(
    text: &str,
    font_size: f32,
    base_weight: shaping::Weight,
    style: &InlineStyle,
    shaper: &mut Shaper,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }

    let shape = effective_segment_shape(base_weight, style);
    shaper.set_font_weight(shape.weight);
    shaper.set_font_style(shape.style);
    let mut width = shaper.shape(text).map(|run| run.width).unwrap_or(0.0);
    if shape.italic {
        width += font_size * ITALIC_VISUAL_WIDTH_EXTRA_RATIO;
    }
    width
}

/// Match the renderer: bold and italic supply their own font treatment, while
/// visual-only spans retain the enclosing line's weight.
fn effective_segment_shape(base_weight: shaping::Weight, style: &InlineStyle) -> SegmentShape {
    match style {
        InlineStyle::Bold => SegmentShape {
            weight: shaping::Weight::SEMIBOLD,
            style: shaping::Style::Normal,
            italic: false,
        },
        InlineStyle::Italic => SegmentShape {
            weight: shaping::Weight::NORMAL,
            style: shaping::Style::Normal,
            italic: true,
        },
        InlineStyle::SourceMarker
        | InlineStyle::Strikethrough
        | InlineStyle::InlineCode
        | InlineStyle::Link { .. } => {
            SegmentShape { weight: base_weight, style: shaping::Style::Normal, italic: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_line_no_shaper_returns_none() {
        let (shaped, layout) =
            shape_line("hello", 14.0, shaping::Weight::NORMAL, shaping::Style::Normal, None, None);
        assert!(shaped.is_none());
        assert!(layout.is_none());
    }

    #[test]
    fn shape_line_empty_text_returns_none() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let (shaped, layout) = shape_line(
            "",
            14.0,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            None,
            Some(&mut shaper),
        );
        assert!(shaped.is_none());
        assert!(layout.is_none());
    }

    #[test]
    fn shape_line_produces_valid_output() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let (shaped, layout) = shape_line(
            "hello",
            14.0,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            None,
            Some(&mut shaper),
        );
        assert!(shaped.is_some(), "should produce shaped run");
        assert!(layout.is_some(), "should produce text layout");
        let shaped = shaped.unwrap();
        let layout = layout.unwrap();
        assert!(!shaped.clusters.is_empty());
        assert_eq!(layout.text, "hello");
    }

    #[test]
    fn shape_line_restores_shaper_state() {
        let mut shaper = shaping::Shaper::new().unwrap();
        shaper.set_font_size(20.0);
        shaper.set_font_weight(shaping::Weight::BOLD);
        let _ = shape_line(
            "test",
            14.0,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            None,
            Some(&mut shaper),
        );
        assert_eq!(shaper.font_size(), 20.0);
        assert_eq!(shaper.font_weight(), shaping::Weight::BOLD);
    }

    #[test]
    fn compute_style_segments_restores_shaper_font_size() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let editor_font_size = 15.0;
        let heading_font_size = 27.0;
        shaper.set_font_size(editor_font_size);
        let text = "large heading";
        let styles = [crate::builder::StyleSpan {
            start: 0,
            len: text.len(),
            style: crate::builder::InlineStyle::Bold,
            source_range: 0..text.len(),
        }];

        let segments = super::compute_style_segments(
            text,
            &styles,
            heading_font_size,
            shaping::Weight::NORMAL,
            Some(&mut shaper),
            &None,
            None,
        );

        assert_eq!(segments.len(), 1);
        assert_eq!(shaper.font_size(), editor_font_size);
    }

    #[test]
    fn compute_style_segments_uses_styled_width_for_bold_tail_position() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let font_size = 16.0;
        let text = "Centered slogan page:";
        let bold_text = "Centered slogan page";
        let styles = [crate::builder::StyleSpan {
            start: 0,
            len: bold_text.len(),
            style: crate::builder::InlineStyle::Bold,
            source_range: 0..bold_text.len(),
        }];

        let segments = super::compute_style_segments(
            text,
            &styles,
            font_size,
            shaping::Weight::NORMAL,
            Some(&mut shaper),
            &None,
            None,
        );

        shaper.set_font_size(font_size);
        shaper.set_font_weight(shaping::Weight::SEMIBOLD);
        let bold_width = shaper.shape(bold_text).expect("bold segment should shape").width;
        assert_eq!(segments.len(), 1);
        assert!(
            (segments[0].width - bold_width).abs() < 0.1,
            "bold segment width {} should match styled width {bold_width}",
            segments[0].width
        );
    }

    #[test]
    fn compute_style_segments_measures_gap_after_bold_as_normal_text() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let font_size = 16.0;
        let text = "aa gap cc";
        let styles = [
            crate::builder::StyleSpan {
                start: 0,
                len: 2,
                style: crate::builder::InlineStyle::Bold,
                source_range: 0..2,
            },
            crate::builder::StyleSpan {
                start: 7,
                len: 2,
                style: crate::builder::InlineStyle::Bold,
                source_range: 7..9,
            },
        ];

        let segments = super::compute_style_segments(
            text,
            &styles,
            font_size,
            shaping::Weight::NORMAL,
            Some(&mut shaper),
            &None,
            None,
        );

        shaper.set_font_size(font_size);
        shaper.set_font_weight(shaping::Weight::SEMIBOLD);
        let first_bold_width = shaper.shape("aa").expect("first bold should shape").width;
        shaper.set_font_weight(shaping::Weight::NORMAL);
        let gap_width = shaper.shape(" gap ").expect("gap should shape").width;

        assert_eq!(segments.len(), 2);
        let expected_second_x = first_bold_width + gap_width;
        assert!(
            (segments[1].x_offset - expected_second_x).abs() < 0.1,
            "second segment x {} should be after normal gap at {expected_second_x}",
            segments[1].x_offset
        );
    }

    #[test]
    fn non_emphasis_styles_inherit_the_line_weight() {
        let non_emphasis_styles = [
            InlineStyle::SourceMarker,
            InlineStyle::Strikethrough,
            InlineStyle::InlineCode,
            InlineStyle::Link { url: "https://example.com".to_string() },
        ];

        for style in &non_emphasis_styles {
            let shape = super::effective_segment_shape(shaping::Weight::SEMIBOLD, style);
            assert_eq!(shape.weight, shaping::Weight::SEMIBOLD, "style {style:?}");
            assert_eq!(shape.style, shaping::Style::Normal, "style {style:?}");
            assert!(!shape.italic, "style {style:?}");
        }
    }

    #[test]
    fn width_at_byte_none_for_no_shaped() {
        assert!(super::width_at_byte(&None, 5).is_none());
    }

    #[test]
    fn width_at_byte_zero_for_start() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let (shaped, _) = shape_line(
            "hello",
            14.0,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            None,
            Some(&mut shaper),
        );
        let w = super::width_at_byte(&shaped, 0);
        assert_eq!(w, Some(0.0));
    }

    #[test]
    fn width_at_byte_full_text() {
        let mut shaper = shaping::Shaper::new().unwrap();
        let (shaped, _) = shape_line(
            "hello",
            14.0,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
            None,
            Some(&mut shaper),
        );
        let shaped_ref = shaped.as_ref().unwrap();
        let expected = shaped_ref.width;
        let w = super::width_at_byte(&shaped, 5); // "hello" is 5 bytes
        assert!(w.is_some());
        assert!((w.unwrap() - expected).abs() < 0.1, "full width should match shaped width");
    }
}
