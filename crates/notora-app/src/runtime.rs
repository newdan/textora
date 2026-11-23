//! Notora 产品与编辑器运行时；平台应用只通过 `NotoraApp` 组合根访问。

#[cfg(test)]
#[path = "runtime/keyboard_shortcut_tests.rs"]
mod keyboard_shortcut_tests;

#[cfg(test)]
#[path = "runtime/document_search_tests.rs"]
mod document_search_tests;

#[path = "runtime/document_search_runtime.rs"]
mod document_search_runtime;

#[path = "runtime/action_runtime.rs"]
mod action_runtime;
#[path = "app/deadline_coordinator.rs"]
mod deadline_coordinator;
#[path = "runtime/document_commands.rs"]
mod document_commands;
#[path = "runtime/document_completions.rs"]
mod document_completions;
#[path = "runtime/document_runtime.rs"]
mod document_runtime;
#[path = "runtime/frame_runtime.rs"]
mod frame_runtime;
#[path = "runtime/persistence_runtime.rs"]
mod persistence_runtime;
#[path = "runtime/product_effects.rs"]
mod product_effects;
#[path = "app/product_event_coordinator.rs"]
mod product_event_coordinator;
#[path = "runtime/session_restore_runtime.rs"]
mod session_restore_runtime;
#[path = "runtime/window_runtime.rs"]
mod window_runtime;
#[path = "runtime/workspace_completions.rs"]
mod workspace_completions;
#[path = "runtime/workspace_transition_runtime.rs"]
mod workspace_transition_runtime;

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
    SaveConflictRequest, WorkspaceTransitionRequest,
};
use crate::autosave::{AutoSaveRequest, AutoSaveScheduler};
use crate::dirty_snapshot::{collect_dirty_snapshots, write_dirty_snapshot};
use crate::editor_adapter::{LoadedDocument, build_editor_plugins, load_document};
use crate::effect_executor::{EffectExecutor, ExternalOpenRequest, ManualSaveRequest};
use crate::events;
use crate::external_files::{CanonicalExternalPath, ExternalFileSession, read_external_text_file};
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
use notora_core::note_command::{
    ConfiguredCreateNoteRequest, CreateNoteStorage, MoveNoteRequest, NoteCommand,
};
use notora_core::{DocumentIdentity, DocumentKind};

use self::action_runtime::ActionRuntime;
use self::deadline_coordinator::{DeadlineCoordinator, DeadlineSnapshot};
use self::document_runtime::{
    DocumentOutcome, DocumentRuntime, DocumentSelection, TitleCommitContext,
};
#[cfg(test)]
use self::document_runtime::{PendingNoteMove, PendingTitleUpdate, PendingTrashMove};
use self::frame_runtime::{FrameInput, FrameRuntime, StartupTrace};
#[cfg(test)]
use self::persistence_runtime::SettingsPersistenceState;
use self::persistence_runtime::{PersistenceCompletionOutcome, PersistenceRuntime};
use self::product_event_coordinator::ProductEventCoordinator;
use self::session_restore_runtime::{
    SessionRestore, SessionRestoreFocusPolicy, SessionRestoreRuntime,
};
use self::window_runtime::WindowRuntime;
use self::workspace_transition_runtime::WorkspaceTransitionRuntime;

const PRODUCT_WINDOW_TITLE: &str = "notora";
const SHUTDOWN_SAVE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_SAVE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_RUNTIME_TAB_LIMIT: usize = 12;
#[cfg(target_os = "windows")]
const WINDOW_ICON_SIZE: u32 = 32;
#[cfg(target_os = "windows")]
const TASKBAR_ICON_SIZE: u32 = 256;

#[cfg(target_os = "windows")]
fn windows_window_icons() -> (winit::window::Icon, winit::window::Icon) {
    use image::imageops::FilterType;

    let artwork = image::load_from_memory_with_format(
        include_bytes!("../../../assets/NotoraAppIcon.ico"),
        image::ImageFormat::Ico,
    )
    .expect("the embedded notora app icon must be a valid ICO image");
    let icon_at_size = |size| {
        let pixels = artwork.resize_exact(size, size, FilterType::Lanczos3).into_rgba8();
        winit::window::Icon::from_rgba(pixels.into_raw(), size, size)
            .expect("the decoded notora app icon must have valid RGBA dimensions")
    };

    (icon_at_size(WINDOW_ICON_SIZE), icon_at_size(TASKBAR_ICON_SIZE))
}

fn notora_window_attributes() -> WindowAttributes {
    let attributes = WindowAttributes::default()
        .with_title(PRODUCT_WINDOW_TITLE)
        .with_decorations(!cfg!(target_os = "windows"))
        .with_min_inner_size(LogicalSize::new(
            crate::shell::layout::MINIMUM_WINDOW_WIDTH_LOGICAL,
            crate::shell::layout::MINIMUM_WINDOW_HEIGHT_LOGICAL,
        ));
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
        let (window_icon, taskbar_icon) = windows_window_icons();
        // 外窗由 DirectComposition 透明合成；精确圆角由浮点 clip 控制。
        attributes
            .with_window_icon(Some(window_icon))
            .with_taskbar_icon(Some(taskbar_icon))
            .with_no_redirection_bitmap(true)
            .with_undecorated_shadow(false)
            .with_corner_preference(CornerPreference::DoNotRound)
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attributes
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
            .with_fullsize_content_view(true)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    attributes
}

#[cfg(test)]
#[test]
fn window_attributes_preserve_product_title_and_platform_decorations() {
    let attributes = notora_window_attributes();
    assert_eq!(attributes.title, "notora");
    assert_eq!(attributes.decorations, !cfg!(target_os = "windows"));
    #[cfg(target_os = "windows")]
    assert!(attributes.window_icon.is_some(), "custom window chrome must retain the app icon");
}

type WorkspaceDirectoryChooser = Box<dyn Fn() -> Option<std::path::PathBuf>>;
type ExternalFileClosePrompt =
    Box<dyn Fn(&str, Option<&winit::window::Window>) -> ExternalFileCloseChoice>;
type ExternalFileSavePathChooser =
    Box<dyn Fn(&str, Option<&winit::window::Window>) -> Option<std::path::PathBuf>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalFileCloseChoice {
    Save,
    Discard,
    Cancel,
}

