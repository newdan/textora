use appkit_core::document::DocumentModel;

use crate::tab_runtime::TabRuntime;

pub struct PreparedTab {
    pub document: DocumentModel,
    pub runtime: TabRuntime,
}

impl PreparedTab {
    pub fn new(document: DocumentModel, runtime: TabRuntime) -> Self {
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
        text_buffer.write_raw(b"prepared document");
        let document = DocumentModel::new(text_buffer);
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        runtime.toc_visible = true;

        let prepared = PreparedTab::new(document, runtime);
        let PreparedTab { document, runtime } = prepared;

        assert_eq!(document.full_text(), "prepared document");
        assert_eq!(runtime.plugin.name(), "editor");
        assert!(runtime.toc_visible);
    }
}
