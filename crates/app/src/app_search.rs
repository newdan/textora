//! Search panel: query handling, match finding, scroll-to-match.
//! Methods on `impl App`, extracted from app.rs.

use std::ops::Range;

use crate::app::App;
use crate::app_effect::AppEffect;
use appkit_shell::document_search::{recompute_matches, select_match};
use appkit_shell::editor_runtime::EditorNotification;
use textora_core::types::ByteIndex;
use textora_core::unicode::{CursorNav, grapheme_start_indices};
use ui::plugin::EditTransaction;
use ui::search_bar::SearchBarAction;

pub(crate) enum SearchReplaceScope {
    ActiveMatch,
    AllMatches,
}

fn search_replacement_transaction(
    source: &str,
    source_generation: u32,
    matches: &[Range<usize>],
    replacement: &str,
) -> Option<(EditTransaction, usize)> {
    let first_match = matches.first()?;
    let mut updated_source = String::with_capacity(source.len());
    let mut offset = 0;
    for range in matches {
        updated_source.push_str(source.get(offset..range.start)?);
        updated_source.push_str(replacement);
        offset = range.end;
    }
    let suffix = source.get(offset..)?;
    updated_source.push_str(suffix);

    let (original_span, updated_span) =
        common_grapheme_spans(source, &updated_source, first_match.start, suffix.len());
    let inserted_end = first_match.start + replacement.len();
    let caret = CursorNav::new(&updated_source).goto_byte(ByteIndex(inserted_end)).offset;
    let transaction = EditTransaction::replace(
        source_generation,
        original_span,
        updated_source[updated_span].to_owned(),
        caret.to_usize(),
    );
    Some((transaction, inserted_end))
}

fn grapheme_boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
    grapheme_start_indices(text).map(|(_, byte)| byte).chain(std::iter::once(text.len()))
}

/// Undo locates edits by grapheme position, so both versions need the same start boundary.
/// Including any joined clusters at the end also keeps the replacement relative to the same suffix.
fn common_grapheme_spans(
    original: &str,
    updated: &str,
    match_start: usize,
    suffix_len: usize,
) -> (Range<usize>, Range<usize>) {
    let mut original_boundaries = grapheme_boundaries(original).peekable();
    let mut updated_boundaries = grapheme_boundaries(updated).peekable();
    let mut original_start = 0;
    while let Some(boundary) = original_boundaries.next_if(|boundary| *boundary <= match_start) {
        original_start = boundary;
    }
    let mut start = 0;
    while let Some(boundary) = updated_boundaries.next_if(|boundary| *boundary <= original_start) {
        start = boundary;
    }

    let original_suffix_start = original.len() - suffix_len;
    let updated_suffix_start = updated.len() - suffix_len;
    let original_suffix_boundaries =
        original_boundaries.filter_map(|boundary| boundary.checked_sub(original_suffix_start));
    let updated_suffix_boundaries = std::iter::once(start)
        .chain(updated_boundaries)
        .filter_map(|boundary| boundary.checked_sub(updated_suffix_start));
    let extend_suffix = first_shared_offset(original_suffix_boundaries, updated_suffix_boundaries)
        .unwrap_or(suffix_len);
    (start..original_suffix_start + extend_suffix, start..updated_suffix_start + extend_suffix)
}

fn first_shared_offset(
    original: impl Iterator<Item = usize>,
    mut updated: impl Iterator<Item = usize>,
) -> Option<usize> {
    let mut updated_offset = updated.next()?;
    for offset in original {
        while updated_offset < offset {
            updated_offset = updated.next()?;
        }
        if updated_offset == offset {
            return Some(offset);
        }
    }
    None
}

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
        let line_height = self.ui_metrics().line_height;
        let Some(mut tab) = self.active_tab_session_mut() else {
            return;
        };
        let (query, match_case, active_idx) =
            if let Some(matched) = tab.search_state().active_match() {
                // Jumping to a match is user-driven caret movement and splits the undo run.
                tab.document_mut().break_edit_merge();
                tab.document_mut().set_cursor_offset_synced(matched.start);
                select_match(&mut tab, &matched);
                tab.ensure_cursor_visible(line_height);

                let search = tab.search_state();
                (search.query.clone(), search.options.match_case, search.active_match_idx)
            } else {
                (String::new(), false, 0)
            };
        tab.scroll_to_search_match(&query, match_case, active_idx);
    }

    /// Run the SIMD search on the active document and update its SearchState.
    pub(crate) fn perform_search_for_active_doc(&mut self) {
        let Some(mut tab) = self.active_tab_session_mut() else {
            return;
        };
        let match_case = tab.search_state().options.match_case;
        recompute_matches(&mut tab, match_case);
        let search = tab.search_state();
        if search.query.is_empty() {
            tab.search_state_mut().active_match_idx = 0;
            return;
        }
        if let Some(matched) = search.matches.first().cloned() {
            tab.document_mut().set_cursor_offset_synced(matched.start);
            select_match(&mut tab, &matched);
        }
    }

    pub(crate) fn replace_search_matches(&mut self, scope: SearchReplaceScope) -> AppEffect {
        let Some(tab) = self.active_tab_session() else {
            return AppEffect::NONE;
        };
        let search = tab.search_state();
        if search.is_stale(tab.document().generation()) {
            self.perform_search_for_active_doc();
            return AppEffect::REDRAW;
        }

        let matches = match scope {
            SearchReplaceScope::ActiveMatch => {
                search.matches.get(search.active_match_idx).map_or(&[][..], std::slice::from_ref)
            }
            SearchReplaceScope::AllMatches => search.matches.as_slice(),
        };
        if matches.is_empty() {
            return AppEffect::NONE;
        }
        let replacement = crate::search_escape::parse_escapes(&search.replace_query);
        let Some((transaction, inserted_end)) = search_replacement_transaction(
            &tab.document().full_text(),
            search.buffer_generation,
            matches,
            &replacement,
        ) else {
            return AppEffect::NONE;
        };
        let outcome = self.editor_runtime.apply_active_document_edit_transaction(transaction);
        if !outcome
            .notifications
            .iter()
            .any(|notification| matches!(notification, EditorNotification::ContentChanged { .. }))
        {
            return outcome.shell_effect;
        }
        self.reveal_match_after_replace(scope, inserted_end);
        let preview_upgrade = self.upgrade_active_editor_preview();
        self.finish_runtime_edit(outcome).merge(self.handle_nav_effect(preview_upgrade))
    }

    fn reveal_match_after_replace(&mut self, scope: SearchReplaceScope, cursor_after: usize) {
        self.perform_search_for_active_doc();
        if let Some(mut tab) = self.active_tab_session_mut()
            && !tab.search_state().matches.is_empty()
        {
            tab.search_state_mut().active_match_idx = match scope {
                SearchReplaceScope::ActiveMatch => tab
                    .search_state()
                    .matches
                    .iter()
                    .position(|range| range.start >= cursor_after)
                    .unwrap_or(0),
                SearchReplaceScope::AllMatches => 0,
            };
            self.scroll_to_active_match();
        }
    }
}
