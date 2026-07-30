//! tab_bar/types.rs — 共享类型定义。
//! 所有子模块统一 `use super::types::*`，不依赖 mod.rs。

use std::path::PathBuf;

/// Pure-data input for a single tab — no DocumentView dependency.
#[derive(Debug, Clone)]
pub struct TabInfo {
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub is_dirty: bool,
    pub pinned: bool,
    pub language: String,
}

/// Compute tab bar height from font size.
pub fn tab_bar_height(dpi_scale: f32) -> f32 {
    32.0 * dpi_scale
}

/// Rendering context for the tab bar.
pub struct TabBarCtx {
    pub screen_w: f32,
    pub screen_h: f32,
    pub dpi: f32,
}

#[cfg(test)]
mod tests {
    use super::TabInfo;

    fn assert_debug_clone<T: std::fmt::Debug + Clone>() {}

    #[test]
    fn tab_info_supports_owned_input_diagnostics() {
        assert_debug_clone::<TabInfo>();
    }
}
