//! Notora 产品与编辑器运行时；平台应用只通过 `NotoraApp` 组合根访问。

#[path = "runtime/action_runtime.rs"]
mod action_runtime;
#[path = "app/deadline_coordinator.rs"]
mod deadline_coordinator;
#[path = "runtime/document_command_executor.rs"]
mod document_command_executor;
#[path = "app/document_completion_interpreter.rs"]
mod document_completion_interpreter;
#[path = "runtime/document_runtime.rs"]
mod document_runtime;
#[path = "runtime/frame_runtime.rs"]
mod frame_runtime;
#[path = "runtime/notora_effect_executor.rs"]
mod notora_effect_executor;
#[path = "app/persistence_completion_interpreter.rs"]
mod persistence_completion_interpreter;
#[path = "runtime/persistence_runtime.rs"]
mod persistence_runtime;
#[path = "app/product_event_coordinator.rs"]
mod product_event_coordinator;
#[path = "runtime/window_runtime.rs"]
mod window_runtime;
#[path = "app/workspace_completion_interpreter.rs"]
mod workspace_completion_interpreter;

use std::num::NonZeroUsize;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use appkit_shell::editor_runtime::{
    EditorNotification, EditorOutcome, EditorRuntime, EditorRuntimeConfig, EditorRuntimeError,
    EditorSurfacePaint, RenderError,
};
use appkit_shell::{DrainStart, ProductHost, ProductWakeHandle, ShellEffect, ShellEvent};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::WindowAttributes;

use crate::action::{
    CardQuery, ConflictResolution, DocumentLoadRequest, NoteCreationTarget, NotoraAction,
    SaveConflictRequest,
};
use crate::autosave::{AutoSaveRequest, AutoSaveScheduler};
use crate::dirty_snapshot::{collect_dirty_snapshots, write_dirty_snapshot};
use crate::editor_adapter::{LoadedDocument, build_editor_plugins, load_document};
use crate::effect_executor::{EffectExecutor, ExternalOpenRequest, ManualSaveRequest};
use crate::events;
use crate::external_files::{
    CanonicalExternalPath, ExternalFileSession, validate_external_text_file,
};
use crate::product::{
    DocumentCompletion, NotoraProduct, NotoraProductEvent, WorkspaceCompletion,
    WorkspaceEventScope, WorkspaceEventSender,
};
use crate::runtime_lru::RuntimeLru;
use crate::shell::layout::{ShellLayout, ShellLayoutInput};
use crate::shell_effect_executor::{ShellEffectExecutor, ShellEffectTarget};
use crate::{
    NotoraPaths, NotoraPathsError, NotoraState, WorkspaceCommand, WorkspaceCommandResult,
    WorkspaceController, WorkspaceControllerError,
};
use notora_core::note_command::{ConfiguredCreateNoteRequest, MoveNoteRequest, NoteCommand};
use notora_core::{DocumentIdentity, DocumentKind, NoteEncryption};

use self::action_runtime::{ActionRuntime, ExternalSaveAsApplication};
use self::deadline_coordinator::{DeadlineCoordinator, DeadlineSnapshot};
use self::document_command_executor::{DocumentCommandExecutor, DocumentCommandTarget};
use self::document_runtime::{
    DocumentOutcome, DocumentRuntime, DocumentSelection, PendingConflictRetry, TitleCommitContext,
};
#[cfg(test)]
use self::document_runtime::{PendingNoteMove, PendingTitleUpdate, PendingTrashMove};
use self::frame_runtime::{FrameInput, FrameRuntime, StartupTrace};
use self::notora_effect_executor::{NotoraEffectExecutor, NotoraEffectTarget};
use self::persistence_runtime::PersistenceRuntime;
#[cfg(test)]
use self::persistence_runtime::SettingsPersistenceState;
use self::product_event_coordinator::{
    DocumentCompletionTarget, LoadedDocumentTarget, PersistenceCompletionTarget,
    ProductActionTarget, ProductEventCoordinator, WorkspaceCompletionTarget,
};
use self::window_runtime::WindowRuntime;

const PRODUCT_WINDOW_TITLE: &str = "notora";
const SHUTDOWN_SAVE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_SAVE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_RUNTIME_TAB_LIMIT: usize = 12;

type WorkspaceDirectoryChooser = Box<dyn Fn() -> Option<std::path::PathBuf>>;

fn resolve_pointer_cursor(
    product_cursor: Option<winit::window::CursorIcon>,
    editor_cursor: Option<winit::window::CursorIcon>,
) -> winit::window::CursorIcon {
    product_cursor.or(editor_cursor).unwrap_or(winit::window::CursorIcon::Default)
}

fn choose_workspace_directory() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().set_title("设置工作区根目录").pick_folder()
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
            Self::Paths(error) => write!(formatter, "无法初始化 notora 路径：{error}"),
            Self::Runtime(error) => {
                write!(formatter, "无法初始化编辑器运行时：{error}")
            }
            Self::PersistenceWorker(error) => {
                write!(formatter, "无法初始化持久化线程：{error}")
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

/// 执行产品状态、后台服务和共享 editor runtime 协调的内部运行时。
pub(crate) struct NotoraRuntime {
    action_runtime: ActionRuntime,
    paths: NotoraPaths,
    persistence_runtime: PersistenceRuntime,
    document_runtime: DocumentRuntime,
    frame_runtime: FrameRuntime,
    product: NotoraProduct,
    workspace_controller: WorkspaceController,
    workspace_directory_chooser: WorkspaceDirectoryChooser,
    window_runtime: WindowRuntime,
}

struct RuntimeShellEffectTarget<'a> {
    document_runtime: &'a mut DocumentRuntime,
    persistence_runtime: &'a mut PersistenceRuntime,
    window_runtime: &'a mut WindowRuntime,
    settings_file: &'a std::path::Path,
}

impl NotoraRuntime {
    pub fn new() -> Self {
        Self::try_new()
            .expect("notora must construct its isolated configuration and editor runtime")
    }

    pub fn try_new() -> Result<Self, NotoraAppError> {
        let startup_trace = StartupTrace::from_environment();
        let paths = NotoraPaths::from_platform_directory().map_err(NotoraAppError::Paths)?;
        let mut app = Self::with_paths_and_startup_trace(paths, startup_trace)?;
        app.document_runtime
            .editor_mut()
            .start_gpu_preparation()
            .map_err(NotoraAppError::Runtime)?;
        app.start_font_system_preparation();
        Ok(app)
    }

    pub fn with_paths(paths: NotoraPaths) -> Result<Self, NotoraAppError> {
        Self::with_paths_and_startup_trace(paths, StartupTrace::from_environment())
    }

    fn with_paths_and_startup_trace(
        paths: NotoraPaths,
        startup_trace: Option<StartupTrace>,
    ) -> Result<Self, NotoraAppError> {
        let configuration_started_at = Instant::now();
        let loaded_product_settings = crate::settings::load_product_settings(&paths.settings_file);
        let loaded_session = crate::session::load_product_session(&paths.session_file);
        let product_settings = loaded_product_settings.settings;
        let mut settings = ui::Settings::new();
        product_settings.apply_to_ui(&mut settings);
        let theme = ui::Theme::resolve_builtin(
            product_settings.appearance.theme_mode,
            winit::window::Theme::Dark,
        );
        if let Some(trace) = &startup_trace {
            trace.record_stage("configuration_loaded", configuration_started_at);
        }
        let editor_runtime_started_at = Instant::now();
        let editor_runtime = build_editor_runtime(&settings, &theme, &paths)?;
        if let Some(trace) = &startup_trace {
            trace.record_stage("editor_runtime_constructed", editor_runtime_started_at);
        }
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
        let mut app = Self {
            action_runtime: ActionRuntime::new(state),
            paths,
            persistence_runtime: PersistenceRuntime::new(
                product_settings,
                loaded_session.session,
                persistence_worker,
            ),
            document_runtime: DocumentRuntime::new(
                RuntimeLru::new(runtime_tab_limit),
                AutoSaveScheduler::with_clock_and_idle_delay(
                    crate::autosave::SystemAutoSaveClock,
                    auto_save_delay,
                ),
                editor_runtime,
            ),
            frame_runtime: FrameRuntime::new(settings, theme, startup_trace),
            product,
            workspace_controller: WorkspaceController::with_catalog_backups_directory_and_retention(
                catalog_backups_directory,
                migration_backup_retention,
            ),
            workspace_directory_chooser: Box::new(choose_workspace_directory),
            window_runtime: WindowRuntime::new(),
        };
        app.synchronize_product_focus();
        app.frame_runtime.record_application_constructed();
        Ok(app)
    }

    pub fn editor_runtime_tab_count(&self) -> usize {
        self.document_runtime.editor().tab_count()
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
        self.action_runtime.state()
    }

    fn record_command_error(&mut self, message: String) {
        self.action_runtime.record_command_error(message);
    }

    fn take_due_search_request(
        &mut self,
        now: Instant,
    ) -> Option<crate::search_controller::SearchRequest> {
        self.action_runtime.take_due_search_request(now)
    }

    fn next_search_deadline(&self) -> Option<Instant> {
        self.action_runtime.next_search_deadline()
    }

    fn accepts_search_generation(
        &self,
        generation: crate::search_controller::SearchGeneration,
    ) -> bool {
        self.action_runtime.accepts_search_generation(generation)
    }

    pub fn document_tab_for(
        &self,
        identity: DocumentIdentity,
    ) -> Option<appkit_core::workspace::types::TabId> {
        self.document_runtime.tab_for(identity)
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
        let Some(tab_id) = self.document_runtime.editor_mut().active_tab_id() else {
            return;
        };
        let Some(request) = self.manual_save_request_for_tab(tab_id) else {
            return;
        };
        self.save_document_manually(request);
    }

    fn manual_save_request_for_tab(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<ManualSaveRequest> {
        let summary = self.document_runtime.editor().document_summary(tab_id)?;
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
            WorkspaceCommandResult::Opened(workspace) => {
                self.action_runtime
                    .set_active_workspace(workspace.descriptor.workspace_id, workspace.generation);
            }
            WorkspaceCommandResult::Closed { .. } => {
                self.action_runtime.clear_active_workspace();
            }
            WorkspaceCommandResult::Unchanged => {}
        }
        if !matches!(result, WorkspaceCommandResult::Unchanged) {
            self.document_runtime.reset_workspace_state();
        }
        if matches!(result, WorkspaceCommandResult::Opened(_)) {
            self.request_navigation_tree();
        }
        self.window_runtime.schedule_redraw();
        Ok(result)
    }

    pub fn shell_layout(&self) -> ShellLayout {
        let dpi = self.document_runtime.editor().scale_factor() as f32;
        let (window_width_px, window_height_px) = self.window_runtime.size();
        ShellLayout::compute(ShellLayoutInput {
            window_width_px,
            window_height_px,
            dpi,
            navigation_width_logical: self.action_runtime.state().layout.navigation_width_logical,
            card_list_width_logical: self.action_runtime.state().layout.card_list_width_logical,
            compact_content: self.action_runtime.state().layout.compact_content,
            compact_navigation: self.action_runtime.state().layout.compact_navigation,
        })
    }

    pub fn dispatch_action(&mut self, action: NotoraAction) {
        if self.action_will_leave_title_focus(&action)
            && let Some(title_commit) = self.title_commit_action()
        {
            self.action_runtime.enqueue(title_commit);
        }
        self.action_runtime.enqueue(action);
        if self.action_runtime.start_draining() == DrainStart::AlreadyDraining {
            return;
        }
        while let Some(action) = self.action_runtime.next_action() {
            self.reduce_action(action);
        }
        self.action_runtime.finish_draining();
    }

    fn reduce_action(&mut self, action: NotoraAction) {
        let should_persist_session = action_requires_session_persistence(&action);
        let reduction = self.action_runtime.reduce(action, Instant::now(), should_persist_session);
        for effect in reduction.effects {
            let execution = EffectExecutor::execute(effect, |effect| {
                NotoraEffectExecutor::execute(self, effect)
            });
            for follow_up_action in execution.follow_up_actions {
                self.action_runtime.enqueue(follow_up_action);
            }
            self.apply_shell_effect(execution.shell_effect);
        }
        self.synchronize_product_focus();
        for follow_up_action in reduction.follow_up_actions {
            self.action_runtime.enqueue(follow_up_action);
        }
        if reduction.should_persist_session {
            self.schedule_session_persistence();
        }
    }

    fn action_will_leave_title_focus(&self, action: &NotoraAction) -> bool {
        if self.action_runtime.state().layout.focus_target != crate::FocusTarget::EditorTitle
            || self.action_runtime.state().library.title_draft.is_none()
            || matches!(
                action,
                NotoraAction::TitleTextChanged(_) | NotoraAction::TitleCommitRequested(_)
            )
        {
            return false;
        }

        let selected_document = self.action_runtime.state().library.selected_card;
        let mut next_state = self.action_runtime.state().clone();
        let _ = next_state.reduce(action.clone());
        next_state.layout.focus_target != crate::FocusTarget::EditorTitle
            || next_state.library.selected_card != selected_document
    }

    fn synchronize_product_focus(&mut self) {
        let focus_target = self.action_runtime.state().layout.focus_target;
        let editor_is_active =
            focus_target == crate::FocusTarget::Editor && self.active_editor_matches_selection();
        self.document_runtime.editor_mut().set_active_cursor_paint_enabled(editor_is_active);
        self.frame_runtime.synchronize_focus(focus_target, Instant::now());
    }

    fn active_editor_matches_selection(&self) -> bool {
        let Some(selected_identity) = self.action_runtime.state().library.selected_card else {
            return false;
        };
        let Some(active_tab_id) = self.document_runtime.editor().active_tab_id() else {
            return false;
        };
        self.document_runtime.identity_for(active_tab_id) == Some(selected_identity)
    }

    fn editor_input_context(&self) -> appkit_shell::editor_runtime::EditorInputContext {
        let mut context = events::editor_input_context(
            self.action_runtime.state(),
            self.shell_layout(),
            self.window_runtime.is_focused(),
        );
        if !self.active_editor_matches_selection() {
            context.focus = appkit_shell::editor_runtime::EditorFocus::Inactive;
            context.modal_blocked = true;
        }
        context
    }

