//! Workspace module: headless document model management and tab aggregation.
//!
//! The Workspace owns all open document models and manages tab lifecycle:
//! opening, closing, switching, pinning, and navigation history.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use appkit_core::document::DocumentModel;
use appkit_core::navigator::{NavEffect, NavEntry, Navigator};
use appkit_core::workspace::model::{WorkspaceEntry, WorkspaceModel};
use appkit_core::workspace::types::TabId;

use crate::editor_plugin::EditorPlugin;
use crate::editor_runtime::OpenDisposition;
use crate::prepared_tab::PreparedTab;

#[cfg(test)]
use crate::tab_runtime::TabRuntime;
use crate::tab_runtime::TabRuntimeStore;
use crate::view_route::{ViewRouteRule, ViewRouteTable};
use ui::plugin::PLUGIN_EDITOR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum WorkspaceEffect {
    None,
    Activated(TabId),
    Closed { closed: TabId, activated: Option<TabId> },
}

impl WorkspaceEffect {
    pub fn nav_effect(self) -> NavEffect {
        match self {
            Self::None => NavEffect::None,
            Self::Activated(_) => NavEffect::ActiveChanged,
            Self::Closed { activated: Some(_), .. } => NavEffect::ActiveChanged,
            Self::Closed { activated: None, .. } => NavEffect::ItemsChanged,
        }
    }

    pub fn reconcile_runtime_store(&self, runtimes: &mut TabRuntimeStore) {
        if let Self::Closed { closed, .. } = self {
            let _ = runtimes.remove(*closed);
        }
    }
}

fn document_title(document: &DocumentModel, suggested_file_name: Option<&str>) -> String {
    document
        .file_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .or_else(|| suggested_file_name.map(str::to_owned))
        .unwrap_or_else(|| "untitled".to_owned())
}

/// 关闭 tab 前的决策 —— dirty 状态需要用户参与判断，不能作为错误返回。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTabDecision {
    /// 可以直接关闭。
    CanClose,
    /// 有未保存修改，需要弹窗让用户选择 Save/Discard/Cancel。
    NeedsSavePrompt,
    /// 该 tab 被固定，不可关闭。
    Pinned,
}

/// Manages all open document tabs and their navigation state.
pub struct Workspace {
    model: WorkspaceModel<DocumentModel>,
    entry_history: Vec<usize>,
    preview_index: Option<usize>,
    registry: ui::plugin::PluginRegistry,
    view_routes: ViewRouteTable,
}

impl Workspace {
    pub fn with_plugins(registry: ui::plugin::PluginRegistry, view_routes: ViewRouteTable) -> Self {
        Self {
            model: WorkspaceModel::new(),
            entry_history: Vec::new(),
            preview_index: None,
            registry,
            view_routes,
        }
    }

