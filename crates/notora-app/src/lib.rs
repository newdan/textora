//! notora 产品层。
//!
//! 本 crate 组合 headless 的 `notora-core`、通用 UI 与 editor runtime；它不依赖
//! `textora-app` 或 `textora-sync`。产品状态、窗口生命周期和编辑器接入在后续 N2
//! 子任务中逐步实现。

mod app;
pub mod effect_executor;
pub mod events;
mod paths;
pub mod product;
pub mod render;
pub mod shell;
mod state;

pub mod action;

pub use app::{NotoraApp, NotoraAppError};
pub use paths::{NotoraPaths, NotoraPathsError};
pub use state::{
    FocusTarget, LayoutState, LibraryState, NotoraState, OverlayState, Pane, ResponsiveLayoutMode,
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
