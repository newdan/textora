//! 每帧渲染缓存：cluster pool、首尾行缓存。

/// 单个可视行的缓存数据（首行或末行）。
#[derive(Clone)]
pub struct LineCache {
    pub visual_lines: Vec<(usize, usize, f32)>,
    pub clusters: Vec<(usize, usize, f32)>,
    pub doc_offset: usize,
}

impl LineCache {
    pub fn empty() -> Self {
        Self { visual_lines: Vec::new(), clusters: Vec::new(), doc_offset: 0 }
    }
}

/// 每帧重建的渲染缓存，生命周期与一帧绑定。
#[derive(Clone)]
pub struct FrameCache {
    pub cluster_pool: Vec<Vec<(usize, f32, u32)>>,
    pub first_line: LineCache,
    pub last_line: LineCache,
}

impl FrameCache {
    pub fn new() -> Self {
        Self {
            cluster_pool: Vec::new(),
            first_line: LineCache::empty(),
            last_line: LineCache::empty(),
        }
    }
}

impl Default for FrameCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::FrameCache;

    fn assert_cache_is_empty(cache: &FrameCache) {
        assert!(cache.cluster_pool.is_empty());
        assert!(cache.first_line.visual_lines.is_empty());
        assert!(cache.first_line.clusters.is_empty());
        assert!(cache.last_line.visual_lines.is_empty());
        assert!(cache.last_line.clusters.is_empty());
    }

    #[test]
    fn new_frame_cache_starts_without_line_or_cluster_data() {
        let cache = FrameCache::new();

        assert_cache_is_empty(&cache);
    }

    #[test]
    fn default_frame_cache_matches_new_state() {
        let default_cache = FrameCache::default();
        let new_cache = FrameCache::new();

        assert_eq!(default_cache.cluster_pool, new_cache.cluster_pool);
        assert_eq!(default_cache.first_line.visual_lines, new_cache.first_line.visual_lines);
        assert_eq!(default_cache.first_line.clusters, new_cache.first_line.clusters);
        assert_eq!(default_cache.first_line.doc_offset, new_cache.first_line.doc_offset);
        assert_eq!(default_cache.last_line.visual_lines, new_cache.last_line.visual_lines);
        assert_eq!(default_cache.last_line.clusters, new_cache.last_line.clusters);
        assert_eq!(default_cache.last_line.doc_offset, new_cache.last_line.doc_offset);
    }
}
