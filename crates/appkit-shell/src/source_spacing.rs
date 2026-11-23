//! Refreshes the semantic spacing cache used by source editors.

use std::ops::Range;
use std::sync::Arc;

use appkit_core::document::DocumentModel;

use crate::document_presentation::SourceSpacingState;
use crate::tab_session::TabSessionMut;

const MARKDOWN_LANGUAGE_ID: &str = "markdown";
const MARKDOWN_FILE_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd"];
const SOURCE_SPACING_SEMANTIC_VERSION: u64 = 1;

/// Update Markdown source protection after changes to the source revision or document kind.
/// An optional plugin identity can identify otherwise unclassified Markdown documents.
pub fn refresh(
    tab: &mut TabSessionMut<'_>,
    markdown_plugin: Option<&str>,
    fallback: impl FnOnce(&DocumentModel) -> Arc<[Range<usize>]>,
) {
    let document = tab.document();
    let source_revision = document.content_revision();
    let is_markdown = markdown_plugin.is_some_and(|name| name == tab.plugin_name())
        || document_is_markdown_source(document);
    let spacing = tab.source_spacing();
    if spacing.source_revision == source_revision && spacing.is_markdown == is_markdown {
        return;
    }

    let protected_ranges = if is_markdown {
        tab.source_spacing_protected_ranges().unwrap_or_else(|| fallback(document))
    } else {
        Arc::default()
    };
    tab.set_source_spacing(SourceSpacingState {
        protected_ranges,
        semantic_version: SOURCE_SPACING_SEMANTIC_VERSION,
        source_revision,
        is_markdown,
    });
}

fn document_is_markdown_source(document: &DocumentModel) -> bool {
    document.language.is_some_and(|language| language.id.eq_ignore_ascii_case(MARKDOWN_LANGUAGE_ID))
        || document
            .file_path
            .as_deref()
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                MARKDOWN_FILE_EXTENSIONS
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use appkit_core::workspace::types::TabIdAllocator;

    use super::*;
    use crate::editor_plugin::EditorPlugin;
    use crate::tab_runtime::TabRuntime;

    #[test]
    fn source_spacing_reuses_fallback_for_the_same_revision_and_classification() {
        let mut document = DocumentModel::from_lines(&["text".to_owned()]);
        document.file_path = Some("draft.MD".into());
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        let mut tab =
            TabSessionMut::new(TabIdAllocator::new().allocate(), &mut document, &mut runtime);
        let calls = Cell::new(0);
        let fallback = |document: &DocumentModel| {
            calls.set(calls.get() + 1);
            std::iter::once(0..document.buffer_len()).collect()
        };

        refresh(&mut tab, None, fallback);
        assert!(tab.source_spacing().is_markdown);
        assert_eq!(&*tab.source_spacing().protected_ranges, std::slice::from_ref(&(0..4)));
        refresh(&mut tab, None, fallback);
        assert_eq!(calls.get(), 1);

        tab.document_mut().replace_range(0..4, "edited");
        refresh(&mut tab, None, fallback);
        assert_eq!(calls.get(), 2);
        assert_eq!(&*tab.source_spacing().protected_ranges, std::slice::from_ref(&(0..6)));

        tab.document_mut().file_path = None;
        refresh(&mut tab, None, fallback);
        assert!(!tab.source_spacing().is_markdown);
        assert!(tab.source_spacing().protected_ranges.is_empty());

        refresh(&mut tab, Some(ui::plugin::PLUGIN_EDITOR), fallback);
        assert!(tab.source_spacing().is_markdown);
        assert_eq!(calls.get(), 3);
    }
}
