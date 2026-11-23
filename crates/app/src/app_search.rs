//! Search panel: query handling, match finding, scroll-to-match.
//! Methods on `impl App`, extracted from app.rs.

use std::ops::Range;

use crate::app::App;
use crate::app_effect::AppEffect;
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
        let lh = self.ui_metrics().line_height;
        let mut active_idx = 0;
        let mut query = String::new();
        let mut match_case = false;

        if let Some(mut tab) = self.active_tab_session_mut()
            && let Some(range) = tab.search_state().active_match()
        {
            // Jumping to a match is user-driven caret movement: split any
            // ongoing undo coalescing run.
            tab.document.break_edit_merge();
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

            use textora_core::document::ReadableDocument;
            let chunk1 = tab.document.tb.gap_buffer().read_forward(0);
            let chunk2 = tab.document.tb.gap_buffer().read_forward(chunk1.len());

            let query_bytes = query.as_bytes();
            let search_fn: fn(&[u8], &[u8]) -> Vec<std::ops::Range<usize>> =
                if tab.search_state().options.match_case {
                    textora_core::buffer::simd_search::find_all
                } else {
                    textora_core::buffer::simd_search::find_all_case_insensitive_ascii
                };

            let mut matches = Vec::new();

            // Search first chunk
            if !chunk1.is_empty() {
                matches.extend(search_fn(query_bytes, chunk1));
            }

            // Search across the gap without overlapping a match in either chunk.
            let mut second_chunk_start = 0;
            if !chunk1.is_empty() && !chunk2.is_empty() && query_bytes.len() > 1 {
                let cross_len = query_bytes.len() - 1;
                let cross_start = chunk1
                    .len()
                    .saturating_sub(cross_len)
                    .max(matches.last().map_or(0, |range| range.end));
                let second_prefix = &chunk2[..cross_len.min(chunk2.len())];
                let mut cross_buf =
                    Vec::with_capacity(chunk1.len() - cross_start + second_prefix.len());
                cross_buf.extend_from_slice(&chunk1[cross_start..]);
                cross_buf.extend_from_slice(second_prefix);

                if let Some(range) = search_fn(query_bytes, &cross_buf).into_iter().next() {
                    let match_end = cross_start + range.end;
                    matches.push(cross_start + range.start..match_end);
                    second_chunk_start = match_end - chunk1.len();
                }
            }

            // Search second chunk
            let second_chunk_offset = chunk1.len() + second_chunk_start;
            matches.extend(
                search_fn(query_bytes, &chunk2[second_chunk_start..]).into_iter().map(|range| {
                    range.start + second_chunk_offset..range.end + second_chunk_offset
                }),
            );

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

    pub(crate) fn replace_search_matches(&mut self, scope: SearchReplaceScope) -> AppEffect {
        let Some(tab) = self.active_tab_session() else {
            return AppEffect::NONE;
        };
        let search = tab.search_state();
        if search.is_stale(tab.document.generation()) {
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
            &tab.document.full_text(),
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
