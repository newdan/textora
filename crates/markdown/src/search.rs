//! Search highlight state for markdown preview.
//!
//! Encapsulates the search-match rectangle cache and highlight generation,
//! isolating interior mutability from [`super::preview::MarkdownView`].

use crate::layout::{FlatLine, grapheme_x};
use std::cell::{Cell, RefCell};
use ui::core::geom::Rect;
use ui::core::paint::{DrawCmd, DrawList};

/// Cached search match rectangles for global indexing and viewport culling.
///
/// Uses interior mutability because [`ViewPlugin::query()`](`ui::plugin::ViewPlugin::query`)
/// takes `&self`, but the rect cache needs updating when the query changes.
pub(crate) struct SearchHighlightCache {
    query: RefCell<String>,
    case_sensitive: Cell<bool>,
    generation: Cell<u32>,
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
            generation: Cell::new(0),
            rects: RefCell::new(Vec::new()),
        }
    }

    /// Incrementally update the search rect cache when query/case/generation changes.
    pub fn update_if_needed(
        &self,
        query: &str,
        case_sensitive: bool,
        generation: u32,
        flat_lines: &[FlatLine],
    ) {
        if *self.query.borrow() == query
            && self.case_sensitive.get() == case_sensitive
            && self.generation.get() == generation
        {
            return;
        }

        *self.query.borrow_mut() = query.to_string();
        self.case_sensitive.set(case_sensitive);
        self.generation.set(generation);

        let q_chars: Vec<char> = query.chars().collect();
        if q_chars.is_empty() {
            self.rects.borrow_mut().clear();
            return;
        }

        let mut new_rects = Vec::new();
        for line in flat_lines {
            let chars: Vec<char> = line.text.chars().collect();
            let mut start_ch_idx = 0;

            while start_ch_idx + q_chars.len() <= chars.len() {
                let is_match = if case_sensitive {
                    chars[start_ch_idx..start_ch_idx + q_chars.len()] == q_chars[..]
                } else {
                    chars[start_ch_idx..start_ch_idx + q_chars.len()]
                        .iter()
                        .zip(&q_chars)
                        .all(|(c1, c2)| c1.to_lowercase().eq(c2.to_lowercase()))
                };

                if is_match {
                    let end_ch_idx = start_ch_idx + q_chars.len();
                    let x0 = line.rect.x + grapheme_x(line, start_ch_idx);
                    let x1 = line.rect.x + grapheme_x(line, end_ch_idx);
                    let w = (x1 - x0).max(0.0);
                    if w > 0.0 {
                        new_rects.push(Rect::new(x0, line.rect.y, w, line.rect.h));
                    }
                    start_ch_idx = end_ch_idx;
                } else {
                    start_ch_idx += 1;
                }
            }
        }
        *self.rects.borrow_mut() = new_rects;
    }

    /// Generate [`DrawList`] commands for search match highlights.
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
            if rect_y + rect.h < 0.0 || rect_y > viewport_h {
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
