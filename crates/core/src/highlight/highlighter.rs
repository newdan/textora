//! LSH-based syntax highlighter, adapted from the original microsoft/edit crate.
//!
//! Wraps `lsh::runtime::Runtime` and provides line-by-line highlighting
//! against a `ReadableDocument`.

use lsh::runtime::{Highlight, Runtime, RuntimeState};
use stdext::arena::Arena;
use stdext::collections::BVec;

use crate::document::ReadableDocument;

use super::definitions::HighlightKind;
use super::definitions::{ASSEMBLY, CHARSETS, STRINGS};
use lsh::runtime::Language;

/// A line-oriented syntax highlighter backed by the LSH runtime.
pub struct Highlighter<'doc, D: ReadableDocument> {
    runtime: Runtime<'static, 'static, 'static>,
    doc: &'doc D,
}

impl<'doc, D: ReadableDocument> Highlighter<'doc, D> {
    /// Creates a new highlighter for the given document and language.
    pub fn new(doc: &'doc D, language: &'static Language) -> Self {
        Self { runtime: Runtime::new(&ASSEMBLY, &STRINGS, &CHARSETS, language.entrypoint), doc }
    }

    /// Returns a snapshot of the current runtime state.
    ///
    /// The snapshot can later be restored via [`restore`](Self::restore) to resume
    /// highlighting from the point where the snapshot was taken.
    pub fn snapshot(&self) -> RuntimeState {
        self.runtime.snapshot()
    }

    /// Restores the runtime to a previously captured state.
    pub fn restore(&mut self, state: &RuntimeState) {
        self.runtime.restore(state);
    }

    /// Highlights the given logical line and returns a list of highlight spans.
    pub fn parse_line<'a>(
        &mut self,
        arena: &'a Arena,
        mut offset: usize,
    ) -> BVec<'a, Highlight<HighlightKind>> {
        let chunk = self.doc.read_forward(offset);

        if let Some(pos) = chunk.iter().position(|&b| b == b'\n') {
            let line = &chunk[..pos];
            self.runtime.parse_next_line(arena, line)
        } else {
            let mut line_buf = Vec::new();
            line_buf.extend_from_slice(chunk);
            offset += chunk.len();

            loop {
                let next_chunk = self.doc.read_forward(offset);
                if next_chunk.is_empty() {
                    break;
                }
                if let Some(pos) = next_chunk.iter().position(|&b| b == b'\n') {
                    line_buf.extend_from_slice(&next_chunk[..pos]);
                    break;
                } else {
                    line_buf.extend_from_slice(next_chunk);
                    offset += next_chunk.len();
                }
            }
            self.runtime.parse_next_line(arena, &line_buf)
        }
    }
}
