//! Immutable source and the syntax derived from its exact bytes.

use std::ops::{Deref, Range};
use std::sync::{Arc, OnceLock};

use crate::builder::MarkdownDoc;
use crate::parser::{ParsedMarkdown, parse_markdown, parse_shared_markdown};

pub(crate) type SourceSnapshot = Arc<MarkdownSource<'static>>;

/// A live snapshot owns its text. Standalone rule calls borrow text until parsing is needed.
pub(crate) struct MarkdownSource<'a> {
    text: SourceText<'a>,
    parsed: OnceLock<ParsedMarkdown>,
    structure: OnceLock<MarkdownDoc>,
}

enum SourceText<'a> {
    Shared(Arc<String>),
    Borrowed(&'a str),
}

impl MarkdownSource<'static> {
    pub(crate) fn snapshot(text: String) -> SourceSnapshot {
        Arc::new(Self::owned(text))
    }

    pub(crate) fn owned(text: String) -> Self {
        Self::new(SourceText::Shared(Arc::new(text)))
    }

    pub(crate) fn shared_text(&self) -> Arc<String> {
        match &self.text {
            SourceText::Shared(text) => Arc::clone(text),
            SourceText::Borrowed(text) => Arc::new((*text).to_owned()),
        }
    }
}

impl<'a> MarkdownSource<'a> {
    fn new(text: SourceText<'a>) -> Self {
        Self { text, parsed: OnceLock::new(), structure: OnceLock::new() }
    }

    pub(crate) fn borrowed(text: &'a str) -> Self {
        Self::new(SourceText::Borrowed(text))
    }

    pub(crate) fn as_str(&self) -> &str {
        match &self.text {
            SourceText::Shared(text) => text.as_str(),
            SourceText::Borrowed(text) => text,
        }
    }

    pub(crate) fn parsed(&self) -> &ParsedMarkdown {
        self.parsed.get_or_init(|| match &self.text {
            SourceText::Shared(text) => parse_shared_markdown(Arc::clone(text)),
            SourceText::Borrowed(text) => parse_markdown(text),
        })
    }

    pub(crate) fn source_spacing_protected_ranges(&self) -> Arc<[Range<usize>]> {
        crate::typography::extract_prose_ranges(self.parsed())
            .protected_ranges()
            .iter()
            .map(|range| range.start..range.end)
            .collect()
    }

    pub(crate) fn structure(&self) -> &MarkdownDoc {
        self.structure.get_or_init(|| MarkdownDoc::build_structure(self.parsed()))
    }

    pub(crate) fn layout_document(&self) -> MarkdownDoc {
        // Only rules that need the unmodified tree keep it in the snapshot.
        self.structure.get().cloned().unwrap_or_else(|| MarkdownDoc::build_structure(self.parsed()))
    }
}

impl Deref for MarkdownSource<'_> {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}
