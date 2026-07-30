//! 行级渲染缓存。存储行内相对坐标的 GlyphInstance，主题切换零失效。
//!
//! 容量 bound = viewport_visual_rows + 2 × OVERSCAN（约 1000 行）。
//! 颜色在渲染时从主题查询，滚动时只加 y_offset ≈ 零计算。

use hashlink::LruCache;

/// 行内相对坐标的字形实例（避免绝对屏幕坐标）。
/// 包含足够信息在渲染时重建 NDC 顶点（无需 atlas 查询或 shape）。
#[derive(Clone, Debug)]
pub struct GlyphInstance {
    /// 行内 x 偏移（像素，相对于行左边缘 = pen position）。
    pub x: f32,
    /// 行内 y 偏移（像素，相对于行 baseline = bearing 调整后）。
    pub y: f32,
    /// 笔位置到字形左边缘的水平偏移。
    pub bearing_x: f32,
    /// Baseline 到字形顶部的垂直偏移。
    pub bearing_y: f32,
    /// 字形宽度（像素）。
    pub width: f32,
    /// 字形高度（像素）。
    pub height: f32,
    /// Atlas UV 坐标（u 左, v 上, u 右, v 下）。
    pub uv: [f32; 4],
    /// 所属 atlas page。
    pub atlas_page: u32,
    pub highlight_kind: u8,
}

/// 缓存的单行渲染数据。
#[derive(Clone)]
pub struct CachedLine {
    /// 该行的所有字形实例（行内相对坐标）。
    pub instances: Vec<GlyphInstance>,
    /// 行号字形实例（可选）。
    pub line_number_glyphs: Vec<GlyphInstance>,
    /// 缓存时的 atlas generation。如果 atlas 被驱逐需 invalidate。
    pub atlas_generation: u64,
    /// 该行的视觉行数。
    pub visual_line_count: u16,
    /// 内容 hash（用于失效检测）。
    pub content_hash: u64,
    /// 缓存的 wrap 结果：(vl_start, vl_end, vl_width)，每个视觉行在 instances 中的索引范围。
    pub visual_lines: Vec<(usize, usize, f32)>,
    /// 每视觉行在 instances 中的起始索引。
    pub visual_line_instance_starts: Vec<usize>,
    /// 缓存的 cluster 数据：(byte_start, byte_end, advance)，用于重建 advance_cache。
    pub cluster_data: Vec<(usize, usize, f32)>,
    /// The starting visual line index of this cache (0 for full lines, skip_visual for subsets).
    pub subset_start: usize,
}

/// 行级渲染缓存。
pub struct RenderCache {
    /// LRU 缓存：doc_line → CachedLine。
    cache: LruCache<usize, CachedLine>,
    /// 每行典型内存占用（字节），用于估算总内存。
    estimated_bytes: usize,
}

#[allow(dead_code)]
const OVERSCAN_ROWS: usize = 500;
const MAX_CACHED_LINES: usize = 1000; // viewport ~40 + 2*500 overscan

/// Convert a stored highlight kind (u8) to an RGBA color using the current theme.
fn highlight_kind_to_color(kind: u8, theme: &ui::theme::Theme) -> [f32; 4] {
    use core::highlight::{HighlightKind, highlight_kind_scope};
    if let Ok(hk) = HighlightKind::try_from(kind as u32) {
        theme.scope_color(highlight_kind_scope(hk))
    } else {
        theme.editor.foreground
    }
}

