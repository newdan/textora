//! Notora 应用组合根。

use appkit_shell::ShellEvent;
use winit::event_loop::EventLoopProxy;

use crate::action::NotoraAction;
use crate::runtime::NotoraRuntime;
use crate::shell::layout::ShellLayout;
use crate::{
    NotoraPaths, NotoraState, WorkspaceCommand, WorkspaceCommandResult, WorkspaceControllerError,
};

pub use crate::runtime::NotoraAppError;

/// 只负责组合 Notora runtime 并把平台生命周期委托给它。
pub struct NotoraApp {
    runtime: NotoraRuntime,
}

impl NotoraApp {
    pub fn new() -> Self {
        Self { runtime: NotoraRuntime::new() }
    }

    pub fn try_new() -> Result<Self, NotoraAppError> {
        NotoraRuntime::try_new().map(|runtime| Self { runtime })
    }

    pub fn with_paths(paths: NotoraPaths) -> Result<Self, NotoraAppError> {
        NotoraRuntime::with_paths(paths).map(|runtime| Self { runtime })
    }

    pub fn editor_runtime_tab_count(&self) -> usize {
        self.runtime.editor_runtime_tab_count()
    }

    pub fn paths(&self) -> &NotoraPaths {
        self.runtime.paths()
    }

    pub fn recoverable_dirty_snapshots(
        &self,
    ) -> std::io::Result<Vec<crate::dirty_snapshot::RecoverableDirtySnapshot>> {
        self.runtime.recoverable_dirty_snapshots()
    }

    pub fn state(&self) -> &NotoraState {
        self.runtime.state()
    }

    pub fn document_tab_for(
        &self,
        identity: notora_core::DocumentIdentity,
    ) -> Option<appkit_core::workspace::types::TabId> {
        self.runtime.document_tab_for(identity)
    }

    pub fn request_preview_promotion(&mut self) {
        self.runtime.request_preview_promotion();
    }

    pub fn receive_system_open_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        self.runtime.receive_system_open_paths(paths);
    }

    pub fn request_external_file_dialog(&mut self) {
        self.runtime.request_external_file_dialog();
    }

    pub fn execute_workspace_command(
        &mut self,
        command: WorkspaceCommand,
    ) -> Result<WorkspaceCommandResult, WorkspaceControllerError> {
        self.runtime.execute_workspace_command(command)
    }

    pub fn shell_layout(&self) -> ShellLayout {
        self.runtime.shell_layout()
    }

    pub fn dispatch_action(&mut self, action: NotoraAction) {
        self.runtime.dispatch_action(action);
    }

    pub fn update_editor_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) -> bool {
        self.runtime.update_editor_preedit(text, cursor)
    }

    pub fn set_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<ShellEvent>) {
        self.runtime.set_event_loop_proxy(event_loop_proxy);
    }

    pub fn drain_product_events(&mut self) {
        self.runtime.drain_product_events();
    }

    pub(crate) fn runtime_mut(&mut self) -> &mut NotoraRuntime {
        &mut self.runtime
    }
}

impl Default for NotoraApp {
    fn default() -> Self {
        Self::new()
    }
}