    pub fn update_editor_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) -> bool {
        let context = self.editor_input_context();
        self.document_runtime.editor_mut().update_preedit(context, text, cursor)
    }

    pub(crate) fn commit_editor_text(&mut self, text: String) {
        let context = self.editor_input_context();
        let outcome = self.document_runtime.editor_mut().commit_text(context, text);
        self.apply_editor_outcome(outcome);
    }

    pub(crate) fn handle_editor_key_input(
        &mut self,
        key: ui::KeyCode,
        modifiers: ui::core::Modifiers,
    ) {
        let context = self.editor_input_context();
        let outcome = self.document_runtime.editor_mut().handle_key_input(context, key, modifiers);
        self.apply_editor_outcome(outcome);
    }

    pub(crate) fn scroll_editor(&mut self, px: f32, py: f32, pixels: f32) {
        let context = self.editor_input_context();
        let outcome = self.document_runtime.editor_mut().scroll_editor(context, (px, py), pixels);
        self.apply_editor_outcome(outcome);
    }

    pub(crate) fn apply_canvas_viewport_action_at(
        &mut self,
        px: f32,
        py: f32,
        action: appkit_shell::canvas_viewport::CanvasViewportAction,
    ) -> bool {
        let context = self.editor_input_context();
        if context.modal_blocked || !context.editor_rect.contains(px, py) {
            return false;
        }
        let outcome =
            self.document_runtime.editor_mut().apply_active_canvas_viewport_action(action);
        let applied = outcome.shell_effect.redraw;
        self.apply_editor_outcome(outcome);
        applied
    }

    fn handle_editor_scrollbar_action(
        &mut self,
        action: ui::canvas_scrollbars::CanvasScrollbarsAction,
    ) {
        let editor_rect = self.shell_layout().editor_body_rect;
        let outcome =
            self.document_runtime.editor_mut().apply_active_scrollbar_action(action, editor_rect);
        self.apply_editor_outcome(outcome);
    }

    pub fn set_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<ShellEvent>) {
        self.window_runtime.set_event_loop_proxy(event_loop_proxy);
    }

    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        self.window_runtime.set_focused(focused, self.document_runtime.editor_mut());
    }

    pub(crate) fn set_window_size(&mut self, width: u32, height: u32) {
        self.window_runtime.set_size(width, height);
        self.action_runtime.set_responsive_mode(self.shell_layout().responsive_mode);
    }

    pub(crate) fn editor_runtime_mut(&mut self) -> &mut EditorRuntime {
        self.document_runtime.editor_mut()
    }

    pub(crate) fn take_redraw_request(&mut self) -> bool {
        let text_cursor_blink_due = self.frame_runtime.advance_text_cursor_blink(Instant::now());
        let editor_cursor_blink_phase = self.editor_cursor_blink_phase();
        let editor_runtime_requested = self.document_runtime.editor_mut().take_redraw_request();
        self.window_runtime.take_redraw_request(
            editor_runtime_requested,
            text_cursor_blink_due,
            editor_cursor_blink_phase,
        )
    }

    pub(crate) fn process_due_autosaves(&mut self) {
        for request in self.take_due_autosaves() {
            self.submit_autosave(request);
        }
    }

    fn take_due_autosaves(&mut self) -> Vec<AutoSaveRequest> {
        self.document_runtime.take_due_autosaves()
    }

    fn next_autosave_deadline(&self) -> Option<Instant> {
        self.document_runtime.next_autosave_deadline()
    }

    fn next_text_cursor_blink_at(&self) -> Option<Instant> {
        self.window_runtime
            .is_focused()
            .then(|| self.frame_runtime.next_text_cursor_blink_at())
            .flatten()
    }

    #[cfg(test)]
    pub(crate) fn process_due_session_persistence(&mut self) {
        self.process_due_session_persistence_at(Instant::now());
    }

    pub(crate) fn process_due_scheduled_work(&mut self) {
        let now = Instant::now();
        self.process_due_autosaves();
        if let Some(request) = self.take_due_search_request(now) {
            self.dispatch_action(NotoraAction::SearchCommitted {
                query: request.query,
                search_generation: Some(request.search_generation),
            });
        }
        self.process_due_session_persistence_at(now);
        if self.persistence_runtime.take_due_catalog_backup(now) {
            self.start_catalog_backup();
        }
    }

    pub(crate) fn next_deadline(&self) -> Option<std::time::Instant> {
        DeadlineCoordinator::next_deadline(DeadlineSnapshot {
            autosave: self.next_autosave_deadline(),
            search: self.next_search_deadline(),
            persistence: self.persistence_runtime.next_deadline(),
            text_cursor_blink: self.next_text_cursor_blink_at(),
            editor_cursor_blink: self
                .editor_cursor_blink_phase()
                .map(|phase| phase.next_transition_at),
        })
    }

    fn process_due_session_persistence_at(&mut self, now: Instant) {
        if !self.persistence_runtime.take_due_session_persistence(now) {
            return;
        }
        let session = self.capture_product_session();
        if let Err(error) =
            self.persistence_runtime.save_session(self.paths.session_file.clone(), session)
        {
            self.record_command_error(error.to_string());
        }
    }

    fn editor_cursor_blink_phase(
        &self,
    ) -> Option<appkit_shell::editor_runtime::EditorCursorBlinkPhase> {
        if self.action_runtime.state().layout.focus_target != crate::FocusTarget::Editor
            || !self.active_editor_matches_selection()
        {
            return None;
        }
        self.document_runtime.editor().active_cursor_blink_phase()
    }

    pub(crate) fn drain_runtime_save_completions(&mut self) {
        for outcome in self.document_runtime.drain_save_completions() {
            self.apply_document_outcome(outcome);
        }
    }

    pub(crate) fn request_window_redraw(&self) {
        self.window_runtime.request_window_redraw(self.document_runtime.editor());
    }

    pub(crate) fn route_pointer_event(&mut self, event: &ui::Event) -> bool {
        let (product_consumed, product_cursor) = self.route_product_event_with_feedback(event);
        let pointer_move = matches!(event, ui::Event::MouseMove { .. });
        let editor_cursor =
            if pointer_move || !product_consumed || self.editor_pointer_is_captured() {
                let context = self.editor_input_context();
                let outcome =
                    self.document_runtime.editor_mut().handle_pointer_event(context, event);
                let cursor_icon = outcome.cursor_icon;
                self.apply_editor_outcome(outcome.editor);
                cursor_icon
            } else {
                None
            };
        if pointer_move || product_cursor.is_some() || editor_cursor.is_some() {
            self.set_window_cursor(resolve_pointer_cursor(product_cursor, editor_cursor));
        }
        product_consumed
    }

    pub(crate) fn editor_pointer_is_captured(&self) -> bool {
        self.document_runtime.editor().pointer_capture()
            != appkit_shell::editor_runtime::MouseCapture::None
    }

    pub(crate) fn set_pointer_position(&mut self, px: f32, py: f32) {
        self.window_runtime.set_pointer_position(px, py);
    }

    pub(crate) fn pointer_position(&self) -> (f32, f32) {
        self.window_runtime.pointer_position()
    }

    pub(crate) fn set_scale_factor(&mut self, scale_factor: f64) {
        self.document_runtime.editor_mut().set_scale_factor(scale_factor);
        self.action_runtime.set_responsive_mode(self.shell_layout().responsive_mode);
        self.window_runtime.schedule_redraw();
    }

    fn current_system_appearance(&self) -> winit::window::Theme {
        self.document_runtime
            .editor()
            .window()
            .and_then(|window| window.theme())
            .unwrap_or(winit::window::Theme::Dark)
    }

    pub(crate) fn follows_system_theme(&self) -> bool {
        self.persistence_runtime.product_settings().appearance.theme_mode == ui::ThemeMode::System
    }

    pub(crate) fn rebuild_theme_for_system_appearance(
        &mut self,
        system_appearance: winit::window::Theme,
    ) {
        self.frame_runtime.rebuild_theme(
            self.persistence_runtime.product_settings().appearance.theme_mode,
            system_appearance,
        );
        self.document_runtime.editor_mut().update_theme(self.frame_runtime.theme().clone());
        self.window_runtime.schedule_redraw();
    }

    pub(crate) fn resume(&mut self, event_loop: &ActiveEventLoop) -> Result<(), NotoraAppError> {
        if self.document_runtime.editor_mut().window().is_some() {
            return Ok(());
        }
        let font_system_started_at = Instant::now();
        let font_system = Arc::new(Mutex::new(self.take_prepared_font_system()));
        self.frame_runtime.record_startup_stage("font_system_ready", font_system_started_at);
        self.document_runtime.editor_mut().set_shared_font_system(Arc::clone(&font_system));
        let editor_runtime_resume_started_at = Instant::now();
        let font_size = self.frame_runtime.settings().font_size;
        let font_family = self.frame_runtime.settings().font_family.clone();
        self.document_runtime
            .editor_mut()
            .resume(
                event_loop,
                WindowAttributes::default().with_title("notora").with_min_inner_size(
                    LogicalSize::new(
                        crate::shell::layout::MINIMUM_WINDOW_WIDTH_LOGICAL,
                        crate::shell::layout::MINIMUM_WINDOW_HEIGHT_LOGICAL,
                    ),
                ),
                font_system,
                font_size,
                &font_family,
            )
            .map_err(NotoraAppError::Runtime)?;
        self.frame_runtime
            .record_startup_stage("window_gpu_text_ready", editor_runtime_resume_started_at);
        self.rebuild_theme_for_system_appearance(self.current_system_appearance());
        if let Some((width, height)) = self.document_runtime.editor_mut().window().map(|window| {
            let geometry = self.persistence_runtime.pending_window_geometry();
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
        if let Some(event_loop_proxy) = self.window_runtime.event_loop_proxy() {
            ProductHost::start_background_services(
                &mut self.product,
                ProductWakeHandle::new(event_loop_proxy),
            );
        }
        self.window_runtime.schedule_redraw();
        Ok(())
    }

    fn start_font_system_preparation(&mut self) {
        self.frame_runtime.start_font_system_preparation(&self.paths.config_directory);
    }

    fn take_prepared_font_system(&mut self) -> shaping::FontSystem {
        self.frame_runtime.take_prepared_font_system(&self.paths.config_directory)
    }

    pub(crate) fn record_first_frame_visible(&mut self) {
        self.frame_runtime.record_first_frame_visible();
    }

    pub(crate) fn resize_window(&mut self, width: u32, height: u32) {
        self.set_window_size(width, height);
        let _ = self.document_runtime.editor_mut().resize_now(width, height);
        self.window_runtime.schedule_redraw();
    }

    /// 处理后台产品结果；无事件循环的宿主可主动轮询此入口。
    pub fn drain_product_events(&mut self) {
        let completions = ProductEventCoordinator::drain(&mut self.product);
        for event in completions.events {
            ProductEventCoordinator::apply(self, event);
        }
        self.apply_shell_effect(completions.shell_effect);
    }

    pub(crate) fn route_product_event(&mut self, event: &ui::Event) -> bool {
        let (consumed, cursor_hint) = self.route_product_event_with_feedback(event);
        if let Some(cursor_hint) = cursor_hint {
            self.set_window_cursor(cursor_hint);
        }
        consumed
    }

    fn route_product_event_with_feedback(
        &mut self,
        event: &ui::Event,
    ) -> (bool, Option<winit::window::CursorIcon>) {
        let focus_target = self.action_runtime.state().layout.focus_target;
        let product_modal_is_open =
            self.action_runtime.state().layout.overlay != crate::state::OverlayState::None;
        if !product_modal_is_open
            && focus_target == crate::FocusTarget::EditorTitle
            && matches!(event, ui::Event::KeyDown(ui::KeyCode::Tab, _))
        {
            self.commit_title_before_focus();
            self.dispatch_action(NotoraAction::FocusRequested(crate::FocusTarget::Editor));
            return (true, None);
        }
        let route = self.frame_runtime.route_product_event(
            event,
            focus_target,
            self.action_runtime.state().layout.overlay,
            self.document_runtime.editor_mut().scale_factor() as f32,
        );
        let cursor_hint = route.cursor_hint;
        if route.consumed {
            self.apply_shell_effect(ShellEffect::REDRAW);
        }
        if let Some(action) = route.canvas_scrollbar_action {
            self.handle_editor_scrollbar_action(action);
        }
        for action in route.actions {
            self.dispatch_action(action);
        }
        (route.consumed, cursor_hint)
    }

    fn commit_title_before_focus(&mut self) {
        let Some(action) = self.title_commit_action() else {
            return;
        };
        self.dispatch_action(action);
    }

    fn title_commit_action(&self) -> Option<NotoraAction> {
        if !matches!(
            self.action_runtime.state().library.selected_card,
            Some(DocumentIdentity::Note(_))
        ) {
            return None;
        }
        Some(NotoraAction::TitleCommitRequested(self.frame_runtime.editor_title_text().to_owned()))
    }

    fn set_window_cursor(&self, cursor_icon: winit::window::CursorIcon) {
        self.window_runtime.set_cursor(self.document_runtime.editor(), cursor_icon);
    }

    pub(crate) fn render(&mut self) -> Result<(), RenderError> {
        let _ = self.render_frame()?;
        self.frame_runtime
            .update_focused_ime_cursor_area(&self.document_runtime, self.action_runtime.state());
        Ok(())
    }

    fn render_frame(&mut self) -> Result<EditorSurfacePaint, RenderError> {
        self.window_runtime.mark_frame_rendered();
        let layout = self.shell_layout();
        let editor_is_active = self.active_editor_matches_selection();
        let (window_width_px, window_height_px) = self.window_runtime.size();
        let input = FrameInput {
            state: self.action_runtime.state(),
            product_settings: self.persistence_runtime.product_settings(),
            persistence_view: self.persistence_runtime.persistence_view(),
            layout,
            window_width_px,
            window_height_px,
            editor_is_active,
        };
        self.frame_runtime.render_frame(&mut self.document_runtime, input)
    }

    #[cfg(test)]
    fn editor_save_status(
        state: Option<crate::autosave::AutoSaveState>,
        dirty: bool,
        failure_message: Option<&str>,
    ) -> String {
        FrameRuntime::editor_save_status(state, dirty, failure_message)
    }

    pub(crate) fn shutdown(&mut self) {
        self.finish_saves_and_snapshot_dirty_documents();
        self.flush_pending_catalog_backup();
        let final_session = self.capture_product_session();
        if let Err(error) =
            self.persistence_runtime.save_session(self.paths.session_file.clone(), final_session)
        {
            self.action_runtime.record_command_error(error.to_string());
        }
        if let Err(error) = self.persistence_runtime.save_settings(self.paths.settings_file.clone())
        {
            self.action_runtime.record_command_error(error.to_string());
        }
        self.persistence_runtime.shutdown();
        ProductHost::shutdown(&mut self.product);
        self.document_runtime.editor_mut().shutdown();
    }

    fn finish_saves_and_snapshot_dirty_documents(&mut self) {
        self.process_due_autosaves();
        let deadline = Instant::now() + SHUTDOWN_SAVE_DRAIN_TIMEOUT;
        while self.document_runtime.has_in_flight_save() && Instant::now() < deadline {
            self.drain_runtime_save_completions();
            if self.document_runtime.has_in_flight_save() {
                thread::sleep(SHUTDOWN_SAVE_DRAIN_POLL_INTERVAL);
            }
        }
        self.drain_runtime_save_completions();
        self.write_dirty_snapshots_in_background();
    }

    fn write_dirty_snapshots_in_background(&self) {
        let plans = collect_dirty_snapshots(&self.document_runtime.editor().workspace_snapshot());
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
        let mut target = RuntimeShellEffectTarget {
            document_runtime: &mut self.document_runtime,
            persistence_runtime: &mut self.persistence_runtime,
            window_runtime: &mut self.window_runtime,
            settings_file: &self.paths.settings_file,
        };
        ShellEffectExecutor::execute(&mut target, effect);
    }

    fn install_loaded_preview(&mut self, request: DocumentLoadRequest, document: LoadedDocument) {
        let selection = self.document_selection();
        let outcome = self.document_runtime.install_loaded_preview(request, document, selection);
        self.apply_document_outcome(outcome);
    }

    fn promote_active_preview_tab(&mut self) {
        let outcome = self.document_runtime.promote_active_preview();
        self.apply_document_outcome(outcome);
    }

    fn evict_excess_runtime_tabs(&mut self) {
        self.document_runtime.evict_excess_runtime_tabs();
    }

    fn synchronize_open_note_path(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    ) {
        let Some(previous_relative_path) = result.previous_relative_path.as_deref() else {
            return;
        };
        let identity = DocumentIdentity::Note(result.note.note_id);
        let Some(tab_id) = self.document_runtime.tab_for(identity) else {
            return;
        };
        let Some(workspace) = self.workspace_controller.active_workspace() else {
            return;
        };
        let previous_path = workspace.descriptor.root.join(previous_relative_path);
        let Some(summary) = self.document_runtime.editor_mut().document_summary(tab_id) else {
            return;
        };
        if summary.path.as_deref() != Some(previous_path.as_path()) {
            return;
        }
        let next_path = workspace.descriptor.root.join(&result.note.relative_path);
        let disk_revision = match appkit_core::file_safety::capture_revision(&next_path) {
            Ok(revision) => Some(revision),
            Err(error) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(format!(
                    "文件已移动，但无法读取新路径状态：{error}"
                )));
                None
            }
        };
        if self.document_runtime.editor_mut().update_document_path(tab_id, next_path, disk_revision)
        {
            self.document_runtime.editor_mut().request_file_safety_check_now(Instant::now());
            self.window_runtime.request_redraw(self.document_runtime.editor_mut());
        }
    }

    fn synchronize_external_note_relocations(
        &mut self,
        relocations: Vec<crate::product::WorkspaceNoteRelocation>,
    ) {
        let Some(workspace_root) = self
            .workspace_controller
            .active_workspace()
            .map(|workspace| workspace.descriptor.root.clone())
        else {
            return;
        };
        for relocation in relocations {
            let identity = DocumentIdentity::Note(relocation.note_id);
            let Some(tab_id) = self.document_runtime.tab_for(identity) else {
                continue;
            };
            let previous_path = workspace_root.join(&relocation.from);
            let Some(summary) = self.document_runtime.editor_mut().document_summary(tab_id) else {
                continue;
            };
            if summary.path.as_deref() != Some(previous_path.as_path()) {
                continue;
            }
            let next_path = workspace_root.join(&relocation.to);
            let disk_revision = match appkit_core::file_safety::capture_revision(&next_path) {
                Ok(revision) => revision,
                Err(error) => {
                    self.dispatch_action(NotoraAction::NoteCommandFailed(format!(
                        "Finder 已重命名文件，但无法读取新路径状态：{error}"
                    )));
                    continue;
                }
            };
            if !self.document_runtime.editor_mut().update_document_path(
                tab_id,
                next_path,
                Some(disk_revision),
            ) {
                continue;
            }
            self.document_runtime.editor_mut().request_file_safety_check_now(Instant::now());
            if self.action_runtime.state().library.selected_card == Some(identity) {
                self.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
                    request: crate::action::DocumentLoadRequest {
                        identity,
                        selection_generation: self
                            .action_runtime
                            .state()
                            .library
                            .selected_document_generation,
                    },
                    metadata: relocation.metadata,
                    tags: relocation.tags,
                });
            }
            self.window_runtime.request_redraw(self.document_runtime.editor_mut());
        }
    }

    fn promote_preview_for_tab(&mut self, tab_id: appkit_core::workspace::types::TabId) {
        let outcome = self.document_runtime.promote_preview_for_tab(tab_id);
        self.apply_document_outcome(outcome);
    }

    fn document_selection(&self) -> DocumentSelection {
        let library = &self.action_runtime.state().library;
        let editing_access = if library.navigation_scope == notora_core::NavigationScope::Trash {
            appkit_shell::tab_runtime::DocumentEditingAccess::ReadOnly
        } else {
            appkit_shell::tab_runtime::DocumentEditingAccess::Editable
        };
        DocumentSelection {
            identity: library.selected_card,
            generation: library.selected_document_generation,
            editing_access,
        }
    }

    fn apply_document_outcome(&mut self, outcome: DocumentOutcome) {
        for notification in &outcome.notifications {
            self.handle_editor_notification(notification);
        }
        for action in outcome.actions {
            self.dispatch_action(action);
        }
        for command in outcome.commands {
            DocumentCommandExecutor::execute(self, command);
        }
        self.apply_shell_effect(outcome.shell_effect);
        self.window_runtime.merge_redraw_request(outcome.needs_redraw);
    }

    fn apply_editor_outcome(&mut self, outcome: EditorOutcome) {
        for notification in &outcome.notifications {
            self.handle_editor_notification(notification);
        }
        self.apply_shell_effect(outcome.shell_effect);
    }

    fn selection_matches(&self, request: DocumentLoadRequest) -> bool {
        DocumentRuntime::selection_matches(request, self.document_selection())
    }

    pub(crate) fn handle_editor_notification(&mut self, notification: &EditorNotification) {
        match notification {
            EditorNotification::ActiveDocumentChanged { tab_id: Some(tab_id) } => {
                self.document_runtime.touch_tab(*tab_id);
            }
            EditorNotification::ContentChanged { tab_id, content_revision } => {
                self.document_runtime.clear_save_failure(*tab_id);
                self.promote_preview_for_tab(*tab_id);
                if let Some(origin) = self.document_origin_for_tab(*tab_id) {
                    self.document_runtime.schedule_autosave(&origin, *tab_id, *content_revision);
                }
            }
            EditorNotification::SaveCompleted { tab_id, content_revision } => {
                if let Some(identity) = self.document_runtime.identity_for(*tab_id) {
                    self.dispatch_action(NotoraAction::ActiveEditorSaved {
                        identity,
                        saved_at: SystemTime::now(),
                    });
                }
                self.submit_document_title_initialization(*tab_id, *content_revision);
            }
            EditorNotification::ActiveDocumentChanged { tab_id: None }
            | EditorNotification::PathChanged { .. }
            | EditorNotification::DirtyChanged { .. }
            | EditorNotification::SaveFailed { .. }
            | EditorNotification::CloseRequested { .. } => {}
        }
    }

    fn submit_autosave(&mut self, request: AutoSaveRequest) {
        let event_loop_proxy = self.window_runtime.event_loop_proxy();
        let outcome = self.document_runtime.submit_autosave(request, event_loop_proxy);
        self.apply_document_outcome(outcome);
    }

    #[cfg(test)]
    fn record_autosave_failure(&mut self, request: AutoSaveRequest, message: String) {
        let outcome = self.document_runtime.record_autosave_failure(request, message);
        self.apply_document_outcome(outcome);
    }

    #[cfg(test)]
    fn pending_trash_move_has_current_saved_document(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
        pending_trash_move: PendingTrashMove,
    ) -> bool {
        self.document_runtime
            .pending_trash_move_has_current_saved_document(tab_id, pending_trash_move)
    }

    #[cfg(test)]
    fn pending_note_move_has_current_saved_document(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
        pending_note_move: &PendingNoteMove,
    ) -> bool {
        self.document_runtime
            .pending_note_move_has_current_saved_document(tab_id, pending_note_move)
    }

    #[cfg(test)]
    fn pending_title_update_has_current_saved_document(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
        pending_title_update: &PendingTitleUpdate,
    ) -> bool {
        self.document_runtime
            .pending_title_update_has_current_saved_document(tab_id, pending_title_update)
    }

    fn save_untitled_external_file(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
    ) {
        let Some(path) = rfd::FileDialog::new().add_filter("文本文档", &["txt", "md"]).save_file()
        else {
            return;
        };
        let outcome = self.document_runtime.save_external_file_as_to_path(
            tab_id,
            external_file_id,
            path,
            self.window_runtime.event_loop_proxy(),
        );
        self.apply_document_outcome(outcome);
    }

    fn complete_pending_external_save_as(
        &mut self,
        request: AutoSaveRequest,
        save_succeeded: bool,
        saved_path: Option<std::path::PathBuf>,
    ) {
        let outcome = self.document_runtime.complete_pending_external_save_as(
            request,
            save_succeeded,
            saved_path,
        );
        self.apply_document_outcome(outcome);
    }

    fn start_external_save_as_canonicalization(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        saved_path: std::path::PathBuf,
    ) {
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-external-save-as-canonicalize".to_owned())
            .spawn(move || {
                let result = CanonicalExternalPath::canonicalize(&saved_path)
                    .map_err(|error| error.to_string());
                let _ = sender.send(NotoraProductEvent::Document(
                    DocumentCompletion::ExternalSaveAsCanonicalized {
                        tab_id,
                        external_file_id,
                        content_revision,
                        result,
                    },
                ));
            })
            .is_err()
        {
            let outcome = self.document_runtime.complete_external_save_as_canonicalization(
                tab_id,
                external_file_id,
                content_revision,
                Err("无法启动外部文件另存为路径处理线程".to_owned()),
            );
            self.apply_document_outcome(outcome);
        }
    }

    fn complete_external_save_as_canonicalization(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        result: Result<CanonicalExternalPath, String>,
    ) {
        let outcome = self.document_runtime.complete_external_save_as_canonicalization(
            tab_id,
            external_file_id,
            content_revision,
            result,
        );
        self.apply_document_outcome(outcome);
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
            self.document_runtime.record_catalog_reconciliation(true);
        }
    }

    fn document_origin_for_tab(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
    ) -> Option<notora_core::DocumentOrigin> {
        let identity = self.document_runtime.identity_for(tab_id)?;
        match identity {
            DocumentIdentity::Note(note_id) => {
                let workspace = self.workspace_controller.active_workspace()?;
                let path = self.document_runtime.editor().document_summary(tab_id)?.path?;
                let relative_path =
                    path.strip_prefix(&workspace.descriptor.root).ok()?.to_path_buf();
                Some(notora_core::DocumentOrigin::Note {
                    workspace_id: workspace.descriptor.workspace_id,
                    note_id,
                    relative_path,
                })
            }
            DocumentIdentity::ExternalFile(external_file_id) => {
                match self.action_runtime.state().external_files.session(external_file_id)? {
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

    fn commit_active_note_title(&mut self, title: String) {
        let library = &self.action_runtime.state().library;
        let context = TitleCommitContext {
            selected_identity: library.selected_card,
            editable_workspace_note: !matches!(
                library.navigation_scope,
                notora_core::NavigationScope::Trash | notora_core::NavigationScope::ExternalFiles
            ),
            metadata: library
                .active_editor_metadata
                .as_ref()
                .filter(|metadata| Some(metadata.identity) == library.selected_card)
                .map(|metadata| metadata.metadata.clone()),
        };
        let outcome = self.document_runtime.commit_active_note_title(title, context);
        self.apply_document_outcome(outcome);
    }

    fn submit_document_title_initialization(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        saved_content_revision: u64,
    ) {
        let identity = self.document_runtime.identity_for(tab_id);
        let initialization = self
            .action_runtime
            .state()
            .library
            .active_editor_metadata
            .as_ref()
            .filter(|metadata| Some(metadata.identity) == identity)
            .map(|metadata| metadata.metadata.title_initialization);
        let outcome = self.document_runtime.initialize_title_after_save(
            tab_id,
            saved_content_revision,
            initialization,
        );
        self.apply_document_outcome(outcome);
    }

    fn apply_title_initialization_outcome(
        &mut self,
        mutation: &crate::action::MetadataMutation,
        outcome: crate::action::MetadataMutationOutcome,
        note_id: notora_core::NoteId,
        title_revision: u64,
    ) {
        let document_outcome = self.document_runtime.apply_title_initialization_outcome(
            mutation,
            outcome,
            note_id,
            title_revision,
        );
        self.apply_document_outcome(document_outcome);
    }

    fn complete_pending_title_seed(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    ) {
        let outcome = self.document_runtime.complete_pending_title_seed(result);
        self.apply_document_outcome(outcome);
    }
}

fn action_requires_session_persistence(action: &NotoraAction) -> bool {
    matches!(
        action,
        NotoraAction::NavigationSelected(_)
            | NotoraAction::NavigationExpansionToggled(_)
            | NotoraAction::CardSelected(_)
            | NotoraAction::CardActivated(_)
            | NotoraAction::NoteCommandCompleted(_)
            | NotoraAction::CompactNavigationRequested
            | NotoraAction::CompactBackRequested
            | NotoraAction::ExternalFileOpened(_)
            | NotoraAction::SplitterDragged { .. }
    )
}

impl NotoraEffectTarget for NotoraRuntime {
    fn query_cards(&mut self, query: CardQuery) -> Vec<NotoraAction> {
        NotoraRuntime::query_cards(self, query)
    }

    fn request_note_creation(
        &mut self,
        kind: DocumentKind,
        target: NoteCreationTarget,
    ) -> Vec<NotoraAction> {
        NotoraRuntime::request_note_creation(self, kind, target)
    }

    fn execute_note_command(&mut self, command: NoteCommand) {
        NotoraRuntime::execute_note_command(self, command);
    }

    fn commit_title(&mut self, title: String) {
        NotoraRuntime::commit_title(self, title);
    }

    fn toggle_editor_view(&mut self) {
        NotoraRuntime::toggle_editor_view(self);
    }

    fn toggle_mindmap_style_panel(&mut self) {
        NotoraRuntime::toggle_mindmap_style_panel(self);
    }

    fn dispatch_mindmap_style_panel(&mut self, action: ui::core::widget::MindmapStylePanelAction) {
        NotoraRuntime::dispatch_mindmap_style_panel(self, action);
    }

    fn execute_semantic_edit(&mut self, command: ui::plugin::SemanticEditCommand) {
        NotoraRuntime::execute_semantic_edit(self, command);
    }

    fn execute_metadata_mutation(
        &mut self,
        mutation: crate::action::MetadataMutation,
    ) -> Vec<NotoraAction> {
        NotoraRuntime::execute_metadata_mutation(self, mutation)
    }

    fn execute_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        NotoraRuntime::execute_trash_operation(self, operation);
    }

    fn choose_note_move_directory(&mut self, note_id: notora_core::NoteId) -> Vec<NotoraAction> {
        NotoraRuntime::choose_note_move_directory(self, note_id)
    }

    fn prepare_document(&mut self, request: DocumentLoadRequest) -> Vec<NotoraAction> {
        NotoraRuntime::prepare_document(self, request)
    }

    fn promote_active_preview(&mut self) {
        NotoraRuntime::promote_active_preview(self);
    }

    fn choose_workspace_root(&mut self) {
        NotoraRuntime::choose_workspace_root(self);
    }

    fn open_external_files(&mut self, request: ExternalOpenRequest) {
        NotoraRuntime::open_external_files(self, request);
    }

    fn create_untitled_external(&mut self, kind: DocumentKind) -> Vec<NotoraAction> {
        NotoraRuntime::create_untitled_external(self, kind)
    }

    fn resolve_save_conflict(&mut self, request: SaveConflictRequest) {
        NotoraRuntime::resolve_save_conflict(self, request);
    }

    fn apply_product_settings_update(
        &mut self,
        update: crate::settings_overlay::ProductSettingsUpdate,
    ) {
        NotoraRuntime::apply_product_settings_update(self, update);
    }

    fn persist_product_settings(&mut self) {
        NotoraRuntime::persist_product_settings(self);
    }

    fn persist_layout(&mut self) {
        NotoraRuntime::persist_layout(self);
    }
}

impl DocumentCommandTarget for NotoraRuntime {
    fn execute_note_command(&mut self, command: NoteCommand) {
        self.submit_note_command(command);
    }

    fn execute_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        self.submit_trash_operation(operation);
    }

    fn retry_title_update(&mut self, request: notora_core::UpdateNoteTitleRequest) {
        self.submit_or_defer_title_update(request);
    }

    fn request_catalog_reindex(&mut self, tab_id: appkit_core::workspace::types::TabId) {
        self.request_catalog_reindex_after_note_save(tab_id);
    }

    fn complete_external_save_as(
        &mut self,
        request: AutoSaveRequest,
        save_succeeded: bool,
        saved_path: Option<std::path::PathBuf>,
    ) {
        self.complete_pending_external_save_as(request, save_succeeded, saved_path);
    }

    fn choose_external_save_path(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
    ) {
        self.save_untitled_external_file(tab_id, external_file_id);
    }

    fn canonicalize_external_save_as(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        saved_path: std::path::PathBuf,
    ) {
        self.start_external_save_as_canonicalization(
            tab_id,
            external_file_id,
            content_revision,
            saved_path,
        );
    }

    fn apply_external_save_as(
        &mut self,
        external_file_id: notora_core::ExternalFileId,
        canonical_path: CanonicalExternalPath,
    ) -> ExternalSaveAsApplication {
        self.action_runtime.apply_external_save_as(external_file_id, canonical_path)
    }

    fn dispatch_action(&mut self, action: NotoraAction) {
        NotoraRuntime::dispatch_action(self, action);
    }

    fn process_due_autosaves(&mut self) {
        NotoraRuntime::process_due_autosaves(self);
    }

    fn execute_metadata_mutation(
        &mut self,
        mutation: crate::action::MetadataMutation,
    ) -> Vec<NotoraAction> {
        NotoraRuntime::execute_metadata_mutation(self, mutation)
    }

    fn capture_conflict_revision(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    ) {
        self.start_conflict_retry_revision_capture(identity, tab_id, content_revision, path);
    }

    fn begin_conflict_retry(
        &mut self,
        request: ManualSaveRequest,
        pending: PendingConflictRetry,
    ) -> DocumentOutcome {
        let origin = match request {
            ManualSaveRequest::Note { tab_id, .. } => self.document_origin_for_tab(tab_id),
            ManualSaveRequest::ExistingExternalFile { .. }
            | ManualSaveRequest::UntitledExternalFile { .. } => None,
        };
        self.document_runtime.begin_conflict_retry(
            request,
            pending,
            origin,
            self.window_runtime.event_loop_proxy(),
        )
    }

    fn apply_document_outcome(&mut self, outcome: DocumentOutcome) {
        NotoraRuntime::apply_document_outcome(self, outcome);
    }

    fn save_conflict_copy(
        &mut self,
        identity: DocumentIdentity,
        prepared: appkit_shell::editor_runtime::PreparedDocumentSave,
    ) {
        self.start_conflict_copy(identity, prepared);
    }

    fn reload_conflict(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    ) {
        self.start_conflict_reload(identity, tab_id, content_revision, path);
    }

    fn read_external_files(&mut self, requests: Vec<(std::path::PathBuf, bool)>) {
        self.start_external_file_reads(requests);
    }

    fn load_external_document(
        &mut self,
        request: DocumentLoadRequest,
        canonical_path: CanonicalExternalPath,
    ) {
        self.start_external_document_load(request, canonical_path);
    }
}

impl ProductActionTarget for NotoraRuntime {
    fn dispatch_action(&mut self, action: NotoraAction) {
        NotoraRuntime::dispatch_action(self, action);
    }
}

impl WorkspaceCompletionTarget for NotoraRuntime {
    fn synchronize_open_note_path(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    ) {
        NotoraRuntime::synchronize_open_note_path(self, result);
    }

    fn complete_pending_title_seed(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    ) {
        NotoraRuntime::complete_pending_title_seed(self, result);
    }

    fn complete_metadata_mutation(
        &mut self,
        mutation: &crate::action::MetadataMutation,
        note_id: notora_core::NoteId,
    ) -> Option<u64> {
        self.document_runtime.complete_metadata_mutation(mutation, note_id)
    }

    fn apply_title_initialization_outcome(
        &mut self,
        mutation: &crate::action::MetadataMutation,
        outcome: crate::action::MetadataMutationOutcome,
        note_id: notora_core::NoteId,
        title_revision: u64,
    ) {
        NotoraRuntime::apply_title_initialization_outcome(
            self,
            mutation,
            outcome,
            note_id,
            title_revision,
        );
    }

    fn selected_document(&self) -> (Option<DocumentIdentity>, u64) {
        (self.state().library.selected_card, self.state().library.selected_document_generation)
    }

    fn schedule_catalog_backup(&mut self) {
        NotoraRuntime::schedule_catalog_backup(self);
    }

    fn request_navigation_tree(&mut self) {
        NotoraRuntime::request_navigation_tree(self);
    }

    fn complete_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        NotoraRuntime::complete_trash_operation(self, operation);
    }

    fn record_catalog_reconciliation(&mut self, pending: bool) {
        self.document_runtime.record_catalog_reconciliation(pending);
    }

    fn accepts_search_generation(
        &self,
        generation: crate::search_controller::SearchGeneration,
    ) -> bool {
        NotoraRuntime::accepts_search_generation(self, generation)
    }

    fn synchronize_external_note_relocations(
        &mut self,
        relocations: Vec<crate::product::WorkspaceNoteRelocation>,
    ) {
        NotoraRuntime::synchronize_external_note_relocations(self, relocations);
    }
}

