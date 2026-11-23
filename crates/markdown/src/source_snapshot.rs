//! Immutable source and the syntax derived from its exact bytes.

use std::ops::{Deref, Range};
use std::sync::{Arc, OnceLock};

use crate::builder::MarkdownDoc;
use crate::layout::source_line_map::SourceLineMap;
use crate::parser::{ParsedMarkdown, parse_markdown, parse_shared_markdown};

pub(crate) type SourceSnapshot = Arc<MarkdownSource<'static>>;

#[cfg(test)]
thread_local! {
    pub(crate) static SOURCE_LINE_MAP_BUILD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A live snapshot owns its text. Standalone rule calls borrow text until parsing is needed.
#[derive(Debug)]
pub(crate) struct MarkdownSource<'a> {
    text: SourceText<'a>,
    parsed: OnceLock<ParsedMarkdown>,
    structure: OnceLock<MarkdownDoc>,
    physical_lines: OnceLock<SourceLineMap>,
}

#[derive(Debug)]
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
}

impl<'a> MarkdownSource<'a> {
    fn new(text: SourceText<'a>) -> Self {
        Self {
            text,
            parsed: OnceLock::new(),
            structure: OnceLock::new(),
            physical_lines: OnceLock::new(),
        }
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

    pub(crate) fn physical_lines(&self) -> &SourceLineMap {
        self.physical_lines.get_or_init(|| {
            #[cfg(test)]
            SOURCE_LINE_MAP_BUILD_COUNT.with(|count| count.set(count.get() + 1));
            SourceLineMap::from_source(self.as_str())
        })
    }

    pub(crate) fn cached_physical_lines(&self) -> Option<&SourceLineMap> {
        self.physical_lines.get()
    }

    pub(crate) fn source_spacing_protected_ranges(&self) -> Arc<[Range<usize>]> {
        crate::typography::parsed_source_spacing_protected_ranges(self.parsed())
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
