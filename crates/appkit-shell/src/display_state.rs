//! 显示状态：视口、换行映射、渲染缓存。
//! 从 DocumentView 提取，统一管理显示相关状态。

use crate::display_line_map::DisplayLineMap;
use crate::render_cache::RenderCache;
use ui::viewport::Viewport;

/// 文档视图的显示状态（视口滚动 + 换行映射 + 渲染缓存）。

pub struct DisplayState {
    /// Viewport for scroll tracking.
    pub viewport: Viewport,
    /// Per-document display line map (wrap index).
    pub display_map: DisplayLineMap,
    /// Per-document render cache (glyph instances keyed by doc_line).
    pub render_cache: RenderCache,
    /// Cached grapheme clusters for hit-testing and cursor rendering.
    pub advance_cache: Vec<ui::render_geom::AdvanceCacheEntry>,
}

impl DisplayState {
    /// Create a new display state with default viewport size.
    pub fn new(visible_rows: usize, viewport_height: f64) -> Self {
        Self {
            viewport: Viewport {
                scroll_top: 0.0,
                visible_rows: visible_rows.max(1),
                viewport_height: viewport_height.max(1.0),
                scroll_anchor: ui::viewport::ScrollAnchor::top(),
            },
            display_map: DisplayLineMap::new(),
            render_cache: RenderCache::new(),
            advance_cache: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DisplayState;

    #[test]
    fn new_clamps_viewport_metrics_and_starts_with_empty_caches() {
        let state = DisplayState::new(0, 0.0);

        assert_eq!(state.viewport.visible_rows, 1);
        assert_eq!(state.viewport.viewport_height, 1.0);
        assert!(state.advance_cache.is_empty());
        assert_eq!(state.display_map.line_count(), 0);
        assert_eq!(state.render_cache.len(), 0);
    }
}