impl CachedLine {
    /// 生成指定视觉行的 NDC 顶点（6 顶点/字形）。
    /// 调用方提供：line_y（该视觉行在屏幕上的 y 像素位置）、screen_w、screen_h、color。
    pub fn emit_vertices_for_visual_line(
        &self,
        vl_idx: usize,
        line_y: f32,
        line_height: f32,
        tab_bar_height: f32,
        screen_w: f32,
        screen_h: f32,
        color: [f32; 4],
        theme: &ui::theme::Theme,
        // IME preedit shift: `(threshold_x, shift_px)`. Instances with `x >= threshold_x`
        // are shifted right by `shift_px`. Pass `None` when no IME preedit is active.
        preedit_shift: Option<(f32, f32)>,
    ) -> Vec<render::GlyphVertex> {
        if vl_idx >= self.visual_line_instance_starts.len() {
            return Vec::new();
        }
        let start = self.visual_line_instance_starts[vl_idx];
        let end = if vl_idx + 1 < self.visual_line_instance_starts.len() {
            self.visual_line_instance_starts[vl_idx + 1]
        } else {
            self.instances.len()
        };
        let y_base = line_y + line_height * ui::constants::BASELINE_RATIO + tab_bar_height;

        let mut vertices = Vec::with_capacity((end - start) * 6);
        for inst in &self.instances[start..end] {
            let c = if inst.highlight_kind != 0 {
                highlight_kind_to_color(inst.highlight_kind, theme)
            } else {
                color
            };
            let inst_x = match preedit_shift {
                Some((threshold, shift)) if inst.x >= threshold => inst.x + shift,
                _ => inst.x,
            };
            let px = (inst_x + inst.bearing_x).round();
            let py = (y_base - inst.bearing_y).round();
            let left = px / screen_w * 2.0 - 1.0;
            let top = 1.0 - py / screen_h * 2.0;
            let right = (px + inst.width) / screen_w * 2.0 - 1.0;
            let bottom = 1.0 - (py + inst.height) / screen_h * 2.0;
            let uv = inst.uv;

            vertices.push(render::GlyphVertex {
                position: [left, top],
                tex_coords: [uv[0], uv[1]],
                color: c,
            });
            vertices.push(render::GlyphVertex {
                position: [right, top],
                tex_coords: [uv[2], uv[1]],
                color: c,
            });
            vertices.push(render::GlyphVertex {
                position: [left, bottom],
                tex_coords: [uv[0], uv[3]],
                color: c,
            });
            vertices.push(render::GlyphVertex {
                position: [right, top],
                tex_coords: [uv[2], uv[1]],
                color: c,
            });
            vertices.push(render::GlyphVertex {
                position: [right, bottom],
                tex_coords: [uv[2], uv[3]],
                color: c,
            });
            vertices.push(render::GlyphVertex {
                position: [left, bottom],
                tex_coords: [uv[0], uv[3]],
                color: c,
            });
        }
        vertices
    }
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderCache {
    pub fn new() -> Self {
        Self { cache: LruCache::new(MAX_CACHED_LINES), estimated_bytes: 0 }
    }

    /// 获取缓存的行数据（提升到 MRU）。
    pub fn get(&self, doc_line: usize) -> Option<&CachedLine> {
        self.cache.peek(&doc_line)
    }

    /// 检查缓存中是否存在指定行。
    pub fn contains(&self, doc_line: usize) -> bool {
        self.cache.contains_key(&doc_line)
    }

    /// 插入缓存行。
    pub fn insert(&mut self, doc_line: usize, line: CachedLine) {
        let entry_bytes = line.instances.len() * std::mem::size_of::<GlyphInstance>()
            + line.line_number_glyphs.len() * std::mem::size_of::<GlyphInstance>();
        self.estimated_bytes += entry_bytes;
        if let Some(old) = self.cache.insert(doc_line, line) {
            self.estimated_bytes -= old.instances.len() * std::mem::size_of::<GlyphInstance>()
                + old.line_number_glyphs.len() * std::mem::size_of::<GlyphInstance>();
        }
    }

    /// 使指定 doc_line 的缓存失效。
    pub fn invalidate(&mut self, doc_line: usize) {
        if let Some(old) = self.cache.remove(&doc_line) {
            self.estimated_bytes -= old.instances.len() * std::mem::size_of::<GlyphInstance>()
                + old.line_number_glyphs.len() * std::mem::size_of::<GlyphInstance>();
        }
    }

    /// 使所有缓存失效（如 resize 时）。
    pub fn invalidate_all(&mut self) {
        self.cache.clear();
        self.estimated_bytes = 0;
    }

    /// 使 atlas generation 不匹配的条目失效（atlas 驱逐后调用）。
    pub fn invalidate_stale_atlas(&mut self, current_generation: u64) {
        let stale: Vec<usize> = self
            .cache
            .iter()
            .filter(|(_, v)| v.atlas_generation != current_generation)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            self.invalidate(k);
        }
    }

