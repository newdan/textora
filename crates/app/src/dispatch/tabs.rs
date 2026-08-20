use crate::app::App;
use crate::app_effect::AppEffect;
use crate::workspace_tab_factory::ProductPreparedTab;
use appkit_core::workspace::types::TabId;
use appkit_shell::editor_runtime::OpenDisposition;

#[derive(Debug, PartialEq, Eq)]
enum BatchCloseSaveTarget {
    Clean,
    ExistingPath(std::path::PathBuf),
    SaveAs(String),
}

fn batch_close_save_target(
    entry: Option<crate::app_tab::EditorSaveContext>,
) -> Option<BatchCloseSaveTarget> {
    let entry = entry?;
    if !entry.dirty {
        return Some(BatchCloseSaveTarget::Clean);
    }

    match entry.file_path {
        Some(path) => Some(BatchCloseSaveTarget::ExistingPath(path)),
        None => Some(BatchCloseSaveTarget::SaveAs(entry.title)),
    }
}

impl App {
    pub(crate) fn apply_workspace_effect(
        &mut self,
        effect: crate::workspace::WorkspaceEffect,
    ) -> AppEffect {
        let effect = match effect {
            crate::workspace::WorkspaceEffect::Closed { activated: None, .. }
                if self.editor_is_empty() =>
            {
                let viewport = self.viewport_dimensions(self.screen_height());
                let ProductPreparedTab { prepared, suggested_file_name } =
                    self.prepare_editor_untitled(viewport);
                self.install_editor_tab(
                    prepared,
                    suggested_file_name,
                    appkit_shell::editor_runtime::OpenDisposition::Persistent,
                )
            }
            other => other,
        };

        self.handle_nav_effect(effect.nav_effect())
    }

    fn finish_active_workspace_change(&mut self) -> AppEffect {
        self.hydrate_active_editor_stub();
        if !self.editor_is_empty() {
            let screen_height = self.screen_height();
            let visible_rows = self.visible_rows(screen_height);
            let viewport_height = self.visible_height_lines(screen_height);
            if let Some(mut tab) = self.active_tab_session_mut() {
                tab.resize_presentation(visible_rows, viewport_height);
                tab.clear_advance_cache();
            }
            if let Some(active_index) = self.active_editor_index() {
                self.init_display_map(active_index);
            }
        }
        self.editor_runtime.clear_frame_cluster_pool();
        let layout_effect = self.update_entry_layout();
        AppEffect::RESHAPE
            .merge(layout_effect)
            .merge(AppEffect::REDRAW)
            .merge(AppEffect::UPDATE_TITLE)
            .merge(AppEffect::PERSIST_WORKSPACE)
    }

    pub(crate) fn handle_nav_effect(&mut self, effect: crate::navigator::NavEffect) -> AppEffect {
        let app_effect = match effect {
            crate::navigator::NavEffect::ActiveChanged => self.finish_active_workspace_change(),
            crate::navigator::NavEffect::ItemsChanged => {
                let layout_effect = self.update_entry_layout();
                layout_effect.merge(AppEffect::REDRAW).merge(AppEffect::PERSIST_WORKSPACE)
            }
            crate::navigator::NavEffect::None => AppEffect::NONE,
        };
        self.refresh_file_monitor_roots();
        app_effect
    }

    pub(crate) fn update_entry_layout(&mut self) -> AppEffect {
        if self.tab_scroll.is_animating() { AppEffect::REDRAW } else { AppEffect::NONE }
    }

    pub(crate) fn open_file(&mut self, path: &std::path::Path) -> Result<AppEffect, String> {
        let viewport = self.viewport_dimensions(self.screen_height());
        let effect = if let Some(tab_id) = self.editor_tab_id_for_path(path) {
            self.activate_editor_tab(tab_id).unwrap_or(crate::workspace::WorkspaceEffect::None)
        } else {
            let ProductPreparedTab { prepared, suggested_file_name } =
                self.prepare_editor_file(path, viewport)?;
            self.install_editor_tab(prepared, suggested_file_name, OpenDisposition::Persistent)
        };
        let app_effect = self.apply_workspace_effect(effect);
        if let Some(active_index) = self.active_editor_index() {
            self.record_entry_to_history(active_index);
        }
        self.rebuild_native_menu();

        Ok(app_effect)
    }

