//! notora 产品层。
//!
//! 本 crate 组合 headless 的 `notora-core`、通用 UI 与 editor runtime；它不依赖
//! `textora-app` 或 `textora-sync`。产品状态、窗口生命周期和编辑器接入在后续 N2
//! 子任务中逐步实现。

mod app;
pub mod autosave;
pub mod dirty_snapshot;
pub mod document_registry;
mod document_search;
mod editor_adapter;
pub mod editor_pane;
pub mod effect_executor;
pub mod events;
pub mod external_files;
mod index_worker;
#[cfg(target_os = "macos")]
mod macos_open_documents;
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
#[cfg(target_os = "macos")]
pub use macos_open_documents::install_macos_open_document_handler;
pub use paths::{NotoraPaths, NotoraPathsError};
pub use product::OpenDocumentSender;
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

/// 从命令行参数中提取待打开的外部文件路径，忽略程序名与 `-` 开头的系统/选项标志（如 macOS `-psn_...`）。
pub fn parse_cli_open_paths(args: &[String]) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    let mut accept_all_as_paths = false;
    for argument in args.iter().skip(1) {
        if !accept_all_as_paths && argument == "--" {
            accept_all_as_paths = true;
            continue;
        }
        if !accept_all_as_paths && argument.starts_with('-') {
            continue;
        }
        if !argument.is_empty() {
            paths.push(std::path::PathBuf::from(argument));
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{NotoraApp, NotoraPaths, parse_cli_open_paths};

    #[test]
    fn creates_an_application_shell_without_textora_product_state() {
        let directory = tempfile::tempdir().expect("test should create a temporary directory");
        let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
            .expect("test should create isolated product paths");
        let app = NotoraApp::with_paths(paths).expect("notora app should construct");
        assert!(app.editor_runtime_tab_count() == 0);
    }

    #[test]
    fn parse_cli_open_paths_extracts_paths_and_ignores_option_flags() {
        let args = vec![
            "notora".to_owned(),
            "-psn_0_12345".to_owned(),
            "architecture.md".to_owned(),
            "--unknown-flag".to_owned(),
            "notes.txt".to_owned(),
            "--".to_owned(),
            "-leading-dash.md".to_owned(),
            String::new(),
        ];

        assert_eq!(
            parse_cli_open_paths(&args),
            vec![
                PathBuf::from("architecture.md"),
                PathBuf::from("notes.txt"),
                PathBuf::from("-leading-dash.md"),
            ]
        );
    }
}
