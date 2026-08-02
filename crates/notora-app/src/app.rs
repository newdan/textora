//! notora 窗口应用状态；编辑器会话只经 shared runtime 管理。

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use appkit_shell::editor_runtime::{
    EditorNotification, EditorOutcome, EditorRuntime, EditorRuntimeConfig, EditorRuntimeError,
    OpenDisposition, RenderError, RenderResources,
};
use appkit_shell::render_state::{GpuState, TextState};
use appkit_shell::{ProductHost, ProductWakeHandle, ShellEffect, ShellEvent};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::WindowAttributes;

use crate::action::{
    CardQuery, ConflictResolution, DocumentLoadRequest, NoteCreationTarget, NotoraAction,
    SaveConflictRequest,
};
use crate::autosave::{AutoSaveRequest, AutoSaveScheduler};
use crate::dirty_snapshot::{collect_dirty_snapshots, write_dirty_snapshot};
use crate::document_registry::DocumentRegistry;
use crate::editor_adapter::{
    LoadedDocument, build_editor_plugins, load_document, prepare_loaded_document,
    prepare_untitled_document,
};
use crate::effect_executor::{
    EffectExecutor, ExternalOpenRequest, ManualSaveRequest, NotoraEffectService,
};
use crate::events;
use crate::external_files::{
    CanonicalExternalPath, ExternalFileSession, SaveExternalFileAs, validate_external_text_file,
};
use crate::product::NotoraProduct;
use crate::render::{NotoraRenderModel, NotoraShell};
use crate::runtime_lru::{RuntimeLru, RuntimeTabState};
use crate::search_controller::SearchController;
use crate::shell::layout::{ShellLayout, ShellLayoutInput};
use crate::{
    NotoraPaths, NotoraPathsError, NotoraState, WorkspaceCommand, WorkspaceCommandResult,
    WorkspaceController, WorkspaceControllerError,
};
use notora_core::{DocumentIdentity, DocumentKind};

const DEFAULT_WINDOW_WIDTH_PX: f32 = 1_200.0;
const DEFAULT_WINDOW_HEIGHT_PX: f32 = 800.0;
const SESSION_PERSIST_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);
const CATALOG_BACKUP_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);
const SHUTDOWN_SAVE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_SAVE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_RUNTIME_TAB_LIMIT: usize = 12;
const TRASH_SAVE_FAILURE_MESSAGE: &str =
    "the note could not be saved, so it was not moved to Trash";
const TRASH_SAVE_STALE_MESSAGE: &str =
    "the note changed before its Trash save completed, so it was not moved to Trash";

#[derive(Clone, Copy, Debug)]
struct PendingExternalSaveAs {
    external_file_id: notora_core::ExternalFileId,
    content_revision: u64,
}

#[derive(Clone, Copy, Debug)]
struct PendingConflictRetry {
    identity: DocumentIdentity,
    content_revision: u64,
}

#[derive(Clone, Copy, Debug)]
struct PendingTrashMove {
    note_id: notora_core::NoteId,
    content_revision: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum SettingsPersistenceState {
    #[default]
    Saved,
    SaveFailed {
        message: String,
    },
}

impl SettingsPersistenceState {
    fn to_view(&self) -> crate::settings_overlay::NotoraSettingsPersistenceView {
        match self {
            Self::Saved => crate::settings_overlay::NotoraSettingsPersistenceView::Saved,
            Self::SaveFailed { message } => {
                crate::settings_overlay::NotoraSettingsPersistenceView::SaveFailed {
                    message: message.clone(),
                }
            }
        }
    }
}

/// notora 应用初始化失败。
#[derive(Debug)]
pub enum NotoraAppError {
    Paths(NotoraPathsError),
    Runtime(EditorRuntimeError),
    PersistenceWorker(std::io::Error),
}

impl std::fmt::Display for NotoraAppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Paths(error) => write!(formatter, "could not initialize notora paths: {error}"),
            Self::Runtime(error) => {
                write!(formatter, "could not initialize editor runtime: {error}")
            }
            Self::PersistenceWorker(error) => {
                write!(formatter, "could not initialize persistence worker: {error}")
            }
        }
    }
}

impl std::error::Error for NotoraAppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Paths(error) => Some(error),
            Self::Runtime(error) => Some(error),
            Self::PersistenceWorker(error) => Some(error),
        }
    }
}

/// 组合产品状态、后台宿主和共享 editor runtime 的 notora 应用。
pub struct NotoraApp {
    paths: NotoraPaths,
    product_settings: crate::settings::ProductSettings,
    pending_session: Option<crate::session::ProductSession>,
    pending_session_persist_at: Option<Instant>,
    pending_catalog_backup_at: Option<Instant>,
    runtime_lru: RuntimeLru,
    settings: ui::Settings,
    settings_persistence: SettingsPersistenceState,
    theme: ui::Theme,
    state: NotoraState,
    product: NotoraProduct,
    persistence_worker: crate::persistence_worker::PersistenceWorker,
    workspace_controller: WorkspaceController,
    document_registry: DocumentRegistry,
    autosave: AutoSaveScheduler,
    pending_external_save_as: HashMap<appkit_core::workspace::types::TabId, PendingExternalSaveAs>,
    pending_external_documents: HashMap<notora_core::ExternalFileId, LoadedDocument>,
    pending_conflict_retries: HashMap<appkit_core::workspace::types::TabId, PendingConflictRetry>,
    pending_trash_moves: HashMap<appkit_core::workspace::types::TabId, PendingTrashMove>,
    catalog_reconciliation_pending: bool,
    search_controller: SearchController,
    editor_runtime: EditorRuntime,
    shell: NotoraShell,
    window_focused: bool,
    window_width_px: f32,
    window_height_px: f32,
    pointer_position: (f32, f32),
    needs_redraw: bool,
    event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
}

impl NotoraApp {
    pub fn new() -> Self {
        Self::try_new()
            .expect("notora must construct its isolated configuration and editor runtime")
    }

    pub fn try_new() -> Result<Self, NotoraAppError> {
        let paths = NotoraPaths::from_platform_directory().map_err(NotoraAppError::Paths)?;
        Self::with_paths(paths)
    }

    pub fn with_paths(paths: NotoraPaths) -> Result<Self, NotoraAppError> {
        let loaded_product_settings = crate::settings::load_product_settings(&paths.settings_file);
        let loaded_session = crate::session::load_product_session(&paths.session_file);
        let product_settings = loaded_product_settings.settings;
        let mut settings = ui::Settings::new();
        product_settings.apply_to_ui(&mut settings);
        let theme = ui::Theme::resolve_builtin(
            product_settings.appearance.theme_mode,
            winit::window::Theme::Dark,
        );
        let editor_runtime = build_editor_runtime(&settings, &theme, &paths)?;
        let mut state = NotoraState::default();
        state.layout.navigation_width_logical = loaded_session.session.navigation_width_logical;
        state.layout.card_list_width_logical = loaded_session.session.card_list_width_logical;
        state.library.last_command_error =
            loaded_product_settings.diagnostic.or(loaded_session.diagnostic);
        let runtime_tab_limit = NonZeroUsize::new(product_settings.interface.runtime_tab_limit)
            .or_else(|| NonZeroUsize::new(DEFAULT_RUNTIME_TAB_LIMIT))
            .expect("default runtime tab limit must be non-zero");
        let auto_save_delay =
            Duration::from_millis(product_settings.workspace.auto_save_delay_millis);
        let migration_backup_retention = notora_core::BackupRetention::keep_latest(
            product_settings.workspace.catalog_backup_retention,
        )
        .unwrap_or_else(|| {
            notora_core::BackupRetention::keep_latest(1)
                .expect("a minimum migration backup retention must be non-zero")
        });
        let catalog_backups_directory = paths.catalog_backups_directory.clone();
        let product = NotoraProduct::new();
        let persistence_worker =
            crate::persistence_worker::PersistenceWorker::start(product.event_sender())
                .map_err(NotoraAppError::PersistenceWorker)?;
        Ok(Self {
            paths,
            product_settings,
            pending_session: Some(loaded_session.session),
            pending_session_persist_at: None,
            pending_catalog_backup_at: None,
            runtime_lru: RuntimeLru::new(runtime_tab_limit),
            settings,
            settings_persistence: SettingsPersistenceState::Saved,
            theme,
            state,
            product,
            persistence_worker,
            workspace_controller: WorkspaceController::with_catalog_backups_directory_and_retention(
                catalog_backups_directory,
                migration_backup_retention,
            ),
            document_registry: DocumentRegistry::default(),
            autosave: AutoSaveScheduler::with_clock_and_idle_delay(
                crate::autosave::SystemAutoSaveClock,
                auto_save_delay,
            ),
            pending_external_save_as: HashMap::new(),
            pending_external_documents: HashMap::new(),
            pending_conflict_retries: HashMap::new(),
            pending_trash_moves: HashMap::new(),
            catalog_reconciliation_pending: false,
            search_controller: SearchController::default(),
            editor_runtime,
            shell: NotoraShell::new(),
            window_focused: true,
            window_width_px: DEFAULT_WINDOW_WIDTH_PX,
            window_height_px: DEFAULT_WINDOW_HEIGHT_PX,
            pointer_position: (0.0, 0.0),
            needs_redraw: true,
            event_loop_proxy: None,
        })
    }

    pub fn editor_runtime_tab_count(&self) -> usize {
        self.editor_runtime.tab_count()
    }

    pub fn paths(&self) -> &NotoraPaths {
        &self.paths
    }

    /// 返回可供 UI 明确确认的恢复候选；此查询绝不修改任何源文件。
    pub fn recoverable_dirty_snapshots(
        &self,
    ) -> std::io::Result<Vec<crate::dirty_snapshot::RecoverableDirtySnapshot>> {
        crate::dirty_snapshot::list_recoverable_snapshots(&self.paths.snapshots_directory)
    }

    pub fn state(&self) -> &NotoraState {
        &self.state
    }

    pub fn document_tab_for(
        &self,
        identity: DocumentIdentity,
    ) -> Option<appkit_core::workspace::types::TabId> {
        self.document_registry.tab_for(identity)
    }

    pub fn request_preview_promotion(&mut self) {
        self.dispatch_action(NotoraAction::PromotePreviewRequested);
    }

