use std::collections::HashMap;

use appkit_core::workspace::types::TabId;
use ui::plugin::ViewPlugin;

use crate::canvas_viewport::CanvasViewportSession;
use crate::document_presentation::DocumentPresentation;
use crate::mindmap_style_panel::MindmapStylePanelSession;

const UNMEASURED_VISIBLE_ROWS: usize = 0;
const UNMEASURED_VIEWPORT_HEIGHT: f64 = 0.0;

pub struct TabRuntime {
    pub plugin: Box<dyn ViewPlugin>,
    pub(crate) cached_toggle_source: Option<Box<dyn ViewPlugin>>,
    pub(crate) toggle_source_scroll_y: f32,
    pub toc_visible: bool,
    pub presentation: DocumentPresentation,
    pub canvas_viewport: CanvasViewportSession,
    pub(crate) mindmap_style_panel: MindmapStylePanelSession,
}

impl TabRuntime {
    pub fn new(plugin: Box<dyn ViewPlugin>) -> Self {
        Self {
            plugin,
            cached_toggle_source: None,
            toggle_source_scroll_y: 0.0,
            toc_visible: false,
            presentation: DocumentPresentation::new(
                UNMEASURED_VISIBLE_ROWS,
                UNMEASURED_VIEWPORT_HEIGHT,
            ),
            canvas_viewport: CanvasViewportSession::default(),
            mindmap_style_panel: MindmapStylePanelSession::Closed,
        }
    }

    pub fn with_presentation(
        plugin: Box<dyn ViewPlugin>,
        presentation: DocumentPresentation,
    ) -> Self {
        Self { presentation, ..Self::new(plugin) }
    }
}

#[derive(Default)]
pub struct TabRuntimeStore {
    entries: HashMap<TabId, TabRuntime>,
}

impl TabRuntimeStore {
    pub fn insert(&mut self, id: TabId, runtime: TabRuntime) -> Option<TabRuntime> {
        self.entries.insert(id, runtime)
    }

    pub fn get(&self, id: TabId) -> Option<&TabRuntime> {
        self.entries.get(&id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut TabRuntime> {
        self.entries.get_mut(&id)
    }

    pub fn contains(&self, id: TabId) -> bool {
        self.entries.contains_key(&id)
    }

    pub fn ids(&self) -> std::collections::HashSet<TabId> {
        self.entries.keys().copied().collect()
    }

    pub fn remove(&mut self, id: TabId) -> Option<TabRuntime> {
        self.entries.remove(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::{TabRuntime, TabRuntimeStore};
    use crate::document_presentation::DocumentPresentation;
    use crate::editor_plugin::EditorPlugin;
    use appkit_core::document::DocumentModel;
    use appkit_core::workspace::types::TabIdAllocator;
    use core::buffer::TextBuffer;

    #[test]
    fn store_insert_get_and_remove_by_exact_tab_id() {
        let mut ids = TabIdAllocator::new();
        let first = ids.allocate();
        let second = ids.allocate();
        let mut store = TabRuntimeStore::default();

        store.insert(first, TabRuntime::new(Box::new(EditorPlugin::new())));
        store.insert(second, TabRuntime::new(Box::new(EditorPlugin::new())));

        assert!(store.get(first).is_some());
        assert!(store.get(second).is_some());
        assert!(store.contains(first));

        let removed = store.remove(first);
        assert!(removed.is_some());
        assert!(store.get(first).is_none());
        assert!(store.get(second).is_some());
    }

    #[test]
    fn runtime_presentation_can_be_rebuilt_without_changing_document_model() {
        let mut text_buffer = TextBuffer::new(false)
            .expect("TextBuffer creation should not require presentation state");
        text_buffer.write_raw(b"hello");
        let document = DocumentModel::new(text_buffer);
        let mut runtime = TabRuntime::new(Box::new(EditorPlugin::new()));

        runtime.presentation = DocumentPresentation::new(20, 240.0);

        assert_eq!(document.full_text(), "hello");
        assert_eq!(runtime.presentation.display.viewport.visible_rows, 20);
        assert_eq!(runtime.presentation.display.viewport.viewport_height, 240.0);
    }
}
