//! Shared document search and selection operations for editor tabs.

use std::ops::Range;

use appkit_core::document::search::find_literal_matches;

use crate::tab_session::TabSessionMut;

/// Update the tab's literal matches and record the document generation used for the scan.
pub fn recompute_matches(tab: &mut TabSessionMut<'_>, match_case: bool) {
    tab.with_presentation(|document, presentation| {
        let search = &mut presentation.search_state;
        let matches = find_literal_matches(document, &search.query, match_case);
        search.update_matches(matches, document.generation());
    });
}

/// Put the caret at the match end and anchor the selection at the match start.
pub fn select_match(tab: &mut TabSessionMut<'_>, matched: &Range<usize>) {
    let document = tab.document_mut();
    document.set_cursor_offset_synced(matched.end);
    document.cursor_mut().selection_anchor = Some(matched.start);
}
