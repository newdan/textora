use crate::app::App;
use crate::app_effect::AppEffect;
use crate::canvas_viewport::CanvasViewportAction;
use crate::dispatch::chrome::SettingsDispatchAction;
use crate::workspace_tab_factory::{self, ProductPreparedTab};
use winit::event_loop::ActiveEventLoop;

const DEFAULT_LOGICAL_FONT_SIZE: f32 = 15.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct RecentFileViewState {
    cursor_line: usize,
    cursor_column: usize,
    scroll_anchor_line: usize,
    scroll_anchor_offset: f32,
}

impl RecentFileViewState {
    fn from_history_entry(entry: Option<&crate::file_history::FileHistoryEntry>) -> Self {
        let Some(entry) = entry else {
            return Self::default();
        };
        Self {
            cursor_line: entry.last_cursor_line,
            cursor_column: entry.last_cursor_col,
            scroll_anchor_line: entry.scroll_anchor_line,
            scroll_anchor_offset: entry.scroll_anchor_offset,
        }
    }
}

fn save_dialog_default_name(title: Option<String>) -> String {
    title.unwrap_or_else(|| "未命名".to_owned())
}

impl App {
    fn reset_canvas_view_or_font_size(&mut self) -> AppEffect {
        let active_plugin_is_canvas = self.active_is_canvas();
        if active_plugin_is_canvas {
            return self.apply_canvas_viewport_action(CanvasViewportAction::ResetView);
        }

        self.apply_zoom(DEFAULT_LOGICAL_FONT_SIZE)
    }

    fn open_recent_file_path(&mut self, path: &std::path::Path) -> Result<AppEffect, String> {
        let view_state = RecentFileViewState::from_history_entry(
            self.file_history.entries.iter().find(|entry| entry.file_path == path),
        );
        let workspace_effect = if let Some(index) = self.workspace.find_by_path(path) {
            self.workspace.switch_to(index)
        } else {
            let viewport = self.viewport_dimensions(self.screen_height());
            let ProductPreparedTab { prepared, suggested_file_name } =
                workspace_tab_factory::prepare_file(&self.workspace, path, viewport)?;
            self.workspace.open_prepared_tab(
                &mut self.tab_runtime_store,
                prepared,
                suggested_file_name,
            )
        };
        let app_effect = self.apply_workspace_effect(workspace_effect);

        if let Some(mut tab) = self.active_tab_session_mut() {
            let document = &mut tab.document;
            let target_line = view_state.cursor_line.min(document.line_count().saturating_sub(1));
            if let Some(line_offset) = document.line_byte_offset(target_line) {
                let column_offset = document
                    .doc_line_bytes(target_line)
                    .map(|line_bytes| {
                        let line = String::from_utf8_lossy(&line_bytes);
                        let target_column = view_state.cursor_column.min(line.chars().count());
                        line.char_indices()
                            .nth(target_column)
                            .map(|(byte_offset, _)| byte_offset)
                            .unwrap_or(line_bytes.len())
                    })
                    .unwrap_or(0);
                document.cursor_move_to_offset(line_offset + column_offset);
            }
            tab.set_scroll_anchor(view_state.scroll_anchor_line, view_state.scroll_anchor_offset);
        }

        Ok(app_effect)
    }

    /// Save the active tab. If `force_dialog` is true, always show the SaveAs
    /// dialog. Otherwise try a direct save first, falling back to the dialog
    /// when the file has no path (untitled).
    pub(crate) fn save_active_entry(&mut self, force_dialog: bool) -> AppEffect {
        let mut effect = AppEffect::NONE;
        let active_idx = self.workspace.active_index();

        if !force_dialog {
            // Try direct save — extract dirty flag to avoid borrow conflict
            let save_result = self.workspace.active_doc_mut().map(|dv| {
                let result = dv.save();
                (result, dv.dirty)
            });
            match save_result {
                Some((Ok(()), _)) => {
                    self.update_document_edited(false);
                    self.refresh_file_monitor_roots();
                    return effect.merge(AppEffect::UPDATE_TITLE).merge(AppEffect::REDRAW);
                }
                Some((Err(crate::document_view::DocumentSaveError::Untitled), _)) => {
                    // fall through to dialog
                }
                Some((Err(e), dirty)) => {
                    eprintln!("save error: {e}");
                    self.update_document_edited(dirty);
                    return effect.merge(AppEffect::REDRAW);
                }
                None => return effect,
            }
        }

        // SaveAs dialog
        let default_name = save_dialog_default_name(self.workspace.entry_title(active_idx));

        let mut dialog = rfd::FileDialog::new().set_file_name(&default_name);
        if let Some(ref w) = self.window {
            dialog = dialog.set_parent(w);
        }

        if let Some(path) = dialog.save_file() {
            let save_result = self.workspace.entry_doc_mut(active_idx).map(|dv| {
                let result = dv.save_as(&path);
                (result, dv.dirty)
            });
            if let Some((result, dirty)) = save_result {
                if let Err(ref e) = result {
                    eprintln!("另存失败: {e}");
                }
                if result.is_ok() {
                    self.workspace.clear_suggested_file_name(active_idx);
                }
                self.update_document_edited(dirty);
                if result.is_ok() {
                    self.refresh_file_monitor_roots();
                }
            }
            effect = effect.merge(AppEffect::UPDATE_TITLE);
        }
        effect.merge(AppEffect::REDRAW)
    }

