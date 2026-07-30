//! Search panel: query handling, match finding, scroll-to-match.
//! Methods on `impl App`, extracted from app.rs.

use crate::app::App;
use ui::search_bar::SearchBarAction;

impl App {
    /// Apply a SearchBarAction to the active document's search state.
    /// Returns true if a search re-run is needed.
    pub(crate) fn apply_search_bar_action(&mut self, action: &SearchBarAction) -> bool {
        use SearchBarAction as SA;
        let needs_search = matches!(action, SA::QueryChanged(_) | SA::ToggleRegex);
        let should_scroll = matches!(action, SA::Next | SA::Prev);
        if let Some(mut tab) = self.active_tab_session_mut() {
            match action {
                SA::QueryChanged(q) => {
                    tab.search_state_mut().query = q.clone();
                    let query_len = tab.search_state().query.len();
                    tab.search_state_mut().set_cursor_byte_pos(query_len);
                    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
                }
                SA::ReplaceQueryChanged(q) => {
                    tab.search_state_mut().replace_query = q.clone();
                    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
                }
                SA::Next => tab.search_state_mut().next_match(),
                SA::Prev => tab.search_state_mut().prev_match(),
                SA::Close => {
                    tab.search_state_mut().clear();
                    self.ui_shell.focus_editor();
                }
                SA::DismissOrClear => {
                    tab.search_state_mut().dismiss_or_clear();
                    if !tab.search_state().panel_visible {
                        self.ui_shell.focus_editor();
                    }
                }
                SA::ToggleReplace => tab.search_state_mut().toggle_replace_mode(),
                SA::ToggleRegex => tab.search_state_mut().toggle_regex(),
                SA::Replace | SA::ReplaceAll => {}
                SA::FocusFind => tab.search_state_mut().focus_replace = false,
                SA::FocusReplace => tab.search_state_mut().focus_replace = true,
                SA::HoverChanged => {}
            }
            if should_scroll {
                self.scroll_to_active_match();
            }
        }
        needs_search
    }

    /// Scroll the viewport to center the currently active search match.
    pub(crate) fn scroll_to_active_match(&mut self) {
        let lh = self.ui_metrics().line_height;
        let mut active_idx = 0;
        let mut query = String::new();
        let mut match_case = false;

        if let Some(mut tab) = self.active_tab_session_mut()
            && let Some(range) = tab.search_state().active_match()
        {
            tab.document.set_cursor_offset_synced(range.start);
            tab.document.cursor_mut().selection_anchor = Some(range.start);
            tab.document.set_cursor_offset_synced(range.end);
            tab.ensure_cursor_visible(lh);

            active_idx = tab.search_state().active_match_idx;
            query = tab.search_state().query.clone();
            match_case = tab.search_state().options.match_case;
        }

        if let Some(mut tab) = self.active_tab_session_mut() {
            tab.send_message(ui::plugin::PluginMessage::ScrollToSearchMatch {
                query: query.clone(),
                match_case,
                active_idx,
            });
        }
    }

