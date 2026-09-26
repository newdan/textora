use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use ui::core::paint::RasterImage;

use crate::builder::{InlineStyle, StyleSpan};
use crate::projection::ProjectedText;

#[derive(Clone, Debug)]
pub(crate) struct EmbeddedImageEntry {
    pub source_range: Range<usize>,
    pub image: Arc<RasterImage>,
    pub baseline: f32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct EmbeddedRegistry {
    entries: BTreeMap<usize, EmbeddedImageEntry>,
}

impl EmbeddedRegistry {
    pub(crate) fn register(
        &mut self,
        source_range: Range<usize>,
        image: Arc<RasterImage>,
        baseline: f32,
    ) {
        self.entries
            .insert(source_range.start, EmbeddedImageEntry { source_range, image, baseline });
    }

    pub(crate) fn image_for(&self, source_range: &Range<usize>) -> Option<&EmbeddedImageEntry> {
        self.entries.get(&source_range.start).filter(|entry| entry.source_range == *source_range)
    }

    pub(crate) fn remove_source_range(&mut self, source_range: &Range<usize>) {
        self.entries.retain(|_, entry| {
            entry.source_range.end <= source_range.start
                || entry.source_range.start >= source_range.end
        });
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
    }
}

pub(crate) fn color_to_rgba(color: [f32; 4]) -> [u8; 4] {
    color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub(crate) fn expose_failed_math(
    projected: &mut ProjectedText,
    styles: &mut Vec<StyleSpan>,
    document: &dyn core::document::DocView,
    font_size: f32,
    foreground: [f32; 4],
    force_source: bool,
    selection_range: Option<&Range<usize>>,
) {
    for index in (0..styles.len()).rev() {
        let span = &styles[index];
        if span.style != InlineStyle::Math {
            continue;
        }
        let spelling = document.doc_text_in_range(span.source_range.clone());
        let expression = spelling.strip_prefix('$').and_then(|body| body.strip_suffix('$'));
        let selected = selection_range.is_some_and(|range| {
            range.start < span.source_range.end && span.source_range.start < range.end
        });
        let renderable = !force_source
            && !selected
            && expression.is_some_and(|expression| {
                crate::embedded::render_embedded(crate::embedded::EmbeddedRequest {
                    kind: crate::embedded::EmbeddedKind::InlineMath,
                    source: expression,
                    font_size,
                    foreground: color_to_rgba(foreground),
                    background: [0, 0, 0, 0],
                })
                .is_ok()
            });
        if renderable {
            continue;
        }
        let span = styles.remove(index);
        let start_grapheme =
            crate::grapheme_map::grapheme_index_at_byte(&projected.text, span.start);
        let end_grapheme =
            crate::grapheme_map::grapheme_index_at_byte(&projected.text, span.start + span.len);
        *projected = projected.clone().replace_graphemes_with_direct(
            start_grapheme,
            end_grapheme,
            &spelling,
            span.source_range,
        );
        let length_change = spelling.len() as isize - span.len as isize;
        for later_style in styles.iter_mut().filter(|later| later.start > span.start) {
            later_style.start = later_style
                .start
                .checked_add_signed(length_change)
                .expect("source fallback grows or preserves visual style positions");
        }
    }
}
