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
    EditorPointerOutcome, EditorRuntimeConfig, EditorRuntimeError, EditorTabSnapshot,
    EditorWorkspaceSnapshot, OpenDisposition,
};
pub use document_save::{
    PreparedDocumentSave, SaveCompletion, SavePayloadTransform, SavePrepareError,
    execute_prepared_save, execute_prepared_save_with_transform,
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
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::prepared_tab::PreparedTab;
use crate::reshape_worker::{ReshapeRequest, ReshapeResult, ReshapeWorker};
use crate::tab_runtime::TabRuntimeStore;
use crate::workspace::Workspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardCommand {
    Copy,
    Cut,
    Paste,
}

fn clipboard_command_for_key(
    key: ui::KeyCode,
    modifiers: ui::core::Modifiers,
) -> Option<ClipboardCommand> {
    if !(modifiers.cmd || modifiers.ctrl) {
        return None;
    }

    match key {
        ui::KeyCode::Char('c' | 'C') => Some(ClipboardCommand::Copy),
        ui::KeyCode::Char('x' | 'X') => Some(ClipboardCommand::Cut),
        ui::KeyCode::Char('v' | 'V') => Some(ClipboardCommand::Paste),
        _ => None,
    }
}

const CURSOR_BLINK_INTERVAL_MS: u64 = 500;
const CURSOR_BLINK_WAKE_TOLERANCE_MS: u64 = 5;
const IME_ANCHOR_WIDTH_PX: f32 = 2.0;

type PaintedEditorBounds = Rc<Cell<Option<ui::Rect>>>;

fn editor_scrollbar_input(
    viewport_extent: f32,
    total_extent: f32,
    scroll_position: f32,
) -> Option<ui::scrollbar::ScrollbarInput> {
    if !viewport_extent.is_finite()
        || !total_extent.is_finite()
        || !scroll_position.is_finite()
        || viewport_extent <= 0.0
        || total_extent <= viewport_extent
    {
        return None;
    }
    Some(ui::scrollbar::ScrollbarInput {
        viewport_height_px: f64::from(viewport_extent),
        total_display_rows: total_extent.ceil() as usize,
        scroll_top_rows: f64::from(scroll_position.max(0.0)),
    })
}

fn redraw_editor_outcome() -> EditorOutcome {
    EditorOutcome { shell_effect: crate::event::ShellEffect::REDRAW, ..EditorOutcome::default() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorCursorBlinkPhase {
    pub visible: bool,
    pub next_transition_at: std::time::Instant,
}

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
    active_cursor_paint_enabled: bool,
    plain_text_preedit_advance_px: f32,
    painted_editor_bounds: PaintedEditorBounds,
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
            active_cursor_paint_enabled: true,
            plain_text_preedit_advance_px: 0.0,
            painted_editor_bounds: Rc::new(Cell::new(None)),
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

    pub fn submit_save_with_transform(
        &mut self,
        prepared: PreparedDocumentSave,
        transform: SavePayloadTransform,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), String> {
        self.save_session.submit_with_transform(prepared, Some(transform), wake)
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
        self.painted_editor_bounds.set(None);
        let theme = self.theme.clone();
        let dpi = self.render_session.scale_factor() as f32;
        Ok(EditorFrame::new_for_backend(
            theme,
            dpi,
            self.ui_shaper.clone(),
            Rc::clone(&self.painted_editor_bounds),
        ))
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
        let canvas_pointer_changed = !focused && self.clear_active_canvas_pointer();
        if self.render_session.window_focused() != focused || canvas_pointer_changed {
            self.render_session.request_redraw();
        }
        self.render_session.set_window_focused(focused);
    }

    pub fn window_focused(&self) -> bool {
        self.render_session.window_focused()
    }

    pub fn active_cursor_blink_phase(&self) -> Option<EditorCursorBlinkPhase> {
        if !self.window_focused() || !self.active_cursor_paint_enabled {
            return None;
        }
        let tab_id = self.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        if !tab.needs_cursor_blink_wakeup() {
            return None;
        }
        Some(cursor_blink_phase(tab.cursor_blink_instant(), std::time::Instant::now()))
    }

    pub fn set_active_cursor_paint_enabled(&mut self, enabled: bool) {
        self.active_cursor_paint_enabled = enabled;
    }

    pub fn active_cursor_paint_enabled(&self) -> bool {
        self.active_cursor_paint_enabled
    }

    /// 返回活动编辑器光标的窗口坐标矩形，供产品层定位系统 IME 候选窗。
    pub fn active_editor_ime_cursor_rect(&self) -> Option<ui::Rect> {
        let editor_bounds = self.painted_editor_bounds()?;
        let tab_id = self.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        if tab.handles_own_rendering() {
            let cursor_byte = tab.document.cursor_offset().to_usize();
            let (cursor_x, cursor_y, cursor_width, cursor_height) =
                tab.query_cursor_screen_rect(cursor_byte)?;
            let bounds = editor_painter::plugin_bounds(
                editor_bounds,
                self.scale_factor() as f32,
                tab.is_canvas(),
            );
            return Some(ui::Rect::new(
                bounds.x + cursor_x,
                bounds.y + cursor_y,
                cursor_width,
                cursor_height,
            ));
        }

        let metrics =
            ui::settings::UiMetrics::from_settings(&self.settings, self.scale_factor() as f32);
        let cursor_visual_line = tab.cursor_visual_line()?;
        let cursor_x = tab.cursor_pixel_x() + self.plain_text_preedit_advance_px;
        let cursor_y = editor_bounds.y
            + cursor_visual_line as f32 * metrics.line_height
            + tab.sub_line_pixel_offset(metrics.line_height);
        Some(ui::Rect::new(cursor_x, cursor_y, IME_ANCHOR_WIDTH_PX, metrics.line_height))
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

        if let Some(clipboard_command) = clipboard_command_for_key(key, modifiers) {
            return self.handle_clipboard_command(clipboard_command);
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

    fn handle_clipboard_command(&mut self, command: ClipboardCommand) -> EditorOutcome {
        match command {
            ClipboardCommand::Copy => {
                if let Some(text) = self.model_session.active_selected_text() {
                    let _ = crate::clipboard::try_write_text(&text);
                }
                EditorOutcome::default()
            }
            ClipboardCommand::Cut => {
                let Some(text) = self.model_session.active_selected_text() else {
                    return EditorOutcome::default();
                };
                if !crate::clipboard::try_write_text(&text) {
                    return EditorOutcome::default();
                }
                self.model_session.edit_active_document(
                    ui::plugin::EditIntent::DeleteBackward,
                    self.editor_line_height(),
                )
            }
            ClipboardCommand::Paste => {
                let Some(text) = crate::clipboard::try_read_text() else {
                    return EditorOutcome::default();
                };
                let normalized_text = text.replace("\r\n", "\n").replace('\r', "\n");
                if normalized_text.is_empty() {
                    return EditorOutcome::default();
                }
                self.model_session.edit_active_document(
                    ui::plugin::EditIntent::InsertText(normalized_text),
                    self.editor_line_height(),
                )
            }
        }
    }

    pub fn execute_semantic_edit(
        &mut self,
        command: ui::plugin::SemanticEditCommand,
    ) -> (SemanticEditResult, EditorOutcome) {
        self.model_session.execute_semantic_edit(command, self.editor_line_height())
    }

    pub fn apply_active_mindmap_theme(&mut self, theme_id: String) -> EditorOutcome {
        self.model_session.apply_active_mindmap_theme(theme_id, self.editor_line_height())
    }

    pub fn scroll_editor(
        &mut self,
        context: EditorInputContext,
        position: (f32, f32),
        pixels: f32,
    ) -> EditorOutcome {
        if !self.editor_hit_test_allowed(context, position) {
            return EditorOutcome::default();
        }
        let Some(editor_bounds) = self.painted_editor_bounds() else {
            return EditorOutcome::default();
        };
        let plugin_viewport_height =
            editor_painter::plugin_bounds(editor_bounds, self.scale_factor() as f32, false).h;
        self.model_session.scroll_active_document(
            pixels,
            plugin_viewport_height,
            self.editor_line_height(),
        )
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

    /// 将活动编辑器的滚动范围转换为产品壳可直接渲染的覆盖式滚动条输入。
    pub fn active_editor_scrollbars_input(
        &self,
    ) -> Option<ui::canvas_scrollbars::CanvasScrollbarsInput> {
        let editor_bounds = self.painted_editor_bounds()?;
        let tab_id = self.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        if tab.is_canvas() {
            if !tab.has_canvas_viewport_snapshot() {
                return None;
            }
            let input = tab.runtime.canvas_viewport.scrollbars_input();
            return Some(ui::canvas_scrollbars::CanvasScrollbarsInput {
                horizontal: input.horizontal,
                vertical: input.vertical,
            });
        }

        let vertical = if tab.handles_own_rendering() {
            let viewport_height =
                editor_painter::plugin_bounds(editor_bounds, self.scale_factor() as f32, false).h;
            editor_scrollbar_input(viewport_height, tab.content_height(), tab.scroll_y())
        } else {
            editor_scrollbar_input(
                tab.viewport_height() as f32,
                tab.total_display_rows() as f32,
                tab.scroll_top() as f32,
            )
        };
        vertical.map(|vertical| ui::canvas_scrollbars::CanvasScrollbarsInput {
            horizontal: None,
            vertical: Some(vertical),
        })
    }

    /// 把产品壳滚动条动作归约到活动编辑器；画布与普通文档共享同一入口。
    pub fn apply_active_scrollbar_action(
        &mut self,
        action: ui::canvas_scrollbars::CanvasScrollbarsAction,
    ) -> EditorOutcome {
        let Some(editor_bounds) = self.painted_editor_bounds() else {
            return EditorOutcome::default();
        };
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        let is_canvas = self.tab_session(tab_id).is_some_and(|tab| tab.is_canvas());
        if is_canvas {
            return self.apply_canvas_scrollbar_action(action);
        }
        self.apply_document_scrollbar_action(tab_id, action, editor_bounds)
    }

    fn apply_canvas_scrollbar_action(
        &mut self,
        action: ui::canvas_scrollbars::CanvasScrollbarsAction,
    ) -> EditorOutcome {
        use crate::canvas_viewport::CanvasViewportAction;
        use ui::scrollbar::ScrollbarAction;

        let viewport_action = match action.action {
            ScrollbarAction::DragTo(position) => CanvasViewportAction::SetAxisPosition {
                axis: action.axis,
                position: position as f32,
            },
            ScrollbarAction::PageUp => {
                CanvasViewportAction::Page { axis: action.axis, direction: -1.0 }
            }
            ScrollbarAction::PageDown => {
                CanvasViewportAction::Page { axis: action.axis, direction: 1.0 }
            }
            ScrollbarAction::StartDrag
            | ScrollbarAction::EndDrag
            | ScrollbarAction::HoverChanged(_) => return redraw_editor_outcome(),
        };
        self.apply_active_canvas_viewport_action(viewport_action)
    }

    fn apply_document_scrollbar_action(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        action: ui::canvas_scrollbars::CanvasScrollbarsAction,
        editor_rect: ui::Rect,
    ) -> EditorOutcome {
        use ui::canvas::CanvasAxis;
        use ui::scrollbar::ScrollbarAction;

        if action.axis != CanvasAxis::Vertical {
            return EditorOutcome::default();
        }

        let line_height = self.editor_line_height();
        let plugin_viewport_height =
            editor_painter::plugin_bounds(editor_rect, self.scale_factor() as f32, false).h;
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return EditorOutcome::default();
        };
        let handles_own_rendering = tab.runtime.plugin.handles_own_rendering();
        match action.action {
            ScrollbarAction::DragTo(position) if handles_own_rendering => {
                tab.send_message(ui::plugin::PluginMessage::SetScrollY(position as f32));
            }
            ScrollbarAction::PageUp if handles_own_rendering => {
                tab.send_message(ui::plugin::PluginMessage::Scroll {
                    delta: -plugin_viewport_height,
                    viewport_h: plugin_viewport_height,
                });
            }
            ScrollbarAction::PageDown if handles_own_rendering => {
                tab.send_message(ui::plugin::PluginMessage::Scroll {
                    delta: plugin_viewport_height,
                    viewport_h: plugin_viewport_height,
                });
            }
            ScrollbarAction::DragTo(position) => tab.set_scroll_top_rows(position, line_height),
            ScrollbarAction::PageUp => tab.scroll_viewport_by_pages(-1.0, line_height),
            ScrollbarAction::PageDown => tab.scroll_viewport_by_pages(1.0, line_height),
            ScrollbarAction::StartDrag
            | ScrollbarAction::EndDrag
            | ScrollbarAction::HoverChanged(_) => return redraw_editor_outcome(),
        }
        EditorOutcome {
            shell_effect: if handles_own_rendering {
                crate::event::ShellEffect::REDRAW
            } else {
                crate::event::ShellEffect::RESHAPE
            },
            ..EditorOutcome::default()
        }
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

    pub fn editor_hit_test_allowed(
        &self,
        context: EditorInputContext,
        position: (f32, f32),
    ) -> bool {
        !context.modal_blocked
            && self
                .painted_editor_bounds()
                .is_some_and(|bounds| bounds.contains(position.0, position.1))
    }

    pub fn pointer_input_allowed(&self, context: EditorInputContext, position: (f32, f32)) -> bool {
        let pointer_inside_editor = self
            .painted_editor_bounds()
            .is_some_and(|bounds| bounds.contains(position.0, position.1));
        self.input_session.pointer_allowed(context, pointer_inside_editor)
    }

    fn painted_editor_bounds(&self) -> Option<ui::Rect> {
        self.painted_editor_bounds.get()
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
        let update = self.input_session.update_preedit(context, text, cursor);
        if update == input_session::PreeditUpdate::Changed {
            if let Some(tab_id) = self.active_tab_id()
                && let Some(mut tab) = self.tab_session_mut(tab_id)
            {
                tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
            }
            self.render_session.request_redraw();
        }
        update.accepted()
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

    pub fn focus_lost(&mut self) {
        let canvas_pointer_changed = self.clear_active_canvas_pointer();
        self.input_session.focus_lost();
        self.render_session.set_window_focused(false);
        if canvas_pointer_changed {
            self.render_session.request_redraw();
        }
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

fn cursor_blink_phase(
    started_at: std::time::Instant,
    now: std::time::Instant,
) -> EditorCursorBlinkPhase {
    let elapsed_ms = now.saturating_duration_since(started_at).as_millis() as u64;
    let cycle_ms = CURSOR_BLINK_INTERVAL_MS * 2;
    let phase_ms = elapsed_ms % cycle_ms;
    let visible = phase_ms < CURSOR_BLINK_INTERVAL_MS;
    let transition_delay_ms =
        if visible { CURSOR_BLINK_INTERVAL_MS - phase_ms } else { cycle_ms - phase_ms };
    EditorCursorBlinkPhase {
        visible,
        next_transition_at: now
            + std::time::Duration::from_millis(
                transition_delay_ms + CURSOR_BLINK_WAKE_TOLERANCE_MS,
            ),
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
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::rc::Rc;
    use ui::plugin::PluginFactory;

    struct PointerProbePlugin;

    struct WysiwygInputProbePlugin {
        scroll_y: Rc<Cell<f32>>,
        hit_test_count: Rc<Cell<usize>>,
    }

    struct CanvasViewportProbePlugin;

    struct CanvasDragProbePlugin {
        phases: Rc<RefCell<Vec<ui::plugin::CanvasDragPhase>>>,
    }

    struct CanvasControlProbePlugin {
        planned_ranges: Rc<RefCell<Vec<std::ops::Range<usize>>>>,
        pointer_positions: Rc<RefCell<Vec<Option<ui::canvas::CanvasPoint>>>>,
    }

    impl ui::plugin::ViewPlugin for CanvasControlProbePlugin {
        fn name(&self) -> &str {
            "canvas-control-probe"
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
                        ui::plugin::EditHitTarget::CanvasControl { source_range: 1..2 },
                    ))
                }
                ui::plugin::PluginQuery::PlanCanvasControl { source_range, source_generation } => {
                    self.planned_ranges.borrow_mut().push(source_range);
                    ui::plugin::PluginResponse::EditPlan(ui::plugin::EditPlan::Apply(
                        ui::plugin::EditTransaction::replace(
                            source_generation,
                            1..2,
                            "expanded".to_owned(),
                            8,
                        ),
                    ))
                }
                ui::plugin::PluginQuery::ContentHeight => {
                    ui::plugin::PluginResponse::Float(1_000.0)
                }
                _ => ui::plugin::PluginResponse::None,
            }
        }

        fn handle_message(
            &mut self,
            message: ui::plugin::PluginMessage,
            _document: &mut dyn core::document::DocViewMut,
        ) -> bool {
            let ui::plugin::PluginMessage::SetCanvasPointer(pointer) = message else {
                return false;
            };
            self.pointer_positions.borrow_mut().push(pointer);
            true
        }

        fn handles_own_rendering(&self) -> bool {
            true
        }

        fn is_canvas(&self) -> bool {
            true
        }
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

        fn needs_cursor_blink_wakeup(&self) -> bool {
            true
        }
    }

    impl ui::plugin::ViewPlugin for WysiwygInputProbePlugin {
        fn name(&self) -> &str {
            "wysiwyg-input-probe"
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

        fn handle_message(
            &mut self,
            message: ui::plugin::PluginMessage,
            _document: &mut dyn core::document::DocViewMut,
        ) -> bool {
            match message {
                ui::plugin::PluginMessage::Scroll { delta, .. } => {
                    self.scroll_y.set(self.scroll_y.get() + delta);
                    true
                }
                ui::plugin::PluginMessage::SetCursorByte(_) => true,
                _ => false,
            }
        }

        fn query(
            &self,
            query: ui::plugin::PluginQuery,
            _document: &dyn core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            match query {
                ui::plugin::PluginQuery::ScrollY => {
                    ui::plugin::PluginResponse::Float(self.scroll_y.get())
                }
                ui::plugin::PluginQuery::HitTestByte { .. } => {
                    let query_index = self.hit_test_count.get();
                    self.hit_test_count.set(query_index + 1);
                    let byte = if query_index == 0 { 2 } else { 3 };
                    ui::plugin::PluginResponse::BytePosition(Some(byte))
                }
                ui::plugin::PluginQuery::CursorScreenPos(_) => {
                    ui::plugin::PluginResponse::CursorScreenRect(Some((24.0, 12.0, 2.0, 18.0)))
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

    fn paint_editor_surface(runtime: &mut EditorRuntime, editor_bounds: ui::Rect) {
        let mut frame = runtime.begin_frame().expect("headless editor frame should begin");
        frame.paint_editor(editor_bounds).expect("finite editor bounds should paint");
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
        assert_eq!(runtime.active_editor_scrollbars_input(), None);
        paint_editor_surface(&mut runtime, ui::Rect::new(200.0, 80.0, 1_000.0, 800.0));
        let before = runtime
            .active_canvas_viewport_snapshot()
            .expect("prepared canvas should expose a viewport snapshot");
        let scrollbars = runtime
            .active_editor_scrollbars_input()
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
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 50.0, 800.0, 600.0));

        let hover_outcome =
            runtime.handle_pointer_event(context, &ui::Event::MouseMove { px: 300.0, py: 240.0 });
        assert_eq!(hover_outcome.cursor_icon, Some(winit::window::CursorIcon::Grab));

        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: 300.0, py: 240.0, button: ui::MouseButton::Left },
        );
        assert_eq!(runtime.pointer_capture(), MouseCapture::CanvasDrag);

        let captured_move =
            runtime.handle_pointer_event(context, &ui::Event::MouseMove { px: 302.0, py: 242.0 });
        assert_eq!(captured_move.cursor_icon, Some(winit::window::CursorIcon::Grabbing));
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
        assert!(outcome.editor.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ContentChanged { tab_id: changed_tab_id, .. }
                if *changed_tab_id == tab_id
        )));
        assert_eq!(runtime.pointer_capture(), MouseCapture::None);
    }

    #[test]
    fn canvas_control_press_applies_its_edit_plan() {
        let mut runtime = runtime_with_clean_tab();
        let planned_ranges = Rc::new(RefCell::new(Vec::new()));
        let tab_id = runtime.active_tab_id().expect("test canvas control tab should be active");
        runtime
            .tab_session_mut(tab_id)
            .expect("test canvas control tab should exist")
            .runtime
            .plugin = Box::new(CanvasControlProbePlugin {
            planned_ranges: planned_ranges.clone(),
            pointer_positions: Rc::new(RefCell::new(Vec::new())),
        });
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 50.0, 800.0, 600.0));

        let outcome = runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: 300.0, py: 240.0, button: ui::MouseButton::Left },
        );

        assert_eq!(*planned_ranges.borrow(), vec![1..2]);
        assert_eq!(runtime.workspace_snapshot().tabs[0].content_lines, vec!["cexpandedean"]);
        assert_eq!(outcome.cursor_icon, Some(winit::window::CursorIcon::Pointer));
        assert!(outcome.editor.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ContentChanged { tab_id: changed_tab_id, .. }
                if *changed_tab_id == tab_id
        )));
        assert_eq!(runtime.pointer_capture(), MouseCapture::None);
    }

    #[test]
    fn canvas_pointer_hover_is_forwarded_and_cleared_by_pointer_lifecycle() {
        let mut runtime = runtime_with_clean_tab();
        let pointer_positions = Rc::new(RefCell::new(Vec::new()));
        let tab_id = runtime.active_tab_id().expect("test canvas hover tab should be active");
        runtime
            .tab_session_mut(tab_id)
            .expect("test canvas hover tab should exist")
            .runtime
            .plugin = Box::new(CanvasControlProbePlugin {
            planned_ranges: Rc::new(RefCell::new(Vec::new())),
            pointer_positions: pointer_positions.clone(),
        });
        let context = EditorInputContext { focus: EditorFocus::Inactive, modal_blocked: false };
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 50.0, 800.0, 600.0));
        let hovered_point = ui::canvas::CanvasPoint::new(300.0, 240.0);

        let hover_outcome = runtime.handle_pointer_event(
            context,
            &ui::Event::MouseMove { px: hovered_point.x, py: hovered_point.y },
        );
        let leave_outcome =
            runtime.handle_pointer_event(context, &ui::Event::MouseMove { px: 20.0, py: 20.0 });
        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseMove { px: hovered_point.x, py: hovered_point.y },
        );
        runtime.set_window_focus(false);

        assert_eq!(
            *pointer_positions.borrow(),
            vec![Some(hovered_point), None, Some(hovered_point), None]
        );
        assert!(hover_outcome.editor.shell_effect.redraw);
        assert!(leave_outcome.editor.shell_effect.redraw);
        assert_eq!(hover_outcome.cursor_icon, Some(winit::window::CursorIcon::Pointer));
        assert_eq!(leave_outcome.cursor_icon, None);
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
    fn cursor_blink_phase_alternates_and_schedules_the_next_transition() {
        let started_at = std::time::Instant::now();
        let visible_now = cursor_blink_phase(started_at, started_at);
        let hidden_now = cursor_blink_phase(
            started_at,
            started_at + std::time::Duration::from_millis(CURSOR_BLINK_INTERVAL_MS),
        );

        assert!(visible_now.visible);
        assert!(!hidden_now.visible);
        assert_eq!(
            visible_now.next_transition_at,
            started_at
                + std::time::Duration::from_millis(
                    CURSOR_BLINK_INTERVAL_MS + CURSOR_BLINK_WAKE_TOLERANCE_MS,
                )
        );
        assert_eq!(
            hidden_now.next_transition_at,
            started_at
                + std::time::Duration::from_millis(
                    CURSOR_BLINK_INTERVAL_MS * 2 + CURSOR_BLINK_WAKE_TOLERANCE_MS,
                )
        );
    }

    #[test]
    fn changed_preedit_immediately_requests_redraw_and_restarts_active_cursor_blink() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        let stale_blink_started_at =
            std::time::Instant::now() - std::time::Duration::from_millis(750);
        runtime
            .tab_session_mut(tab_id)
            .expect("active tab should have a runtime")
            .cursor_render_state_mut()
            .cursor_blink_instant = stale_blink_started_at;
        let _ = runtime.take_redraw_request();
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };

        assert!(runtime.update_preedit(context, "拼音".to_owned(), Some((0, 6))));

        assert!(runtime.take_redraw_request());
        let restarted_blink_started_at = runtime
            .tab_session(tab_id)
            .expect("active tab should remain available")
            .cursor_blink_instant();
        assert!(restarted_blink_started_at > stale_blink_started_at);
        assert!(runtime.active_cursor_blink_phase().expect("active cursor should blink").visible);

        assert!(runtime.update_preedit(context, "拼音".to_owned(), Some((0, 6))));
        assert!(!runtime.take_redraw_request());
        assert_eq!(
            runtime
                .tab_session(tab_id)
                .expect("active tab should remain available")
                .cursor_blink_instant(),
            restarted_blink_started_at
        );
    }

    #[test]
    fn disabling_active_cursor_paint_suppresses_its_blink_phase() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime
            .tab_session_mut(tab_id)
            .expect("active tab should have a runtime")
            .replace_plugin(Box::new(PointerProbePlugin));

        assert!(runtime.active_cursor_blink_phase().is_some());

        runtime.set_active_cursor_paint_enabled(false);

        assert!(!runtime.active_cursor_paint_enabled());
        assert_eq!(runtime.active_cursor_blink_phase(), None);
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
    fn first_plain_text_frame_without_render_resources_uses_the_editor_viewport() {
        let mut runtime = runtime_with_clean_tab();
        let focused_context =
            EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        let _ = runtime.commit_text(focused_context, "\nsecond line".to_owned());
        let editor_rect = ui::Rect::new(240.0, 32.0, 640.0, 480.0);
        let mut resources = runtime.take_render_resources();
        assert!(resources.text.is_none());
        assert!(resources.gpu.is_none());
        let mut frame = runtime.begin_frame().expect("headless frame should begin");

        runtime
            .paint_active_editor(&mut frame, &mut resources, editor_rect)
            .expect("plain-text editor should measure its viewport before resources are ready");

        let tab_id = runtime.active_tab_id().expect("first TXT tab should stay active");
        let tab = runtime.tab_session(tab_id).expect("first TXT tab should remain available");
        assert!(tab.viewport_height() > 2.0);
        assert_eq!(tab.scroll_top(), 0.0, "resizing must clamp the startup scroll anchor");
        assert_eq!(
            runtime.active_editor_scrollbars_input(),
            None,
            "a short first TXT document must not expose a transient scrollbar"
        );
        runtime.restore_render_resources(resources);
    }

    #[test]
    fn second_loaded_plain_text_tab_exposes_overflow_before_render_resources_are_ready() {
        let mut runtime = runtime_with_clean_tab();
        let source = (0..100).map(|line| format!("line {line}\n")).collect::<String>();
        let mut text_buffer = TextBuffer::new(false).expect("second TXT buffer should be writable");
        text_buffer.write_raw(source.as_bytes());
        runtime.install_prepared_tab(
            PreparedTab::new(
                DocumentModel::new(text_buffer),
                TabRuntime::new(EditorPluginFactory.create()),
            ),
            None,
            OpenDisposition::Persistent,
        );
        let editor_rect = ui::Rect::new(240.0, 32.0, 640.0, 480.0);
        let mut resources = runtime.take_render_resources();
        assert!(resources.text.is_none());
        assert!(resources.gpu.is_none());
        let mut frame = runtime.begin_frame().expect("second TXT frame should begin");

        runtime
            .paint_active_editor(&mut frame, &mut resources, editor_rect)
            .expect("second TXT should synchronize its viewport without render resources");

        let scrollbar = runtime
            .active_editor_scrollbars_input()
            .and_then(|input| input.vertical)
            .expect("overflowing second TXT document should expose a vertical scrollbar");
        assert_eq!(scrollbar.total_display_rows, 101);
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
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };

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
    fn committed_text_cannot_mutate_a_read_only_runtime_tab() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime
            .tab_session_mut(tab_id)
            .expect("active tab should exist")
            .runtime
            .set_editing_access(crate::tab_runtime::DocumentEditingAccess::ReadOnly);
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };

        let outcome = runtime.commit_text(context, "中".to_owned());

        assert_eq!(runtime.workspace_snapshot().tabs[0].content_lines, vec!["clean"]);
        assert!(outcome.notifications.is_empty());
    }

    #[test]
    fn pointer_hit_testing_uses_only_the_current_frames_editor_bounds() {
        let mut runtime = runtime_with_clean_tab();
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        let editor_bounds = ui::Rect::new(100.0, 200.0, 640.0, 480.0);

        assert!(!runtime.pointer_input_allowed(context, (180.0, 260.0)));
        paint_editor_surface(&mut runtime, editor_bounds);
        assert!(runtime.pointer_input_allowed(context, (180.0, 260.0)));
        assert!(!runtime.pointer_input_allowed(context, (80.0, 260.0)));

        let _next_frame = runtime.begin_frame().expect("next headless frame should begin");
        assert!(!runtime.pointer_input_allowed(context, (180.0, 260.0)));
    }

    #[test]
    fn custom_editor_pointer_press_places_the_document_caret() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime
            .tab_session_mut(tab_id)
            .expect("active tab should have a runtime")
            .replace_plugin(Box::new(PointerProbePlugin));
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 200.0, 640.0, 480.0));

        let outcome = runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: 180.0, py: 260.0, button: ui::MouseButton::Left },
        );

        assert_eq!(runtime.workspace_snapshot().tabs[0].cursor_offset, 2);
        assert_ne!(outcome.editor, EditorOutcome::default());
    }

    #[test]
    fn wysiwyg_wheel_scrolls_the_plugin_viewport() {
        let mut runtime = runtime_with_clean_tab();
        let scroll_y = Rc::new(Cell::new(0.0));
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime.tab_session_mut(tab_id).expect("active tab should have a runtime").replace_plugin(
            Box::new(WysiwygInputProbePlugin {
                scroll_y: scroll_y.clone(),
                hit_test_count: Rc::new(Cell::new(0)),
            }),
        );
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 200.0, 640.0, 480.0));

        let outcome = runtime.scroll_editor(context, (180.0, 260.0), 72.0);

        assert_eq!(scroll_y.get(), 72.0);
        assert!(outcome.shell_effect.redraw);
        assert!(!outcome.shell_effect.reshape);
    }

    #[test]
    fn active_editor_ime_cursor_rect_translates_plugin_coordinates_to_window_space() {
        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime.tab_session_mut(tab_id).expect("active tab should have a runtime").replace_plugin(
            Box::new(WysiwygInputProbePlugin {
                scroll_y: Rc::new(Cell::new(0.0)),
                hit_test_count: Rc::new(Cell::new(0)),
            }),
        );
        let editor_rect = ui::Rect::new(100.0, 200.0, 640.0, 480.0);
        let mut resources = runtime.take_render_resources();
        let mut frame = runtime.begin_frame().expect("headless frame should begin");
        runtime
            .paint_active_editor(&mut frame, &mut resources, editor_rect)
            .expect("headless plugin frame should record its painted bounds");

        let ime_rect = runtime
            .active_editor_ime_cursor_rect()
            .expect("active editor cursor should provide an IME candidate anchor");

        assert_eq!(ime_rect, ui::Rect::new(148.0, 220.0, 2.0, 18.0));

        let _next_frame = runtime.begin_frame().expect("next headless frame should begin");
        assert_eq!(runtime.active_editor_ime_cursor_rect(), None);
    }

    #[test]
    fn active_plain_text_editor_exposes_its_painted_caret_as_an_ime_anchor() {
        const CURSOR_X_PX: f32 = 286.0;
        const CURSOR_VISUAL_ROW: usize = 4;
        const SUB_LINE_OFFSET_ROWS: f32 = 0.25;
        const PREEDIT_ADVANCE_PX: f32 = 42.0;

        let mut runtime = runtime_with_clean_tab();
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        {
            let mut tab = runtime
                .tab_session_mut(tab_id)
                .expect("active plain-text tab should have a runtime");
            tab.cursor_render_state_mut().cursor_pixel_x = CURSOR_X_PX;
            tab.cursor_render_state_mut().cursor_visual_line = Some(CURSOR_VISUAL_ROW);
            tab.display_mut().viewport.scroll_anchor.pixel_offset = SUB_LINE_OFFSET_ROWS;
        }
        let editor_rect = ui::Rect::new(100.0, 200.0, 640.0, 480.0);
        let metrics = ui::settings::UiMetrics::from_settings(&runtime.settings, 1.0);
        let sub_line_offset_px = runtime
            .tab_session(tab_id)
            .expect("active plain-text tab should remain available")
            .sub_line_pixel_offset(metrics.line_height);
        let mut resources = runtime.take_render_resources();
        let mut frame = runtime.begin_frame().expect("headless frame should begin");
        runtime
            .paint_active_editor(&mut frame, &mut resources, editor_rect)
            .expect("headless plain-text frame should record its painted bounds");
        runtime.plain_text_preedit_advance_px = PREEDIT_ADVANCE_PX;

        let ime_rect = runtime
            .active_editor_ime_cursor_rect()
            .expect("plain-text caret should provide an IME candidate anchor");

        assert_eq!(ime_rect.x, CURSOR_X_PX + PREEDIT_ADVANCE_PX);
        assert_eq!(
            ime_rect.y,
            editor_rect.y + CURSOR_VISUAL_ROW as f32 * metrics.line_height + sub_line_offset_px
        );
        assert_eq!(ime_rect.w, 2.0);
        assert_eq!(ime_rect.h, metrics.line_height);
    }

    #[test]
    fn wysiwyg_pointer_press_rehits_after_expanding_the_candidate_cursor() {
        let mut runtime = runtime_with_clean_tab();
        let hit_test_count = Rc::new(Cell::new(0));
        let tab_id = runtime.active_tab_id().expect("test runtime should have an active tab");
        runtime.tab_session_mut(tab_id).expect("active tab should have a runtime").replace_plugin(
            Box::new(WysiwygInputProbePlugin {
                scroll_y: Rc::new(Cell::new(0.0)),
                hit_test_count: hit_test_count.clone(),
            }),
        );
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 200.0, 640.0, 480.0));

        runtime.handle_pointer_event(
            context,
            &ui::Event::MouseDown { px: 180.0, py: 260.0, button: ui::MouseButton::Left },
        );

        assert_eq!(hit_test_count.get(), 2);
        assert_eq!(runtime.workspace_snapshot().tabs[0].cursor_offset, 3);
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
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        paint_editor_surface(&mut runtime, ui::Rect::new(100.0, 200.0, 640.0, 480.0));

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
            EditorInputContext { focus: EditorFocus::Active, modal_blocked: false },
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
        let context = EditorInputContext { focus: EditorFocus::Inactive, modal_blocked: false };

        let outcome =
            runtime.handle_key_input(context, ui::KeyCode::Backspace, ui::core::Modifiers::NONE);

        assert_eq!(outcome, EditorOutcome::default());
        assert_eq!(runtime.workspace_snapshot().tabs[0].content_lines, vec!["clean"]);
    }

    #[test]
    fn editor_wheel_scroll_is_allowed_without_keyboard_focus() {
        let mut runtime = runtime_with_clean_tab();
        let editor_rect = ui::Rect::new(0.0, 0.0, 640.0, 480.0);
        let focused_context =
            EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        let long_document = (0..100).map(|line| format!("line {line}\n")).collect();
        let _ = runtime.commit_text(focused_context, long_document);
        let tab_id = runtime.active_tab_id().expect("test tab should be active");
        runtime
            .tab_session_mut(tab_id)
            .expect("test tab should exist")
            .resize_presentation(10, 10.0);
        let unfocused_context =
            EditorInputContext { focus: EditorFocus::Inactive, ..focused_context };
        paint_editor_surface(&mut runtime, editor_rect);

        let outcome = runtime.scroll_editor(unfocused_context, (320.0, 240.0), 80.0);
        let scroll_top = runtime.tab_session(tab_id).expect("test tab should exist").scroll_top();

        assert!(outcome.shell_effect.redraw);
        assert!(scroll_top > 0.0, "hovered editor should scroll without keyboard focus");
    }

    #[test]
    fn overflowing_text_editor_exposes_vertical_scrollbar_and_applies_drag() {
        let mut runtime = runtime_with_clean_tab();
        let editor_rect = ui::Rect::new(0.0, 0.0, 640.0, 480.0);
        let focused_context =
            EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };
        let long_document = (0..100).map(|line| format!("line {line}\n")).collect();
        let _ = runtime.commit_text(focused_context, long_document);
        let tab_id = runtime.active_tab_id().expect("test tab should be active");
        runtime
            .tab_session_mut(tab_id)
            .expect("test tab should exist")
            .resize_presentation(10, 10.0);
        assert_eq!(runtime.active_editor_scrollbars_input(), None);
        let scroll_top_before_ignored_action =
            runtime.tab_session(tab_id).expect("test tab should exist").scroll_top();
        let ignored_outcome =
            runtime.apply_active_scrollbar_action(ui::canvas_scrollbars::CanvasScrollbarsAction {
                axis: ui::canvas::CanvasAxis::Vertical,
                action: ui::scrollbar::ScrollbarAction::DragTo(20.0),
            });
        assert_eq!(ignored_outcome, EditorOutcome::default());
        assert_eq!(
            runtime.tab_session(tab_id).expect("test tab should exist").scroll_top(),
            scroll_top_before_ignored_action
        );
        paint_editor_surface(&mut runtime, editor_rect);

        let scrollbars = runtime
            .active_editor_scrollbars_input()
            .expect("overflowing text editor should expose scrollbar input");

        assert!(scrollbars.horizontal.is_none());
        assert!(scrollbars.vertical.is_some());

        let outcome =
            runtime.apply_active_scrollbar_action(ui::canvas_scrollbars::CanvasScrollbarsAction {
                axis: ui::canvas::CanvasAxis::Vertical,
                action: ui::scrollbar::ScrollbarAction::DragTo(20.0),
            });
        let scroll_top = runtime.tab_session(tab_id).expect("test tab should exist").scroll_top();

        assert!(outcome.shell_effect.reshape);
        assert!((scroll_top - 20.0).abs() < 0.01, "scroll_top={scroll_top}");
    }

    #[test]
    fn command_modifier_clipboard_keys_are_recognized_by_editor_runtime() {
        let command_modifiers = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };
        let control_modifiers = ui::core::Modifiers { ctrl: true, ..ui::core::Modifiers::NONE };

        assert_eq!(
            clipboard_command_for_key(ui::KeyCode::Char('c'), command_modifiers),
            Some(ClipboardCommand::Copy)
        );
        assert_eq!(
            clipboard_command_for_key(ui::KeyCode::Char('x'), command_modifiers),
            Some(ClipboardCommand::Cut)
        );
        assert_eq!(
            clipboard_command_for_key(ui::KeyCode::Char('v'), control_modifiers),
            Some(ClipboardCommand::Paste)
        );
    }
}
