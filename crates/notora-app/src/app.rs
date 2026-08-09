//! notora 窗口应用状态；编辑器会话只经 shared runtime 管理。

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use appkit_shell::editor_runtime::{
    DocumentTextEditError, DocumentTextReplacement, EditorNotification, EditorOutcome,
    EditorRuntime, EditorRuntimeConfig, EditorRuntimeError, OpenDisposition, RenderError,
    RenderResources,
};
use appkit_shell::render_state::{GpuState, TextState};
use appkit_shell::{ProductHost, ProductWakeHandle, ShellEffect, ShellEvent};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::WindowAttributes;

use crate::action::{
    CardQuery, ConflictResolution, DocumentLoadRequest, NoteCreationTarget, NotoraAction,
    SaveConflictRequest,
};
use crate::autosave::{AutoSaveRequest, AutoSaveScheduler, AutoSaveState};
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
use crate::render::{EditorPaneState, NotoraRenderModel, NotoraShell};
use crate::runtime_lru::{RuntimeLru, RuntimeTabState};
use crate::search_controller::SearchController;
use crate::shell::layout::{ShellLayout, ShellLayoutInput};
use crate::state::normalize_notora_title;
use crate::{
    NotoraPaths, NotoraPathsError, NotoraState, WorkspaceCommand, WorkspaceCommandResult,
    WorkspaceController, WorkspaceControllerError, WorkspaceRootState,
};
use notora_core::note_command::{ConfiguredCreateNoteRequest, MoveNoteRequest, NoteCommand};
use notora_core::{
    DocumentIdentity, DocumentKind, NoteEncryption, document_title_projection,
    replace_document_title,
};

const DEFAULT_WINDOW_WIDTH_PX: f32 = 1_200.0;
const DEFAULT_WINDOW_HEIGHT_PX: f32 = 800.0;
const SESSION_PERSIST_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);
const CATALOG_BACKUP_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);
const SHUTDOWN_SAVE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_SAVE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_RUNTIME_TAB_LIMIT: usize = 12;
const STARTUP_TRACE_ENVIRONMENT_VARIABLE: &str = "NOTORA_TRACE_STARTUP";
const TRASH_SAVE_FAILURE_MESSAGE: &str = "笔记保存失败，因此未移入回收站";
const TRASH_SAVE_STALE_MESSAGE: &str = "笔记在保存完成前发生变化，因此未移入回收站";
const MOVE_SAVE_FAILURE_MESSAGE: &str = "笔记保存失败，因此未移动";
const MOVE_SAVE_STALE_MESSAGE: &str = "笔记在保存完成前发生变化，因此未移动";
const SAVE_STATUS_SAVED: &str = "已保存";
const SAVE_STATUS_UNSAVED: &str = "未保存";
const SAVE_STATUS_PENDING: &str = "待保存";
const SAVE_STATUS_SAVING: &str = "保存中";
const SAVE_STATUS_FAILED: &str = "保存失败";

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

#[derive(Debug)]
struct StartupTrace {
    started_at: Instant,
    first_frame_reported: bool,
}

enum FontSystemPreparation {
    Deferred,
    InProgress(thread::JoinHandle<shaping::FontSystem>),
}

impl StartupTrace {
    fn from_environment() -> Option<Self> {
        std::env::var_os(STARTUP_TRACE_ENVIRONMENT_VARIABLE).is_some().then(Self::started_now)
    }

    fn started_now() -> Self {
        Self { started_at: Instant::now(), first_frame_reported: false }
    }

    fn record_stage(&self, label: &str, stage_started_at: Instant) {
        eprintln!(
            "[startup] {label} stage={:.2}ms total={:.2}ms",
            stage_started_at.elapsed().as_secs_f64() * 1_000.0,
            self.started_at.elapsed().as_secs_f64() * 1_000.0,
        );
    }