fn prompt_external_file_close(
    file_name: &str,
    parent_window: Option<&winit::window::Window>,
) -> ExternalFileCloseChoice {
    let description = format!("是否保存对「{file_name}」的更改？");
    let mut dialog = rfd::MessageDialog::new()
        .set_title("未保存的更改")
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
            "保存".to_owned(),
            "放弃".to_owned(),
            "取消".to_owned(),
        ))
        .set_level(rfd::MessageLevel::Warning);
    if let Some(parent_window) = parent_window {
        dialog = dialog.set_parent(parent_window);
    }
    let result = dialog.show();
    match result {
        rfd::MessageDialogResult::Custom(label) if label == "保存" => {
            ExternalFileCloseChoice::Save
        }
        rfd::MessageDialogResult::Custom(label) if label == "放弃" => {
            ExternalFileCloseChoice::Discard
        }
        _ => ExternalFileCloseChoice::Cancel,
    }
}

fn choose_external_file_save_path(
    default_file_name: &str,
    parent_window: Option<&winit::window::Window>,
) -> Option<std::path::PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .add_filter("文本文档", notora_core::EXTERNAL_TEXT_FILE_EXTENSIONS)
        .set_file_name(default_file_name);
    if let Some(parent_window) = parent_window {
        dialog = dialog.set_parent(parent_window);
    }
    dialog.save_file()
}

fn resolve_pointer_cursor(
    product_cursor: Option<winit::window::CursorIcon>,
    editor_cursor: Option<winit::window::CursorIcon>,
) -> winit::window::CursorIcon {
    product_cursor.or(editor_cursor).unwrap_or(winit::window::CursorIcon::Default)
}

fn choose_workspace_directory() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().set_title("设置工作区根目录").pick_folder()
}