    fn default_plugin_for_path(&self, path: &Path) -> &'static str {
        self.plugin_route_for_path(path).map_or(PLUGIN_EDITOR, |route| route.default_plugin)
    }

    fn toggle_target_for_path(&self, path: &Path) -> Option<&'static str> {
        self.plugin_route_for_path(path).and_then(|route| route.toggle_target)
    }

    pub fn plugin_route_for_path(&self, path: &Path) -> Option<ViewRouteRule> {
        self.view_routes.resolve(path).copied()
    }

    pub fn create_plugin_by_name(&self, plugin_name: &str) -> Box<dyn ui::plugin::ViewPlugin> {
        self.registry.create_by_name(plugin_name, Box::new(EditorPlugin::new()))
    }

    pub fn create_plugin_for_path(&self, path: &Path) -> Box<dyn ui::plugin::ViewPlugin> {
        let plugin_name =
            self.plugin_route_for_path(path).map_or(PLUGIN_EDITOR, |route| route.default_plugin);
        self.create_plugin_by_name(plugin_name)
    }

    // ── Convenience accessors ──

    pub fn is_empty(&self) -> bool {
        self.model.is_empty()
    }

    pub fn len(&self) -> usize {
        self.model.len()
    }

    pub fn tab_indices(&self) -> std::ops::Range<usize> {
        0..self.model.len()
    }

    fn insert_prepared_tab(
        &mut self,
        runtimes: &mut TabRuntimeStore,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
    ) -> TabId {
        let PreparedTab { document, runtime } = prepared;
        let id = self.allocate_tab_id();
        assert!(
            !runtimes.contains(id),
            "newly allocated workspace tab ID must not already exist in its runtime store"
        );
        self.model.push_entry(WorkspaceEntry::new(id, document, suggested_file_name));
        let replaced_runtime = runtimes.insert(id, runtime);
        debug_assert!(
            replaced_runtime.is_none(),
            "runtime store precondition was checked before prepared tab insertion"
        );
        id
    }

    pub fn append_prepared_tab(
        &mut self,
        runtimes: &mut TabRuntimeStore,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
    ) -> TabId {
        self.insert_prepared_tab(runtimes, prepared, suggested_file_name)
    }

    pub fn open_prepared_tab(
        &mut self,
        runtimes: &mut TabRuntimeStore,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
    ) -> WorkspaceEffect {
        let id = self.insert_prepared_tab(runtimes, prepared, suggested_file_name);
        if self.len() == 1 {
            self.model.record_nav_step();
            return WorkspaceEffect::Activated(id);
        }
        let index = self
            .index_of(id)
            .expect("a prepared tab must remain installed until open activation completes");
        self.switch_to(index)
    }

    /// 按稳定的打开策略安装并激活一个 prepared tab。
    pub fn install_prepared_tab(
        &mut self,
        runtimes: &mut TabRuntimeStore,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
        disposition: OpenDisposition,
    ) -> WorkspaceEffect {
        let id = self.insert_prepared_tab(runtimes, prepared, suggested_file_name);
        let mut replaced_preview = None;

        if disposition == OpenDisposition::Preview
            && let Some(preview_index) = self.preview_index
            && let Some(preview_id) = self.model.id_at(preview_index)
            && preview_id != id
            && !self.model.is_pinned(preview_id)
            && !self.model.entry(preview_index).is_some_and(|entry| entry.value.dirty)
        {
            self.preview_index = None;
            let _ = self
                .close_entry_inner(preview_index)
                .expect("tracked preview must remain closable during replacement");
            replaced_preview = Some(preview_id);
        }

        let index = self
            .index_of(id)
            .expect("a newly installed tab must remain addressable by its stable ID");
        if disposition == OpenDisposition::Preview {
            self.preview_index = Some(index);
        }

        let activation = if self.len() == 1 {
            self.model.record_nav_step();
            WorkspaceEffect::Activated(id)
        } else {
            self.switch_to(index)
        };

        replaced_preview.map_or(activation, |closed| WorkspaceEffect::Closed {
            closed,
            activated: self.active_tab_id(),
        })
    }

    pub fn active_index(&self) -> usize {
        self.model.active_index()
    }

    pub fn active_entry(&self) -> Option<&DocumentModel> {
        self.model.active_entry().map(|e| &e.value)
    }

    pub fn active_entry_mut(&mut self) -> Option<&mut DocumentModel> {
        self.model.active_entry_mut().map(|e| &mut e.value)
    }

    pub fn active_doc(&self) -> Option<&DocumentModel> {
        self.active_entry()
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut DocumentModel> {
        self.active_entry_mut()
    }

    pub fn entry(&self, index: usize) -> Option<&DocumentModel> {
        self.model.entry(index).map(|e| &e.value)
    }

    pub fn entry_mut(&mut self, index: usize) -> Option<&mut DocumentModel> {
        self.model.entry_mut(index).map(|e| &mut e.value)
    }

    pub fn entry_doc(&self, index: usize) -> Option<&DocumentModel> {
        self.model.entry(index).map(|entry| &entry.value)
    }

    pub fn entry_doc_mut(&mut self, index: usize) -> Option<&mut DocumentModel> {
        self.model.entry_mut(index).map(|entry| &mut entry.value)
    }

    pub fn entry_title(&self, index: usize) -> Option<String> {
        self.model
            .entry(index)
            .map(|entry| document_title(&entry.value, entry.suggested_file_name.as_deref()))
    }

    pub fn clear_suggested_file_name(&mut self, index: usize) {
        if let Some(entry) = self.model.entry_mut(index) {
            entry.suggested_file_name = None;
        }
    }

    pub fn set_suggested_file_name(&mut self, index: usize, file_name: Option<String>) {
        if let Some(entry) = self.model.entry_mut(index) {
            entry.suggested_file_name = file_name;
        }
    }

    pub fn suggested_file_name(&self, index: usize) -> Option<&str> {
        self.model.entry(index)?.suggested_file_name.as_deref()
    }

    pub fn entries(&self) -> &[WorkspaceEntry<DocumentModel>] {
        self.model.entries()
    }

    /// Allocate a fresh tab ID for production open paths and test fixtures.
    fn allocate_tab_id(&mut self) -> TabId {
        self.model.allocate_id()
    }

    /// Return the stable ID of the tab at `index`, if one exists.
    pub fn tab_id_at(&self, index: usize) -> Option<TabId> {
        self.model.id_at(index)
    }

    /// Return a document by its stable tab ID.
    pub fn entry_by_id(&self, id: TabId) -> Option<&DocumentModel> {
        self.model.entry_by_id(id).map(|entry| &entry.value)
    }

    pub(crate) fn apply_save_completion(
        &mut self,
        tab_id: TabId,
        path: PathBuf,
        content_revision: u64,
        disk_revision: appkit_core::file_safety::DiskRevision,
    ) -> Option<(bool, bool)> {
        let entry = self.model.entry_by_id_mut(tab_id)?;
        let path_changed = entry.value.file_path.as_ref() != Some(&path);
        let clean = entry.value.apply_save_completion(path, content_revision, disk_revision);
        Some((clean, path_changed))
    }

    /// Return the active stable tab ID, if the workspace is non-empty.
    pub fn active_tab_id(&self) -> Option<TabId> {
        self.model.active_id()
    }

    /// Return the pinned state for a stable tab ID.
    pub fn is_pinned_id(&self, id: TabId) -> bool {
        self.model.is_pinned(id)
    }

    /// Return the current index of the tab with the given ID, if it is open.
    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.model.index_of(id)
    }

    pub fn tab_ids(&self) -> std::collections::HashSet<TabId> {
        (0..self.model.len()).filter_map(|index| self.model.id_at(index)).collect()
    }

    pub fn switch_plugin_with_runtime(
        &mut self,
        runtime_store: &mut crate::tab_runtime::TabRuntimeStore,
    ) {
        let active_idx = self.model.active_index();
        if active_idx >= self.model.len() {
            return;
        }
        let path = match self.model.entries()[active_idx].value.file_path.clone() {
            Some(path) => path,
            None => return,
        };
        let route = match self.view_routes.resolve(&path).copied() {
            Some(route) => route,
            None => return,
        };
        let default_plugin = route.default_plugin;
        let toggle_target = match route.toggle_target {
            Some(toggle_target) => toggle_target,
            None => return,
        };
        let tab_id = self.model.entries()[active_idx].id;
        let current_name = match runtime_store.get(tab_id) {
            Some(runtime) => runtime.plugin.name().to_string(),
            None => return,
        };
        let is_default = current_name == default_plugin;
        let plugin_name = if is_default { toggle_target } else { default_plugin };
        let replacement = self.registry.create_by_name(plugin_name, Box::new(EditorPlugin::new()));

        let Some(runtime) = runtime_store.get_mut(tab_id) else {
            return;
        };
        let document = &mut self.model.entries_mut()[active_idx].value;
        let mut session = crate::tab_session::TabSessionMut::new(tab_id, document, runtime);
        if is_default {
            session.swap_in_toggle_plugin(replacement);
        } else if !session.restore_cached_toggle_source() {
            session.replace_plugin(replacement);
        }
    }

    /// 返回当前 tab 的切换目标视图名。None 表示不可切换。
    pub fn toggle_target(&self) -> Option<&'static str> {
        let path = self.active_doc()?.file_path.as_deref()?;
        self.toggle_target_for_path(path)
    }

    /// 当前 tab 是否处于切换后的视图（非默认视图）。
    pub fn is_toggled_for_plugin(&self, plugin_name: &str) -> bool {
        let document = match self.active_doc() {
            Some(document) => document,
            None => return false,
        };
        let path = match document.file_path.as_deref() {
            Some(p) => p,
            None => return false,
        };
        plugin_name != self.default_plugin_for_path(path)
    }

    pub fn pinned_indices(&self) -> HashSet<usize> {
        self.model.pinned_ids().iter().filter_map(|&id| self.model.index_of(id)).collect()
    }

    /// Find a tab by file path. Returns the index if found.
    pub fn find_by_path(&self, path: &Path) -> Option<usize> {
        self.model.entries().iter().position(|e| e.value.file_path.as_deref() == Some(path))
    }

    // ── Navigation history ──

    /// Record current active tab into back-history before a navigation.
    pub fn record_nav_step(&mut self) {
        self.model.record_nav_step();
    }

    pub fn go_back(&mut self) -> NavEffect {
        self.model.go_back()
    }

    pub fn go_forward(&mut self) -> NavEffect {
        self.model.go_forward()
    }

    pub fn has_back_history(&self) -> bool {
        !self.model.back_history().is_empty()
    }

    pub fn has_forward_history(&self) -> bool {
        !self.model.forward_history().is_empty()
    }

    // ── Tab switching ──

    /// Switch to a tab by index.
    /// Returns the new target index if switched, or None if already active.
    pub fn switch_to(&mut self, index: usize) -> WorkspaceEffect {
        if index >= self.model.len() {
            return WorkspaceEffect::None;
        }
        let mut target = index;
        let mut preview_closed_id = None;
        // Auto-close preview tab when switching away
        if let Some(prev_idx) = self.preview_index
            && prev_idx != index
            && prev_idx < self.model.len()
            && !self.model.entry(prev_idx).map(|e| e.value.dirty).unwrap_or(true)
        {
            self.preview_index = None;
            if let Some(id) = self.model.id_at(prev_idx) {
                preview_closed_id = Some(id);
                let _ = self.model.close_by_id(id);
                if prev_idx < index {
                    target = index.saturating_sub(1);
                }
            }
        }
        let Some(id) = self.model.id_at(target) else { return WorkspaceEffect::None };
        let effect = self.model.switch_to(id);
        match (preview_closed_id, effect) {
            (Some(closed), _) => {
                WorkspaceEffect::Closed { closed, activated: self.model.active_id() }
            }
            (None, NavEffect::ActiveChanged) => WorkspaceEffect::Activated(id),
            (None, _) => WorkspaceEffect::None,
        }
    }

    /// Close a tab by index (internal, no pinned/dirty guards).
    fn close_entry_inner(&mut self, index: usize) -> Result<WorkspaceEffect, String> {
        if index >= self.model.len() {
            return Err("index out of range".into());
        }
        let Some(id) = self.model.id_at(index) else {
            return Err("index out of range".into());
        };

        let was_active = index == self.model.active_index();

        self.entry_history.push(index);
        let Some(effect) = self.model.close_by_id(id) else {
            return Err("tab not found".into());
        };

        if let Some(pi) = self.preview_index {
            if pi == effect.removed_index {
                self.preview_index = None;
            } else if pi > effect.removed_index {
                self.preview_index = Some(pi - 1);
            }
        }

        if was_active {
            Ok(WorkspaceEffect::Closed { closed: id, activated: self.model.active_id() })
        } else {
            Ok(WorkspaceEffect::Closed { closed: id, activated: None })
        }
    }

    /// 判断关闭 tab 前是否需要用户干预。
    pub fn try_close_entry(&self, index: usize) -> CloseTabDecision {
        if index >= self.model.len() {
            return CloseTabDecision::CanClose; // out of range, let close_entry handle
        }
        let Some(id) = self.model.id_at(index) else {
            return CloseTabDecision::CanClose;
        };
        if self.model.is_pinned(id) {
            return CloseTabDecision::Pinned;
        }
        if self.model.entry(index).map(|e| e.value.dirty).unwrap_or(false) {
            return CloseTabDecision::NeedsSavePrompt;
        }
        CloseTabDecision::CanClose
    }

    /// 关闭 tab（仅检查固定状态；dirty 应由调用方通过 try_close_entry 提前处理）。
    pub fn close_entry(&mut self, index: usize) -> Result<WorkspaceEffect, String> {
        if index >= self.model.len() {
            return Err("out of range".into());
        }
        let Some(id) = self.model.id_at(index) else {
            return Err("out of range".into());
        };
        if self.model.is_pinned(id) {
            return Err("pinned tab".into());
        }
        self.close_entry_inner(index)
    }

    // ── Pin management ──

    pub fn toggle_pin(&mut self) -> NavEffect {
        self.toggle_pin_at(self.model.active_index())
    }

    pub fn toggle_pin_at(&mut self, idx: usize) -> NavEffect {
        if let Some(id) = self.model.id_at(idx) {
            self.model.toggle_pin(id);
        }
        NavEffect::ItemsChanged
    }

    #[allow(dead_code)]
    pub fn is_pinned(&self, index: usize) -> bool {
        self.model.id_at(index).map(|id| self.model.is_pinned(id)).unwrap_or(false)
    }

    /// Pinned paths that should be saved to store.
    pub fn pinned_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for &id in self.model.pinned_ids() {
            if let Some(entry) = self.model.entry_by_id(id)
                && let Some(ref fp) = entry.value.file_path
            {
                paths.push(fp.clone());
            }
        }
        paths
    }

    /// Restore pinned tabs from paths
    pub fn restore_pinned(&mut self, paths: &[PathBuf]) {
        for path in paths {
            for entry in self.model.entries().iter() {
                if entry.value.file_path.as_ref() == Some(path) {
                    self.model.pin(entry.id);
                    break;
                }
            }
        }
    }

    /// Upgrade preview tab to regular tab. Returns true if upgraded.
    pub fn upgrade_preview_if_needed(&mut self) -> NavEffect {
        if let Some(idx) = self.preview_index
            && idx == self.model.active_index()
        {
            self.preview_index = None;
            return NavEffect::ItemsChanged;
        }
        NavEffect::None
    }
}