    fn take_first_frame_elapsed(&mut self) -> Option<Duration> {
        if self.first_frame_reported {
            return None;
        }
        self.first_frame_reported = true;
        Some(self.started_at.elapsed())
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingNoteMove {
    request: MoveNoteRequest,
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

/// 组合产品状态、后台宿主和共享 editor runtime 的 notora 应用。
pub struct NotoraApp {
    startup_trace: Option<StartupTrace>,
    font_system_preparation: FontSystemPreparation,
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
    workspace_directory_chooser: WorkspaceDirectoryChooser,
    document_registry: DocumentRegistry,
    autosave: AutoSaveScheduler,
    save_failure_messages: HashMap<appkit_core::workspace::types::TabId, String>,
    pending_external_save_as: HashMap<appkit_core::workspace::types::TabId, PendingExternalSaveAs>,
    pending_external_documents: HashMap<notora_core::ExternalFileId, LoadedDocument>,
    pending_conflict_retries: HashMap<appkit_core::workspace::types::TabId, PendingConflictRetry>,
    pending_trash_moves: HashMap<appkit_core::workspace::types::TabId, PendingTrashMove>,
    pending_note_moves: HashMap<appkit_core::workspace::types::TabId, PendingNoteMove>,
    pending_metadata_generations: HashMap<notora_core::NoteId, VecDeque<u64>>,
    pending_metadata_mutations: Vec<crate::action::MetadataMutation>,
    catalog_reconciliation_pending: bool,
    search_controller: SearchController,
    editor_runtime: EditorRuntime,
    shell: NotoraShell,
    window_focused: bool,
    window_width_px: f32,
    window_height_px: f32,
    pointer_position: (f32, f32),
    last_editor_cursor_visible: bool,
    needs_redraw: bool,
    event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
}

impl NotoraApp {
    pub fn new() -> Self {
        Self::try_new()
            .expect("notora must construct its isolated configuration and editor runtime")
    }

    pub fn try_new() -> Result<Self, NotoraAppError> {
        let startup_trace = StartupTrace::from_environment();
        let paths = NotoraPaths::from_platform_directory().map_err(NotoraAppError::Paths)?;
        let mut app = Self::with_paths_and_startup_trace(paths, startup_trace)?;
        app.editor_runtime.start_gpu_preparation().map_err(NotoraAppError::Runtime)?;
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
            startup_trace,
            font_system_preparation: FontSystemPreparation::Deferred,
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
            workspace_directory_chooser: Box::new(choose_workspace_directory),
            document_registry: DocumentRegistry::default(),
            autosave: AutoSaveScheduler::with_clock_and_idle_delay(
                crate::autosave::SystemAutoSaveClock,
                auto_save_delay,
            ),
            save_failure_messages: HashMap::new(),
            pending_external_save_as: HashMap::new(),
            pending_external_documents: HashMap::new(),
            pending_conflict_retries: HashMap::new(),
            pending_trash_moves: HashMap::new(),
            pending_note_moves: HashMap::new(),
            pending_metadata_generations: HashMap::new(),
            pending_metadata_mutations: Vec::new(),
            catalog_reconciliation_pending: false,
            search_controller: SearchController::default(),
            editor_runtime,
            shell: NotoraShell::new(),
            window_focused: true,
            window_width_px: DEFAULT_WINDOW_WIDTH_PX,
            window_height_px: DEFAULT_WINDOW_HEIGHT_PX,
            pointer_position: (0.0, 0.0),
            last_editor_cursor_visible: true,
            needs_redraw: true,
            event_loop_proxy: None,
        };
        app.synchronize_product_focus();
        if let Some(trace) = &app.startup_trace {
            trace.record_stage("application_constructed", trace.started_at);
        }
        Ok(app)
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
            WorkspaceCommandResult::Opened(workspace) => {
                self.state.workspace_root = WorkspaceRootState::Active;
                self.search_controller
                    .set_active_workspace(workspace.descriptor.workspace_id, workspace.generation);
            }
            WorkspaceCommandResult::Closed { .. } => {
                self.state.workspace_root = WorkspaceRootState::Missing;
                self.search_controller.clear_active_workspace();
            }
            WorkspaceCommandResult::Unchanged => {}
        }
        if !matches!(result, WorkspaceCommandResult::Unchanged) {
            self.autosave.clear();
            self.save_failure_messages.clear();
            self.pending_external_save_as.clear();
            self.pending_conflict_retries.clear();
            self.pending_trash_moves.clear();
            self.pending_note_moves.clear();
            self.pending_metadata_generations.clear();
            self.pending_metadata_mutations.clear();
            self.catalog_reconciliation_pending = false;
        }
        if matches!(result, WorkspaceCommandResult::Opened(_)) {
            self.request_navigation_tree();
        }
        self.needs_redraw = true;
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
        if self.action_will_leave_title_focus(&action) {
            self.commit_title_before_focus();
        }
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
        self.synchronize_product_focus();
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

    fn action_will_leave_title_focus(&self, action: &NotoraAction) -> bool {
        if self.state.layout.focus_target != crate::FocusTarget::EditorTitle
            || self.state.library.title_draft.is_none()
            || matches!(
                action,
                NotoraAction::TitleTextChanged(_) | NotoraAction::TitleCommitRequested(_)
            )
        {
            return false;
        }

        let selected_document = self.state.library.selected_card;
        let mut next_state = self.state.clone();
        let _ = next_state.reduce(action.clone());
        next_state.layout.focus_target != crate::FocusTarget::EditorTitle
            || next_state.library.selected_card != selected_document
    }

    fn synchronize_product_focus(&mut self) {
        let focus_target = self.state.layout.focus_target;
        self.editor_runtime
            .set_active_cursor_paint_enabled(focus_target == crate::FocusTarget::Editor);
        self.shell.synchronize_focus(focus_target, Instant::now());
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

    pub(crate) fn apply_canvas_viewport_action_at(
        &mut self,
        px: f32,
        py: f32,
        action: appkit_shell::canvas_viewport::CanvasViewportAction,
    ) -> bool {
        let context =
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused);
        if context.modal_blocked || !context.editor_rect.contains(px, py) {
            return false;
        }
        let outcome = self.editor_runtime.apply_active_canvas_viewport_action(action);
        let applied = outcome.shell_effect.redraw;
        self.apply_editor_outcome(outcome);
        applied
    }

    fn handle_canvas_scrollbar_action(
        &mut self,
        action: ui::canvas_scrollbars::CanvasScrollbarsAction,
    ) {
        use appkit_shell::canvas_viewport::CanvasViewportAction;
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
            | ScrollbarAction::HoverChanged(_) => {
                if self.editor_runtime.active_canvas_viewport_snapshot().is_some() {
                    self.apply_shell_effect(ShellEffect::REDRAW);
                }
                return;
            }
        };
        let outcome = self.editor_runtime.apply_active_canvas_viewport_action(viewport_action);
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
        let text_cursor_blink_due = self.shell.advance_text_cursor_blink(Instant::now());
        let editor_cursor_blink_due = match self.editor_cursor_blink_phase() {
            Some(phase) => {
                let changed = phase.visible != self.last_editor_cursor_visible;
                self.last_editor_cursor_visible = phase.visible;
                changed
            }
            None => {
                self.last_editor_cursor_visible = true;
                false
            }
        };
        std::mem::take(&mut self.needs_redraw)
            || self.editor_runtime.take_redraw_request()
            || text_cursor_blink_due
            || editor_cursor_blink_due
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
        let text_cursor_blink_at =
            if self.window_focused { self.shell.next_text_cursor_blink_at() } else { None };
        let editor_cursor_blink_at =
            self.editor_cursor_blink_phase().map(|phase| phase.next_transition_at);
        [
            self.autosave.next_deadline(),
            self.search_controller.next_deadline(),
            self.pending_session_persist_at,
            self.pending_catalog_backup_at,
            text_cursor_blink_at,
            editor_cursor_blink_at,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn editor_cursor_blink_phase(
        &self,
    ) -> Option<appkit_shell::editor_runtime::EditorCursorBlinkPhase> {
        if self.state.layout.focus_target != crate::FocusTarget::Editor {
            return None;
        }
        self.editor_runtime.active_cursor_blink_phase()
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
            let failure_message =
                completion.result.as_ref().err().map(std::string::ToString::to_string);
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
            let pending_note_move = self
                .pending_note_moves
                .get(&request.tab_id)
                .cloned()
                .filter(|pending| pending.content_revision == request.content_revision);
            if completed_conflict_retry.is_some() {
                self.pending_conflict_retries.remove(&request.tab_id);
            }
            let saved_path = completion.result.as_ref().ok().map(|revision| revision.path.clone());
            let outcome = self.editor_runtime.apply_save_completion(completion);
            self.apply_editor_outcome(outcome);
            self.complete_pending_external_save_as(request, save_succeeded, saved_path);
            if save_succeeded {
                self.save_failure_messages.remove(&request.tab_id);
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
                if let Some(pending_note_move) = pending_note_move {
                    if self.pending_note_move_has_current_saved_document(
                        request.tab_id,
                        &pending_note_move,
                    ) {
                        self.pending_note_moves.remove(&request.tab_id);
                        self.submit_note_command(NoteCommand::Move(pending_note_move.request));
                    } else {
                        self.cancel_pending_note_move(request, MOVE_SAVE_STALE_MESSAGE);
                    }
                }
                if let Some(retry) = completed_conflict_retry {
                    self.dispatch_action(NotoraAction::SaveConflictResolved {
                        identity: retry.identity,
                    });
                }
            } else {
                if let Some(message) = failure_message {
                    self.record_autosave_failure(request, message);
                } else {
                    self.autosave.on_save_failed(request);
                }
                self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
                self.cancel_pending_note_move(request, MOVE_SAVE_FAILURE_MESSAGE);
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

    pub(crate) fn route_pointer_event(&mut self, event: &ui::Event) -> bool {
        let (product_consumed, product_cursor) = self.route_product_event_with_feedback(event);
        let pointer_move = matches!(event, ui::Event::MouseMove { .. });
        let editor_cursor = if pointer_move
            || !product_consumed
            || self.editor_pointer_is_captured()
        {
            let context =
                events::editor_input_context(&self.state, self.shell_layout(), self.window_focused);
            let outcome = self.editor_runtime.handle_pointer_event(context, event);
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
        self.editor_runtime.pointer_capture() != appkit_shell::editor_runtime::MouseCapture::None
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
        let font_system_started_at = Instant::now();
        let font_system = Arc::new(Mutex::new(self.take_prepared_font_system()));
        if let Some(trace) = &self.startup_trace {
            trace.record_stage("font_system_ready", font_system_started_at);
        }
        self.editor_runtime.set_shared_font_system(Arc::clone(&font_system));
        let editor_runtime_resume_started_at = Instant::now();
        self.editor_runtime
            .resume(
                event_loop,
                WindowAttributes::default().with_title("notora").with_min_inner_size(
                    LogicalSize::new(
                        crate::shell::layout::MINIMUM_WINDOW_WIDTH_LOGICAL,
                        crate::shell::layout::MINIMUM_WINDOW_HEIGHT_LOGICAL,
                    ),
                ),
                font_system,
                self.settings.font_size,
                &self.settings.font_family,
            )
            .map_err(NotoraAppError::Runtime)?;
        if let Some(trace) = &self.startup_trace {
            trace.record_stage("window_gpu_text_ready", editor_runtime_resume_started_at);
        }
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

    fn start_font_system_preparation(&mut self) {
        if matches!(self.font_system_preparation, FontSystemPreparation::InProgress(_)) {
            return;
        }
        let cache_path = self.paths.config_directory.join("font-cache.bin");
        match thread::Builder::new()
            .name("notora-font-system-preparation".to_owned())
            .spawn(move || shaping::font_cache::new_font_system_with_cache(&cache_path))
        {
            Ok(worker) => {
                self.font_system_preparation = FontSystemPreparation::InProgress(worker);
            }
            Err(error) => {
                eprintln!(
                    "notora font preparation worker unavailable; falling back to synchronous initialization: {error}"
                );
            }
        }
    }

    fn take_prepared_font_system(&mut self) -> shaping::FontSystem {
        let preparation =
            std::mem::replace(&mut self.font_system_preparation, FontSystemPreparation::Deferred);
        if let FontSystemPreparation::InProgress(worker) = preparation
            && let Ok(font_system) = worker.join()
        {
            return font_system;
        }
        shaping::font_cache::new_font_system_with_cache(
            &self.paths.config_directory.join("font-cache.bin"),
        )
    }

    pub(crate) fn record_first_frame_visible(&mut self) {
        let Some(trace) = self.startup_trace.as_mut() else {
            return;
        };
        let Some(elapsed) = trace.take_first_frame_elapsed() else {
            return;
        };
        eprintln!("[startup] first_frame_visible total={:.2}ms", elapsed.as_secs_f64() * 1_000.0,);
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
                crate::product::NotoraProductEvent::MetadataMutationCompleted {
                    mutation,
                    note_id,
                    metadata,
                    tags,
                    outcome,
                    ..
                } => {
                    remove_pending_metadata_mutation(
                        &mut self.pending_metadata_mutations,
                        &mutation,
                    );
                    let Some(selection_generation) = self.take_pending_metadata_generation(note_id)
                    else {
                        continue;
                    };
                    self.apply_title_initialization_outcome(&mutation, outcome, note_id);
                    if self.state.library.selected_card != Some(DocumentIdentity::Note(note_id))
                        || self.state.library.selected_document_generation != selection_generation
                    {
                        continue;
                    }
                    self.schedule_catalog_backup();
                    self.request_navigation_tree();
                    self.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
                        request: crate::action::DocumentLoadRequest {
                            identity: DocumentIdentity::Note(note_id),
                            selection_generation,
                        },
                        metadata: metadata.clone(),
                        tags,
                    });
                    self.dispatch_action(NotoraAction::MetadataMutationCompleted {
                        note_id,
                        metadata,
                        selection_generation,
                    });
                }
                crate::product::NotoraProductEvent::MetadataMutationFailed {
                    mutation,
                    message,
                    ..
                } => {
                    remove_pending_metadata_mutation(
                        &mut self.pending_metadata_mutations,
                        &mutation,
                    );
                    self.take_pending_metadata_generation(metadata_mutation_note_id(&mutation));
                    self.dispatch_action(NotoraAction::MetadataMutationFailed(message));
                }
                crate::product::NotoraProductEvent::CatalogBackupCompleted { .. } => {}
                crate::product::NotoraProductEvent::CatalogBackupFailed { message, .. } => {
                    self.dispatch_action(NotoraAction::MetadataMutationFailed(format!(
                        "元数据已保存，但目录索引备份失败：{message}"
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
                    request,
                    document,
                    metadata,
                    tags,
                    ..
                } => {
                    self.install_loaded_preview(request, document);
                    self.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
                        request,
                        metadata,
                        tags,
                    });
                }
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
                    self.request_navigation_tree();
                    self.dispatch_action(NotoraAction::CatalogReindexed);
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
        let focus_target = self.state.layout.focus_target;
        let product_modal_is_open = self.state.layout.overlay != crate::state::OverlayState::None;
        if !product_modal_is_open
            && focus_target == crate::FocusTarget::EditorTitle
            && matches!(event, ui::Event::KeyDown(ui::KeyCode::Tab, _))
        {
            self.commit_title_before_focus();
            self.dispatch_action(NotoraAction::FocusRequested(crate::FocusTarget::Editor));
            return (true, None);
        }
        let route = self.shell.route_event_with_overlay(
            event,
            focus_target,
            self.state.layout.overlay,
            &self.theme,
            self.editor_runtime.scale_factor() as f32,
        );
        let cursor_hint = route.cursor_hint;
        if route.consumed {
            self.apply_shell_effect(ShellEffect::REDRAW);
        }
        if let Some(action) = route.canvas_scrollbar_action {
            self.handle_canvas_scrollbar_action(action);
        }
        for action in route.actions {
            self.dispatch_action(action);
        }
        (route.consumed, cursor_hint)
    }

    fn commit_title_before_focus(&mut self) {
        if !matches!(self.state.library.selected_card, Some(DocumentIdentity::Note(_))) {
            return;
        }
        self.dispatch_action(NotoraAction::TitleCommitRequested(
            self.shell.editor_title_text().to_owned(),
        ));
    }

    fn set_window_cursor(&self, cursor_icon: winit::window::CursorIcon) {
        if let Some(window) = self.editor_runtime.window() {
            window.set_cursor(cursor_icon);
        }
    }

    pub(crate) fn render(&mut self) -> Result<(), RenderError> {
        let _ = self.render_frame()?;
        self.update_focused_text_input_ime_cursor_area();
        Ok(())
    }

    fn update_focused_text_input_ime_cursor_area(&self) {
        let ime_rect = self.shell.focused_text_input_ime_cursor_rect().or_else(|| {
            (self.state.layout.focus_target == crate::FocusTarget::Editor).then(|| {
                self.editor_runtime.active_editor_ime_cursor_rect(self.shell_layout().editor_rect)
            })?
        });
        let Some(ime_rect) = ime_rect else {
            return;
        };
        let Some(window) = self.editor_runtime.window() else {
            return;
        };
        window.set_ime_cursor_area(
            PhysicalPosition::new(ime_rect.x as f64, (ime_rect.y + ime_rect.h) as f64),
            PhysicalSize::new(ime_rect.w.max(2.0) as f64, ime_rect.h as f64),
        );
    }

    fn update_editor_render_model(
        &self,
        model: &mut NotoraRenderModel,
        tab_id: appkit_core::workspace::types::TabId,
        layout: ShellLayout,
    ) {
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            model.editor_chrome = crate::editor_pane::EditorPaneInput::default();
            return;
        };
        model.editor_chrome.header.save_status_text = Self::editor_save_status(
            self.autosave.state(tab_id),
            summary.dirty,
            self.save_failure_messages.get(&tab_id).map(String::as_str),
        );
        model.editor_chrome.header.compact = layout.editor_header_rect.h / layout.dpi
            < crate::shell::layout::EDITOR_COMPACT_HEIGHT_THRESHOLD_LOGICAL;
        if let Some(plugin_name) =
            self.editor_runtime.tab_session(tab_id).map(|tab| tab.plugin_name())
        {
            model.editor_chrome.toolbar = crate::render::editor_toolbar_input_for_plugin(
                model.editor_chrome.mode,
                plugin_name,
            );
            if self.editor_runtime.toggle_target().is_some() {
                let showing_source = self.editor_runtime.active_is_toggled(plugin_name);
                crate::render::add_source_toggle_command(
                    &mut model.editor_chrome.toolbar,
                    showing_source,
                );
            }
            if model.editor_chrome.header.compact {
                crate::render::add_compact_editor_toolbar_commands(
                    &mut model.editor_chrome.toolbar,
                );
            }
        }
    }

    fn render_frame(
        &mut self,
    ) -> Result<appkit_shell::editor_runtime::EditorSurfacePaint, RenderError> {
        self.needs_redraw = false;
        let layout = self.shell_layout();
        let mut model =
            NotoraRenderModel::from_state_and_settings(&self.state, &self.product_settings);
        model.settings_overlay.persistence = self.settings_persistence.to_view();
        model.editor_pane = if self.editor_runtime.active_tab_id().is_some() {
            EditorPaneState::Active
        } else {
            EditorPaneState::Empty
        };
        if let Some(tab_id) = self.editor_runtime.active_tab_id() {
            self.update_editor_render_model(&mut model, tab_id, layout);
        } else {
            model.editor_chrome = crate::editor_pane::EditorPaneInput::default();
        }
        let mut render_resources = self.editor_runtime.take_render_resources();
        let mut frame = self.editor_runtime.begin_frame()?;
        self.shell.render(&mut frame, layout, &model)?;
        let editor_surface = self.editor_runtime.paint_active_editor(
            &mut frame,
            &mut render_resources,
            layout.editor_body_rect,
        )?;
        let canvas_scrollbars_input = (self.state.layout.overlay == crate::OverlayState::None)
            .then(|| self.editor_runtime.active_canvas_scrollbars_input())
            .flatten();
        frame.with_layout_context(|context| {
            self.shell.set_canvas_scrollbars_input(
                canvas_scrollbars_input,
                layout.editor_body_rect,
                context,
            );
        });
        frame.with_paint_context(|context| self.shell.paint_canvas_scrollbars(context));
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
        Ok(editor_surface)
    }

    fn editor_save_status(
        state: Option<AutoSaveState>,
        dirty: bool,
        failure_message: Option<&str>,
    ) -> String {
        match state {
            Some(AutoSaveState::Saving { .. }) => SAVE_STATUS_SAVING.to_owned(),
            Some(AutoSaveState::Failed { .. }) => failure_message
                .map(|message| format!("{SAVE_STATUS_FAILED}：{message}"))
                .unwrap_or_else(|| SAVE_STATUS_FAILED.to_owned()),
            Some(AutoSaveState::Scheduled { .. }) => SAVE_STATUS_PENDING.to_owned(),
            Some(AutoSaveState::Idle) | None if dirty => SAVE_STATUS_UNSAVED.to_owned(),
            Some(AutoSaveState::Idle) | None => SAVE_STATUS_SAVED.to_owned(),
        }
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
                "编辑器运行时未激活已安装的预览".to_owned(),
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
            self.save_failure_messages.remove(&candidate.tab_id);
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
                self.save_failure_messages.remove(tab_id);
                self.promote_preview_for_tab(*tab_id);
                if let Some(origin) = self.document_origin_for_tab(*tab_id) {
                    self.autosave.on_content_changed(&origin, *tab_id, *content_revision);
                }
            }
            EditorNotification::SaveCompleted { tab_id, content_revision } => {
                if let Some(identity) = self.document_registry.identity_for(*tab_id) {
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
        let Some(summary) = self.editor_runtime.document_summary(request.tab_id) else {
            self.autosave.cancel(request.tab_id);
            self.save_failure_messages.remove(&request.tab_id);
            self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
            self.cancel_pending_note_move(request, MOVE_SAVE_FAILURE_MESSAGE);
            return;
        };
        if !summary.dirty || summary.content_revision != request.content_revision {
            self.record_autosave_failure(request, "保存请求对应的内容版本已经过期".to_owned());
            self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
            self.cancel_pending_note_move(request, MOVE_SAVE_STALE_MESSAGE);
            return;
        }
        let prepared = match self.editor_runtime.prepare_save(request.tab_id) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.record_autosave_failure(request, error.to_string());
                self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
                self.cancel_pending_note_move(request, MOVE_SAVE_FAILURE_MESSAGE);
                return;
            }
        };
        if let Err(message) = self.submit_prepared_save(prepared) {
            self.record_autosave_failure(request, message);
            self.cancel_pending_trash_move(request, TRASH_SAVE_FAILURE_MESSAGE);
            self.cancel_pending_note_move(request, MOVE_SAVE_FAILURE_MESSAGE);
        }
    }

    fn record_autosave_failure(&mut self, request: AutoSaveRequest, message: String) {
        self.save_failure_messages.insert(request.tab_id, message);
        self.autosave.on_save_failed(request);
        self.needs_redraw = true;
        self.editor_runtime.request_redraw();
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

    fn pending_note_move_has_current_saved_document(
        &self,
        tab_id: appkit_core::workspace::types::TabId,
        pending_note_move: &PendingNoteMove,
    ) -> bool {
        self.editor_runtime.document_summary(tab_id).is_some_and(|summary| {
            !summary.dirty && summary.content_revision == pending_note_move.content_revision
        })
    }

    fn cancel_pending_note_move(&mut self, request: AutoSaveRequest, message: &str) {
        let is_matching_pending_move = self
            .pending_note_moves
            .get(&request.tab_id)
            .is_some_and(|pending| pending.content_revision == request.content_revision);
        if !is_matching_pending_move {
            return;
        }
        self.pending_note_moves.remove(&request.tab_id);
        self.dispatch_action(NotoraAction::NoteCommandFailed(message.to_owned()));
    }

    fn submit_prepared_save(
        &mut self,
        prepared: appkit_shell::editor_runtime::PreparedDocumentSave,
    ) -> Result<(), String> {
        let event_loop_proxy = self
            .event_loop_proxy
            .clone()
            .ok_or_else(|| "事件循环启动前保存线程不可用".to_owned())?;
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
        let Some(path) = rfd::FileDialog::new().add_filter("文本文档", &["txt", "md"]).save_file()
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
                "无法启动外部文件另存为路径处理线程".to_owned(),
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
            Some(SaveExternalFileAs::PathAlreadyOpen(_)) => self.dispatch_action(
                NotoraAction::NoteCommandFailed("另存为目标已在其他外部文件会话中打开".to_owned()),
            ),
            None => self.dispatch_action(NotoraAction::NoteCommandFailed(
                "另存为完成前外部文件会话已关闭".to_owned(),
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

    fn commit_active_note_title(&mut self, title: String) {
        let Some(tab_id) = self.editor_runtime.active_tab_id() else {
            self.dispatch_action(NotoraAction::NoteCommandFailed("当前没有活动笔记".to_owned()));
            return;
        };
        let Some(identity @ DocumentIdentity::Note(_)) =
            self.document_registry.identity_for(tab_id)
        else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "标题只能编辑工作区笔记".to_owned(),
            ));
            return;
        };
        if self.state.library.selected_card != Some(identity)
            || matches!(
                self.state.library.navigation_scope,
                notora_core::NavigationScope::Trash | notora_core::NavigationScope::ExternalFiles
            )
        {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "当前活动文档不是可编辑的工作区笔记".to_owned(),
            ));
            return;
        }
        let DocumentIdentity::Note(note_id) = identity else {
            return;
        };
        let normalized_title = normalize_notora_title(&title);
        let title_initialization = self
            .state
            .library
            .active_editor_metadata
            .as_ref()
            .filter(|metadata| metadata.identity == identity)
            .map(|metadata| metadata.metadata.title_initialization)
            .unwrap_or(notora_core::TitleInitialization::Independent);
        let mutation = match title_initialization {
            notora_core::TitleInitialization::AwaitingFirstCommit => {
                crate::action::MetadataMutation::CompleteTitleInitializationFromHeader {
                    note_id,
                    title: normalized_title,
                }
            }
            notora_core::TitleInitialization::Independent => {
                crate::action::MetadataMutation::SetTitle { note_id, title: normalized_title }
            }
        };
        self.execute_metadata_mutation(mutation);
    }

    fn submit_document_title_initialization(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        saved_content_revision: u64,
    ) {
        let Some(DocumentIdentity::Note(note_id)) = self.document_registry.identity_for(tab_id)
        else {
            return;
        };
        let initialization = self
            .state
            .library
            .active_editor_metadata
            .as_ref()
            .filter(|metadata| metadata.identity == DocumentIdentity::Note(note_id))
            .map(|metadata| metadata.metadata.title_initialization);
        if initialization != Some(notora_core::TitleInitialization::AwaitingFirstCommit) {
            return;
        }
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return;
        };
        let Some(path) = summary.path.as_deref() else {
            return;
        };
        let Some(kind @ (DocumentKind::Markdown | DocumentKind::Mindmap)) =
            DocumentKind::from_path(path)
        else {
            return;
        };
        let Some(snapshot) = self.editor_runtime.document_text_snapshot(tab_id) else {
            return;
        };
        if snapshot.content_revision != saved_content_revision {
            return;
        }
        let title = initial_title_from_document(kind, &snapshot.text);
        self.execute_metadata_mutation(
            crate::action::MetadataMutation::CompleteTitleInitializationFromDocument {
                note_id,
                title,
            },
        );
    }

    fn apply_title_initialization_outcome(
        &mut self,
        mutation: &crate::action::MetadataMutation,
        outcome: crate::action::MetadataMutationOutcome,
        note_id: notora_core::NoteId,
    ) {
        match (mutation, outcome) {
            (
                crate::action::MetadataMutation::CompleteTitleInitializationFromHeader {
                    title,
                    ..
                },
                crate::action::MetadataMutationOutcome::TitleInitializationWon,
            ) => self.seed_document_title(note_id, title),
            (
                crate::action::MetadataMutation::CompleteTitleInitializationFromHeader {
                    title,
                    ..
                },
                crate::action::MetadataMutationOutcome::TitleInitializationLost,
            ) => self.execute_metadata_mutation(crate::action::MetadataMutation::SetTitle {
                note_id,
                title: title.clone(),
            }),
            _ => {}
        }
    }

    fn seed_document_title(&mut self, note_id: notora_core::NoteId, title: &str) {
        let identity = DocumentIdentity::Note(note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return;
        };
        let Some(snapshot) = self.editor_runtime.document_text_snapshot(tab_id) else {
            return;
        };
        let Some(path) =
            self.editor_runtime.document_summary(tab_id).and_then(|summary| summary.path)
        else {
            return;
        };
        let Some(kind @ (DocumentKind::Markdown | DocumentKind::Mindmap)) =
            DocumentKind::from_path(&path)
        else {
            return;
        };
        let projected_source = replace_document_title(kind, &snapshot.text, title);
        let Some((range, replacement)) =
            single_range_replacement(&snapshot.text, &projected_source)
        else {
            return;
        };
        let request = DocumentTextReplacement {
            tab_id,
            content_revision: snapshot.content_revision,
            range,
            replacement,
        };
        match self.editor_runtime.replace_document_text(request) {
            Ok(editor_outcome) => {
                self.apply_editor_outcome(editor_outcome);
                if kind == DocumentKind::Mindmap {
                    self.move_mindmap_cursor_to_root_end(tab_id);
                }
            }
            Err(error) => self
                .dispatch_action(NotoraAction::NoteCommandFailed(title_edit_error_message(error))),
        }
    }

    fn move_mindmap_cursor_to_root_end(&mut self, tab_id: appkit_core::workspace::types::TabId) {
        let Some(snapshot) = self.editor_runtime.document_text_snapshot(tab_id) else {
            return;
        };
        let Ok(tree) = textora_markdown::mmf::parser::parse(&snapshot.text) else {
            return;
        };
        let Some(tab) = self.editor_runtime.tab_session_mut(tab_id) else {
            return;
        };
        tab.document.cursor_mut().selection_anchor = None;
        tab.document.cursor_move_to_offset(tree.root.title_byte_range.end);
    }
}

fn single_range_replacement(
    original: &str,
    projected: &str,
) -> Option<(std::ops::Range<usize>, String)> {
    if original == projected {
        return None;
    }
    let original_bytes = original.as_bytes();
    let projected_bytes = projected.as_bytes();
    let mut prefix = 0;
    while prefix < original_bytes.len()
        && prefix < projected_bytes.len()
        && original_bytes[prefix] == projected_bytes[prefix]
    {
        prefix += 1;
    }
    while prefix > 0 && (!original.is_char_boundary(prefix) || !projected.is_char_boundary(prefix))
    {
        prefix -= 1;
    }

    let mut suffix = 0;
    while suffix < original_bytes.len().saturating_sub(prefix)
        && suffix < projected_bytes.len().saturating_sub(prefix)
        && original_bytes[original_bytes.len() - suffix - 1]
            == projected_bytes[projected_bytes.len() - suffix - 1]
    {
        suffix += 1;
    }
    while suffix > 0
        && (!original.is_char_boundary(original.len() - suffix)
            || !projected.is_char_boundary(projected.len() - suffix))
    {
        suffix -= 1;
    }

    let original_end = original.len() - suffix;
    let projected_end = projected.len() - suffix;
    Some((prefix..original_end, projected[prefix..projected_end].to_owned()))
}

fn title_edit_error_message(error: DocumentTextEditError) -> String {
    match error {
        DocumentTextEditError::UnknownTab { .. } => "当前笔记已关闭，请重新选择".to_owned(),
        DocumentTextEditError::StaleRevision { .. } => "笔记已发生变化，请重新提交标题".to_owned(),
        DocumentTextEditError::InvalidByteRange { .. } => "标题范围无效，请重新提交".to_owned(),
        DocumentTextEditError::ReadOnly { .. } => "当前笔记不可编辑".to_owned(),
    }
}

fn initial_title_from_document(kind: DocumentKind, source: &str) -> Option<String> {
    let candidate = match kind {
        DocumentKind::Markdown => document_title_projection(kind, source).title,
        DocumentKind::Mindmap => textora_markdown::mmf::parser::parse(source).ok()?.root.title,
        DocumentKind::Text => return None,
    };
    let trimmed_candidate = candidate.trim();
    (!trimmed_candidate.is_empty()).then(|| trimmed_candidate.to_owned())
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
            | NotoraAction::NoteCommandCompleted(_)
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
    use std::time::Instant;

    use super::{NotoraApp, action_requires_session_persistence};
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
            NotoraApp::editor_save_status(
                Some(AutoSaveState::Saving { content_revision: 3 }),
                true,
                None,
            ),
            "保存中"
        );
        assert_eq!(
            NotoraApp::editor_save_status(
                Some(AutoSaveState::Failed { content_revision: 3 }),
                true,
                None,
            ),
            "保存失败"
        );
        assert_eq!(
            NotoraApp::editor_save_status(
                Some(AutoSaveState::Scheduled { deadline, content_revision: 3 }),
                true,
                None,
            ),
            "待保存"
        );
        assert_eq!(NotoraApp::editor_save_status(None, true, None), "未保存");
        assert_eq!(NotoraApp::editor_save_status(None, false, None), "已保存");
    }

