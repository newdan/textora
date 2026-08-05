//! 可嵌入产品的最小编辑器 runtime facade。

mod contract;
mod document_save;
mod editor_frame;
mod editor_painter;
mod editor_pointer;
#[allow(dead_code, reason = "ER4-4 wires file safety results into the App lifecycle")]
mod file_safety_session;
#[allow(dead_code, reason = "ER2-2 wires the product-neutral input session into event routing")]
mod input_session;
mod model_session;
#[allow(dead_code, reason = "ER3-2 wires window resources into the App lifecycle")]
mod render_session;
#[allow(dead_code, reason = "ER3-1 wires reshape results into the App lifecycle")]
mod reshape_session;

pub use crate::mouse_state::MouseCapture;
pub use contract::{
    CloseConfirmation, DocumentTextEditError, DocumentTextReplacement, DocumentTextSnapshot,
    EditorDocumentSummary, EditorFocus, EditorInputContext, EditorNotification, EditorOutcome,
    EditorRuntimeConfig, EditorRuntimeError, EditorTabSnapshot, EditorWorkspaceSnapshot,
    OpenDisposition,
};
pub use document_save::{
    PreparedDocumentSave, SaveCompletion, SavePrepareError, execute_prepared_save,
};
pub use editor_frame::{EditorFrame, RenderError, RenderResources};
pub use editor_painter::EditorSurfacePaint;
pub use file_safety_session::{FileSafetyCandidate, FileSafetyObservation};
pub const RESHAPE_AHEAD_LINES: usize = reshape_session::RESHAPE_AHEAD_LINES;

/// Product semantic editing result; unsupported commands are explicit and never
/// silently fall back to a source mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticEditResult {
    Applied,
    NoChange,
    Unsupported,
}