    /// 系统 open event 与拖入路径的统一产品入口。
    pub fn receive_system_open_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        self.dispatch_action(NotoraAction::ExternalPathsReceived(paths));
    }

    pub fn request_external_file_dialog(&mut self) {
        self.dispatch_action(NotoraAction::OpenExternalFileDialogRequested);
    }

    pub(crate) fn request_manual_save(&mut self) {
        let Some(tab_id) = self.editor_runtime.active_tab_id() else {
            return;
        };
        let Some(request) = self.manual_save_request_for_tab(tab_id) else {
            return;
        };
        EffectExecutor::save_document_manually(self, request);
    }

    fn manual_save_request_for_tab(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<ManualSaveRequest> {
        let summary = self.editor_runtime.document_summary(tab_id)?;
        let origin = self.document_origin_for_tab(tab_id)?;
        Some(match origin {
            notora_core::DocumentOrigin::Note { .. } => {
                ManualSaveRequest::Note { tab_id, content_revision: summary.content_revision }
            }
            notora_core::DocumentOrigin::ExternalFile { .. } => {
                ManualSaveRequest::ExistingExternalFile { tab_id }
            }
            notora_core::DocumentOrigin::UntitledExternal { external_file_id, .. } => {
                ManualSaveRequest::UntitledExternalFile { tab_id, external_file_id }
            }
        })
    }

    pub fn execute_workspace_command(
        &mut self,
        command: WorkspaceCommand,
    ) -> Result<WorkspaceCommandResult, WorkspaceControllerError> {
        let result = self.workspace_controller.execute(command, &mut self.product)?;
        match &result {
            WorkspaceCommandResult::Opened(workspace) => self
                .search_controller
                .set_active_workspace(workspace.descriptor.workspace_id, workspace.generation),
            WorkspaceCommandResult::Closed { .. } => {
                self.search_controller.clear_active_workspace()
            }
            WorkspaceCommandResult::Unchanged => {}
        }
        if !matches!(result, WorkspaceCommandResult::Unchanged) {
            self.autosave.clear();
            self.pending_external_save_as.clear();
            self.pending_conflict_retries.clear();
            self.pending_trash_moves.clear();
            self.catalog_reconciliation_pending = false;
        }
        if matches!(result, WorkspaceCommandResult::Opened(_)) {
            self.request_navigation_tree();
        }
        Ok(result)
    }

    pub fn shell_layout(&self) -> ShellLayout {
        let dpi = self.editor_runtime.scale_factor() as f32;
        ShellLayout::compute(ShellLayoutInput {
            window_width_px: self.window_width_px,
            window_height_px: self.window_height_px,
            dpi,
            navigation_width_logical: self.state.layout.navigation_width_logical,
            card_list_width_logical: self.state.layout.card_list_width_logical,
            compact_content: self.state.layout.compact_content,
            compact_navigation: self.state.layout.compact_navigation,
        })
    }

    pub fn dispatch_action(&mut self, action: NotoraAction) {
        let should_persist_session = action_requires_session_persistence(&action);
        let committed_without_workspace = match &action {
            NotoraAction::SearchTextChanged(query) => {
                !self.search_controller.schedule_committed_query(query.clone(), Instant::now())
            }
            _ => false,
        };
        for effect in self.state.reduce(action) {
            let shell_effect = EffectExecutor::execute(self, effect);
            self.apply_shell_effect(shell_effect);
        }
        if committed_without_workspace {
            self.dispatch_action(NotoraAction::SearchCommitted {
                query: self.state.library.search_text.clone(),
                search_generation: None,
            });
        }
        if should_persist_session {
            self.schedule_session_persistence();
        }
    }

    pub fn update_editor_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) -> bool {
        let context =
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused);
        self.editor_runtime.update_preedit(context, text, cursor)
    }

    pub(crate) fn commit_editor_text(&mut self, text: String) {
        let context =
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused);
        let outcome = self.editor_runtime.commit_text(context, text);
        self.apply_editor_outcome(outcome);
    }

    pub(crate) fn handle_editor_key_input(
        &mut self,
        key: ui::KeyCode,
        modifiers: ui::core::Modifiers,
    ) {
        let context =
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused);
        let outcome = self.editor_runtime.handle_key_input(context, key, modifiers);
        self.apply_editor_outcome(outcome);
    }

    pub(crate) fn scroll_editor(&mut self, px: f32, py: f32, pixels: f32) {
        let context =
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused);
        let outcome = self.editor_runtime.scroll_editor(context, (px, py), pixels);
        self.apply_editor_outcome(outcome);
    }

    pub fn set_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<ShellEvent>) {
        self.event_loop_proxy = Some(event_loop_proxy);
    }

    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        self.window_focused = focused;
        self.editor_runtime.set_window_focus(focused);
    }

    pub(crate) fn set_window_size(&mut self, width: u32, height: u32) {
        self.window_width_px = width as f32;
        self.window_height_px = height as f32;
        self.state.layout.responsive_mode = self.shell_layout().responsive_mode;
    }

    pub(crate) fn editor_runtime_mut(&mut self) -> &mut EditorRuntime {
        &mut self.editor_runtime
    }

    pub(crate) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw) || self.editor_runtime.take_redraw_request()
    }

    pub(crate) fn process_due_autosaves(&mut self) {
        for request in self.autosave.take_due_saves() {
            self.submit_autosave(request);
        }
    }

    pub(crate) fn process_due_searches(&mut self) {
        let Some(request) = self.search_controller.take_due_request(Instant::now()) else {
            return;
        };
        self.dispatch_action(NotoraAction::SearchCommitted {
            query: request.query,
            search_generation: Some(request.search_generation),
        });
    }

    pub(crate) fn process_due_session_persistence(&mut self) {
        let Some(deadline) = self.pending_session_persist_at else {
            return;
        };
        if deadline > Instant::now() {
            return;
        }
        self.pending_session_persist_at = None;
        if let Err(error) = self
            .persistence_worker
            .save_session(self.paths.session_file.clone(), self.capture_product_session())
        {
            self.state.library.last_command_error = Some(error.to_string());
        }
    }

    pub(crate) fn process_due_catalog_backups(&mut self) {
        let Some(deadline) = self.pending_catalog_backup_at else {
            return;
        };
        if deadline > Instant::now() {
            return;
        }
        self.pending_catalog_backup_at = None;
        self.start_catalog_backup();
    }

    pub(crate) fn next_deadline(&self) -> Option<std::time::Instant> {
        [
            self.autosave.next_deadline(),
            self.search_controller.next_deadline(),
            self.pending_session_persist_at,
            self.pending_catalog_backup_at,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub(crate) fn drain_runtime_save_completions(&mut self) {
        for completion in self.editor_runtime.drain_save_completions() {
            let request = AutoSaveRequest {
                tab_id: completion.tab_id,
                content_revision: completion.content_revision,
            };
            let concurrent_modification = matches!(
                &completion.result,
                Err(appkit_core::document::DocumentSaveError::ConcurrentModification)
            );
            let conflict_identity = concurrent_modification
                .then(|| self.document_registry.identity_for(request.tab_id))
                .flatten();
            let save_succeeded = completion.result.is_ok();
            let completed_conflict_retry = self
                .pending_conflict_retries
                .get(&request.tab_id)
                .copied()
                .filter(|retry| retry.content_revision == request.content_revision);
            let pending_trash_move = self
                .pending_trash_moves
                .get(&request.tab_id)
                .copied()
                .filter(|pending| pending.content_revision == request.content_revision);
            if completed_conflict_retry.is_some() {
                self.pending_conflict_retries.remove(&request.tab_id);
            }
            let saved_path = completion.result.as_ref().ok().map(|revision| revision.path.clone());
            let outcome = self.editor_runtime.apply_save_completion(completion);
            self.apply_editor_outcome(outcome);
            self.complete_pending_external_save_as(request, save_succeeded, saved_path);
            if save_succeeded {
                self.autosave.on_save_completed(request);
                self.request_catalog_reindex_after_note_save(request.tab_id);
                if let Some(pending_trash_move) = pending_trash_move {
                    if self.pending_trash_move_has_current_saved_document(
                        request.tab_id,
                        pending_trash_move,
                    ) {
                        self.pending_trash_moves.remove(&request.tab_id);
                        self.execute_trash_operation(crate::action::TrashOperation::MoveToTrash {
                            note_id: pending_trash_move.note_id,
                        });
                    } else {
                        self.cancel_pending_trash_move(request, TRASH_SAVE_STALE_MESSAGE);
                    }
                }
                if let Some(retry) = completed_conflict_retry {
                    self.dispatch_action(NotoraAction::SaveConflictResolved {
                        identity: retry.identity,
                    });
                }
            } else {
                self.autosave.on_save_failed(request);
                self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
                if let Some(identity) = conflict_identity {
                    self.dispatch_action(NotoraAction::SaveConflictDetected {
                        identity,
                        content_revision: request.content_revision,
                    });
                }
            }
        }
    }

    pub(crate) fn request_window_redraw(&self) {
        if let Some(window) = self.editor_runtime.window() {
            window.request_redraw();
        }
    }

    pub(crate) fn runtime_accepts_pointer_input(&self, px: f32, py: f32) -> bool {
        self.editor_runtime.pointer_input_allowed(
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused),
            (px, py),
        )
    }

    pub(crate) fn begin_editor_text_selection(&mut self) -> bool {
        self.editor_runtime.begin_text_selection(events::editor_input_context(
            &self.state,
            self.shell_layout(),
            self.window_focused,
        ))
    }

    pub(crate) fn end_editor_pointer_capture(&mut self) {
        self.editor_runtime.end_pointer_capture();
    }

    pub(crate) fn set_pointer_position(&mut self, px: f32, py: f32) {
        self.pointer_position = (px, py);
    }

    pub(crate) fn pointer_position(&self) -> (f32, f32) {
        self.pointer_position
    }

    pub(crate) fn set_scale_factor(&mut self, scale_factor: f64) {
        self.editor_runtime.set_scale_factor(scale_factor);
        self.state.layout.responsive_mode = self.shell_layout().responsive_mode;
        self.needs_redraw = true;
    }

    fn current_system_appearance(&self) -> winit::window::Theme {
        self.editor_runtime
            .window()
            .and_then(|window| window.theme())
            .unwrap_or(winit::window::Theme::Dark)
    }

    pub(crate) fn follows_system_theme(&self) -> bool {
        self.product_settings.appearance.theme_mode == ui::ThemeMode::System
    }

    pub(crate) fn rebuild_theme_for_system_appearance(
        &mut self,
        system_appearance: winit::window::Theme,
    ) {
        self.theme = ui::Theme::resolve_builtin(
            self.product_settings.appearance.theme_mode,
            system_appearance,
        );
        self.editor_runtime.update_theme(self.theme.clone());
        self.needs_redraw = true;
    }

    pub(crate) fn resume(&mut self, event_loop: &ActiveEventLoop) -> Result<(), NotoraAppError> {
        if self.editor_runtime.window().is_some() {
            return Ok(());
        }
        let font_system = Arc::new(Mutex::new(shaping::font_cache::new_font_system_with_cache(
            &self.paths.config_directory.join("font-cache.bin"),
        )));
        self.editor_runtime.set_shared_font_system(Arc::clone(&font_system));
        self.editor_runtime
            .resume(
                event_loop,
                WindowAttributes::default().with_title("notora"),
                font_system,
                self.settings.font_size,
                &self.settings.font_family,
            )
            .map_err(NotoraAppError::Runtime)?;
        self.rebuild_theme_for_system_appearance(self.current_system_appearance());
        if let Some((width, height)) = self.editor_runtime.window().map(|window| {
            let geometry = self
                .pending_session
                .as_ref()
                .map(|session| session.window_geometry.clone())
                .unwrap_or_default();
            window.set_outer_position(PhysicalPosition::new(
                geometry.x_px.round() as i32,
                geometry.y_px.round() as i32,
            ));
            let _ = window.request_inner_size(PhysicalSize::new(
                geometry.width_px.round().max(1.0) as u32,
                geometry.height_px.round().max(1.0) as u32,
            ));
            let size = window.inner_size();
            window.set_ime_allowed(true);
            (size.width, size.height)
        }) {
            self.set_window_size(width, height);
        }
        if let Some(event_loop_proxy) = self.event_loop_proxy.clone() {
            ProductHost::start_background_services(
                &mut self.product,
                ProductWakeHandle::new(event_loop_proxy),
            );
        }
        self.needs_redraw = true;
        Ok(())
    }

    pub(crate) fn resize_window(&mut self, width: u32, height: u32) {
        self.set_window_size(width, height);
        let _ = self.editor_runtime.resize_now(width, height);
        self.needs_redraw = true;
    }

    /// 处理后台产品结果；无事件循环的宿主可主动轮询此入口。
    pub fn drain_product_events(&mut self) {
        let effect = ProductHost::drain_product_events(&mut self.product);
        for event in self.product.take_workspace_events() {
            match event {
                crate::product::NotoraProductEvent::NoteCommandCompleted { result, .. } => {
                    self.synchronize_open_note_path(&result);
                    self.dispatch_action(NotoraAction::NoteCommandCompleted(result));
                }
                crate::product::NotoraProductEvent::NoteCommandFailed { message, .. } => {
                    self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                }
                crate::product::NotoraProductEvent::MetadataMutationCompleted { .. } => {
                    self.schedule_catalog_backup();
                    self.request_navigation_tree();
                    self.dispatch_action(NotoraAction::MetadataMutationCompleted);
                }
                crate::product::NotoraProductEvent::MetadataMutationFailed { message, .. } => {
                    self.dispatch_action(NotoraAction::MetadataMutationFailed(message));
                }
                crate::product::NotoraProductEvent::CatalogBackupCompleted { .. } => {}
                crate::product::NotoraProductEvent::CatalogBackupFailed { message, .. } => {
                    self.dispatch_action(NotoraAction::MetadataMutationFailed(format!(
                        "metadata was saved, but its catalog backup failed: {message}"
                    )));
                }
                crate::product::NotoraProductEvent::CatalogRecoveryNotified { message, .. } => {
                    self.dispatch_action(NotoraAction::CatalogRecoveryNotified(message));
                }
                crate::product::NotoraProductEvent::TrashOperationCompleted {
                    operation, ..
                } => {
                    self.complete_trash_operation(operation);
                    self.schedule_catalog_backup();
                    self.request_navigation_tree();
                    self.dispatch_action(NotoraAction::TrashOperationCompleted);
                }
                crate::product::NotoraProductEvent::TrashOperationFailed { failure, .. } => {
                    self.dispatch_action(NotoraAction::TrashOperationFailed(failure));
                }
                crate::product::NotoraProductEvent::DocumentLoaded {
                    request, document, ..
                } => self.install_loaded_preview(request, document),
                crate::product::NotoraProductEvent::DocumentLoadFailed {
                    request, message, ..
                } if self.selection_matches(request) => {
                    self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                }
                crate::product::NotoraProductEvent::DocumentLoadFailed { .. } => {}
                crate::product::NotoraProductEvent::ConflictCopyCompleted {
                    identity,
                    result,
                    ..
                } => match result {
                    Ok(()) => self.dispatch_action(NotoraAction::SaveConflictResolved { identity }),
                    Err(message) => self.dispatch_action(NotoraAction::NoteCommandFailed(message)),
                },
                crate::product::NotoraProductEvent::ExternalFileOpenCompleted {
                    canonical_path,
                    document,
                    activate,
                } => self.complete_external_file_open(canonical_path, document, activate),
                crate::product::NotoraProductEvent::ExternalFileOpenFailed { message } => {
                    self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                }
                crate::product::NotoraProductEvent::ExternalDocumentLoaded {
                    request,
                    document,
                } => self.install_loaded_preview(request, document),
                crate::product::NotoraProductEvent::ExternalDocumentLoadFailed {
                    request,
                    message,
                } if self.selection_matches(request) => {
                    self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                }
                crate::product::NotoraProductEvent::ExternalDocumentLoadFailed { .. } => {}
                crate::product::NotoraProductEvent::ExternalSaveAsCanonicalized {
                    tab_id,
                    external_file_id,
                    content_revision,
                    result,
                } => self.complete_external_save_as_canonicalization(
                    tab_id,
                    external_file_id,
                    content_revision,
                    result,
                ),
                crate::product::NotoraProductEvent::ConflictReloadCompleted {
                    identity,
                    tab_id,
                    content_revision,
                    document,
                } => self.complete_conflict_reload(identity, tab_id, content_revision, document),
                crate::product::NotoraProductEvent::ConflictReloadFailed { identity, message } => {
                    if self
                        .state
                        .library
                        .save_conflict
                        .is_some_and(|conflict| conflict.identity == identity)
                    {
                        self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                    }
                }
                crate::product::NotoraProductEvent::ConflictRetryRevisionCaptured {
                    identity,
                    tab_id,
                    content_revision,
                    path,
                    disk_revision,
                } => self.complete_conflict_retry_revision_capture(
                    identity,
                    tab_id,
                    content_revision,
                    path,
                    disk_revision,
                ),
                crate::product::NotoraProductEvent::ConflictRetryRevisionFailed {
                    identity,
                    message,
                } => {
                    if self
                        .state
                        .library
                        .save_conflict
                        .is_some_and(|conflict| conflict.identity == identity)
                    {
                        self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                    }
                }
                crate::product::NotoraProductEvent::SettingsPersistenceCompleted { result } => {
                    self.record_settings_persistence_result(result);
                }
                crate::product::NotoraProductEvent::SessionPersistenceFailed { message } => {
                    self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                }
                crate::product::NotoraProductEvent::WorkspaceScanCompleted { .. } => {
                    self.catalog_reconciliation_pending = false;
                }
                crate::product::NotoraProductEvent::WorkspaceIndexFailed { message, .. } => {
                    self.catalog_reconciliation_pending = true;
                    self.dispatch_action(NotoraAction::NavigationTreeFailed(message));
                }
                crate::product::NotoraProductEvent::CardQueryCompleted { query, page, .. }
                    if query.search_generation.is_none_or(|generation| {
                        self.search_controller.accepts_generation(generation)
                    }) =>
                {
                    self.dispatch_action(NotoraAction::CardQueryCompleted { query, page });
                }
                crate::product::NotoraProductEvent::CardQueryCompleted { .. } => {}
                crate::product::NotoraProductEvent::CardQueryFailed { query, message, .. }
                    if query.search_generation.is_none_or(|generation| {
                        self.search_controller.accepts_generation(generation)
                    }) =>
                {
                    self.dispatch_action(NotoraAction::CardQueryFailed { query, message });
                }
                crate::product::NotoraProductEvent::CardQueryFailed { .. } => {}
                crate::product::NotoraProductEvent::NavigationTreeLoaded { tree, .. } => {
                    self.dispatch_action(NotoraAction::NavigationTreeLoaded(tree));
                }
                crate::product::NotoraProductEvent::NavigationTreeFailed { message, .. } => {
                    self.dispatch_action(NotoraAction::NavigationTreeFailed(message));
                }
                crate::product::NotoraProductEvent::WorkspaceChanged { .. } => {}
            }
        }
        self.apply_shell_effect(effect);
    }

    pub(crate) fn route_product_event(&mut self, event: &ui::Event) -> bool {
        let focus_target = self.state.layout.focus_target;
        let settings_modal_is_open =
            self.state.layout.overlay == crate::state::OverlayState::Settings;
        let actions = self.shell.route_event(
            event,
            focus_target,
            &self.theme,
            self.editor_runtime.scale_factor() as f32,
        );
        if actions.is_empty() {
            return settings_modal_is_open;
        }
        for action in actions {
            self.dispatch_action(action);
        }
        true
    }

    pub(crate) fn render(&mut self) -> Result<(), RenderError> {
        self.needs_redraw = false;
        let layout = self.shell_layout();
        let mut model =
            NotoraRenderModel::from_state_and_settings(&self.state, &self.product_settings);
        model.settings_overlay.persistence = self.settings_persistence.to_view();
        let mut render_resources = self.editor_runtime.take_render_resources();
        let mut frame = self.editor_runtime.begin_frame()?;
        self.shell.render(&mut frame, layout, &model)?;
        let mut vertices = Vec::new();
        frame.drain_into(
            ui::Screen::new(self.window_width_px, self.window_height_px),
            &mut render_resources,
            &mut vertices,
        );
        submit_shell_frame(
            &mut render_resources,
            &vertices,
            self.theme.application_theme().editor_surface,
        );
        let _ = frame.present()?;
        self.editor_runtime.restore_render_resources(render_resources);
        self.editor_runtime.mark_frame_presented();
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) {
        self.finish_saves_and_snapshot_dirty_documents();
        self.flush_pending_catalog_backup();
        if let Err(error) = self
            .persistence_worker
            .save_session(self.paths.session_file.clone(), self.capture_product_session())
        {
            self.state.library.last_command_error = Some(error.to_string());
        }
        if let Err(error) = self
            .persistence_worker
            .save_settings(self.paths.settings_file.clone(), self.product_settings.clone())
        {
            self.state.library.last_command_error = Some(error.to_string());
        }
        self.persistence_worker.shutdown();
        ProductHost::shutdown(&mut self.product);
        self.editor_runtime.shutdown();
    }

    fn finish_saves_and_snapshot_dirty_documents(&mut self) {
        self.process_due_autosaves();
        let deadline = Instant::now() + SHUTDOWN_SAVE_DRAIN_TIMEOUT;
        while self.autosave.has_in_flight_save() && Instant::now() < deadline {
            self.drain_runtime_save_completions();
            if self.autosave.has_in_flight_save() {
                thread::sleep(SHUTDOWN_SAVE_DRAIN_POLL_INTERVAL);
            }
        }
        self.drain_runtime_save_completions();
        self.write_dirty_snapshots_in_background();
    }

    fn write_dirty_snapshots_in_background(&self) {
        let plans = collect_dirty_snapshots(&self.editor_runtime.workspace_snapshot());
        if plans.is_empty() {
            return;
        }
        let snapshots_directory = self.paths.snapshots_directory.clone();
        let (completed_sender, completed_receiver) = mpsc::channel();
        let writer_started = thread::Builder::new()
            .name("notora-dirty-snapshot".to_owned())
            .spawn(move || {
                for plan in plans {
                    let _ = write_dirty_snapshot(&snapshots_directory, &plan);
                }
                let _ = completed_sender.send(());
            })
            .is_ok();
        if writer_started {
            let _ = completed_receiver.recv_timeout(SHUTDOWN_SAVE_DRAIN_TIMEOUT);
        }
    }

    fn apply_shell_effect(&mut self, effect: ShellEffect) {
        if effect.redraw {
            self.needs_redraw = true;
            self.editor_runtime.request_redraw();
        }
    }

    fn install_loaded_preview(&mut self, request: DocumentLoadRequest, document: LoadedDocument) {
        if !self.selection_matches(request) {
            return;
        }
        let identity = request.identity;
        if let Some(tab_id) = self.document_registry.tab_for(identity) {
            self.document_registry.touch_tab(tab_id);
            let outcome = self.editor_runtime.activate(tab_id);
            self.apply_editor_outcome(outcome);
            return;
        }
        let prepared = match prepare_loaded_document(&self.editor_runtime, document) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
                return;
            }
        };
        self.install_prepared_preview(request, prepared, None);
    }

    fn install_prepared_preview(
        &mut self,
        request: DocumentLoadRequest,
        prepared: appkit_shell::prepared_tab::PreparedTab,
        suggested_file_name: Option<String>,
    ) {
        if !self.selection_matches(request) {
            return;
        }
        let identity = request.identity;
        if let Some(tab_id) = self.document_registry.tab_for(identity) {
            self.document_registry.touch_tab(tab_id);
            let outcome = self.editor_runtime.activate(tab_id);
            self.apply_editor_outcome(outcome);
            return;
        }
        let replaced_preview = self.document_registry.preview_tab();
        let outcome = self.editor_runtime.install_prepared_tab(
            prepared,
            suggested_file_name,
            OpenDisposition::Preview,
        );
        let Some(tab_id) = self.editor_runtime.active_tab_id() else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "editor runtime did not activate the installed preview".to_owned(),
            ));
            return;
        };
        if let Some(replaced_preview) = replaced_preview {
            self.document_registry.remove_tab(replaced_preview);
        }
        let _ = self.document_registry.register_preview(identity, tab_id);
        self.apply_editor_outcome(outcome);
        self.evict_excess_runtime_tabs();
    }

    fn promote_active_preview_tab(&mut self) {
        let Some(tab_id) = self.editor_runtime.active_tab_id() else {
            return;
        };
        self.promote_preview_for_tab(tab_id);
    }

    fn evict_excess_runtime_tabs(&mut self) {
        let active_tab_id = self.editor_runtime.active_tab_id();
        let runtime_tabs = self
            .editor_runtime
            .tab_ids_in_order()
            .into_iter()
            .filter_map(|tab_id| {
                let summary = self.editor_runtime.document_summary(tab_id)?;
                Some(RuntimeTabState {
                    tab_id,
                    is_dirty: summary.dirty,
                    is_saving: matches!(
                        self.autosave.state(tab_id),
                        Some(crate::autosave::AutoSaveState::Saving { .. })
                    ),
                    is_pinned: self.editor_runtime.is_pinned(tab_id),
                    is_active: active_tab_id == Some(tab_id),
                })
            })
            .collect::<Vec<_>>();
        for candidate in self.runtime_lru.select_evictions(&self.document_registry, &runtime_tabs) {
            self.autosave.cancel(candidate.tab_id);
            let _ = self.editor_runtime.close_for_product(candidate.tab_id);
            self.document_registry.remove_tab(candidate.tab_id);
        }
    }

    fn synchronize_open_note_path(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    ) {
        let Some(previous_relative_path) = result.previous_relative_path.as_deref() else {
            return;
        };
        let identity = DocumentIdentity::Note(result.note.note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return;
        };
        let Some(workspace) = self.workspace_controller.active_workspace() else {
            return;
        };
        let previous_path = workspace.descriptor.root.join(previous_relative_path);
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return;
        };
        if summary.path.as_deref() != Some(previous_path.as_path()) {
            return;
        }
        let next_path = workspace.descriptor.root.join(&result.note.relative_path);
        if self.editor_runtime.update_document_path(tab_id, next_path, None) {
            self.editor_runtime.request_file_safety_check_now(Instant::now());
            self.needs_redraw = true;
            self.editor_runtime.request_redraw();
        }
    }

    fn promote_preview_for_tab(&mut self, tab_id: appkit_core::workspace::types::TabId) {
        if self.editor_runtime.active_tab_id() != Some(tab_id) {
            return;
        }
        if self.editor_runtime.upgrade_active_preview() == appkit_core::navigator::NavEffect::None {
            return;
        }
        if self.document_registry.upgrade_preview(tab_id) {
            self.needs_redraw = true;
            self.editor_runtime.request_redraw();
        }
    }

    fn apply_editor_outcome(&mut self, outcome: EditorOutcome) {
        for notification in &outcome.notifications {
            self.handle_editor_notification(notification);
        }
        self.apply_shell_effect(outcome.shell_effect);
    }

    fn selection_matches(&self, request: DocumentLoadRequest) -> bool {
        self.state.library.selected_card == Some(request.identity)
            && self.state.library.selected_document_generation == request.selection_generation
    }

    pub(crate) fn handle_editor_notification(&mut self, notification: &EditorNotification) {
        match notification {
            EditorNotification::ActiveDocumentChanged { tab_id: Some(tab_id) } => {
                self.document_registry.touch_tab(*tab_id);
            }
            EditorNotification::ContentChanged { tab_id, content_revision } => {
                self.promote_preview_for_tab(*tab_id);
                if let Some(origin) = self.document_origin_for_tab(*tab_id) {
                    self.autosave.on_content_changed(&origin, *tab_id, *content_revision);
                }
            }
            EditorNotification::ActiveDocumentChanged { tab_id: None }
            | EditorNotification::PathChanged { .. }
            | EditorNotification::DirtyChanged { .. }
            | EditorNotification::SaveCompleted { .. }
            | EditorNotification::SaveFailed { .. }
            | EditorNotification::CloseRequested { .. } => {}
        }
    }

    fn submit_autosave(&mut self, request: AutoSaveRequest) {
        let Some(summary) = self.editor_runtime.document_summary(request.tab_id) else {
            self.autosave.cancel(request.tab_id);
            self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
            return;
        };
        if !summary.dirty || summary.content_revision != request.content_revision {
            self.autosave.on_save_failed(request);
            self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
            return;
        }
        let prepared = match self.editor_runtime.prepare_save(request.tab_id) {
            Ok(prepared) => prepared,
            Err(_) => {
                self.autosave.on_save_failed(request);
                self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
                return;
            }
        };
        if self.submit_prepared_save(prepared).is_err() {
            self.autosave.on_save_failed(request);
            self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
        }
    }

    fn pending_trash_move_has_current_saved_document(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
        pending_trash_move: PendingTrashMove,
    ) -> bool {
        self.editor_runtime.document_summary(tab_id).is_some_and(|summary| {
            !summary.dirty && summary.content_revision == pending_trash_move.content_revision
        })
    }

    fn cancel_pending_trash_move(&mut self, request: AutoSaveRequest, message: &str) {
        let is_matching_pending_move = self
            .pending_trash_moves
            .get(&request.tab_id)
            .is_some_and(|pending| pending.content_revision == request.content_revision);
        if !is_matching_pending_move {
            return;
        }
        self.pending_trash_moves.remove(&request.tab_id);
        self.dispatch_action(NotoraAction::TrashOperationFailed(
            crate::action::TrashOperationFailure::Message(message.to_owned()),
        ));
    }

    fn submit_prepared_save(
        &mut self,
        prepared: appkit_shell::editor_runtime::PreparedDocumentSave,
    ) -> Result<(), String> {
        let event_loop_proxy = self
            .event_loop_proxy
            .clone()
            .ok_or_else(|| "save worker is unavailable before the event loop starts".to_owned())?;
        self.editor_runtime.submit_save(prepared, move || {
            let _ = event_loop_proxy.send_event(ShellEvent::SaveResultsReady);
        })
    }

    fn submit_manual_external_save(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> bool {
        let prepared = match self.editor_runtime.prepare_save(tab_id) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
                return false;
            }
        };
        if let Err(error) = self.submit_prepared_save(prepared) {
            self.dispatch_action(NotoraAction::NoteCommandFailed(error));
            return false;
        }
        true
    }

    fn save_untitled_external_file(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
    ) {
        let Some(path) =
            rfd::FileDialog::new().add_filter("Text documents", &["txt", "md"]).save_file()
        else {
            return;
        };
        self.save_external_file_as_to_path(tab_id, external_file_id, path);
    }

    fn save_external_file_as_to_path(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        path: std::path::PathBuf,
    ) {
        let prepared = match self.editor_runtime.prepare_save_as(tab_id, &path) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
                return;
            }
        };
        let pending_save =
            PendingExternalSaveAs { external_file_id, content_revision: prepared.content_revision };
        if let Err(error) = self.submit_prepared_save(prepared) {
            self.dispatch_action(NotoraAction::NoteCommandFailed(error));
            return;
        }
        self.pending_external_save_as.insert(tab_id, pending_save);
    }

    fn complete_pending_external_save_as(
        &mut self,
        request: AutoSaveRequest,
        save_succeeded: bool,
        saved_path: Option<std::path::PathBuf>,
    ) {
        let Some(pending_save) = self.pending_external_save_as.get(&request.tab_id).copied() else {
            return;
        };
        if pending_save.content_revision != request.content_revision {
            return;
        }
        if !save_succeeded {
            self.pending_external_save_as.remove(&request.tab_id);
            return;
        }
        let Some(saved_path) = saved_path else {
            self.pending_external_save_as.remove(&request.tab_id);
            return;
        };
        let sender = self.product.event_sender();
        let external_file_id = pending_save.external_file_id;
        let content_revision = pending_save.content_revision;
        if thread::Builder::new()
            .name("notora-external-save-as-canonicalize".to_owned())
            .spawn(move || {
                let result = CanonicalExternalPath::canonicalize(&saved_path)
                    .map_err(|error| error.to_string());
                let _ =
                    sender.send(crate::product::NotoraProductEvent::ExternalSaveAsCanonicalized {
                        tab_id: request.tab_id,
                        external_file_id,
                        content_revision,
                        result,
                    });
            })
            .is_err()
        {
            self.pending_external_save_as.remove(&request.tab_id);
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "could not start the external Save As path worker".to_owned(),
            ));
        }
    }

    fn complete_external_save_as_canonicalization(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        result: Result<CanonicalExternalPath, String>,
    ) {
        let Some(pending_save) = self.pending_external_save_as.get(&tab_id).copied() else {
            return;
        };
        if pending_save.external_file_id != external_file_id
            || pending_save.content_revision != content_revision
        {
            return;
        }
        self.pending_external_save_as.remove(&tab_id);
        let canonical_path = match result {
            Ok(canonical_path) => canonical_path,
            Err(message) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                return;
            }
        };
        match self.state.external_files.save_as(external_file_id, canonical_path) {
            Some(SaveExternalFileAs::Updated(_)) => {}
            Some(SaveExternalFileAs::PathAlreadyOpen(_)) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(
                    "save as target is already open in another external session".to_owned(),
                ))
            }
            None => self.dispatch_action(NotoraAction::NoteCommandFailed(
                "external session was closed before save as completed".to_owned(),
            )),
        }
    }

    fn request_catalog_reindex_after_note_save(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
    ) {
        if !matches!(
            self.document_origin_for_tab(tab_id),
            Some(notora_core::DocumentOrigin::Note { .. })
        ) {
            return;
        }
        if self.workspace_controller.request_catalog_reindex().is_err() {
            self.catalog_reconciliation_pending = true;
        }
    }

    fn document_origin_for_tab(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<notora_core::DocumentOrigin> {
        let identity = self.document_registry.identity_for(tab_id)?;
        match identity {
            DocumentIdentity::Note(note_id) => {
                let workspace = self.workspace_controller.active_workspace()?;
                let path = self.editor_runtime.document_summary(tab_id)?.path?;
                let relative_path =
                    path.strip_prefix(&workspace.descriptor.root).ok()?.to_path_buf();
                Some(notora_core::DocumentOrigin::Note {
                    workspace_id: workspace.descriptor.workspace_id,
                    note_id,
                    relative_path,
                })
            }
            DocumentIdentity::ExternalFile(external_file_id) => {
                match self.state.external_files.session(external_file_id)? {
                    ExternalFileSession::Existing { canonical_path, .. } => {
                        Some(notora_core::DocumentOrigin::ExternalFile {
                            external_file_id,
                            canonical_path: canonical_path.as_path().to_path_buf(),
                        })
                    }
                    ExternalFileSession::Untitled { kind, .. } => {
                        Some(notora_core::DocumentOrigin::UntitledExternal {
                            external_file_id,
                            kind: *kind,
                        })
                    }
                    ExternalFileSession::Missing { .. } => None,
                }
            }
        }
    }
}

