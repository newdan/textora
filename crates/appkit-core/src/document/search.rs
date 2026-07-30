//! Search state management for per-document search.
//!
//! Holds the current query, options, match list, and active match index.
//! Used by the search bar UI and the highlight renderer.

use std::ops::Range;

use core::buffer::search::SearchOptions;

/// Holds the state of an active search for a single document.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// The current search query string.
    pub query: String,
    /// Search options (case sensitivity, whole word, regex).
    pub options: SearchOptions,
    /// All match byte ranges in the document. Empty if no query or no matches.
    pub matches: Vec<Range<usize>>,
    /// Index into `matches` of the currently active (highlighted) match.
    pub active_match_idx: usize,
    /// Whether the search panel is visible for this document.
    pub panel_visible: bool,
    /// Cursor byte position within the query string.
    pub cursor_byte_pos: usize,
    /// Generation counter of the TextBuffer at the time matches were computed.
    /// Used to detect stale matches after edits.
    pub buffer_generation: u32,
    /// Replace text (only relevant when replace mode is active).
    pub replace_query: String,
    /// Whether the replace sub-panel is expanded.
    pub replace_mode: bool,
    /// Keyboard focus: false = find input, true = replace input.
    pub focus_replace: bool,
}

impl SearchState {
    /// Returns true if there is an active search query.
    pub fn is_active(&self) -> bool {
        !self.query.is_empty()
    }

    /// Returns the currently active match range, if any.
    pub fn active_match(&self) -> Option<Range<usize>> {
        self.matches.get(self.active_match_idx).cloned()
    }

    /// Returns the total number of matches.
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Returns a display string like "3/15" for the match counter.
    pub fn match_counter_text(&self) -> String {
        if self.matches.is_empty() {
            if self.query.is_empty() { String::new() } else { "0/0".to_string() }
        } else {
            format!("{}/{}", self.active_match_idx + 1, self.matches.len())
        }
    }