    #[test]
    fn editor_save_status_includes_the_failure_reason() {
        assert_eq!(
            NotoraApp::editor_save_status(
                Some(AutoSaveState::Failed { content_revision: 3 }),
                true,
                Some("file is read-only"),
            ),
            "保存失败：file is read-only"
        );
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

    fn request_note_creation(&mut self, kind: DocumentKind, target: NoteCreationTarget) {
        if self.workspace_controller.active_workspace().is_none() {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "请先设置工作区根目录".to_owned(),
            ));
            return;
        }
        self.submit_note_command(NoteCommand::CreateConfigured(ConfiguredCreateNoteRequest {
            kind,
            target_directory: target.directory,
            encryption: NoteEncryption::Unencrypted,
        }));
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
        self.editor_runtime.switch_active_plugin();
    }

    fn execute_semantic_edit(&mut self, command: ui::plugin::SemanticEditCommand) {
        let (_result, outcome) = self.editor_runtime.execute_semantic_edit(command);
        self.apply_editor_outcome(outcome);
    }

    fn execute_metadata_mutation(&mut self, mutation: crate::action::MetadataMutation) {
        if !register_pending_metadata_mutation(
            &mut self.pending_metadata_mutations,
            mutation.clone(),
        ) {
            return;
        }
        let note_id = metadata_mutation_note_id(&mutation);
        let selection_generation = self.state.library.selected_document_generation;
        self.pending_metadata_generations
            .entry(note_id)
            .or_default()
            .push_back(selection_generation);
        if let Err(error) = self.workspace_controller.execute_metadata_mutation(mutation.clone()) {
            self.take_pending_metadata_generation(note_id);
            remove_pending_metadata_mutation(&mut self.pending_metadata_mutations, &mutation);
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
                            "已打开的笔记不再可用".to_owned(),
                        ),
                    ));
                    return;
                };
                if summary.dirty {
                    let Some(origin) = self.document_origin_for_tab(tab_id) else {
                        self.dispatch_action(NotoraAction::TrashOperationFailed(
                            crate::action::TrashOperationFailure::Message(
                                "只有工作区笔记可以移入回收站".to_owned(),
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
                "请先打开笔记再重命名".to_owned(),
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
                .add_filter("文本文档", &["txt", "md"])
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

fn register_pending_metadata_mutation(
    pending_mutations: &mut Vec<crate::action::MetadataMutation>,
    mutation: crate::action::MetadataMutation,
) -> bool {
    if pending_mutations.contains(&mutation) {
        return false;
    }
    pending_mutations.push(mutation);
    true
}

fn remove_pending_metadata_mutation(
    pending_mutations: &mut Vec<crate::action::MetadataMutation>,
    completed_mutation: &crate::action::MetadataMutation,
) {
    if let Some(index) =
        pending_mutations.iter().position(|mutation| mutation == completed_mutation)
    {
        pending_mutations.remove(index);
    }
}

impl NotoraApp {
    fn take_pending_metadata_generation(&mut self, note_id: notora_core::NoteId) -> Option<u64> {
        let queue = self.pending_metadata_generations.get_mut(&note_id)?;
        let generation = queue.pop_front();
        if queue.is_empty() {
            self.pending_metadata_generations.remove(&note_id);
        }
        generation
    }

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
        let identity = DocumentIdentity::Note(request.note_id);
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            self.submit_note_command(NoteCommand::Move(request));
            return;
        };
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "已打开的笔记不再可用，因此未移动".to_owned(),
            ));
            return;
        };
        if !summary.dirty {
            self.submit_note_command(NoteCommand::Move(request));
            return;
        }
        let Some(origin) = self.document_origin_for_tab(tab_id) else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "只有工作区笔记可以移动".to_owned(),
            ));
            return;
        };
        self.pending_note_moves.insert(
            tab_id,
            PendingNoteMove { request, content_revision: summary.content_revision },
        );
        self.autosave.request_immediate_save(&origin, tab_id, summary.content_revision);
        self.process_due_autosaves();
    }

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
            self.state.library.last_command_error =
                Some("上次使用的工作区不可用，或与保存的标识不再匹配".to_owned());
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
                let identity = DocumentIdentity::Note(note_id);
                self.dispatch_action(NotoraAction::CardSelected(identity));
                self.dispatch_action(NotoraAction::CardActivated(identity));
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
        self.save_failure_messages.remove(&tab_id);
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
                "元数据已保存，但无法启动目录索引备份：{error}"
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
                    Err(error) => crate::product::NotoraProductEvent::ConflictRetryRevisionFailed {
                        identity,
                        message: format!("重试保存前无法读取当前磁盘版本：{error}"),
                    },
                };
                let _ = sender.send(event);
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
                    "未命名文档没有可重试的磁盘冲突".to_owned(),
                ));
            }
        }
    }

    fn save_conflicted_note_copy(&mut self, identity: DocumentIdentity) {
        let Some(tab_id) = self.document_registry.tab_for(identity) else {
            return;
        };
        let Some(path) = rfd::FileDialog::new().add_filter("文本文档", &["txt", "md"]).save_file()
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
                "工作区关闭后无法保存冲突副本".to_owned(),
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
                "无法启动冲突副本保存线程".to_owned(),
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
                "无法启动冲突重新加载线程".to_owned(),
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
                "无法启动外部文件读取线程".to_owned(),
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
                "外部文档不可用；请重新定位或移除对应会话".to_owned(),
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
                    "外部文档不可用；请重新定位或移除对应会话".to_owned(),
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
        if self.document_registry.identity_for(tab_id) != Some(identity) {
            return;
        }
        let Some(summary) = self.editor_runtime.document_summary(tab_id) else {
            return;
        };
        if summary.content_revision != content_revision {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "加载磁盘版本时文档已发生变化".to_owned(),
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
            self.save_failure_messages.remove(&tab_id);
            self.dispatch_action(NotoraAction::SaveConflictResolved { identity });
        }
    }
}