fn submit_shell_frame(
    render_resources: &mut RenderResources,
    vertices: &[render::GlyphVertex],
    background: [f32; 4],
) {
    let Some(gpu) = render_resources.gpu.as_mut() else {
        return;
    };
    let surface_texture = match gpu.ctx.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
        _ => return,
    };
    let surface_view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("notora shell render encoder"),
    });
    match render_resources.text.as_mut() {
        Some(text) if !vertices.is_empty() => {
            upload_shell_vertices(text, gpu, vertices);
            render_shell_vertices(
                &mut encoder,
                text,
                gpu,
                &surface_view,
                vertices.len(),
                background,
            );
        }
        _ => clear_shell_surface(&mut encoder, &surface_view, background),
    }
    gpu.ctx.queue.submit(std::iter::once(encoder.finish()));
    surface_texture.present();
}

fn upload_shell_vertices(text: &mut TextState, gpu: &GpuState, vertices: &[render::GlyphVertex]) {
    let vertex_count = vertices.len() as u32;
    if vertex_count > text.vertex_capacity {
        let capacity = vertex_count.next_power_of_two();
        text.vertex_buffer = gpu.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("notora shell vertex buffer"),
            size: (capacity as usize * std::mem::size_of::<render::GlyphVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        text.vertex_capacity = capacity;
    }
    gpu.ctx.queue.write_buffer(&text.vertex_buffer, 0, bytemuck::cast_slice(vertices));
}

fn render_shell_vertices(
    encoder: &mut wgpu::CommandEncoder,
    text: &TextState,
    gpu: &GpuState,
    surface_view: &wgpu::TextureView,
    vertex_count: usize,
    background: [f32; 4],
) {
    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("notora shell text pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &gpu.ctx.msaa_view,
            depth_slice: None,
            resolve_target: Some(surface_view),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(to_wgpu_color(background)),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    });
    render_pass.set_pipeline(text.renderer.pipeline());
    render_pass.set_bind_group(0, &text.bind_group, &[]);
    render_pass.set_vertex_buffer(0, text.vertex_buffer.slice(..));
    render_pass.draw(0..vertex_count as u32, 0..1);
}