fn validate_workspace_transition_target(
    request: &WorkspaceTransitionRequest,
) -> Result<(), String> {
    let root = request.root();
    match request {
        WorkspaceTransitionRequest::OpenExisting { .. } => {
            let metadata = std::fs::metadata(root)
                .map_err(|error| format!("无法读取工作区目录 {}：{error}", root.display()))?;
            if !metadata.is_dir() {
                return Err(format!("工作区路径不是目录：{}", root.display()));
            }
        }
        WorkspaceTransitionRequest::Create { .. } => {
            match std::fs::symlink_metadata(root) {
                Ok(_) => return Err(format!("新工作区路径已存在：{}", root.display())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("无法检查新工作区路径 {}：{error}", root.display()));
                }
            }
            let Some(parent) = root.parent() else {
                return Err("新工作区路径缺少父目录".to_owned());
            };
            let parent_metadata = std::fs::metadata(parent)
                .map_err(|error| format!("无法读取新工作区父目录 {}：{error}", parent.display()))?;
            if !parent_metadata.is_dir() {
                return Err(format!("新工作区父路径不是目录：{}", parent.display()));
            }
        }
    }
    Ok(())
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
    session_restore_runtime: SessionRestoreRuntime,
    document_runtime: DocumentRuntime,
    frame_runtime: FrameRuntime,
    product: NotoraProduct,
    workspace_controller: WorkspaceController,
    workspace_directory_chooser: WorkspaceDirectoryChooser,
    external_file_close_prompt: ExternalFileClosePrompt,
    external_file_save_path_chooser: ExternalFileSavePathChooser,
    workspace_transition_runtime: WorkspaceTransitionRuntime,
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
        state.layout.navigation_pane_visibility = loaded_session.session.navigation_pane_visibility;
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
            session_restore_runtime: SessionRestoreRuntime::default(),
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
            external_file_close_prompt: Box::new(prompt_external_file_close),
            external_file_save_path_chooser: Box::new(choose_external_file_save_path),
            workspace_transition_runtime: WorkspaceTransitionRuntime::default(),
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
    ) -> Option<appkit_core::tab_set::types::TabId> {
        self.document_runtime.tab_for(identity)
    }

    pub fn request_preview_promotion(&mut self) {
        self.dispatch_action(NotoraAction::PromotePreviewRequested);
    }

    /// 系统 open event 与拖入路径的统一产品入口。
    pub fn receive_system_open_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        self.dispatch_action(NotoraAction::ExternalPathsReceived(paths));
    }

    pub fn open_document_sender(&self) -> crate::product::OpenDocumentSender {
        self.product.open_document_sender()
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
        tab_id: appkit_core::tab_set::types::TabId,
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
        if !matches!(command, WorkspaceCommand::SelectionCancelled) {
            self.session_restore_runtime.cancel();
        }
        let result = self.workspace_controller.execute(command, &mut self.product)?;
        self.apply_workspace_command_result(&result);
        Ok(result)
    }

    fn apply_workspace_command_result(&mut self, result: &WorkspaceCommandResult) {
        match result {
            WorkspaceCommandResult::Opened(workspace) => {
                self.action_runtime.set_active_workspace(
                    workspace.descriptor.workspace_id,
                    workspace.generation,
                    workspace.descriptor.root.clone(),
                );
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
    }

    fn prepare_workspace_transition(&mut self, request: WorkspaceTransitionRequest) {
        if self.workspace_transition_runtime.is_active() {
            self.dispatch_action(NotoraAction::WorkspaceTransitionFailed(
                "已有工作区切换正在进行".to_owned(),
            ));
            return;
        }
        if let Err(message) = validate_workspace_transition_target(&request) {
            self.dispatch_action(NotoraAction::WorkspaceTransitionFailed(message));
            return;
        }

        let candidates = self.document_runtime.workspace_note_save_candidates();
        if !self.workspace_transition_runtime.begin(request, &candidates) {
            self.dispatch_action(NotoraAction::WorkspaceTransitionFailed(
                "已有工作区切换正在进行".to_owned(),
            ));
            return;
        }

        for candidate in candidates {
            let Some(origin) = self.document_origin_for_tab(candidate.tab_id) else {
                self.fail_workspace_transition("工作区未切换：无法确定待保存笔记的来源".to_owned());
                return;
            };
            self.document_runtime.request_immediate_workspace_note_save(&origin, candidate);
        }
        self.process_due_autosaves();
        self.reconcile_workspace_transition_saves();
    }

    fn reconcile_workspace_transition_saves(&mut self) {
        if !self.workspace_transition_runtime.is_active() {
            return;
        }
        let candidates = self.workspace_transition_runtime.save_candidates();
        let mut saved_tabs = Vec::new();
        for candidate in candidates {
            if let Some(message) = self.document_runtime.save_failure_message(candidate.tab_id) {
                self.fail_workspace_transition(format!("工作区未切换：笔记保存失败：{message}"));
                return;
            }
            let Some((content_revision, dirty)) =
                self.document_runtime.workspace_note_revision(candidate.tab_id)
            else {
                self.fail_workspace_transition(
                    "工作区未切换：待保存笔记在切换前已不可用".to_owned(),
                );
                return;
            };
            if content_revision != candidate.content_revision {
                self.fail_workspace_transition(
                    "工作区未切换：笔记在保存完成前又发生了变化".to_owned(),
                );
                return;
            }
            if !dirty && self.document_runtime.workspace_note_is_saved_at(candidate) {
                saved_tabs.push(candidate.tab_id);
            }
        }

        self.workspace_transition_runtime.complete_saves(saved_tabs);
        let Some(request) = self.workspace_transition_runtime.take_ready_request() else {
            return;
        };
        self.apply_workspace_transition(request);
    }

    fn apply_workspace_transition(&mut self, request: WorkspaceTransitionRequest) {
        self.dispatch_action(NotoraAction::WorkspaceTransitionApplying);
        let command = match &request {
            WorkspaceTransitionRequest::Create { root } => {
                WorkspaceCommand::Create { root: root.clone() }
            }
            WorkspaceTransitionRequest::OpenExisting { root } => {
                WorkspaceCommand::OpenExisting { root: root.clone() }
            }
        };
        let result = self.workspace_controller.execute(command, &mut self.product);
        match result {
            Ok(WorkspaceCommandResult::Opened(workspace)) => {
                self.document_runtime.close_workspace_note_tabs();
                self.action_runtime.set_active_workspace(
                    workspace.descriptor.workspace_id,
                    workspace.generation,
                    workspace.descriptor.root.clone(),
                );
                self.request_navigation_tree();
                self.dispatch_action(NotoraAction::WorkspaceTransitionCompleted);
                self.schedule_session_persistence();
                self.window_runtime.schedule_redraw();
            }
            Ok(WorkspaceCommandResult::Unchanged | WorkspaceCommandResult::Closed { .. }) => {
                self.fail_workspace_transition("工作区未切换：目标工作区没有成功激活".to_owned());
            }
            Err(error) => {
                let mut message = format!("工作区未切换：{error}");
                if matches!(request, WorkspaceTransitionRequest::Create { .. })
                    && request.root().exists()
                {
                    message.push_str("；新目录可能已创建，但初始化未完成，可检查后重试");
                }
                self.fail_workspace_transition(message);
            }
        }
    }

    fn fail_workspace_transition(&mut self, message: String) {
        self.workspace_transition_runtime.cancel();
        self.dispatch_action(NotoraAction::WorkspaceTransitionFailed(message));
    }

    pub fn shell_layout(&self) -> ShellLayout {
        let dpi = self.document_runtime.editor().scale_factor() as f32;
        let (window_width_px, window_height_px) = self.window_runtime.size();
        let editor_pane_mode =
            crate::render::selected_editor_pane_mode(self.action_runtime.state());
        let title_height =
            window_runtime::frame_state(self.document_runtime.editor().window()).title_height(dpi);
        let mut layout = ShellLayout::compute_below_title_bar(
            ShellLayoutInput {
                window_width_px,
                window_height_px,
                dpi,
                navigation_width_logical: self
                    .action_runtime
                    .state()
                    .layout
                    .navigation_width_logical,
                card_list_width_logical: self.action_runtime.state().layout.card_list_width_logical,
                navigation_pane_visibility: self
                    .action_runtime
                    .state()
                    .layout
                    .navigation_pane_visibility,
                compact_content: self.action_runtime.state().layout.compact_content,
                compact_navigation: self.action_runtime.state().layout.compact_navigation,
                editor_property_row_visible: editor_pane_mode.shows_property_row(),
                editor_header_visible: editor_pane_mode.shows_header(),
            },
            title_height,
        );
        if self.document_search_is_visible() && layout.editor_rect != ui::Rect::ZERO {
            let search_height =
                (ui::search_bar::SEARCH_BAR_HEIGHT * dpi).min(layout.editor_body_rect.h);
            layout.editor_body_rect.y += search_height;
            layout.editor_body_rect.h -= search_height;
        }
        layout
    }

    pub fn dispatch_action(&mut self, action: NotoraAction) {
        if matches!(action, NotoraAction::DocumentSearchRequested) {
            self.open_document_search();
            return;
        }
        if self.action_runtime.state().will_leave_title_focus(&action)
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
            let execution =
                EffectExecutor::execute(effect, |effect| self.execute_product_effect(effect));
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

    fn synchronize_product_focus(&mut self) {
        let focus_target = self.action_runtime.state().layout.focus_target;
        let editor_is_active =
            focus_target == crate::FocusTarget::Editor && self.active_editor_matches_selection();
        self.document_runtime.editor_mut().set_active_cursor_paint_enabled(editor_is_active);
        if !editor_is_active {
            let _ = self.update_editor_preedit(String::new(), None);
        }
        self.frame_runtime.synchronize_focus(focus_target, Instant::now());
        self.synchronize_document_search();
        let ime_allowed = self.frame_runtime.focused_text_input_ime_allowed();
        if let Some(window) = self.document_runtime.editor().window() {
            window.set_ime_allowed(ime_allowed);
        }
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
        self.handle_editor_key_input_with_physical_key(key, modifiers, None);
    }

    pub(crate) fn handle_editor_key_input_with_physical_key(
        &mut self,
        key: ui::KeyCode,
        modifiers: ui::core::Modifiers,
        physical_key: Option<winit::keyboard::PhysicalKey>,
    ) {
        let context = self.editor_input_context();
        let outcome = self.document_runtime.editor_mut().handle_key_input_with_physical_key(
            context,
            key,
            modifiers,
            physical_key,
        );
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
        if !self.document_runtime.editor().editor_hit_test_allowed(context, (px, py)) {
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
        let outcome = self.document_runtime.editor_mut().apply_active_scrollbar_action(action);
        self.apply_editor_outcome(outcome);
    }

    pub fn set_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<ShellEvent>) {
        self.window_runtime.set_event_loop_proxy(event_loop_proxy);
    }

    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        self.window_runtime.set_focused(focused, self.document_runtime.editor_mut());
        self.synchronize_product_focus();
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

    pub(crate) fn route_window_frame_event(
        &mut self,
        event: &ui::Event,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        let route = self.frame_runtime.route_window_frame_event(event);
        if let Some(cursor) = route.cursor {
            self.set_window_cursor(cursor);
        }
        if route.needs_redraw {
            self.window_runtime.schedule_redraw();
        }
        if let Some(action) = route.action {
            self.apply_window_frame_action(action, event_loop);
        }
        route.consumed
    }

    fn apply_window_frame_action(
        &mut self,
        action: ui::window_frame::WindowFrameAction,
        event_loop: &ActiveEventLoop,
    ) {
        use ui::window_frame::WindowFrameAction;
        if action == WindowFrameAction::ToggleNavigation {
            self.toggle_navigation_from_title_bar();
            return;
        }
        if action == WindowFrameAction::Close {
            self.shutdown();
            event_loop.exit();
            return;
        }
        let Some(window) = self.document_runtime.editor().window() else {
            return;
        };
        match action {
            WindowFrameAction::Drag => {
                if let Err(error) = window_runtime::drag_window(window) {
                    eprintln!("notora window drag failed: {error}");
                }
            }
            WindowFrameAction::Resize(direction) => {
                if let Err(error) = window.drag_resize_window(direction) {
                    eprintln!("notora window resize failed: {error}");
                }
            }
            WindowFrameAction::Minimize => window.set_minimized(true),
            WindowFrameAction::ToggleMaximize => window.set_maximized(!window.is_maximized()),
            WindowFrameAction::Close => {}
            WindowFrameAction::ToggleNavigation => {}
        }
        window.request_redraw();
    }

    fn toggle_navigation_from_title_bar(&mut self) {
        if !self.frame_runtime.navigation_toggle_available(self.state().layout.overlay) {
            return;
        }
        self.action_runtime.set_responsive_mode(self.shell_layout().responsive_mode);
        self.dispatch_action(NotoraAction::NavigationPaneVisibilityToggled);
    }

    pub(crate) fn editor_pointer_is_captured(&self) -> bool {
        self.document_runtime.editor().pointer_capture()
            != appkit_shell::editor_runtime::MouseCapture::None
    }

    pub(crate) fn set_pointer_position(&mut self, px: f32, py: f32) {
        self.window_runtime.set_pointer_position(px, py, self.document_runtime.editor());
    }

    pub(crate) fn pointer_position(&self) -> (f32, f32) {
        self.window_runtime.pointer_position()
    }

    pub(crate) fn set_scale_factor(&mut self, scale_factor: f64) {
        self.document_runtime.editor_mut().set_scale_factor(scale_factor);
        self.window_runtime.synchronize_window_chrome(self.document_runtime.editor());
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
            .resume(event_loop, notora_window_attributes(), font_system, font_size, &font_family)
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
        self.window_runtime.synchronize_window_chrome(self.document_runtime.editor());
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
        self.window_runtime.synchronize_window_chrome(self.document_runtime.editor());
        let _ = self.document_runtime.editor_mut().resize_now(width, height);
        self.window_runtime.schedule_redraw();
    }

    /// 处理后台产品结果；无事件循环的宿主可主动轮询此入口。
    pub fn drain_product_events(&mut self) {
        let completions = ProductEventCoordinator::drain(&mut self.product);
        for event in completions.events {
            match event {
                NotoraProductEvent::WorkspaceBootstrap(completion) => {
                    self.complete_session_workspace_restore(completion);
                }
                NotoraProductEvent::Workspace(event) => {
                    self.apply_workspace_completion(event.completion);
                }
                NotoraProductEvent::Document(completion) => {
                    self.apply_document_completion(completion);
                }
                NotoraProductEvent::Persistence(completion) => {
                    match self.persistence_runtime.apply_completion(completion) {
                        PersistenceCompletionOutcome::SettingsUpdated => {
                            self.window_runtime.schedule_redraw();
                        }
                        PersistenceCompletionOutcome::SessionFailed { message } => {
                            self.dispatch_action(NotoraAction::NoteCommandFailed(message));
                        }
                    }
                }
            }
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
        if self.route_document_search_event(event) {
            return (true, None);
        }
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
        if let Some(command) = route.canvas_view_command
            && let Some(snapshot) = self.document_runtime.editor().active_canvas_viewport_snapshot()
        {
            let outcome = self
                .document_runtime
                .editor_mut()
                .apply_active_canvas_viewport_action(command.viewport_action(snapshot.viewport));
            self.apply_editor_outcome(outcome);
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
        self.synchronize_document_search();
        self.window_runtime.synchronize_window_chrome(self.document_runtime.editor());
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
        let rendered_frame = self.frame_runtime.render_frame(&mut self.document_runtime, input);
        if rendered_frame.is_ok() && editor_is_active {
            self.frame_runtime.record_restored_document_rendered();
        }
        rendered_frame
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
        self.request_immediate_shutdown_workspace_note_saves();
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

    fn request_immediate_shutdown_workspace_note_saves(&mut self) {
        for candidate in self.document_runtime.workspace_note_save_candidates() {
            let Some(origin) = self.document_origin_for_tab(candidate.tab_id) else {
                continue;
            };
            self.document_runtime.request_immediate_workspace_note_save(&origin, candidate);
        }
    }

    fn write_dirty_snapshots_in_background(&self) {
        let plans = collect_dirty_snapshots(
            &self.document_runtime.editor().workspace_snapshot(),
            self.document_runtime.encrypted_note_tabs(),
        );
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

    fn install_created_encrypted_note(
        &mut self,
        result: &notora_core::note_command::NoteCommandResult,
    ) {
        let Some(notora_core::CreatedNoteAccess::Encrypted { session }) =
            result.created_access.as_ref()
        else {
            return;
        };
        let Some(workspace) = self.workspace_controller.active_workspace() else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "加密笔记已创建，但工作区已经关闭".to_owned(),
            ));
            return;
        };
        let identity = DocumentIdentity::Note(result.note.note_id);
        let path = workspace.descriptor.root.join(&result.note.relative_path);
        let disk_revision = match appkit_core::file_safety::capture_revision(&path) {
            Ok(revision) => revision,
            Err(error) => {
                self.dispatch_action(NotoraAction::NoteCommandFailed(format!(
                    "加密笔记已创建，但无法读取磁盘状态：{error}"
                )));
                return;
            }
        };
        let request = DocumentLoadRequest {
            identity,
            selection_generation: self.action_runtime.state().library.selected_document_generation,
        };
        let selection = self.document_selection();
        let outcome = self.document_runtime.install_created_encrypted_note(
            request,
            LoadedDocument { path, contents: String::new(), disk_revision: Some(disk_revision) },
            std::sync::Arc::clone(session),
            selection,
        );
        self.apply_document_outcome(outcome);
        let Some(metadata) = result.created_editor_metadata.clone() else {
            self.dispatch_action(NotoraAction::NoteCommandFailed(
                "加密笔记已创建，但编辑区元数据缺失".to_owned(),
            ));
            return;
        };
        self.dispatch_action(NotoraAction::ActiveEditorMetadataLoaded {
            request,
            metadata,
            tags: Vec::new(),
        });
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

    fn promote_preview_for_tab(&mut self, tab_id: appkit_core::tab_set::types::TabId) {
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
            self.execute_document_command(command);
        }
        self.apply_shell_effect(outcome.shell_effect);
        self.window_runtime.merge_redraw_request(outcome.needs_redraw);
        self.reconcile_workspace_transition_saves();
    }

    fn apply_editor_outcome(&mut self, outcome: EditorOutcome) {
        for notification in &outcome.notifications {
            self.handle_editor_notification(notification);
        }
        self.apply_shell_effect(outcome.shell_effect);
        self.reconcile_workspace_transition_saves();
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
                self.complete_pending_external_file_close(*tab_id);
            }
            EditorNotification::SaveFailed { tab_id, .. } => {
                self.cancel_pending_external_file_close(*tab_id);
            }
            EditorNotification::ActiveDocumentChanged { tab_id: None }
            | EditorNotification::PathChanged { .. }
            | EditorNotification::DirtyChanged { .. }
            | EditorNotification::CloseRequested { .. } => {}
        }
    }

    fn complete_pending_external_file_close(&mut self, tab_id: appkit_core::tab_set::types::TabId) {
        let Some(DocumentIdentity::ExternalFile(external_file_id)) =
            self.document_runtime.identity_for(tab_id)
        else {
            return;
        };
        let document_is_clean = self
            .document_runtime
            .editor()
            .document_summary(tab_id)
            .is_some_and(|summary| !summary.dirty);
        let save_as_still_pending = matches!(
            self.action_runtime.state().external_files.session(external_file_id),
            Some(ExternalFileSession::Untitled { .. } | ExternalFileSession::Missing { .. })
        );
        if !document_is_clean
            || save_as_still_pending
            || !self.document_runtime.take_external_file_close_after_save(external_file_id)
        {
            return;
        }
        self.dispatch_action(NotoraAction::ExternalFileCloseRequested(external_file_id));
    }

    fn cancel_pending_external_file_close(&mut self, tab_id: appkit_core::tab_set::types::TabId) {
        let Some(DocumentIdentity::ExternalFile(external_file_id)) =
            self.document_runtime.identity_for(tab_id)
        else {
            return;
        };
        self.document_runtime.cancel_external_file_close_after_save(external_file_id);
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
        tab_id: appkit_core::tab_set::types::TabId,
        pending_trash_move: PendingTrashMove,
    ) -> bool {
        self.document_runtime
            .pending_trash_move_has_current_saved_document(tab_id, pending_trash_move)
    }

    #[cfg(test)]
    fn pending_note_move_has_current_saved_document(
        &self,
        tab_id: appkit_core::tab_set::types::TabId,
        pending_note_move: &PendingNoteMove,
    ) -> bool {
        self.document_runtime
            .pending_note_move_has_current_saved_document(tab_id, pending_note_move)
    }

    #[cfg(test)]
    fn pending_title_update_has_current_saved_document(
        &self,
        tab_id: appkit_core::tab_set::types::TabId,
        pending_title_update: &PendingTitleUpdate,
    ) -> bool {
        self.document_runtime
            .pending_title_update_has_current_saved_document(tab_id, pending_title_update)
    }

    fn save_untitled_external_file(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
        external_file_id: notora_core::ExternalFileId,
    ) {
        let default_file_name =
            self.document_runtime.editor().tab_title(tab_id).unwrap_or_else(|| "未命名".to_owned());
        let Some(path) = (self.external_file_save_path_chooser)(
            &default_file_name,
            self.document_runtime.editor().window(),
        ) else {
            return;
        };
        let (_, outcome) = self.document_runtime.save_external_file_as_to_path(
            tab_id,
            external_file_id,
            path,
            self.window_runtime.event_loop_proxy(),
        );
        self.apply_document_outcome(outcome);
    }

    fn start_external_save_as_canonicalization(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
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

    fn request_catalog_reindex_after_note_save(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
        saved_path: Option<&std::path::Path>,
    ) {
        if !matches!(self.document_runtime.identity_for(tab_id), Some(DocumentIdentity::Note(_))) {
            return;
        }
        let relative_path = self.workspace_controller.active_workspace().and_then(|workspace| {
            let saved_path = saved_path?;
            saved_path
                .strip_prefix(&workspace.descriptor.root)
                .ok()
                .map(std::path::Path::to_path_buf)
        });
        if self.workspace_controller.request_catalog_reindex(relative_path).is_err() {
            self.document_runtime.record_catalog_reconciliation(true);
        }
    }

    fn document_origin_for_tab(
        &self,
        tab_id: appkit_core::tab_set::types::TabId,
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
        tab_id: appkit_core::tab_set::types::TabId,
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
            | NotoraAction::WorkspaceRootExpansionToggled
            | NotoraAction::TagRootExpansionToggled
            | NotoraAction::NavigationExpansionToggled(_)
            | NotoraAction::CardSelected(_)
            | NotoraAction::CardActivated(_)
            | NotoraAction::NoteCommandCompleted(_)
            | NotoraAction::CompactNavigationRequested
            | NotoraAction::NavigationPaneVisibilityToggled
            | NotoraAction::CompactBackRequested
            | NotoraAction::ExternalFileOpened(_)
            | NotoraAction::ExternalFileCloseCompleted(_)
            | NotoraAction::ExternalFilesClearCompleted { .. }
            | NotoraAction::SplitterDragged { .. }
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalFileCloseBlocker {
    UnsavedChanges,
    PinnedTab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnsavedExternalFileCloseHandling {
    Prompt,
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalFileCloseOutcome {
    Closed,
    SavePending,
    Cancelled,
    Blocked(ExternalFileCloseBlocker),
}

impl ExternalFileCloseBlocker {
    fn message(self) -> &'static str {
        match self {
            Self::UnsavedChanges => "文件仍有未保存修改，请先保存后再关闭",
            Self::PinnedTab => "文件标签页已固定，请先取消固定",
        }
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
mod session_restore_tests;

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
            storage: CreateNoteStorage::Unencrypted,
        }));
        Vec::new()
    }

    fn choose_workspace_root(&mut self) {
        self.select_workspace_root();
    }

    fn close_external_file(
        &mut self,
        external_file_id: notora_core::ExternalFileId,
    ) -> Vec<NotoraAction> {
        match self
            .try_close_external_file(external_file_id, UnsavedExternalFileCloseHandling::Prompt)
        {
            ExternalFileCloseOutcome::Closed => {
                vec![NotoraAction::ExternalFileCloseCompleted(external_file_id)]
            }
            ExternalFileCloseOutcome::SavePending | ExternalFileCloseOutcome::Cancelled => {
                Vec::new()
            }
            ExternalFileCloseOutcome::Blocked(blocker) => {
                vec![NotoraAction::ExternalFileCloseFailed(blocker.message().to_owned())]
            }
        }
    }

    fn close_all_external_files(&mut self) -> Vec<NotoraAction> {
        let external_file_ids = self
            .action_runtime
            .state()
            .external_files
            .sessions()
            .iter()
            .map(crate::external_files::ExternalFileSession::external_file_id)
            .collect::<Vec<_>>();
        let mut closed_external_file_ids = Vec::with_capacity(external_file_ids.len());
        let mut blocked_count = 0;
        for external_file_id in external_file_ids {
            match self
                .try_close_external_file(external_file_id, UnsavedExternalFileCloseHandling::Block)
            {
                ExternalFileCloseOutcome::Closed => {
                    closed_external_file_ids.push(external_file_id);
                }
                ExternalFileCloseOutcome::SavePending
                | ExternalFileCloseOutcome::Cancelled
                | ExternalFileCloseOutcome::Blocked(_) => blocked_count += 1,
            }
        }
        vec![NotoraAction::ExternalFilesClearCompleted { closed_external_file_ids, blocked_count }]
    }

    fn try_close_external_file(
        &mut self,
        external_file_id: notora_core::ExternalFileId,
        unsaved_handling: UnsavedExternalFileCloseHandling,
    ) -> ExternalFileCloseOutcome {
        let identity = DocumentIdentity::ExternalFile(external_file_id);
        let Some(tab_id) = self.document_runtime.tab_for(identity) else {
            return ExternalFileCloseOutcome::Closed;
        };
        if self.document_runtime.external_file_close_is_pending(external_file_id) {
            return ExternalFileCloseOutcome::SavePending;
        }
        match self.document_runtime.editor().close_decision(tab_id) {
            Some(appkit_shell::editor_tab_set::CloseTabDecision::NeedsSavePrompt) => {
                return self.resolve_unsaved_external_file_close(
                    identity,
                    tab_id,
                    unsaved_handling,
                );
            }
            Some(appkit_shell::editor_tab_set::CloseTabDecision::Pinned) => {
                return ExternalFileCloseOutcome::Blocked(ExternalFileCloseBlocker::PinnedTab);
            }
            Some(appkit_shell::editor_tab_set::CloseTabDecision::CanClose) | None => {}
        }
        self.close_document_runtime(identity);
        ExternalFileCloseOutcome::Closed
    }

    fn resolve_unsaved_external_file_close(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::tab_set::types::TabId,
        unsaved_handling: UnsavedExternalFileCloseHandling,
    ) -> ExternalFileCloseOutcome {
        if unsaved_handling == UnsavedExternalFileCloseHandling::Block {
            return ExternalFileCloseOutcome::Blocked(ExternalFileCloseBlocker::UnsavedChanges);
        }
        let file_name =
            self.document_runtime.editor().tab_title(tab_id).unwrap_or_else(|| "未命名".to_owned());
        match (self.external_file_close_prompt)(&file_name, self.document_runtime.editor().window())
        {
            ExternalFileCloseChoice::Save => self.save_external_file_before_close(identity, tab_id),
            ExternalFileCloseChoice::Discard => {
                self.discard_external_file_changes(identity, tab_id)
            }
            ExternalFileCloseChoice::Cancel => ExternalFileCloseOutcome::Cancelled,
        }
    }

    fn save_external_file_before_close(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::tab_set::types::TabId,
    ) -> ExternalFileCloseOutcome {
        let DocumentIdentity::ExternalFile(external_file_id) = identity else {
            return ExternalFileCloseOutcome::Cancelled;
        };
        let Some(session) =
            self.action_runtime.state().external_files.session(external_file_id).cloned()
        else {
            return ExternalFileCloseOutcome::Cancelled;
        };
        let save_started = match session {
            ExternalFileSession::Existing { .. } => {
                let (save_started, outcome) = self
                    .document_runtime
                    .submit_manual_external_save(tab_id, self.window_runtime.event_loop_proxy());
                self.apply_document_outcome(outcome);
                save_started
            }
            ExternalFileSession::Untitled { .. } | ExternalFileSession::Missing { .. } => {
                self.save_external_file_as_before_close(tab_id, external_file_id)
            }
        };
        if !save_started {
            return ExternalFileCloseOutcome::Cancelled;
        }
        self.document_runtime.request_external_file_close_after_save(external_file_id);
        ExternalFileCloseOutcome::SavePending
    }

    fn save_external_file_as_before_close(
        &mut self,
        tab_id: appkit_core::tab_set::types::TabId,
        external_file_id: notora_core::ExternalFileId,
    ) -> bool {
        let default_file_name =
            self.document_runtime.editor().tab_title(tab_id).unwrap_or_else(|| "未命名".to_owned());
        let Some(path) = (self.external_file_save_path_chooser)(
            &default_file_name,
            self.document_runtime.editor().window(),
        ) else {
            return false;
        };
        let (save_started, outcome) = self.document_runtime.save_external_file_as_to_path(
            tab_id,
            external_file_id,
            path,
            self.window_runtime.event_loop_proxy(),
        );
        self.apply_document_outcome(outcome);
        save_started
    }

    fn discard_external_file_changes(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::tab_set::types::TabId,
    ) -> ExternalFileCloseOutcome {
        let outcome = self
            .document_runtime
            .editor_mut()
            .confirm_close(tab_id, appkit_shell::editor_runtime::CloseConfirmation::Discard);
        if self.document_runtime.editor().document_summary(tab_id).is_some() {
            return ExternalFileCloseOutcome::Blocked(ExternalFileCloseBlocker::PinnedTab);
        }
        self.document_runtime.cancel_autosave(tab_id);
        self.document_runtime.clear_save_failure(tab_id);
        self.document_runtime.remove_tab(tab_id);
        self.apply_editor_outcome(outcome);
        if self.action_runtime.state().library.selected_card == Some(identity) {
            self.action_runtime.invalidate_document_selection();
        }
        ExternalFileCloseOutcome::Closed
    }

    fn execute_note_command(&mut self, command: notora_core::note_command::NoteCommand) {
        if let NoteCommand::Move(request) = command {
            self.submit_or_defer_note_move(request);
            return;
        }
        self.submit_note_command(command);
    }

    fn execute_directory_command(&mut self, command: notora_core::WorkspaceDirectoryCommand) {
        if let Err(error) = self.workspace_controller.execute_directory_command(command) {
            self.dispatch_action(NotoraAction::DirectoryCreationFailed(error.to_string()));
        }
    }

    fn commit_title(&mut self, title: String) {
        self.commit_active_note_title(title);
    }

    fn toggle_editor_view(&mut self) {
        self.document_runtime.editor_mut().switch_active_plugin();
    }

    fn toggle_inspector_panel(&mut self) {
        if !self.active_editor_matches_selection() {
            return;
        }
        let outcome = self.document_runtime.editor_mut().toggle_active_inspector_panel();
        self.apply_editor_outcome(outcome);
    }

    fn dispatch_inspector_panel(
        &mut self,
        action: appkit_shell::inspector_panel::StampedInspectorPanelAction,
    ) {
        if !self.active_editor_matches_selection() {
            return;
        }
        let outcome = self.document_runtime.editor_mut().dispatch_inspector_panel_action(action);
        self.apply_editor_outcome(outcome);
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

    fn relock_conflicted_document(
        &mut self,
        identity: DocumentIdentity,
        tab_id: appkit_core::tab_set::types::TabId,
    ) {
        if self.document_runtime.tab_for(identity) != Some(tab_id) {
            return;
        }
        if let DocumentIdentity::Note(note_id) = identity {
            self.document_runtime.discard_cached_unlocked_workspace_note_session(note_id);
        }
        self.close_document_runtime(identity);
        self.dispatch_action(NotoraAction::SaveConflictResolved { identity });
        self.dispatch_action(NotoraAction::CardSelected(identity));
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
        if let Some(outcome) = self.document_runtime.activate_registered_document(identity) {
            self.apply_editor_outcome(outcome);
            return Vec::new();
        }
        let cached_session = match identity {
            DocumentIdentity::Note(note_id) => {
                self.document_runtime.cached_unlocked_workspace_note_session(note_id)
            }
            DocumentIdentity::ExternalFile(_) => None,
        };
        let preparation = if self.action_runtime.state().library.navigation_scope
            == notora_core::NavigationScope::Trash
        {
            self.workspace_controller.prepare_trashed_document(request, cached_session)
        } else {
            self.workspace_controller.prepare_document(request, cached_session)
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
                .add_filter("文本文档", notora_core::EXTERNAL_TEXT_FILE_EXTENSIONS)
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
        self.dispatch_action(NotoraAction::WorkspaceTransitionConfirmed(
            WorkspaceTransitionRequest::OpenExisting { root },
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
            workspace_root_expanded: self
                .action_runtime
                .state()
                .library
                .navigation_tree
                .workspace_root_expanded,
            tag_root_expanded: self
                .action_runtime
                .state()
                .library
                .navigation_tree
                .tag_root_expanded,
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
            navigation_pane_visibility: self
                .action_runtime
                .state()
                .layout
                .navigation_pane_visibility,
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
        let restore_started_at = Instant::now();
        self.frame_runtime.record_session_restore_started();
        self.frame_runtime.expect_restored_document_frame(session.last_document.is_some());
        self.window_runtime
            .restore_size(session.window_geometry.width_px, session.window_geometry.height_px);
        let Some(root) = session.workspace_root.clone().filter(|root| root.is_dir()) else {
            let preserved_external_selection = self.selected_external_document_identity();
            self.finish_session_restore(
                session,
                false,
                restore_started_at,
                preserved_external_selection,
            );
            return;
        };
        let workspace_started_at = Instant::now();
        match self.workspace_controller.begin_open_existing(root, &self.product) {
            Ok(workspace_generation) => {
                self.session_restore_runtime.start(SessionRestore {
                    session,
                    workspace_generation,
                    restore_started_at,
                    workspace_started_at,
                });
            }
            Err(error) => {
                self.action_runtime.record_command_error(error.to_string());
                let preserved_external_selection = self.selected_external_document_identity();
                self.finish_session_restore(
                    session,
                    false,
                    restore_started_at,
                    preserved_external_selection,
                );
            }
        }
    }

    fn complete_session_workspace_restore(
        &mut self,
        completion: crate::product::WorkspaceBootstrapCompletion,
    ) {
        let Some(pending_restore) = self.session_restore_runtime.take() else {
            return;
        };
        if pending_restore.workspace_generation != completion.generation {
            self.session_restore_runtime.start(pending_restore);
            return;
        }
        let preserved_external_selection = self.selected_external_document_identity();
        let completion_result = self
            .workspace_controller
            .complete_open_existing(completion.generation, &mut self.product);
        let workspace_restored = match completion_result {
            Ok(Some(result @ WorkspaceCommandResult::Opened(_))) => {
                let WorkspaceCommandResult::Opened(workspace) = &result else {
                    unreachable!("the match arm only accepts opened workspaces");
                };
                if pending_restore
                    .session
                    .workspace_id
                    .is_some_and(|saved_id| saved_id != workspace.descriptor.workspace_id)
                {
                    if let Ok(closed) = self
                        .workspace_controller
                        .execute(WorkspaceCommand::Close, &mut self.product)
                    {
                        self.apply_workspace_command_result(&closed);
                    }
                    false
                } else {
                    self.apply_workspace_command_result(&result);
                    self.frame_runtime
                        .record_workspace_session_ready(pending_restore.workspace_started_at);
                    true
                }
            }
            Ok(Some(WorkspaceCommandResult::Unchanged | WorkspaceCommandResult::Closed { .. })) => {
                false
            }
            Ok(None) => {
                self.session_restore_runtime.start(pending_restore);
                return;
            }
            Err(error) => {
                self.action_runtime.record_command_error(error.to_string());
                false
            }
        };
        self.finish_session_restore(
            pending_restore.session,
            workspace_restored,
            pending_restore.restore_started_at,
            preserved_external_selection,
        );
    }

    fn selected_external_document_identity(&self) -> Option<DocumentIdentity> {
        self.action_runtime
            .state()
            .library
            .selected_card
            .filter(|identity| matches!(identity, DocumentIdentity::ExternalFile(_)))
    }

    fn finish_session_restore(
        &mut self,
        session: crate::session::ProductSession,
        workspace_restored: bool,
        restore_started_at: Instant,
        preserved_external_selection: Option<DocumentIdentity>,
    ) {
        let focus_policy = self.session_restore_runtime.take_focus_policy();
        let preserve_system_open_target =
            focus_policy == SessionRestoreFocusPolicy::PreserveSystemOpenTarget;
        if !workspace_restored && session.workspace_id.is_some() {
            self.action_runtime
                .record_command_error("上次使用的工作区不可用，或与保存的标识不再匹配".to_owned());
        }
        let saved_last_document =
            if preserve_system_open_target { None } else { session.last_document.clone() };
        let saved_external_path = match &saved_last_document {
            Some(crate::session::SavedDocument::ExternalPath { path }) => Some(path.as_path()),
            Some(crate::session::SavedDocument::Note { .. }) | None => None,
        };
        self.restore_external_paths(session.external_paths, saved_external_path);
        if workspace_restored {
            self.action_runtime.restore_navigation_expansion(
                session.workspace_root_expanded,
                session.tag_root_expanded,
                session.expanded_directories,
            );
            if !preserve_system_open_target {
                self.dispatch_action(NotoraAction::NavigationSelected(
                    session.last_navigation_scope.into(),
                ));
            }
        } else if !preserve_system_open_target
            && session.last_navigation_scope == crate::session::SavedNavigationScope::ExternalFiles
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
        if preserve_system_open_target && let Some(identity) = preserved_external_selection {
            self.dispatch_action(NotoraAction::ExternalFileOpened(identity));
        }
        self.frame_runtime.record_session_restore_finished(restore_started_at);
        self.window_runtime.schedule_redraw();
    }

    pub(crate) fn restore_session_after_first_frame(&mut self) {
        if !self.persistence_runtime.has_pending_session() {
            return;
        }
        self.restore_pending_session();
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
        tab_id: appkit_core::tab_set::types::TabId,
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

    fn save_conflicted_note_copy(&mut self, identity: DocumentIdentity) {
        let Some(path) = rfd::FileDialog::new().add_filter("文本文档", &["txt", "md"]).save_file()
        else {
            return;
        };
        if self.document_runtime.is_encrypted_note(identity) {
            self.dispatch_action(NotoraAction::EncryptedConflictCopyRequired {
                identity,
                target_path: path,
            });
            return;
        }
        let outcome = self.document_runtime.prepare_conflict_copy(identity, path);
        self.apply_document_outcome(outcome);
    }

    fn start_conflict_copy(
        &mut self,
        identity: DocumentIdentity,
        prepared: appkit_shell::editor_runtime::PreparedDocumentSave,
        transform: Option<appkit_shell::editor_runtime::SavePayloadTransform>,
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
                let completion = match transform {
                    Some(transform) => {
                        appkit_shell::editor_runtime::execute_prepared_save_with_transform(
                            prepared, transform,
                        )
                    }
                    None => appkit_shell::editor_runtime::execute_prepared_save(prepared),
                };
                let result = completion.result.map(|_| ()).map_err(|error| error.to_string());
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
        tab_id: appkit_core::tab_set::types::TabId,
        content_revision: u64,
        path: std::path::PathBuf,
        session: Option<std::sync::Arc<textora_encryption::UnlockedNoteSession>>,
    ) {
        let sender = self.product.event_sender();
        if thread::Builder::new()
            .name("notora-conflict-reload".to_owned())
            .spawn(move || match load_conflicted_document(&path, session.as_deref()) {
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
                Err(ConflictReloadError::RequiresUnlock) => {
                    let _ = sender.send(NotoraProductEvent::Document(
                        DocumentCompletion::ConflictReloadRequiresUnlock { identity, tab_id },
                    ));
                }
                Err(ConflictReloadError::Message(message)) => {
                    let _ = sender.send(NotoraProductEvent::Document(
                        DocumentCompletion::ConflictReloadFailed { identity, message },
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
        if !paths.is_empty()
            && (self.persistence_runtime.has_pending_session()
                || self.session_restore_runtime.is_active())
        {
            self.session_restore_runtime.preserve_system_open_target();
        }
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
                    let completion = match read_external_text_file(&path) {
                        Ok((validated, contents, revision)) => {
                            let canonical_path = validated.canonical_path;
                            let document = LoadedDocument {
                                path: canonical_path.as_path().to_path_buf(),
                                contents,
                                disk_revision: Some(revision),
                            };
                            DocumentCompletion::ExternalFileOpenCompleted {
                                canonical_path,
                                document,
                                activate,
                            }
                        }
                        Err(error) => DocumentCompletion::ExternalFileOpenFailed {
                            message: error.to_string(),
                        },
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

fn load_conflicted_document(
    path: &std::path::Path,
    session: Option<&textora_encryption::UnlockedNoteSession>,
) -> Result<LoadedDocument, ConflictReloadError> {
    let Some(session) = session else {
        return load_document(path)
            .map_err(|error| ConflictReloadError::Message(error.to_string()));
    };
    let serialized =
        std::fs::read(path).map_err(|error| ConflictReloadError::Message(error.to_string()))?;
    let contents = match textora_encryption::decrypt_markdown_with_session(&serialized, session) {
        Ok(contents) => contents,
        Err(textora_encryption::EncryptionError::SessionMismatch) => {
            return Err(ConflictReloadError::RequiresUnlock);
        }
        Err(_) => {
            return Err(ConflictReloadError::Message(
                "加密文件内容已损坏，无法重新加载".to_owned(),
            ));
        }
    };
    let disk_revision = appkit_core::file_safety::capture_revision(path)
        .map_err(|error| ConflictReloadError::Message(error.to_string()))?;
    Ok(LoadedDocument { path: path.to_path_buf(), contents, disk_revision: Some(disk_revision) })
}

enum ConflictReloadError {
    RequiresUnlock,
    Message(String),
}

#[cfg(test)]
mod tests;