impl LoadedDocumentTarget for NotoraRuntime {
    fn install_loaded_preview(&mut self, request: DocumentLoadRequest, document: LoadedDocument) {
        NotoraRuntime::install_loaded_preview(self, request, document);
    }

    fn selection_matches(&self, request: DocumentLoadRequest) -> bool {
        NotoraRuntime::selection_matches(self, request)
    }
}

impl DocumentCompletionTarget for NotoraRuntime {
    fn complete_external_file_open(
        &mut self,
        canonical_path: CanonicalExternalPath,
        document: LoadedDocument,
        activate: bool,
    ) {
        NotoraRuntime::complete_external_file_open(self, canonical_path, document, activate);
    }

    fn complete_external_save_as_canonicalization(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        external_file_id: notora_core::ExternalFileId,
        content_revision: u64,
        result: Result<CanonicalExternalPath, String>,
    ) {
        NotoraRuntime::complete_external_save_as_canonicalization(
            self,
            tab_id,
            external_file_id,
            content_revision,
            result,
        );
    }

    fn complete_conflict_reload(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        document: LoadedDocument,
    ) {
        NotoraRuntime::complete_conflict_reload(self, identity, tab_id, content_revision, document);
    }

    fn active_save_conflict_identity(&self) -> Option<DocumentIdentity> {
        self.state().library.save_conflict.map(|conflict| conflict.identity)
    }