    /// Moves to the next match, wrapping around.
    pub fn next_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.active_match_idx = (self.active_match_idx + 1) % self.matches.len();
    }

    /// Moves to the previous match, wrapping around.
    pub fn prev_match(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.active_match_idx =
            self.active_match_idx.checked_sub(1).unwrap_or(self.matches.len() - 1);
    }

    /// Clears all search state.
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.active_match_idx = 0;
        self.panel_visible = false;
        self.cursor_byte_pos = 0;
        self.replace_query.clear();
        self.replace_mode = false;
        self.focus_replace = false;
    }

    /// Two-stage Escape: if query is non-empty, clear query but keep panel open;
    /// if query is empty, close the panel entirely.
    pub fn dismiss_or_clear(&mut self) {
        if self.query.is_empty() {
            self.clear();
        } else {
            self.query.clear();
            self.matches.clear();
            self.active_match_idx = 0;
            self.cursor_byte_pos = 0;
            self.replace_query.clear();
        }
    }

    /// Toggle the replace sub-panel. When opening, focus the replace field.
    pub fn toggle_replace_mode(&mut self) {
        self.replace_mode = !self.replace_mode;
        if self.replace_mode {
            self.focus_replace = true;
        } else {
            self.focus_replace = false;
            self.replace_query.clear();
        }
    }

    /// Toggle regex search mode.
    pub fn toggle_regex(&mut self) {
        self.options.use_regex = !self.options.use_regex;
    }

    /// Set cursor_byte_pos clamped to a valid UTF-8 char boundary.
    /// Prevents panics when slicing multi-byte strings.
    pub fn set_cursor_byte_pos(&mut self, pos: usize) {
        let clamped = pos.min(self.query.len());
        // Snap to the nearest preceding char boundary
        self.cursor_byte_pos = self.query.floor_char_boundary(clamped);
    }

    /// Marks the search state as stale (edits happened after search).
    pub fn is_stale(&self, current_generation: u32) -> bool {
        self.is_active() && self.buffer_generation != current_generation
    }

    /// Sets query and options, resets matches. Caller must then call `update_matches`.
    pub fn set_query(&mut self, query: String, options: SearchOptions) {
        self.query = query;
        self.options = options;
    }

    /// Updates matches from a SIMD search result.
    pub fn update_matches(&mut self, matches: Vec<Range<usize>>, generation: u32) {
        self.matches = matches;
        self.buffer_generation = generation;
        if self.active_match_idx >= self.matches.len() && !self.matches.is_empty() {
            self.active_match_idx = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_counter_empty_query() {
        let state = SearchState::default();
        assert_eq!(state.match_counter_text(), "");
    }

    #[test]
    fn match_counter_no_matches() {
        let state = SearchState { query: "nothing".to_string(), ..Default::default() };
        assert_eq!(state.match_counter_text(), "0/0");
    }

    #[test]
    fn match_counter_has_multiple_matches() {
        let state = SearchState {
            query: "hello".to_string(),
            matches: vec![0..5, 10..15, 20..25],
            active_match_idx: 1,
            ..Default::default()
        };
        assert_eq!(state.match_counter_text(), "2/3");
    }

    #[test]
    fn next_match_wraps() {
        let mut state = SearchState {
            matches: vec![0..1, 1..2, 2..3],
            active_match_idx: 2,
            ..Default::default()
        };
        state.next_match();
        assert_eq!(state.active_match_idx, 0);
    }

    #[test]
    fn prev_match_wraps() {
        let mut state = SearchState { matches: vec![0..1, 1..2, 2..3], ..Default::default() };
        state.prev_match();
        assert_eq!(state.active_match_idx, 2);
    }

    #[test]
    fn is_stale_different_generation() {
        let state =
            SearchState { query: "needle".to_string(), buffer_generation: 5, ..Default::default() };
        assert!(state.is_stale(10));
    }

    #[test]
    fn is_stale_same_generation() {
        let state =
            SearchState { query: "needle".to_string(), buffer_generation: 5, ..Default::default() };
        assert!(!state.is_stale(5));
    }

    #[test]
    fn update_matches_resets_active_idx() {
        let mut state = SearchState {
            matches: vec![0..1, 1..2, 2..3, 3..4, 4..5],
            active_match_idx: 4,
            ..Default::default()
        };
        state.update_matches(vec![0..1, 1..2], 10);
        assert_eq!(state.active_match_idx, 0);
        assert_eq!(state.matches.len(), 2);
    }

    #[test]
    fn clear_resets_all_state() {
        let mut state = SearchState {
            query: "test".to_string(),
            options: SearchOptions { match_case: true, ..Default::default() },
            matches: vec![0..1, 1..2],
            active_match_idx: 1,
            panel_visible: true,
            buffer_generation: 42,
            cursor_byte_pos: 5,
            replace_query: String::new(),
            replace_mode: false,
            focus_replace: false,
        };
        state.clear();
        assert!(state.query.is_empty());
        assert!(state.matches.is_empty());
        assert_eq!(state.active_match_idx, 0);
        assert!(!state.panel_visible);
        assert_eq!(state.cursor_byte_pos, 0);
    }

    #[test]
    fn dismiss_or_clear_with_query_clears_but_keeps_panel() {
        let mut state = SearchState {
            query: "hello".to_string(),
            matches: std::iter::once(0..5).collect(),
            active_match_idx: 0,
            panel_visible: true,
            cursor_byte_pos: 3,
            ..Default::default()
        };
        state.dismiss_or_clear();
        assert!(state.query.is_empty());
        assert!(state.matches.is_empty());
        assert_eq!(state.cursor_byte_pos, 0);
        // Panel stays open
        assert!(state.panel_visible);
    }

    #[test]
    fn dismiss_or_clear_without_query_closes_panel() {
        let mut state = SearchState { panel_visible: true, ..Default::default() };
        state.dismiss_or_clear();
        assert!(!state.panel_visible);
    }

    #[test]
    fn set_cursor_byte_pos_snaps_to_char_boundary() {
        let mut state = SearchState { query: "中abc".to_string(), ..Default::default() }; // '中' = 3 bytes
        // pos=1 falls inside '中' (bytes 0..3), should snap to 0
        state.set_cursor_byte_pos(1);
        assert_eq!(state.cursor_byte_pos, 0);
        // pos=3 is at 'a', valid boundary
        state.set_cursor_byte_pos(3);
        assert_eq!(state.cursor_byte_pos, 3);
        // pos=100 exceeds len, clamped to len
        state.set_cursor_byte_pos(100);
        assert_eq!(state.cursor_byte_pos, state.query.len());
    }

    #[test]
    fn set_cursor_byte_pos_ascii_works_normally() {
        let mut state = SearchState { query: "hello".to_string(), ..Default::default() };
        state.set_cursor_byte_pos(3);
        assert_eq!(state.cursor_byte_pos, 3);
    }

    #[test]
    fn toggle_replace_mode_expands_and_focuses_replace() {
        let mut state = SearchState::default();
        state.toggle_replace_mode();
        assert!(state.replace_mode);
        assert!(state.focus_replace);
        state.toggle_replace_mode();
        assert!(!state.replace_mode);
        assert!(!state.focus_replace);
        assert!(state.replace_query.is_empty());
    }

    #[test]
    fn toggle_regex_flips_option() {
        let mut state = SearchState::default();
        assert!(!state.options.use_regex);
        state.toggle_regex();
        assert!(state.options.use_regex);
        state.toggle_regex();
        assert!(!state.options.use_regex);
    }

    #[test]
    fn clear_resets_replace_fields() {
        let mut state = SearchState {
            query: "test".to_string(),
            replace_query: "repl".to_string(),
            replace_mode: true,
            focus_replace: true,
            panel_visible: true,
            ..Default::default()
        };
        state.clear();
        assert!(state.replace_query.is_empty());
        assert!(!state.replace_mode);
        assert!(!state.focus_replace);
    }

    #[test]
    fn dismiss_or_clear_with_query_clears_replace_query() {
        let mut state = SearchState {
            query: "hello".to_string(),
            replace_query: "world".to_string(),
            replace_mode: true,
            focus_replace: true,
            panel_visible: true,
            ..Default::default()
        };
        state.dismiss_or_clear();
        assert!(state.query.is_empty());
        assert!(state.replace_query.is_empty());
        // replace_mode and focus_replace stay
        assert!(state.replace_mode);
        assert!(state.panel_visible);
    }
}
