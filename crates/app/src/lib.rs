//! textora application crate.
//!
//! Provides the winit + wgpu application lifecycle.

#![allow(
    clippy::too_many_arguments,
    reason = "init/dispatch functions take many params; remove when refactored into builder"
)]
#![allow(
    clippy::field_reassign_with_default,
    reason = "builder-pattern idiom in app init; remove after unifying constructors"
)]
#![allow(
    clippy::single_range_in_vec_init,
    reason = "text-shaping pipeline uses single-element ranges; remove when generalized"
)]
#![allow(
    clippy::if_same_then_else,
    reason = "branches kept symmetric during migration; remove when platform paths diverge"
)]
#![allow(
    clippy::question_mark,
    reason = "explicit early-return in GPU init for clearer error context; remove when unified"
)]
#![allow(
    clippy::unnecessary_unwrap,
    reason = "explicit unwrap for clarity in window setup; remove when Option handling is consolidated"
)]
#![allow(
    clippy::redundant_locals,
    reason = "temporary binding during reshape migration; remove when pipeline is stable"
)]
#![allow(
    clippy::unnecessary_sort_by,
    reason = "custom sort for tab ordering; remove when Ord is derived"
)]
#![allow(
    clippy::large_enum_variant,
    reason = "AppEvent variants differ in size; boxing the large variant is a separate perf task"
)]
#![allow(
    clippy::empty_line_after_doc_comments,
    reason = "style preference for readability during active development"
)]
#![allow(
    clippy::match_like_matches_macro,
    reason = "explicit match arms for readability in state-machine code"
)]

mod actions;
mod app;
pub(crate) mod app_effect;
mod app_event;
mod app_init;
mod app_lifecycle;
mod app_renderer;
mod app_reshape;
mod app_scroll;
mod app_tab;
mod app_window;
mod cli;
pub(crate) mod dispatch;
#[cfg(test)]
mod document_fixture;
mod editor_host;
#[cfg(test)]
mod external_change_tests;
mod input_mapper;
mod library_file_monitor;
mod library_registry;
#[cfg(target_os = "macos")]
mod macos_open_documents;
mod mouse;
mod product_paths;
pub(crate) mod settings_io;
mod sync_connection_store;
mod sync_controller;
mod sync_secret_store;
mod sync_settings_page;
mod sync_settings_types;
mod sync_view_model;
pub(crate) mod sys;
mod textora_product;
mod textora_settings_overlay;
pub(crate) mod theme_loader;

mod events;

mod menu_handler;
mod native_menu;
pub(crate) mod plugins;
mod search_escape;
mod settings_overlay;
mod ui_shell;
mod workspace_persistence;
mod workspace_product;
mod workspace_tab_factory;

pub use app::App;
pub use app_event::AppEvent;
pub use appkit_shell::gpu::{GpuError, headless_init};
pub use cli::{CliArgs, parse_args};
#[cfg(target_os = "macos")]
pub use macos_open_documents::install_macos_open_document_handler;
pub use textora_product::OpenDocumentSender;

/// Hidden re-export module for tests and benchmarks.
/// Not part of the public API — may change without notice.
#[doc(hidden)]
pub mod dev_support {
    pub use appkit_shell::measure_adapter::MeasureFromShaper;
    pub use appkit_shell::snap_tree::{DisplayLineEntry, SnapTree};
}

pub mod clipboard;
#[cfg(test)]
mod dispatch_boundary_tests;