fn clear_shell_surface(
    encoder: &mut wgpu::CommandEncoder,
    surface_view: &wgpu::TextureView,
    background: [f32; 4],
) {
    let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("notora shell clear pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: surface_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(to_wgpu_color(background)),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    });
}

fn to_wgpu_color(color: [f32; 4]) -> wgpu::Color {
    wgpu::Color { r: color[0] as f64, g: color[1] as f64, b: color[2] as f64, a: color[3] as f64 }
}

fn action_requires_session_persistence(action: &NotoraAction) -> bool {
    matches!(
        action,
        NotoraAction::NavigationSelected(_)
            | NotoraAction::NavigationExpansionToggled(_)
            | NotoraAction::CardSelected(_)
            | NotoraAction::CardActivated(_)
            | NotoraAction::CompactNavigationRequested
            | NotoraAction::CompactBackRequested
            | NotoraAction::ExternalFileOpened(_)
            | NotoraAction::SplitterDragged { .. }
    )
}

impl Default for NotoraApp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod session_restore_tests {
    use super::action_requires_session_persistence;
    use crate::action::NotoraAction;

    #[test]
    fn session_restore_related_user_actions_are_debounced_for_persistence() {
        assert!(action_requires_session_persistence(&NotoraAction::NavigationSelected(
            notora_core::NavigationScope::Starred
        )));
        assert!(action_requires_session_persistence(&NotoraAction::CardSelected(
            notora_core::DocumentIdentity::Note(notora_core::NoteId::generate())
        )));
        assert!(!action_requires_session_persistence(&NotoraAction::CardListScrolled {
            offset_px: 5.0,
            near_end: false
        }));
    }
}

impl NotoraEffectService for NotoraApp {
    fn query_cards(&mut self, query: CardQuery) {
        if let Err(error) = self.workspace_controller.query_cards(query.clone()) {
            self.dispatch_action(NotoraAction::CardQueryFailed {
                query,
                message: error.to_string(),
            });
        }
    }

    fn request_note_creation(&mut self, _kind: DocumentKind, _target: NoteCreationTarget) {}

    fn execute_note_command(&mut self, command: notora_core::note_command::NoteCommand) {
        if let Err(error) = self.workspace_controller.execute_note_command(command) {
            self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
        }
    }

    fn execute_metadata_mutation(&mut self, mutation: crate::action::MetadataMutation) {
        if let Err(error) = self.workspace_controller.execute_metadata_mutation(mutation) {
            self.dispatch_action(NotoraAction::MetadataMutationFailed(error.to_string()));
        }
    }

    fn execute_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        if let crate::action::TrashOperation::MoveToTrash { note_id } = operation {
            let identity = DocumentIdentity::Note(note_id);
            if let Some(tab_id) = self.document_registry.tab_for(identity) {
                let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
                    self.dispatch_action(NotoraAction::TrashOperationFailed(
                        crate::action::TrashOperationFailure::Message(
                            "the open note is no longer available".to_owned(),
                        ),
                    ));
                    return;
                };
                if summary.dirty {
                    let Some(origin) = self.document_origin_for_tab(tab_id) else {
                        self.dispatch_action(NotoraAction::TrashOperationFailed(
                            crate::action::TrashOperationFailure::Message(
                                "only workspace notes can be moved to Trash".to_owned(),
                            ),
                        ));
                        return;
                    };
                    self.pending_trash_moves.insert(
                        tab_id,
                        PendingTrashMove { note_id, content_revision: summary.content_revision },
                    );
                    self.autosave.request_immediate_save(&origin, tab_id, summary.content_revision);
                    self.process_due_autosaves();
                    return;
                }
            }
        }
        if let Err(error) = self.workspace_controller.execute_trash_operation(operation) {
            self.dispatch_action(NotoraAction::TrashOperationFailed(
                crate::action::TrashOperationFailure::Message(error.to_string()),
            ));
        }
    }

    fn choose_note_rename_destination(&mut self, note_id: notora_core::NoteId) {
        let identity = DocumentIdentity::Note(note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "open the note before renaming it".to_owned(),
            ));
            return;
        };
        let Some(current_path) =
            self.editor_runtime.document_summary(tab_id).and_then(|summary| summary.path)
        else {
            return;
        };
        let Some(file_name) = current_path.file_name() else {
            return;
        };
        let mut dialog = rfd::FileDialog::new().set_file_name(file_name.to_string_lossy());
        if let Some(parent) = current_path.parent() {
            dialog = dialog.set_directory(parent);
        }
        let Some(destination) = dialog.save_file() else {
            return;
        };
        let new_file_name = match rename_file_name_for_destination(&current_path, &destination) {
            Ok(file_name) => file_name,
            Err(message) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                return;
            }
        };
        self.dispatch_action(NotoraAction::RenameRequested { note_id, new_file_name });
    }

    fn choose_note_move_directory(&mut self, note_id: notora_core::NoteId) {
        let identity = DocumentIdentity::Note(note_id);
        let current_directory = self
            .document_registry
            .tab_for(identity)
            .and_then(|tab_id| self.editor_runtime.document_summary(tab_id))
            .and_then(|summary| summary.path)
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
        let mut dialog = rfd::FileDialog::new();
        if let Some(directory) = current_directory {
            dialog = dialog.set_directory(directory);
        }
        let Some(destination) = dialog.pick_folder() else {
            return;
        };
        let Some(workspace) = self.workspace_controller.active_workspace() else {
            return;
        };
        let target_directory =
            match workspace_relative_directory(&workspace.descriptor.root, &destination) {
                Ok(relative_path) => relative_path,
                Err(message) => {
                    self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                    return;
                }
            };
        self.dispatch_action(NotoraAction::MoveRequested { note_id, target_directory });
    }

    fn prepare_document(&mut self, request: DocumentLoadRequest) {
        let identity = request.identity;
        if let DocumentIdentity::ExternalFile(external_file_id) = identity {
            self.prepare_external_document(request, external_file_id);
            return;
        }
        if let Some(tab_id) = self.document_registry.tab_for(identity) {
            self.document_registry.touch_tab(tab_id);
            let outcome = self.editor_runtime.activate(tab_id);
            self.apply_editor_outcome(outcome);
            return;
        }
        if let Err(error) = self.workspace_controller.prepare_document(request) {
            self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
        }
    }

    fn promote_active_preview(&mut self) {
        self.promote_active_preview_tab();
    }

    fn open_external_files(&mut self, request: ExternalOpenRequest) {
        let paths = match request {
            ExternalOpenRequest::ShowFileDialog => rfd::FileDialog::new()
                .add_filter("Text documents", &["txt", "md"])
                .pick_files()
                .unwrap_or_default(),
            ExternalOpenRequest::Paths(paths) => paths,
        };
        self.open_external_paths(paths);
    }

    fn create_untitled_external(&mut self, kind: notora_core::DocumentKind) {
        let identity = self.state.external_files.create_untitled(kind);
        self.dispatch_action(NotoraAction::ExternalFileOpened(identity));
    }

    fn save_document_manually(&mut self, request: ManualSaveRequest) {
        match request {
            ManualSaveRequest::Note { tab_id, content_revision } => {
                let Some(origin) = self.document_origin_for_tab(tab_id) else {
                    return;
                };
                self.autosave.request_immediate_save(&origin, tab_id, content_revision);
                self.process_due_autosaves();
            }
            ManualSaveRequest::ExistingExternalFile { tab_id } => {
                let _ = self.submit_manual_external_save(tab_id);
            }
            ManualSaveRequest::UntitledExternalFile { tab_id, external_file_id } => {
                self.save_untitled_external_file(tab_id, external_file_id);
            }
        }
    }

    fn resolve_save_conflict(&mut self, request: SaveConflictRequest) {
        match request.resolution {
            ConflictResolution::ReloadFromDisk => self.reload_conflicted_document(request.identity),
            ConflictResolution::RetrySave => self.retry_conflicted_document_save(request.identity),
            ConflictResolution::SaveCopy => self.save_conflicted_note_copy(request.identity),
            ConflictResolution::Cancel => {}
        }
    }

    fn apply_product_settings_update(
        &mut self,
        update: crate::settings_overlay::ProductSettingsUpdate,
    ) {
        update.apply_to(&mut self.product_settings);
        self.product_settings.apply_to_ui(&mut self.settings);
        let runtime_tab_limit =
            NonZeroUsize::new(self.product_settings.interface.runtime_tab_limit)
                .or_else(|| NonZeroUsize::new(DEFAULT_RUNTIME_TAB_LIMIT))
                .expect("default runtime tab limit must be non-zero");
        self.runtime_lru = RuntimeLru::new(runtime_tab_limit);
        self.evict_excess_runtime_tabs();
        self.autosave.set_idle_delay(Duration::from_millis(
            self.product_settings.workspace.auto_save_delay_millis,
        ));
        self.editor_runtime.update_settings(self.settings.clone());
        self.rebuild_theme_for_system_appearance(self.current_system_appearance());
        self.enqueue_product_settings_persistence();
    }

    fn persist_product_settings(&mut self) {
        self.enqueue_product_settings_persistence();
    }

    fn persist_layout(&mut self) {
        self.schedule_session_persistence();
    }
}

