use crate::app::App;
use crate::app_effect::AppEffect;
use crate::workspace_tab_factory::{self, ProductPreparedTab};
use appkit_core::document::DocumentModel;
use appkit_core::workspace::types::TabId;

#[derive(Debug, PartialEq, Eq)]
enum BatchCloseSaveTarget {
    Clean,
    ExistingPath(std::path::PathBuf),
    SaveAs(String),
}

fn batch_close_save_target(
    entry: Option<(&DocumentModel, String)>,
) -> Option<BatchCloseSaveTarget> {
    let (document, title) = entry?;
    if !document.dirty {
        return Some(BatchCloseSaveTarget::Clean);
    }

    match document.file_path.clone() {
        Some(path) => Some(BatchCloseSaveTarget::ExistingPath(path)),
        None => Some(BatchCloseSaveTarget::SaveAs(title)),
    }
}

impl App {
    pub(crate) fn apply_workspace_effect(
        &mut self,
        effect: crate::workspace::WorkspaceEffect,
    ) -> AppEffect {
        effect.reconcile_runtime_store(&mut self.tab_runtime_store);

        let effect = match effect {
            crate::workspace::WorkspaceEffect::Closed { activated: None, .. }
                if self.workspace.is_empty() =>
            {
                let viewport = self.viewport_dimensions(self.screen_height());
                let ProductPreparedTab { prepared, suggested_file_name } =
                    workspace_tab_factory::prepare_untitled(&self.workspace, viewport);
                self.workspace.open_prepared_tab(
                    &mut self.tab_runtime_store,
                    prepared,
                    suggested_file_name,
                )
            }
            other => other,
        };

        self.handle_nav_effect(effect.nav_effect())
    }

    fn finish_active_workspace_change(&mut self) -> AppEffect {
        crate::workspace_product::hydrate_active_stub(&mut self.workspace);
        if !self.workspace.is_empty() {
            let screen_height = self.screen_height();
            let visible_rows = self.visible_rows(screen_height);
            let viewport_height = self.visible_height_lines(screen_height);
            if let Some(mut tab) = self.active_tab_session_mut() {
                tab.resize_presentation(visible_rows, viewport_height);
                tab.clear_advance_cache();
            }
            self.init_display_map(self.workspace.active_index());
        }
        self.frame_cache.cluster_pool.clear();
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
        let effect = if let Some(index) = self.workspace.find_by_path(path) {
            self.workspace.switch_to(index)
        } else {
            let ProductPreparedTab { prepared, suggested_file_name } =
                workspace_tab_factory::prepare_file(&self.workspace, path, viewport)?;
            self.workspace.open_prepared_tab(
                &mut self.tab_runtime_store,
                prepared,
                suggested_file_name,
            )
        };
        let app_effect = self.apply_workspace_effect(effect);
        self.record_entry_to_history(self.workspace.active_index());
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
            workspace_tab_factory::prepare_untitled(&self.workspace, viewport);
        let effect = self.workspace.open_prepared_tab(
            &mut self.tab_runtime_store,
            prepared,
            suggested_file_name,
        );
        self.apply_workspace_effect(effect)
    }

    pub(crate) fn new_typed_untitled_doc(
        &mut self,
        kind: ui::sidebar::NewDocumentKind,
    ) -> AppEffect {
        let viewport = self.viewport_dimensions(self.screen_height());
        let ProductPreparedTab { prepared, suggested_file_name } =
            workspace_tab_factory::prepare_typed_untitled(&self.workspace, kind, viewport);
        let effect = self.workspace.open_prepared_tab(
            &mut self.tab_runtime_store,
            prepared,
            suggested_file_name,
        );
        self.apply_workspace_effect(effect)
    }