    fn complete_conflict_retry_revision_capture(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
        disk_revision: appkit_core::file_safety::DiskRevision,
    ) {
        NotoraRuntime::complete_conflict_retry_revision_capture(
            self,
            identity,
            tab_id,
            content_revision,
            path,
            disk_revision,
        );
    }
}

impl PersistenceCompletionTarget for NotoraRuntime {
    fn record_settings_persistence_result(&mut self, result: Result<(), String>) {
        NotoraRuntime::record_settings_persistence_result(self, result);
    }
}

impl Default for NotoraRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellEffectTarget for RuntimeShellEffectTarget<'_> {
    fn invalidate_reshape(&mut self) {
        self.document_runtime.editor_mut().invalidate_reshape();
    }

    fn update_window_title(&mut self) {
        self.window_runtime.update_title(self.document_runtime.editor(), PRODUCT_WINDOW_TITLE);
    }

    fn persist_settings(&mut self) {
        if let Err(error) = self.persistence_runtime.save_settings(self.settings_file.to_path_buf())
        {
            self.persistence_runtime.record_settings_result(Err(error.to_string()));
            self.window_runtime.schedule_redraw();
        }
    }

    fn persist_workspace(&mut self) {
        self.persistence_runtime.schedule_session_persistence(Instant::now());
    }

    fn request_redraw(&mut self) {
        self.window_runtime.request_redraw(self.document_runtime.editor_mut());
    }
}

#[cfg(test)]
mod session_restore_tests {
    use std::time::Instant;

    use super::{NotoraRuntime, action_requires_session_persistence};
    use crate::action::NotoraAction;
    use crate::autosave::AutoSaveState;

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

    #[test]
    fn editor_save_status_is_derived_from_typed_autosave_state() {
        let deadline = Instant::now();
        assert_eq!(
            NotoraRuntime::editor_save_status(
                Some(AutoSaveState::Saving { content_revision: 3 }),
                true,
                None,
            ),
            "保存中"
        );
        assert_eq!(
            NotoraRuntime::editor_save_status(
                Some(AutoSaveState::Failed { content_revision: 3 }),
                true,
                None,
            ),
            "保存失败"
        );
        assert_eq!(
            NotoraRuntime::editor_save_status(
                Some(AutoSaveState::Scheduled { deadline, content_revision: 3 }),
                true,
                None,
            ),
            "待保存"
        );
        assert_eq!(NotoraRuntime::editor_save_status(None, true, None), "未保存");
        assert_eq!(NotoraRuntime::editor_save_status(None, false, None), "已保存");
    }

    #[test]
    fn editor_save_status_includes_the_failure_reason() {
        assert_eq!(
            NotoraRuntime::editor_save_status(
                Some(AutoSaveState::Failed { content_revision: 3 }),
                true,
                Some("file is read-only"),
            ),
            "保存失败：file is read-only"
        );
    }
}

impl NotoraRuntime {
    fn query_cards(&mut self, query: CardQuery) -> Vec<NotoraAction> {
        self.workspace_controller.query_cards(query.clone()).err().map_or_else(Vec::new, |error| {
            vec![NotoraAction::CardQueryFailed { query, message: error.to_string() }]
        })
    }

