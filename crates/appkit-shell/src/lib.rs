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

pub mod canvas_viewport;
mod clipboard;
pub mod cursor_motion;
pub mod display_line_map;
pub mod display_state;
pub mod document_presentation;
pub mod editor_host;
pub mod editor_plugin;
pub mod event;
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
pub mod snap_tree;
pub mod tab_runtime;
pub mod tab_session;
pub mod text_rasterize;
pub mod ui_shell;
pub mod view_route;
pub mod window_input;
pub mod workspace;

pub use event::{ShellEffect, ShellEffectStep, ShellEvent};
pub use product_host::{ProductHost, ProductWakeHandle, WakeError};
