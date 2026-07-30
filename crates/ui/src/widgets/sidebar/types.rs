//! Sidebar component — pure type definitions (config, input, actions, enums).
//!
//! State machine logic lives in `state.rs`; layout types in `layout.rs`.

use crate::tab_bar::TabInfo;
use crate::view_mode::ViewMode;
use crate::widgets::popup_menu::ContextMenuAction;
use serde::{Deserialize, Serialize};

// ── Configuration (persisted per-workspace) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidebarConfig {
    pub pinned: bool,
    pub width: f32,
}

impl SidebarConfig {
    pub fn new_default(dpi_scale: f32) -> Self {
        Self { pinned: true, width: 220.0 * dpi_scale }
    }

    pub fn clamp_width(&mut self, dpi_scale: f32) {
        let lo = 160.0 * dpi_scale;
        let hi = 400.0 * dpi_scale;
        self.width = self.width.clamp(lo, hi);
    }
}

// ── Visibility state machine ──

#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub enum Visibility {
    Hidden,
    HoverPeek,
    HoverPeekFadingOut,
    #[default]
    Pinned,
}

// ── Input / Output types ──

pub(crate) struct SidebarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: Option<usize>,
    pub screen_w: f32,
    pub screen_h: f32,
    pub traffic_light_inset: (f32, f32), // (left, top) — 阶段 5 才会非零
    pub content_top: f32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SidebarKey {
    TogglePin,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewDocumentKind {
    Text,
    Mindmap,
    Markdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SidebarAction {
    SwitchTab(usize),
    CloseTab(usize),
    NewDocument(NewDocumentKind),
    OpenNewDocumentMenu,
    OpenDocument,
    OpenSettingsMenu,
    ToggleViewMode,
    TogglePin,
    SetWidth(f32),
    Context { action: ContextMenuAction, tab_index: usize },
    PersistConfig,
    SetViewMode(ViewMode),
    OpenSettingsFile,
    StartResize,
    ResizeTo(f32),
    EndResize,
    Hovered,
    ContextMenuPx { tab_index: usize, anchor_px: (f32, f32), screen_size: (f32, f32) },
    ToggleLineNumbers,
    ToggleWordWrap,
    ToggleStatusBar,
    SetThemeMode(crate::settings::ThemeMode),
}

// ── Hover buttons enum ──

#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub enum SidebarHoverButton {
    #[default]
    None,
    Hamburger,
    NewDoc,
    OpenFile,
    Settings,
}

/// Hot zone width (logical pixels) for auto-hide hover trigger at left screen edge.
pub(crate) const HOT_BAND_LOGICAL: f32 = 10.0;

// ── SidebarSettingsInput (behavior-only fields) ──

/// Behavior-only input for the settings menu.
///
/// Carries toggles/modes that the user can change via the menu.
/// Layout/DPI information comes from [`crate::settings::UiMetrics`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarSettingsInput {
    pub show_line_numbers: bool,
    pub word_wrap: bool,
    pub show_status_bar: bool,
    pub theme_mode: crate::settings::ThemeMode,
    pub view_mode: crate::view_mode::ViewMode,
}

impl From<&crate::settings::Settings> for SidebarSettingsInput {
    fn from(settings: &crate::settings::Settings) -> Self {
        Self {
            show_line_numbers: settings.show_line_numbers,
            word_wrap: settings.word_wrap,
            show_status_bar: settings.show_status_bar,
            theme_mode: settings.theme_mode,
            view_mode: settings.view_mode,
        }
    }
}

impl Default for SidebarSettingsInput {
    fn default() -> Self {
        Self {
            show_line_numbers: true,
            word_wrap: true,
            show_status_bar: false,
            theme_mode: crate::settings::ThemeMode::default(),
            view_mode: crate::view_mode::ViewMode::default(),
        }
    }
}

// ── Owned widget input (replaces long parameter list) ──

/// Owned input bundle for [`SidebarWidget::set_input`].
///
/// Collects all per-frame sidebar data into a single struct so the caller
/// does not need to remember positional parameter order.
#[derive(Debug, Clone)]
pub struct SidebarWidgetInput {
    pub tabs: Vec<TabInfo>,
    pub active_index: Option<usize>,
    pub traffic_light_inset_px: (f32, f32),
    pub screen_size_px: (f32, f32),
    pub metrics: crate::settings::UiMetrics,
    pub settings: SidebarSettingsInput,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_settings_input_copies_behavior_only() {
        let mut settings = crate::settings::Settings::new();
        settings.show_line_numbers = false;
        settings.word_wrap = false;
        settings.show_status_bar = true;
        settings.theme_mode = crate::settings::ThemeMode::Dark;
        settings.view_mode = crate::view_mode::ViewMode::Tabs;

        let input = SidebarSettingsInput::from(&settings);

        assert!(!input.show_line_numbers);
        assert!(!input.word_wrap);
        assert!(input.show_status_bar);
        assert_eq!(input.theme_mode, crate::settings::ThemeMode::Dark);
        assert_eq!(input.view_mode, crate::view_mode::ViewMode::Tabs);
    }
}