    pub(crate) fn try_close_entry_with_prompt(&mut self, id: TabId) -> AppEffect {
        let Some(idx) = self.workspace.index_of(id) else {
            return AppEffect::NONE;
        };
        use crate::workspace::CloseTabDecision;
        let mut app_effect = AppEffect::NONE;
        let decision = self.workspace.try_close_entry(idx);
        match decision {
            CloseTabDecision::CanClose => {
                self.record_entry_to_history(idx);
                if let Ok(effect) = self.workspace.close_entry(idx) {
                    app_effect = app_effect.merge(self.apply_workspace_effect(effect));
                }
                self.save_history();
                self.rebuild_native_menu();
                app_effect = app_effect.merge(AppEffect::REDRAW);
            }
            CloseTabDecision::Pinned => {}
            CloseTabDecision::NeedsSavePrompt => {
                let file_name =
                    self.workspace.entry_title(idx).unwrap_or_else(|| "未命名".to_owned());
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
                if let Some(ref w) = self.window {
                    dialog = dialog.set_parent(w);
                }
                let result = dialog.show();
                match result {
                    rfd::MessageDialogResult::Custom(ref label) if label == "保存" => {
                        let need_save_as = self
                            .workspace
                            .entry(idx)
                            .map(|v| v.file_path.is_none())
                            .unwrap_or(false);
                        if need_save_as {
                            let default_name = self
                                .workspace
                                .entry_title(idx)
                                .unwrap_or_else(|| "未命名".to_owned());
                            if let Some(path) =
                                rfd::FileDialog::new().set_file_name(&default_name).save_file()
                            {
                                let Some(document) = self.workspace.entry_doc_mut(idx) else {
                                    return app_effect;
                                };
                                if let Err(e) = document.save_as(&path) {
                                    eprintln!("保存失败: {e}");
                                    return app_effect;
                                }
                                self.workspace.clear_suggested_file_name(idx);
                            } else {
                                return app_effect;
                            }
                        } else {
                            if let Some(dv) = self.workspace.entry_doc_mut(idx)
                                && let Err(e) = dv.save()
                            {
                                eprintln!("保存失败: {e}");
                                return app_effect;
                            }
                        }
                        self.record_entry_to_history(idx);
                        if let Ok(effect) = self.workspace.close_entry(idx) {
                            app_effect = app_effect.merge(self.apply_workspace_effect(effect));
                        }
                        self.save_history();
                        self.rebuild_native_menu();
                        app_effect = app_effect.merge(AppEffect::REDRAW);
                    }
                    rfd::MessageDialogResult::Custom(ref label) if label == "放弃" => {
                        self.record_entry_to_history(idx);
                        if let Ok(effect) = self.workspace.close_entry(idx) {
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
        let Some(tab_index) = self.workspace.index_of(id) else {
            return app_effect;
        };
        let indices: Vec<usize> = match action {
            ui::popup_menu::ContextMenuAction::CloseOthers => (0..self.workspace.len())
                .filter(|&i| i != tab_index && !self.workspace.is_pinned(i))
                .collect(),
            ui::popup_menu::ContextMenuAction::CloseRight => ((tab_index + 1)
                ..self.workspace.len())
                .filter(|i| !self.workspace.is_pinned(*i))
                .collect(),
            ui::popup_menu::ContextMenuAction::CloseAll => {
                (0..self.workspace.len()).filter(|i| !self.workspace.is_pinned(*i)).collect()
            }
            _ => return app_effect,
        };

        let dirty_count = indices
            .iter()
            .filter(|&&i| self.workspace.entry_doc(i).is_some_and(|document| document.dirty))
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
            if let Some(ref w) = self.window {
                dialog = dialog.set_parent(w);
            }
            let result = dialog.show();
            match result {
                rfd::MessageDialogResult::Custom(ref label) if label == "全部保存" => {
                    for &i in &indices {
                        let close_context =
                            self.workspace.entry(i).zip(self.workspace.entry_title(i));
                        let Some(target) = batch_close_save_target(close_context) else {
                            return app_effect;
                        };
                        match target {
                            BatchCloseSaveTarget::Clean => {}
                            BatchCloseSaveTarget::ExistingPath(path) => {
                                let Some(document) = self.workspace.entry_doc_mut(i) else {
                                    return app_effect;
                                };
                                if let Err(error) = document.save_as(&path) {
                                    eprintln!("保存失败: {error}");
                                    return app_effect;
                                }
                                self.workspace.clear_suggested_file_name(i);
                            }
                            BatchCloseSaveTarget::SaveAs(default_name) => {
                                let Some(path) =
                                    rfd::FileDialog::new().set_file_name(&default_name).save_file()
                                else {
                                    return app_effect;
                                };
                                let Some(document) = self.workspace.entry_doc_mut(i) else {
                                    return app_effect;
                                };
                                if let Err(error) = document.save_as(&path) {
                                    eprintln!("保存失败: {error}");
                                    return app_effect;
                                }
                                self.workspace.clear_suggested_file_name(i);
                            }
                        }
                    }
                }
                rfd::MessageDialogResult::Custom(ref label) if label == "全部放弃" => {}
                _ => return app_effect,
            }
        }

        for &i in indices.iter().rev() {
            self.record_entry_to_history(i);
            if let Ok(effect) = self.workspace.close_entry(i) {
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
            if let Ok(next) = self.workspace.close_entry(index) {
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
                let Some(tab_index) = self.workspace.index_of(id) else {
                    return AppEffect::NONE;
                };
                crate::workspace_product::copy_tab_path(&self.workspace, tab_index);
                AppEffect::NONE
            }
            ContextMenuAction::TogglePin => {
                let Some(tab_index) = self.workspace.index_of(id) else {
                    return AppEffect::NONE;
                };
                let workspace_effect = self.workspace.toggle_pin_at(tab_index);
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
            workspace_tab_factory::prepare_file(&self.workspace, path, dimensions)?;
        let title = format!("{} — edit+", path.display());
        if let Some(window) = &self.window {
            window.set_title(&title);
        }
        let id = self.workspace.append_prepared_tab(
            &mut self.tab_runtime_store,
            prepared,
            suggested_file_name,
        );
        let appended_index = self
            .workspace
            .index_of(id)
            .expect("a startup tab must remain installed until display-map initialization");
        self.init_display_map(appended_index);
        Ok(id)
    }

    pub(crate) fn load_file(&mut self) {
        let Some(path) = self.file_path.clone() else {
            return;
        };
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let visible_rows = self.visible_rows(gpu.ctx.config.height as f32);
        let viewport_height = self.visible_height_lines(gpu.ctx.config.height as f32);
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
            workspace_tab_factory::prepare_untitled(&app.workspace, test_viewport());
        let effect = app.workspace.open_prepared_tab(
            &mut app.tab_runtime_store,
            prepared,
            suggested_file_name,
        );
        effect.reconcile_runtime_store(&mut app.tab_runtime_store);
    }

    fn app_with_file_stub_and_active_document(file_path: &std::path::Path) -> App {
        let mut app = App::new(None);
        let mut stub = DocumentView::new(vec![String::new()], 10, 160.0);
        stub.file_path = Some(file_path.to_owned());
        app.push_entry_for_test(stub, Box::new(EditorPlugin::new()));
        let active = DocumentView::new(vec!["active".to_owned()], 10, 160.0);
        app.push_entry_for_test(active, Box::new(EditorPlugin::new()));
        let switch_effect = app.workspace.switch_to(1);
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

        let nav_effect = app.workspace.go_back();

        assert_eq!(nav_effect, crate::navigator::NavEffect::ActiveChanged);
        assert_eq!(app.workspace.active_doc().expect("stub should become active").buffer_len(), 0);

        let app_effect = app.handle_nav_effect(nav_effect);

        assert_eq!(
            app.workspace.active_doc().expect("active stub should hydrate").full_text(),
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

        let workspace_effect =
            app.workspace.close_entry(1).expect("active document should close cleanly");

        assert_eq!(app.workspace.active_doc().expect("stub should become active").buffer_len(), 0);

        let app_effect = app.apply_workspace_effect(workspace_effect);

        assert_eq!(
            app.workspace.active_doc().expect("active stub should hydrate").full_text(),
            "loaded after close"
        );
        assert!(app_effect.reshape);
        assert!(app_effect.redraw);
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());
    }

    #[test]
    fn new_untitled_promotes_runtime_into_store() {
        let mut app = App::new(None);

        let effect = app.new_untitled_doc();

        let id = app.active_tab_id().expect("new tab id");
        assert!(app.tab_runtime_store.contains(id));
        assert!(effect.reshape);
        assert!(effect.redraw);
        assert!(effect.update_title);
        assert!(effect.persist_workspace);
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());
    }

    #[test]
    fn copy_path_for_untitled_tab_is_a_noop() {
        let mut app = App::new(None);
        app.new_untitled_doc();
        let tab_id = app.active_tab_id().expect("untitled tab should have an ID");

        let effect =
            app.dispatch_context_menu_action(ui::popup_menu::ContextMenuAction::CopyPath, tab_id);

        assert_eq!(effect, AppEffect::NONE);
        assert_eq!(app.workspace.len(), 1);
        assert_eq!(app.active_tab_id(), Some(tab_id));
    }

    #[test]
    fn toggle_pin_context_action_applies_workspace_navigation_effect() {
        let mut app = App::new(None);
        app.new_untitled_doc();
        let tab_id = app.active_tab_id().expect("untitled tab should have an ID");

        let effect =
            app.dispatch_context_menu_action(ui::popup_menu::ContextMenuAction::TogglePin, tab_id);

        assert!(app.workspace.is_pinned(0));
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
        assert_eq!(app.workspace.len(), 1);
        assert_eq!(
            app.active_tab_session().expect("opened file runtime should exist").plugin_name(),
            ui::plugin::PLUGIN_MARKDOWN_EDITOR
        );
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());

        app.new_untitled_doc();
        let len_before_reopen = app.workspace.len();
        std::fs::remove_file(&path).expect("existing-tab reopen must not need the source file");
        let reopen_effect = app.open_file(&path).expect("existing file should reactivate");

        assert!(reopen_effect.reshape);
        assert!(reopen_effect.redraw);
        assert!(reopen_effect.update_title);
        assert!(reopen_effect.persist_workspace);
        assert_eq!(app.workspace.len(), len_before_reopen);
        assert_eq!(app.active_tab_id(), Some(opened_id));
        assert_eq!(
            app.active_tab_session()
                .expect("existing file runtime should remain")
                .document
                .full_text(),
            "# Product tab"
        );
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());
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
        let back_effect = app.workspace.go_back();
        let _ = app.handle_nav_effect(back_effect);
        let active_before_append = app.active_tab_id();
        let had_back_history = app.workspace.has_back_history();
        let had_forward_history = app.workspace.has_forward_history();
        app.needs_redraw = false;
        app.skip_reshape_submit = false;

        let appended_id = app
            .load_file_with_dimensions(&path, test_viewport())
            .expect("startup file should append");
        let appended_index =
            app.workspace.index_of(appended_id).expect("startup file should remain appended");
        let appended =
            app.tab_session(appended_id).expect("startup file model and runtime should be paired");

        assert_eq!(app.active_tab_id(), active_before_append);
        assert_eq!(app.workspace.has_back_history(), had_back_history);
        assert_eq!(app.workspace.has_forward_history(), had_forward_history);
        assert!(!app.needs_redraw);
        assert!(app.skip_reshape_submit);
        assert_eq!(appended_index, app.workspace.len() - 1);
        assert_eq!(appended.document.file_path.as_deref(), Some(path.as_path()));
        assert_eq!(appended.document.full_text(), "startup");
        assert_eq!(appended.plugin_name(), ui::plugin::PLUGIN_EDITOR);
        assert_eq!(app.workspace.entry_title(appended_index).as_deref(), Some("startup.txt"));
        assert_eq!(appended.display().display_map.line_count(), appended.document.line_count());
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());

        let forward_effect = app.workspace.go_forward();
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
        let first_id = app.workspace.tab_id_at(0).expect("first tab ID");
        let second_id = app.workspace.tab_id_at(1).expect("second tab ID");
        let third_id = app.workspace.tab_id_at(2).expect("third tab ID");
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());

        app.execute_batch_close(&[0, 2]);

        assert!(app.tab_runtime_store.get(first_id).is_none());
        assert!(app.tab_runtime_store.get(second_id).is_some());
        assert!(app.tab_runtime_store.get(third_id).is_none());
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());
    }

    #[test]
    fn closing_the_last_tab_creates_an_editable_default_document() {
        let mut app = App::new(None);
        app.new_untitled_doc();
        let closed_id = app.active_tab_id().expect("the original tab should have an ID");
        assert!(app.tab_runtime_store.contains(closed_id));

        let workspace_effect =
            app.workspace.close_entry(0).expect("the only unpinned tab should close");
        let app_effect = app.apply_workspace_effect(workspace_effect);

        assert!(app_effect.redraw);
        assert_eq!(app.workspace.len(), 1);
        assert_eq!(app.workspace.active_index(), 0);
        let replacement_id = app.active_tab_id().expect("the replacement tab should have an ID");
        assert_ne!(replacement_id, closed_id);
        assert!(!app.tab_runtime_store.contains(closed_id));
        assert!(app.tab_runtime_store.contains(replacement_id));
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());

        let default_entry = app.active_tab_session().expect("a default document should remain");
        assert_eq!(app.workspace.entry_title(0).as_deref(), Some("untitled"));
        assert_eq!(default_entry.buffer_len(), 0);
        assert!(default_entry.file_path.is_none());
        assert!(!default_entry.dirty);

        app.workspace
            .active_doc_mut()
            .expect("the default document should be editable")
            .insert_at_cursor(b"x");
        let edited_document = app.workspace.active_doc().expect("default document exists");
        assert_eq!(edited_document.buffer_len(), 1);
        assert!(edited_document.dirty);
    }