    fn request_note_creation(
        &mut self,
        kind: DocumentKind,
        target: NoteCreationTarget,
    ) -> Vec<NotoraAction> {
        if self.workspace_controller.active_workspace().is_none() {
            return vec![NotoraAction::NoteCommandFailed("请先设置工作区根目录".to_owned())];
        }
        self.submit_note_command(NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
            kind,
            target_directory: target.directory,
            encryption: NoteEncryption::Unencrypted,
        }));
        Vec::new()
    }

    fn choose_workspace_root(&mut self) {
        self.select_workspace_root();
    }

    fn execute_note_command(&mut self, command: notora_core::note_command::NoteCommand) {
        if let NoteCommand::Move(request) = command {
            self.submit_or_defer_note_move(request);
            return;
        }
        self.submit_note_command(command);
    }

    fn commit_title(&mut self, title: String) {
        self.commit_active_note_title(title);
    }

    fn toggle_editor_view(&mut self) {
        self.document_runtime.editor_mut().switch_active_plugin();
    }

    fn toggle_mindmap_style_panel(&mut self) {
        let editor = self.document_runtime.editor_mut();
        let Some(tab_id) = editor.active_tab_id() else {
            return;
        };
        let Some(mut tab) = editor.tab_session_mut(tab_id) else {
            return;
        };
        if tab.plugin_name() == ui::plugin::PLUGIN_MINDMAP {
            tab.toggle_mindmap_style_panel();
        }
    }

    fn dispatch_mindmap_style_panel(&mut self, action: ui::core::widget::MindmapStylePanelAction) {
        use ui::core::widget::MindmapStylePanelAction;

        match action {
            MindmapStylePanelAction::SelectTheme(theme_id) => {
                let outcome =
                    self.document_runtime.editor_mut().apply_active_mindmap_theme(theme_id);
                self.apply_editor_outcome(outcome);
            }
            MindmapStylePanelAction::Close | MindmapStylePanelAction::TogglePresets => {
                let editor = self.document_runtime.editor_mut();
                let Some(tab_id) = editor.active_tab_id() else {
                    return;
                };
                let Some(mut tab) = editor.tab_session_mut(tab_id) else {
                    return;
                };
                if tab.plugin_name() != ui::plugin::PLUGIN_MINDMAP {
                    return;
                }
                match action {
                    MindmapStylePanelAction::Close => tab.close_mindmap_style_panel(),
                    MindmapStylePanelAction::TogglePresets => tab.toggle_mindmap_style_presets(),
                    MindmapStylePanelAction::SelectTheme(_) => unreachable!(),
                }
            }
        }
    }

    fn execute_semantic_edit(&mut self, command: ui::plugin::SemanticEditCommand) {
        let (_result, outcome) = self.document_runtime.editor_mut().execute_semantic_edit(command);
        self.apply_editor_outcome(outcome);
    }

    fn execute_metadata_mutation(
        &mut self,
        mutation: crate::action::MetadataMutation,
    ) -> Vec<NotoraAction> {
        let note_id = metadata_mutation_note_id(&mutation);
        let selection_generation = self.action_runtime.state().library.selected_document_generation;
        if !self.document_runtime.register_metadata_mutation(
            mutation.clone(),
            note_id,
            selection_generation,
        ) {
            return Vec::new();
        }
        if let Err(error) = self.workspace_controller.execute_metadata_mutation(mutation.clone()) {
            self.document_runtime.complete_metadata_mutation(&mutation, note_id);
            return vec![NotoraAction::MetadataMutationFailed(error.to_string())];
        }
        Vec::new()
    }

    fn execute_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        let origin = match operation {
            crate::action::TrashOperation::MoveToTrash { note_id } => self
                .document_runtime
                .tab_for(DocumentIdentity::Note(note_id))
                .and_then(|tab_id| self.document_origin_for_tab(tab_id)),
            crate::action::TrashOperation::Restore { .. }
            | crate::action::TrashOperation::RestoreWithRenamedPath { .. }
            | crate::action::TrashOperation::PermanentlyDelete { .. }
            | crate::action::TrashOperation::Empty => None,
        };
        let outcome = self.document_runtime.prepare_trash_operation(operation, origin);
        self.apply_document_outcome(outcome);
    }

    fn submit_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        if let Err(error) = self.workspace_controller.execute_trash_operation(operation) {
            self.dispatch_action(NotoraAction::TrashOperationFailed(
                crate::action::TrashOperationFailure::Message(error.to_string()),
            ));
        }
    }

    fn choose_note_move_directory(&mut self, note_id: notora_core::NoteId) -> Vec<NotoraAction> {
        let identity = DocumentIdentity::Note(note_id);
        let current_directory = self
            .document_runtime
            .tab_for(identity)
            .and_then(|tab_id| self.document_runtime.editor_mut().document_summary(tab_id))
            .and_then(|summary| summary.path)
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
        let mut dialog = rfd::FileDialog::new();
        if let Some(directory) = current_directory {
            dialog = dialog.set_directory(directory);
        }
        let Some(destination) = dialog.pick_folder() else {
            return Vec::new();
        };
        let Some(workspace) = self.workspace_controller.active_workspace() else {
            return Vec::new();
        };
        let target_directory =
            match workspace_relative_directory(&workspace.descriptor.root, &destination) {
                Ok(relative_path) => relative_path,
                Err(message) => {
                    return vec![NotoraAction::NoteCommandFailed(message)];
                }
            };
        vec![NotoraAction::MoveRequested { note_id, target_directory }]
    }

    fn prepare_document(&mut self, request: DocumentLoadRequest) -> Vec<NotoraAction> {
        let identity = request.identity;
        if let DocumentIdentity::ExternalFile(external_file_id) = identity {
            self.prepare_external_document(request, external_file_id);
            return Vec::new();
        }
        if let Some(tab_id) = self.document_runtime.tab_for(identity) {
            self.document_runtime.touch_tab(tab_id);
            let outcome = self.document_runtime.editor_mut().activate(tab_id);
            self.apply_editor_outcome(outcome);
            return Vec::new();
        }
        let preparation = if self.action_runtime.state().library.navigation_scope
            == notora_core::NavigationScope::Trash
        {
            self.workspace_controller.prepare_trashed_document(request)
        } else {
            self.workspace_controller.prepare_document(request)
        };
        preparation
            .err()
            .map_or_else(Vec::new, |error| vec![NotoraAction::NoteCommandFailed(error.to_string())])
    }

    fn promote_active_preview(&mut self) {
        self.promote_active_preview_tab();
    }

    fn open_external_files(&mut self, request: ExternalOpenRequest) {
        let paths = match request {
            ExternalOpenRequest::ShowFileDialog => rfd::FileDialog::new()
                .add_filter("文本文档", &["txt", "md"])
                .pick_files()
                .unwrap_or_default(),
            ExternalOpenRequest::Paths(paths) => paths,
        };
        self.open_external_paths(paths);
    }

    fn create_untitled_external(&mut self, kind: notora_core::DocumentKind) -> Vec<NotoraAction> {
        let identity = self.action_runtime.create_untitled_external(kind);
        vec![NotoraAction::ExternalFileOpened(identity)]
    }

    fn save_document_manually(&mut self, request: ManualSaveRequest) {
        let origin = match request {
            ManualSaveRequest::Note { tab_id, .. } => self.document_origin_for_tab(tab_id),
            ManualSaveRequest::ExistingExternalFile { .. }
            | ManualSaveRequest::UntitledExternalFile { .. } => None,
        };
        let outcome = self.document_runtime.save_manually(
            request,
            origin,
            self.window_runtime.event_loop_proxy(),
        );
        self.apply_document_outcome(outcome);
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
        self.persistence_runtime.apply_settings_update(update);
        self.frame_runtime.apply_product_settings(self.persistence_runtime.product_settings());
        let runtime_tab_limit = NonZeroUsize::new(
            self.persistence_runtime.product_settings().interface.runtime_tab_limit,
        )
        .or_else(|| NonZeroUsize::new(DEFAULT_RUNTIME_TAB_LIMIT))
        .expect("default runtime tab limit must be non-zero");
        self.document_runtime.set_runtime_tab_limit(runtime_tab_limit);
        self.evict_excess_runtime_tabs();
        self.document_runtime.set_autosave_idle_delay(Duration::from_millis(
            self.persistence_runtime.product_settings().workspace.auto_save_delay_millis,
        ));
        self.document_runtime.editor_mut().update_settings(self.frame_runtime.settings().clone());
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

fn metadata_mutation_note_id(mutation: &crate::action::MetadataMutation) -> notora_core::NoteId {
    match mutation {
        crate::action::MetadataMutation::ToggleStar { note_id }
        | crate::action::MetadataMutation::AttachTagByName { note_id, .. }
        | crate::action::MetadataMutation::DetachTag { note_id, .. }
        | crate::action::MetadataMutation::SetTitle { note_id, .. }
        | crate::action::MetadataMutation::CompleteTitleInitializationFromHeader {
            note_id, ..
        }
        | crate::action::MetadataMutation::CompleteTitleInitializationFromDocument {
            note_id,
            ..
        } => *note_id,
    }
}

impl NotoraRuntime {
    fn select_workspace_root(&mut self) -> bool {
        let Some(root) = (self.workspace_directory_chooser)() else {
            return false;
        };
        if let Err(error) = self.execute_workspace_command(WorkspaceCommand::OpenExisting { root })
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
            return false;
        }
        if self.workspace_controller.active_workspace().is_none() {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "选择的目录未能激活为工作区".to_owned(),
            ));
            return false;
        }
        self.dispatch_action(NotoraAction::NavigationSelected(
            notora_core::NavigationScope::WorkspaceRoot,
        ));
        true
    }

    fn submit_note_command(&mut self, command: notora_core::note_command::NoteCommand) {
        if let Err(error) = self.workspace_controller.execute_note_command(command) {
            self.dispatch_action(NotoraAction::NoteCommandFailed(error.to_string()));
        }
    }

    fn submit_or_defer_note_move(&mut self, request: MoveNoteRequest) {
        let origin = self
            .document_runtime
            .tab_for(DocumentIdentity::Note(request.note_id))
            .and_then(|tab_id| self.document_origin_for_tab(tab_id));
        let outcome = self.document_runtime.prepare_note_move(request, origin);
        self.apply_document_outcome(outcome);
    }

    fn submit_or_defer_title_update(&mut self, request: notora_core::UpdateNoteTitleRequest) {
        let origin = self
            .document_runtime
            .tab_for(DocumentIdentity::Note(request.note_id))
            .and_then(|tab_id| self.document_origin_for_tab(tab_id));
        let outcome = self.document_runtime.prepare_title_update(request, origin);
        self.apply_document_outcome(outcome);
    }

    fn enqueue_product_settings_persistence(&mut self) {
        if let Err(error) = self.persistence_runtime.save_settings(self.paths.settings_file.clone())
        {
            self.record_settings_persistence_result(Err(error.to_string()));
        }
    }

    fn record_settings_persistence_result(&mut self, result: Result<(), String>) {
        self.persistence_runtime.record_settings_result(result);
        self.window_runtime.schedule_redraw();
    }

    fn schedule_session_persistence(&mut self) {
        self.persistence_runtime.schedule_session_persistence(Instant::now());
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
            .action_runtime
            .state()
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
        let last_document = match self.action_runtime.state().library.selected_card {
            Some(DocumentIdentity::Note(note_id)) => {
                Some(crate::session::SavedDocument::Note { note_id })
            }
            Some(DocumentIdentity::ExternalFile(external_file_id)) => {
                self.action_runtime.state().external_files.session(external_file_id).and_then(
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
            last_navigation_scope: (&self.action_runtime.state().library.navigation_scope).into(),
            last_document,
            expanded_directories: self
                .action_runtime
                .state()
                .library
                .navigation_tree
                .expanded_directories
                .iter()
                .cloned()
                .collect(),
            navigation_width_logical: self.action_runtime.state().layout.navigation_width_logical,
            card_list_width_logical: self.action_runtime.state().layout.card_list_width_logical,
            window_geometry: self.capture_window_geometry(),
            ..crate::session::ProductSession::default()
        }
    }

    fn capture_window_geometry(&self) -> crate::session::WindowGeometry {
        let (width_px, height_px) = self.window_runtime.size();
        let fallback = crate::session::WindowGeometry {
            width_px,
            height_px,
            ..crate::session::WindowGeometry::default()
        };
        let Some(window) = self.document_runtime.editor().window() else {
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
        let Some(session) = self.persistence_runtime.take_pending_session() else {
            return;
        };
        self.window_runtime
            .restore_size(session.window_geometry.width_px, session.window_geometry.height_px);
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
            self.action_runtime
                .record_command_error("上次使用的工作区不可用，或与保存的标识不再匹配".to_owned());
        }
        let saved_last_document = session.last_document.clone();
        let saved_external_path = match &saved_last_document {
            Some(crate::session::SavedDocument::ExternalPath { path }) => Some(path.as_path()),
            Some(crate::session::SavedDocument::Note { .. }) | None => None,
        };
        self.restore_external_paths(session.external_paths, saved_external_path);
        if workspace_restored {
            self.action_runtime.restore_expanded_directories(session.expanded_directories);
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
                let identity = DocumentIdentity::Note(note_id);
                self.dispatch_action(NotoraAction::CardSelected(identity));
                self.dispatch_action(NotoraAction::CardActivated(identity));
            }
            Some(crate::session::SavedDocument::ExternalPath { .. }) => {}
            Some(crate::session::SavedDocument::Note { .. }) | None => {}
        }
    }

    pub(crate) fn restore_session_after_first_frame(&mut self) {
        if !self.persistence_runtime.has_pending_session() {
            return;
        }
        self.restore_pending_session();
        self.window_runtime.schedule_redraw();
    }

    fn request_navigation_tree(&mut self) {
        if let Err(error) = self.workspace_controller.query_navigation_tree() {
            self.dispatch_action(NotoraAction::NavigationTreeFailed(error.to_string()));
        }
    }

    fn complete_trash_operation(&mut self, operation: crate::action::TrashOperation) {
        match operation {
            crate::action::TrashOperation::MoveToTrash { note_id }
            | crate::action::TrashOperation::Restore { note_id }
            | crate::action::TrashOperation::RestoreWithRenamedPath { note_id }
            | crate::action::TrashOperation::PermanentlyDelete { note_id } => {
                self.close_document_runtime(DocumentIdentity::Note(note_id));
            }
            crate::action::TrashOperation::Empty => self.close_read_only_note_runtimes(),
        }
    }

    fn close_document_runtime(&mut self, identity: DocumentIdentity) {
        let Some(tab_id) = self.document_runtime.tab_for(identity) else {
            return;
        };
        self.document_runtime.cancel_autosave(tab_id);
        self.document_runtime.clear_save_failure(tab_id);
        let _ = self.document_runtime.editor_mut().close_for_product(tab_id);
        self.document_runtime.remove_tab(tab_id);
        if self.action_runtime.state().library.selected_card == Some(identity)
            && self.action_runtime.state().library.navigation_scope
                != notora_core::NavigationScope::Trash
        {
            self.action_runtime.invalidate_document_selection();
        }
    }

    fn close_read_only_note_runtimes(&mut self) {
        let read_only_tabs = self
            .document_runtime
            .editor()
            .workspace_snapshot()
            .tabs
            .into_iter()
            .filter(|tab| {
                self.document_runtime.editor().tab_session(tab.tab_id).is_some_and(|session| {
                    session.editing_access()
                        == appkit_shell::tab_runtime::DocumentEditingAccess::ReadOnly
                })
            })
            .filter_map(|tab| {
                matches!(
                    self.document_runtime.identity_for(tab.tab_id),
                    Some(DocumentIdentity::Note(_))
                )
                .then_some(tab.tab_id)
            })
            .collect::<Vec<_>>();
        for tab_id in read_only_tabs {
            self.document_runtime.cancel_autosave(tab_id);
            self.document_runtime.clear_save_failure(tab_id);
            let _ = self.document_runtime.editor_mut().close_for_product(tab_id);
            self.document_runtime.remove_tab(tab_id);
        }
    }

    fn schedule_catalog_backup(&mut self) {
        self.persistence_runtime.schedule_catalog_backup(Instant::now());
    }

    fn start_catalog_backup(&mut self) {
        let Some(active_workspace) = self.workspace_controller.active_workspace() else {
            return;
        };
        let Some(retention) = notora_core::BackupRetention::keep_latest(
            self.persistence_runtime.product_settings().workspace.catalog_backup_retention,
        ) else {
            return;
        };
        let directory = self
            .paths
            .catalog_backups_directory
            .join(active_workspace.descriptor.workspace_id.to_string());
        if let Err(error) = self.workspace_controller.create_catalog_backup(directory, retention) {
            self.dispatch_action(NotoraAction::MetadataMutationFailed(format!(
                "元数据已保存，但无法启动目录索引备份：{error}"
            )));
        }
    }

    fn flush_pending_catalog_backup(&mut self) {
        if self.persistence_runtime.take_pending_catalog_backup() {
            self.start_catalog_backup();
        }
    }

    fn retry_conflicted_document_save(&mut self, identity: DocumentIdentity) {
        let outcome = self.document_runtime.retry_conflicted_document_save(identity);
        self.apply_document_outcome(outcome);
    }

    fn start_conflict_retry_revision_capture(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    ) {
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-conflict-retry-revision".to_owned())
            .spawn(move || {
                let completion = match appkit_core::file_safety::capture_revision(&path) {
                    Ok(disk_revision) => DocumentCompletion::ConflictRetryRevisionCaptured {
                        identity,
                        tab_id,
                        content_revision,
                        path,
                        disk_revision,
                    },
                    Err(error) => DocumentCompletion::ConflictRetryRevisionFailed {
                        identity,
                        message: format!("重试保存前无法读取当前磁盘版本：{error}"),
                    },
                };
                let _ = sender.send(NotoraProductEvent::Document(completion));
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "无法启动冲突保存重试线程".to_owned(),
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
        let request = self.manual_save_request_for_tab(tab_id);
        let outcome = self.document_runtime.complete_conflict_retry_revision_capture(
            identity,
            tab_id,
            content_revision,
            path,
            disk_revision,
            request,
        );
        self.apply_document_outcome(outcome);
    }

    fn save_conflicted_note_copy(&mut self, identity: DocumentIdentity) {
        let Some(path) = rfd::FileDialog::new().add_filter("文本文档", &["txt", "md"]).save_file()
        else {
            return;
        };
        let outcome = self.document_runtime.prepare_conflict_copy(identity, path);
        self.apply_document_outcome(outcome);
    }

    fn start_conflict_copy(
        &mut self,
        identity: DocumentIdentity,
        prepared: appkit_shell::editor_runtime::PreparedDocumentSave,
    ) {
        let Some(workspace) = self.workspace_controller.active_workspace() else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "工作区关闭后无法保存冲突副本".to_owned(),
            ));
            return;
        };
        let sender = WorkspaceEventSender::new(
            self.product.event_sender(),
            WorkspaceEventScope {
                workspace_id: workspace.descriptor.workspace_id,
                generation: workspace.generation,
            },
        );
        if thread::Builder::new()
            .name("notora-conflict-copy".to_owned())
            .spawn(move || {
                let result = appkit_shell::editor_runtime::execute_prepared_save(prepared)
                    .result
                    .map(|_| ())
                    .map_err(|error| error.to_string());
                let _ =
                    sender.send(WorkspaceCompletion::ConflictCopyCompleted { identity, result });
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "无法启动冲突副本保存线程".to_owned(),
            ));
        }
    }

    fn reload_conflicted_document(&mut self, identity: DocumentIdentity) {
        let outcome = self.document_runtime.reload_conflicted_document(identity);
        self.apply_document_outcome(outcome);
    }

    fn start_conflict_reload(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::workspace::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
    ) {
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-conflict-reload".to_owned())
            .spawn(move || match load_document(&path) {
                Ok(document) => {
                    let _ = sender.send(NotoraProductEvent::Document(
                        DocumentCompletion::ConflictReloadCompleted {
                            identity,
                            tab_id,
                            content_revision,
                            document,
                        },
                    ));
                }
                Err(error) => {
                    let _ = sender.send(NotoraProductEvent::Document(
                        DocumentCompletion::ConflictReloadFailed {
                            identity,
                            message: error.to_string(),
                        },
                    ));
                }
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "无法启动冲突重新加载线程".to_owned(),
            ));
        }
    }

    fn open_external_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        let outcome = self.document_runtime.open_external_paths(paths);
        self.apply_document_outcome(outcome);
    }

    fn restore_external_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        saved_last_path: Option<&std::path::Path>,
    ) {
        let outcome = self.document_runtime.restore_external_paths(paths, saved_last_path);
        self.apply_document_outcome(outcome);
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
                    let completion = match result {
                        Ok((canonical_path, document)) => {
                            DocumentCompletion::ExternalFileOpenCompleted {
                                canonical_path,
                                document,
                                activate,
                            }
                        }
                        Err(message) => DocumentCompletion::ExternalFileOpenFailed { message },
                    };
                    let _ = sender.send(NotoraProductEvent::Document(completion));
                }
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "无法启动外部文件读取线程".to_owned(),
            ));
        }
    }

    fn prepare_external_document(
        &mut self,
        request: DocumentLoadRequest,
        external_file_id: notora_core::ExternalFileId,
    ) {
        let session = self.action_runtime.external_file_session(external_file_id);
        let selection = self.document_selection();
        let outcome = self.document_runtime.prepare_external_document(request, session, selection);
        self.apply_document_outcome(outcome);
    }

    fn complete_external_file_open(
        &mut self,
        canonical_path: CanonicalExternalPath,
        document: LoadedDocument,
        activate: bool,
    ) {
        let identity = self.action_runtime.open_existing_external(canonical_path);
        let outcome =
            self.document_runtime.complete_external_file_open(identity, document, activate);
        self.apply_document_outcome(outcome);
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
                let completion = match load_document(canonical_path.as_path()) {
                    Ok(document) => {
                        DocumentCompletion::ExternalDocumentLoaded { request, document }
                    }
                    Err(error) => DocumentCompletion::ExternalDocumentLoadFailed {
                        request,
                        message: error.to_string(),
                    },
                };
                let _ = sender.send(NotoraProductEvent::Document(completion));
            })
            .is_err()
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "无法启动外部文档读取线程".to_owned(),
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
        let outcome = self.document_runtime.complete_conflict_reload(
            identity,
            tab_id,
            content_revision,
            loaded,
        );
        self.apply_document_outcome(outcome);
    }
}