    /// 使受影响的 doc_line 范围失效。
    pub fn invalidate_range(&mut self, range: std::ops::Range<usize>) {
        let keys: Vec<usize> = self.cache.iter().map(|(k, _)| *k).collect();
        for k in keys {
            if range.contains(&k) {
                self.invalidate(k);
            }
        }
    }

    /// 估算内存使用（字节）。
    pub fn estimated_memory(&self) -> usize {
        self.estimated_bytes
    }

    /// 缓存条目数。
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// 预览专用缓存：key = UiTextLayout.id (u64)，layout 时分配，跨帧稳定
pub struct PreviewRenderCache {
    cache: LruCache<u64, CachedLine>,
}

impl Default for PreviewRenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewRenderCache {
    pub fn new() -> Self {
        Self { cache: LruCache::new(MAX_CACHED_LINES) }
    }

    pub fn get(&mut self, key: u64) -> Option<&CachedLine> {
        self.cache.get(&key)
    }

    pub fn insert(&mut self, key: u64, line: CachedLine) {
        self.cache.insert(key, line);
    }

    pub fn invalidate_stale_atlas(&mut self, current_generation: u64) {
        let stale: Vec<u64> = self
            .cache
            .iter()
            .filter(|(_, v)| v.atlas_generation != current_generation)
            .map(|(k, _)| *k)
            .collect();
        for k in stale {
            self.cache.remove(&k);
        }
    }

