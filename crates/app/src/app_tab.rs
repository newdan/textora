//! Tab management: open, close, history, workspace effects.
//! Methods on `impl App`, extracted from app.rs.

use crate::app::App;
use crate::file_history::{FileHistoryEntry, compute_workspace_root};
use crate::tab_runtime::TabRuntime;
use crate::tab_session::{TabSession, TabSessionMut};
use crate::workspace::Workspace;
use crate::workspace_persistence::snapshot_runtime_workspace;
use appkit_core::workspace::types::TabId;
use appkit_shell::editor_runtime::OpenDisposition;
#[cfg(test)]
use appkit_shell::prepared_tab::PreparedTab;
use core::types::UniCharOffset;
use winit::event::ElementState;
use winit::keyboard::ModifiersState;

pub(crate) struct EditorSaveContext {
    pub(crate) dirty: bool,
    pub(crate) file_path: Option<std::path::PathBuf>,
    pub(crate) title: String,
}

impl App {
    /// 安装产品准备好的 tab，并在 runtime 内完成生命周期收口。
    pub(crate) fn install_editor_tab(
        &mut self,
        prepared: appkit_shell::prepared_tab::PreparedTab,
        suggested_file_name: Option<String>,
        disposition: OpenDisposition,
    ) -> crate::workspace::WorkspaceEffect {
        self.editor_runtime.install_prepared_tab_for_product(
            prepared,
            suggested_file_name,
            disposition,
        )
    }

    pub(crate) fn activate_editor_tab(
        &mut self,
        tab_id: TabId,
    ) -> Option<crate::workspace::WorkspaceEffect> {
        self.editor_runtime.activate_for_product(tab_id)
    }

    /// 追加产品准备好的 tab，但保留当前活动 tab；用于启动文件和恢复流程。
    pub(crate) fn append_editor_tab(
        &mut self,
        prepared: appkit_shell::prepared_tab::PreparedTab,
        suggested_file_name: Option<String>,
    ) -> TabId {
        self.editor_runtime.append_prepared_tab(prepared, suggested_file_name)
    }

    pub(crate) fn editor_close_decision(
        &mut self,
        tab_id: TabId,
    ) -> Option<crate::workspace::CloseTabDecision> {
        self.editor_runtime.close_decision(tab_id)
    }

    pub(crate) fn close_editor_tab(
        &mut self,
        tab_id: TabId,
    ) -> Option<crate::workspace::WorkspaceEffect> {
        self.editor_runtime.close_for_product(tab_id)
    }

    pub(crate) fn active_tab_id(&self) -> Option<TabId> {
        self.editor_runtime.active_tab_id()
    }

    pub(crate) fn editor_tab_index(&self, tab_id: TabId) -> Option<usize> {
        self.editor_runtime.tab_index(tab_id)
    }

    pub(crate) fn active_editor_index(&self) -> Option<usize> {
        self.active_tab_id().and_then(|tab_id| self.editor_tab_index(tab_id))
    }

    pub(crate) fn editor_tab_id_at(&self, index: usize) -> Option<TabId> {
        self.editor_runtime.tab_id_at(index)
    }

    pub(crate) fn is_editor_tab_pinned_at(&self, index: usize) -> bool {
        self.editor_tab_id_at(index).is_some_and(|tab_id| self.editor_runtime.is_pinned(tab_id))
    }

    pub(crate) fn editor_tab_id_for_path(&self, path: &std::path::Path) -> Option<TabId> {
        self.editor_runtime.tab_id_for_path(path)
    }

    pub(crate) fn editor_tab_count(&self) -> usize {
        self.editor_runtime.tab_count()
    }

    pub(crate) fn editor_tab_ids_in_order(&self) -> Vec<TabId> {
        self.editor_runtime.tab_ids_in_order()
    }

    pub(crate) fn editor_runtime_tab_ids(&self) -> std::collections::HashSet<TabId> {
        self.editor_runtime.runtime_tab_ids()
    }

    pub(crate) fn editor_is_empty(&self) -> bool {
        self.editor_runtime.is_empty()
    }

    pub(crate) fn editor_has_back_history(&mut self) -> bool {
        self.editor_runtime.has_back_history()
    }

