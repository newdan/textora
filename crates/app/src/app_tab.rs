//! Tab management: open, close, history, workspace effects.
//! Methods on `impl App`, extracted from app.rs.

use crate::app::App;
use crate::file_history::{FileHistoryEntry, compute_workspace_root};
use crate::tab_runtime::TabRuntime;
use crate::tab_session::{TabSession, TabSessionMut};
use crate::workspace::Workspace;
use crate::workspace_persistence::snapshot_workspace;
use crate::workspace_product::history_entry;
use appkit_core::workspace::types::TabId;
#[cfg(test)]
use appkit_shell::prepared_tab::PreparedTab;

pub(crate) fn compose_tab_session<'a>(
    workspace: &'a Workspace,
    runtime_store: &'a crate::tab_runtime::TabRuntimeStore,
    id: TabId,
) -> Option<TabSession<'a>> {
    let index = workspace.index_of(id)?;
    let entry = workspace.entry(index)?;
    let runtime = runtime_store.get(id)?;

    Some(TabSession::new(id, entry, runtime))
}

pub(crate) fn compose_tab_session_mut<'a>(
    workspace: &'a mut Workspace,
    runtime_store: &'a mut crate::tab_runtime::TabRuntimeStore,
    id: TabId,
) -> Option<TabSessionMut<'a>> {
    let index = workspace.index_of(id)?;

    let runtime = runtime_store.get_mut(id)?;
    let document = workspace.entry_mut(index)?;
    Some(TabSessionMut::new(id, document, runtime))
}

impl App {
    pub(crate) fn active_tab_id(&self) -> Option<TabId> {
        self.workspace.tab_id_at(self.workspace.active_index())
    }

    pub(crate) fn tab_runtime(&self, id: TabId) -> Option<&TabRuntime> {
        self.tab_runtime_store.get(id)
    }

    pub(crate) fn tab_runtime_mut(&mut self, id: TabId) -> Option<&mut TabRuntime> {
        self.tab_runtime_store.get_mut(id)
    }

    pub(crate) fn tab_session(&self, id: TabId) -> Option<TabSession<'_>> {
        compose_tab_session(&self.workspace, &self.tab_runtime_store, id)
    }

    pub(crate) fn tab_session_mut(&mut self, id: TabId) -> Option<TabSessionMut<'_>> {
        compose_tab_session_mut(&mut self.workspace, &mut self.tab_runtime_store, id)
    }

    pub(crate) fn active_tab_session(&self) -> Option<TabSession<'_>> {
        self.active_tab_id().and_then(|id| self.tab_session(id))
    }

    pub(crate) fn active_tab_session_mut(&mut self) -> Option<TabSessionMut<'_>> {
        self.active_tab_id().and_then(|id| self.tab_session_mut(id))
    }

    pub(crate) fn switch_active_plugin(&mut self) {
        self.workspace.switch_plugin_with_runtime(&mut self.tab_runtime_store);
    }

    #[cfg(test)]
    pub(crate) fn push_entry_for_test(
        &mut self,
        document: crate::document_view::DocumentView,
        plugin: Box<dyn ui::plugin::ViewPlugin>,
    ) -> TabId {
        let (document, presentation) = document.into_parts();
        let runtime = TabRuntime::with_presentation(plugin, presentation);
        self.workspace.append_prepared_tab(
            &mut self.tab_runtime_store,
            PreparedTab::new(document, runtime),
            None,
        )
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

    pub(crate) fn active_is_toggled(&self) -> bool {
        self.active_plugin_name()
            .is_some_and(|plugin_name| self.workspace.is_toggled_for_plugin(plugin_name))
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
    pub(crate) fn persist_workspace_state(&self) {
        let snap = snapshot_workspace(
            &self.workspace,
            &self.tab_runtime_store,
            self.ui_shell.sidebar_pinned(),
            Some(self.ui_shell.sidebar_width()),
            &self.paths.snapshots_dir,
        );
        if let Err(e) = self.workspace_store.save_workspace(&snap) {
            eprintln!("[workspace] save_workspace error: {}", e);
        }
        self.save_pinned_paths();
    }

    pub(crate) fn save_pinned_paths(&self) {
        let paths = self.workspace.pinned_paths();
        if let Err(e) = self.workspace_store.save_pinned_paths(&paths) {
            eprintln!("[workspace] save_pinned_paths error: {}", e);
        }
    }

    /// Record a tab to file history before closing.
    pub(crate) fn record_entry_to_history(&mut self, index: usize) {
        let scroll_anchor = self
            .workspace
            .tab_id_at(index)
            .and_then(|id| self.tab_session(id))
            .map(|tab| tab.scroll_anchor_state())
            .unwrap_or_else(|| ui::viewport::ScrollAnchor::new(0, 0.0));
        if let Some(mut entry) = history_entry(&self.workspace, index, scroll_anchor) {
            let paths: Vec<&std::path::Path> = self
                .workspace
                .entries()
                .iter()
                .filter_map(|v| v.value.file_path.as_deref())
                .collect();
            entry.workspace_root = compute_workspace_root(&paths);
            self.file_history.record(entry);
        }
    }

    /// Record all open tabs to history (on quit).
    pub(crate) fn record_all_entries_to_history(&mut self) {
        let paths: Vec<&std::path::Path> = self
            .workspace
            .entries()
            .iter()
            .map(|t| &t.value)
            .filter_map(|dv| dv.file_path.as_deref())
            .collect();
        let ws_root = compute_workspace_root(&paths);
        let entries: Vec<FileHistoryEntry> = self
            .workspace
            .entries()
            .iter()
            .filter_map(|entry| {
                let doc = &entry.value;
                let fp = doc.file_path.clone()?;
                let scroll_anchor = self
                    .tab_session(entry.id)
                    .map(|tab| tab.scroll_anchor_state())
                    .unwrap_or_else(|| ui::viewport::ScrollAnchor::new(0, 0.0));
                Some(FileHistoryEntry {
                    file_path: fp,
                    workspace_root: ws_root.clone(),
                    last_closed_at: 0,
                    last_cursor_line: doc.cursor_line(),
                    last_cursor_col: doc.cursor_column(),
                    scroll_anchor_line: scroll_anchor.doc_line,
                    scroll_anchor_offset: scroll_anchor.pixel_offset,
                })
            })
            .collect();
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
            if let Some(window) = &self.window {
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
        if let Some(ref w) = self.window {
            if let Some(dv) = self.workspace.active_doc() {
                let title = format!(
                    "{} — edit+",
                    dv.file_path
                        .as_ref()
                        .map(|p| p.file_name().unwrap_or_default().to_string_lossy())
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
        app.tab_runtime_store
            .insert(tab_id, crate::tab_runtime::TabRuntime::new(Box::new(EditorPlugin::new())));
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
