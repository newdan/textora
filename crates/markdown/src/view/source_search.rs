//! Search geometry uses the same source projection as editing and selection.

use super::*;
use std::ops::Range;

impl MarkdownEditorView {
    pub(super) fn source_search_highlights(
        &self,
        matches: &[Range<usize>],
        active_idx: usize,
        match_color: [f32; 4],
        inactive_color: [f32; 4],
    ) -> DrawList {
        let mut highlights = DrawList::new();
        let engine = &self.engine;
        for (index, matched) in matches.iter().enumerate() {
            if matched.is_empty() || self.source.get(matched.clone()).is_none() {
                continue;
            }
            let Some(start) = self.search_position_for_byte(SourceByte::new(matched.start), false)
            else {
                continue;
            };
            let Some(end) = self.search_position_for_byte(SourceByte::new(matched.end), true)
            else {
                continue;
            };
            let selection = SelectionState { anchor: Some(start), cursor: Some(end) };
            let color = if index == active_idx { match_color } else { inactive_color };
            highlights.cmds.extend(
                selection
                    .highlights(
                        engine.flat_lines(),
                        engine.scroll_y,
                        engine.cached_offset_x,
                        engine.cached_offset_y,
                        engine.cached_dl_viewport.1,
                        color,
                    )
                    .cmds,
            );
        }
        engine.clip_code_highlights(highlights)
    }

    fn search_position_for_byte(&self, byte: SourceByte, is_end: bool) -> Option<ViewPos> {
        if let Some(position) = self.engine.selection_position_for_byte(byte, is_end) {
            return Some(position);
        }
        // Search can match only the combining mark or one scalar of an emoji.
        // Expand that match to its visible grapheme without changing source offsets.
        self.engine.flat_lines().iter().enumerate().find_map(|(line_index, line)| {
            let projection = line.source_projection.as_ref()?;
            projection.boundaries.windows(2).enumerate().find_map(|(grapheme, anchors)| {
                let range = anchors[0].byte..anchors[1].byte;
                (range.start < byte
                    && byte < range.end
                    && crate::grapheme_map::grapheme_count(
                        self.source.get(range.start.get()..range.end.get())?,
                    ) == 1)
                    .then(|| ViewPos::from_indices(line_index, grapheme + usize::from(is_end)))
            })
        })
    }

    pub(super) fn query_search_highlights(
        &self,
        query: &str,
        match_case: bool,
        use_regex: bool,
        active_idx: usize,
        match_color: [f32; 4],
        inactive_color: [f32; 4],
    ) -> DrawList {
        if query.is_empty() {
            return DrawList::new();
        }
        let pattern = if use_regex { query.to_owned() } else { regex::escape(query) };
        let Ok(expression) =
            regex::RegexBuilder::new(&pattern).case_insensitive(!match_case).build()
        else {
            return DrawList::new();
        };
        let matches =
            expression.find_iter(&self.source).map(|matched| matched.range()).collect::<Vec<_>>();
        self.source_search_highlights(&matches, active_idx, match_color, inactive_color)
    }
}

impl ui::plugin::SearchView for MarkdownEditorView {
    fn search_highlights(&self, request: ui::plugin::SearchHighlightsRequest<'_>) -> DrawList {
        if self.engine.replacement_preedit.is_some() {
            return DrawList::new();
        }
        self.query_search_highlights(
            request.query,
            request.match_case,
            request.use_regex,
            request.active_idx,
            request.match_color,
            request.inactive_color,
        )
    }

    fn source_search_highlights(
        &self,
        request: ui::plugin::SourceSearchHighlightsRequest<'_>,
    ) -> Option<DrawList> {
        if self.engine.replacement_preedit.is_some()
            || self.needs_source_update(request.source_generation)
        {
            return Some(DrawList::new());
        }
        Some(MarkdownEditorView::source_search_highlights(
            self,
            request.matches,
            request.active_idx,
            request.match_color,
            request.inactive_color,
        ))
    }
}

impl ui::plugin::SearchView for MarkdownView {
    fn search_highlights(&self, request: ui::plugin::SearchHighlightsRequest<'_>) -> DrawList {
        self.engine.search_highlights(
            request.query,
            request.match_case,
            request.use_regex,
            request.active_idx,
            request.match_color,
            request.inactive_color,
        )
    }

    fn scroll_to_search_match(&mut self, query: &str, match_case: bool, active_idx: usize) {
        self.engine.scroll_to_search_match(query, match_case, active_idx);
    }
}
