use appkit_core::document::DocumentModel;

use crate::tab_runtime::TabRuntime;

pub(crate) fn placeholder_display_entries(
    document: &DocumentModel,
) -> Vec<crate::snap_tree::DisplayLineEntry> {
    (0..document.line_count())
        .map(|document_line| {
            let byte_offset = document.line_byte_offset(document_line).unwrap_or(0);
            let byte_length = document.line_byte_length(document_line).unwrap_or(0) as u32;
            crate::snap_tree::DisplayLineEntry::placeholder(byte_offset, byte_length, 0, 1)
        })
        .collect()
}

pub struct PreparedTab {
    pub document: DocumentModel,
    pub runtime: TabRuntime,
}

impl PreparedTab {
    pub fn new(document: DocumentModel, mut runtime: TabRuntime) -> Self {
        let display_map = &mut runtime.presentation.display.display_map;
        if display_map.entry_count() != document.line_count() {
            display_map.set_entries(placeholder_display_entries(&document));
        }
        Self { document, runtime }
    }
}

#[cfg(test)]
mod tests {
    use super::PreparedTab;
    use crate::editor_plugin::EditorPlugin;
    use crate::tab_runtime::TabRuntime;
    use appkit_core::document::DocumentModel;
    use core::buffer::TextBuffer;

    #[test]
    fn prepared_tab_preserves_document_and_runtime() {
        let mut text_buffer =
            TextBuffer::new(false).expect("prepared tab test requires a writable text buffer");
        text_buffer.write_raw(b"prepared\ndocument");
        let document = DocumentModel::new(text_buffer);
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        runtime.toc_visible = true;

        let prepared = PreparedTab::new(document, runtime);
        let PreparedTab { document, runtime } = prepared;

        assert_eq!(document.full_text(), "prepared\ndocument");
        assert_eq!(runtime.plugin.name(), "editor");
        assert!(runtime.toc_visible);
        assert_eq!(runtime.presentation.display.display_map.total_rows(), 2);
    }

    #[test]
    fn prepared_tab_preserves_a_complete_existing_display_map() {
        let mut text_buffer =
            TextBuffer::new(false).expect("prepared tab test requires a writable text buffer");
        text_buffer.write_raw(b"wrapped\ncontent");
        let document = DocumentModel::new(text_buffer);
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        let mut first_line = crate::snap_tree::DisplayLineEntry::placeholder(0, 7, 0, 1);
        first_line.visual_line_count = 3;
        let mut second_line = crate::snap_tree::DisplayLineEntry::placeholder(8, 7, 0, 1);
        second_line.visual_line_count = 2;
        runtime.presentation.display.display_map.set_entries(vec![first_line, second_line]);

        let prepared = PreparedTab::new(document, runtime);

        assert_eq!(prepared.runtime.presentation.display.display_map.total_rows(), 5);
    }
}