impl NotoraApp {
    fn enqueue_product_settings_persistence(&mut self) {
        if let Err(error) = self
            .persistence_worker
            .save_settings(self.paths.settings_file.clone(), self.product_settings.clone())
        {
            self.record_settings_persistence_result(Err(error.to_string()));
        }
    }

    fn record_settings_persistence_result(&mut self, result: Result<(), String>) {
        self.settings_persistence = match result {
            Ok(()) => SettingsPersistenceState::Saved,
            Err(message) => SettingsPersistenceState::SaveFailed { message },
        };
        self.needs_redraw = true;
    }

    fn schedule_session_persistence(&mut self) {
        self.pending_session_persist_at = Some(Instant::now() + SESSION_PERSIST_DEBOUNCE_DELAY);
    }
    fn capture_product_session(&self) -> crate::session::ProductSession {
        let (workspace_root, workspace_id) = self
            .workspace_controller
            .active_workspace()
            .map(|workspace| {
                (Some(workspace.descriptor.root), Some(workspace.descriptor.workspace_id))
            })
            .unwrap_or((None, None));
        let external_paths = self
            .state
            .external_files
            .sessions()
            .iter()
            .filter_map(|session| match session {
                ExternalFileSession::Existing { canonical_path, .. } => {
                    Some(canonical_path.as_path().to_path_buf())
                }
                ExternalFileSession::Untitled { .. } | ExternalFileSession::Missing { .. } => None,
            })
            .collect();
        let last_document = match self.state.library.selected_card {
            Some(DocumentIdentity::Note(note_id)) => {
                Some(crate::session::SavedDocument::Note { note_id })
            }
            Some(DocumentIdentity::ExternalFile(external_file_id)) => {
                self.state.external_files.session(external_file_id).and_then(
                    |session| match session {
                        ExternalFileSession::Existing { canonical_path, .. } => {
                            Some(crate::session::SavedDocument::ExternalPath {
                                path: canonical_path.as_path().to_path_buf(),
                            })
                        }
                        ExternalFileSession::Untitled { .. }
                        | ExternalFileSession::Missing { .. } => None,
                    },
                )
            }
            None => None,
        };
        crate::session::ProductSession {
            workspace_root,
            workspace_id,
            external_paths,
            last_navigation_scope: (&self.state.library.navigation_scope).into(),
            last_document,
            expanded_directories: self
                .state
                .library
                .navigation_tree
                .expanded_directories
                .iter()
                .cloned()
                .collect(),
            navigation_width_logical: self.state.layout.navigation_width_logical,
            card_list_width_logical: self.state.layout.card_list_width_logical,
            window_geometry: self.capture_window_geometry(),
            ..crate::session::ProductSession::default()
        }
    }

