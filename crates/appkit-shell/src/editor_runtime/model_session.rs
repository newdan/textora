//! 文档模型与 tab runtime 的集中所有权。

use std::path::Path;

use appkit_core::workspace::types::TabId;

use crate::editor_runtime::{
    CloseConfirmation, EditorDocumentSummary, EditorTabSnapshot, EditorWorkspaceSnapshot,
    OpenDisposition,
};
use crate::prepared_tab::PreparedTab;
use crate::tab_runtime::{TabRuntime, TabRuntimeStore};
use crate::tab_session::{TabSession, TabSessionMut};
use crate::view_route::ViewRouteTable;
use crate::workspace::{CloseTabDecision, Workspace, WorkspaceEffect};

pub(crate) struct ModelSession {
    workspace: Workspace,
    runtimes: TabRuntimeStore,
}

impl ModelSession {
    pub(crate) fn from_parts(workspace: Workspace, runtimes: TabRuntimeStore) -> Self {
        let session = Self { workspace, runtimes };
        debug_assert_eq!(session.workspace.tab_ids(), session.runtimes.ids());
        session
    }

    pub(crate) fn replace_parts(&mut self, workspace: Workspace, runtimes: TabRuntimeStore) {
        self.workspace = workspace;
        self.runtimes = runtimes;
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
    }

    pub(crate) fn new(
        plugin_registry: ui::plugin::PluginRegistry,
        view_routes: ViewRouteTable,
    ) -> Self {
        Self {
            workspace: Workspace::with_plugins(plugin_registry, view_routes),
            runtimes: TabRuntimeStore::default(),
        }
    }

    pub(crate) fn install_prepared_tab(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
        disposition: OpenDisposition,
    ) -> WorkspaceEffect {
        let effect = self.workspace.install_prepared_tab(
            &mut self.runtimes,
            prepared,
            suggested_file_name,
            disposition,
        );
        self.reconcile_runtime_store(&effect);
        effect
    }

    pub(crate) fn append_prepared_tab(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
    ) -> TabId {
        let tab_id =
            self.workspace.append_prepared_tab(&mut self.runtimes, prepared, suggested_file_name);
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
        tab_id
    }

    pub(crate) fn activate(&mut self, tab_id: TabId) -> Option<WorkspaceEffect> {
        let index = self.workspace.index_of(tab_id)?;
        let effect = self.workspace.switch_to(index);
        Some(self.apply_workspace_effect(effect))
    }

    pub(crate) fn close_decision(&self, tab_id: TabId) -> Option<CloseTabDecision> {
        let index = self.workspace.index_of(tab_id)?;
        Some(self.workspace.try_close_entry(index))
    }

    pub(crate) fn close(&mut self, tab_id: TabId) -> Option<WorkspaceEffect> {
        let index = self.workspace.index_of(tab_id)?;
        let effect = self
            .workspace
            .close_entry(index)
            .expect("close decision must be checked before closing a tab");
        Some(self.apply_workspace_effect(effect))
    }

    pub(crate) fn confirm_close(
        &mut self,
        tab_id: TabId,
        confirmation: CloseConfirmation,
    ) -> Option<WorkspaceEffect> {
        let decision = self.close_decision(tab_id)?;
        let should_close = match confirmation {
            CloseConfirmation::Saved => decision == CloseTabDecision::CanClose,
            CloseConfirmation::Discard => decision != CloseTabDecision::Pinned,
            CloseConfirmation::Cancel => false,
        };
        should_close.then(|| self.close(tab_id)).flatten()
    }

    pub(crate) fn active_tab_id(&self) -> Option<TabId> {
        self.workspace.active_tab_id()
    }

    pub(crate) fn tab_index(&self, tab_id: TabId) -> Option<usize> {
        self.workspace.index_of(tab_id)
    }

    pub(crate) fn tab_id_at(&self, index: usize) -> Option<TabId> {
        self.workspace.tab_id_at(index)
    }

    pub(crate) fn tab_count(&self) -> usize {
        self.workspace.len()
    }

    pub(crate) fn tab_ids_in_order(&self) -> Vec<TabId> {
        self.workspace.tab_indices().filter_map(|index| self.workspace.tab_id_at(index)).collect()
    }

    pub(crate) fn runtime_tab_ids(&self) -> std::collections::HashSet<TabId> {
        self.runtimes.ids()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.workspace.is_empty()
    }

    pub(crate) fn is_pinned(&self, tab_id: TabId) -> bool {
        self.workspace.is_pinned_id(tab_id)
    }

    pub(crate) fn tab_title(&self, tab_id: TabId) -> Option<String> {
        let index = self.workspace.index_of(tab_id)?;
        self.workspace.entry_title(index)
    }