fn rename_file_name_for_destination(
    current_path: &std::path::Path,
    destination: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let current_parent = current_path.parent().ok_or_else(|| "当前笔记没有父目录".to_owned())?;
    let destination_parent =
        destination.parent().ok_or_else(|| "重命名目标没有父目录".to_owned())?;
    let current_parent = std::fs::canonicalize(current_parent)
        .map_err(|error| format!("无法解析当前笔记目录：{error}"))?;
    let destination_parent = std::fs::canonicalize(destination_parent)
        .map_err(|error| format!("无法解析重命名目标：{error}"))?;
    if current_parent != destination_parent {
        return Err("重命名只能保留在当前文件夹中；如需更换文件夹，请使用移动".to_owned());
    }
    destination
        .file_name()
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "重命名目标没有文件名".to_owned())
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

    use super::{
        FontSystemPreparation, NotoraApp, PendingNoteMove, PendingTrashMove,
        SettingsPersistenceState, StartupTrace, initial_title_from_document,
        normalize_notora_title, register_pending_metadata_mutation,
        remove_pending_metadata_mutation, rename_file_name_for_destination, resolve_pointer_cursor,
        workspace_relative_directory,
    };
    use crate::action::{MetadataMutation, NotoraAction};
    use crate::autosave::{AutoSaveRequest, AutoSaveState};
    use crate::editor_adapter::LoadedDocument;
    use crate::state::CardPageState;
    use crate::{
        CompactContent, ExternalFileSession, FocusTarget, NotoraPaths, OverlayState,
        WorkspaceCommand, WorkspaceRootState,
    };
    use appkit_shell::editor_runtime::{DocumentTextReplacement, EditorNotification};
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

    fn app() -> NotoraApp {
        let directory = tempfile::tempdir().expect("test should create a temporary directory");
        let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
            .expect("test should create isolated product paths");
        NotoraApp::with_paths(paths).expect("notora app should construct without a window")
    }

    fn drain_until_document_text(
        app: &mut NotoraApp,
        tab_id: appkit_core::workspace::types::TabId,
        expected_text: &str,
        deadline: Instant,
    ) {
        loop {
            app.drain_product_events();
            let text = app
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
        app.autosave.request_immediate_save(&origin, tab_id, request.content_revision);
        assert_eq!(app.autosave.take_due_saves(), vec![request]);

        app.record_autosave_failure(request, "file is read-only".to_owned());

        assert_eq!(
            app.autosave.state(tab_id),
            Some(AutoSaveState::Failed { content_revision: request.content_revision })
        );
        assert_eq!(
            app.save_failure_messages.get(&tab_id).map(String::as_str),
            Some("file is read-only")
        );
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
        let mut pending_mutations = Vec::new();

        assert!(register_pending_metadata_mutation(&mut pending_mutations, mutation.clone()));
        assert!(!register_pending_metadata_mutation(&mut pending_mutations, mutation.clone()));
        assert_eq!(pending_mutations, vec![mutation.clone()]);

        remove_pending_metadata_mutation(&mut pending_mutations, &mutation);
        assert!(pending_mutations.is_empty());
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
            NotoraApp::with_paths(paths).expect("notora app should construct without a window");

        app.start_font_system_preparation();
        assert!(matches!(&app.font_system_preparation, FontSystemPreparation::InProgress(_)));

        let font_system = app.take_prepared_font_system();

        std::hint::black_box(font_system);
        assert!(matches!(app.font_system_preparation, FontSystemPreparation::Deferred));
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
        assert_eq!(app.state().layout.focus_target, FocusTarget::Editor);
    }

    #[test]
    fn focused_markdown_document_schedules_cursor_blink() {
        let mut app = app();
        let prepared = crate::editor_adapter::prepare_loaded_document(
            &app.editor_runtime,
            LoadedDocument {
                path: std::path::PathBuf::from("caret.md"),
                contents: "正文内容".to_owned(),
                disk_revision: None,
            },
        )
        .expect("Markdown fixture should prepare");
        app.editor_runtime.install_prepared_tab(
            prepared,
            None,
            appkit_shell::editor_runtime::OpenDisposition::Persistent,
        );
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));

        assert!(app.next_deadline().is_some());
        assert!(app.editor_runtime.active_cursor_paint_enabled());

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::EditorTitle));
        assert!(!app.editor_runtime.active_cursor_paint_enabled());

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::EditorTag));
        assert!(!app.editor_runtime.active_cursor_paint_enabled());

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::CardList));
        assert_eq!(app.next_deadline(), None);
    }

    #[test]
    fn product_text_focus_schedules_its_blink_before_another_input_event() {
        let mut app = app();
        assert_eq!(app.shell.next_text_cursor_blink_at(), None);

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::NavigationSearch));

        assert!(app.shell.next_text_cursor_blink_at().is_some());
        assert_eq!(app.next_deadline(), app.shell.next_text_cursor_blink_at());
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

        assert_eq!(app.state().layout.focus_target, FocusTarget::NavigationSearch);
        assert!(app.shell.search_box_is_focused());
    }

    #[test]
    fn search_text_drag_requests_redraw_in_the_same_pointer_event_cycle() {
        let mut app = app();
        app.dispatch_action(NotoraAction::SearchTextChanged("路线图".to_owned()));
        app.render().expect("headless search box should render");
        let search_rect = app.shell.search_box_rect();

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
        assert!(app.needs_redraw, "drag selection should schedule a redraw");
        assert!(
            app.editor_runtime.take_redraw_request(),
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
        let navigation_width_before_modal = app.state().layout.navigation_width_logical;

        app.dispatch_action(NotoraAction::OpenSettings);

        assert_eq!(app.state().layout.overlay, OverlayState::Settings);
        assert!(app.route_product_event(&ui::Event::KeyDown(
            ui::KeyCode::Left,
            ui::core::Modifiers::NONE,
        )));
        assert_eq!(
            app.state().layout.navigation_width_logical,
            navigation_width_before_modal,
            "modal keyboard input must not resize an underlying splitter"
        );
    }

    #[test]
    fn modal_state_blocks_workspace_shortcuts_and_escape_closes_the_modal() {
        let mut app = app();
        app.dispatch_action(NotoraAction::OpenSettings);
        let modal_focus = app.state().layout.focus_target;

        let command_modifiers = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };
        for key_code in [
            ui::KeyCode::Char('n'),
            ui::KeyCode::Char('o'),
            ui::KeyCode::Char('f'),
            ui::KeyCode::Char('s'),
        ] {
            app.handle_key_input(key_code, command_modifiers);
        }

        assert_eq!(app.state().layout.overlay, OverlayState::Settings);
        assert_eq!(app.state().layout.focus_target, modal_focus);

        app.handle_key_input(ui::KeyCode::Escape, ui::core::Modifiers::NONE);
        assert_eq!(app.state().layout.overlay, OverlayState::None);
        assert_eq!(app.state().layout.focus_target, FocusTarget::NavigationTree);
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
                message: "工作区文件监视器已断开，自动同步已停止".to_owned(),
            })
            .expect("product receiver should stay available");

        app.drain_product_events();

        assert!(app.catalog_reconciliation_pending);
        assert!(
            app.state()
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
            if matches!(app.state().library.selected_card, Some(DocumentIdentity::Note(_))) {
                break;
            }
            assert!(Instant::now() < deadline, "configured note should be created");
            thread::sleep(Duration::from_millis(10));
        }

        assert!(notes_directory.join("未命名 1.txt").is_file());
        assert_eq!(app.state().layout.overlay, OverlayState::None);
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
        while app.state().library.last_command_error.is_none() {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "creation failure should return to the app");
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(app.state().layout.overlay, OverlayState::None);
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
            if let Some(identity) = app.state().library.selected_card {
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
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created note text should remain available")
            .text
            .is_empty()
        {
            app.drain_product_events();
            assert!(Instant::now() < deadline, "title initialization should complete");
            thread::sleep(Duration::from_millis(10));
        }

        let snapshot = app
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created note text should remain available");
        assert_eq!(snapshot.text, "# 项目路线图\n\n");
        assert!(snapshot.content_revision > 0);

        app.dispatch_action(NotoraAction::TitleCommitRequested("独立的 Notora 标题".to_owned()));
        let second_deadline = Instant::now() + Duration::from_secs(2);
        while app
            .pending_metadata_mutations
            .iter()
            .any(|mutation| matches!(mutation, MetadataMutation::SetTitle { .. }))
        {
            app.drain_product_events();
            assert!(Instant::now() < second_deadline, "independent title should persist");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.editor_runtime
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
            if let Some(identity) = app.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created note should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created note should render its title editor");

        assert!(app.route_product_event(&ui::Event::ImeCommit("项目路线图".to_owned())));
        assert_eq!(app.shell.editor_title_text(), "项目路线图");

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));

        drain_until_document_text(&mut app, tab_id, "# 项目路线图\n\n", deadline);
        app.render().expect("committed title should render");
        assert_eq!(app.shell.editor_title_text(), "项目路线图");
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
            if let Some(DocumentIdentity::Note(note_id)) = app.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(DocumentIdentity::Note(note_id))
                && app.state().library.active_editor_metadata.is_some()
            {
                break (note_id, tab_id);
            }
            assert!(Instant::now() < deadline, "created note should finish loading");
            thread::sleep(Duration::from_millis(10));
        };
        let snapshot =
            app.editor_runtime.document_text_snapshot(tab_id).expect("created source should exist");
        let outcome = app
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
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("edited source should exist")
            .content_revision;
        app.handle_editor_notification(&EditorNotification::SaveCompleted {
            tab_id,
            content_revision,
        });

        while app.pending_metadata_mutations.iter().any(|mutation| {
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
            .pending_metadata_mutations
            .iter()
            .any(|mutation| matches!(mutation, MetadataMutation::SetTitle { .. }))
        {
            app.drain_product_events();
            assert!(Instant::now() < second_deadline, "independent title should persist");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            app.editor_runtime
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
            if let Some(identity) = app.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            app.editor_runtime
                .document_text_snapshot(tab_id)
                .expect("created mmap source should exist")
                .text,
            "#"
        );

        app.dispatch_action(NotoraAction::TitleCommitRequested("项目路线图".to_owned()));

        while app
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
            if let Some(identity) = app.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created mmap should render its empty root");
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        let Some(tab) = app.editor_runtime.tab_session_mut(tab_id) else {
            panic!("created mmap should keep its runtime session");
        };
        tab.document.cursor_mut().selection_anchor = Some(0);
        tab.document.cursor_move_to_offset(1);

        app.commit_editor_text("kk".to_owned());
        assert_eq!(
            app.editor_runtime
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
            app.editor_runtime
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
            if let Some(identity) = app.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created mmap should render its empty root editor");
        assert_eq!(
            app.editor_runtime
                .tab_session(tab_id)
                .expect("created mmap should have a runtime session")
                .plugin_name(),
            ui::plugin::PLUGIN_MINDMAP
        );
        assert!(app.shell.editor_title_text().is_empty());

        let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        assert!(app.route_product_event(&tab_event));
        drain_until_document_text(&mut app, tab_id, "# 无标题", deadline);
        assert_eq!(
            app.editor_runtime
                .tab_session(tab_id)
                .expect("created mmap should retain its runtime session")
                .document
                .cursor_offset()
                .to_usize(),
            "# 无标题".len()
        );
        app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);

        let snapshot = app
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap source should remain available");
        assert_eq!(snapshot.text, "# 无标题\n##\n");
        assert_eq!(app.state().layout.focus_target, FocusTarget::Editor);
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
            if let Some(identity) = app.state().library.selected_card
                && let Some(tab_id) = app.document_tab_for(identity)
            {
                break tab_id;
            }
            assert!(Instant::now() < deadline, "created mmap should have an editor tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.render().expect("created mmap should render its title editor");
        assert!(app.route_product_event(&ui::Event::ImeCommit("项目路线图".to_owned())));
        assert_eq!(app.state().layout.focus_target, FocusTarget::EditorTitle);
        assert_eq!(app.shell.editor_title_text(), "项目路线图");

        let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        assert!(app.route_product_event(&tab_event));
        drain_until_document_text(&mut app, tab_id, "# 项目路线图", deadline);
        app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);

        let snapshot = app
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap source should remain available");
        assert_eq!(snapshot.text, "# 项目路线图\n##\n");
        assert_eq!(app.state().layout.focus_target, FocusTarget::Editor);

        app.commit_editor_text("子节点".to_owned());
        let snapshot = app
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created mmap child should remain available");
        assert_eq!(snapshot.text, "# 项目路线图\n##子节点\n");

        let tab_event = ui::Event::KeyDown(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        if !app.route_product_event(&tab_event) {
            app.handle_editor_key_input(ui::KeyCode::Tab, ui::core::Modifiers::NONE);
        }
        let snapshot = app
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
            if let Some(identity) = app.state().library.selected_card
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
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created markdown source should remain available");
        assert_eq!(snapshot.text, "# 项目记录\n\n");
        assert_eq!(app.state().layout.focus_target, FocusTarget::Editor);
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
            if let Some(identity) = app.state().library.selected_card
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
        assert_eq!(app.state().layout.focus_target, FocusTarget::Editor);
        assert_eq!(
            app.editor_runtime
                .tab_session(tab_id)
                .expect("source view should keep the runtime session")
                .plugin_name(),
            ui::plugin::PLUGIN_EDITOR
        );

        let backspace_event = ui::Event::KeyDown(ui::KeyCode::Backspace, ui::core::Modifiers::NONE);
        if !app.route_product_event(&backspace_event) {
            app.handle_editor_key_input(ui::KeyCode::Backspace, ui::core::Modifiers::NONE);
        }
        app.editor_runtime
            .tab_session_mut(tab_id)
            .expect("source view should keep the mutable runtime session")
            .document
            .cursor_move_to_offset("# ".len());
        let delete_event = ui::Event::KeyDown(ui::KeyCode::Delete, ui::core::Modifiers::NONE);
        if !app.route_product_event(&delete_event) {
            app.handle_editor_key_input(ui::KeyCode::Delete, ui::core::Modifiers::NONE);
        }

        let snapshot = app
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
                if let Some(identity) = app.state().library.selected_card
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
                app.editor_runtime
                    .tab_session(tab_id)
                    .expect("created note should have a runtime session")
                    .plugin_name(),
                visual_plugin
            );

            app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
            assert_eq!(
                app.editor_runtime
                    .tab_session(tab_id)
                    .expect("source view should keep the runtime session")
                    .plugin_name(),
                ui::plugin::PLUGIN_EDITOR
            );

            app.dispatch_action(NotoraAction::ToggleSourceViewRequested);
            assert_eq!(
                app.editor_runtime
                    .tab_session(tab_id)
                    .expect("visual view should be restorable")
                    .plugin_name(),
                visual_plugin
            );
        }
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
            if let Some(identity) = app.state().library.selected_card {
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
            .editor_runtime
            .document_text_snapshot(tab_id)
            .expect("created note text should remain available");
        assert_eq!(snapshot.text, "");
        assert_eq!(app.state().layout.focus_target, FocusTarget::EditorTitle);
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
            if let Some(DocumentIdentity::Note(note_id)) = app.state().library.selected_card
                && app.state().library.active_editor_metadata.is_some()
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

        assert_eq!(app.state().layout.overlay, OverlayState::None);
        assert_eq!(app.state().layout.focus_target, FocusTarget::NavigationTree);
    }

    #[test]
    fn workspace_root_selection_is_completed_before_note_creation_becomes_available() {
        let workspace_directory =
            tempfile::tempdir().expect("workspace test directory should be created");
        let selected_workspace_root = workspace_directory.path().to_path_buf();
        let mut app = app();
        app.workspace_directory_chooser = Box::new(move || Some(selected_workspace_root.clone()));

        app.dispatch_action(NotoraAction::WorkspaceRootSelectionRequested);

        assert_eq!(app.state().workspace_root, WorkspaceRootState::Active);
        assert_eq!(app.state().layout.overlay, OverlayState::None);

        app.dispatch_action(NotoraAction::OpenNewDocumentMenu);
        assert_eq!(app.state().layout.overlay, OverlayState::NewDocumentMenu);
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
            NotoraApp::with_paths(paths.clone()).expect("first app should construct");
        let selected_workspace_root = workspace_directory.path().to_path_buf();
        first_app.workspace_directory_chooser =
            Box::new(move || Some(selected_workspace_root.clone()));

        assert!(first_app.select_workspace_root());
        first_app.pending_session_persist_at = Some(Instant::now());
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
            if let Some(DocumentIdentity::Note(note_id)) = first_app.state().library.selected_card
                && first_app.document_tab_for(DocumentIdentity::Note(note_id)).is_some()
            {
                break note_id;
            }
            assert!(Instant::now() < creation_deadline, "created note should load promptly");
            thread::sleep(Duration::from_millis(10));
        };

        assert!(
            first_app.pending_session_persist_at.is_some(),
            "note completion should schedule persistence for the new last document"
        );
        assert_eq!(
            first_app.capture_product_session().last_document,
            Some(crate::session::SavedDocument::Note { note_id })
        );
        first_app.pending_session_persist_at = Some(Instant::now());
        first_app.process_due_session_persistence();
        first_app.persistence_worker.shutdown();
        first_app.drain_product_events();
        assert_eq!(first_app.state().library.last_command_error, None);
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
            NotoraApp::with_paths(paths).expect("restarted app should construct");
        restarted_app.restore_pending_session();
        let restore_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            restarted_app.drain_product_events();
            let identity = DocumentIdentity::Note(note_id);
            if card_page_contains_note(&restarted_app.state().library.card_page, note_id)
                && restarted_app.document_tab_for(identity).is_some()
            {
                break;
            }
            assert!(
                Instant::now() < restore_deadline,
                "restarted app should restore the new note in the workspace"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(restarted_app.state().layout.focus_target, FocusTarget::Editor);
        assert_eq!(restarted_app.state().layout.compact_content, CompactContent::Editor);
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
                app.state().library.selected_card
                && let Some(tab_id) =
                    app.document_tab_for(notora_core::DocumentIdentity::Note(note_id))
                && let Some(path) =
                    app.editor_runtime.document_summary(tab_id).and_then(|summary| summary.path)
            {
                break (note_id, tab_id, path);
            }
            assert!(Instant::now() < deadline, "created note should install a preview tab");
            thread::sleep(Duration::from_millis(10));
        };
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        app.commit_editor_text("unsaved edit".to_owned());
        assert!(
            app.editor_runtime
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
            app.state()
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