    fn capture_window_geometry(&self) -> crate::session::WindowGeometry {
        let fallback = crate::session::WindowGeometry {
            width_px: self.window_width_px,
            height_px: self.window_height_px,
            ..crate::session::WindowGeometry::default()
        };
        let Some(window) = self.editor_runtime.window() else {
            return fallback;
        };
        let outer_position = window.outer_position().ok();
        let inner_size = window.inner_size();
        crate::session::WindowGeometry {
            x_px: outer_position.map_or(fallback.x_px, |position| position.x as f32),
            y_px: outer_position.map_or(fallback.y_px, |position| position.y as f32),
            width_px: inner_size.width as f32,
            height_px: inner_size.height as f32,
        }
    }

    fn restore_pending_session(&mut self) {
        let Some(session) = self.pending_session.take() else {
            return;
        };
        self.window_width_px = session.window_geometry.width_px;
        self.window_height_px = session.window_geometry.height_px;
        let workspace_restored = match session.workspace_root {
            Some(root) if root.is_dir() => match self
                .execute_workspace_command(WorkspaceCommand::OpenExisting { root })
            {
                Ok(WorkspaceCommandResult::Opened(workspace))
                    if session
                        .workspace_id
                        .is_none_or(|saved_id| saved_id == workspace.descriptor.workspace_id) =>
                {
                    true
                }
                Ok(WorkspaceCommandResult::Opened(_)) => {
                    let _ = self.execute_workspace_command(WorkspaceCommand::Close);
                    false
                }
                Ok(WorkspaceCommandResult::Unchanged | WorkspaceCommandResult::Closed { .. })
                | Err(_) => false,
            },
            Some(_) | None => false,
        };
        if !workspace_restored && session.workspace_id.is_some() {
            self.state.library.last_command_error = Some(
                "the previous workspace is unavailable or no longer matches its saved identity"
                    .to_owned(),
            );
        }
        let saved_last_document = session.last_document.clone();
        let saved_external_path = match &saved_last_document {
            Some(crate::session::SavedDocument::ExternalPath { path }) => Some(path.as_path()),
            Some(crate::session::SavedDocument::Note { .. }) | None => None,
        };
        self.restore_external_paths(session.external_paths, saved_external_path);
        if workspace_restored {
            self.state
                .library
                .navigation_tree
                .expanded_directories
                .extend(session.expanded_directories);
            self.dispatch_action(NotoraAction::NavigationSelected(
                session.last_navigation_scope.into(),
            ));
        } else if session.last_navigation_scope
            == crate::session::SavedNavigationScope::ExternalFiles
        {
            self.dispatch_action(NotoraAction::NavigationSelected(
                notora_core::NavigationScope::ExternalFiles,
            ));
        }
        match saved_last_document {
            Some(crate::session::SavedDocument::Note { note_id }) if workspace_restored => {
                self.dispatch_action(NotoraAction::CardSelected(DocumentIdentity::Note(note_id)));
            }
            Some(crate::session::SavedDocument::ExternalPath { .. }) => {}
            Some(crate::session::SavedDocument::Note { .. }) | None => {}
        }
    }

    pub(crate) fn restore_session_after_first_frame(&mut self) {
        if self.pending_session.is_none() {
            return;
        }
        self.restore_pending_session();
        self.needs_redraw = true;
    }

    fn request_navigation_tree(&mut self) {
        if let Err(error) = self.workspace_controller.query_navigation_tree() {
            self.dispatch_action(NotoraAction::NavigationTreeFailed(error.to_string()));
        }
    }

    fn complete_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        let crate::action::TrashOperation::MoveToTrash { note_id } = operation else {
            return;
        };
        let identity = DocumentIdentity::Note(note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return;
        };
        self.autosave.cancel(tab_id);
        let _ = self.editor_runtime.close_for_product(tab_id);
        self.document_registry.remove_tab(tab_id);
        if self.state.library.selected_card == Some(identity) {
            self.state.library.selected_card = None;
        }
    }

    fn schedule_catalog_backup(&mut self) {
        self.pending_catalog_backup_at = Some(Instant::now() + CATALOG_BACKUP_DEBOUNCE_DELAY);
    }

    fn start_catalog_backup(&mut self) {
        let Some(active_workspace) = self.workspace_controller.active_workspace() else {
            return;
        };
        let Some(retention) = notora_core::BackupRetention::keep_latest(
            self.product_settings.workspace.catalog_backup_retention,
        ) else {
            return;
        };
        let directory = self
            .paths
            .catalog_backups_directory
            .join(active_workspace.descriptor.workspace_id.to_string());
        if let Err(error) = self.workspace_controller.create_catalog_backup(directory, retention) {
            self.dispatch_action(NotoraAction::MetadataMutationFailed(format!(
                "metadata was saved, but its catalog backup could not start: {error}"
            )));
        }
    }

    fn flush_pending_catalog_backup(&mut self) {
        if self.pending_catalog_backup_at.take().is_some() {
            self.start_catalog_backup();
        }
    }

    fn retry_conflicted_document_save(&mut self, identity: DocumentIdentity) {
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return;
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return;
        };
        let Some(path) = summary.path else {
            return;
        };
        let content_revision = summary.content_revision;
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-conflict-retry-revision".to_owned())
            .spawn(move || {
                let event = match appkit_core::file_safety::capture_revision(&path) {
                    Ok(disk_revision) => {
                        crate::product::NotoraProductEvent::ConflictRetryRevisionCaptured {
                            identity,
                            tab_id,
                            content_revision,
                            path,
                            disk_revision,
                        }
                    }
                    Err(error) => {
                        crate::product::NotoraProductEvent::ConflictRetryRevisionFailed {
                            identity,
                            message: format!(
                                "could not read the current disk version before retrying the save: {error}"
                            ),
                        }
                    }
                };
                let _ = sender.send(event);
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "could not start the conflict retry revision worker".to_owned(),
            ));
        }
    }

    fn complete_conflict_retry_revision_capture(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
        disk_revision: appkit_core::file_safety::DiskRevision,
    ) {
        if self.document_registry.identity_for(tab_id) != Some(identity) {
            return;
        }
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return;
        };
        if summary.content_revision != content_revision
            || !self.editor_runtime.update_document_path(tab_id, path, Some(disk_revision))
        {
            return;
        }
        let Some(request) = self.manual_save_request_for_tab(tab_id) else {
            return;
        };
        let pending_retry = PendingConflictRetry { identity, content_revision };
        match request {
            ManualSaveRequest::Note { .. } => {
                self.pending_conflict_retries.insert(tab_id, pending_retry);
                EffectExecutor::save_document_manually(self, request);
            }
            ManualSaveRequest::ExistingExternalFile { .. } => {
                if self.submit_manual_external_save(tab_id) {
                    self.pending_conflict_retries.insert(tab_id, pending_retry);
                }
            }
            ManualSaveRequest::UntitledExternalFile { .. } => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(
                    "an untitled document has no disk conflict to retry".to_owned(),
                ));
            }
        }
    }

    fn save_conflicted_note_copy(&mut self, identity: DocumentIdentity) {
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return;
        };
        let Some(path) =
            rfd::FileDialog::new().add_filter("Text documents", &["txt", "md"]).save_file()
        else {
            return;
        };
        let prepared = match self.editor_runtime.prepare_save_as(tab_id, &path) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
                return;
            }
        };
        let Some(workspace) = self.workspace_controller.active_workspace() else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "cannot save a conflict copy after its workspace is closed".to_owned(),
            ));
            return;
        };
        let sender = self.product.event_sender();
        let workspace_id = workspace.descriptor.workspace_id;
        let workspace_generation = workspace.generation;
        if thread::Builder::new()
            .name("notora-conflict-copy".to_owned())
            .spawn(move || {
                let result = appkit_shell::editor_runtime::execute_prepared_save(prepared)
                    .result
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                let _ = sender.send(crate::product::NotoraProductEvent::ConflictCopyCompleted {
                    workspace_id,
                    workspace_generation,
                    identity,
                    result,
                });
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "could not start the conflict copy worker".to_owned(),
            ));
        }
    }

    fn reload_conflicted_document(&mut self, identity: DocumentIdentity) {
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return;
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return;
        };
        let Some(path) = summary.path else {
            return;
        };
        let content_revision = summary.content_revision;
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-conflict-reload".to_owned())
            .spawn(move || match load_document(&path) {
                Ok(document) => {
                    let _ =
                        sender.send(crate::product::NotoraProductEvent::ConflictReloadCompleted {
                            identity,
                            tab_id,
                            content_revision,
                            document,
                        });
                }
                Err(error) => {
                    let _ = sender.send(crate::product::NotoraProductEvent::ConflictReloadFailed {
                        identity,
                        message: error.to_string(),
                    });
                }
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "could not start the conflict reload worker".to_owned(),
            ));
        }
    }

    fn open_external_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        self.start_external_file_reads(paths.into_iter().map(|path| (path, true)).collect());
    }

    fn restore_external_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        saved_last_path: Option<&std::path::Path>,
    ) {
        let requests = paths
            .into_iter()
            .map(|path| {
                let activate = saved_last_path.is_some_and(|saved_path| saved_path == path);
                (path, activate)
            })
            .collect();
        self.start_external_file_reads(requests);
    }

    fn start_external_file_reads(&mut self, requests: Vec<(std::path::PathBuf, bool)>) {
        if requests.is_empty() {
            return;
        }
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-external-open".to_owned())
            .spawn(move || {
                for (path, activate) in requests {
                    let result = validate_external_text_file(&path)
                        .map_err(|error| error.to_string())
                        .and_then(|validated| {
                            load_document(validated.canonical_path.as_path())
                                .map(|document| (validated.canonical_path, document))
                                .map_err(|error| error.to_string())
                        });
                    let event = match result {
                        Ok((canonical_path, document)) => {
                            crate::product::NotoraProductEvent::ExternalFileOpenCompleted {
                                canonical_path,
                                document,
                                activate,
                            }
                        }
                        Err(message) => {
                            crate::product::NotoraProductEvent::ExternalFileOpenFailed { message }
                        }
                    };
                    let _ = sender.send(event);
                }
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "could not start the external file reader".to_owned(),
            ));
        }
    }

    fn prepare_external_document(
        &mut self,
        request: DocumentLoadRequest,
        external_file_id: notora_core::ExternalFileId,
    ) {
        let Some(session) = self.state.external_files.session(external_file_id).cloned() else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "external document is unavailable; relocate or remove its session".to_owned(),
            ));
            return;
        };
        match session {
            ExternalFileSession::Existing { canonical_path, .. } => {
                if let Some(document) = self.pending_external_documents.remove(&external_file_id) {
                    self.install_loaded_preview(request, document);
                    return;
                }
                self.start_external_document_load(request, canonical_path);
            }
            ExternalFileSession::Untitled { kind, .. } => {
                let (prepared, suggested_file_name) =
                    match prepare_untitled_document(&self.editor_runtime, kind) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            self.dispatch_action(NotoraAction::NoteCommandFailed(
                                error.to_string(),
                            ));
                            return;
                        }
                    };
                self.install_prepared_preview(request, prepared, Some(suggested_file_name));
            }
            ExternalFileSession::Missing { .. } => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(
                    "external document is unavailable; relocate or remove its session".to_owned(),
                ))
            }
        }
    }

    fn complete_external_file_open(
        &mut self,
        canonical_path: CanonicalExternalPath,
        document: LoadedDocument,
        activate: bool,
    ) {
        let identity = self.state.external_files.open_existing(canonical_path).identity();
        if !activate {
            return;
        }
        let DocumentIdentity::ExternalFile(external_file_id) = identity else {
            return;
        };
        if self.document_registry.tab_for(identity).is_none() {
            self.pending_external_documents.insert(external_file_id, document);
        }
        self.dispatch_action(NotoraAction::ExternalFileOpened(identity));
    }

    fn start_external_document_load(
        &mut self,
        request: DocumentLoadRequest,
        canonical_path: CanonicalExternalPath,
    ) {
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-external-document-load".to_owned())
            .spawn(move || {
                let event = match load_document(canonical_path.as_path()) {
                    Ok(document) => crate::product::NotoraProductEvent::ExternalDocumentLoaded {
                        request,
                        document,
                    },
                    Err(error) => crate::product::NotoraProductEvent::ExternalDocumentLoadFailed {
                        request,
                        message: error.to_string(),
                    },
                };
                let _ = sender.send(event);
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "could not start the external document reader".to_owned(),
            ));
        }
    }

    fn complete_conflict_reload(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        loaded: LoadedDocument,
    ) {
        if self.document_registry.identity_for(tab_id) != Some(identity) {
            return;
        }
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return;
        };
        if summary.content_revision != content_revision {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "the document changed while its disk version was loading".to_owned(),
            ));
            return;
        }
        let prepared = match prepare_loaded_document(&self.editor_runtime, loaded) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
                return;
            }
        };
        if self.editor_runtime.replace_document(tab_id, prepared.document) {
            self.autosave.cancel(tab_id);
            self.dispatch_action(NotoraAction::SaveConflictResolved { identity });
        }
    }
}

