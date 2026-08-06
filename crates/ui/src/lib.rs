//! edit+ UI — pure UI component library.
//!
//! Provides rendering primitives and widget components.
//! Depends on core, render, shaping, stdext — no app-layer types.

#![allow(clippy::too_many_arguments)]
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(clippy::overly_complex_bool_expr)]
#![allow(unused_mut)]
#![allow(clippy::type_complexity)]
#![allow(clippy::empty_line_after_doc_comments)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(unused_must_use)]

pub mod canvas;
pub mod constants;
pub mod core;
pub mod decorations;
pub mod gutter;
mod hex_color;
pub mod layout;
pub mod plugin;
pub mod render_geom;
pub mod settings;
pub mod tapered_path;
mod text_renderer;
pub mod theme;
mod theme_file;
mod theme_registry;
pub mod view_mode;
pub mod viewport;
mod widgets;

// 语义根级 re-export：稳定路径
pub use widgets::{
    button, canvas_scrollbars, checkbox, editor_header, editor_toolbar, form, icon, inline_group,
    label, list, location_picker, mindmap_style_panel, modal_frame, popup_menu, scrollbar,
    search_bar, settings_view, sidebar, split_button, splitter, status_bar, status_state, switch,
    tab_bar, tag_editor, text_box, title_bar, title_bar_spacer, toc, tooltip, tree_list,
    virtual_card_list,
};

pub use gutter::RenderContext;
pub use settings::{Settings, ThemeMode, UiMetrics};
pub use theme::Theme;

// 骨架（Phase 1）
pub use core::{
    DismissPolicy, Dock, DockChild, DrawCmd, DrawList, Event, EventCtx, KeyCode, LayoutCtx,
    MouseButton, NoopMeasure, OverlayAction, OverlayInputPolicy, OverlayLayout, PaintCtx, Rect,
    Screen, Side, TextMeasure, Widget, WidgetId,
};
