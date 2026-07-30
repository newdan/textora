//! Per-line highlight cache for incremental syntax highlighting.
//!
//! Stores previously computed highlights and associated runtime state so that
//! re-rendering the same region of a document doesn't require re-parsing
//! unchanged lines, and so that re-highlighting after an edit can resume
//! from the correct runtime state.

use lsh::runtime::{Highlight, RuntimeState};
use stdext::arena::Arena;

use crate::document::ReadableDocument;
use crate::helpers::CoordType;

use super::definitions::HighlightKind;
use super::highlighter::Highlighter;

/// Cache entry: highlights for a single logical line, plus the runtime state
/// *after* processing that line. The state is used as the starting point for
/// the next line.
struct CacheEntry {
    highlights: Vec<Highlight<HighlightKind>>,
    state: RuntimeState,
}

/// A cache of per-line syntax highlighting results.
pub struct HighlighterCache {
    entries: Vec<CacheEntry>,
    /// Line index of entries[0]. When the cache is reset on a large jump,
    /// this shifts to avoid allocating padding entries.
    base_line: usize,
}

impl HighlighterCache {
    pub fn new() -> Self {
        Self { entries: Vec::new(), base_line: 0 }
    }

    /// Discards all cached highlights.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.base_line = 0;
    }

    /// Invalidates cached highlights at and after the given line index.
    pub fn invalidate_from(&mut self, line: isize) {
        let line = if line < 0 { 0 } else { line as usize };
        if line <= self.base_line {
            self.entries.clear();
            self.base_line = line;
        } else {
            let idx = line - self.base_line;
            self.entries.truncate(idx);
        }
    }

    /// Returns the highlight spans for the given logical line, computing
    /// and caching them if necessary.
    ///
    /// When filling uncached lines, the runtime state is restored from the
    /// last cached line (if any) so that multi-line constructs like block
    /// comments are correctly tracked.
    pub fn parse_line<D: ReadableDocument, F>(
        &mut self,
        arena: &Arena,
        highlighter: &mut Highlighter<'_, D>,
        line_index: CoordType,
        mut get_offset: F,
    ) -> &[Highlight<HighlightKind>]
    where
        F: FnMut(CoordType) -> usize,
    {
        let idx = line_index as usize;

        // If the gap from the last cached line is too large (either forward
        // or backward), reset cache to avoid O(n) tree-sitter cascade.
        const MAX_HIGHLIGHT_GAP: usize = 200;
        let last_line = self.base_line + self.entries.len().saturating_sub(1);
        let gap = idx.abs_diff(last_line);
        if self.entries.is_empty() || idx < self.base_line || gap > MAX_HIGHLIGHT_GAP {
            // Reset: start fresh from the requested line
            self.entries.clear();
            self.base_line = idx;
            let offset = get_offset(line_index);
            let bvec = highlighter.parse_line(arena, offset);
            let highlights: Vec<_> = bvec.iter().cloned().collect();
            let state = highlighter.snapshot();
            self.entries.push(CacheEntry { highlights, state });
            return &self.entries[0].highlights;
        }

        // Restore runtime state from the last cached line, if any.
        if let Some(last) = self.entries.last() {
            highlighter.restore(&last.state);
        }

        // Fill the cache from the last known line up to the requested one.
        let start_line = self.base_line + self.entries.len();
        for line in start_line..=idx {
            let offset = get_offset(line as CoordType);
            let bvec = highlighter.parse_line(arena, offset);
            let highlights: Vec<_> = bvec.iter().cloned().collect();
            let state = highlighter.snapshot();
            self.entries.push(CacheEntry { highlights, state });
        }

        &self.entries[idx - self.base_line].highlights
    }
}

impl Default for HighlighterCache {
    fn default() -> Self {
        Self::new()
    }
}