fn rename_file_name_for_destination(
    current_path: &std::path::Path,
    destination: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let current_parent = current_path
        .parent()
        .ok_or_else(|| "the current note has no parent directory".to_owned())?;
    let destination_parent = destination
        .parent()
        .ok_or_else(|| "the rename destination has no parent directory".to_owned())?;
    let current_parent = std::fs::canonicalize(current_parent)
        .map_err(|error| format!("could not resolve the current note directory: {error}"))?;
    let destination_parent = std::fs::canonicalize(destination_parent)
        .map_err(|error| format!("could not resolve the rename destination: {error}"))?;
    if current_parent != destination_parent {
        return Err(
            "Rename keeps the note in its current folder; use Move for another folder".to_owned()
        );
    }
    destination
        .file_name()
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "the rename destination has no file name".to_owned())
}

fn workspace_relative_directory(
    workspace_root: &std::path::Path,
    destination: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let workspace_root = std::fs::canonicalize(workspace_root)
        .map_err(|error| format!("could not resolve the active workspace: {error}"))?;
    let destination = std::fs::canonicalize(destination)
        .map_err(|error| format!("could not resolve the move destination: {error}"))?;
    destination
        .strip_prefix(&workspace_root)
        .map(std::path::Path::to_path_buf)
        .map_err(|_| "Move destination must stay inside the active workspace".to_owned())
}

