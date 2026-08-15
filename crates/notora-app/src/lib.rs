//! notora 产品层。
//!
//! 本 crate 组合 headless 的 `notora-core`、通用 UI 与 editor runtime；它不依赖
//! `textora-app` 或 `textora-sync`。产品状态、窗口生命周期和编辑器接入在后续 N2
//! 子任务中逐步实现。

mod app;
pub mod autosave;
pub mod dirty_snapshot;
pub mod document_registry;
mod editor_adapter;
pub mod editor_pane;
pub mod effect_executor;
pub mod events;
pub mod external_files;
mod index_worker;
mod new_workspace_dialog;
mod notora_settings_view;
mod paths;
mod persistence_worker;
pub mod product;
pub mod render;
mod runtime;
pub mod runtime_lru;
pub mod search_controller;
pub mod session;
pub mod settings;
pub mod settings_overlay;
pub mod shell;
mod shell_effect_executor;
mod state;
pub mod workspace_controller;

pub mod action;

pub use app::{NotoraApp, NotoraAppError};
pub use external_files::{
    CanonicalExternalPath, ExternalFileOpenError, ExternalFilePathError, ExternalFileSession,
    ExternalFileSessions, OpenExistingExternalFile, RelocateExternalFile,
    ValidatedExternalTextFile, validate_external_text_file,
};
pub use paths::{NotoraPaths, NotoraPathsError};
pub use session::{
    LoadedProductSession, ProductSession, SavedDocument, SavedNavigationScope, SessionError,
    WindowGeometry, load_product_session, save_product_session,
};
pub use settings::{
    AppearanceSettings, EditorSettings, InterfaceSettings, LoadedProductSettings, ProductSettings,
    SettingsError, WorkspaceSettings, load_product_settings, save_product_settings,
};
pub use state::{
    CompactContent, CompactNavigation, FocusTarget, LayoutState, LibraryState,
    NavigationPaneVisibility, NavigationTreeState, NotoraState, OverlayState, Pane,
    ResponsiveLayoutMode, WorkspaceRootState,
};
pub use workspace_controller::{
    ActiveWorkspace, WorkspaceCommand, WorkspaceCommandResult, WorkspaceController,
    WorkspaceControllerError,
};

#[cfg(test)]
mod tests {
    use super::{NotoraApp, NotoraPaths};

    #[test]
    fn creates_an_application_shell_without_textora_product_state() {
        let directory = tempfile::tempdir().expect("test should create a temporary directory");
        let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
            .expect("test should create isolated product paths");
        let app = NotoraApp::with_paths(paths).expect("notora app should construct");
        assert!(app.editor_runtime_tab_count() == 0);
    }
}
