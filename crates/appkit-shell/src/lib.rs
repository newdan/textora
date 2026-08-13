//! Window, input, plugin-session, and rendering runtime.

#![allow(
    clippy::too_many_arguments,
    reason = "init/dispatch functions take many params; remove when refactored into builder"
)]
#![allow(
    clippy::question_mark,
    reason = "explicit early-return in GPU init for clearer error context; remove when unified"
)]
#![allow(
    clippy::redundant_locals,
    reason = "temporary binding during reshape migration; remove when pipeline is stable"
)]
#![allow(
    clippy::empty_line_after_doc_comments,
    reason = "style preference for readability during active development"
)]

pub mod accessibility_adapter;
pub mod canvas_viewport;
mod clipboard;
pub mod cursor_motion;
pub mod display_line_map;
pub mod display_state;
pub mod document_presentation;
pub mod editor_host;
pub mod editor_plugin;
pub mod editor_runtime;
pub mod event;
mod event_runtime;
pub mod frame_cache;
pub mod gpu;
pub mod input_mapper;
pub mod measure_adapter;
pub mod mindmap_style_panel;
pub mod mouse_state;
pub mod paint_backend;
pub mod prepared_tab;
mod product_host;
pub mod render_cache;
pub mod render_pipeline;
pub mod render_state;
pub mod reshape_worker;
pub mod smooth_scroll;
pub use clipboard::SystemClipboard;
pub mod snap_tree;
pub mod tab_runtime;
pub mod tab_session;
pub mod text_rasterize;
pub mod ui_shell;
pub mod view_route;
pub mod window_input;
pub mod workspace;

pub use event::{ShellEffect, ShellEffectStep, ShellEvent};
pub use event_runtime::{
    DrainStart, EventPump, ProductEventInbox, ProductEventSendError, ProductEventSender,
    ProductWakeRegistrationError, product_event_channel,
};
pub use product_host::{ProductHost, ProductWakeHandle, WakeError};

#[cfg(test)]
mod architecture_boundary_tests {
    #[test]
    fn shell_manifest_has_no_product_dependencies() {
        let manifest = include_str!("../Cargo.toml");
        let markdown_dependency = ["textora", "-markdown"].concat();
        let sync_dependency = ["textora", "-sync"].concat();
        let app_dependency = ["textora", "-app"].concat();

        assert!(!manifest.contains(&markdown_dependency));
        assert!(!manifest.contains(&sync_dependency));
        assert!(
            !manifest
                .lines()
                .any(|line| { line.trim_start().starts_with(&format!("{app_dependency} =")) })
        );
    }

    #[test]
    fn shared_event_runtime_has_no_product_or_domain_dependencies() {
        let source = include_str!("event_runtime.rs");
        let textora_product = ["Textora", "Product"].concat();
        for forbidden in [
            "Notora",
            textora_product.as_str(),
            "NotoraAction",
            "AppAction",
            "WorkspaceId",
            "DocumentIdentity",
        ] {
            assert!(!source.contains(forbidden), "shared event runtime contains {forbidden}");
        }
    }

    #[test]
    fn runtime_has_no_migration_model_bridge() {
        let source = include_str!("editor_runtime/mod.rs");
        let migration_bridge = ["with_model_session_", "for_migration"].concat();
        assert!(!source.contains(&migration_bridge));
    }
}
