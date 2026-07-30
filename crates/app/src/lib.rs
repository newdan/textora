//! edit+ application crate.
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
    dead_code,
    reason = "crate is mid-migration; unused items will be wired up or removed in later phases"
)]
#![allow(
    clippy::match_like_matches_macro,
    reason = "explicit match arms for readability in state-machine code"
)]

mod actions;
mod app;
mod app_dispatch;
pub(crate) mod app_effect;
mod app_event;
pub(crate) mod dispatch {
    pub(crate) mod chrome;
    pub(crate) mod commands;
    pub(crate) mod editor;
    pub(crate) mod mouse;
    pub(crate) mod search;
    pub(crate) mod tabs;
    pub(crate) mod viewport;
    pub(crate) mod wysiwyg;
}
mod app_init;
mod app_lifecycle;
mod app_renderer;
mod app_reshape;
mod app_scroll;
mod app_search;
mod app_tab;
mod app_window;
pub(crate) use appkit_shell::canvas_viewport;
mod cli;
mod commands;
pub(crate) use appkit_core::content_hash;
pub(crate) use appkit_shell::cursor_motion;
#[cfg(test)]
pub(crate) use appkit_shell::display_line_map;
pub(crate) use appkit_shell::document_presentation;
mod document_view;
pub(crate) mod edit_transaction;
#[cfg(test)]
mod external_change_tests;
pub(crate) use appkit_core::external_document_change;
pub(crate) use appkit_shell::gpu;
pub(crate) use appkit_shell::input_mapper as input;
mod library_file_monitor;
mod library_registry;
pub(crate) use appkit_core::line_index;
#[cfg(target_os = "macos")]
mod macos_open_documents;
mod mouse;
pub(crate) use appkit_core::persistence;
mod product_paths;
#[cfg(test)]
pub(crate) use appkit_shell::render_cache;
pub(crate) use appkit_shell::render_pipeline;
pub(crate) use appkit_shell::render_state;
pub(crate) mod settings_io;
pub(crate) use appkit_shell::smooth_scroll;
pub(crate) use appkit_shell::snap_tree;
mod sync_connection_store;
mod sync_controller;
mod sync_secret_store;
mod sync_settings_page;
mod sync_settings_types;
mod sync_view_model;
pub(crate) mod sys;
pub(crate) use appkit_shell::tab_runtime;
pub(crate) use appkit_shell::tab_session;
#[allow(
    unused_imports,
    reason = "temporary semantic re-export keeps the moved shell module addressable from app"
)]
pub(crate) use appkit_shell::text_rasterize;
mod textora_product;
mod textora_settings_overlay;
pub(crate) mod theme_loader;
pub(crate) use appkit_core::workspace::store as workspace_store;
pub(crate) use appkit_shell::view_route;

pub(crate) use appkit_core::snapshot as dirty_snapshot;
mod events;
pub(crate) use appkit_core::file_history;
pub(crate) use appkit_core::file_safety;

pub(crate) use appkit_shell::measure_adapter;
mod menu_handler;
mod native_menu;
pub(crate) use appkit_core::navigator;
pub(crate) use appkit_shell::paint_backend;
pub(crate) mod plugins;
pub(crate) use appkit_shell::reshape_worker;
mod search_escape;
mod settings_overlay;
#[cfg(test)]
pub(crate) use appkit_shell::mindmap_style_panel as tab;
pub(crate) use appkit_shell::ui_shell;
pub(crate) mod workspace {
    pub(crate) use crate::workspace_tab_factory::ViewportDimensions;
    pub(crate) use appkit_shell::workspace::*;
}
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
    pub use crate::document_view::DocumentView;
    pub use crate::measure_adapter::MeasureFromShaper;
    pub use crate::snap_tree::{DisplayLineEntry, SnapTree};
}

pub mod clipboard;
#[cfg(test)]
mod dispatch_boundary_tests;