fn workspace_relative_directory(
    workspace_root: &std::path::Path,
    destination: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let workspace_root = std::fs::canonicalize(workspace_root)
        .map_err(|error| format!("无法解析活动工作区：{error}"))?;
    let destination =
        std::fs::canonicalize(destination).map_err(|error| format!("无法解析移动目标：{error}"))?;
    destination
        .strip_prefix(&workspace_root)
        .map(std::path::Path::to_path_buf)
        .map_err(|_| "移动目标必须位于活动工作区内".to_owned())
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

    use super::document_runtime::{
        PendingNoteMove, PendingTitleUpdate, PendingTrashMove, initial_title_from_document,
    };
    use super::frame_runtime::FontSystemPreparation;
    use super::{
        NotoraRuntime, SettingsPersistenceState, StartupTrace, resolve_pointer_cursor,
        workspace_relative_directory,
    };
    use crate::action::{MetadataMutation, NotoraAction};
    use crate::autosave::{AutoSaveRequest, AutoSaveState};
    use crate::editor_adapter::LoadedDocument;
    use crate::state::{CardPageState, normalize_notora_title};
    use crate::{
        CompactContent, ExternalFileSession, FocusTarget, NotoraPaths, OverlayState,
        WorkspaceCommand, WorkspaceRootState,
    };
    use appkit_shell::editor_runtime::{
        DocumentTextReplacement, EditorFocus, EditorInputContext, EditorNotification,
    };
    use notora_core::{DocumentIdentity, DocumentKind, NavigationScope, WorkspaceId};

    #[test]
    fn product_pointer_cursor_takes_priority_over_editor_feedback() {
        use winit::window::CursorIcon;

        assert_eq!(
            resolve_pointer_cursor(Some(CursorIcon::EwResize), Some(CursorIcon::Text)),
            CursorIcon::EwResize
        );
        assert_eq!(resolve_pointer_cursor(None, Some(CursorIcon::Grab)), CursorIcon::Grab);
        assert_eq!(resolve_pointer_cursor(None, None), CursorIcon::Default);
    }

    fn app() -> NotoraRuntime {
        let directory = tempfile::tempdir().expect("test should create a temporary directory");
        let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
            .expect("test should create isolated product paths");
        NotoraRuntime::with_paths(paths).expect("notora app should construct without a window")
    }

    fn install_registered_note(
        app: &mut NotoraRuntime,
        path: &str,
        contents: &str,
    ) -> (DocumentIdentity, appkit_core::workspace::types::TabId) {
        let identity = DocumentIdentity::Note(notora_core::NoteId::generate());
        let prepared = crate::editor_adapter::prepare_loaded_document(
            &app.document_runtime.editor_runtime,
            LoadedDocument {
                path: std::path::PathBuf::from(path),
                contents: contents.to_owned(),
                disk_revision: None,
            },
        )
        .expect("registered note fixture should prepare");
        let _ = app.document_runtime.editor_runtime.install_prepared_tab(
            prepared,
            None,
            appkit_shell::editor_runtime::OpenDisposition::Persistent,
        );
        let tab_id = app
            .document_runtime
            .editor_runtime
            .active_tab_id()
            .expect("registered note fixture should become active");
        let _ = app.document_runtime.document_registry.register(identity, tab_id);
        app.action_runtime.state.library.selected_card = Some(identity);
        (identity, tab_id)
    }

    fn active_editor_input_context() -> EditorInputContext {
        EditorInputContext {
            editor_rect: ui::Rect::new(0.0, 0.0, 640.0, 480.0),
            focus: EditorFocus::Active,
            modal_blocked: false,
        }
    }

    fn note_origin(relative_path: &str) -> notora_core::DocumentOrigin {
        notora_core::DocumentOrigin::Note {
            workspace_id: notora_core::WorkspaceId::generate(),
            note_id: notora_core::NoteId::generate(),
            relative_path: relative_path.into(),
        }
    }

    fn drain_until_document_text(
        app: &mut NotoraRuntime,
        tab_id: appkit_core::workspace::types::TabId,
        expected_text: &str,
        deadline: Instant,
    ) {
        loop {
            app.drain_product_events();
            let text = app
                .document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("document text should remain available")
                .text;
            if text == expected_text {
                return;
            }
            assert!(Instant::now() < deadline, "document text should reach {expected_text:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn notora_title_normalization_does_not_depend_on_document_source() {
        assert_eq!(normalize_notora_title("  项目路线图  "), "项目路线图");
        assert_eq!(normalize_notora_title("   "), "无标题");
    }

    #[test]
    fn autosave_failure_retains_the_diagnostic_for_the_editor_header() {
        let mut app = app();
        let tab_id = appkit_core::workspace::types::TabIdAllocator::new().allocate();
        let request = AutoSaveRequest { tab_id, content_revision: 7 };
        let origin = notora_core::DocumentOrigin::Note {
            workspace_id: notora_core::WorkspaceId::generate(),
            note_id: notora_core::NoteId::generate(),
            relative_path: "diagram.mmap.md".into(),
        };
        app.document_runtime.autosave.request_immediate_save(
            &origin,
            tab_id,
            request.content_revision,
        );
        assert_eq!(app.document_runtime.autosave.take_due_saves(), vec![request]);

        app.record_autosave_failure(request, "file is read-only".to_owned());

        assert_eq!(
            app.document_runtime.autosave.state(tab_id),
            Some(AutoSaveState::Failed { content_revision: request.content_revision })
        );
        assert_eq!(
            app.document_runtime.save_failure_messages.get(&tab_id).map(String::as_str),
            Some("file is read-only")
        );
    }

    #[test]
    fn autosave_request_for_a_clean_document_finishes_without_a_failure() {
        let directory = tempfile::tempdir().expect("save fixture directory should exist");
        let path = directory.path().join("clean.md");
        let mut app = app();
        let (_, tab_id) = install_registered_note(
            &mut app,
            path.to_str().expect("fixture path should be valid UTF-8"),
            "原始正文",
        );
        let edit_outcome = app
            .document_runtime
            .editor_runtime
            .commit_text(active_editor_input_context(), "已保存修改".to_owned());
        app.apply_editor_outcome(edit_outcome);
        let edited_summary = app
            .document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .expect("edited note should remain available");
        app.document_runtime.autosave.request_immediate_save(
            &note_origin("clean.md"),
            tab_id,
            edited_summary.content_revision,
        );
        let request = app
            .document_runtime
            .autosave
            .take_due_saves()
            .pop()
            .expect("edited revision request should become due");
        let prepared = app
            .document_runtime
            .editor_runtime
            .prepare_save(tab_id)
            .expect("edited note should prepare a save");
        let save_outcome = app
            .document_runtime
            .editor_runtime
            .apply_save_completion(appkit_shell::editor_runtime::execute_prepared_save(prepared));
        app.apply_editor_outcome(save_outcome);
        let clean_summary = app
            .document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .expect("registered note should remain available");
        assert!(!clean_summary.dirty);

        app.submit_autosave(request);

        assert_eq!(app.document_runtime.autosave.state(tab_id), None);
        assert_eq!(app.document_runtime.save_failure_messages.get(&tab_id), None);
    }

    #[test]
    fn stale_autosave_request_is_replaced_by_the_current_dirty_revision() {
        let mut app = app();
        let (_, tab_id) = install_registered_note(&mut app, "stale.md", "原始正文");
        let first_edit = app
            .document_runtime
            .editor_runtime
            .commit_text(active_editor_input_context(), "第一次修改".to_owned());
        app.apply_editor_outcome(first_edit);
        let first_revision = app
            .document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .expect("edited note should remain available")
            .content_revision;
        app.document_runtime.autosave.request_immediate_save(
            &note_origin("stale.md"),
            tab_id,
            first_revision,
        );
        let stale_request = app
            .document_runtime
            .autosave
            .take_due_saves()
            .pop()
            .expect("first revision request should become due");
        let second_edit = app
            .document_runtime
            .editor_runtime
            .commit_text(active_editor_input_context(), "第二次修改".to_owned());
        app.apply_editor_outcome(second_edit);
        let current_revision = app
            .document_runtime
            .editor_runtime
            .document_summary(tab_id)
            .expect("twice-edited note should remain available")
            .content_revision;
        assert_ne!(current_revision, stale_request.content_revision);

        app.submit_autosave(stale_request);

        assert!(matches!(
            app.document_runtime.autosave.state(tab_id),
            Some(AutoSaveState::Scheduled { content_revision, .. })
                if content_revision == current_revision
        ));
        assert_eq!(app.document_runtime.save_failure_messages.get(&tab_id), None);
    }

    #[test]
    fn initial_document_title_supports_markdown_h1_and_compact_mmap_root_syntax() {
        assert_eq!(
            initial_title_from_document(DocumentKind::Markdown, "# Markdown Title\n"),
            Some("Markdown Title".to_owned())
        );
        assert_eq!(
            initial_title_from_document(DocumentKind::Mindmap, "#Mindmap Root\n##Child\n"),
            Some("Mindmap Root".to_owned())
        );
    }

    fn card_page_contains_note(card_page: &CardPageState, note_id: notora_core::NoteId) -> bool {
        let cards = match card_page {
            CardPageState::LoadingNextPage { cards, .. }
            | CardPageState::Refreshing { cards, .. }
            | CardPageState::Ready { cards, .. }
            | CardPageState::Failed { cards, .. } => cards,
            CardPageState::Idle
            | CardPageState::LoadingInitial { .. }
            | CardPageState::Empty { .. } => return false,
        };
        cards.iter().any(|card| card.note_id == note_id)
    }

    #[test]
    fn duplicate_metadata_mutations_share_one_pending_worker_request() {
        let note_id = notora_core::NoteId::generate();
        let mutation =
            MetadataMutation::AttachTagByName { note_id, display_name: "产品/Notora".to_owned() };
        let mut app = app();

        assert!(app.document_runtime.register_metadata_mutation(mutation.clone(), note_id, 7,));
        assert!(!app.document_runtime.register_metadata_mutation(mutation.clone(), note_id, 7,));
        assert_eq!(app.document_runtime.pending_metadata_mutations, vec![mutation.clone()]);

        assert_eq!(app.document_runtime.complete_metadata_mutation(&mutation, note_id), Some(7));
        assert!(app.document_runtime.pending_metadata_mutations.is_empty());
    }

    #[test]
    fn startup_trace_reports_the_first_frame_once() {
        let mut trace = StartupTrace::started_now();

        assert!(trace.take_first_frame_elapsed().is_some());
        assert!(trace.take_first_frame_elapsed().is_none());
    }

    #[test]
    fn background_font_preparation_returns_to_the_deferred_state_after_join() {
        let directory = tempfile::tempdir().expect("test should create a temporary directory");
        let paths = NotoraPaths::from_config_directory(directory.path().join("notora"))
            .expect("test should create isolated product paths");
        let mut app =
            NotoraRuntime::with_paths(paths).expect("notora app should construct without a window");

        app.start_font_system_preparation();
        assert!(matches!(
            &app.frame_runtime.font_system_preparation,
            FontSystemPreparation::InProgress(_)
        ));

        let font_system = app.take_prepared_font_system();

        std::hint::black_box(font_system);
        assert!(matches!(
            app.frame_runtime.font_system_preparation,
            FontSystemPreparation::Deferred
        ));
    }

    #[test]
    fn move_dialog_destination_is_reduced_to_a_safe_domain_input() {
        let workspace = tempfile::tempdir().expect("workspace fixture should exist");
        let archive_directory = workspace.path().join("archive");
        let outside_directory = tempfile::tempdir().expect("outside fixture should exist");
        std::fs::create_dir_all(&archive_directory).expect("archive directory should exist");

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
        assert!(!app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

        let _ = install_registered_note(&mut app, "ime.md", "");
        assert!(app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

        app.dispatch_action(NotoraAction::OpenSettings);
        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::Settings);
        assert!(!app.update_editor_preedit("音".to_owned(), Some((0, 1))));
    }

    #[test]
    fn navigation_change_detaches_the_visible_editor_from_the_previous_runtime_tab() {
        let mut app = app();
        let _ = install_registered_note(&mut app, "previous.md", "旧正文");

        assert!(app.active_editor_matches_selection());

        app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Trash));

        assert!(!app.active_editor_matches_selection());
    }

    #[test]
    fn clicking_the_editor_transfers_keyboard_focus_from_the_card_list() {
        let mut app = app();
        app.render().expect("shell layout should be available for pointer routing");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
        let editor_rect = app.shell_layout().editor_rect;

        assert!(!app.route_pointer_event(&ui::Event::MouseDown {
            px: editor_rect.x + editor_rect.w * 0.5,
            py: editor_rect.y + editor_rect.h * 0.5,
            button: ui::core::widget::MouseButton::Left,
        }));
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
    }

    #[test]
    fn focused_markdown_document_schedules_cursor_blink() {
        let mut app = app();
        let _ = install_registered_note(&mut app, "caret.md", "正文内容");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));

        assert!(app.next_deadline().is_some());
        assert!(app.document_runtime.editor_runtime.active_cursor_paint_enabled());

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::EditorTitle));
        assert!(!app.document_runtime.editor_runtime.active_cursor_paint_enabled());

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::EditorTag));
        assert!(!app.document_runtime.editor_runtime.active_cursor_paint_enabled());

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
        assert_eq!(app.next_deadline(), None);
    }

    #[test]
    fn product_text_focus_schedules_its_blink_before_another_input_event() {
        let mut app = app();
        assert_eq!(app.frame_runtime.shell.next_text_cursor_blink_at(), None);

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::NavigationSearch));

        assert!(app.frame_runtime.shell.next_text_cursor_blink_at().is_some());
        assert_eq!(app.next_deadline(), app.frame_runtime.shell.next_text_cursor_blink_at());
    }

    #[test]
    fn first_search_click_synchronizes_widget_focus_before_the_next_input_event() {
        let mut app = app();
        app.render().expect("headless shell frame should render");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));

        assert!(app.route_product_event(&ui::Event::MouseDown {
            px: 24.0,
            py: 24.0,
            button: ui::core::MouseButton::Left,
        }));

        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationSearch);
        assert!(app.frame_runtime.shell.search_box_is_focused());
    }

    #[test]
    fn search_text_drag_requests_redraw_in_the_same_pointer_event_cycle() {
        let mut app = app();
        app.dispatch_action(NotoraAction::SearchTextChanged("路线图".to_owned()));
        app.render().expect("headless search box should render");
        let search_rect = app.frame_runtime.shell.search_box_rect();

        assert!(app.route_pointer_event(&ui::Event::MouseDown {
            px: search_rect.x + 8.0,
            py: search_rect.y + search_rect.h * 0.5,
            button: ui::core::widget::MouseButton::Left,
        }));
        app.render().expect("focused search box should render");
        let _ = app.take_redraw_request();
        assert!(!app.take_redraw_request());

        assert!(app.route_pointer_event(&ui::Event::MouseMove {
            px: search_rect.right() - 8.0,
            py: search_rect.y + search_rect.h * 0.5,
        }));
        assert!(
            app.window_runtime.redraw_is_requested(),
            "drag selection should schedule a redraw"
        );
        assert!(
            app.document_runtime.editor_runtime.take_redraw_request(),
            "drag selection should wake the renderer without waiting for about_to_wait"
        );
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
    fn settings_modal_blocks_splitter_keyboard_adjustments() {
        let mut app = app();
        app.render().expect("shell layout should initialize splitter inputs");
        let navigation_width_before_modal =
            app.action_runtime.state().layout.navigation_width_logical;

        app.dispatch_action(NotoraAction::OpenSettings);

        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::Settings);
        assert!(app.route_product_event(&ui::Event::KeyDown(
            ui::KeyCode::Left,
            ui::core::Modifiers::NONE,
        )));
        assert_eq!(
            app.action_runtime.state().layout.navigation_width_logical,
            navigation_width_before_modal,
            "modal keyboard input must not resize an underlying splitter"
        );
    }

    #[test]
    fn modal_state_blocks_workspace_shortcuts_and_escape_closes_the_modal() {
        let mut app = app();
        app.dispatch_action(NotoraAction::OpenSettings);
        let modal_focus = app.action_runtime.state().layout.focus_target;

        let command_modifiers = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };
        for key_code in [
            ui::KeyCode::Char('n'),
            ui::KeyCode::Char('o'),
            ui::KeyCode::Char('f'),
            ui::KeyCode::Char('s'),
        ] {
            app.handle_key_input(key_code, command_modifiers);
        }

        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::Settings);
        assert_eq!(app.action_runtime.state().layout.focus_target, modal_focus);

        app.handle_key_input(ui::KeyCode::Escape, ui::core::Modifiers::NONE);
        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationTree);
    }

    #[test]
    fn notora_theme_mode_resolves_against_its_own_product_settings() {
        let mut app = app();
        app.persistence_runtime.product_settings.appearance.theme_mode = ui::ThemeMode::System;
        app.rebuild_theme_for_system_appearance(winit::window::Theme::Light);
        assert!(!app.frame_runtime.theme.is_dark);
        app.rebuild_theme_for_system_appearance(winit::window::Theme::Dark);
        assert!(app.frame_runtime.theme.is_dark);

        app.persistence_runtime.product_settings.appearance.theme_mode = ui::ThemeMode::Light;
        app.rebuild_theme_for_system_appearance(winit::window::Theme::Dark);
        assert!(!app.frame_runtime.theme.is_dark);
    }

    #[test]
    fn notora_editor_setting_updates_its_product_and_runtime_snapshots() {
        let mut app = app();

        app.dispatch_action(NotoraAction::ProductSettingsUpdateRequested(
            crate::settings_overlay::ProductSettingsUpdate::FontSize(20.0),
        ));

        assert_eq!(app.persistence_runtime.product_settings.editor.font_size, 20.0);
        assert_eq!(app.document_runtime.editor_runtime.settings_snapshot().font_size, 20.0);
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
        crate::product::WorkspaceEventSender::new(
            app.product.event_sender(),
            crate::product::WorkspaceEventScope {
                workspace_id: active_workspace.descriptor.workspace_id,
                generation: active_workspace.generation,
            },
        )
        .send(crate::product::WorkspaceCompletion::WorkspaceIndexFailed {
            message: "工作区文件监视器已断开，自动同步已停止".to_owned(),
        })
        .expect("product receiver should stay available");

        app.drain_product_events();

        assert!(app.document_runtime.catalog_reconciliation_pending);
        assert!(
            app.action_runtime
                .state()
                .library
                .last_command_error
                .as_deref()
                .is_some_and(|message| message.contains("文件监视器已断开"))
        );
    }

    #[test]
    fn session_workspace_identity_mismatch_closes_the_unexpected_workspace() {
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        notora_core::Workspace::open_or_initialize(workspace_directory.path())
            .expect("workspace metadata should initialize");
        let mut app = app();
        app.persistence_runtime.pending_session = Some(crate::session::ProductSession {
            workspace_root: Some(workspace_directory.path().to_path_buf()),
            workspace_id: Some(WorkspaceId::generate()),
            ..crate::session::ProductSession::default()
        });

        app.restore_pending_session();

        assert_eq!(app.workspace_controller.active_workspace(), None);
        assert!(
            app.action_runtime
                .state
                .library
                .last_command_error
                .as_deref()
                .is_some_and(|message| message.contains("不再匹配"))
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
        app.persistence_runtime.pending_session = Some(crate::session::ProductSession {
            external_paths: vec![saved_last_path.clone(), other_path],
            last_document: Some(crate::session::SavedDocument::ExternalPath {
                path: saved_last_path.clone(),
            }),
            ..crate::session::ProductSession::default()
        });

        app.restore_pending_session();

        let deadline = Instant::now() + Duration::from_secs(2);
        while app.action_runtime.state.external_files.sessions().len() < 2 {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "external sessions should restore promptly");
            thread::sleep(Duration::from_millis(10));
        }
        let selected_identity = app
            .action_runtime
            .state
            .library
            .selected_card
            .expect("saved last external document should be selected");
        let selected_path = match selected_identity {
            DocumentIdentity::ExternalFile(external_file_id) => {
                app.action_runtime
                    .state
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

        app.apply_product_settings_update(
            crate::settings_overlay::ProductSettingsUpdate::WordWrap(false),
        );

        assert_eq!(app.action_runtime.state.library.last_command_error, None);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.drain_product_events();
            if matches!(
                app.persistence_runtime.settings_state,
                SettingsPersistenceState::SaveFailed { .. }
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "settings persistence failure should arrive");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(matches!(
            app.persistence_runtime.settings_state.to_view(),
            crate::settings_overlay::NotoraSettingsPersistenceView::SaveFailed { .. }
        ));
        let retry_path = app.paths.config_directory.join("retry-settings.toml");
        app.paths.settings_file = retry_path.clone();
        app.dispatch_action(NotoraAction::RetryProductSettingsPersistence);

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.drain_product_events();
            if app.persistence_runtime.settings_state == SettingsPersistenceState::Saved {
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
        let _ = install_registered_note(&mut app, "search.md", "搜索正文");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        assert!(app.update_editor_preedit("document".to_owned(), Some((0, 8))));
        app.render().expect("headless shell frame should render");

        assert!(app.route_product_event(&ui::Event::MouseDown {
            px: 24.0,
            py: 24.0,
            button: ui::core::widget::MouseButton::Left,
        }));
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationSearch);
        assert!(app.route_product_event(&ui::Event::ImeCommit("路线图".to_owned())));

        assert_eq!(
            app.action_runtime.state().library.navigation_scope,
            NavigationScope::Search { query: "路线图".to_owned() }
        );
        assert_eq!(app.document_runtime.editor_runtime.preedit().0, "document");
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
                app.action_runtime.state().library.selected_card,
                Some(notora_core::DocumentIdentity::Note(_))
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "note completion should update product state");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(workspace_directory.path().join("无标题.md").is_file());
        assert_eq!(app.action_runtime.state().library.last_command_error, None);
    }

    #[test]
    fn typed_menu_creation_uses_the_selected_directory_and_opens_the_note() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let notes_directory = workspace_directory.path().join("notes");
        std::fs::create_dir_all(&notes_directory).expect("notes directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Directory {
            relative_path: "notes".into(),
        }));
        app.dispatch_action(NotoraAction::OpenNewDocumentMenu);
        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Text));

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.drain_product_events();
            if matches!(
                app.action_runtime.state().library.selected_card,
                Some(DocumentIdentity::Note(_))
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "configured note should be created");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(notes_directory.join("无标题.txt").is_file());
        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
    }

    #[test]
    fn creation_failure_closes_the_menu_and_reports_the_error() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Directory {
            relative_path: "missing".into(),
        }));
        app.dispatch_action(NotoraAction::OpenNewDocumentMenu);
        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        while app.action_runtime.state().library.last_command_error.is_none() {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "creation failure should return to the app");
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
        assert!(!workspace_directory.path().join("missing").exists());
    }

    #[test]
    fn title_commit_updates_the_active_workspace_note_through_editor_runtime() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        let identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card {
                break identity;
            }
            assert!(Instant::now() < deadline, "note completion should select a note");
            thread::sleep(Duration::from_millis(10));
        };
        let tab_id = loop {
            app.drain_product_events();
            if let Some(tab_id) = app.document_tab_for(identity) {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created note should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };

        app.dispatch_action(NotoraAction::TitleCommitRequested("项目路线图".to_owned()));

        while app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created note text should remain available")
            .text
            .is_empty()
        {
            app.drain_runtime_save_completions();
            app.drain_product_events();
            assert!(Instant::now() < deadline, "title initialization should complete");
            thread::sleep(Duration::from_millis(10));
        }

        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created note text should remain available");
        assert_eq!(snapshot.text, "# 项目路线图\n\n");
        assert!(snapshot.content_revision > 0);
        while !workspace_directory.path().join("项目路线图.md").is_file()
            || app
                .state()
                .library
                .active_editor_metadata
                .as_ref()
                .is_none_or(|metadata| metadata.metadata.title_revision < 1)
        {
            app.drain_runtime_save_completions();
            app.drain_product_events();
            assert!(Instant::now() < deadline, "first title should rename its entity file",);
            thread::sleep(Duration::from_millis(10));
        }

        let prepared = app
            .document_runtime
            .editor_runtime
            .prepare_save(tab_id)
            .expect("seeded title body should prepare for save");
        let completion = appkit_shell::editor_runtime::execute_prepared_save(prepared);
        assert!(completion.result.is_ok(), "seeded body should save: {:?}", completion.result);
        let outcome = app.document_runtime.editor_runtime.apply_save_completion(completion);
        app.apply_editor_outcome(outcome);
        app.document_runtime.autosave.cancel(tab_id);

        app.dispatch_action(NotoraAction::TitleCommitRequested("独立的 Notora 标题".to_owned()));
        let second_deadline = Instant::now() + Duration::from_secs(2);
        while !workspace_directory.path().join("独立的 Notora 标题.md").is_file() {
            app.drain_runtime_save_completions();
            app.drain_product_events();
            assert!(
                Instant::now() < second_deadline,
                "independent title should persist; dirty={:?}, error={:?}, revision={:?}",
                app.document_runtime
                    .editor_runtime
                    .document_summary(tab_id)
                    .map(|summary| summary.dirty),
                app.action_runtime.state().library.last_command_error,
                app.action_runtime
                    .state()
                    .library
                    .active_editor_metadata
                    .as_ref()
                    .map(|metadata| metadata.metadata.title_revision),
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!workspace_directory.path().join("项目路线图.md").exists());
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("independent body should remain available")
                .text,
            "# 项目路线图\n\n"
        );
    }

    #[test]
    fn leaving_title_focus_commits_the_current_draft() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created note should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created note should render its title editor");

        assert!(app.route_product_event(&ui::Event::ImeCommit("项目路线图".to_owned())));
        assert_eq!(app.frame_runtime.shell.editor_title_text(), "项目路线图");

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));

        drain_until_document_text(&mut app, tab_id, "# 项目路线图\n\n", deadline);
        app.render().expect("committed title should render");
        assert_eq!(app.frame_runtime.shell.editor_title_text(), "项目路线图");
    }

    #[test]
    fn first_saved_markdown_h1_initializes_the_title_and_then_becomes_independent() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        let (note_id, tab_id) = loop {
            app.drain_product_events();
            if let Some(DocumentIdentity::Note(note_id)) =
                app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(DocumentIdentity::Note(note_id))
                && app.action_runtime.state().library.active_editor_metadata.is_some()
            {
                break (note_id, tab_id);
            }
            assert!(Instant::now() < deadline, "created note should finish loading");
            thread::sleep(Duration::from_millis(10));
        };
        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created source should exist");
        let outcome = app
            .document_runtime
            .editor_runtime
            .replace_document_text(DocumentTextReplacement {
                tab_id,
                content_revision: snapshot.content_revision,
                range: 0..snapshot.text.len(),
                replacement: "# 正文优先\n\n正文".to_owned(),
            })
            .expect("body fixture edit should apply");
        app.apply_editor_outcome(outcome);
        let content_revision = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("edited source should exist")
            .content_revision;
        app.handle_editor_notification(&EditorNotification::SaveCompleted {
            tab_id,
            content_revision,
        });

        while app.document_runtime.pending_metadata_mutations.iter().any(|mutation| {
            matches!(mutation, MetadataMutation::CompleteTitleInitializationFromDocument { .. })
        }) {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "document initialization should persist");
            thread::sleep(Duration::from_millis(10));
        }
        let catalog =
            notora_core::Catalog::open(&workspace_directory.path().join(".notora/catalog.sqlite3"))
                .expect("catalog should reopen for verification");
        assert_eq!(
            catalog
                .active_note(note_id)
                .expect("note lookup should succeed")
                .expect("note should remain active")
                .title,
            "正文优先"
        );

        app.dispatch_action(NotoraAction::TitleCommitRequested("后改的 Notora 标题".to_owned()));
        let second_deadline = Instant::now() + Duration::from_secs(2);
        while app
            .document_runtime
            .pending_metadata_mutations
            .iter()
            .any(|mutation| matches!(mutation, MetadataMutation::SetTitle { .. }))
        {
            app.drain_product_events();
            assert!(Instant::now() < second_deadline, "independent title should persist");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("independent body should remain available")
                .text,
            "# 正文优先\n\n正文"
        );
    }

    #[test]
    fn mindmap_title_commit_renames_the_existing_empty_root() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("created mmap source should exist")
                .text,
            "#"
        );

        app.dispatch_action(NotoraAction::TitleCommitRequested("项目路线图".to_owned()));

        while app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap source should remain available")
            .text
            == "#"
        {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "mmap title initialization should complete");
            thread::sleep(Duration::from_millis(10));
        }

        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("renamed mmap source should remain available");
        assert_eq!(snapshot.text, "# 项目路线图");
        let tree = textora_markdown::mmf::parser::parse(&snapshot.text)
            .expect("renamed mmap must retain one valid root");
        assert_eq!(tree.root.title, "项目路线图");
    }

    #[test]
    fn tab_after_typing_into_an_empty_mindmap_root_creates_a_child() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created mmap should render its empty root");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        let Some(tab) = app.document_runtime.editor_runtime.tab_session_mut(tab_id) else {
            panic!("created mmap should keep its runtime session");
        };
        tab.document.cursor_mut().selection_anchor = Some(0);
        tab.document.cursor_move_to_offset(1);

        app.commit_editor_text("kk".to_owned());
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("typed mmap root should remain available")
                .text,
            "#kk"
        );

        let mapped_tab = appkit_shell::window_input::winit_key_to_keycode(
            &winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab),
            Some("\t"),
        )
        .expect("native Tab should map to an editor key");
        let tab_event = ui::Event::KeyDown(mapped_tab, ui::core::Modifiers::NONE);
        if !app.route_product_event(&tab_event) {
            app.handle_editor_key_input(mapped_tab, ui::core::Modifiers::NONE);
        }

        assert_eq!(
            app.document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("indented mmap root should remain available")
                .text,
            "#kk\n##\n"
        );
    }

    #[test]
    fn tab_from_an_untitled_new_mindmap_root_creates_a_child_without_spaces() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created mmap should render its empty root editor");
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("created mmap should have a runtime session")
                .plugin_name(),
            ui::plugin::PLUGIN_MINDMAP
        );
        assert!(app.frame_runtime.shell.editor_title_text().is_empty());

        let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        assert!(app.route_product_event(&tab_event));
        drain_until_document_text(&mut app, tab_id, "# 无标题", deadline);
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("created mmap should retain its runtime session")
                .document
                .cursor_offset()
                .to_usize(),
            "# 无标题".len()
        );
        app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);

        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap source should remain available");
        assert_eq!(snapshot.text, "# 无标题\n##\n");
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
    }

    #[test]
    fn tab_from_a_new_mindmap_title_hands_input_to_the_canvas_and_creates_a_child() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created mmap should render its title editor");
        assert!(app.route_product_event(&ui::Event::ImeCommit("项目路线图".to_owned())));
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::EditorTitle);
        assert_eq!(app.frame_runtime.shell.editor_title_text(), "项目路线图");

        let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        assert!(app.route_product_event(&tab_event));
        drain_until_document_text(&mut app, tab_id, "# 项目路线图", deadline);
        app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);

        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap source should remain available");
        assert_eq!(snapshot.text, "# 项目路线图\n##\n");
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);

        app.commit_editor_text("子节点".to_owned());
        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap child should remain available");
        assert_eq!(snapshot.text, "# 项目路线图\n##子节点\n");

        let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        if !app.route_product_event(&tab_event) {
            app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        }
        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap grandchild should remain available");
        assert_eq!(snapshot.text, "# 项目路线图\n##子节点\n###\n");
    }

    #[test]
    fn tab_from_a_new_markdown_title_focuses_the_body_without_indenting_the_heading() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created markdown should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created markdown should render its title editor");
        assert!(app.route_product_event(&ui::Event::ImeCommit("项目记录".to_owned())));

        let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        assert!(app.route_product_event(&tab_event));
        drain_until_document_text(&mut app, tab_id, "# 项目记录\n\n", deadline);

        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created markdown source should remain available");
        assert_eq!(snapshot.text, "# 项目记录\n\n");
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
    }

    #[test]
    fn source_view_backspace_and_delete_edit_graphemes_through_product_routing() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.dispatch_action(NotoraAction::TitleCommitRequested("项目路线图".to_owned()));
        drain_until_document_text(&mut app, tab_id, "# 项目路线图", deadline);
        app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("source view should keep the runtime session")
                .plugin_name(),
            ui::plugin::PLUGIN_EDITOR
        );

        let backspace_event = ui::Event::KeyDown(ui::KeyCode::Backspace, ui::core::Modifiers::NONE);
        if !app.route_product_event(&backspace_event) {
            app.handle_editor_key_input(ui::KeyCode::Backspace, ui::core::Modifiers::NONE);
        }
        app.document_runtime
            .editor_runtime
            .tab_session_mut(tab_id)
            .expect("source view should keep the mutable runtime session")
            .document
            .cursor_move_to_offset("# ".len());
        let delete_event = ui::Event::KeyDown(ui::KeyCode::Delete, ui::core::Modifiers::NONE);
        if !app.route_product_event(&delete_event) {
            app.handle_editor_key_input(ui::KeyCode::Delete, ui::core::Modifiers::NONE);
        }

        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("source text should remain available");
        assert_eq!(snapshot.text, "# 目路线");
    }

    #[test]
    fn markdown_and_mindmap_toolbar_actions_toggle_real_source_views() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        let mut previous_identity = None;
        for (kind, visual_plugin) in [
            (DocumentKind::Markdown, ui::plugin::PLUGIN_MARKDOWN_EDITOR),
            (DocumentKind::Mindmap, ui::plugin::PLUGIN_MINDMAP),
        ] {
            app.dispatch_action(NotoraAction::CreateRequested(kind));
            let deadline = Instant::now() + Duration::from_secs(2);
            let (identity, tab_id) = loop {
                app.drain_product_events();
                if let Some(identity) = app.action_runtime.state().library.selected_card
                    && Some(identity) != previous_identity
                    && let Some(tab_id) = app.document_tab_for(identity)
                {
                    break (identity, tab_id);
                }
                assert!(Instant::now() < deadline, "created note should install its visual view");
                thread::sleep(Duration::from_millis(10));
            };
            previous_identity = Some(identity);
            assert_eq!(
                app.document_runtime
                    .editor_runtime
                    .tab_session(tab_id)
                    .expect("created note should have a runtime session")
                    .plugin_name(),
                visual_plugin
            );

            app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
            assert_eq!(
                app.document_runtime
                    .editor_runtime
                    .tab_session(tab_id)
                    .expect("source view should keep the runtime session")
                    .plugin_name(),
                ui::plugin::PLUGIN_EDITOR
            );

            app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
            assert_eq!(
                app.document_runtime
                    .editor_runtime
                    .tab_session(tab_id)
                    .expect("visual view should be restorable")
                    .plugin_name(),
                visual_plugin
            );
        }
    }

    #[test]
    fn mindmap_style_actions_open_the_panel_and_write_theme_metadata() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Mindmap));
        let deadline = Instant::now() + Duration::from_secs(2);
        let tab_id = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };

        app.dispatch_action(NotoraAction::ToggleMindmapStylePanelRequested);
        app.render().expect("open mmap style panel should render");
        assert!(
            app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("mmap runtime session should exist")
                .mindmap_style_panel()
                .is_visible()
        );

        app.dispatch_action(NotoraAction::MindmapStylePanel(
            ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".to_owned()),
        ));
        app.render().expect("selected mmap theme should render");
        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("themed mmap source should exist");
        assert!(snapshot.text.contains("theme = \"tide\""));

        app.dispatch_action(NotoraAction::MindmapStylePanel(
            ui::core::widget::MindmapStylePanelAction::Close,
        ));
        assert!(
            !app.document_runtime
                .editor_runtime
                .tab_session(tab_id)
                .expect("mmap runtime session should remain available")
                .mindmap_style_panel()
                .is_visible()
        );
    }

    #[test]
    fn title_text_change_does_not_update_the_active_note_before_explicit_commit() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        let identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card {
                break identity;
            }
            assert!(Instant::now() < deadline, "note completion should select a note");
            thread::sleep(Duration::from_millis(10));
        };
        let tab_id = loop {
            app.drain_product_events();
            if let Some(tab_id) = app.document_tab_for(identity) {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created note should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };

        app.dispatch_action(NotoraAction::TitleTextChanged("项目路线图".to_owned()));

        let snapshot = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created note text should remain available");
        assert_eq!(snapshot.text, "");
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::EditorTitle);
    }

    #[test]
    fn completed_tag_mutation_refreshes_active_editor_chips() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");

        app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(2);
        let note_id = loop {
            app.drain_product_events();
            if let Some(DocumentIdentity::Note(note_id)) =
                app.action_runtime.state().library.selected_card
                && app.action_runtime.state().library.active_editor_metadata.is_some()
            {
                break note_id;
            }
            assert!(Instant::now() < deadline, "created note metadata should load");
            thread::sleep(Duration::from_millis(10));
        };

        app.dispatch_action(NotoraAction::MetadataMutationRequested(
            MetadataMutation::AttachTagByName { note_id, display_name: "产品/Notora".to_owned() },
        ));

        loop {
            app.drain_product_events();
            let labels = app
                .state()
                .library
                .active_editor_metadata
                .as_ref()
                .map(|snapshot| {
                    snapshot.tags.iter().map(|tag| tag.display_name.as_str()).collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if labels == ["产品/Notora"] {
                break;
            }
            assert!(Instant::now() < deadline, "completed tag mutation should refresh chips");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn new_note_button_is_disabled_until_a_workspace_root_is_selected() {
        let mut app = app();
        app.render().expect("headless shell frame should render");
        let layout = app.shell_layout();
        let click_x = layout.card_list_rect.right() - 76.0 * layout.dpi;
        let click_y = 22.0 * layout.dpi;

        app.route_product_event(&ui::Event::MouseDown {
            px: click_x,
            py: click_y,
            button: ui::core::widget::MouseButton::Left,
        });
        app.route_product_event(&ui::Event::MouseUp {
            px: click_x,
            py: click_y,
            button: ui::core::widget::MouseButton::Left,
        });

        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);
        assert_eq!(app.action_runtime.state().layout.focus_target, FocusTarget::NavigationTree);
    }

    #[test]
    fn workspace_root_selection_is_completed_before_note_creation_becomes_available() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let selected_workspace_root = workspace_directory.path().to_path_buf();
        let mut app = app();
        app.workspace_directory_chooser = Box::new(move || Some(selected_workspace_root.clone()));

        app.dispatch_action(NotoraAction::WorkspaceRootSelectionRequested);

        assert_eq!(app.action_runtime.state().workspace_root, WorkspaceRootState::Active);
        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::None);

        app.dispatch_action(NotoraAction::OpenNewDocumentMenu);
        assert_eq!(app.action_runtime.state().layout.overlay, OverlayState::NewDocumentMenu);
    }

    #[test]
    fn completed_note_creation_is_persisted_and_restored_after_restart() {
        let configuration_directory =
            tempfile::tempdir().expect("configuration fixture should be created");
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        let paths =
            NotoraPaths::from_config_directory(configuration_directory.path().join("notora"))
                .expect("isolated product paths should be created");
        let mut first_app =
            NotoraRuntime::with_paths(paths.clone()).expect("first app should construct");
        let selected_workspace_root = workspace_directory.path().to_path_buf();
        first_app.workspace_directory_chooser =
            Box::new(move || Some(selected_workspace_root.clone()));

        assert!(first_app.select_workspace_root());
        first_app.persistence_runtime.session_persist_at = Some(Instant::now());
        first_app.process_due_session_persistence();

        let workspace_session_deadline = Instant::now() + Duration::from_secs(2);
        while crate::session::load_product_session(&paths.session_file)
            .session
            .workspace_root
            .is_none()
        {
            assert!(
                Instant::now() < workspace_session_deadline,
                "selected workspace should be persisted before note completion"
            );
            thread::sleep(Duration::from_millis(10));
        }

        first_app.dispatch_action(NotoraAction::CreateRequested(DocumentKind::Markdown));
        let creation_deadline = Instant::now() + Duration::from_secs(2);
        let note_id = loop {
            first_app.drain_product_events();
            if let Some(DocumentIdentity::Note(note_id)) =
                first_app.action_runtime.state().library.selected_card
                && first_app.document_tab_for(DocumentIdentity::Note(note_id)).is_some()
            {
                break note_id;
            }
            assert!(Instant::now() < creation_deadline, "created note should load promptly");
            thread::sleep(Duration::from_millis(10));
        };

        assert!(
            first_app.persistence_runtime.session_persist_at.is_some(),
            "note completion should schedule persistence for the new last document"
        );
        assert_eq!(
            first_app.capture_product_session().last_document,
            Some(crate::session::SavedDocument::Note { note_id })
        );
        first_app.persistence_runtime.session_persist_at = Some(Instant::now());
        first_app.process_due_session_persistence();
        first_app.persistence_runtime.worker.shutdown();
        first_app.drain_product_events();
        assert_eq!(first_app.action_runtime.state().library.last_command_error, None);
        drop(first_app);
        let note_session_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let loaded_session = crate::session::load_product_session(&paths.session_file);
            if loaded_session.session.last_document
                == Some(crate::session::SavedDocument::Note { note_id })
            {
                break;
            }
            assert!(
                Instant::now() < note_session_deadline,
                "created note should become the persisted last document: {loaded_session:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let mut restarted_app =
            NotoraRuntime::with_paths(paths).expect("restarted app should construct");
        restarted_app.restore_pending_session();
        let restore_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            restarted_app.drain_product_events();
            let identity = DocumentIdentity::Note(note_id);
            if card_page_contains_note(
                &restarted_app.action_runtime.state().library.card_page,
                note_id,
            ) && restarted_app.document_tab_for(identity).is_some()
            {
                break;
            }
            assert!(
                Instant::now() < restore_deadline,
                "restarted app should restore the new note in the workspace"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(restarted_app.action_runtime.state().layout.focus_target, FocusTarget::Editor);
        assert_eq!(
            restarted_app.action_runtime.state().layout.compact_content,
            CompactContent::Editor
        );
        assert!(matches!(
            restarted_app.render_frame().expect("restored note frame should compose"),
            appkit_shell::editor_runtime::EditorSurfacePaint::Document { .. }
        ));
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
                app.action_runtime.state().library.selected_card
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
            app.document_runtime
                .editor_runtime
                .document_summary(tab_id)
                .expect("tab should remain available")
                .dirty
        );

        app.dispatch_action(NotoraAction::TrashOperationRequested(
            crate::action::TrashOperation::MoveToTrash { note_id },
        ));

        assert!(workspace_directory.path().join("无标题.md").is_file());
        assert_eq!(app.document_tab_for(identity), Some(tab_id));
        assert_eq!(app.action_runtime.state().library.selected_card, Some(identity));
        assert!(app.document_runtime.pending_trash_moves.is_empty());
        assert!(
            app.action_runtime
                .state()
                .library
                .last_command_error
                .as_deref()
                .is_some_and(|message| message.contains("未移入回收站"))
        );
    }

    #[test]
    fn dirty_note_move_waits_for_save_before_worker_command() {
        let workspace_directory = tempfile::tempdir().expect("workspace fixture should be created");
        let archive_directory = workspace_directory.path().join("archive");
        std::fs::create_dir(&archive_directory).expect("move target directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));

        let deadline = Instant::now() + Duration::from_secs(2);
        let (note_id, tab_id, original_path) = loop {
            app.drain_product_events();
            if let Some(notora_core::DocumentIdentity::Note(note_id)) =
                app.action_runtime.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
                && let Some(path) = app
                    .document_runtime
                    .editor_runtime
                    .document_summary(tab_id)
                    .and_then(|summary| summary.path)
            {
                break (note_id, tab_id, path);
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        app.commit_editor_text("unsaved edit".to_owned());
        assert!(
            app.document_runtime
                .editor_runtime
                .document_summary(tab_id)
                .expect("note tab should remain available")
                .dirty
        );

        app.dispatch_action(NotoraAction::MoveRequested {
            note_id,
            target_directory: std::path::PathBuf::from("archive"),
        });

        let moved_path = archive_directory
            .join(original_path.file_name().expect("created note should have a file name"));
        let move_check_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < move_check_deadline
            && original_path.is_file()
            && !moved_path.is_file()
        {
            app.drain_product_events();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(original_path.is_file());
        assert!(!moved_path.is_file());
        assert!(
            app.action_runtime
                .state()
                .library
                .last_command_error
                .as_deref()
                .is_some_and(|message| message.contains("未移动"))
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
                app.action_runtime.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
                && let Some(summary) = app.document_runtime.editor_runtime.document_summary(tab_id)
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
    fn pending_note_move_rejects_a_document_changed_after_its_save_started() {
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
                app.action_runtime.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
                && let Some(summary) = app.document_runtime.editor_runtime.document_summary(tab_id)
            {
                break (note_id, tab_id, summary.content_revision);
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        let pending_move = PendingNoteMove {
            request: notora_core::note_command::MoveNoteRequest {
                note_id,
                target_directory: std::path::PathBuf::from("archive"),
            },
            content_revision: saved_revision,
        };
        assert!(app.pending_note_move_has_current_saved_document(tab_id, &pending_move));

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        app.commit_editor_text("newer edit".to_owned());

        assert!(!app.pending_note_move_has_current_saved_document(tab_id, &pending_move));
    }

    #[test]
    fn pending_title_update_rejects_a_document_changed_after_its_save_started() {
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
                app.action_runtime.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
                && let Some(summary) = app.document_runtime.editor_runtime.document_summary(tab_id)
            {
                break (note_id, tab_id, summary.content_revision);
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        let pending_update = PendingTitleUpdate {
            request: notora_core::UpdateNoteTitleRequest {
                note_id,
                expected_title_revision: 0,
                title: "新标题".to_owned(),
            },
            content_revision: saved_revision,
        };
        assert!(app.pending_title_update_has_current_saved_document(tab_id, &pending_update));

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        app.commit_editor_text("newer edit".to_owned());

        assert!(!app.pending_title_update_has_current_saved_document(tab_id, &pending_update));
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
                app.action_runtime.state().library.selected_card
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

        assert!(!workspace_directory.path().join("无标题.md").exists());
        assert_eq!(app.action_runtime.state().library.selected_card, None);
        assert_eq!(app.editor_runtime_tab_count(), 0);
        assert_eq!(app.action_runtime.state().library.last_command_error, None);
    }

    #[test]
    fn selecting_a_trashed_note_loads_a_read_only_runtime_document() {
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
            if let Some(DocumentIdentity::Note(note_id)) =
                app.action_runtime.state().library.selected_card
                && app.document_tab_for(DocumentIdentity::Note(note_id)).is_some()
            {
                break note_id;
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        let identity = DocumentIdentity::Note(note_id);
        app.dispatch_action(NotoraAction::TrashOperationRequested(
            crate::action::TrashOperation::MoveToTrash { note_id },
        ));
        loop {
            app.drain_product_events();
            if app.document_tab_for(identity).is_none() {
                break;
            }
            assert!(Instant::now() < deadline, "trash move should close the editable tab");
            thread::sleep(Duration::from_millis(10));
        }

        app.dispatch_action(NotoraAction::NavigationSelected(NavigationScope::Trash));
        loop {
            app.drain_product_events();
            if matches!(
                &app.action_runtime.state().library.card_page,
                CardPageState::Ready { cards, .. }
                    if cards.iter().any(|card| card.note_id == note_id)
            ) {
                break;
            }
            assert!(Instant::now() < deadline, "trash card should load promptly");
            thread::sleep(Duration::from_millis(10));
        }
        app.dispatch_action(NotoraAction::CardSelected(identity));
        let tab_id = loop {
            app.drain_product_events();
            if let Some(tab_id) = app.document_tab_for(identity) {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "trash document should install promptly");
            thread::sleep(Duration::from_millis(10));
        };
        let before = app
            .document_runtime
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("trash document text should be available");

        assert!(
            !app.document_runtime
                .editor_runtime
                .workspace_snapshot()
                .tabs
                .iter()
                .find(|tab| tab.tab_id == tab_id)
                .expect("trash runtime tab should exist")
                .allows_editing
        );
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        app.commit_editor_text("禁止写入".to_owned());
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .document_text_snapshot(tab_id)
                .expect("trash document text should remain available")
                .text,
            before.text
        );

        app.dispatch_action(NotoraAction::TrashOperationRequested(
            crate::action::TrashOperation::Restore { note_id },
        ));
        loop {
            app.drain_product_events();
            if app.document_tab_for(identity).is_none()
                && app.action_runtime.state().library.selected_card.is_none()
                && workspace_directory.path().join("无标题.md").is_file()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "restore should close the read-only tab and clear its selection"
            );
            thread::sleep(Duration::from_millis(10));
        }
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
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "selected note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(app.editor_runtime_tab_count(), 1);
        assert!(app.document_tab_for(selected_identity).is_some());
        assert_eq!(app.action_runtime.state().library.last_command_error, None);
    }

    #[test]
    fn external_note_relocation_rebinds_the_open_runtime_and_refreshes_title_revision() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let mut app = app();
        app.execute_workspace_command(WorkspaceCommand::OpenExisting {
            root: workspace_directory.path().to_path_buf(),
        })
        .expect("workspace should open");
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Markdown));
        let deadline = Instant::now() + Duration::from_secs(3);
        let (note_id, tab_id) = loop {
            app.drain_product_events();
            if let Some(notora_core::DocumentIdentity::Note(note_id)) =
                app.action_runtime.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
                && app.action_runtime.state().library.active_editor_metadata.is_some()
            {
                break (note_id, tab_id);
            }
            assert!(Instant::now() < deadline, "created note should install a preview");
            thread::sleep(Duration::from_millis(10));
        };
        let original_path = workspace_directory.path().join("无标题.md");
        let finder_path = workspace_directory.path().join("Finder 改名.md");
        std::fs::rename(&original_path, &finder_path).expect("Finder rename should succeed");
        let active_metadata = app
            .state()
            .library
            .active_editor_metadata
            .as_ref()
            .expect("active metadata should be loaded");
        let mut relocated_metadata = active_metadata.metadata.clone();
        relocated_metadata.title_revision = 1;
        app.synchronize_external_note_relocations(vec![crate::product::WorkspaceNoteRelocation {
            note_id,
            from: "无标题.md".into(),
            to: "Finder 改名.md".into(),
            metadata: relocated_metadata,
            tags: active_metadata.tags.clone(),
        }]);

        assert_eq!(
            app.document_tab_for(notora_core::DocumentIdentity::Note(note_id)),
            Some(tab_id)
        );
        assert!(!original_path.exists());
        assert!(finder_path.is_file());
        assert_eq!(
            app.action_runtime
                .state()
                .library
                .active_editor_metadata
                .as_ref()
                .map(|snapshot| snapshot.metadata.title_revision),
            Some(1)
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
            if let Some(identity) = app.action_runtime.state().library.selected_card
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
            if let Some(identity) = app.action_runtime.state().library.selected_card
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
            if let Some(identity) = app.action_runtime.state().library.selected_card
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
            app.document_runtime.autosave.state(first_tab_id),
            Some(AutoSaveState::Scheduled { content_revision: 1, .. })
        ));
        app.dispatch_action(NotoraAction::CreateRequested(notora_core::DocumentKind::Text));

        loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
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
            app.action_runtime.state().library.selected_card,
            None,
            "external path validation and reads must not complete on the caller thread"
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        let identity = loop {
            app.drain_product_events();
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "external preview should install promptly");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            app.action_runtime.state().library.navigation_scope,
            notora_core::NavigationScope::ExternalFiles
        );
        assert!(matches!(identity, notora_core::DocumentIdentity::ExternalFile(_)));
        assert_eq!(app.editor_runtime_tab_count(), 1);
        assert!(app.document_tab_for(identity).is_some());
        assert_eq!(app.action_runtime.state().external_files.sessions().len(), 1);
        assert_eq!(app.action_runtime.state().library.last_command_error, None);
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
            if let Some(identity) = app.action_runtime.state().library.selected_card
                && app.document_tab_for(identity).is_some()
            {
                break identity;
            }
            assert!(Instant::now() < deadline, "external preview should install promptly");
            thread::sleep(Duration::from_millis(10));
        };
        let tab_id = app.document_tab_for(identity).expect("external file should have a tab");
        let initial_summary = app
            .document_runtime
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
            app.document_runtime
                .editor_runtime
                .document_summary(tab_id)
                .and_then(|summary| summary.disk_revision),
            initial_disk_revision,
            "revision capture must not run on the conflict action caller"
        );
        loop {
            app.drain_product_events();
            if app
                .document_runtime
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
        let outdated_generation = app.action_runtime.state().library.selected_document_generation;
        app.dispatch_action(NotoraAction::CardSelected(intervening_identity));
        app.dispatch_action(NotoraAction::CardSelected(selected_identity));
        let current_generation = app.action_runtime.state().library.selected_document_generation;

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
            app.document_runtime
                .editor_runtime
                .create_plugin_for_path(std::path::Path::new("draft.txt"))
                .name(),
            ui::plugin::PLUGIN_EDITOR
        );
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .create_plugin_for_path(std::path::Path::new("draft.md"))
                .name(),
            ui::plugin::PLUGIN_MARKDOWN_EDITOR
        );
        assert_eq!(
            app.document_runtime
                .editor_runtime
                .create_plugin_for_path(std::path::Path::new("draft.mmap.md"))
                .name(),
            ui::plugin::PLUGIN_MINDMAP
        );
    }
}