use crate::tab_runtime::TabRuntime;
use crate::tab_session::{TabSession, TabSessionMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::prepared_tab::PreparedTab;
use crate::reshape_worker::{ReshapeRequest, ReshapeResult, ReshapeWorker};
use crate::tab_runtime::TabRuntimeStore;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeOutcome {
    NotReady,
    Deferred,
    Applied { width_changed: bool, width: u32, height: u32 },
}

/// 编辑器 runtime 的最小构造壳。
///
/// ER0 只冻结公共边界；具体模型、输入、渲染和文件安全 session 在后续阶段接管。
pub struct EditorRuntime {
    model_session: model_session::ModelSession,
    input_session: input_session::EditorInputSession,
    render_session: render_session::RenderSession,
    reshape_session: reshape_session::ReshapeSession,
    file_safety_session: file_safety_session::FileSafetySession,
    save_session: document_save::SaveSession,
    settings: ui::settings::Settings,
    theme: ui::Theme,
    ui_shaper: Option<Arc<Mutex<shaping::Shaper>>>,
    _snapshots_directory: PathBuf,
}

impl EditorRuntime {
    pub fn new(config: EditorRuntimeConfig) -> Result<Self, EditorRuntimeError> {
        let EditorRuntimeConfig {
            plugin_registry,
            view_routes,
            initial_settings,
            initial_theme,
            snapshots_directory,
        } = config;
        Ok(Self {
            model_session: model_session::ModelSession::new(plugin_registry, view_routes),
            input_session: input_session::EditorInputSession::new(),
            render_session: render_session::RenderSession::new(),
            reshape_session: reshape_session::ReshapeSession::new(),
            file_safety_session: file_safety_session::FileSafetySession::new(),
            save_session: document_save::SaveSession::new(),
            settings: initial_settings,
            theme: initial_theme,
            ui_shaper: None,
            _snapshots_directory: snapshots_directory,
        })
    }

    /// Start surface-independent GPU initialization while the product prepares
    /// its event loop and persisted state.
    pub fn start_gpu_preparation(&mut self) -> Result<(), EditorRuntimeError> {
        self.render_session.start_gpu_preparation().map_err(|error| {
            EditorRuntimeError::GpuInitialization {
                message: format!("could not start GPU preparation worker: {error}"),
            }
        })
    }

    #[doc(hidden)]
    pub fn new_with_model(
        config: EditorRuntimeConfig,
        workspace: Workspace,
        runtimes: TabRuntimeStore,
    ) -> Result<Self, EditorRuntimeError> {
        let mut runtime = Self::new(config)?;
        runtime.model_session = model_session::ModelSession::from_parts(workspace, runtimes);
        Ok(runtime)
    }

    pub fn install_prepared_tab(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
        disposition: OpenDisposition,
    ) -> EditorOutcome {
        let effect =
            self.model_session.install_prepared_tab(prepared, suggested_file_name, disposition);
        self.outcome_for_workspace_effect(effect)
    }

    pub fn install_prepared_tab_for_product(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
        disposition: OpenDisposition,
    ) -> crate::workspace::WorkspaceEffect {
        let effect =
            self.model_session.install_prepared_tab(prepared, suggested_file_name, disposition);
        let _ = self.outcome_for_workspace_effect(effect);
        effect
    }

    pub fn append_prepared_tab(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
    ) -> appkit_core::workspace::types::TabId {
        self.model_session.append_prepared_tab(prepared, suggested_file_name)
    }

    pub fn replace_model_state(&mut self, workspace: Workspace, runtimes: TabRuntimeStore) {
        self.model_session.replace_parts(workspace, runtimes);
    }

    pub fn activate(&mut self, tab_id: appkit_core::workspace::types::TabId) -> EditorOutcome {
        self.model_session
            .activate(tab_id)
            .map_or_else(EditorOutcome::default, |effect| self.outcome_for_workspace_effect(effect))
    }

    pub fn activate_for_product(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<crate::workspace::WorkspaceEffect> {
        let effect = self.model_session.activate(tab_id)?;
        let _ = self.outcome_for_workspace_effect(effect);
        Some(effect)
    }

    pub fn request_close(&mut self, tab_id: appkit_core::workspace::types::TabId) -> EditorOutcome {
        let Some(decision) = self.model_session.close_decision(tab_id) else {
            return EditorOutcome::default();
        };
        if decision == crate::workspace::CloseTabDecision::CanClose {
            return self
                .model_session
                .close(tab_id)
                .map_or_else(EditorOutcome::default, |effect| {
                    self.outcome_for_workspace_effect(effect)
                });
        }
        EditorOutcome {
            shell_effect: crate::event::ShellEffect::REDRAW,
            notifications: smallvec::smallvec![EditorNotification::CloseRequested {
                tab_id,
                decision,
            }],
        }
    }

    pub fn confirm_close(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        confirmation: CloseConfirmation,
    ) -> EditorOutcome {
        self.model_session
            .confirm_close(tab_id, confirmation)
            .map_or_else(EditorOutcome::default, |effect| self.outcome_for_workspace_effect(effect))
    }

    pub fn close_for_product(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<crate::workspace::WorkspaceEffect> {
        let effect = self.model_session.close(tab_id)?;
        let _ = self.outcome_for_workspace_effect(effect);
        Some(effect)
    }

    pub fn active_tab_id(&self) -> Option<appkit_core::workspace::types::TabId> {
        self.model_session.active_tab_id()
    }

    pub fn tab_index(&self, tab_id: appkit_core::workspace::types::TabId) -> Option<usize> {
        self.model_session.tab_index(tab_id)
    }

    pub fn tab_id_at(&self, index: usize) -> Option<appkit_core::workspace::types::TabId> {
        self.model_session.tab_id_at(index)
    }

    pub fn tab_count(&self) -> usize {
        self.model_session.tab_count()
    }

    pub fn tab_ids_in_order(&self) -> Vec<appkit_core::workspace::types::TabId> {
        self.model_session.tab_ids_in_order()
    }

    pub fn runtime_tab_ids(
        &self,
    ) -> std::collections::HashSet<appkit_core::workspace::types::TabId> {
        self.model_session.runtime_tab_ids()
    }

    pub fn is_empty(&self) -> bool {
        self.model_session.is_empty()
    }

    pub fn is_pinned(&self, tab_id: appkit_core::workspace::types::TabId) -> bool {
        self.model_session.is_pinned(tab_id)
    }

    pub fn close_decision(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<crate::workspace::CloseTabDecision> {
        self.model_session.close_decision(tab_id)
    }

    pub fn tab_title(&self, tab_id: appkit_core::workspace::types::TabId) -> Option<String> {
        self.model_session.tab_title(tab_id)
    }

    pub fn clear_suggested_file_name(&mut self, tab_id: appkit_core::workspace::types::TabId) {
        self.model_session.clear_suggested_file_name(tab_id);
    }

    pub fn has_back_history(&self) -> bool {
        self.model_session.has_back_history()
    }

    pub fn has_forward_history(&self) -> bool {
        self.model_session.has_forward_history()
    }

    pub fn toggle_target(&self) -> Option<&'static str> {
        self.model_session.toggle_target()
    }

    pub fn toggle_pin(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<appkit_core::navigator::NavEffect> {
        self.model_session.toggle_pin(tab_id)
    }

    pub fn toggle_active_pin(&mut self) -> appkit_core::navigator::NavEffect {
        self.model_session.toggle_active_pin()
    }

    pub fn navigate_back(&mut self) -> appkit_core::navigator::NavEffect {
        self.model_session.navigate_back()
    }

    pub fn navigate_forward(&mut self) -> appkit_core::navigator::NavEffect {
        self.model_session.navigate_forward()
    }

    pub fn upgrade_active_preview(&mut self) -> appkit_core::navigator::NavEffect {
        self.model_session.upgrade_active_preview()
    }

    pub fn switch_active_plugin(&mut self) {
        self.model_session.switch_active_plugin();
    }

    pub fn active_is_toggled(&self, plugin_name: &str) -> bool {
        self.model_session.active_is_toggled(plugin_name)
    }

    pub fn pinned_paths(&self) -> Vec<PathBuf> {
        self.model_session.pinned_paths()
    }

    pub fn restore_pinned(&mut self, paths: &[PathBuf]) {
        self.model_session.restore_pinned(paths);
    }

    pub fn create_plugin_for_path(&self, path: &Path) -> Box<dyn ui::plugin::ViewPlugin> {
        self.model_session.create_plugin_for_path(path)
    }

    pub fn create_plugin_by_name(&self, plugin_name: &str) -> Box<dyn ui::plugin::ViewPlugin> {
        self.model_session.create_plugin_by_name(plugin_name)
    }

    pub fn tab_session(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<TabSession<'_>> {
        self.model_session.tab_session(tab_id)
    }

    pub fn tab_session_mut(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<TabSessionMut<'_>> {
        self.model_session.tab_session_mut(tab_id)
    }

    pub fn tab_runtime(&self, tab_id: appkit_core::workspace::types::TabId) -> Option<&TabRuntime> {
        self.model_session.tab_runtime(tab_id)
    }

    pub fn tab_runtime_mut(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<&mut TabRuntime> {
        self.model_session.tab_runtime_mut(tab_id)
    }

    pub fn tab_id_for_path(&self, path: &Path) -> Option<appkit_core::workspace::types::TabId> {
        self.model_session.tab_id_for_path(path)
    }

    pub fn document_summary(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<EditorDocumentSummary> {
        self.model_session.document_summary(tab_id)
    }

    pub fn document_text_snapshot(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<DocumentTextSnapshot> {
        self.model_session.document_text_snapshot(tab_id)
    }

    pub fn replace_document_text(
        &mut self,
        request: DocumentTextReplacement,
    ) -> Result<EditorOutcome, DocumentTextEditError> {
        self.model_session.replace_document_text(request, self.editor_line_height())
    }

    pub fn document_summaries(&self) -> Vec<EditorDocumentSummary> {
        self.model_session.document_summaries()
    }

    pub fn workspace_snapshot(&self) -> EditorWorkspaceSnapshot {
        self.model_session.workspace_snapshot()
    }

    pub fn replace_document(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        document: appkit_core::document::DocumentModel,
    ) -> bool {
        self.model_session.replace_document(tab_id, document)
    }

    pub fn update_document_path(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        path: PathBuf,
        disk_revision: Option<appkit_core::file_safety::DiskRevision>,
    ) -> bool {
        self.model_session.update_document_path(tab_id, path, disk_revision)
    }

    pub fn detach_document(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        suggested_file_name: Option<String>,
        dirty_snapshot_id: Option<String>,
    ) -> bool {
        self.model_session.detach_document(tab_id, suggested_file_name, dirty_snapshot_id)
    }

    pub fn prepare_save(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Result<PreparedDocumentSave, SavePrepareError> {
        if self.model_session.document_summary(tab_id).is_none() {
            return Err(SavePrepareError::UnknownTab { tab_id });
        }
        let Some((path, serialized_contents, expected_disk_revision, content_revision)) =
            self.model_session.document_save_snapshot(tab_id)
        else {
            return Err(SavePrepareError::Untitled { tab_id });
        };
        Ok(PreparedDocumentSave {
            tab_id,
            path,
            serialized_contents,
            expected_disk_revision,
            content_revision,
        })
    }

    pub fn prepare_save_as(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
        path: &Path,
    ) -> Result<PreparedDocumentSave, SavePrepareError> {
        if self.model_session.document_summary(tab_id).is_none() {
            return Err(SavePrepareError::UnknownTab { tab_id });
        }
        let Some((serialized_contents, expected_disk_revision, content_revision)) =
            self.model_session.document_save_snapshot_as(tab_id, path)
        else {
            return Err(SavePrepareError::UnknownTab { tab_id });
        };
        Ok(PreparedDocumentSave {
            tab_id,
            path: path.to_owned(),
            serialized_contents,
            expected_disk_revision,
            content_revision,
        })
    }

    pub fn apply_save_completion(&mut self, completion: SaveCompletion) -> EditorOutcome {
        let tab_id = completion.tab_id;
        match completion.result {
            Ok(disk_revision) => {
                let saved_path = disk_revision.path.clone();
                let Some((clean, path_changed)) = self.model_session.apply_save_completion(
                    tab_id,
                    saved_path.clone(),
                    completion.content_revision,
                    disk_revision,
                ) else {
                    return EditorOutcome::default();
                };
                let mut notifications = smallvec::smallvec![EditorNotification::SaveCompleted {
                    tab_id,
                    content_revision: completion.content_revision,
                }];
                if path_changed {
                    notifications
                        .push(EditorNotification::PathChanged { tab_id, path: saved_path });
                }
                if clean {
                    notifications.push(EditorNotification::DirtyChanged { tab_id, dirty: false });
                }
                EditorOutcome {
                    shell_effect: crate::event::ShellEffect::PERSIST_WORKSPACE
                        .merge(crate::event::ShellEffect::UPDATE_TITLE),
                    notifications,
                }
            }
            Err(error) => EditorOutcome {
                shell_effect: crate::event::ShellEffect::REDRAW,
                notifications: smallvec::smallvec![EditorNotification::SaveFailed {
                    tab_id,
                    message: error.to_string(),
                }],
            },
        }
    }

    pub fn submit_save(
        &mut self,
        prepared: PreparedDocumentSave,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), String> {
        self.save_session.submit(prepared, wake)
    }

    pub fn drain_save_completions(&mut self) -> Vec<SaveCompletion> {
        self.save_session.drain()
    }

    pub fn start_file_safety_worker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.file_safety_session.start_worker(wake);
    }

    pub fn file_safety_worker_started(&self) -> bool {
        self.file_safety_session.worker_started()
    }

    pub fn file_safety_next_check(&self) -> std::time::Instant {
        self.file_safety_session.next_check()
    }

    pub fn file_safety_should_check(&self, now: std::time::Instant) -> bool {
        self.file_safety_session.should_check(now)
    }

    pub fn schedule_file_safety_check(&mut self, now: std::time::Instant) {
        self.file_safety_session.schedule_next_check(now);
    }

    pub fn request_file_safety_check_now(&mut self, now: std::time::Instant) {
        self.file_safety_session.request_check_now(now);
    }

    pub fn submit_file_safety_checks(&mut self, local_device_short_id: &str) -> usize {
        let candidates: Vec<FileSafetyCandidate> = self
            .model_session
            .document_summaries()
            .into_iter()
            .filter_map(|summary| {
                let path = summary.path?;
                let current_content = self.model_session.tab_session(summary.tab_id)?.full_text();
                Some(FileSafetyCandidate {
                    tab_id: summary.tab_id,
                    path,
                    dirty: summary.dirty,
                    content_revision: summary.content_revision,
                    current_content,
                    baseline: summary.disk_revision,
                })
            })
            .collect();
        self.file_safety_session.submit_candidates(candidates, local_device_short_id)
    }

    pub fn drain_file_safety_observations(&mut self) -> Vec<FileSafetyObservation> {
        self.file_safety_session.drain_observations()
    }

    pub fn forget_file_safety_tab(&mut self, tab_id: appkit_core::workspace::types::TabId) {
        self.file_safety_session.forget_tab(tab_id);
    }

    pub fn begin_frame(&mut self) -> Result<EditorFrame, RenderError> {
        let theme = self.theme.clone();
        let dpi = self.render_session.scale_factor() as f32;
        Ok(EditorFrame::new_for_backend(theme, dpi, self.ui_shaper.clone()))
    }

    pub fn update_theme(&mut self, theme: ui::Theme) {
        self.theme = theme;
    }

    #[doc(hidden)]
    pub fn take_render_resources(&mut self) -> RenderResources {
        self.render_session.take_render_resources()
    }

    #[doc(hidden)]
    pub fn restore_render_resources(&mut self, resources: RenderResources) {
        self.render_session.restore_render_resources(resources);
    }

    pub fn clear_frame_cluster_pool(&mut self) {
        self.render_session.clear_frame_cluster_pool();
    }

    pub fn frame_cache_snapshot(&self) -> crate::frame_cache::FrameCache {
        self.render_session.frame_cache_snapshot()
    }

    pub fn note_render_started(&mut self, started_at: std::time::Instant) -> u128 {
        self.render_session.note_render_started(started_at)
    }

    pub fn render_frame_count(&self) -> u32 {
        self.render_session.render_frame_count()
    }

    pub fn note_reshape_result_arrived(&mut self, arrived_at: std::time::Instant) -> u128 {
        self.render_session.note_result_arrived(arrived_at)
    }

    pub fn request_resize(&mut self, width: u32, height: u32) -> ResizeOutcome {
        let result =
            self.render_session.request_resize(winit::dpi::PhysicalSize::new(width, height));
        self.resize_outcome(result)
    }

    pub fn resize_now(&mut self, width: u32, height: u32) -> ResizeOutcome {
        let result = self.render_session.resize_now(winit::dpi::PhysicalSize::new(width, height));
        self.resize_outcome(result)
    }

    pub fn flush_resize(&mut self) -> ResizeOutcome {
        let result = self.render_session.flush_pending_resize();
        self.resize_outcome(result)
    }

    pub fn scale_factor(&self) -> f64 {
        self.render_session.scale_factor()
    }

    pub fn resume(
        &mut self,
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
        font_system: Arc<Mutex<shaping::FontSystem>>,
        font_size: f32,
        font_family: &str,
    ) -> Result<(), EditorRuntimeError> {
        let ui_font_system = Arc::clone(&font_system);
        self.render_session
            .resume(event_loop, attributes, font_system, font_size, font_family)
            .map_err(|error| EditorRuntimeError::GpuInitialization {
                message: error.to_string(),
            })?;
        let scaled_font_size = font_size * self.render_session.scale_factor() as f32;
        self.ui_shaper = Some(Arc::new(Mutex::new(shaping::Shaper::from_shared_font_system(
            ui_font_system,
            scaled_font_size,
            font_family,
        ))));
        Ok(())
    }

    pub fn window(&self) -> Option<&Window> {
        self.render_session.window().map(Arc::as_ref)
    }

    pub fn surface_size(&self) -> Option<(u32, u32)> {
        self.render_session.surface_size().map(|size| (size.width, size.height))
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.render_session.set_scale_factor(scale_factor);
    }

    pub fn mark_frame_presented(&mut self) {
        self.render_session.mark_first_frame_presented();
    }

    pub fn first_frame_presented(&self) -> bool {
        self.render_session.first_frame_presented()
    }

    pub fn set_window_focus(&mut self, focused: bool) {
        self.render_session.set_window_focused(focused);
    }

    pub fn window_focused(&self) -> bool {
        self.render_session.window_focused()
    }

    pub fn request_redraw(&mut self) {
        self.render_session.request_redraw();
    }

    pub fn take_redraw_request(&mut self) -> bool {
        self.render_session.take_redraw_request()
    }

    pub fn keyboard_input_allowed(&self, context: EditorInputContext) -> bool {
        self.input_session.keyboard_allowed(context)
    }

    pub fn commit_text(&mut self, context: EditorInputContext, text: String) -> EditorOutcome {
        if text.is_empty() || !self.input_session.keyboard_allowed(context) {
            return EditorOutcome::default();
        }
        let _ = self.input_session.update_preedit(context, String::new(), None);
        self.model_session.edit_active_document(
            ui::plugin::EditIntent::InsertText(text),
            self.editor_line_height(),
        )
    }

    pub fn handle_key_input(
        &mut self,
        context: EditorInputContext,
        key: ui::KeyCode,
        modifiers: ui::core::Modifiers,
    ) -> EditorOutcome {
        if !self.input_session.keyboard_allowed(context) {
            return EditorOutcome::default();
        }

        let command = modifiers.cmd || modifiers.ctrl;
        if command {
            match key {
                ui::KeyCode::Char('a' | 'A') => {
                    return self.model_session.select_all_active_document();
                }
                ui::KeyCode::Char('z' | 'Z') => {
                    return self
                        .model_session
                        .undo_or_redo_active_document(modifiers.shift, self.editor_line_height());
                }
                ui::KeyCode::Char('y' | 'Y') if modifiers.ctrl => {
                    return self
                        .model_session
                        .undo_or_redo_active_document(true, self.editor_line_height());
                }
                _ => {}
            }
        }

        let intent = match key {
            ui::KeyCode::Enter => Some(ui::plugin::EditIntent::InsertParagraphBreak),
            ui::KeyCode::Backspace => Some(ui::plugin::EditIntent::DeleteBackward),
            ui::KeyCode::Delete => Some(ui::plugin::EditIntent::DeleteForward),
            ui::KeyCode::Tab if modifiers.shift => Some(ui::plugin::EditIntent::Outdent),
            ui::KeyCode::Tab => Some(ui::plugin::EditIntent::Indent),
            ui::KeyCode::Char(character) if !command && !modifiers.alt => {
                Some(ui::plugin::EditIntent::InsertText(character.to_string()))
            }
            ui::KeyCode::Left
            | ui::KeyCode::Right
            | ui::KeyCode::Up
            | ui::KeyCode::Down
            | ui::KeyCode::Home
            | ui::KeyCode::End
            | ui::KeyCode::PageUp
            | ui::KeyCode::PageDown => {
                return self.model_session.navigate_active_document(
                    key,
                    modifiers,
                    self.editor_line_height(),
                );
            }
            ui::KeyCode::Escape | ui::KeyCode::Char(_) => None,
        };
        intent.map_or_else(EditorOutcome::default, |intent| {
            self.model_session.edit_active_document(intent, self.editor_line_height())
        })
    }

    pub fn execute_semantic_edit(
        &mut self,
        command: ui::plugin::SemanticEditCommand,
    ) -> (SemanticEditResult, EditorOutcome) {
        self.model_session.execute_semantic_edit(command, self.editor_line_height())
    }

    pub fn scroll_editor(
        &mut self,
        context: EditorInputContext,
        position: (f32, f32),
        pixels: f32,
    ) -> EditorOutcome {
        if !self.input_session.pointer_allowed(context, position) {
            return EditorOutcome::default();
        }
        self.model_session.scroll_active_document(pixels, self.editor_line_height())
    }

    /// 返回活动画布最近一次成功解析的视口快照。
    pub fn active_canvas_viewport_snapshot(&self) -> Option<ui::canvas::CanvasViewportSnapshot> {
        let tab_id = self.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        if !tab.is_canvas() {
            return None;
        }
        tab.runtime.canvas_viewport.snapshot()
    }

    /// 将活动画布的二维滚动范围转换为产品壳可直接渲染的纯 UI 输入。
    pub fn active_canvas_scrollbars_input(
        &self,
    ) -> Option<ui::canvas_scrollbars::CanvasScrollbarsInput> {
        let tab_id = self.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        if !tab.is_canvas() || !tab.has_canvas_viewport_snapshot() {
            return None;
        }
        let input = tab.runtime.canvas_viewport.scrollbars_input();
        Some(ui::canvas_scrollbars::CanvasScrollbarsInput {
            horizontal: input.horizontal,
            vertical: input.vertical,
        })
    }

    /// 把产品输入归约为活动画布视口动作；普通文档保持无副作用。
    pub fn apply_active_canvas_viewport_action(
        &mut self,
        action: crate::canvas_viewport::CanvasViewportAction,
    ) -> EditorOutcome {
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return EditorOutcome::default();
        };
        if !tab.is_canvas() || !tab.has_canvas_viewport_snapshot() {
            return EditorOutcome::default();
        }
        tab.apply_canvas_viewport_action(action);
        EditorOutcome {
            shell_effect: crate::event::ShellEffect::REDRAW,
            ..EditorOutcome::default()
        }
    }

    pub fn pointer_input_allowed(&self, context: EditorInputContext, position: (f32, f32)) -> bool {
        self.input_session.pointer_allowed(context, position)
    }

    fn editor_line_height(&self) -> f32 {
        ui::settings::UiMetrics::from_settings(
            &self.settings,
            self.render_session.scale_factor() as f32,
        )
        .line_height
    }

    pub fn begin_text_selection(&mut self, context: EditorInputContext) -> bool {
        self.input_session.start_text_selection(context)
    }

    pub fn begin_canvas_drag(&mut self, context: EditorInputContext) -> bool {
        self.input_session.start_canvas_drag(context)
    }

    pub fn end_pointer_capture(&mut self) {
        self.input_session.end_pointer_capture();
    }

    pub fn pointer_capture(&self) -> MouseCapture {
        self.input_session.pointer_capture()
    }

    pub fn set_input_modifiers(&mut self, modifiers: winit::keyboard::ModifiersState) {
        self.input_session.set_modifiers(modifiers);
    }

    pub fn input_modifiers(&self) -> winit::keyboard::ModifiersState {
        self.input_session.modifiers()
    }

    pub fn update_preedit(
        &mut self,
        context: EditorInputContext,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        self.input_session.update_preedit(context, text, cursor)
    }

    pub fn preedit(&self) -> (String, Option<(usize, usize)>) {
        let (text, cursor) = self.input_session.preedit();
        (text.to_owned(), cursor)
    }

    pub fn set_preferred_x(&mut self, preferred_x: Option<f32>) {
        self.input_session.set_preferred_x(preferred_x);
    }

    pub fn preferred_x(&self) -> Option<f32> {
        self.input_session.preferred_x()
    }

    pub fn set_wysiwyg_recursing(&mut self, recursing: bool) {
        self.input_session.set_wysiwyg_recursing(recursing);
    }

    pub fn wysiwyg_recursing(&self) -> bool {
        self.input_session.wysiwyg_recursing()
    }

    pub fn focus_lost(&mut self) {
        self.input_session.focus_lost();
        self.render_session.set_window_focused(false);
    }

    pub fn invalidate_reshape(&mut self) -> u64 {
        self.reshape_session.invalidate()
    }

    pub fn reshape_generation(&self) -> u64 {
        self.reshape_session.generation()
    }

    pub fn start_reshape_worker(
        &mut self,
        font_system: Arc<Mutex<shaping::FontSystem>>,
        font_size: f32,
        font_family: String,
    ) {
        self.reshape_session.start_worker(font_system, font_size, font_family);
    }

    pub fn set_shared_font_system(&mut self, font_system: Arc<Mutex<shaping::FontSystem>>) {
        self.reshape_session.set_shared_font_system(font_system);
    }

    pub fn shared_font_system(&self) -> Option<Arc<Mutex<shaping::FontSystem>>> {
        self.reshape_session.shared_font_system()
    }

    pub fn new_shaper(&self, font_size: f32, font_family: &str) -> Option<shaping::Shaper> {
        self.reshape_session.new_shaper(font_size, font_family)
    }

    #[doc(hidden)]
    pub fn adopt_reshape_worker(
        &mut self,
        worker: ReshapeWorker,
        shared_font_system: Option<Arc<Mutex<shaping::FontSystem>>>,
    ) {
        self.reshape_session.attach_worker(worker, shared_font_system);
    }

    pub fn has_reshape_worker(&self) -> bool {
        self.reshape_session.has_worker()
    }

    pub fn submit_reshape(&mut self, request: ReshapeRequest) -> bool {
        self.reshape_session.submit(request)
    }

    pub fn drain_reshape_results(&self, limit: usize) -> Vec<ReshapeResult> {
        self.reshape_session.drain_completed(limit)
    }

    pub fn accepts_reshape_result(
        &self,
        result: &ReshapeResult,
        active_document_index: usize,
    ) -> bool {
        self.reshape_session.accepts(result, active_document_index)
    }

    pub fn mark_reshape_pending(&mut self, line: usize) -> bool {
        self.reshape_session.mark_pending(line)
    }

    pub fn clear_reshape_pending(&mut self, line: usize) {
        self.reshape_session.clear_pending(line);
    }

    pub fn reshape_pending(&self, line: usize) -> bool {
        self.reshape_session.is_pending(line)
    }

    pub fn mark_reshape_anchor_submitted(&mut self, anchor: usize) {
        self.reshape_session.mark_submitted(anchor);
    }

    pub fn should_submit_reshape_anchor(&mut self, anchor: usize, now: std::time::Instant) -> bool {
        self.reshape_session.should_submit_ahead(anchor, now)
    }

    pub fn mark_skip_next_reshape_submit(&mut self) {
        self.reshape_session.skip_next_submit();
    }

    pub fn take_skip_next_reshape_submit(&mut self) -> bool {
        self.reshape_session.take_skip_next_submit()
    }

    pub fn update_settings(&mut self, settings: ui::settings::Settings) {
        self.settings = settings;
    }

    pub fn settings_snapshot(&self) -> ui::settings::Settings {
        self.settings.clone()
    }

    pub fn shutdown(&mut self) {
        self.file_safety_session.shutdown();
        self.reshape_session.shutdown();
        self.render_session.shutdown();
        self.ui_shaper = None;
    }

    fn resize_outcome(&self, result: render_session::ResizeResult) -> ResizeOutcome {
        match result {
            render_session::ResizeResult::NotReady => ResizeOutcome::NotReady,
            render_session::ResizeResult::Deferred => ResizeOutcome::Deferred,
            render_session::ResizeResult::Applied { width_changed } => {
                let (width, height) = self.surface_size().unwrap_or((0, 0));
                ResizeOutcome::Applied { width_changed, width, height }
            }
        }
    }

    fn outcome_for_workspace_effect(
        &mut self,
        effect: crate::workspace::WorkspaceEffect,
    ) -> EditorOutcome {
        if let crate::workspace::WorkspaceEffect::Closed { closed, .. } = effect {
            self.reshape_session.invalidate();
            self.file_safety_session.forget_tab(closed);
        }
        let mut outcome = EditorOutcome {
            shell_effect: crate::event::ShellEffect::PERSIST_WORKSPACE
                .merge(crate::event::ShellEffect::UPDATE_TITLE),
            notifications: smallvec::SmallVec::new(),
        };
        match effect {
            crate::workspace::WorkspaceEffect::None => {}
            crate::workspace::WorkspaceEffect::Activated(tab_id) => {
                outcome
                    .notifications
                    .push(EditorNotification::ActiveDocumentChanged { tab_id: Some(tab_id) });
            }
            crate::workspace::WorkspaceEffect::Closed { activated, .. } => {
                if activated.is_some() || self.active_tab_id().is_none() {
                    outcome
                        .notifications
                        .push(EditorNotification::ActiveDocumentChanged { tab_id: activated });
                }
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_plugin::EditorPluginFactory;
    use crate::prepared_tab::PreparedTab;
    use crate::tab_runtime::TabRuntime;
    use crate::view_route::ViewRouteTable;
    use appkit_core::document::DocumentModel;
    use core::buffer::TextBuffer;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use ui::plugin::PluginFactory;

    struct PointerProbePlugin;

    struct CanvasViewportProbePlugin;

    struct CanvasDragProbePlugin {
        phases: Rc<RefCell<Vec<ui::plugin::CanvasDragPhase>>>,
    }

    impl ui::plugin::ViewPlugin for CanvasDragProbePlugin {
        fn name(&self) -> &str {
            "canvas-drag-probe"
        }

        fn render(
            &mut self,
            _document: &dyn core::document::DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }

        fn query(
            &self,
            query: ui::plugin::PluginQuery,
            _document: &dyn core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            match query {
                ui::plugin::PluginQuery::HitTestEditTarget { .. } => {
                    ui::plugin::PluginResponse::EditHitTarget(Some(
                        ui::plugin::EditHitTarget::SourceObject { source_range: 0..5 },
                    ))
                }
                _ => ui::plugin::PluginResponse::None,
            }
        }

        fn handle_canvas_drag(
            &mut self,
            request: ui::plugin::CanvasDragRequest,
            _document: &dyn core::document::DocView,
        ) -> ui::plugin::CanvasDragResponse {
            self.phases.borrow_mut().push(request.phase);
            if request.phase == ui::plugin::CanvasDragPhase::Drop {
                return ui::plugin::CanvasDragResponse::Apply(
                    ui::plugin::EditTransaction::replace(
                        request.source_generation,
                        request.source_range,
                        "moved".to_owned(),
                        5,
                    ),
                );
            }
            ui::plugin::CanvasDragResponse::Ignore
        }

        fn handles_own_rendering(&self) -> bool {
            true
        }

        fn is_canvas(&self) -> bool {
            true
        }
    }

    impl ui::plugin::ViewPlugin for CanvasViewportProbePlugin {
        fn name(&self) -> &str {
            "canvas-viewport-probe"
        }

        fn render(
            &mut self,
            _document: &dyn core::document::DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }

        fn handles_own_rendering(&self) -> bool {
            true
        }

        fn is_canvas(&self) -> bool {
            true
        }
    }

    impl ui::plugin::ViewPlugin for PointerProbePlugin {
        fn name(&self) -> &str {
            "pointer-probe"
        }

        fn render(
            &mut self,
            _document: &dyn core::document::DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }

        fn query(
            &self,
            query: ui::plugin::PluginQuery,
            _document: &dyn core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            match query {
                ui::plugin::PluginQuery::HitTestEditTarget { .. } => {
                    ui::plugin::PluginResponse::EditHitTarget(Some(
                        ui::plugin::EditHitTarget::TextCaret {
                            byte_offset: 2,
                            selection_scope: None,
                        },
                    ))
                }
                _ => ui::plugin::PluginResponse::None,
            }
        }

        fn handles_own_rendering(&self) -> bool {
            true
        }
    }

    fn runtime_with_clean_tab() -> EditorRuntime {
        let mut registry = ui::plugin::PluginRegistry::new();
        registry.register(Box::new(EditorPluginFactory));
        let view_routes = ViewRouteTable::new(
            Vec::new(),
            &std::collections::HashSet::from([ui::plugin::PLUGIN_EDITOR]),
        )
        .expect("test editor route table should be valid");
        let mut runtime = EditorRuntime::new(EditorRuntimeConfig {
            plugin_registry: registry,
            view_routes,
            initial_settings: ui::Settings::new(),
            initial_theme: ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark()),
            snapshots_directory: PathBuf::from("snapshots"),
        })
        .expect("test editor runtime should construct");
        let mut text_buffer =
            TextBuffer::new(false).expect("test editor buffer should be writable");
        text_buffer.write_raw(b"clean");
        runtime.install_prepared_tab(
            PreparedTab::new(
                DocumentModel::new(text_buffer),
                TabRuntime::new(EditorPluginFactory.create()),
            ),
            None,
            OpenDisposition::Persistent,
        );
        runtime
    }

    fn runtime_with_prepared_canvas() -> EditorRuntime {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test canvas tab should be active");
        {
            let tab = runtime.tab_session_mut(tab_id).expect("test canvas tab should exist");
            tab.runtime.plugin = Box::new(CanvasViewportProbePlugin);
            let snapshot = tab.runtime.canvas_viewport.prepare(
                ui::plugin::CanvasContentMetrics {
                    content_bounds: ui::Rect::new(0.0, 0.0, 5_000.0, 4_000.0),
                    focus_anchor: None,
                },
                ui::Rect::new(200.0, 80.0, 1_000.0, 800.0),
                ui::canvas::CanvasViewportConfig::for_dpi(1.0),
            );
            assert!(snapshot.is_some(), "test canvas viewport should prepare");
        }
        runtime
    }

    #[test]
    fn active_canvas_exposes_scrollbars_and_applies_viewport_actions() {
        let mut runtime = runtime_with_prepared_canvas();
        let before = runtime
            .active_canvas_viewport_snapshot()
            .expect("prepared canvas should expose a viewport snapshot");
        let scrollbars = runtime
            .active_canvas_scrollbars_input()
            .expect("overflowing canvas should expose scrollbar input");

        assert!(scrollbars.horizontal.is_some());
        assert!(scrollbars.vertical.is_some());

        let outcome = runtime.apply_active_canvas_viewport_action(
            crate::canvas_viewport::CanvasViewportAction::ZoomBy {
                factor: 1.25,
                screen_anchor: ui::canvas::CanvasPoint::new(700.0, 480.0),
            },
        );
        let after = runtime
            .active_canvas_viewport_snapshot()
            .expect("canvas action should retain the viewport snapshot");

        assert!(outcome.shell_effect.redraw);
        assert!(after.zoom > before.zoom);
    }

    #[test]
    fn canvas_source_object_drag_crosses_threshold_and_applies_drop_transaction() {
        let mut runtime = runtime_with_clean_tab();
        let phases = Rc::new(RefCell::new(Vec::new()));
        let tab_id = runtime.active_tab_id().expect("test canvas drag tab should be active");
        runtime
            .tab_session_mut(tab_id)
            .expect("test canvas drag tab should exist")
            .runtime
            .plugin = Box::new(CanvasDragProbePlugin { phases: phases.clone() });
        let context = EditorInputContext {
            editor_rect: ui::Rect::new(100.0, 50.0, 800.0, 600.0),
            focus: EditorFocus::Active,
            modal_blocked: false,
        };

        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: 300.0, py: 240.0, button: ui::MouseButton::Left },
        );
        assert_eq!(runtime.pointer_capture(), MouseCapture::CanvasDrag);

        runtime.handle_pointer_event(context, &ui::Event::MouseMove { px: 302.0, py: 242.0 });
        assert!(phases.borrow().is_empty(), "small pointer jitter must not start a drag");

        runtime.handle_pointer_event(context, &ui::Event::MouseMove { px: 310.0, py: 250.0 });
        runtime.handle_pointer_event(context, &ui::Event::MouseMove { px: 320.0, py: 260.0 });
        let outcome = runtime.handle_pointer_event(
            context,
            &ui::Event::MouseUp { px: 320.0, py: 260.0, button: ui::MouseButton::Left },
        );

        assert_eq!(
            *phases.borrow(),
            vec![
                ui::plugin::CanvasDragPhase::Start,
                ui::plugin::CanvasDragPhase::Update,
                ui::plugin::CanvasDragPhase::Drop,
            ]
        );
        assert_eq!(runtime.workspace_snapshot().tabs[0].content_lines, vec!["moved"]);
        assert!(outcome.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ContentChanged { tab_id: changed_tab_id, .. }
                if *changed_tab_id == tab_id
        )));
        assert_eq!(runtime.pointer_capture(), MouseCapture::None);
    }

    #[test]
    fn runtime_can_be_constructed_without_product_state() {
        let view_routes =
            crate::view_route::ViewRouteTable::new(Vec::new(), &std::collections::HashSet::new())
                .expect("an empty route table must be valid");
        let mut runtime = EditorRuntime::new(EditorRuntimeConfig {
            plugin_registry: ui::plugin::PluginRegistry::new(),
            view_routes,
            initial_settings: ui::Settings::new(),
            initial_theme: ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark()),
            snapshots_directory: PathBuf::from("snapshots"),
        })
        .expect("runtime contract shell should construct");

        let mut frame = runtime.begin_frame().expect("headless frame should begin");
        frame
            .paint_editor(ui::Rect::new(16.0, 24.0, 320.0, 240.0))
            .expect("headless editor rect should paint");
        frame.present().expect("headless frame should present");
    }

    #[test]
    fn active_document_uses_the_shared_editor_surface_in_a_product_frame() {
        let mut runtime = runtime_with_clean_tab();
        let mut resources = runtime.take_render_resources();
        let mut frame = runtime.begin_frame().expect("headless frame should begin");

        let painted = runtime
            .paint_active_editor(
                &mut frame,
                &mut resources,
                ui::Rect::new(240.0, 32.0, 640.0, 480.0),
            )
            .expect("active editor should accept a product-owned rect");

        assert!(matches!(painted, EditorSurfacePaint::Document { .. }));
        runtime.restore_render_resources(resources);
    }

    #[test]
    fn closing_a_tab_invalidates_late_reshape_results() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        let result = ReshapeResult {
            generation: runtime.reshape_generation(),
            doc_line: 0,
            entry: crate::snap_tree::DisplayLineEntry::placeholder(0, 5, 1, 1),
            dv_idx: 0,
        };

        let close_outcome = runtime.request_close(tab_id);

        assert!(close_outcome.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ActiveDocumentChanged { tab_id: None }
        )));
        assert!(runtime.document_summary(tab_id).is_none());
        assert!(!runtime.accepts_reshape_result(&result, 0));
    }

    #[test]
    fn committed_text_mutates_the_active_document_and_reports_content_change() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        let context = EditorInputContext {
            editor_rect: ui::Rect::new(0.0, 0.0, 640.0, 480.0),
            focus: EditorFocus::Active,
            modal_blocked: false,
        };

        let outcome = runtime.commit_text(context, "中".to_owned());

        assert_eq!(runtime.workspace_snapshot().tabs[0].content_lines, vec!["clean中"]);
        assert!(outcome.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ContentChanged { tab_id: changed_tab_id, content_revision: 1 }
                if *changed_tab_id == tab_id
        )));
        assert!(outcome.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::DirtyChanged { tab_id: changed_tab_id, dirty: true }
                if *changed_tab_id == tab_id
        )));
    }

    #[test]
    fn custom_editor_pointer_press_places_the_document_caret() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime
            .tab_session_mut(tab_id)
            .expect("active tab should have a runtime")
            .replace_plugin(Box::new(PointerProbePlugin));
        let context = EditorInputContext {
            editor_rect: ui::Rect::new(100.0, 200.0, 640.0, 480.0),
            focus: EditorFocus::Active,
            modal_blocked: false,
        };

        let outcome = runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: 180.0, py: 260.0, button: ui::MouseButton::Left },
        );

        assert_eq!(runtime.workspace_snapshot().tabs[0].cursor_offset, 2);
        assert_ne!(outcome, EditorOutcome::default());
    }

    #[test]
    fn text_editor_pointer_press_uses_the_rendered_cluster_cache() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime
            .tab_session_mut(tab_id)
            .expect("active tab should have a runtime")
            .display_mut()
            .advance_cache = vec![ui::render_geom::AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 0,
            vl_grapheme_start: 0,
            clusters: vec![(1, 180.0, 0), (2, 220.0, 1), (3, 260.0, 2)],
        }];
        let context = EditorInputContext {
            editor_rect: ui::Rect::new(100.0, 200.0, 640.0, 480.0),
            focus: EditorFocus::Active,
            modal_blocked: false,
        };

        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: 210.0, py: 205.0, button: ui::MouseButton::Left },
        );

        assert_eq!(runtime.workspace_snapshot().tabs[0].cursor_offset, 2);
    }

    #[test]
    fn revision_checked_document_replacement_uses_undo_and_rejects_stale_or_invalid_ranges() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        let snapshot = runtime.document_text_snapshot(tab_id).expect("text snapshot should exist");

        let outcome = runtime
            .replace_document_text(DocumentTextReplacement {
                tab_id,
                content_revision: snapshot.content_revision,
                range: 0..5,
                replacement: "changed".to_owned(),
            })
            .expect("revision-checked replacement should succeed");
        assert_eq!(
            runtime.document_text_snapshot(tab_id).expect("snapshot should exist").text,
            "changed"
        );
        assert!(outcome.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ContentChanged { tab_id: changed_tab_id, .. }
                if *changed_tab_id == tab_id
        )));

        let stale_error = runtime
            .replace_document_text(DocumentTextReplacement {
                tab_id,
                content_revision: snapshot.content_revision,
                range: 0..0,
                replacement: "stale".to_owned(),
            })
            .expect_err("stale replacement must not overwrite newer input");
        assert!(matches!(stale_error, DocumentTextEditError::StaleRevision { .. }));

        let current_revision =
            runtime.document_text_snapshot(tab_id).expect("snapshot should exist").content_revision;
        let invalid_error = runtime
            .replace_document_text(DocumentTextReplacement {
                tab_id,
                content_revision: current_revision,
                range: 0..8,
                replacement: "invalid".to_owned(),
            })
            .expect_err("out-of-bounds byte range must be rejected");
        assert!(matches!(invalid_error, DocumentTextEditError::InvalidByteRange { .. }));

        let undo_outcome = runtime.handle_key_input(
            EditorInputContext {
                editor_rect: ui::Rect::new(0.0, 0.0, 640.0, 480.0),
                focus: EditorFocus::Active,
                modal_blocked: false,
            },
            ui::KeyCode::Char('z'),
            ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE },
        );
        assert!(undo_outcome.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ContentChanged { tab_id: changed_tab_id, .. }
                if *changed_tab_id == tab_id
        )));
        assert_eq!(
            runtime.document_text_snapshot(tab_id).expect("snapshot should exist").text,
            "clean"
        );
    }

    #[test]
    fn editor_key_input_is_rejected_without_editor_focus() {
        let mut runtime = runtime_with_clean_tab();
        let context = EditorInputContext {
            editor_rect: ui::Rect::new(0.0, 0.0, 640.0, 480.0),
            focus: EditorFocus::Inactive,
            modal_blocked: false,
        };

        let outcome =
            runtime.handle_key_input(context, ui::KeyCode::Backspace, ui::core::Modifiers::NONE);

        assert_eq!(outcome, EditorOutcome::default());
        assert_eq!(runtime.workspace_snapshot().tabs[0].content_lines, vec!["clean"]);
    }
}