impl Navigator for Workspace {
    fn id(&self) -> &str {
        "builtin.files"
    }
    fn name(&self) -> &str {
        "Open Files"
    }

    fn items(&self) -> Vec<NavEntry> {
        self.model
            .entries()
            .iter()
            .enumerate()
            .map(|(i, e)| NavEntry {
                title: document_title(&e.value, e.suggested_file_name.as_deref()),
                file_path: e.value.file_path.clone(),
                is_dirty: e.value.dirty,
                pinned: self.is_pinned(i),
            })
            .collect()
    }

    fn active_index(&self) -> usize {
        self.model.active_index()
    }

    fn toggle_pin(&mut self, index: usize) -> NavEffect {
        self.toggle_pin_at(index)
    }
    fn is_pinned(&self, index: usize) -> bool {
        self.is_pinned(index)
    }
    fn pinned_indices(&self) -> HashSet<usize> {
        self.pinned_indices()
    }
    fn len(&self) -> usize {
        self.model.len()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_core::workspace::types::TabIdAllocator;
    use core::buffer::TextBuffer;

    use crate::editor_plugin::EditorPluginFactory;
    use crate::prepared_tab::PreparedTab;

    fn test_workspace() -> Workspace {
        let mut registry = ui::plugin::PluginRegistry::new();
        registry.register(Box::new(EditorPluginFactory));
        let registered_plugin_ids = HashSet::from([PLUGIN_EDITOR]);
        let routes = ViewRouteTable::new(Vec::new(), &registered_plugin_ids)
            .expect("empty controller test routes should be valid");
        Workspace::with_plugins(registry, routes)
    }

    fn document_from_lines(lines: Vec<String>) -> DocumentModel {
        let mut buffer =
            TextBuffer::new(false).expect("controller test buffer should be constructible");
        let content = lines.join("\n");
        if !content.is_empty() {
            buffer.write_raw(content.as_bytes());
        }
        buffer.mark_as_clean();
        DocumentModel::new(buffer)
    }

    fn document(text: &str) -> DocumentModel {
        document_from_lines(vec![text.to_owned()])
    }

    fn set_active_fixture(workspace: &mut Workspace, index: usize) {
        let id = workspace.tab_id_at(index).expect("fixture tab index should be valid");
        workspace.model.set_active_id(id);
    }

    fn runtime_at<'a>(
        workspace: &Workspace,
        runtimes: &'a TabRuntimeStore,
        index: usize,
    ) -> &'a TabRuntime {
        let id = workspace.tab_id_at(index).expect("test tab must exist");
        runtimes.get(id).expect("test tab must have a runtime")
    }

    fn prepared_tab(
        document: DocumentModel,
        plugin: Box<dyn ui::plugin::ViewPlugin>,
    ) -> PreparedTab {
        PreparedTab::new(document, TabRuntime::new(plugin))
    }

    fn prepared_editor_tab(text: &str) -> PreparedTab {
        prepared_tab(document(text), Box::new(EditorPlugin::new()))
    }

    fn append_document(
        workspace: &mut Workspace,
        runtimes: &mut TabRuntimeStore,
        document: DocumentModel,
        plugin: Box<dyn ui::plugin::ViewPlugin>,
    ) -> TabId {
        workspace.append_prepared_tab(runtimes, prepared_tab(document, plugin), None)
    }

    fn append_editor_tab(
        workspace: &mut Workspace,
        runtimes: &mut TabRuntimeStore,
        text: &str,
    ) -> TabId {
        workspace.append_prepared_tab(runtimes, prepared_editor_tab(text), None)
    }

    fn open_editor_tab(workspace: &mut Workspace, runtimes: &mut TabRuntimeStore, text: &str) {
        let effect = workspace.open_prepared_tab(runtimes, prepared_editor_tab(text), None);
        effect.reconcile_runtime_store(runtimes);
    }

    // ── CloseTabDecision & try_close_entry ──

    fn make_dirty_ws() -> (Workspace, TabRuntimeStore) {
        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        append_editor_tab(&mut ws, &mut runtimes, "");
        append_editor_tab(&mut ws, &mut runtimes, "");
        if let Some(entry) = ws.entry_mut(0) {
            entry.dirty = true;
            entry.file_path = Some(std::path::PathBuf::from("/test/file.txt"));
        }
        (ws, runtimes)
    }

    #[test]
    fn try_close_clean_tab_returns_can_close() {
        let (ws, _runtimes) = make_dirty_ws();
        // tab 1 is clean
        assert_eq!(ws.try_close_entry(1), CloseTabDecision::CanClose);
    }

    #[test]
    fn try_close_dirty_tab_returns_needs_save_prompt() {
        let (ws, _runtimes) = make_dirty_ws();
        // tab 0 is dirty
        assert_eq!(ws.try_close_entry(0), CloseTabDecision::NeedsSavePrompt);
    }

    #[test]
    fn try_close_pinned_tab_returns_pinned() {
        let (mut ws, _runtimes) = make_dirty_ws();
        ws.toggle_pin_at(0);
        // tab 0 is dirty AND pinned → Pinned takes priority
        assert_eq!(ws.try_close_entry(0), CloseTabDecision::Pinned);
    }

    #[test]
    fn try_close_out_of_range_returns_can_close() {
        let (ws, _runtimes) = make_dirty_ws();
        assert_eq!(ws.try_close_entry(999), CloseTabDecision::CanClose);
    }

    #[test]
    fn close_entry_allows_dirty() {
        let (mut ws, _runtimes) = make_dirty_ws();
        // Previously returned Err("unsaved changes"); now should succeed
        assert!(ws.close_entry(0).is_ok());
        assert_eq!(ws.len(), 1);
    }

    #[test]
    fn close_entry_rejects_pinned() {
        let (mut ws, _runtimes) = make_dirty_ws();
        ws.toggle_pin_at(0);
        assert!(ws.close_entry(0).is_err());
        assert_eq!(ws.len(), 2); // still there
    }

    #[test]
    fn close_entry_rejects_out_of_range() {
        let (mut ws, _runtimes) = make_dirty_ws();
        assert!(ws.close_entry(999).is_err());
        assert_eq!(ws.len(), 2);
    }

    // ── Tab identity (TabId) ──

    #[test]
    fn append_prepared_tab_installs_matching_model_and_runtime_ids() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        let first =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("first"), None);
        let mut prepared_second = prepared_editor_tab("second document");
        prepared_second.runtime.toc_visible = true;
        let second = workspace.append_prepared_tab(&mut runtimes, prepared_second, None);
        let third =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("third"), None);

        assert_eq!(workspace.tab_ids(), runtimes.ids());
        assert_eq!(workspace.tab_id_at(0), Some(first));
        assert_eq!(workspace.tab_id_at(1), Some(second));
        assert_eq!(workspace.tab_id_at(2), Some(third));
        let second_index =
            workspace.index_of(second).expect("second prepared document must be installed");
        assert_eq!(
            workspace.entry(second_index).expect("second prepared document must exist").full_text(),
            "second document"
        );
        let second_runtime =
            runtimes.get(second).expect("the same tab ID must address the prepared runtime");
        assert_eq!(second_runtime.plugin.name(), PLUGIN_EDITOR);
        assert!(second_runtime.toc_visible);

        let effect = workspace.close_entry(1).expect("middle prepared tab should close");
        effect.reconcile_runtime_store(&mut runtimes);

        assert_eq!(workspace.tab_ids(), runtimes.ids());
        assert_eq!(workspace.tab_id_at(0), Some(first));
        assert_eq!(workspace.tab_id_at(1), Some(third));
        assert!(!runtimes.contains(second));
    }

    #[test]
    fn append_prepared_tab_preserves_suggested_file_name() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        let id = workspace.append_prepared_tab(
            &mut runtimes,
            prepared_editor_tab("# prepared"),
            Some("未命名.md".to_owned()),
        );

        assert_eq!(workspace.index_of(id), Some(0));
        assert_eq!(workspace.suggested_file_name(0), Some("未命名.md"));
        assert_eq!(workspace.entry_title(0).as_deref(), Some("未命名.md"));
    }

    #[test]
    fn append_prepared_tab_preserves_active_tab_and_existing_history() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let active_id =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("active"), None);
        let history_id =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("history"), None);
        workspace.model.set_back_history(vec![history_id, active_id]);
        workspace.model.set_forward_history(vec![history_id]);
        let back_history_before = workspace.model.back_history().to_vec();
        let forward_history_before = workspace.model.forward_history().to_vec();

        let appended_id =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("appended"), None);

        assert_eq!(workspace.tab_id_at(workspace.active_index()), Some(active_id));
        assert_ne!(appended_id, active_id);
        assert_eq!(workspace.model.back_history(), back_history_before);
        assert_eq!(workspace.model.forward_history(), forward_history_before);
    }

    #[test]
    fn first_append_prepared_tab_naturally_becomes_active_without_history() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        let id = workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("first"), None);

        assert_eq!(workspace.tab_id_at(workspace.active_index()), Some(id));
        assert!(workspace.model.back_history().is_empty());
        assert!(workspace.model.forward_history().is_empty());
    }

    #[test]
    fn prepared_tab_id_collision_fails_before_either_side_is_modified() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let mut independent_ids = TabIdAllocator::new();
        let occupied_id = independent_ids.allocate();
        let mut existing_runtime = TabRuntime::new(Box::new(EditorPlugin::new()));
        existing_runtime.toc_visible = true;
        assert!(runtimes.insert(occupied_id, existing_runtime).is_none());

        let insertion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            workspace.append_prepared_tab(
                &mut runtimes,
                prepared_editor_tab("must not install"),
                None,
            );
        }));

        assert!(insertion.is_err());
        assert!(workspace.is_empty());
        assert!(workspace.tab_ids().is_empty());
        assert_eq!(runtimes.ids(), HashSet::from([occupied_id]));
        let preserved_runtime =
            runtimes.get(occupied_id).expect("the pre-existing runtime must remain installed");
        assert!(preserved_runtime.toc_visible);
    }

    #[test]
    fn open_prepared_tab_activates_and_records_navigation_history() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let first_id =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("first"), None);

        let effect =
            workspace.open_prepared_tab(&mut runtimes, prepared_editor_tab("second"), None);
        let second_id =
            workspace.tab_id_at(workspace.active_index()).expect("opened tab should be active");

        assert_eq!(effect, WorkspaceEffect::Activated(second_id));
        assert_ne!(second_id, first_id);
        assert!(workspace.has_back_history());
        assert_eq!(workspace.go_back(), NavEffect::ActiveChanged);
        assert_eq!(workspace.tab_id_at(workspace.active_index()), Some(first_id));
        assert_eq!(workspace.tab_ids(), runtimes.ids());
    }

    #[test]
    fn first_open_prepared_tab_records_its_initial_history_entry() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        let effect = workspace.open_prepared_tab(&mut runtimes, prepared_editor_tab("first"), None);
        let id = workspace.tab_id_at(0).expect("first opened tab must be installed");

        assert_eq!(effect, WorkspaceEffect::Activated(id));
        assert_eq!(workspace.tab_id_at(workspace.active_index()), Some(id));
        assert_eq!(workspace.model.back_history(), &[id]);
        assert!(workspace.model.forward_history().is_empty());
    }

    #[test]
    fn open_prepared_tab_closes_preview_and_reconciles_only_its_runtime() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let preview_id =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("preview"), None);
        workspace.preview_index = Some(0);

        let effect =
            workspace.open_prepared_tab(&mut runtimes, prepared_editor_tab("opened"), None);
        let opened_id =
            workspace.tab_id_at(workspace.active_index()).expect("opened tab should be active");

        assert_eq!(
            effect,
            WorkspaceEffect::Closed { closed: preview_id, activated: Some(opened_id) }
        );
        assert!(runtimes.contains(preview_id));
        assert!(runtimes.contains(opened_id));

        effect.reconcile_runtime_store(&mut runtimes);

        assert!(!runtimes.contains(preview_id));
        assert!(runtimes.contains(opened_id));
        assert_eq!(workspace.tab_ids(), runtimes.ids());
    }

    #[test]
    fn close_effect_reconciles_only_the_closed_tab_runtime() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let first =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("first"), None);
        let second =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("second"), None);

        let effect = workspace.close_entry(0).expect("first prepared tab should close");
        effect.reconcile_runtime_store(&mut runtimes);

        assert!(!runtimes.contains(first));
        assert!(runtimes.contains(second));
        assert_eq!(workspace.tab_ids(), runtimes.ids());
    }

    #[test]
    fn non_closing_effects_leave_runtime_store_unchanged() {
        let mut workspace = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let tab_id =
            workspace.append_prepared_tab(&mut runtimes, prepared_editor_tab("stable"), None);
        let expected_ids = runtimes.ids();

        WorkspaceEffect::None.reconcile_runtime_store(&mut runtimes);
        WorkspaceEffect::Activated(tab_id).reconcile_runtime_store(&mut runtimes);

        assert_eq!(runtimes.ids(), expected_ids);
    }

    fn make_three_tab_ws() -> (Workspace, TabRuntimeStore, Vec<TabId>) {
        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        open_editor_tab(&mut ws, &mut runtimes, "");
        open_editor_tab(&mut ws, &mut runtimes, "");
        open_editor_tab(&mut ws, &mut runtimes, "");
        let ids = (0..ws.len()).map(|i| ws.tab_id_at(i).unwrap()).collect();
        (ws, runtimes, ids)
    }

    #[test]
    fn tab_ids_are_independent_of_indices() {
        let (ws, _runtimes, ids) = make_three_tab_ws();
        for (i, &id) in ids.iter().enumerate() {
            assert_eq!(ws.tab_id_at(i), Some(id));
            assert_eq!(ws.index_of(id), Some(i));
        }
    }

    #[test]
    fn closing_a_tab_keeps_remaining_ids_stable() {
        let (mut ws, _runtimes, ids) = make_three_tab_ws();
        let closed_id = ids[1];
        assert_eq!(
            ws.close_entry_inner(1).expect("closing a non-active tab should succeed"),
            WorkspaceEffect::Closed { closed: closed_id, activated: None }
        );

        assert_eq!(ws.index_of(closed_id), None);
        assert_eq!(ws.tab_id_at(0), Some(ids[0]));
        assert_eq!(ws.tab_id_at(1), Some(ids[2]));
        assert_eq!(ws.index_of(ids[0]), Some(0));
        assert_eq!(ws.index_of(ids[2]), Some(1));
    }

    #[test]
    fn switch_to_returns_activated_tab_id() {
        let (mut ws, _runtimes, ids) = make_three_tab_ws();

        let effect = ws.switch_to(0);

        assert_eq!(effect, WorkspaceEffect::Activated(ids[0]));
    }

    #[test]
    fn close_entry_returns_closed_and_reactivated_tab_ids() {
        let (mut ws, _runtimes, ids) = make_three_tab_ws();
        let switch_effect = ws.switch_to(1);
        assert_eq!(switch_effect, WorkspaceEffect::Activated(ids[1]));

        let effect = ws.close_entry(1).expect("closing active tab should succeed");

        assert_eq!(effect, WorkspaceEffect::Closed { closed: ids[1], activated: Some(ids[2]) });
    }

    #[test]
    fn reordering_tabs_preserves_ids() {
        let (mut ws, _runtimes, ids) = make_three_tab_ws();
        // Simulate a drag reorder: swap the first and last tabs.
        ws.model.entries_mut().swap(0, 2);

        assert_eq!(ws.tab_id_at(0), Some(ids[2]));
        assert_eq!(ws.tab_id_at(2), Some(ids[0]));
        assert_eq!(ws.index_of(ids[0]), Some(2));
        assert_eq!(ws.index_of(ids[2]), Some(0));
        assert_eq!(ws.index_of(ids[1]), Some(1));
    }

    #[test]
    fn navigation_history_follows_tab_ids_after_closing() {
        let (mut ws, _runtimes, ids) = make_three_tab_ws();
        let _ = ws.switch_to(0); // records nav step from tab 2 -> tab 0
        assert_eq!(
            ws.close_entry_inner(1).expect("closing the middle tab should succeed"),
            WorkspaceEffect::Closed { closed: ids[1], activated: None }
        ); // old tab 2 shifts to index 1

        // The back history still navigates using indices, and the surviving tabs keep
        // their stable IDs even though their positions changed.
        assert_eq!(ws.go_back(), NavEffect::ActiveChanged);
        assert_eq!(ws.tab_id_at(ws.active_index()), Some(ids[2]));
        assert_eq!(ws.index_of(ids[2]), Some(1));
        assert_eq!(ws.index_of(ids[0]), Some(0));
    }

    // ── Navigation history ──

    fn make_nav_ws() -> (Workspace, TabRuntimeStore) {
        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        open_editor_tab(&mut ws, &mut runtimes, "");
        open_editor_tab(&mut ws, &mut runtimes, "");
        open_editor_tab(&mut ws, &mut runtimes, "");
        (ws, runtimes)
    }

    #[test]
    fn go_back_returns_none_when_truly_empty() {
        // test_workspace() creates no tabs, so history is empty

        let mut ws = test_workspace();
        assert_eq!(ws.go_back(), NavEffect::None);
    }

    #[test]
    fn go_forward_returns_none_when_truly_empty() {
        let mut ws = test_workspace();
        assert_eq!(ws.go_forward(), NavEffect::None);
    }

    #[test]
    fn switch_to_records_nav_step() {
        let (mut ws, _runtimes) = make_nav_ws();
        let prev_len = ws.model.back_history().len();
        let _ = ws.switch_to(0);
        // switch_to pushes the previous active tab id to back_history
        assert_eq!(ws.model.back_history().len(), prev_len + 1);
        assert_eq!(ws.active_index(), 0);
    }

    #[test]
    fn go_back_and_forward_roundtrip() {
        let (mut ws, _runtimes) = make_nav_ws();
        let _ = ws.switch_to(0);
        assert!(!ws.has_forward_history());
        let effect = ws.go_back();
        assert_eq!(effect, NavEffect::ActiveChanged);
        assert_eq!(ws.active_index(), 2);
        assert!(ws.has_forward_history());
        let effect = ws.go_forward();
        assert_eq!(effect, NavEffect::ActiveChanged);
        assert_eq!(ws.active_index(), 0);
    }

    #[test]
    fn new_tab_clears_forward_history() {
        let (mut ws, mut runtimes) = make_nav_ws();
        let _ = ws.switch_to(0);
        ws.go_back(); // creates forward history
        assert!(ws.has_forward_history());
        let effect = ws.open_prepared_tab(&mut runtimes, prepared_editor_tab(""), None);
        effect.reconcile_runtime_store(&mut runtimes);
        assert!(!ws.has_forward_history());
    }

    #[test]
    fn close_entry_removes_closed_id_from_history() {
        let (mut ws, _runtimes) = make_nav_ws();
        let id0 = ws.tab_id_at(0).unwrap();
        let id1 = ws.tab_id_at(1).unwrap();
        // After make_nav_ws: back_history contains the closed tab id twice.
        let count_0 = ws.model.back_history().iter().filter(|&&id| id == id0).count();
        assert_eq!(count_0, 2);
        assert_eq!(
            ws.close_entry_inner(0).expect("closing tab 0 should succeed"),
            WorkspaceEffect::Closed { closed: id0, activated: None }
        );
        // The closed tab id is fully removed; surviving ids stay in history.
        let count_0_after = ws.model.back_history().iter().filter(|&&id| id == id0).count();
        assert_eq!(count_0_after, 0);
        assert!(ws.model.back_history().contains(&id1));
    }

    #[test]
    fn close_entry_keeps_surviving_ids_in_history() {
        let (mut ws, _runtimes) = make_nav_ws();
        let id1 = ws.tab_id_at(1).unwrap();
        let closed_id = ws.tab_id_at(0).expect("tab 0 should exist");
        assert_eq!(
            ws.close_entry_inner(0).expect("closing tab 0 should succeed"),
            WorkspaceEffect::Closed { closed: closed_id, activated: None }
        );
        assert!(ws.model.back_history().contains(&id1));
        let effect = ws.go_back();
        assert_eq!(effect, NavEffect::ActiveChanged);
    }

    #[test]
    fn toggle_pin_at_pins_correct_tab_not_active() {
        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        for _i in 0..5 {
            append_editor_tab(&mut ws, &mut runtimes, "");
        }
        set_active_fixture(&mut ws, 1); // tab 1 is active

        // Pin tab 3 via context menu (not the active tab)
        ws.toggle_pin_at(3);
        assert!(ws.pinned_indices().contains(&3), "tab 3 should be pinned");
        assert!(!ws.pinned_indices().contains(&1), "active tab 1 should NOT be pinned");

        // Pin tab 1 via keyboard shortcut (uses active_index)
        ws.toggle_pin();
        assert!(ws.pinned_indices().contains(&1), "active tab 1 should now be pinned");

        // Unpin tab 3 via context menu
        ws.toggle_pin_at(3);
        assert!(!ws.pinned_indices().contains(&3), "tab 3 should be unpinned");
        assert!(ws.pinned_indices().contains(&1), "tab 1 should still be pinned");
    }

    #[test]
    fn active_accessors_are_safe_for_empty_workspace() {
        let mut ws = test_workspace();

        assert_eq!(ws.active_index(), 0);
        assert!(ws.active_entry().is_none());
        assert!(ws.active_entry_mut().is_none());
        assert!(ws.active_doc().is_none());
        assert!(ws.active_doc_mut().is_none());
        assert!(ws.entry(0).is_none());
        assert!(ws.entry_mut(0).is_none());
        assert!(ws.entries().is_empty());
        assert!(ws.pinned_indices().is_empty());
    }

    #[test]
    fn push_entry_appends_without_changing_the_active_entry() {
        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        append_document(&mut ws, &mut runtimes, document("active"), Box::new(EditorPlugin::new()));
        append_document(
            &mut ws,
            &mut runtimes,
            document("inactive"),
            Box::new(EditorPlugin::new()),
        );
        let previous_len = ws.len();
        let previous_active_index = ws.active_index();
        let previous_active_buffer_len = ws.active_doc().unwrap().buffer_len();

        append_document(
            &mut ws,
            &mut runtimes,
            document("# appended"),
            Box::new(EditorPlugin::new()),
        );

        assert_eq!(ws.len(), previous_len + 1);
        assert_eq!(ws.active_index(), previous_active_index);
        assert!(runtime_at(&ws, &runtimes, ws.active_index()).plugin.allows_editing());
        assert_eq!(ws.active_doc().unwrap().buffer_len(), previous_active_buffer_len);
        assert!(runtime_at(&ws, &runtimes, previous_len).plugin.allows_editing());
    }

    #[test]
    fn active_accessors_expose_and_mutate_editor_view() {
        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        let effect = ws.open_prepared_tab(&mut runtimes, prepared_editor_tab(""), None);
        effect.reconcile_runtime_store(&mut runtimes);
        ws.active_doc_mut().unwrap().insert_at_cursor(b"inactive");
        let effect = ws.open_prepared_tab(&mut runtimes, prepared_editor_tab(""), None);
        effect.reconcile_runtime_store(&mut runtimes);

        assert_eq!(ws.active_index(), 1);
        assert!(runtime_at(&ws, &runtimes, ws.active_index()).plugin.allows_editing());
        assert_eq!(ws.active_entry().unwrap().buffer_len(), 0);
        assert_eq!(ws.active_doc().unwrap().buffer_len(), 0);
        let Some(tab) = ws.active_entry_mut() else {
            panic!("active_entry_mut should expose the active editor view");
        };
        tab.insert_at_cursor(b"a");
        ws.active_doc_mut().unwrap().insert_at_cursor(b"b");
        let Some(tab) = ws.entry_mut(1) else {
            panic!("tab_mut should expose an editor view by index");
        };
        tab.insert_at_cursor(b"c");

        assert_eq!(ws.active_entry().unwrap().buffer_len(), 3);
        assert_eq!(ws.active_doc().unwrap().buffer_len(), 3);
        assert_eq!(ws.entry(0).unwrap().buffer_len(), 8);
        assert_eq!(ws.entry(1).unwrap().buffer_len(), 3);
        assert!(runtime_at(&ws, &runtimes, 0).plugin.allows_editing());
        assert_eq!(ws.entries().len(), 2);
        ws.toggle_pin_at(1);
        assert!(ws.pinned_indices().contains(&1));
        assert!(!ws.pinned_indices().contains(&0));
    }

    #[test]
    fn close_entry_inner_leaves_new_active_stub_unloaded() {
        let file_path = PathBuf::from("/workspace-fixtures/close-stub.txt");

        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        // Tab 0: stub with file_path (simulating workspace restore)
        let mut stub = document("");
        stub.file_path = Some(file_path.clone());
        append_document(&mut ws, &mut runtimes, stub, Box::new(EditorPlugin::new()));

        // Tab 1: active, dirty
        let mut dirty = document("dirty");
        dirty.dirty = true;
        append_document(&mut ws, &mut runtimes, dirty, Box::new(EditorPlugin::new()));
        set_active_fixture(&mut ws, 1);

        assert_eq!(ws.len(), 2);

        // Close active dirty tab (tab 1)
        let result = ws.close_entry_inner(1);
        assert!(result.is_ok());
        assert_eq!(ws.active_index(), 0);
        assert_eq!(ws.len(), 1);

        let document = ws.active_doc().expect("stub should become active after close");
        assert_eq!(document.buffer_len(), 0);
        assert_eq!(document.file_path.as_ref(), Some(&file_path));
    }

    #[test]
    fn restore_pinned_is_idempotent() {
        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();
        let path_a = std::path::PathBuf::from("/tmp/restore_pinned_a.txt");
        let path_b = std::path::PathBuf::from("/tmp/restore_pinned_b.txt");

        let mut doc_a = document("a");
        doc_a.file_path = Some(path_a.clone());
        append_document(&mut ws, &mut runtimes, doc_a, Box::new(EditorPlugin::new()));

        let mut doc_b = document("b");
        doc_b.file_path = Some(path_b.clone());
        append_document(&mut ws, &mut runtimes, doc_b, Box::new(EditorPlugin::new()));

        ws.toggle_pin_at(0);
        assert!(ws.pinned_indices().contains(&0), "tab 0 should be pinned initially");

        ws.restore_pinned(std::slice::from_ref(&path_a));
        assert!(ws.pinned_indices().contains(&0), "tab 0 should stay pinned after first restore");

        ws.restore_pinned(std::slice::from_ref(&path_a));
        assert!(ws.pinned_indices().contains(&0), "tab 0 should stay pinned after second restore");

        assert!(!ws.pinned_indices().contains(&1), "tab 1 should remain unpinned");
        assert_eq!(ws.len(), 2);
    }

    #[test]
    fn switch_to_after_preview_close_leaves_target_stub_unloaded() {
        let file_path = PathBuf::from("/workspace-fixtures/preview-stub.txt");

        let mut ws = test_workspace();
        let mut runtimes = TabRuntimeStore::default();

        // Tab 0: clean stub with a file path, marked as the preview tab.
        let mut preview_stub = document("");
        preview_stub.file_path = Some(file_path.clone());
        append_document(&mut ws, &mut runtimes, preview_stub, Box::new(EditorPlugin::new()));
        ws.preview_index = Some(0);

        // Tab 1: clean stub with the same file path; becomes the target after the preview closes.
        let mut target_stub = document("");
        target_stub.file_path = Some(file_path.clone());
        append_document(&mut ws, &mut runtimes, target_stub, Box::new(EditorPlugin::new()));
        set_active_fixture(&mut ws, 1);

        assert_eq!(ws.len(), 2);
        assert_eq!(ws.active_index(), 1);

        // Switching away from the preview closes it. The active tab (tab 1) stays the same ID
        // and remains a stub until the application consumes the returned effect.
        let preview_id = ws.tab_id_at(0).expect("preview tab id");
        let active_id = ws.tab_id_at(1).expect("active tab id");
        let effect = ws.switch_to(1);
        effect.reconcile_runtime_store(&mut runtimes);
        assert_eq!(
            effect,
            WorkspaceEffect::Closed { closed: preview_id, activated: Some(active_id) }
        );
        assert_eq!(ws.len(), 1);
        assert!(ws.preview_index.is_none());
        assert_eq!(ws.active_index(), 0);

        let document = ws.active_doc().expect("target stub should remain active");
        assert_eq!(document.buffer_len(), 0);
        assert_eq!(document.file_path.as_ref(), Some(&file_path));
    }

    #[test]
    fn doc_title_returns_filename() {
        let document = document("hello");
        assert_eq!(document_title(&document, None), "untitled");
    }

    #[test]
    fn doc_title_returns_file_path_name() {
        let mut document = document("hello");
        document.file_path = Some(std::path::PathBuf::from("/tmp/test.rs"));
        assert_eq!(document_title(&document, None), "test.rs");
    }
}
