//! Search highlight state for markdown preview.
//!
//! Encapsulates the search-match rectangle cache and highlight generation,
//! isolating interior mutability from [`crate::view::MarkdownView`].

use crate::layout::{FlatLine, grapheme_x};
use std::cell::{Cell, RefCell};
use textora_core::unicode::grapheme_byte_boundaries;
use ui::core::geom::Rect;
use ui::core::paint::{DrawCmd, DrawList};

/// Cached search match rectangles for global indexing and viewport culling.
///
/// The search view queries highlights through a shared reference.
pub(crate) struct SearchHighlightCache {
    query: RefCell<String>,
    case_sensitive: Cell<bool>,
    version: Cell<Option<(u32, u64)>>,
    rects: RefCell<Vec<Rect>>,
}

impl Default for SearchHighlightCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchHighlightCache {
    pub fn new() -> Self {
        Self {
            query: RefCell::new(String::new()),
            case_sensitive: Cell::new(false),
            version: Cell::new(None),
            rects: RefCell::new(Vec::new()),
        }
    }

    /// Incrementally update the search rect cache when query/case/generation changes.
    pub fn update_if_needed(
        &self,
        query: &str,
        case_sensitive: bool,
        version: (u32, u64),
        flat_lines: &[FlatLine],
    ) {
        if *self.query.borrow() == query
            && self.case_sensitive.get() == case_sensitive
            && self.version.get() == Some(version)
        {
            return;
        }

        *self.query.borrow_mut() = query.to_string();
        self.case_sensitive.set(case_sensitive);
        self.version.set(Some(version));

        let mut rects = self.rects.borrow_mut();
        rects.clear();
        if query.is_empty() {
            return;
        }

        for line in flat_lines {
            append_match_rectangles(&mut rects, line, query, case_sensitive);
        }
    }

    pub fn invalidate(&self) {
        self.version.set(None);
    }

    /// Generate [`DrawList`] commands for search match highlights.
    #[allow(clippy::too_many_arguments)]
    pub fn highlights(
        &self,
        scroll_y: f32,
        viewport_h: f32,
        offset_x: f32,
        offset_y: f32,
        active_match_idx: usize,
        match_color: [f32; 4],
        inactive_color: [f32; 4],
    ) -> DrawList {
        let mut dl = DrawList::new();
        let rects = self.rects.borrow();
        let active_idx = active_match_idx.min(rects.len().saturating_sub(1));

        for (idx, rect) in rects.iter().enumerate() {
            let rect_y = rect.y - scroll_y + offset_y;
            if rect_y + rect.h < offset_y || rect_y > offset_y + viewport_h {
                continue;
            }

            let color = if idx == active_idx { match_color } else { inactive_color };
            dl.cmds.push(DrawCmd::FillRect {
                rect: Rect::new(rect.x + offset_x, rect_y, rect.w, rect.h),
                color,
                radius: 2.0,
            });
        }
        dl
    }

    /// Compute the scroll position to bring the Nth search match into view.
    pub fn scroll_to(
        &self,
        active_match_idx: usize,
        viewport_h: f32,
        current_scroll_y: f32,
    ) -> f32 {
        let rects = self.rects.borrow();
        if rects.is_empty() {
            return current_scroll_y;
        }

        let target_idx = active_match_idx.min(rects.len() - 1);
        let rect = rects[target_idx];
        let pad = 40.0;

        if rect.y < current_scroll_y {
            (rect.y - pad).max(0.0)
        } else if rect.y + rect.h > current_scroll_y + viewport_h {
            (rect.y + rect.h - viewport_h + pad).max(0.0)
        } else {
            current_scroll_y
        }
    }
}

fn append_match_rectangles(
    rects: &mut Vec<Rect>,
    line: &FlatLine,
    query: &str,
    case_sensitive: bool,
) {
    let mut boundaries = None;
    let mut remaining = line.text.as_str();
    while let Some(first) = remaining.chars().next() {
        let Some(matched_bytes) = matching_prefix_bytes(remaining, query, case_sensitive) else {
            remaining = &remaining[first.len_utf8()..];
            continue;
        };

        let start_byte = line.text.len() - remaining.len();
        let end_byte = start_byte + matched_bytes;
        remaining = &remaining[matched_bytes..];

        let boundaries = boundaries.get_or_insert_with(|| grapheme_byte_boundaries(&line.text));
        let start = boundaries.partition_point(|&byte| byte <= start_byte) - 1;
        let end = boundaries.partition_point(|&byte| byte < end_byte);
        let x0 = grapheme_x(line, start);
        let x1 = grapheme_x(line, end);
        if x1 > x0 {
            rects.push(Rect::new(line.rect.x + x0, line.rect.y, x1 - x0, line.rect.h));
        }
    }
}

fn matching_prefix_bytes(candidate: &str, query: &str, case_sensitive: bool) -> Option<usize> {
    if case_sensitive {
        return candidate.starts_with(query).then_some(query.len());
    }

    let mut chars = candidate.chars();
    for expected in query.chars() {
        let actual = chars.next()?;
        if !actual.to_lowercase().eq(expected.to_lowercase()) {
            return None;
        }
    }
    Some(candidate.len() - chars.as_str().len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, y: f32) -> FlatLine {
        FlatLine {
            flat_idx: 0,
            rect: Rect::new(20.0, y, 400.0, 10.0),
            text: text.to_owned(),
            font_size: 10.0,
            is_code: false,
            shaped: None,
            requires_source_projection: false,
            source_projection: None,
            atomic_source_range: None,
        }
    }

    #[test]
    fn regression_search_uses_grapheme_indices_for_match_geometry() {
        let cache = SearchHighlightCache::new();
        let lines = [line("a\u{0301}👩🏽‍🔬 hit trailing", 0.0)];
        cache.update_if_needed("hit", true, (1, 1), &lines);
        let rects = cache.rects.borrow();

        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].x, lines[0].rect.x + grapheme_x(&lines[0], 3));
        assert_eq!(rects[0].w, grapheme_x(&lines[0], 6) - grapheme_x(&lines[0], 3));
    }

    #[test]
    fn regression_search_culls_against_the_offset_viewport() {
        let cache = SearchHighlightCache::new();
        let lines = [line("hit", 5.0), line("hit", 20.0), line("hit", 115.0)];
        cache.update_if_needed("hit", true, (1, 1), &lines);

        let commands = cache.highlights(20.0, 100.0, 0.0, 40.0, 0, [1.0; 4], [0.0; 4]);
        let positions = commands.cmds.iter().filter_map(|command| match command {
            DrawCmd::FillRect { rect, .. } => Some(rect.y),
            _ => None,
        });
        assert_eq!(positions.collect::<Vec<_>>(), [40.0, 135.0]);
    }

    #[test]
    fn search_rebuilds_match_geometry_for_a_new_generation() {
        let cache = SearchHighlightCache::new();
        cache.update_if_needed("hit", true, (1, 1), &[line("hit", 10.0)]);
        cache.update_if_needed("hit", true, (1, 2), &[line("hit", 50.0)]);

        assert_eq!(cache.rects.borrow()[0].y, 50.0);
    }
}