    pub(crate) fn open_file_dialog(&mut self) -> AppEffect {
        let mut app_effect = AppEffect::NONE;
        let text_extensions = &[
            "txt",
            "md",
            "markdown",
            "rst",
            "rs",
            "py",
            "js",
            "ts",
            "jsx",
            "tsx",
            "go",
            "java",
            "c",
            "cpp",
            "cc",
            "cxx",
            "h",
            "hpp",
            "rb",
            "php",
            "swift",
            "kt",
            "kts",
            "scala",
            "clj",
            "elm",
            "lua",
            "r",
            "html",
            "htm",
            "css",
            "scss",
            "less",
            "sass",
            "json",
            "xml",
            "yaml",
            "yml",
            "toml",
            "ini",
            "cfg",
            "conf",
            "properties",
            "sh",
            "bash",
            "zsh",
            "fish",
            "bat",
            "ps1",
            "sql",
            "csv",
            "tsv",
            "log",
            "tex",
            "bib",
            "vue",
            "svelte",
            "astro",
            "Makefile",
            "Dockerfile",
            "gitignore",
            "env",
        ];
        let result = rfd::FileDialog::new()
            .add_filter("Text Files", text_extensions)
            .add_filter("All Files", &["*"])
            .pick_files();
        if let Some(paths) = result {
            for path in &paths {
                match self.open_file(path) {
                    Ok(effect) => app_effect = app_effect.merge(effect),
                    Err(e) => eprintln!("Error opening file: {e}"),
                }
            }
        }
        app_effect
    }

    pub(crate) fn new_untitled_doc(&mut self) -> AppEffect {
        let viewport = self.viewport_dimensions(self.screen_height());
        let ProductPreparedTab { prepared, suggested_file_name } =
            self.prepare_editor_untitled(viewport);
        let effect =
            self.install_editor_tab(prepared, suggested_file_name, OpenDisposition::Persistent);
        self.apply_workspace_effect(effect)
    }

    pub(crate) fn new_typed_untitled_doc(
        &mut self,
        kind: ui::sidebar::NewDocumentKind,
    ) -> AppEffect {
        if kind == ui::sidebar::NewDocumentKind::EncryptedMarkdown {
            return AppEffect::NONE;
        }
        let viewport = self.viewport_dimensions(self.screen_height());
        let ProductPreparedTab { prepared, suggested_file_name } =
            self.prepare_typed_editor_untitled(kind, viewport);
        let effect =
            self.install_editor_tab(prepared, suggested_file_name, OpenDisposition::Persistent);
        self.apply_workspace_effect(effect)
    }

