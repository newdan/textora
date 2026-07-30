//! Sidebar layout types.

use crate::core::Rect;
use crate::tab_bar::TabIndicator;

#[derive(Debug, Clone)]
pub struct SidebarLayoutItem {
    pub tab_index: usize,
    pub rect: Rect,
    pub title: String,
    pub indicator: TabIndicator,
}

#[derive(Debug, Clone, Default)]
pub struct SidebarLayout {
    pub bg_rect: Rect,
    pub header_rect: Rect,
    pub menu_btn_rect: Rect,
    pub new_btn_rect: Rect,
    pub new_menu_btn_rect: Rect,
    pub open_btn_rect: Rect,
    pub items: Vec<SidebarLayoutItem>,
    pub files_header_rect: Rect,
    pub list_clip: Rect,
    pub settings_btn_rect: Rect,
    pub edge_resize_rect: Rect,
}