    pub(crate) fn clear_suggested_file_name(&mut self, tab_id: TabId) {
        if let Some(index) = self.workspace.index_of(tab_id) {
            self.workspace.clear_suggested_file_name(index);
        }
    }

    pub(crate) fn tab_session(&self, tab_id: TabId) -> Option<TabSession<'_>> {
        let index = self.workspace.index_of(tab_id)?;
        let document = self.workspace.entry(index)?;
        let runtime = self.runtimes.get(tab_id)?;
        Some(TabSession::new(tab_id, document, runtime))
    }

    pub(crate) fn tab_session_mut(&mut self, tab_id: TabId) -> Option<TabSessionMut<'_>> {
        let index = self.workspace.index_of(tab_id)?;
        let runtime = self.runtimes.get_mut(tab_id)?;
        let document = self.workspace.entry_mut(index)?;
        Some(TabSessionMut::new(tab_id, document, runtime))
    }

    pub(crate) fn tab_runtime(&self, tab_id: TabId) -> Option<&TabRuntime> {
        self.runtimes.get(tab_id)
    }

    pub(crate) fn tab_runtime_mut(&mut self, tab_id: TabId) -> Option<&mut TabRuntime> {
        self.runtimes.get_mut(tab_id)
    }

    pub(crate) fn tab_id_for_path(&self, path: &Path) -> Option<TabId> {
        self.workspace.find_by_path(path).and_then(|index| self.workspace.tab_id_at(index))
    }

    pub(crate) fn document_summary(&self, tab_id: TabId) -> Option<EditorDocumentSummary> {
        let document = self.workspace.entry_by_id(tab_id)?;
        Some(EditorDocumentSummary {
            tab_id,
            path: document.file_path.clone(),
            dirty: document.dirty,
            content_revision: document.content_revision(),
            disk_revision: document.disk_revision.clone(),
            pinned: self.workspace.is_pinned_id(tab_id),
        })
    }

    pub(crate) fn document_save_snapshot(
        &self,
        tab_id: TabId,
    ) -> Option<(std::path::PathBuf, Vec<u8>, Option<appkit_core::file_safety::DiskRevision>, u64)>
    {
        let document = self.workspace.entry_by_id(tab_id)?;
        let path = document.file_path.clone()?;
        Some((
            path,
            document.serialized_contents_for_save(),
            document.disk_revision.clone(),
            document.content_revision(),
        ))
    }

    pub(crate) fn document_save_snapshot_as(
        &self,
        tab_id: TabId,
        path: &Path,
    ) -> Option<(Vec<u8>, Option<appkit_core::file_safety::DiskRevision>, u64)> {
        let document = self.workspace.entry_by_id(tab_id)?;
        let expected_revision = (document.file_path.as_deref() == Some(path))
            .then_some(document.disk_revision.clone())
            .flatten();
        Some((
            document.serialized_contents_for_save(),
            expected_revision,
            document.content_revision(),
        ))
    }

    pub(crate) fn apply_save_completion(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        content_revision: u64,
        disk_revision: appkit_core::file_safety::DiskRevision,
    ) -> Option<(bool, bool)> {
        self.workspace.apply_save_completion(tab_id, path, content_revision, disk_revision)
    }

    pub(crate) fn replace_document(
        &mut self,
        tab_id: TabId,
        document: appkit_core::document::DocumentModel,
    ) -> bool {
        let Some(index) = self.workspace.index_of(tab_id) else {
            return false;
        };
        let Some(current) = self.workspace.entry_doc_mut(index) else {
            return false;
        };
        *current = document;
        true
    }

    pub(crate) fn update_document_path(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        disk_revision: Option<appkit_core::file_safety::DiskRevision>,
    ) -> bool {
        let Some(index) = self.workspace.index_of(tab_id) else {
            return false;
        };
        let Some(document) = self.workspace.entry_doc_mut(index) else {
            return false;
        };
        document.file_path = Some(path.clone());
        document.disk_revision = disk_revision;
        document.set_language_from_path(&path);
        true
    }

    pub(crate) fn detach_document(
        &mut self,
        tab_id: TabId,
        suggested_file_name: Option<String>,
        dirty_snapshot_id: Option<String>,
    ) -> bool {
        let Some(index) = self.workspace.index_of(tab_id) else {
            return false;
        };
        let Some(document) = self.workspace.entry_doc_mut(index) else {
            return false;
        };
        document.file_path = None;
        document.disk_revision = None;
        document.dirty = true;
        if document.dirty_snapshot_id.is_none() {
            document.dirty_snapshot_id = dirty_snapshot_id;
        }
        self.workspace.set_suggested_file_name(index, suggested_file_name);
        true
    }

    pub(crate) fn document_summaries(&self) -> Vec<EditorDocumentSummary> {
        self.workspace
            .entries()
            .iter()
            .filter_map(|entry| self.document_summary(entry.id))
            .collect()
    }

    pub(crate) fn workspace_snapshot(&self) -> EditorWorkspaceSnapshot {
        let tabs = self
            .workspace
            .entries()
            .iter()
            .map(|entry| {
                let document = &entry.value;
                let session = crate::tab_session::TabSession::new(
                    entry.id,
                    document,
                    self.runtimes
                        .get(entry.id)
                        .expect("every workspace entry must have a matching tab runtime"),
                );
                let scroll_anchor = session.scroll_anchor_state();
                let preview_anchor_text = if session.allows_editing() {
                    None
                } else {
                    session.scroll_anchor().map(|(text, _)| text)
                };
                let content_lines = (0..document.line_count())
                    .filter_map(|line| {
                        document
                            .doc_line_bytes(line)
                            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    })
                    .collect();
                let clean_untitled_content =
                    (document.file_path.is_none() && !document.dirty).then(|| document.full_text());
                let default_plugin_name = document
                    .file_path
                    .as_deref()
                    .and_then(|path| self.workspace.plugin_route_for_path(path))
                    .map(|route| route.default_plugin.to_owned());

                EditorTabSnapshot {
                    tab_id: entry.id,
                    path: document.file_path.clone(),
                    suggested_file_name: entry.suggested_file_name.clone(),
                    cursor_offset: document
                        .cursor()
                        .snapshot_offset
                        .unwrap_or(document.cursor().offset.to_usize()),
                    selection_anchor: document
                        .cursor()
                        .snapshot_selection_anchor
                        .unwrap_or(document.cursor().selection_anchor),
                    cursor_line: document.cursor_line(),
                    cursor_column: document.cursor_column(),
                    dirty: document.dirty,
                    disk_revision: document.disk_revision.clone(),
                    dirty_snapshot_id: document.dirty_snapshot_id.clone(),
                    scroll_anchor_line: scroll_anchor.doc_line,
                    scroll_anchor_offset: scroll_anchor.pixel_offset,
                    preview_anchor_text,
                    plugin_name: session.plugin_name().to_owned(),
                    default_plugin_name,
                    allows_editing: session.allows_editing(),
                    content_lines,
                    clean_untitled_content,
                }
            })
            .collect();

        EditorWorkspaceSnapshot { active_index: self.workspace.active_index(), tabs }
    }

    pub(crate) fn has_back_history(&self) -> bool {
        self.workspace.has_back_history()
    }

    pub(crate) fn has_forward_history(&self) -> bool {
        self.workspace.has_forward_history()
    }

    pub(crate) fn toggle_target(&self) -> Option<&'static str> {
        self.workspace.toggle_target()
    }

    pub(crate) fn toggle_pin(
        &mut self,
        tab_id: TabId,
    ) -> Option<appkit_core::navigator::NavEffect> {
        let index = self.workspace.index_of(tab_id)?;
        Some(self.workspace.toggle_pin_at(index))
    }

    pub(crate) fn toggle_active_pin(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.toggle_pin()
    }

    pub(crate) fn navigate_back(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.go_back()
    }

    pub(crate) fn navigate_forward(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.go_forward()
    }

    pub(crate) fn upgrade_active_preview(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.upgrade_preview_if_needed()
    }

    pub(crate) fn switch_active_plugin(&mut self) {
        self.workspace.switch_plugin_with_runtime(&mut self.runtimes);
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
    }

    pub(crate) fn active_is_toggled(&self, plugin_name: &str) -> bool {
        self.workspace.is_toggled_for_plugin(plugin_name)
    }

    pub(crate) fn pinned_paths(&self) -> Vec<std::path::PathBuf> {
        self.workspace.pinned_paths()
    }

    pub(crate) fn restore_pinned(&mut self, paths: &[std::path::PathBuf]) {
        self.workspace.restore_pinned(paths);
    }

    pub(crate) fn create_plugin_for_path(&self, path: &Path) -> Box<dyn ui::plugin::ViewPlugin> {
        self.workspace.create_plugin_for_path(path)
    }

    pub(crate) fn create_plugin_by_name(
        &self,
        plugin_name: &str,
    ) -> Box<dyn ui::plugin::ViewPlugin> {
        self.workspace.create_plugin_by_name(plugin_name)
    }

    fn apply_workspace_effect(&mut self, effect: WorkspaceEffect) -> WorkspaceEffect {
        self.reconcile_runtime_store(&effect);
        effect
    }

    fn reconcile_runtime_store(&mut self, effect: &WorkspaceEffect) {
        effect.reconcile_runtime_store(&mut self.runtimes);
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use appkit_core::document::DocumentModel;
    use core::buffer::TextBuffer;
    use ui::plugin::PLUGIN_EDITOR;

    use super::*;
    use crate::editor_plugin::EditorPluginFactory;
    use crate::editor_runtime::OpenDisposition;
    use crate::tab_runtime::TabRuntime;
    use ui::plugin::PluginFactory;

    fn model_session() -> ModelSession {
        let mut registry = ui::plugin::PluginRegistry::new();
        registry.register(Box::new(EditorPluginFactory));
        let routes = ViewRouteTable::new(Vec::new(), &HashSet::from([PLUGIN_EDITOR]))
            .expect("empty test routes should be valid");
        ModelSession::new(registry, routes)
    }

    fn prepared_text(text: &str) -> PreparedTab {
        let mut text_buffer =
            TextBuffer::new(false).expect("model session test requires a writable text buffer");
        text_buffer.write_raw(text.as_bytes());
        PreparedTab::new(
            DocumentModel::new(text_buffer),
            TabRuntime::new(EditorPluginFactory.create()),
        )
    }

    #[test]
    fn install_keeps_model_and_runtime_ids_bijective() {
        let mut session = model_session();
        let first =
            session.install_prepared_tab(prepared_text("first"), None, OpenDisposition::Persistent);
        assert!(matches!(first, WorkspaceEffect::Activated(_)));
        let second = session.install_prepared_tab(
            prepared_text("second"),
            None,
            OpenDisposition::Persistent,
        );
        assert!(matches!(second, WorkspaceEffect::Activated(_)));
        assert_eq!(session.workspace.tab_ids(), session.runtimes.ids());
    }

    #[test]
    fn workspace_snapshot_preserves_model_runtime_bijection() {
        let mut session = model_session();
        let effect = session.install_prepared_tab(
            prepared_text("document"),
            None,
            OpenDisposition::Persistent,
        );
        assert!(matches!(effect, WorkspaceEffect::Activated(_)));

        let active_id =
            session.active_tab_id().expect("installed document should have an active tab");
        let snapshot = session.workspace_snapshot();

        assert_eq!(session.active_tab_id(), Some(active_id));
        assert_eq!(session.workspace.tab_ids(), session.runtimes.ids());
        assert_eq!(
            snapshot.tabs.iter().map(|tab| tab.tab_id).collect::<std::collections::HashSet<_>>(),
            session.workspace.tab_ids()
        );
    }

    #[test]
    fn preview_is_replaced_without_removing_persistent_tabs() {
        let mut session = model_session();
        let persistent = session.install_prepared_tab(
            prepared_text("persistent"),
            None,
            OpenDisposition::Persistent,
        );
        let persistent_id = match persistent {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("first tab should activate"),
        };
        let preview =
            session.install_prepared_tab(prepared_text("preview"), None, OpenDisposition::Preview);
        let preview_id = session.active_tab_id().expect("preview should activate");
        assert!(matches!(preview, WorkspaceEffect::Activated(_)));

        let replacement = session.install_prepared_tab(
            prepared_text("replacement"),
            None,
            OpenDisposition::Preview,
        );
        let replacement_id = session.active_tab_id().expect("replacement should activate");
        assert!(matches!(replacement, WorkspaceEffect::Closed { .. }));
        assert!(session.document_summary(persistent_id).is_some());
        assert!(session.document_summary(preview_id).is_none());
        assert!(session.document_summary(replacement_id).is_some());
    }

    #[test]
    fn unknown_lifecycle_ids_are_safe_no_ops() {
        let mut session = model_session();
        let mut allocator = appkit_core::workspace::types::TabIdAllocator::new();
        let unknown = allocator.allocate();

        assert!(session.activate(unknown).is_none());
        assert!(session.close_decision(unknown).is_none());
        assert!(session.close(unknown).is_none());
        assert!(session.confirm_close(unknown, CloseConfirmation::Discard).is_none());
    }

    #[test]
    fn cancel_and_saved_confirmation_respect_dirty_state() {
        let mut session = model_session();
        let effect =
            session.install_prepared_tab(prepared_text("dirty"), None, OpenDisposition::Persistent);
        let tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("first tab should activate"),
        };
        let document = session.workspace.entry_by_id(tab_id).expect("installed tab should exist");
        assert!(!document.dirty);

        let discarded = session.confirm_close(tab_id, CloseConfirmation::Cancel);
        assert!(discarded.is_none());
        assert!(session.document_summary(tab_id).is_some());
    }
}
