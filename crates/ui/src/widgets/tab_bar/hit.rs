//! tab_bar/hit.rs — 点击测试。

use super::layout::TabBarLayout;
use super::layout::is_tab_in_clip;

/// Hit test: given a mouse position (x, y in physical pixels) and screen dimensions,
/// return the index of the tab clicked, or the close button clicked.
pub fn hit_test(x: f32, y: f32, layout: &TabBarLayout) -> Option<TabHit> {
    // Check overflow scroll arrows
    if !layout.left_arrow_disabled && layout.overflow_left_rect_px.contains(x, y) {
        return Some(TabHit::ScrollLeft);
    }
    if !layout.right_arrow_disabled && layout.overflow_right_rect_px.contains(x, y) {
        return Some(TabHit::ScrollRight);
    }

    // Check dropdown "all tabs" button
    if layout.dropdown_rect_px.contains(x, y) {
        return Some(TabHit::Dropdown);
    }

    // Check "+" button
    if layout.new_tab_rect_px.contains(x, y) {
        return Some(TabHit::NewTab);
    }

    for entry in &layout.tabs {
        if entry.rect_px.contains(x, y) {
            // Non-pinned tabs must be within clip bounds to be clickable
            if !entry.pinned {
                if !is_tab_in_clip(x, layout) {
                    continue;
                }
                if entry.close_rect_px.contains(x, y) {
                    return Some(TabHit::Close(entry.index));
                }
            }
            return Some(TabHit::Tab(entry.index));
        }
    }
    None
}

/// Result of tab bar hit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    Close(usize),
    NewTab,
    ScrollLeft,
    ScrollRight,
    /// Open the dropdown menu listing all open tabs
    Dropdown,
}