    pub(crate) fn dispatch_app_command(
        &mut self,
        command: crate::menu_handler::AppCommand,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect {
        let mut effect = AppEffect::NONE;
        match command {
            crate::menu_handler::AppCommand::Noop => {}
            crate::menu_handler::AppCommand::Quit => self.quit_app(event_loop),
            crate::menu_handler::AppCommand::NewEmptyTab => {
                effect = effect.merge(self.new_untitled_doc());
            }
            crate::menu_handler::AppCommand::OpenFileDialog => {
                effect = effect.merge(self.open_file_dialog())
            }
            crate::menu_handler::AppCommand::SaveActiveTab => {
                effect = effect.merge(self.save_active_entry(false));
            }
            crate::menu_handler::AppCommand::SaveActiveTabAs => {
                effect = effect.merge(self.save_active_entry(true));
            }
            crate::menu_handler::AppCommand::CloseTab(_) => {
                if let Some(id) = self.workspace.tab_id_at(self.workspace.active_index()) {
                    effect = effect.merge(self.try_close_entry_with_prompt(id));
                }
            }
            crate::menu_handler::AppCommand::CloseOthers(idx) => {
                if let Some(id) = self.workspace.tab_id_at(idx) {
                    effect = effect.merge(self.try_close_multiple_with_prompt(
                        ui::popup_menu::ContextMenuAction::CloseOthers,
                        id,
                    ));
                }
            }
            crate::menu_handler::AppCommand::CloseRight(idx) => {
                if let Some(id) = self.workspace.tab_id_at(idx) {
                    effect = effect.merge(self.try_close_multiple_with_prompt(
                        ui::popup_menu::ContextMenuAction::CloseRight,
                        id,
                    ));
                }
            }
            crate::menu_handler::AppCommand::CloseAll => {
                if let Some(id) = self.workspace.tab_id_at(self.workspace.active_index()) {
                    effect = effect.merge(self.try_close_multiple_with_prompt(
                        ui::popup_menu::ContextMenuAction::CloseAll,
                        id,
                    ));
                }
            }
            crate::menu_handler::AppCommand::TogglePin => {
                let ws_effect = self.workspace.toggle_pin();
                effect = effect.merge(self.handle_nav_effect(ws_effect));
            }
            crate::menu_handler::AppCommand::ToggleFind => {
                if let Some(mut tab) = self.active_tab_session_mut() {
                    if tab.search_state().panel_visible {
                        self.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);
                    } else {
                        tab.search_state_mut().panel_visible = true;
                        self.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);
                    }
                    effect = effect.merge(AppEffect::REDRAW);
                }
            }
            crate::menu_handler::AppCommand::Edit(cmd) => {
                effect = effect.merge(self.dispatch_edit_command(cmd, event_loop));
            }
            crate::menu_handler::AppCommand::ClearRecentFiles => {
                self.file_history.entries.clear();
                self.save_history();
            }
            crate::menu_handler::AppCommand::OpenRecentFile(idx) => {
                let recent_path = {
                    let guard = crate::native_menu::RECENT_FILES
                        .lock()
                        .expect("recent-file menu cache mutex should not be poisoned");
                    guard.get(idx).cloned()
                };
                if let Some(path) = recent_path {
                    match self.open_recent_file_path(&path) {
                        Ok(open_effect) => effect = effect.merge(open_effect),
                        Err(e) => eprintln!("open recent file failed: {e}"),
                    }
                }
            }
            crate::menu_handler::AppCommand::ZoomIn => {
                effect = effect.merge(self.apply_zoom(self.settings.font_size + 1.0));
            }
            crate::menu_handler::AppCommand::ZoomOut => {
                effect = effect.merge(self.apply_zoom((self.settings.font_size - 1.0).max(6.0)));
            }
            crate::menu_handler::AppCommand::ZoomReset => {
                effect = effect.merge(self.reset_canvas_view_or_font_size());
            }
            crate::menu_handler::AppCommand::ToggleStatusBar => {
                effect = effect
                    .merge(self.dispatch_settings_action(SettingsDispatchAction::ToggleStatusBar));
            }
            crate::menu_handler::AppCommand::ToggleTabBar => {
                let new_mode = match self.settings.view_mode {
                    ui::view_mode::ViewMode::Tabs => ui::view_mode::ViewMode::Sidebar,
                    ui::view_mode::ViewMode::Sidebar => ui::view_mode::ViewMode::Tabs,
                };
                effect = effect.merge(
                    self.dispatch_settings_action(SettingsDispatchAction::SetViewMode(new_mode)),
                );
            }
            crate::menu_handler::AppCommand::OpenSettings => {
                effect = effect.merge(self.open_settings_overlay());
            }
            crate::menu_handler::AppCommand::SetThemeModeSystem => {
                effect = effect.merge(self.dispatch_settings_action(
                    SettingsDispatchAction::SetThemeMode(ui::settings::ThemeMode::System),
                ));
            }
            crate::menu_handler::AppCommand::SetThemeModeDark => {
                effect = effect.merge(self.dispatch_settings_action(
                    SettingsDispatchAction::SetThemeMode(ui::settings::ThemeMode::Dark),
                ));
            }
            crate::menu_handler::AppCommand::SetThemeModeLight => {
                effect = effect.merge(self.dispatch_settings_action(
                    SettingsDispatchAction::SetThemeMode(ui::settings::ThemeMode::Light),
                ));
            }
            crate::menu_handler::AppCommand::ToggleLineNumbers => {
                effect = effect.merge(
                    self.dispatch_settings_action(SettingsDispatchAction::ToggleLineNumbers),
                );
            }
            crate::menu_handler::AppCommand::ToggleWordWrap => {
                effect = effect
                    .merge(self.dispatch_settings_action(SettingsDispatchAction::ToggleWordWrap));
            }
            crate::menu_handler::AppCommand::SetViewModeSidebar => {
                effect = effect.merge(self.dispatch_settings_action(
                    SettingsDispatchAction::SetViewMode(ui::view_mode::ViewMode::Sidebar),
                ));
            }
            crate::menu_handler::AppCommand::SetViewModeTabs => {
                effect = effect.merge(self.dispatch_settings_action(
                    SettingsDispatchAction::SetViewMode(ui::view_mode::ViewMode::Tabs),
                ));
            }
        }
        effect
    }
}