    pub(crate) fn try_close_entry_with_prompt(&mut self, id: TabId) -> AppEffect {
        let Some(idx) = self.editor_tab_index(id) else {
            return AppEffect::NONE;
        };
        use crate::workspace::CloseTabDecision;
        let mut app_effect = AppEffect::NONE;
        let decision = self
            .editor_close_decision(id)
            .expect("tab index lookup should make close decision available");
        match decision {
            CloseTabDecision::CanClose => {
                self.record_entry_to_history(idx);
                if let Some(effect) = self.close_editor_tab(id) {
                    app_effect = app_effect.merge(self.apply_workspace_effect(effect));
                }
                self.save_history();
                self.rebuild_native_menu();
                app_effect = app_effect.merge(AppEffect::REDRAW);
            }
            CloseTabDecision::Pinned => {}
            CloseTabDecision::NeedsSavePrompt => {
                let file_name = self
                    .editor_save_context(id)
                    .map(|context| context.title)
                    .unwrap_or_else(|| "未命名".to_owned());
                let msg = format!("是否保存对「{}」的更改？", file_name);
                let mut dialog = rfd::MessageDialog::new()
                    .set_title("未保存的更改")
                    .set_description(&msg)
                    .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                        "保存".to_string(),
                        "放弃".to_string(),
                        "取消".to_string(),
                    ))
                    .set_level(rfd::MessageLevel::Warning);
                if let Some(w) = self.editor_runtime.window() {
                    dialog = dialog.set_parent(w);
                }
                let result = dialog.show();
                match result {
                    rfd::MessageDialogResult::Custom(ref label) if label == "保存" => {
                        let need_save_as = self
                            .editor_save_context(id)
                            .is_some_and(|context| context.file_path.is_none());
                        if need_save_as {
                            let default_name = self
                                .editor_save_context(id)
                                .map(|context| context.title)
                                .unwrap_or_else(|| "未命名".to_owned());
                            if let Some(path) =
                                rfd::FileDialog::new().set_file_name(&default_name).save_file()
                            {
                                if let Err(e) =
                                    self.submit_editor_save_before_close(id, Some(&path))
                                {
                                    eprintln!("保存失败: {e}");
                                    return app_effect;
                                }
                            } else {
                                return app_effect;
                            }
                        } else if let Err(e) = self.submit_editor_save_before_close(id, None) {
                            eprintln!("保存失败: {e}");
                            return app_effect;
                        }
                        app_effect = app_effect.merge(AppEffect::REDRAW);
                    }
                    rfd::MessageDialogResult::Custom(ref label) if label == "放弃" => {
                        self.record_entry_to_history(idx);
                        if let Some(effect) = self.close_editor_tab(id) {
                            app_effect = app_effect.merge(self.apply_workspace_effect(effect));
                        }
                        self.save_history();
                        self.rebuild_native_menu();
                        app_effect = app_effect.merge(AppEffect::REDRAW);
                    }
                    _ => {}
                }
            }
        }
        app_effect
    }

    pub(crate) fn try_close_multiple_with_prompt(
        &mut self,
        action: ui::popup_menu::ContextMenuAction,
        id: TabId,
    ) -> AppEffect {
        let mut app_effect = AppEffect::NONE;
        let Some(tab_index) = self.editor_tab_index(id) else {
            return app_effect;
        };
        let indices: Vec<usize> = match action {
            ui::popup_menu::ContextMenuAction::CloseOthers => (0..self.editor_tab_count())
                .filter(|&i| i != tab_index && !self.is_editor_tab_pinned_at(i))
                .collect(),
            ui::popup_menu::ContextMenuAction::CloseRight => ((tab_index + 1)
                ..self.editor_tab_count())
                .filter(|i| !self.is_editor_tab_pinned_at(*i))
                .collect(),
            ui::popup_menu::ContextMenuAction::CloseAll => {
                (0..self.editor_tab_count()).filter(|i| !self.is_editor_tab_pinned_at(*i)).collect()
            }
            _ => return app_effect,
        };
        let mut clean_tab_ids = Vec::new();

        let dirty_count = indices
            .iter()
            .filter(|&&i| {
                self.editor_tab_id_at(i).is_some_and(|tab_id| {
                    self.editor_save_context(tab_id).is_some_and(|context| context.dirty)
                })
            })
            .count();
        if dirty_count > 0 {
            let msg = format!("有 {} 个文件包含未保存的更改。\n是否保存后再关闭？", dirty_count);
            let mut dialog = rfd::MessageDialog::new()
                .set_title("未保存的更改")
                .set_description(&msg)
                .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                    "全部保存".to_string(),
                    "全部放弃".to_string(),
                    "取消".to_string(),
                ))
                .set_level(rfd::MessageLevel::Warning);
            if let Some(w) = self.editor_runtime.window() {
                dialog = dialog.set_parent(w);
            }
            let result = dialog.show();
            match result {
                rfd::MessageDialogResult::Custom(ref label) if label == "全部保存" => {
                    for &i in &indices {
                        let close_context = self
                            .editor_tab_id_at(i)
                            .and_then(|tab_id| self.editor_save_context(tab_id));
                        let Some(target) = batch_close_save_target(close_context) else {
                            return app_effect;
                        };
                        match target {
                            BatchCloseSaveTarget::Clean => {
                                if let Some(tab_id) = self.editor_tab_id_at(i) {
                                    clean_tab_ids.push(tab_id);
                                }
                            }
                            BatchCloseSaveTarget::ExistingPath(path) => {
                                let Some(tab_id) = self.editor_tab_id_at(i) else {
                                    return app_effect;
                                };
                                if let Err(error) =
                                    self.submit_editor_save_before_close(tab_id, Some(&path))
                                {
                                    eprintln!("保存失败: {error}");
                                    return app_effect;
                                }
                            }
                            BatchCloseSaveTarget::SaveAs(default_name) => {
                                let Some(path) =
                                    rfd::FileDialog::new().set_file_name(&default_name).save_file()
                                else {
                                    return app_effect;
                                };
                                let Some(tab_id) = self.editor_tab_id_at(i) else {
                                    return app_effect;
                                };
                                if let Err(error) =
                                    self.submit_editor_save_before_close(tab_id, Some(&path))
                                {
                                    eprintln!("保存失败: {error}");
                                    return app_effect;
                                }
                            }
                        }
                    }
                }
                rfd::MessageDialogResult::Custom(ref label) if label == "全部放弃" => {}
                _ => return app_effect,
            }
        }

        if !self.pending_close_after_save.is_empty() {
            for tab_id in clean_tab_ids {
                if let Some(index) = self.editor_tab_index(tab_id) {
                    self.record_entry_to_history(index);
                    if let Some(effect) = self.close_editor_tab(tab_id) {
                        app_effect = app_effect.merge(self.apply_workspace_effect(effect));
                    }
                }
            }
            self.save_history();
            self.rebuild_native_menu();
            return app_effect.merge(AppEffect::REDRAW);
        }

        for &i in indices.iter().rev() {
            self.record_entry_to_history(i);
            let Some(tab_id) = self.editor_tab_id_at(i) else {
                continue;
            };
            if let Some(effect) = self.close_editor_tab(tab_id) {
                app_effect = app_effect.merge(self.apply_workspace_effect(effect));
            }
        }
        self.save_history();
        self.rebuild_native_menu();
        app_effect.merge(AppEffect::REDRAW)
    }
    pub(crate) fn execute_batch_close(&mut self, indices: &[usize]) -> AppEffect {
        if indices.is_empty() {
            return AppEffect::NONE;
        }
        let mut sorted = indices.to_vec();
        sorted.sort_by(|left, right| right.cmp(left));
        for &index in &sorted {
            self.record_entry_to_history(index);
        }
        let mut app_effect = AppEffect::NONE;
        for &index in &sorted {
            let Some(tab_id) = self.editor_tab_id_at(index) else {
                continue;
            };
            if let Some(next) = self.close_editor_tab(tab_id) {
                app_effect = app_effect.merge(self.apply_workspace_effect(next));
            }
        }
        self.save_history();
        self.rebuild_native_menu();
        app_effect.merge(AppEffect::REDRAW)
    }

    pub(crate) fn open_settings_file(&mut self) -> AppEffect {
        let path = self.paths.settings_file.clone();
        if let Err(error) = crate::settings_io::ensure_exists(&path) {
            eprintln!("[settings] save error: {error}");
            return AppEffect::NONE;
        }
        match self.open_file(&path) {
            Ok(effect) => effect,
            Err(error) => {
                eprintln!("Failed to open settings.toml: {error}");
                AppEffect::NONE
            }
        }
    }

    pub(crate) fn dispatch_context_menu_action(
        &mut self,
        action: ui::popup_menu::ContextMenuAction,
        id: TabId,
    ) -> AppEffect {
        use ui::popup_menu::ContextMenuAction;
        match action {
            ContextMenuAction::Close => self.try_close_entry_with_prompt(id),
            ContextMenuAction::CloseOthers
            | ContextMenuAction::CloseRight
            | ContextMenuAction::CloseAll => self.try_close_multiple_with_prompt(action, id),
            ContextMenuAction::CopyPath => {
                if !self.copy_editor_path(id) {
                    return AppEffect::NONE;
                }
                AppEffect::NONE
            }
            ContextMenuAction::TogglePin => {
                let Some(workspace_effect) = self.toggle_editor_pin(id) else {
                    return AppEffect::NONE;
                };
                self.handle_nav_effect(workspace_effect)
            }
        }
    }

    fn load_file_with_dimensions(
        &mut self,
        path: &std::path::Path,
        dimensions: crate::workspace::ViewportDimensions,
    ) -> Result<TabId, String> {
        let ProductPreparedTab { prepared, suggested_file_name } =
            self.prepare_editor_file(path, dimensions)?;
        let title = format!("{} — edit+", path.display());
        if let Some(window) = self.editor_runtime.window() {
            window.set_title(&title);
        }
        let id = self.append_editor_tab(prepared, suggested_file_name);
        let appended_index = self
            .editor_tab_index(id)
            .expect("a startup tab must remain installed until display-map initialization");
        self.init_display_map(appended_index);
        Ok(id)
    }

    pub(crate) fn load_file(&mut self) {
        let Some(path) = self.file_path.clone() else {
            return;
        };
        let Some((_, screen_height)) = self.editor_runtime.surface_size() else {
            return;
        };
        let visible_rows = self.visible_rows(screen_height as f32);
        let viewport_height = self.visible_height_lines(screen_height as f32);
        let dimensions = crate::workspace::ViewportDimensions { visible_rows, viewport_height };
        let _ = self.load_file_with_dimensions(&path, dimensions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;
    use crate::workspace::ViewportDimensions;

    fn test_viewport() -> ViewportDimensions {
        ViewportDimensions { visible_rows: 22, viewport_height: 22.0 }
    }

    fn open_untitled_fixture(app: &mut App) {
        let ProductPreparedTab { prepared, suggested_file_name } =
            app.prepare_editor_untitled(test_viewport());
        let _ = app.install_editor_tab(prepared, suggested_file_name, OpenDisposition::Persistent);
    }

    fn app_with_file_stub_and_active_document(file_path: &std::path::Path) -> App {
        let mut app = App::new(None);
        let mut stub = DocumentView::new(vec![String::new()], 10, 160.0);
        stub.file_path = Some(file_path.to_owned());
        app.push_entry_for_test(stub, Box::new(EditorPlugin::new()));
        let active = DocumentView::new(vec!["active".to_owned()], 10, 160.0);
        app.push_entry_for_test(active, Box::new(EditorPlugin::new()));
        let switch_effect = app
            .activate_editor_tab(app.editor_tab_id_at(1).expect("active tab index"))
            .expect("tab switch should produce an effect");
        app.apply_workspace_effect(switch_effect);
        app
    }

    #[test]
    fn nav_effect_handler_hydrates_only_after_workspace_go_back_returns() {
        let directory = tempfile::tempdir().expect("navigation hydration directory should exist");
        let file_path = directory.path().join("back.txt");
        std::fs::write(&file_path, "loaded by back navigation")
            .expect("navigation hydration fixture should be written");
        let mut app = app_with_file_stub_and_active_document(&file_path);

        let nav_effect = app.navigate_editor_back();

        assert_eq!(nav_effect, crate::navigator::NavEffect::ActiveChanged);
        assert_eq!(app.active_tab_session().expect("stub should become active").buffer_len(), 0);

        let app_effect = app.handle_nav_effect(nav_effect);

        assert_eq!(
            app.active_tab_session().expect("active stub should hydrate").full_text(),
            "loaded by back navigation"
        );
        assert!(app_effect.reshape);
        assert!(app_effect.redraw);
    }

    #[test]
    fn workspace_effect_handler_hydrates_only_after_active_close_returns() {
        let directory = tempfile::tempdir().expect("close hydration directory should exist");
        let file_path = directory.path().join("close.txt");
        std::fs::write(&file_path, "loaded after close")
            .expect("close hydration fixture should be written");
        let mut app = app_with_file_stub_and_active_document(&file_path);

        let workspace_effect = app
            .close_editor_tab(app.editor_tab_id_at(1).expect("active tab index"))
            .expect("active document should close cleanly");

        assert_eq!(app.active_tab_session().expect("stub should become active").buffer_len(), 0);

        let app_effect = app.apply_workspace_effect(workspace_effect);

        assert_eq!(
            app.active_tab_session().expect("active stub should hydrate").full_text(),
            "loaded after close"
        );
        assert!(app_effect.reshape);
        assert!(app_effect.redraw);
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );
    }

    #[test]
    fn new_untitled_promotes_runtime_into_store() {
        let mut app = App::new(None);

        let effect = app.new_untitled_doc();

        let id = app.active_tab_id().expect("new tab id");
        assert!(app.tab_runtime(id).is_some());
        assert!(effect.reshape);
        assert!(effect.redraw);
        assert!(effect.update_title);
        assert!(effect.persist_workspace);
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );
    }

    #[test]
    fn copy_path_for_untitled_tab_is_a_noop() {
        let mut app = App::new(None);
        app.new_untitled_doc();
        let tab_id = app.active_tab_id().expect("untitled tab should have an ID");

        let effect =
            app.dispatch_context_menu_action(ui::popup_menu::ContextMenuAction::CopyPath, tab_id);

        assert_eq!(effect, AppEffect::NONE);
        assert_eq!(app.editor_tab_count(), 1);
        assert_eq!(app.active_tab_id(), Some(tab_id));
    }

    #[test]
    fn toggle_pin_context_action_applies_workspace_navigation_effect() {
        let mut app = App::new(None);
        app.new_untitled_doc();
        let tab_id = app.active_tab_id().expect("untitled tab should have an ID");

        let effect =
            app.dispatch_context_menu_action(ui::popup_menu::ContextMenuAction::TogglePin, tab_id);

        assert!(app.is_editor_tab_pinned_at(0));
        assert!(effect.redraw);
        assert!(effect.persist_workspace);
    }

    #[test]
    fn open_file_activates_new_product_tab_and_reuses_existing_tab() {
        let directory = tempfile::tempdir().expect("open-file test directory should exist");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "# Product tab").expect("open-file fixture should be writable");
        let mut app = App::new(None);

        let first_effect = app.open_file(&path).expect("new file should open");
        let opened_id = app.active_tab_id().expect("opened file should become active");

        assert!(first_effect.reshape);
        assert!(first_effect.redraw);
        assert!(first_effect.update_title);
        assert!(first_effect.persist_workspace);
        assert_eq!(app.editor_tab_count(), 1);
        assert_eq!(
            app.active_tab_session().expect("opened file runtime should exist").plugin_name(),
            ui::plugin::PLUGIN_MARKDOWN_EDITOR
        );
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        app.new_untitled_doc();
        let len_before_reopen = app.editor_tab_count();
        std::fs::remove_file(&path).expect("existing-tab reopen must not need the source file");
        let reopen_effect = app.open_file(&path).expect("existing file should reactivate");

        assert!(reopen_effect.reshape);
        assert!(reopen_effect.redraw);
        assert!(reopen_effect.update_title);
        assert!(reopen_effect.persist_workspace);
        assert_eq!(app.editor_tab_count(), len_before_reopen);
        assert_eq!(app.active_tab_id(), Some(opened_id));
        assert_eq!(
            app.active_tab_session()
                .expect("existing file runtime should remain")
                .document
                .full_text(),
            "# Product tab"
        );
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );
    }

    #[test]
    fn startup_file_append_preserves_active_tab_navigation_and_effect_state() {
        let directory = tempfile::tempdir().expect("startup-file test directory should exist");
        let path = directory.path().join("startup.txt");
        std::fs::write(&path, "startup").expect("startup fixture should be writable");
        let mut app = App::new(None);
        app.new_untitled_doc();
        app.new_untitled_doc();
        let forward_target = app.active_tab_id();
        let back_effect = app.navigate_editor_back();
        let _ = app.handle_nav_effect(back_effect);
        let active_before_append = app.active_tab_id();
        let had_back_history = app.editor_has_back_history();
        let had_forward_history = app.editor_has_forward_history();
        app.needs_redraw = false;
        let _ = app.editor_runtime.take_skip_next_reshape_submit();

        let appended_id = app
            .load_file_with_dimensions(&path, test_viewport())
            .expect("startup file should append");
        let appended_index =
            app.editor_tab_index(appended_id).expect("startup file should remain appended");
        let appended_title = app.editor_save_context(appended_id).map(|context| context.title);

        assert_eq!(app.active_tab_id(), active_before_append);
        assert_eq!(app.editor_has_back_history(), had_back_history);
        assert_eq!(app.editor_has_forward_history(), had_forward_history);
        assert!(!app.needs_redraw);
        assert!(app.editor_runtime.take_skip_next_reshape_submit());
        assert_eq!(appended_index, app.editor_tab_count() - 1);
        let appended =
            app.tab_session(appended_id).expect("startup file model and runtime should be paired");
        assert_eq!(appended.document.file_path.as_deref(), Some(path.as_path()));
        assert_eq!(appended.document.full_text(), "startup");
        assert_eq!(appended.plugin_name(), ui::plugin::PLUGIN_EDITOR);
        assert_eq!(appended_title.as_deref(), Some("startup.txt"));
        assert_eq!(appended.display().display_map.line_count(), appended.document.line_count());
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        let forward_effect = app.navigate_editor_forward();
        let _ = app.handle_nav_effect(forward_effect);
        assert_eq!(app.active_tab_id(), forward_target);
    }

    #[test]
    fn batch_close_returns_effect_without_applying_it() {
        let mut app = App::new(None);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);
        app.switch_workspace_for_test(0);
        app.needs_redraw = false;

        let effect = app.execute_batch_close(&[1]);

        assert!(effect.redraw);
        assert!(effect.persist_workspace);
        assert!(!app.needs_redraw);
    }

    #[test]
    fn batch_close_removes_each_matching_runtime_by_tab_id() {
        let mut app = App::new(None);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);
        let first_id = app.editor_tab_id_at(0).expect("first tab ID");
        let second_id = app.editor_tab_id_at(1).expect("second tab ID");
        let third_id = app.editor_tab_id_at(2).expect("third tab ID");
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        app.execute_batch_close(&[0, 2]);

        assert!(app.tab_runtime(first_id).is_none());
        assert!(app.tab_runtime(second_id).is_some());
        assert!(app.tab_runtime(third_id).is_none());
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );
    }

    #[test]
    fn closing_the_last_tab_creates_an_editable_default_document() {
        let mut app = App::new(None);
        app.new_untitled_doc();
        let closed_id = app.active_tab_id().expect("the original tab should have an ID");
        assert!(app.tab_runtime(closed_id).is_some());

        let workspace_effect =
            app.close_editor_tab(closed_id).expect("the only unpinned tab should close");
        let app_effect = app.apply_workspace_effect(workspace_effect);

        assert!(app_effect.redraw);
        assert_eq!(app.editor_tab_count(), 1);
        assert_eq!(app.active_editor_index(), Some(0));
        let replacement_id = app.active_tab_id().expect("the replacement tab should have an ID");
        assert_ne!(replacement_id, closed_id);
        assert!(app.tab_runtime(closed_id).is_none());
        assert!(app.tab_runtime(replacement_id).is_some());
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        let replacement_title =
            app.editor_save_context(replacement_id).map(|context| context.title);
        let default_entry = app.active_tab_session().expect("a default document should remain");
        assert_eq!(replacement_title.as_deref(), Some("untitled"));
        assert_eq!(default_entry.buffer_len(), 0);
        assert!(default_entry.file_path.is_none());
        assert!(!default_entry.dirty);

        app.active_tab_session_mut()
            .expect("the default document should be editable")
            .insert_at_cursor(b"x");
        let edited_document = app.active_tab_session().expect("default document exists");
        assert_eq!(edited_document.buffer_len(), 1);
        assert!(edited_document.dirty);
    }

    #[test]
    fn new_typed_untitled_doc_activates_markdown_with_suggested_title() {
        let mut app = App::new(None);

        let effect = app.new_typed_untitled_doc(ui::sidebar::NewDocumentKind::Markdown);
        let active_tab_id = app.active_tab_id().expect("active tab");
        let active_title = app.editor_save_context(active_tab_id).map(|context| context.title);
        let entry = app.active_tab_session().expect("new document must be active");

        assert!(effect.redraw);
        assert_eq!(active_title.as_deref(), Some("未命名.md"));
        assert!(entry.file_path.is_none());
        assert_eq!(
            app.active_tab_session().expect("active runtime").plugin_name(),
            ui::plugin::PLUGIN_MARKDOWN_EDITOR
        );
    }

    #[test]
    fn generic_app_does_not_create_plaintext_for_encrypted_kind() {
        let mut app = App::new(None);
        let original_tab_id = app.active_tab_id();

        let effect = app.new_typed_untitled_doc(ui::sidebar::NewDocumentKind::EncryptedMarkdown);

        assert_eq!(effect, AppEffect::NONE);
        assert_eq!(app.active_tab_id(), original_tab_id);
    }

    #[test]
    fn batch_close_save_target_classifies_dirty_typed_documents_as_save_as() {
        let cases = [
            (ui::sidebar::NewDocumentKind::Markdown, "未命名.md"),
            (ui::sidebar::NewDocumentKind::Text, "未命名.txt"),
            (ui::sidebar::NewDocumentKind::Mindmap, "未命名.mmap.md"),
        ];

        for (kind, expected_name) in cases {
            let mut app = App::new(None);
            app.new_typed_untitled_doc(kind);
            app.active_tab_session_mut().expect("typed entry exists").document.dirty = true;

            let target = app.editor_tab_id_at(0).and_then(|tab_id| app.editor_save_context(tab_id));
            let target = batch_close_save_target(target);

            assert_eq!(target, Some(BatchCloseSaveTarget::SaveAs(expected_name.to_owned())));
        }
    }

    #[test]
    fn batch_close_save_target_classifies_file_backed_and_clean_entries() {
        assert_eq!(batch_close_save_target(None), None);

        let mut app = App::new(None);
        open_untitled_fixture(&mut app);
        assert_eq!(
            batch_close_save_target(
                app.editor_tab_id_at(0).and_then(|tab_id| app.editor_save_context(tab_id)),
            ),
            Some(BatchCloseSaveTarget::Clean)
        );

        {
            let session = app.active_tab_session_mut().expect("untitled entry exists");
            session.document.file_path = Some(std::path::PathBuf::from("/tmp/existing.txt"));
            session.document.dirty = true;
        }
        assert_eq!(
            batch_close_save_target(
                app.editor_tab_id_at(0).and_then(|tab_id| app.editor_save_context(tab_id)),
            ),
            Some(BatchCloseSaveTarget::ExistingPath(std::path::PathBuf::from("/tmp/existing.txt")))
        );
    }

    #[test]
    fn close_by_id_survives_tab_reordering() {
        let mut app = App::new(None);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);

        // Capture the IDs of the tabs originally at index 1 and 2.
        let closed_id = app.editor_tab_id_at(0).expect("tab 0 must exist");
        let target_id = app.editor_tab_id_at(1).expect("tab 1 must exist");
        let other_id = app.editor_tab_id_at(2).expect("tab 2 must exist");

        // Reorder the workspace by closing tab 0: the targeted tab shifts to index 0.
        let effect = app.close_editor_tab(closed_id).expect("close clean tab 0");
        assert_eq!(
            effect,
            crate::workspace::WorkspaceEffect::Closed { closed: closed_id, activated: None }
        );
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        // A stale index-based close of "index 1" would now remove the wrong tab.
        // Closing by ID must still remove the originally targeted tab.
        app.try_close_entry_with_prompt(target_id);

        assert!(
            app.editor_tab_index(target_id).is_none(),
            "the originally targeted tab must be closed"
        );
        assert_eq!(app.editor_tab_count(), 1);
        assert_eq!(app.editor_tab_index(other_id), Some(0));
    }

    #[test]
    fn stale_batch_close_id_does_not_fall_back_to_first_tab() {
        let mut app = App::new(None);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);

        let stale_id = app.editor_tab_id_at(1).expect("tab 1 must exist");
        let closed_id = stale_id;
        let expected_remaining = vec![
            app.editor_tab_id_at(0).expect("tab 0 must exist"),
            app.editor_tab_id_at(2).expect("tab 2 must exist"),
        ];

        let effect = app.close_editor_tab(stale_id).expect("closing the target tab should succeed");
        assert_eq!(
            effect,
            crate::workspace::WorkspaceEffect::Closed { closed: closed_id, activated: None }
        );
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        let _ = app.try_close_multiple_with_prompt(
            ui::popup_menu::ContextMenuAction::CloseOthers,
            stale_id,
        );

        let actual_remaining = app.editor_tab_ids_in_order();
        assert_eq!(actual_remaining, expected_remaining);
    }
}