    /// Run the SIMD search on the active document and update its SearchState.
    pub(crate) fn perform_search_for_active_doc(&mut self) {
        if let Some(mut tab) = self.active_tab_session_mut() {
            let query = tab.search_state().query.clone();
            if query.is_empty() {
                let generation = tab.document.tb.gap_buffer().generation();
                let search_state = tab.search_state_mut();
                search_state.matches.clear();
                search_state.active_match_idx = 0;
                search_state.buffer_generation = generation;
                return;
            }

            use core::document::ReadableDocument;
            let chunk1 = tab.document.tb.gap_buffer().read_forward(0);
            let chunk2 = tab.document.tb.gap_buffer().read_forward(chunk1.len());

            let query_bytes = query.as_bytes();
            let search_fn: fn(&[u8], &[u8]) -> Vec<std::ops::Range<usize>> =
                if tab.search_state().options.match_case {
                    core::buffer::simd_search::find_all
                } else {
                    core::buffer::simd_search::find_all_case_insensitive_ascii
                };

            let mut matches = Vec::new();

            // Search first chunk
            if !chunk1.is_empty() {
                matches.extend(search_fn(query_bytes, chunk1));
            }

            // Search across the gap
            if !chunk1.is_empty() && !chunk2.is_empty() && query_bytes.len() > 1 {
                let cross_len = query_bytes.len() - 1;
                let take1 = cross_len.min(chunk1.len());
                let take2 = cross_len.min(chunk2.len());

                let mut cross_buf = Vec::with_capacity(take1 + take2);
                cross_buf.extend_from_slice(&chunk1[chunk1.len() - take1..]);
                cross_buf.extend_from_slice(&chunk2[..take2]);

                let cross_matches = search_fn(query_bytes, &cross_buf);
                for m in cross_matches {
                    let start_in_doc = chunk1.len() - take1 + m.start;
                    matches.push(start_in_doc..start_in_doc + query_bytes.len());
                }
            }

            // Search second chunk
            if !chunk2.is_empty() {
                let m2 = search_fn(query_bytes, chunk2);
                for m in m2 {
                    matches.push(m.start + chunk1.len()..m.end + chunk1.len());
                }
            }

            let generation = tab.document.tb.gap_buffer().generation();
            tab.search_state_mut().update_matches(matches, generation);

            // Jump to first match
            if tab.search_state().active_match().is_some() {
                let range = tab.search_state().matches[0].clone();
                tab.document.set_cursor_offset_synced(range.start);
                tab.document.cursor_mut().selection_anchor = Some(range.start);
                tab.document.set_cursor_offset_synced(range.end);
            }
        }
    }

    /// Replace the current active match with the replace_query text.
    /// After replacement, re-searches to update match list.
    pub(crate) fn perform_replace(&mut self) {
        let cursor_after = if let Some(tab) = self.active_tab_session_mut() {
            let Some(range) = tab.search_state().active_match() else {
                return;
            };
            let replacement =
                crate::search_escape::parse_escapes(&tab.search_state().replace_query);
            let after = range.start + replacement.len();
            tab.document.tb.replace_range(range, replacement.as_bytes());
            tab.document.dirty = true;
            Some(after)
        } else {
            None
        };
        // Re-search to update matches after the edit
        self.perform_search_for_active_doc();
        // Jump to the first match at or after the replacement position
        let lh = self.ui_metrics().line_height;
        if let (Some(after), Some(mut tab)) = (cursor_after, self.active_tab_session_mut())
            && let Some(idx) = tab.search_state().matches.iter().position(|r| r.start >= after)
        {
            tab.search_state_mut().active_match_idx = idx;
            let range = tab.search_state().matches[idx].clone();
            tab.document.set_cursor_offset_synced(range.start);
            tab.document.cursor_mut().selection_anchor = Some(range.start);
            tab.document.set_cursor_offset_synced(range.end);
            tab.ensure_cursor_visible(lh);
        }
    }

    /// Replace all matches with the replace_query text.
    pub(crate) fn perform_replace_all(&mut self) {
        if let Some(tab) = self.active_tab_session_mut() {
            let count = tab.search_state().matches.len();
            if count == 0 {
                return;
            }
            let replacement =
                crate::search_escape::parse_escapes(&tab.search_state().replace_query);
            let replacement_bytes = replacement.as_bytes();
            let matches: Vec<_> = tab.search_state().matches.clone();

            // Replace in reverse order to preserve offsets
            tab.document.tb.edit_begin_grouping();
            for range in matches.into_iter().rev() {
                tab.document.tb.replace_range(range, replacement_bytes);
            }
            tab.document.tb.edit_end_grouping();
            tab.document.dirty = true;
        }
        // Re-search to update matches
        self.perform_search_for_active_doc();
    }

    /// Get the count of matches for the current search (for confirmation dialog).
    pub(crate) fn current_match_count(&self) -> usize {
        self.active_tab_session().map(|tab| tab.search_state().matches.len()).unwrap_or(0)
    }
}