#[cfg(test)]
mod recent_file_tests {
    use std::path::Path;

    use appkit_core::file_history::FileHistoryEntry;

    use super::App;

    fn history_entry(
        file_path: &Path,
        cursor_line: usize,
        cursor_column: usize,
        anchor_line: usize,
        anchor_offset: f32,
    ) -> FileHistoryEntry {
        FileHistoryEntry {
            file_path: file_path.to_owned(),
            workspace_root: None,
            last_closed_at: 1,
            last_cursor_line: cursor_line,
            last_cursor_col: cursor_column,
            scroll_anchor_line: anchor_line,
            scroll_anchor_offset: anchor_offset,
        }
    }

    #[test]
    fn recent_file_new_open_restores_view_state_and_installs_product_runtime() {
        let directory = tempfile::tempdir().expect("recent-file test directory should exist");
        let path = directory.path().join("recent.md");
        std::fs::write(&path, "zero\none\ntwo\nlast")
            .expect("recent-file fixture should be writable");
        let mut app = App::new(None);
        app.file_history.entries = vec![history_entry(&path, 2, 2, 3, 8.5)];

        let effect =
            app.open_recent_file_path(&path).expect("new recent file should prepare and open");

        let active_id = app.active_tab_id().expect("recent file should become active");
        {
            let tab = app
                .active_tab_session()
                .expect("recent file should have a matching product runtime");
            assert_eq!(tab.document.cursor_line(), 2);
            assert_eq!(tab.document.cursor_column(), 2);
            assert_eq!(tab.scroll_anchor_doc_line(), 3);
            assert_eq!(tab.scroll_anchor_pixel_offset(), 8.5);
            assert_eq!(tab.plugin_name(), ui::plugin::PLUGIN_MARKDOWN_EDITOR);
        }
        assert!(app.tab_runtime_store.contains(active_id));
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());
        assert!(effect.reshape);
        assert!(effect.redraw);
        assert!(effect.update_title);
        assert!(effect.persist_workspace);
    }

    #[test]
    fn recent_file_existing_tab_short_circuits_loading_and_restores_view_state() {
        let directory = tempfile::tempdir().expect("recent-file test directory should exist");
        let path = directory.path().join("existing.txt");
        std::fs::write(&path, "first\nsecond\nthird")
            .expect("recent-file fixture should be writable");
        let mut app = App::new(None);
        app.open_file(&path).expect("fixture file should open");
        let existing_id = app.active_tab_id().expect("fixture file should have a tab id");
        app.new_untitled_doc();
        let tab_count_before_reopen = app.workspace.len();
        app.file_history.entries = vec![history_entry(&path, 1, 3, 2, 4.25)];
        std::fs::remove_file(&path)
            .expect("source deletion should prove existing-tab short-circuiting");

        let effect = app
            .open_recent_file_path(&path)
            .expect("existing recent tab should reactivate without reading the file");

        assert_eq!(app.workspace.len(), tab_count_before_reopen);
        assert_eq!(app.active_tab_id(), Some(existing_id));
        {
            let tab = app.active_tab_session().expect("existing runtime should remain installed");
            assert_eq!(tab.document.full_text(), "first\nsecond\nthird");
            assert_eq!(tab.document.cursor_line(), 1);
            assert_eq!(tab.document.cursor_column(), 3);
            assert_eq!(tab.scroll_anchor_doc_line(), 2);
            assert_eq!(tab.scroll_anchor_pixel_offset(), 4.25);
        }
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());
        assert!(effect.reshape);
        assert!(effect.redraw);
        assert!(effect.update_title);
        assert!(effect.persist_workspace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas_viewport::CanvasViewportAction;
    use crate::document_view::DocumentView;

    struct CanvasCommandPlugin;

    impl ui::plugin::ViewPlugin for CanvasCommandPlugin {
        fn name(&self) -> &str {
            "canvas_command"
        }

        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }

        fn is_canvas(&self) -> bool {
            true
        }
    }

    fn app_with_prepared_canvas_viewport() -> App {
        let mut app = App::new(None);
        let document = DocumentView::new(vec!["canvas".to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(CanvasCommandPlugin));
        app.switch_workspace_for_test(0);

        let tab = app.active_tab_session_mut().expect("test canvas tab must be active");
        let snapshot = tab.runtime.canvas_viewport.prepare(
            ui::plugin::CanvasContentMetrics {
                content_bounds: ui::core::geom::Rect::new(0.0, 0.0, 5_000.0, 5_000.0),
                focus_anchor: None,
            },
            ui::core::geom::Rect::new(0.0, 0.0, 1_000.0, 800.0),
            ui::canvas::CanvasViewportConfig::for_dpi(1.0),
        );
        assert!(snapshot.is_some(), "test canvas viewport must prepare a snapshot");
        app
    }

    #[test]
    fn canvas_zoom_reset_restores_view_without_changing_global_font_size() {
        let mut app = app_with_prepared_canvas_viewport();
        let initial = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        let font_size_before = app.settings.font_size;
        app.apply_canvas_viewport_action(CanvasViewportAction::ZoomBy {
            factor: 1.25,
            screen_anchor: ui::canvas::CanvasPoint::new(500.0, 400.0),
        });

        assert_eq!(app.reset_canvas_view_or_font_size(), AppEffect::REDRAW);

        let reset = app
            .active_tab_session()
            .expect("test canvas tab must be active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared canvas viewport must retain a snapshot");
        assert_eq!(reset, initial);
        assert_eq!(app.settings.font_size, font_size_before);
    }

    #[test]
    fn non_canvas_zoom_reset_keeps_global_font_size_semantics() {
        let mut app = App::new(None);
        app.settings.font_size = 20.0;

        let effect = app.reset_canvas_view_or_font_size();

        assert!(effect.reshape);
        assert_eq!(app.settings.font_size, 15.0);
    }

    #[test]
    fn save_dialog_default_name_uses_typed_suggestion() {
        assert_eq!(save_dialog_default_name(Some("未命名.mmap.md".to_owned())), "未命名.mmap.md");
    }

    #[test]
    fn save_dialog_default_name_falls_back_when_title_is_absent() {
        assert_eq!(save_dialog_default_name(None), "未命名");
    }
}