    #[test]
    fn new_typed_untitled_doc_activates_markdown_with_suggested_title() {
        let mut app = App::new(None);

        let effect = app.new_typed_untitled_doc(ui::sidebar::NewDocumentKind::Markdown);
        let entry = app.active_tab_session().expect("new document must be active");

        assert!(effect.redraw);
        assert_eq!(app.workspace.entry_title(0).as_deref(), Some("未命名.md"));
        assert!(entry.file_path.is_none());
        assert_eq!(
            app.active_tab_session().expect("active runtime").plugin_name(),
            ui::plugin::PLUGIN_MARKDOWN_EDITOR
        );
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
            app.workspace.entry_mut(0).expect("typed entry exists").dirty = true;

            let target =
                batch_close_save_target(app.workspace.entry(0).zip(app.workspace.entry_title(0)));

            assert_eq!(target, Some(BatchCloseSaveTarget::SaveAs(expected_name.to_owned())));
        }
    }

    #[test]
    fn batch_close_save_target_classifies_file_backed_and_clean_entries() {
        assert_eq!(batch_close_save_target(None), None);

        let mut app = App::new(None);
        open_untitled_fixture(&mut app);
        assert_eq!(
            batch_close_save_target(app.workspace.entry(0).zip(app.workspace.entry_title(0)),),
            Some(BatchCloseSaveTarget::Clean)
        );

        app.workspace.entry_mut(0).expect("untitled entry exists").file_path =
            Some(std::path::PathBuf::from("/tmp/existing.txt"));
        app.workspace.entry_mut(0).expect("file-backed entry exists").dirty = true;
        assert_eq!(
            batch_close_save_target(app.workspace.entry(0).zip(app.workspace.entry_title(0)),),
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
        let closed_id = app.workspace.tab_id_at(0).expect("tab 0 must exist");
        let target_id = app.workspace.tab_id_at(1).expect("tab 1 must exist");
        let other_id = app.workspace.tab_id_at(2).expect("tab 2 must exist");

        // Reorder the workspace by closing tab 0: the targeted tab shifts to index 0.
        let effect = app.workspace.close_entry(0).expect("close clean tab 0");
        assert_eq!(
            effect,
            crate::workspace::WorkspaceEffect::Closed { closed: closed_id, activated: None }
        );
        effect.reconcile_runtime_store(&mut app.tab_runtime_store);
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());

        // A stale index-based close of "index 1" would now remove the wrong tab.
        // Closing by ID must still remove the originally targeted tab.
        app.try_close_entry_with_prompt(target_id);

        assert!(
            app.workspace.index_of(target_id).is_none(),
            "the originally targeted tab must be closed"
        );
        assert_eq!(app.workspace.len(), 1);
        assert_eq!(app.workspace.index_of(other_id), Some(0));
    }

    #[test]
    fn stale_batch_close_id_does_not_fall_back_to_first_tab() {
        let mut app = App::new(None);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);

        let stale_id = app.workspace.tab_id_at(1).expect("tab 1 must exist");
        let closed_id = stale_id;
        let expected_remaining = vec![
            app.workspace.tab_id_at(0).expect("tab 0 must exist"),
            app.workspace.tab_id_at(2).expect("tab 2 must exist"),
        ];

        let effect = app.workspace.close_entry(1).expect("closing the target tab should succeed");
        assert_eq!(
            effect,
            crate::workspace::WorkspaceEffect::Closed { closed: closed_id, activated: None }
        );
        effect.reconcile_runtime_store(&mut app.tab_runtime_store);
        assert_eq!(app.workspace.tab_ids(), app.tab_runtime_store.ids());

        let _ = app.try_close_multiple_with_prompt(
            ui::popup_menu::ContextMenuAction::CloseOthers,
            stale_id,
        );

        let actual_remaining: Vec<_> =
            (0..app.workspace.len()).map(|index| app.workspace.tab_id_at(index).unwrap()).collect();
        assert_eq!(actual_remaining, expected_remaining);
    }
}