fn build_editor_runtime(
    settings: &ui::Settings,
    theme: &ui::Theme,
    paths: &NotoraPaths,
) -> Result<EditorRuntime, NotoraAppError> {
    let (plugin_registry, view_routes) = build_editor_plugins()
        .map_err(EditorRuntimeError::InvalidRoute)
        .map_err(NotoraAppError::Runtime)?;
    EditorRuntime::new(EditorRuntimeConfig {
        plugin_registry,
        view_routes,
        initial_settings: settings.clone(),
        initial_theme: theme.clone(),
        snapshots_directory: paths.snapshots_directory.clone(),
    })
    .map_err(NotoraAppError::Runtime)
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{
        NotoraApp, PendingTrashMove, SettingsPersistenceState, rename_file_name_for_destination,
        workspace_relative_directory,
    };
    use crate::action::NotoraAction;
    use crate::autosave::AutoSaveState;
    use crate::editor_adapter::LoadedDocument;
    use crate::{ExternalFileSession, FocusTarget, NotoraPaths, OverlayState, WorkspaceCommand};
    use notora_core::{DocumentIdentity, NavigationScope, WorkspaceId};

    fn app() -> NotoraApp {
        let directory = tempfile::tempdir().expect("test should create a temporary directory");
        let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
            .expect("test should create isolated product paths");
        NotoraApp::with_paths(paths).expect("notora app should construct without a window")
    }

    #[test]
    fn note_dialog_destinations_are_reduced_to_safe_domain_inputs() {
        let workspace = tempfile::tempdir().expect("workspace fixture should exist");
        let notes_directory = workspace.path().join("notes");
        let archive_directory = workspace.path().join("archive");
        let outside_directory = tempfile::tempdir().expect("outside fixture should exist");
        std::fs::create_dir_all(&notes_directory).expect("notes directory should exist");
        std::fs::create_dir_all(&archive_directory).expect("archive directory should exist");
        let current_path = notes_directory.join("current.md");

        assert_eq!(
            rename_file_name_for_destination(&current_path, &notes_directory.join("renamed.md")),
            Ok("renamed.md".into())
        );
        assert!(
            rename_file_name_for_destination(&current_path, &archive_directory.join("renamed.md"))
                .is_err()
        );
        assert_eq!(
            workspace_relative_directory(workspace.path(), &archive_directory),
            Ok("archive".into())
        );
        assert!(workspace_relative_directory(workspace.path(), outside_directory.path()).is_err());
    }

    #[test]
    fn document_ime_is_gated_by_product_focus_and_modal_state() {
        let mut app = app();
        assert!(!app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        assert!(app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

        app.dispatch_action(NotoraAction::OpenSettings);
        assert_eq!(app.state().layout.overlay, OverlayState::Settings);
        assert!(!app.update_editor_preedit("音".to_owned(), Some((0, 1))));
    }

    #[test]
    fn settings_modal_consumes_unhandled_keyboard_input() {
        let mut app = app();
        app.dispatch_action(NotoraAction::OpenSettings);

        assert!(app.route_product_event(&ui::Event::KeyDown(
            ui::KeyCode::Char('x'),
            ui::core::Modifiers::NONE,
        )));
    }

    #[test]
    fn notora_theme_mode_resolves_against_its_own_product_settings() {
        let mut app = app();
        app.product_settings.appearance.theme_mode = ui::ThemeMode::System;
        app.rebuild_theme_for_system_appearance(winit::window::Theme::Light);
        assert!(!app.theme.is_dark);
        app.rebuild_theme_for_system_appearance(winit::window::Theme::Dark);
        assert!(app.theme.is_dark);

        app.product_settings.appearance.theme_mode = ui::ThemeMode::Light;
        app.rebuild_theme_for_system_appearance(winit::window::Theme::Dark);
        assert!(!app.theme.is_dark);
    }

    #[test]
    fn notora_editor_setting_updates_its_product_and_runtime_snapshots() {
        let mut app = app();

        app.dispatch_action(NotoraAction::ProductSettingsUpdateRequested(
            crate::settings_overlay::ProductSettingsUpdate::FontSize(20.0),
        ));

        assert_eq!(app.product_settings.editor.font_size, 20.0);
        assert_eq!(app.editor_runtime.settings_snapshot().font_size, 20.0);
    }

    #[test]
    fn watcher_disconnection_marks_reconciliation_pending_and_surfaces_a_diagnostic() {
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.drain_product_events();
        let active_workspace = app
            .workspace_controller
            .active_workspace()
            .expect("opened workspace should stay active");
        app.product
            .event_sender()
            .send(crate::product::NotoraProductEvent::WorkspaceIndexFailed {
                workspace_id: active_workspace.descriptor.workspace_id,
                workspace_generation: active_workspace.generation,
                message: "workspace file monitor disconnected; automatic reconciliation stopped"
                    .to_owned(),
            })
            .expect("product receiver should stay available");

        app.drain_product_events();

        assert!(app.catalog_reconciliation_pending);
        assert!(
            app.state()
                .library
                .last_command_error
                .as_deref()
                .is_some_and(|message| message.contains("file monitor disconnected"))
        );
    }

    #[test]
    fn session_workspace_identity_mismatch_closes_the_unexpected_workspace() {
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        notora_core::Workspace::open_or_initialize(workspace_directory.path())
            .expect("workspace metadata should initialize");
        let mut app = app();
        app.pending_session = Some(crate::session::ProductSession {
            workspace_root: Some(workspace_directory.path().to_path_buf()),
            workspace_id: Some(WorkspaceId::generate()),
            ..crate::session::ProductSession::default()
        });

        app.restore_pending_session();

        assert_eq!(app.workspace_controller.active_workspace(), None);
        assert!(
            app.state
                .library
                .last_command_error
                .as_deref()
                .is_some_and(|message| message.contains("no longer matches"))
        );
    }

    #[test]
    fn session_restore_registers_all_external_paths_but_activates_only_the_saved_last_document() {
        let directory = tempfile::tempdir().expect("external fixture directory should be created");
        let saved_last_path = directory.path().join("last.md");
        let other_path = directory.path().join("other.md");
        std::fs::write(&saved_last_path, "# Last").expect("last document should be written");
        std::fs::write(&other_path, "# Other").expect("other document should be written");
        let mut app = app();
        app.pending_session = Some(crate::session::ProductSession {
            external_paths: vec![saved_last_path.clone(), other_path],
            last_document: Some(crate::session::SavedDocument::ExternalPath {
                path: saved_last_path.clone(),
            }),
            ..crate::session::ProductSession::default()
        });

        app.restore_pending_session();

        let deadline = Instant::now() + Duration::from_secs(2);
        while app.state.external_files.sessions().len() < 2 {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "external sessions should restore promptly");
            thread::sleep(Duration::from_millis(10));
        }
        let selected_identity = app
            .state
            .library
            .selected_card
            .expect("saved last external document should be selected");
        let selected_path = match selected_identity {
            DocumentIdentity::ExternalFile(external_file_id) => {
                app.state
                    .external_files
                    .session(external_file_id)
                    .and_then(|session| match session {
                        ExternalFileSession::Existing { canonical_path, .. } => {
                            Some(canonical_path.as_path())
                        }
                        ExternalFileSession::Untitled { .. }
                        | ExternalFileSession::Missing { .. } => None,
                    })
                    .expect("selected external session should have a path")
            }
            DocumentIdentity::Note(_) => panic!("session should restore an external document"),
        };
        let canonical_saved_last_path = std::fs::canonicalize(saved_last_path)
            .expect("saved last document should canonicalize");
        assert_eq!(selected_path, canonical_saved_last_path.as_path());
        assert_eq!(app.editor_runtime_tab_count(), 1);
    }

    #[test]
    fn headless_session_capture_uses_the_latest_window_size_with_safe_position_defaults() {
        let mut app = app();
        app.set_window_size(1_024, 768);

        let geometry = app.capture_window_geometry();

        assert_eq!(geometry.x_px, crate::session::WindowGeometry::default().x_px);
        assert_eq!(geometry.y_px, crate::session::WindowGeometry::default().y_px);
        assert_eq!(geometry.width_px, 1_024.0);
        assert_eq!(geometry.height_px, 768.0);
    }

    #[test]
    fn settings_persistence_failure_is_visible_and_retry_clears_it() {
        let mut app = app();
        let occupied_parent = app.paths.config_directory.join("occupied-settings-parent");
        std::fs::write(&occupied_parent, "not a directory")
            .expect("settings failure fixture should be written");
        app.paths.settings_file = occupied_parent.join("settings.toml");

        crate::effect_executor::NotoraEffectService::apply_product_settings_update(
            &mut app,
            crate::settings_overlay::ProductSettingsUpdate::WordWrap(false),
        );

        assert_eq!(app.state.library.last_command_error, None);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.drain_product_events();
            if matches!(app.settings_persistence, SettingsPersistenceState::SaveFailed { .. }) {
                break;
            }
            assert!(Instant::now() < deadline, "settings persistence failure should arrive");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(matches!(
            app.settings_persistence.to_view(),
            crate::settings_overlay::NotoraSettingsPersistenceView::SaveFailed { .. }
        ));
        let retry_path = app.paths.config_directory.join("retry-settings.toml");
        app.paths.settings_file = retry_path.clone();
        app.dispatch_action(NotoraAction::RetryProductSettingsPersistence);

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.drain_product_events();
            if app.settings_persistence == SettingsPersistenceState::Saved {
                break;
            }
            assert!(Instant::now() < deadline, "settings persistence retry should complete");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!crate::settings::load_product_settings(&retry_path).settings.editor.word_wrap);
    }

    #[test]
    fn search_ime_commit_is_consumed_before_it_can_reach_the_editor_runtime() {
        let mut app = app();
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        assert!(app.update_editor_preedit("document".to_owned(), Some((0, 8))));
        app.render().expect("headless shell frame should render");

        assert!(app.route_product_event(&ui::Event::MouseDown {
            px: 24.0,
            py: 24.0,
            button: ui::core::widget::MouseButton::Left,
        }));
        assert_eq!(app.state().layout.focus_target, FocusTarget::NavigationSearch);
        assert!(app.route_product_event(&ui::Event::ImeCommit("路线图".to_owned())));

        assert_eq!(
            app.state().library.navigation_scope,
            NavigationScope::Search { query: "路线图".to_owned() }
        );
        assert_eq!(app.editor_runtime.preedit().0, "document");
    }

    #[test]
    fn active_workspace_note_creation_reaches_the_worker_and_updates_product_state() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.drain_product_events();
            if matches!(
                app.state().library.selected_card,
                Some(notora_core::DocumentIdentity::Note(_))
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "note completion should update product state");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(workspace_directory.path().join("未命名 1.md").is_file());
        assert_eq!(app.state().library.last_command_error, None);
    }

    #[test]
    fn dirty_note_is_not_moved_to_trash_when_its_required_save_fails() {
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        let note_id = loop {
            app.drain_product_events();
            if let Some(notora_core::DocumentIdentity::Note(note_id)) =
                app.state().library.selected_card
                && app.document_tab_for(notora_core::DocumentIdentity::Note(note_id)).is_some()
            {
                break note_id;
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        let identity = notora_core::DocumentIdentity::Note(note_id);
        let tab_id = app.document_tab_for(identity).expect("note should have an open tab");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        app.commit_editor_text("unsaved edit".to_owned());
        assert!(
            app.editor_runtime.document_summary(tab_id).expect("tab should remain available").dirty
        );

        app.dispatch_action(NotoraAction::TrashOperationRequested(
            crate::action::TrashOperation::MoveToTrash { note_id },
        ));

        assert!(workspace_directory.path().join("未命名 1.md").is_file());
        assert_eq!(app.document_tab_for(identity), Some(tab_id));
        assert_eq!(app.state().library.selected_card, Some(identity));
        assert!(app.pending_trash_moves.is_empty());
        assert!(
            app.state()
                .library
                .last_command_error
                .as_deref()
                .is_some_and(|message| message.contains("not moved to Trash"))
        );
    }

    #[test]
    fn pending_trash_move_rejects_a_document_changed_after_its_save_started() {
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        let (note_id, tab_id, saved_revision) = loop {
            app.drain_product_events();
            if let Some(notora_core::DocumentIdentity::Note(note_id)) =
                app.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
                && let Some(summary) = app.editor_runtime.document_summary(tab_id)
            {
                break (note_id, tab_id, summary.content_revision);
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        let pending_move = PendingTrashMove { note_id, content_revision: saved_revision };
        assert!(app.pending_trash_move_has_current_saved_document(tab_id, pending_move));

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        app.commit_editor_text("newer edit".to_owned());

        assert!(!app.pending_trash_move_has_current_saved_document(tab_id, pending_move));
    }

    #[test]
    fn clean_note_moves_to_trash_and_closes_its_runtime_after_the_worker_confirms() {
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        let note_id = loop {
            app.drain_product_events();
            if let Some(notora_core::DocumentIdentity::Note(note_id)) =
                app.state().library.selected_card
                && app.document_tab_for(notora_core::DocumentIdentity::Note(note_id)).is_some()
            {
                break note_id;
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        let identity = notora_core::DocumentIdentity::Note(note_id);
        app.dispatch_action(NotoraAction::TrashOperationRequested(
            crate::action::TrashOperation::MoveToTrash { note_id },
        ));

        loop {
            app.drain_product_events();
            if app.document_tab_for(identity).is_none() {
                break;
            }
            assert!(Instant::now() < deadline, "trash completion should close the matching tab");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(!workspace_directory.path().join("未命名 1.md").exists());
        assert_eq!(app.state().library.selected_card, None);
        assert_eq!(app.editor_runtime_tab_count(), 0);
        assert_eq!(app.state().library.last_command_error, None);
    }

    #[test]
    fn selected_note_is_loaded_by_the_worker_and_installed_as_a_preview_tab() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        let selected_identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "selected note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(app.editor_runtime_tab_count(), 1);
        assert!(app.document_tab_for(selected_identity).is_some());
        assert_eq!(app.state().library.last_command_error, None);
    }

    #[test]
    fn renaming_an_open_note_updates_the_existing_runtime_path() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        let (note_id, tab_id) = loop {
            app.drain_product_events();
            if let Some(notora_core::DocumentIdentity::Note(note_id)) =
                app.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
            {
                break (note_id, tab_id);
            }
            assert!(Instant::now() < deadline, "created note should install a preview");
            thread::sleep(Duration::from_millis(10));
        };

        app.dispatch_action(NotoraAction::RenameRequested {
            note_id,
            new_file_name: "renamed.md".into(),
        });
        loop {
            app.drain_product_events();
            if app
                .editor_runtime
                .document_summary(tab_id)
                .and_then(|summary| summary.path)
                .is_some_and(|path| path.ends_with("renamed.md"))
            {
                break;
            }
            assert!(Instant::now() < deadline, "rename command should complete");
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(
            app.document_tab_for(notora_core::DocumentIdentity::Note(note_id)),
            Some(tab_id)
        );
        assert_eq!(
            app.editor_runtime.document_summary(tab_id).and_then(|summary| summary.path),
            Some(
                app.workspace_controller
                    .active_workspace()
                    .expect("workspace should remain active")
                    .descriptor
                    .root
                    .join("renamed.md")
            )
        );
    }

    #[test]
    fn promoting_a_preview_preserves_its_tab_when_the_next_preview_opens() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        let first_identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "first preview should install");
            thread::sleep(Duration::from_millis(10));
        };
        app.request_preview_promotion();
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Text));

        loop {
            app.drain_product_events();
            if let Some(identity) = app.state().library.selected_card
                && identity != first_identity
                && app.document_tab_for(identity).is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline, "second preview should install");
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.editor_runtime_tab_count(), 2);
        assert!(app.document_tab_for(first_identity).is_some());
    }

    #[test]
    fn first_content_change_promotes_the_active_preview_before_the_next_preview_opens() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        let first_identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "first preview should install");
            thread::sleep(Duration::from_millis(10));
        };
        let first_tab_id = app
            .document_tab_for(first_identity)
            .expect("first preview should have a registered tab");
        app.handle_editor_notification(
            &appkit_shell::editor_runtime::EditorNotification::ContentChanged {
                tab_id: first_tab_id,
                content_revision: 1,
            },
        );
        assert!(matches!(
            app.autosave.state(first_tab_id),
            Some(AutoSaveState::Scheduled { content_revision: 1, .. })
        ));
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Text));

        loop {
            app.drain_product_events();
            if let Some(identity) = app.state().library.selected_card
                && identity != first_identity
                && app.document_tab_for(identity).is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline, "second preview should install");
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.editor_runtime_tab_count(), 2);
        assert!(app.document_tab_for(first_identity).is_some());
    }

    #[test]
    fn open_external_path_switches_to_files_and_reuses_the_existing_tab() {
        let directory = tempfile::tempdir().expect("external file fixture directory should exist");
        let path = directory.path().join("outside.md");
        std::fs::write(&path, "# Outside").expect("external file fixture should be written");
        let mut app = app();

        app.receive_system_open_paths(vec![path.clone()]);
        app.receive_system_open_paths(vec![path]);
        assert_eq!(
            app.state().library.selected_card,
            None,
            "external path validation and reads must not complete on the caller thread"
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        let identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "external preview should install promptly");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            app.state().library.navigation_scope,
            notora_core::NavigationScope::ExternalFiles
        );
        assert!(matches!(identity, notora_core::DocumentIdentity::ExternalFile(_)));
        assert_eq!(app.editor_runtime_tab_count(), 1);
        assert!(app.document_tab_for(identity).is_some());
        assert_eq!(app.state().external_files.sessions().len(), 1);
        assert_eq!(app.state().library.last_command_error, None);
    }

    #[test]
    fn conflict_retry_captures_the_replacement_revision_off_the_caller_thread() {
        let directory = tempfile::tempdir().expect("external file fixture directory should exist");
        let path = directory.path().join("outside.md");
        std::fs::write(&path, "# Outside").expect("external file fixture should be written");
        let mut app = app();
        app.receive_system_open_paths(vec![path]);
        let deadline = Instant::now() + Duration::from_secs(2);
        let identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "external preview should install promptly");
            thread::sleep(Duration::from_millis(10));
        };
        let tab_id = app.document_tab_for(identity).expect("external file should have a tab");
        let initial_summary = app
            .editor_runtime
            .document_summary(tab_id)
            .expect("external tab should have a summary");
        let external_path =
            initial_summary.path.clone().expect("external tab should retain its path");
        let initial_disk_revision = initial_summary.disk_revision;
        std::fs::write(&external_path, "# Changed elsewhere")
            .expect("external fixture should change on disk");
        app.dispatch_action(NotoraAction::SaveConflictDetected {
            identity,
            content_revision: initial_summary.content_revision,
        });
        app.dispatch_action(NotoraAction::SaveConflictResolutionRequested(
            crate::action::ConflictResolution::RetrySave,
        ));

        assert_eq!(
            app.editor_runtime.document_summary(tab_id).and_then(|summary| summary.disk_revision),
            initial_disk_revision,
            "revision capture must not run on the conflict action caller"
        );
        loop {
            app.drain_product_events();
            if app
                .editor_runtime
                .document_summary(tab_id)
                .is_some_and(|summary| summary.disk_revision != initial_disk_revision)
            {
                break;
            }
            assert!(Instant::now() < deadline, "replacement revision should arrive promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn late_document_load_from_an_earlier_same_identity_selection_is_discarded() {
        let mut app = app();
        let selected_identity =
            notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        let intervening_identity =
            notora_core::DocumentIdentity::Note(notora_core::NoteId::generate());
        app.dispatch_action(NotoraAction::CardSelected(selected_identity));
        let outdated_generation = app.state().library.selected_document_generation;
        app.dispatch_action(NotoraAction::CardSelected(intervening_identity));
        app.dispatch_action(NotoraAction::CardSelected(selected_identity));
        let current_generation = app.state().library.selected_document_generation;

        app.install_loaded_preview(
            crate::action::DocumentLoadRequest {
                identity: selected_identity,
                selection_generation: outdated_generation,
            },
            LoadedDocument {
                path: std::path::PathBuf::from("older.md"),
                contents: "# Older".to_owned(),
                disk_revision: None,
            },
        );
        app.install_loaded_preview(
            crate::action::DocumentLoadRequest {
                identity: selected_identity,
                selection_generation: current_generation,
            },
            LoadedDocument {
                path: std::path::PathBuf::from("current.md"),
                contents: "# Current".to_owned(),
                disk_revision: None,
            },
        );

        assert_eq!(app.editor_runtime_tab_count(), 1);
        assert!(app.document_tab_for(selected_identity).is_some());
        assert_eq!(app.document_tab_for(intervening_identity), None);
    }

    #[test]
    fn editor_runtime_routes_text_markdown_and_mindmap_documents_to_product_plugins() {
        let app = app();

        assert_eq!(
            app.editor_runtime.create_plugin_for_path(std::path::Path::new("draft.txt")).name(),
            ui::plugin::PLUGIN_EDITOR
        );
        assert_eq!(
            app.editor_runtime.create_plugin_for_path(std::path::Path::new("draft.md")).name(),
            ui::plugin::PLUGIN_MARKDOWN_EDITOR
        );
        assert_eq!(
            app.editor_runtime.create_plugin_for_path(std::path::Path::new("draft.mmap.md")).name(),
            ui::plugin::PLUGIN_MINDMAP
        );
    }
}