    pub(crate) fn editor_has_forward_history(&mut self) -> bool {
        self.editor_runtime.has_forward_history()
    }

    pub(crate) fn active_editor_file_path(&self) -> Option<std::path::PathBuf> {
        self.active_tab_session().and_then(|tab| tab.document.file_path.clone())
    }

    pub(crate) fn active_editor_toggle_target(&mut self) -> Option<&'static str> {
        self.editor_runtime.toggle_target()
    }

    pub(crate) fn active_document_line_count(&self) -> usize {
        self.active_tab_session().map_or(0, |tab| tab.document.line_count())
    }

    pub(crate) fn active_editor_title(&mut self) -> Option<String> {
        let tab_id = self.active_tab_id()?;
        self.editor_save_context(tab_id).map(|context| context.title)
    }

    pub(crate) fn prepare_editor_file(
        &mut self,
        path: &std::path::Path,
        viewport: crate::workspace::ViewportDimensions,
    ) -> Result<crate::workspace_tab_factory::ProductPreparedTab, String> {
        let plugin = self.editor_runtime.create_plugin_for_path(path);
        crate::workspace_tab_factory::prepare_file_with_plugin(path, viewport, plugin)
    }

    pub(crate) fn prepare_editor_untitled(
        &mut self,
        viewport: crate::workspace::ViewportDimensions,
    ) -> crate::workspace_tab_factory::ProductPreparedTab {
        let plugin = self.editor_runtime.create_plugin_by_name(ui::plugin::PLUGIN_EDITOR);
        crate::workspace_tab_factory::prepare_untitled_with_plugin(viewport, plugin)
    }

    pub(crate) fn prepare_typed_editor_untitled(
        &mut self,
        kind: ui::sidebar::NewDocumentKind,
        viewport: crate::workspace::ViewportDimensions,
    ) -> crate::workspace_tab_factory::ProductPreparedTab {
        let plugin_name = match kind {
            ui::sidebar::NewDocumentKind::Text => ui::plugin::PLUGIN_EDITOR,
            ui::sidebar::NewDocumentKind::Mindmap => ui::plugin::PLUGIN_MINDMAP,
            ui::sidebar::NewDocumentKind::Markdown => ui::plugin::PLUGIN_MARKDOWN_EDITOR,
        };
        let plugin = self.editor_runtime.create_plugin_by_name(plugin_name);
        crate::workspace_tab_factory::prepare_typed_untitled_with_plugin(kind, viewport, plugin)
    }

    pub(crate) fn submit_editor_save(
        &mut self,
        tab_id: TabId,
        path: Option<&std::path::Path>,
    ) -> Result<(), appkit_shell::editor_runtime::SavePrepareError> {
        let prepared = match path {
            Some(path) => self.editor_runtime.prepare_save_as(tab_id, path),
            None => self.editor_runtime.prepare_save(tab_id),
        }?;
        let Some(event_loop_proxy) = self.event_loop_proxy.clone() else {
            return Err(appkit_shell::editor_runtime::SavePrepareError::SubmitFailed {
                message: "event loop proxy is not available".to_owned(),
            });
        };
        self.editor_runtime
            .submit_save(prepared, move || {
                let _ = event_loop_proxy.send_event(crate::app_event::AppEvent::SaveResultsReady);
            })
            .map_err(|message| appkit_shell::editor_runtime::SavePrepareError::SubmitFailed {
                message,
            })
    }

    pub(crate) fn submit_editor_save_before_close(
        &mut self,
        tab_id: TabId,
        path: Option<&std::path::Path>,
    ) -> Result<(), appkit_shell::editor_runtime::SavePrepareError> {
        self.submit_editor_save(tab_id, path)?;
        self.pending_close_after_save.insert(tab_id);
        Ok(())
    }

    pub(crate) fn prepare_external_editor_content(
        &mut self,
        path: &std::path::Path,
        content: &str,
        viewport: crate::workspace::ViewportDimensions,
    ) -> crate::workspace_tab_factory::ProductPreparedTab {
        let plugin = self.editor_runtime.create_plugin_for_path(path);
        crate::workspace_tab_factory::prepare_external_content_with_plugin(
            path, content, viewport, plugin,
        )
    }

    pub(crate) fn editor_save_context(&self, tab_id: TabId) -> Option<EditorSaveContext> {
        let summary = self.editor_runtime.document_summary(tab_id)?;
        Some(EditorSaveContext {
            dirty: summary.dirty,
            file_path: summary.path,
            title: self.editor_runtime.tab_title(tab_id)?,
        })
    }

    pub(crate) fn clear_editor_suggested_file_name(&mut self, tab_id: TabId) {
        self.editor_runtime.clear_suggested_file_name(tab_id);
    }

    pub(crate) fn hydrate_active_editor_stub(&mut self) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let Some(tab) = self.tab_session_mut(tab_id) else {
            return false;
        };
        crate::workspace_product::hydrate_stub_document(tab.document)
    }

    pub(crate) fn toggle_editor_pin(
        &mut self,
        tab_id: TabId,
    ) -> Option<crate::navigator::NavEffect> {
        self.editor_runtime.toggle_pin(tab_id)
    }

    pub(crate) fn copy_editor_path(&mut self, tab_id: TabId) -> bool {
        let Some(tab) = self.tab_session(tab_id) else {
            return false;
        };
        crate::workspace_product::copy_document_path(tab.document);
        true
    }

    pub(crate) fn replace_editor_document(
        &mut self,
        tab_id: TabId,
        document: appkit_core::document::DocumentModel,
    ) -> bool {
        self.editor_runtime.replace_document(tab_id, document)
    }

    pub(crate) fn update_editor_document_path(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        disk_revision: Option<appkit_core::file_safety::DiskRevision>,
    ) -> bool {
        self.editor_runtime.update_document_path(tab_id, path, disk_revision)
    }

    pub(crate) fn detach_deleted_editor_document(
        &mut self,
        tab_id: TabId,
        original_path: &std::path::Path,
    ) -> bool {
        let file_name =
            original_path.file_name().and_then(|name| name.to_str()).unwrap_or("untitled");
        let suggested_file_name = Some(format!("恢复：{file_name}"));
        let dirty_snapshot_id =
            Some(crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id()));
        self.editor_runtime.detach_document(tab_id, suggested_file_name, dirty_snapshot_id)
    }

    pub(crate) fn toggle_active_editor_pin(&mut self) -> crate::navigator::NavEffect {
        self.editor_runtime.toggle_active_pin()
    }

    pub(crate) fn navigate_editor_back(&mut self) -> crate::navigator::NavEffect {
        self.editor_runtime.navigate_back()
    }

    pub(crate) fn navigate_editor_forward(&mut self) -> crate::navigator::NavEffect {
        self.editor_runtime.navigate_forward()
    }

    pub(crate) fn upgrade_active_editor_preview(&mut self) -> crate::navigator::NavEffect {
        self.editor_runtime.upgrade_active_preview()
    }

    pub(crate) fn handle_editor_mouse_input(
        &mut self,
        state: ElementState,
        px: f32,
        py: f32,
        modifiers: ModifiersState,
        hit: Option<(UniCharOffset, usize, usize)>,
        line_height: f32,
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let App { editor_runtime, mouse, .. } = self;
        let Some(mut tab) = editor_runtime.tab_session_mut(tab_id) else {
            return false;
        };
        let mut presentation = tab.take_presentation();
        let handled = crate::mouse::handle_mouse_input_with_cursor_state(
            state,
            px,
            py,
            mouse,
            tab.document,
            &mut presentation.cursor_render_state,
            modifiers,
            hit,
        );
        tab.restore_presentation(presentation);
        if handled {
            tab.ensure_cursor_visible(line_height);
        }
        handled
    }

    pub(crate) fn handle_editor_cursor_moved(
        &mut self,
        px: f32,
        py: f32,
        hit: Option<(UniCharOffset, usize, usize)>,
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let App { editor_runtime, mouse, .. } = self;
        let Some(tab) = editor_runtime.tab_session_mut(tab_id) else {
            return false;
        };
        crate::mouse::handle_cursor_moved(px, py, mouse, tab.document, hit)
    }

    pub(crate) fn switch_editor_tab(
        &mut self,
        tab_id: TabId,
    ) -> Option<crate::workspace::WorkspaceEffect> {
        if self.active_tab_id() == Some(tab_id) {
            return None;
        }
        self.activate_editor_tab(tab_id)
    }

    pub(crate) fn replace_editor_model(
        &mut self,
        workspace: Workspace,
        runtimes: crate::tab_runtime::TabRuntimeStore,
    ) {
        self.editor_runtime.replace_model_state(workspace, runtimes);
    }

    pub(crate) fn restore_editor_pinned_paths(&mut self, paths: &[std::path::PathBuf]) {
        self.editor_runtime.restore_pinned(paths);
    }

    pub(crate) fn invalidate_editor_render_caches(&mut self, tab_ids: &[TabId]) {
        for &tab_id in tab_ids {
            if let Some(mut tab) = self.tab_session_mut(tab_id) {
                tab.invalidate_render_cache_all();
            }
        }
    }

    pub(crate) fn clear_active_editor_advance_cache(&mut self) {
        let Some(tab_id) = self.active_tab_id() else {
            return;
        };
        if let Some(mut tab) = self.tab_session_mut(tab_id) {
            tab.clear_advance_cache();
        }
    }

    #[cfg(test)]
    pub(crate) fn with_editor_document_and_mouse_for_test<R>(
        &mut self,
        tab_id: TabId,
        callback: impl FnOnce(
            &mut crate::mouse::MouseState,
            &mut appkit_core::document::DocumentModel,
        ) -> R,
    ) -> Option<R> {
        let App { editor_runtime, mouse, .. } = self;
        let tab = editor_runtime.tab_session_mut(tab_id)?;
        Some(callback(mouse, tab.document))
    }

    pub(crate) fn tab_runtime(&self, id: TabId) -> Option<&TabRuntime> {
        self.editor_runtime.tab_runtime(id)
    }

    pub(crate) fn tab_runtime_mut(&mut self, id: TabId) -> Option<&mut TabRuntime> {
        self.editor_runtime.tab_runtime_mut(id)
    }

    pub(crate) fn tab_session(&self, id: TabId) -> Option<TabSession<'_>> {
        self.editor_runtime.tab_session(id)
    }

    pub(crate) fn tab_session_mut(&mut self, id: TabId) -> Option<TabSessionMut<'_>> {
        self.editor_runtime.tab_session_mut(id)
    }

    pub(crate) fn active_tab_session(&self) -> Option<TabSession<'_>> {
        self.active_tab_id().and_then(|id| self.tab_session(id))
    }

    pub(crate) fn active_tab_session_mut(&mut self) -> Option<TabSessionMut<'_>> {
        self.active_tab_id().and_then(|id| self.tab_session_mut(id))
    }

    pub(crate) fn switch_active_plugin(&mut self) {
        self.editor_runtime.switch_active_plugin();
    }

    #[cfg(test)]
    pub(crate) fn push_entry_for_test(
        &mut self,
        document: crate::document_view::DocumentView,
        plugin: Box<dyn ui::plugin::ViewPlugin>,
    ) -> TabId {
        let (document, presentation) = document.into_parts();
        let runtime = TabRuntime::with_presentation(plugin, presentation);
        self.editor_runtime.append_prepared_tab(PreparedTab::new(document, runtime), None)
    }

    pub(crate) fn active_runtime(&self) -> Option<&TabRuntime> {
        self.active_tab_id().and_then(|id| self.tab_runtime(id))
    }

    pub(crate) fn active_runtime_mut(&mut self) -> Option<&mut TabRuntime> {
        self.active_tab_id().and_then(|id| self.tab_runtime_mut(id))
    }

    pub(crate) fn active_allows_editing(&self) -> bool {
        self.active_tab_session().is_some_and(|session| session.allows_editing())
    }

    pub(crate) fn active_plugin_name(&self) -> Option<&str> {
        self.active_tab_session().map(|session| session.plugin_name())
    }

    pub(crate) fn active_handles_own_rendering(&self) -> bool {
        self.active_tab_session().is_some_and(|session| session.handles_own_rendering())
    }

    pub(crate) fn active_is_canvas(&self) -> bool {
        self.active_tab_session().is_some_and(|session| session.is_canvas())
    }

    pub(crate) fn active_has_canvas_viewport_snapshot(&self) -> bool {
        self.active_tab_session().is_some_and(|session| session.has_canvas_viewport_snapshot())
    }

    pub(crate) fn active_is_reading_mode(&self) -> bool {
        self.active_tab_session().is_some_and(|session| !session.allows_editing())
    }

    pub(crate) fn active_is_mindmap(&self) -> bool {
        self.active_plugin_name().is_some_and(|name| name == ui::plugin::PLUGIN_MINDMAP)
    }

    pub(crate) fn active_is_toggled(&mut self) -> bool {
        let Some(plugin_name) = self.active_plugin_name().map(str::to_owned) else {
            return false;
        };
        self.editor_runtime.active_is_toggled(&plugin_name)
    }

    pub(crate) fn active_shows_gutter(&self) -> bool {
        self.active_tab_session().is_none_or(|session| session.shows_gutter())
    }

    pub(crate) fn active_needs_cursor_blink_wakeup(&self) -> bool {
        self.active_tab_session().is_some_and(|session| session.needs_cursor_blink_wakeup())
    }

    pub(crate) fn active_toc_visible(&self) -> bool {
        self.active_tab_session().is_some_and(|session| session.toc_visible())
    }

    /// Persist workspace snapshot with correct sidebar settings.
    pub(crate) fn persist_workspace_state(&mut self) {
        let sidebar_pinned = self.ui_shell.sidebar_pinned();
        let sidebar_width = self.ui_shell.sidebar_width();
        let snapshots_directory = &self.paths.snapshots_dir;
        let runtime_snapshot = self.editor_runtime.workspace_snapshot();
        let snap = snapshot_runtime_workspace(
            &runtime_snapshot,
            sidebar_pinned,
            Some(sidebar_width),
            snapshots_directory,
        );
        if let Err(e) = self.workspace_store.save_workspace(&snap) {
            eprintln!("[workspace] save_workspace error: {}", e);
        }
        self.save_pinned_paths();
    }

    pub(crate) fn save_pinned_paths(&mut self) {
        let paths = self.editor_runtime.pinned_paths();
        if let Err(e) = self.workspace_store.save_pinned_paths(&paths) {
            eprintln!("[workspace] save_pinned_paths error: {}", e);
        }
    }

    /// Record a tab to file history before closing.
    pub(crate) fn record_entry_to_history(&mut self, index: usize) {
        let snapshot = self.editor_runtime.workspace_snapshot();
        let Some(tab) = snapshot.tabs.get(index) else {
            return;
        };
        let Some(file_path) = tab.path.clone() else {
            return;
        };
        let paths: Vec<&std::path::Path> =
            snapshot.tabs.iter().filter_map(|tab| tab.path.as_deref()).collect();
        let entry = Some(FileHistoryEntry {
            file_path,
            workspace_root: compute_workspace_root(&paths),
            last_closed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            last_cursor_line: tab.cursor_line,
            last_cursor_col: tab.cursor_column,
            scroll_anchor_line: tab.scroll_anchor_line,
            scroll_anchor_offset: tab.scroll_anchor_offset,
        });
        if let Some(entry) = entry {
            self.file_history.record(entry);
        }
    }

    /// Record all open tabs to history (on quit).
    pub(crate) fn record_all_entries_to_history(&mut self) {
        let snapshot = self.editor_runtime.workspace_snapshot();
        let paths: Vec<&std::path::Path> =
            snapshot.tabs.iter().filter_map(|tab| tab.path.as_deref()).collect();
        let workspace_root = compute_workspace_root(&paths);
        let entries = snapshot
            .tabs
            .into_iter()
            .filter_map(|tab| {
                Some(FileHistoryEntry {
                    file_path: tab.path?,
                    workspace_root: workspace_root.clone(),
                    last_closed_at: 0,
                    last_cursor_line: tab.cursor_line,
                    last_cursor_col: tab.cursor_column,
                    scroll_anchor_line: tab.scroll_anchor_line,
                    scroll_anchor_offset: tab.scroll_anchor_offset,
                })
            })
            .collect::<Vec<_>>();
        self.file_history.record_batch(entries);
    }

    /// Persist history to disk (non-fatal on error).
    pub(crate) fn save_history(&self) {
        if let Err(e) = self.file_history.save(&self.paths.history_file) {
            eprintln!("[file_history] save failed: {e}");
        }
    }

    pub(crate) fn update_document_edited(&self, edited: bool) {
        #[cfg(target_os = "macos")]
        {
            if let Some(window) = self.editor_runtime.window() {
                use winit::platform::macos::WindowExtMacOS;
                window.set_document_edited(edited);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = edited;
        }
    }

    pub(crate) fn update_window_title(&self) {
        if let Some(w) = self.editor_runtime.window() {
            if let Some(file_path) = self.active_editor_file_path() {
                let title = format!(
                    "{} — edit+",
                    file_path
                        .file_name()
                        .map(std::ffi::OsStr::to_string_lossy)
                        .unwrap_or_else(|| "untitled".into())
                );
                w.set_title(&title);
            } else {
                w.set_title("edit+");
            }
        }
    }

    /// Rebuild the native menu bar with current recent files.
    pub(crate) fn rebuild_native_menu(&mut self) {
        let recent: Vec<std::path::PathBuf> = self
            .file_history
            .get_valid_entries(crate::file_history::MENU_LIMIT)
            .iter()
            .map(|e| e.file_path.clone())
            .collect();
        self.set_native_menu(crate::native_menu::NativeMenu::build(&recent));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;
    use shaping::Shaper;
    use ui::core::geom::Rect;
    use ui::core::paint::DrawList;
    use ui::plugin::ViewPlugin;
    use ui::theme::Theme;

    #[test]
    fn rebuild_native_menu_uses_menu_accessor() {
        let source = include_str!("app_tab.rs");
        let forbidden_assignment = ["self.native_", "menu ="].concat();

        assert!(
            !source.contains(&forbidden_assignment),
            "tab updates must set the native menu through App::set_native_menu"
        );
    }

    #[test]
    fn active_is_reading_mode_tracks_preview_state() {
        let mut app = App::new(None);
        let mut doc = DocumentView::new(vec!["hello".into()], 80, 10.0);
        doc.file_path = Some(std::path::PathBuf::from("test.txt"));
        let tab_id = app.push_entry_for_test(doc, Box::new(EditorPlugin::new()));
        app.tab_runtime_mut(tab_id).expect("test tab runtime must exist").plugin =
            Box::new(EditorPlugin::new());
        assert!(!app.active_is_reading_mode(), "fresh editor tab should not be reading mode");

        app.switch_active_plugin();

        assert!(app.active_is_reading_mode(), "preview tab should report reading mode");
    }

    #[test]
    fn active_canvas_viewport_snapshot_tracks_active_canvas_tab() {
        struct CanvasPlugin;

        impl ViewPlugin for CanvasPlugin {
            fn name(&self) -> &str {
                "app-tab-canvas-test"
            }

            fn render(
                &mut self,
                _: &dyn core::document::DocView,
                _: Rect,
                _: &Theme,
                _: &mut Shaper,
                _: f32,
            ) -> DrawList {
                DrawList::new()
            }

            fn is_canvas(&self) -> bool {
                true
            }
        }

        let mut app = App::new(None);
        let tab_id = app.push_entry_for_test(
            DocumentView::new(vec!["canvas".into()], 80, 10.0),
            Box::new(CanvasPlugin),
        );
        app.switch_workspace_for_test(0);

        assert!(!app.active_has_canvas_viewport_snapshot());

        let snapshot = app
            .tab_runtime_mut(tab_id)
            .expect("canvas tab should have a runtime")
            .canvas_viewport
            .prepare(
                ui::plugin::CanvasContentMetrics {
                    content_bounds: Rect::new(0.0, 0.0, 2_000.0, 1_500.0),
                    focus_anchor: None,
                },
                Rect::new(0.0, 0.0, 800.0, 600.0),
                ui::canvas::CanvasViewportConfig::for_dpi(1.0),
            );
        assert!(snapshot.is_some(), "canvas viewport fixture should prepare a snapshot");

        assert!(app.active_has_canvas_viewport_snapshot());
    }
}
