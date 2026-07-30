use appkit_core::workspace::types::TabId;
use core::types::UniCharOffset;
use ui::canvas::{CanvasAxis, CanvasPoint};
use winit::event::{ElementState, MouseScrollDelta};
use winit::window::CursorIcon;

use crate::menu_handler::AppCommand;
use crate::sync_settings_types::SyncSettingsAction;
use ui::popup_menu::{ContextMenuAction, PopupMenu};
use ui::scrollbar::ScrollbarAction;
use ui::search_bar::SearchBarAction;
use ui::view_mode::ViewMode;

/// AppAction represents a pure intent to modify the application state.
/// It is the result of processing user input events.
pub(crate) enum AppAction {
    // ------------------------------------------------------------------------
    /// Request a full application redraw on the next frame.
    RequestRedraw,
    /// Update the window cursor icon.
    SetCursor(CursorIcon),

    // ------------------------------------------------------------------------
    // Commands and Menus
    // ------------------------------------------------------------------------
    /// Execute one or more application commands (like open, save, zoom, close tab).
    ExecuteAppCommands(Vec<AppCommand>),
    /// Forward key to active plugin. If not handled, execute fallback command.
    /// Open the context menu for a specific tab.
    OpenPopupMenu(PopupMenu),
    /// Execute an action selected from the context menu.
    ExecuteContextMenuAction(ContextMenuAction, TabId),
    /// Open the tab overflow dropdown menu.
    OpenPopupOverflow,
    /// Dismiss the tab overflow dropdown menu.
    ClearPopupMenu,
    /// Dismiss the active modal overlay and restore the previous focus.
    DismissOverlay,
    /// Apply a validated action emitted by the SettingsView widget.
    Settings(ui::settings_view::SettingsViewAction),
    /// Apply an action emitted by Textora's product-owned Sync settings page.
    Sync(SyncSettingsAction),

    // ------------------------------------------------------------------------
    // Mouse and scroll inputs
    // ------------------------------------------------------------------------
    /// Update the tracked mouse physical position.
    UpdateMousePos(f64, f64),
    /// Handle mouse wheel/trackpad scrolling.
    HandleScroll(MouseScrollDelta),
    /// Handle a mouse input (click/release) targeting the editor area.
    EditorMouseInput {
        state: ElementState,
        px: f32,
        py: f32,
        /// Pre-computed hit test result containing (unichar_offset, doc_line_index, vis_line_index).
        hit: Option<(UniCharOffset, usize, usize)>,
    },
    /// Handle cursor drag movement targeting the editor area.
    EditorCursorMoved { px: f32, py: f32, hit: Option<(UniCharOffset, usize, usize)> },

    // ------------------------------------------------------------------------
    // Tab interactions
    // ------------------------------------------------------------------------
    /// Switch to the tab with the given ID.
    SwitchTab(TabId),
    /// Close the tab with the given ID.
    CloseTab(TabId),
    /// Open a new empty tab.
    NewEmptyTab,
    /// Create an untitled document with a type-specific name and view plugin.
    NewDocument(ui::sidebar::NewDocumentKind),
    /// Toggle the pinned state of the active tab.
    TogglePin,
    /// Scroll the tab bar left.
    ScrollTabLeft,
    /// Scroll the tab bar right.
    ScrollTabRight,
    /// Update the currently hovered tab ID.
    HoverTab(Option<TabId>),

    // ------------------------------------------------------------------------
    // Scrollbar interactions
    // ------------------------------------------------------------------------
    /// Handle a structural action on the scrollbar (PageUp, PageDown, StartDrag).
    #[allow(dead_code)]
    ScrollbarAction(ScrollbarAction),
    /// Update the viewport's top scroll offset (e.g. from drag).
    UpdateScrollTop(f64),
    /// Route an overlay canvas scrollbar action to the active canvas viewport.
    CanvasScrollbar { axis: CanvasAxis, action: ScrollbarAction },
    /// Zoom the active canvas viewport around a screen-space pinch anchor.
    CanvasPinch { delta: f64, screen_anchor: CanvasPoint },

    // ------------------------------------------------------------------------
    // Viewport and cursor
    // ------------------------------------------------------------------------
    /// Request that the viewport scroll by a relative pixel amount.
    ScrollViewportBy(f64),

    // ------------------------------------------------------------------------
    // View mode
    // ------------------------------------------------------------------------
    /// Switch between Sidebar and Tabs view mode.
    SetViewMode(ViewMode),
    /// Open the settings.toml file in a new tab.
    OpenSettingsFile,
    /// Toggle line number display.
    ToggleLineNumbers,
    /// Toggle word wrap.
    ToggleWordWrap,
    /// Toggle status bar visibility.
    ToggleStatusBar,
    /// Set theme mode (System / Dark / Light).
    SetThemeMode(ui::settings::ThemeMode),
    /// Toggle the active mmap tab's style panel.
    ToggleMindmapStylePanel,
    /// Handle an action emitted by the mmap style panel.
    MindmapStylePanel(ui::core::widget::MindmapStylePanelAction),

    // ------------------------------------------------------------------------
    // Sidebar interactions (Phase 7 widget path)
    // ------------------------------------------------------------------------
    /// Start sidebar edge resize drag.
    SidebarResizeStart,
    /// End sidebar edge resize drag.
    SidebarResizeEnd,
    /// Set sidebar width (px), write back to workspace.sidebar_cfg.
    SetSidebarWidth(f32),
    /// Open the sidebar settings popup menu.
    OpenSidebarSettingsMenu,
    /// Toggle sidebar pinned state (hamburger button).
    ToggleSidebarPin,

    // ------------------------------------------------------------------------
    // Search bar interactions
    // ------------------------------------------------------------------------
    /// Handle a search bar action (from mouse clicks on buttons).
    SearchBarAction(SearchBarAction),

    // ------------------------------------------------------------------------
    // TOC interactions
    // ------------------------------------------------------------------------
    /// Jump to a heading in the markdown preview.
    JumpToHeading(usize),
}

#[cfg(test)]
mod tests {
    #[test]
    fn standalone_sync_app_actions_are_removed() {
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"));
        assert!(!source.contains(concat!("Open", "Sync", "Panel")));
        assert!(!source.contains(concat!("Sync", "Panel(")));
    }
}