    pub fn invalidate_all(&mut self) {
        self.cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_instance(x: f32) -> GlyphInstance {
        GlyphInstance {
            x,
            y: 0.0,
            bearing_x: 0.0,
            bearing_y: 0.0,
            width: 10.0,
            height: 14.0,
            uv: [0.0, 0.0, 0.1, 0.1],
            atlas_page: 0,
            highlight_kind: 0,
        }
    }

    fn make_line(generation: u64, count: usize) -> CachedLine {
        CachedLine {
            instances: (0..count).map(|i| make_instance(i as f32 * 10.0)).collect(),
            line_number_glyphs: vec![],
            atlas_generation: generation,
            visual_line_count: 1,
            content_hash: 42,
            visual_lines: vec![(0, count, count as f32 * 10.0)],
            visual_line_instance_starts: vec![0],
            cluster_data: (0..count).map(|i| (i, i + 1, 10.0)).collect(),
            subset_start: 0,
        }
    }

    #[test]
    fn insert_and_get() {
        let mut rc = RenderCache::new();
        rc.insert(0, make_line(1, 5));
        assert!(rc.get(0).is_some());
        assert_eq!(rc.get(0).unwrap().instances.len(), 5);
    }

    #[test]
    fn invalidate_single_line() {
        let mut rc = RenderCache::new();
        rc.insert(0, make_line(1, 3));
        rc.insert(1, make_line(1, 4));
        rc.invalidate(0);
        assert!(rc.get(0).is_none());
        assert!(rc.get(1).is_some());
    }

    #[test]
    fn invalidate_all_clears_everything() {
        let mut rc = RenderCache::new();
        rc.insert(0, make_line(1, 3));
        rc.insert(1, make_line(1, 4));
        rc.invalidate_all();
        assert!(rc.is_empty());
    }

    #[test]
    fn invalidate_range() {
        let mut rc = RenderCache::new();
        for i in 0..10 {
            rc.insert(i, make_line(1, 2));
        }
        rc.invalidate_range(3..7);
        assert!(rc.get(2).is_some());
        assert!(rc.get(3).is_none());
        assert!(rc.get(6).is_none());
        assert!(rc.get(7).is_some());
    }

    #[test]
    fn invalidate_range_out_of_bounds() {
        let mut rc = RenderCache::new();
        for i in 0..5 {
            rc.insert(i, make_line(1, 2));
        }
        // Range completely out of bounds should not panic
        rc.invalidate_range(10..20);
        assert_eq!(rc.len(), 5);

        // Range overlapping bounds should invalidate correctly
        rc.invalidate_range(3..10);
        assert!(rc.get(2).is_some());
        assert!(rc.get(3).is_none());
        assert!(rc.get(4).is_none());
        assert_eq!(rc.len(), 3);
    }

    #[test]
    fn stale_atlas_invalidation() {
        let mut rc = RenderCache::new();
        rc.insert(0, make_line(1, 3));
        rc.insert(1, make_line(5, 3)); // old generation
        rc.invalidate_stale_atlas(3);
        assert!(rc.get(0).is_none()); // gen 1 ≠ 3
        assert!(rc.get(1).is_none()); // gen 5 ≠ 3
    }

    #[test]
    fn lru_eviction() {
        let mut rc = RenderCache::new();
        // Insert more than MAX_CACHED_LINES
        for i in 0..2000 {
            rc.insert(i, make_line(i as u64, 1));
        }
        // Should have evicted some
        assert!(rc.len() <= MAX_CACHED_LINES);
        // Oldest should be gone
        assert!(rc.get(0).is_none());
        // Newest should stay
        assert!(rc.get(1999).is_some());
    }

    #[test]
    fn memory_estimation_grows_and_shrinks() {
        let mut rc = RenderCache::new();
        rc.insert(0, make_line(1, 100));
        let m1 = rc.estimated_memory();
        assert!(m1 > 0);
        rc.invalidate(0);
        assert_eq!(rc.estimated_memory(), 0);
    }

    #[test]
    fn emit_vertices_generates_6_per_glyph() {
        let line = make_line(1, 3);
        let verts = line.emit_vertices_for_visual_line(
            0,
            100.0,
            14.0,
            0.0,
            800.0,
            600.0,
            [1.0, 1.0, 1.0, 1.0],
            &ui::theme::test_theme(),
            None,
        );
        // 3 glyphs × 6 vertices = 18
        assert_eq!(verts.len(), 18);
    }

    #[test]
    fn emit_vertices_empty_for_out_of_range() {
        let line = make_line(1, 3);
        let verts = line.emit_vertices_for_visual_line(
            99,
            100.0,
            14.0,
            0.0,
            800.0,
            600.0,
            [1.0, 1.0, 1.0, 1.0],
            &ui::theme::test_theme(),
            None,
        );
        assert!(verts.is_empty());
    }

    // ===== PreviewRenderCache tests =====

    #[test]
    fn preview_cache_insert_and_get() {
        let mut pc = PreviewRenderCache::new();
        let line = make_line(1, 3);
        pc.insert(42, line);
        assert!(pc.get(42).is_some());
        assert_eq!(pc.get(42).unwrap().instances.len(), 3);
    }

    #[test]
    fn preview_cache_miss() {
        let mut pc = PreviewRenderCache::new();
        assert!(pc.get(999).is_none());
    }

    #[test]
    fn preview_cache_invalidate_stale_atlas() {
        let mut pc = PreviewRenderCache::new();
        pc.insert(1, make_line(1, 2));
        pc.insert(2, make_line(5, 2));
        pc.invalidate_stale_atlas(3);
        assert!(pc.get(1).is_none());
        assert!(pc.get(2).is_none());
    }

    #[test]
    fn preview_cache_invalidate_all() {
        let mut pc = PreviewRenderCache::new();
        pc.insert(1, make_line(1, 2));
        pc.insert(2, make_line(2, 2));
        pc.invalidate_all();
        assert!(pc.get(1).is_none());
        assert!(pc.get(2).is_none());
    }

    #[test]
    fn preview_cache_lru_eviction() {
        let mut pc = PreviewRenderCache::new();
        for i in 0..1100u64 {
            pc.insert(i, make_line(i, 1));
        }
        assert!(pc.get(0).is_none());
        assert!(pc.get(1099).is_some());
    }

    #[test]
    fn preview_cache_overwrite() {
        let mut pc = PreviewRenderCache::new();
        pc.insert(1, make_line(1, 2));
        pc.insert(1, make_line(1, 5));
        assert_eq!(pc.get(1).unwrap().instances.len(), 5);
    }
}
