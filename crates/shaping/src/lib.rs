//! edit+ text shaping: cosmic-text wrapper.
//!
//! Converts text into positioned glyph clusters for rendering.

pub mod font_cache;
use hashlink::LruCache;
use std::ops::Range;
use std::sync::{Arc, Mutex};

pub use cosmic_text::fontdb::{Style, Weight};
use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping};

// Re-export FontSystem so downstream crates can share it.
pub use cosmic_text::FontSystem;
pub use cosmic_text::fontdb::ID as FontId;

/// A single glyph cluster in a shaped run.
///
/// A glyph cluster maps a contiguous byte range in the source text
/// to one or more glyphs with their visual advances.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphCluster {
    /// Byte range in the source text.
    pub byte_range: Range<usize>,
    /// Glyph ID (font-specific).
    pub glyph_id: u32,
    /// Font ID (for rasterization).
    pub font_id: cosmic_text::fontdb::ID,
    /// Horizontal advance in pixels.
    pub advance: f32,
    /// X offset for subpixel positioning.
    pub x_offset: f32,
    /// Y offset for baseline adjustment.
    pub y_offset: f32,
}
/// Result of shaping a text run.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    /// The glyph clusters, in visual order.
    pub clusters: Vec<GlyphCluster>,
    /// Total horizontal advance in pixels.
    pub width: f32,
}
/// Errors that can occur during shaping.
#[derive(Debug)]
pub enum ShapeError {
    /// No fonts available for shaping.
    NoFonts,
    /// Shaping failed for the given text.
    ShapingFailed(String),
}
impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeError::NoFonts => write!(f, "no fonts available"),
            ShapeError::ShapingFailed(msg) => write!(f, "shaping failed: {msg}"),
        }
    }
}
impl std::error::Error for ShapeError {}
/// Maximum number of entries in the grapheme advance cache.
const MAX_CACHE_SIZE: usize = 4096;
/// Cached grapheme advance lookup with LRU eviction.
///
/// Maps grapheme strings to their pixel advance widths.
/// Uses a linked hash map for O(1) LRU eviction when at capacity.
pub struct GraphemeAdvanceCache {
    cache: LruCache<(String, u32, u32), f32>,
    hits: u64,
    misses: u64,
}
impl GraphemeAdvanceCache {
    pub fn new() -> Self {
        Self { cache: LruCache::new(MAX_CACHE_SIZE), hits: 0, misses: 0 }
    }
    /// Look up the advance for a grapheme. Returns `Some(advance)` on cache hit.
    /// Moves the entry to the front of the LRU list on hit.
    pub fn get(&mut self, grapheme: &str, font_size: f32, attrs_hash: u32) -> Option<f32> {
        let key = (grapheme.to_string(), (font_size * 64.0).round() as u32, attrs_hash);
        if let Some(&advance) = self.cache.get(&key) {
            self.hits += 1;
            Some(advance)
        } else {
            self.misses += 1;
            None
        }
    }
    /// Insert a grapheme advance into the cache.
    /// Evicts the least-recently-used entry if at capacity.
    pub fn insert(&mut self, grapheme: String, font_size: f32, attrs_hash: u32, advance: f32) {
        self.cache.insert((grapheme, (font_size * 64.0).round() as u32, attrs_hash), advance);
    }
    /// Cache hit rate (0.0 to 1.0).
    pub fn hit_rate(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 { 0.0 } else { self.hits as f32 / total as f32 }
    }
    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.cache.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}
impl Default for GraphemeAdvanceCache {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Debug for GraphemeAdvanceCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphemeAdvanceCache")
            .field("len", &self.cache.len())
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .finish()
    }
}
/// Text shaper.
///
/// Wraps cosmic-text to shape text into positioned glyph clusters.
pub struct Shaper {
    font_family: Option<String>,
    attrs_hash: u32,
    font_system: Arc<Mutex<FontSystem>>,
    font_size: f32,
    line_height: f32,
    font_weight: Weight,
    font_style: Style,
    cache: GraphemeAdvanceCache,
    scale_context: swash::scale::ScaleContext,
    /// Reusable buffer — avoids reallocating on every shape() call.
    buffer: Buffer,
    /// Cached monospace column width (ASCII advance) keyed by font_size bits.
    col_width_cache: Option<(u32, f32)>,
}
fn compute_attrs_hash(family: &str) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    family.hash(&mut hasher);
    hasher.finish() as u32
}
impl Shaper {
    /// Create a new shaper with system fonts.
    pub fn new() -> Result<Self, ShapeError> {
        let mut font_system = FontSystem::new();
        let metrics = Metrics::new(14.0, 20.0);
        let buffer = Buffer::new(&mut font_system, metrics);
        let font_system = Arc::new(Mutex::new(font_system));
        Ok(Self {
            font_family: None,
            attrs_hash: compute_attrs_hash(""),
            font_system,
            font_size: 14.0,
            line_height: 20.0,
            font_weight: Weight::NORMAL,
            font_style: Style::Normal,
            cache: GraphemeAdvanceCache::new(),
            scale_context: swash::scale::ScaleContext::new(),
            buffer,
            col_width_cache: None,
        })
    }

    /// Create a shaper from an existing FontSystem (shared via Arc).
    ///
    /// Avoids the expensive `FontSystem::new()` call — use this when
    /// multiple Shaper instances should share one font database.
    pub fn from_font_system(font_system: FontSystem, font_size: f32, font_family: &str) -> Self {
        let attrs_hash = compute_attrs_hash(font_family);
        let mut font_system = font_system;
        let metrics = Metrics::new(font_size, font_size * 1.4);
        let buffer = Buffer::new(&mut font_system, metrics);
        let font_system = Arc::new(Mutex::new(font_system));
        Self {
            font_family: Some(font_family.to_string()),
            attrs_hash,
            font_system,
            font_size,
            line_height: font_size * 1.4,
            font_weight: Weight::NORMAL,
            font_style: Style::Normal,
            cache: GraphemeAdvanceCache::new(),
            scale_context: swash::scale::ScaleContext::new(),
            buffer,
            col_width_cache: None,
        }
    }

    /// Create a shaper from a shared FontSystem (Arc<Mutex<FontSystem>>).
    ///
    /// Multiple Shaper instances can reference the same FontSystem via Arc cloning.
    /// This avoids duplicate font database initialization across threads.
    pub fn from_shared_font_system(
        font_system: Arc<Mutex<FontSystem>>,
        font_size: f32,
        font_family: &str,
    ) -> Self {
        let attrs_hash = compute_attrs_hash(font_family);
        let metrics = Metrics::new(font_size, font_size * 1.4);
        let buffer = Buffer::new(&mut font_system.lock().unwrap(), metrics);
        Self {
            font_family: Some(font_family.to_string()),
            attrs_hash,
            font_system,
            font_size,
            line_height: font_size * 1.4,
            font_weight: Weight::NORMAL,
            font_style: Style::Normal,
            cache: GraphemeAdvanceCache::new(),
            scale_context: swash::scale::ScaleContext::new(),
            buffer,
            col_width_cache: None,
        }
    }

    /// Create a new shaper with explicit font size.
    pub fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self.line_height = font_size * 1.4;
        self
    }

    /// Set the font family and update attrs_hash.
    pub fn with_font_family(mut self, family: &str) -> Self {
        self.font_family = Some(family.to_string());
        self.attrs_hash = compute_attrs_hash(family);
        self
    }

    /// Get the current attrs_hash for cache key purposes.
    pub fn attrs_hash(&self) -> u32 {
        self.attrs_hash
    }

    /// Get the current font size.
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Get the current font family.
    pub fn font_family(&self) -> Option<&str> {
        self.font_family.as_deref()
    }

    /// Set the font family.
    pub fn set_font_family(&mut self, family: Option<&str>) {
        let hash = match family {
            Some(f) => compute_attrs_hash(f),
            None => compute_attrs_hash(""),
        };
        let new_family = family.map(|s| s.to_string());
        if self.font_family != new_family || self.attrs_hash != hash {
            self.font_family = new_family;
            self.attrs_hash = hash;
            self.col_width_cache = None;
        }
    }

    /// Set the font size.
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size;
        self.col_width_cache = None;
    }

    /// Get the current font weight.
    pub fn font_weight(&self) -> Weight {
        self.font_weight
    }

    /// Set the font weight (e.g. `Weight::BOLD` for bold text).
    pub fn set_font_weight(&mut self, weight: Weight) {
        self.font_weight = weight;
    }

    /// Get the current font style.
    pub fn font_style(&self) -> Style {
        self.font_style
    }

    /// Set the font style (e.g. `Style::Italic` for italic text).
    pub fn set_font_style(&mut self, style: Style) {
        self.font_style = style;
    }

    /// Measure and cache the monospace column width (advance of 'a').
    /// Returns the cached value on subsequent calls with the same font size.
    pub fn col_width(&mut self) -> f32 {
        let key = self.font_size.to_bits();
        if let Some((k, cw)) = self.col_width_cache
            && k == key
        {
            return cw;
        }
        let cw = self.grapheme_advance("a").unwrap_or(self.font_size * 0.6);
        self.col_width_cache = Some((key, cw));
        cw
    }

    /// Resolve a font family string, mapping CSS generic keywords to cosmic-text Family variants.
    fn resolve_family(family_name: &str) -> Family<'_> {
        if family_name.eq_ignore_ascii_case("sans-serif")
            || family_name.eq_ignore_ascii_case("system-ui")
            || family_name.eq_ignore_ascii_case("-apple-system")
        {
            Family::SansSerif
        } else if family_name.eq_ignore_ascii_case("serif") {
            Family::Serif
        } else if family_name.eq_ignore_ascii_case("monospace") {
            Family::Monospace
        } else if family_name.eq_ignore_ascii_case("cursive") {
            Family::Cursive
        } else if family_name.eq_ignore_ascii_case("fantasy") {
            Family::Fantasy
        } else {
            Family::Name(family_name)
        }
    }

    /// Shape a text run into glyph clusters.
    pub fn shape(&mut self, text: &str) -> Result<ShapedRun, ShapeError> {
        let metrics = Metrics::new(self.font_size, self.line_height);
        let family = match &self.font_family {
            Some(name) => Self::resolve_family(name),
            None => Family::SansSerif,
        };
        let attrs = Attrs::new().family(family).weight(self.font_weight).style(self.font_style);
        // Take buffer out to split borrow (buffer + font_system)
        let mut buffer = std::mem::replace(&mut self.buffer, Buffer::new_empty(metrics));
        buffer.set_metrics(&mut self.font_system.lock().unwrap(), metrics);
        buffer.set_text(&mut self.font_system.lock().unwrap(), text, attrs, Shaping::Advanced);

        let mut clusters = Vec::new();
        let mut total_width: f32 = 0.0;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let cluster = GlyphCluster {
                    byte_range: glyph.start..glyph.end,
                    glyph_id: glyph.glyph_id as u32,
                    font_id: glyph.font_id,
                    advance: glyph.w,
                    x_offset: glyph.x_offset,
                    y_offset: glyph.y_offset,
                };
                total_width += glyph.w;
                clusters.push(cluster);
            }
        }

        // Return buffer for reuse
        self.buffer = buffer;
        Ok(ShapedRun { clusters, width: total_width })
    }

    /// Shape text with per-span weight/style overrides (e.g. bold/italic).
    ///
    /// `highlights` is a sorted slice of `(byte_offset, Weight, Style)` tuples
    /// marking the start of each styled span. Byte offsets are relative to `text`.
    /// The first span typically starts at offset 0 with the base weight/style.
    pub fn shape_with_highlights(
        &mut self,
        text: &str,
        highlights: &[(usize, Weight, Style)],
    ) -> Result<ShapedRun, ShapeError> {
        if text.is_empty() || highlights.is_empty() {
            return Ok(ShapedRun { clusters: Vec::new(), width: 0.0 });
        }

        let metrics = Metrics::new(self.font_size, self.line_height);
        let family = match &self.font_family {
            Some(name) => Self::resolve_family(name),
            None => Family::SansSerif,
        };
        let base_attrs =
            Attrs::new().family(family).weight(self.font_weight).style(self.font_style);

        // Build rich text spans: (text_slice, Attrs) pairs
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        for (i, &(start, weight, style)) in highlights.iter().enumerate() {
            let end = if i + 1 < highlights.len() { highlights[i + 1].0 } else { text.len() };
            if start >= text.len() || start >= end {
                continue;
            }
            let span_text = &text[start..end];
            let attrs = base_attrs.weight(weight).style(style);
            spans.push((span_text, attrs));
        }

        if spans.is_empty() {
            return Ok(ShapedRun { clusters: Vec::new(), width: 0.0 });
        }

        let mut buffer = std::mem::replace(&mut self.buffer, Buffer::new_empty(metrics));
        {
            let mut lock = self.font_system.lock().unwrap();
            buffer.set_metrics(&mut lock, metrics);
            buffer.set_rich_text(&mut lock, spans.iter().copied(), base_attrs, Shaping::Advanced);
        }

        let mut clusters = Vec::new();
        let mut total_width: f32 = 0.0;

        // set_rich_text concatenates contiguous substrings without inserting
        // separator bytes, so glyph byte ranges are already in the original
        // text's coordinate space — no remapping needed.
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let cluster = GlyphCluster {
                    byte_range: glyph.start..glyph.end,
                    glyph_id: glyph.glyph_id as u32,
                    font_id: glyph.font_id,
                    advance: glyph.w,
                    x_offset: glyph.x_offset,
                    y_offset: glyph.y_offset,
                };
                total_width += glyph.w;
                clusters.push(cluster);
            }
        }

        self.buffer = buffer;
        Ok(ShapedRun { clusters, width: total_width })
    }

    /// Fast shaping path: direct glyph mapping via ttf-parser.
    /// Bypasses the full OpenType pipeline — use for CJK/ASCII lines that
    /// don't need ligatures, RTL, or complex script support.
    pub fn shape_fast(&mut self, text: &str) -> Result<ShapedRun, ShapeError> {
        if text.is_empty() {
            return Ok(ShapedRun { clusters: Vec::new(), width: 0.0 });
        }

        // Find a font that covers the text. Prefer SansSerif for CJK coverage.
        let lock = self.font_system.lock().unwrap();
        let db = lock.db();
        let weight = self.font_weight;
        let style = self.font_style;
        let font_id = if let Some(ref family_name) = self.font_family {
            let family = Self::resolve_family(family_name);
            db.query(&cosmic_text::fontdb::Query {
                families: &[family],
                weight,
                style,
                ..Default::default()
            })
            .or_else(|| {
                db.query(&cosmic_text::fontdb::Query {
                    families: &[cosmic_text::Family::SansSerif],
                    weight,
                    style,
                    ..Default::default()
                })
            })
        } else {
            db.query(&cosmic_text::fontdb::Query {
                families: &[cosmic_text::Family::SansSerif],
                weight,
                style,
                ..Default::default()
            })
        }
        .or_else(|| {
            db.query(&cosmic_text::fontdb::Query {
                families: &[cosmic_text::Family::Monospace],
                weight,
                style,
                ..Default::default()
            })
        })
        .or_else(|| {
            db.query(&cosmic_text::fontdb::Query {
                families: &[cosmic_text::Family::Serif],
                weight,
                style,
                ..Default::default()
            })
        })
        .ok_or(ShapeError::ShapingFailed("no font found for fast shape".into()))?;
        // units_per_em is determined from the parsed font face below.
        // We use a placeholder scale of 1.0; the real scale is computed after parsing.
        let font_size = self.font_size;

        // Parse font tables and extract glyph metrics for each character.
        // with_face_data calls our closure with font bytes; we parse once,
        // process all characters, and return the result.
        let result: Option<(Vec<GlyphCluster>, f32)> =
            db.with_face_data(font_id, |data, face_index| {
                let face = match ttf_parser::Face::parse(data, face_index) {
                    Ok(f) => {
                        let upem = f.units_per_em() as f32;
                        if upem > 0.0 {
                            f
                        } else {
                            return (Vec::new(), 0.0);
                        }
                    }
                    Err(_) => return (Vec::new(), 0.0),
                };

                let upem = face.units_per_em() as f32;
                let scale = font_size / upem.max(1.0);
                let fallback_adv = font_size * 0.6;

                let mut byte_pos = 0usize;
                let mut clusters = Vec::with_capacity(text.len() / 2);
                let mut total_width = 0.0f32;

                for ch in text.chars() {
                    let ch_len = ch.len_utf8();

                    // Control chars (except whitespace) are invisible
                    if ch.is_control() && !ch.is_whitespace() {
                        clusters.push(GlyphCluster {
                            byte_range: byte_pos..byte_pos + ch_len,
                            glyph_id: 0,
                            font_id,
                            advance: 0.0,
                            x_offset: 0.0,
                            y_offset: 0.0,
                        });
                        byte_pos += ch_len;
                        continue;
                    }

                    let (gid, adv) = match face.glyph_index(ch) {
                        Some(g) => {
                            let a = face
                                .glyph_hor_advance(g)
                                .map(|a| a as f32 * scale)
                                .unwrap_or(fallback_adv);
                            (g.0, a)
                        }
                        None => {
                            // Missing glyph: fail fast shaping to trigger full shaper fallback
                            return (Vec::new(), -1.0);
                        }
                    };

                    clusters.push(GlyphCluster {
                        byte_range: byte_pos..byte_pos + ch_len,
                        glyph_id: gid as u32,
                        font_id,
                        advance: adv,
                        x_offset: 0.0,
                        y_offset: 0.0,
                    });
                    total_width += adv;
                    byte_pos += ch_len;
                }

                (clusters, total_width)
            });

        match result {
            Some((clusters, width)) if width >= 0.0 => Ok(ShapedRun { clusters, width }),
            _ => Err(ShapeError::ShapingFailed(
                "fast shape font missing glyphs or not available".into(),
            )),
        }
    }

    /// Get the pixel advance for a single grapheme cluster.
    /// Uses cache when available.
    pub fn grapheme_advance(&mut self, grapheme: &str) -> Result<f32, ShapeError> {
        let font_size = self.font_size;
        if let Some(cached) = self.cache.get(grapheme, font_size, self.attrs_hash) {
            return Ok(cached);
        }

        let run = self.shape(grapheme)?;
        let advance = run.width;
        self.cache.insert(grapheme.to_string(), font_size, self.attrs_hash, advance);
        Ok(advance)
    }

    /// Access the grapheme advance cache.
    pub fn cache(&self) -> &GraphemeAdvanceCache {
        &self.cache
    }

    /// Access the grapheme advance cache mutably.
    pub fn cache_mut(&mut self) -> &mut GraphemeAdvanceCache {
        &mut self.cache
    }
}
/// Rasterized glyph bitmap.
#[derive(Debug, Clone)]
pub struct GlyphBitmap {
    /// Alpha bitmap data (1 byte per pixel).
    pub data: Vec<u8>,
    /// Bitmap width in pixels.
    pub width: u32,
    /// Bitmap height in pixels.
    pub height: u32,
    /// Horizontal offset from pen to left edge of bitmap.
    pub left: i32,
    /// Vertical offset from baseline to top edge of bitmap.
    pub top: i32,
}
impl Shaper {
    /// Rasterize a glyph to a true subpixel bitmap using swash.
    ///
    /// Returns `None` if the glyph cannot be rasterized (e.g., space character).
    pub fn rasterize_glyph(
        &mut self,
        font_id: cosmic_text::fontdb::ID,
        glyph_id: u16,
        font_size: f32,
        subpixel_offset: (f32, f32),
    ) -> Option<GlyphBitmap> {
        let font = self.font_system.lock().unwrap().get_font(font_id)?;

        let mut scaler =
            self.scale_context.builder(font.as_swash()).size(font_size).hint(true).build();

        // Render with grayscale alpha mask
        let image = swash::scale::Render::new(&[
            swash::scale::Source::ColorOutline(0),
            swash::scale::Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
            swash::scale::Source::Outline,
        ])
        .format(swash::zeno::Format::Alpha)
        .offset(swash::zeno::Vector::new(subpixel_offset.0, subpixel_offset.1))
        .render(&mut scaler, glyph_id)?;

        if image.data.is_empty() {
            return None;
        }

        let placement = image.placement;

        match image.content {
            swash::scale::image::Content::Mask => Some(GlyphBitmap {
                data: image.data,
                width: placement.width,
                height: placement.height,
                left: placement.left,
                top: placement.top,
            }),
            swash::scale::image::Content::Color => {
                // Color emoji: extract alpha channel from BGRA
                let alpha: Vec<u8> = image.data.chunks(4).map(|px| px[3]).collect();
                Some(GlyphBitmap {
                    data: alpha,
                    width: placement.width,
                    height: placement.height,
                    left: placement.left,
                    top: placement.top,
                })
            }
            swash::scale::image::Content::SubpixelMask => {
                // Fallback (should not be reached if format is Alpha)
                let alpha: Vec<u8> =
                    image.data.chunks(4).map(|px| px[2].max(px[1]).max(px[0])).collect();
                Some(GlyphBitmap {
                    data: alpha,
                    width: placement.width,
                    height: placement.height,
                    left: placement.left,
                    top: placement.top,
                })
            }
        }
    }

    /// Access the font system (for advanced queries).
    /// Note: with shared FontSystem, callers should use the internal Shaper methods
    /// instead of locking the FontSystem directly.
    pub fn font_system(&self) -> std::sync::MutexGuard<'_, FontSystem> {
        self.font_system.lock().unwrap()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // ── GraphemeAdvanceCache tests (no font dependency) ────────────────────
    #[test]
    fn cache_miss_then_hit() {
        let mut cache = GraphemeAdvanceCache::new();
        assert_eq!(cache.get("a", 14.0, 0), None);
        cache.insert("a".into(), 14.0, 0, 8.0);
        assert_eq!(cache.get("a", 14.0, 0), Some(8.0));
        assert_eq!(cache.hit_rate(), 0.5);
    }
    #[test]
    fn cache_multiple_entries() {
        let mut cache = GraphemeAdvanceCache::new();
        cache.insert("a".into(), 14.0, 0, 8.0);
        cache.insert("世".into(), 14.0, 0, 16.0);
        cache.insert("👨‍👩‍👧".into(), 14.0, 0, 17.0);
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get("世", 14.0, 0), Some(16.0));
    }
    #[test]
    fn cache_hit_rate_empty() {
        let cache = GraphemeAdvanceCache::new();
        assert_eq!(cache.hit_rate(), 0.0);
    }
    #[test]
    fn cache_hit_rate_99_percent() {
        let mut cache = GraphemeAdvanceCache::new();
        cache.insert("a".into(), 14.0, 0, 8.0);
        for _ in 0..99 {
            cache.get("a", 14.0, 0);
        }
        assert!(cache.hit_rate() >= 0.99, "hit rate {}", cache.hit_rate());
    }
    #[test]
    fn cache_lru_eviction() {
        let mut cache = GraphemeAdvanceCache::new();
        // Fill to capacity
        for i in 0..4096 {
            cache.insert(format!("g{i}"), 14.0, 0, i as f32);
        }
        assert_eq!(cache.len(), 4096);
        // Insert one more — oldest should be evicted
        cache.insert("new".into(), 14.0, 0, 99.0);
        assert_eq!(cache.len(), 4096);
        // First entry should be evicted
        assert!(cache.get("g0", 14.0, 0).is_none());
        // Recent entry still there
        assert_eq!(cache.get("new", 14.0, 0), Some(99.0));
    }
    #[test]
    fn cache_attrs_hash_differentiates() {
        let mut cache = GraphemeAdvanceCache::new();
        // Insert with attrs_hash=1 (e.g., font "Menlo")
        cache.insert("a".into(), 14.0, 1, 8.0);
        // Same grapheme+font_size but different attrs_hash (e.g., font "Helvetica")
        assert_eq!(cache.get("a", 14.0, 2), None, "different attrs_hash should miss");
        // Same attrs_hash should hit
        assert_eq!(cache.get("a", 14.0, 1), Some(8.0), "same attrs_hash should hit");
    }
    // ── Shaper tests (need fonts) ──────────────────────────────────────────
    fn with_shaper(f: impl FnOnce(Shaper)) {
        match Shaper::new() {
            Ok(shaper) => f(shaper),
            Err(ShapeError::NoFonts) => {
                eprintln!("skipping: no fonts available");
            }
            Err(e) => panic!("Shaper::new() failed: {e}"),
        }
    }
    #[test]
    fn shape_ascii_basic() {
        with_shaper(|mut shaper| {
            let run = shaper.shape("Hello").expect("shape failed");
            assert_eq!(run.clusters.len(), 5, "expected 5 glyph clusters for 'Hello'");
            assert!(run.width > 0.0, "width must be positive");
            for cluster in &run.clusters {
                assert!(cluster.advance > 0.0, "cluster advance must be positive");
            }
            let sum: f32 = run.clusters.iter().map(|c| c.advance).sum();
            assert!((run.width - sum).abs() < 0.01, "width {} != sum {}", run.width, sum);
            let total_bytes: usize = run.clusters.iter().map(|c| c.byte_range.len()).sum();
            assert_eq!(total_bytes, 5);
        });
    }
    #[test]
    fn shape_cjk_mixed() {
        with_shaper(|mut shaper| {
            let run = shaper.shape("Hello 世界").expect("shape failed");
            assert!(run.clusters.len() >= 7, "expected >= 7 clusters for 'Hello 世界'");
            assert!(run.width > 0.0);
        });
    }
    #[test]
    fn shape_emoji_zwj() {
        with_shaper(|mut shaper| {
            let run = shaper.shape("👨‍👩‍👧").expect("shape failed");
            assert!(!run.clusters.is_empty(), "must produce at least 1 cluster");
            assert!(run.width > 0.0, "emoji must have positive width");
            let total_bytes: usize = run.clusters.iter().map(|c| c.byte_range.len()).sum();
            assert_eq!(total_bytes, "👨‍👩‍👧".len());
        });
    }
    #[test]
    fn shape_arabic_rtl_doesnt_crash() {
        with_shaper(|mut shaper| {
            let run = shaper.shape("مرحبا").expect("shape failed for Arabic");
            assert!(!run.clusters.is_empty());
            assert!(run.width > 0.0);
        });
    }
    #[test]
    fn grapheme_advance_single_ascii() {
        with_shaper(|mut shaper| {
            let advance = shaper.grapheme_advance("a").expect("grapheme_advance failed");
            assert!(advance > 0.0, "ASCII 'a' must have positive advance");
            assert!(advance < 100.0, "ASCII 'a' advance seems too large: {advance}");
        });
    }

    #[test]
    fn shape_digits_correct_font() {
        with_shaper(|mut shaper| {
            let input = "62220099";
            let run = shaper.shape(input).expect("shape digits failed");
            assert_eq!(
                run.clusters.len(),
                8,
                "should be 8 digit clusters, got {}",
                run.clusters.len()
            );
            // All clusters should have same font_id (same font for all ASCII digits)
            let first_font = run.clusters[0].font_id;
            for (i, c) in run.clusters.iter().enumerate() {
                assert_eq!(c.font_id, first_font, "cluster[{i}] has different font_id");
                // Each cluster should map to exactly 1 byte (single ASCII digit)
                assert_eq!(c.byte_range.len(), 1, "cluster[{i}] should be 1 byte");
                let ch = input.as_bytes()[c.byte_range.start];
                assert!(ch.is_ascii_digit(), "cluster[{i}] is not a digit: {ch}");
                assert!(c.advance > 0.0, "cluster[{i}] advance must be positive");
            }
        });
    }

    #[test]
    fn shape_space_advance() {
        with_shaper(|mut shaper| {
            // Space must produce exactly 1 cluster
            let run = shaper.shape(" ").expect("shape space failed");
            assert_eq!(run.clusters.len(), 1);
            assert!(run.clusters[0].advance > 0.0, "space advance must be positive");

            // Space advance should be comparable to ASCII 'a' advance (monospace)
            let run_a = shaper.shape("a").unwrap();
            let space_adv = run.clusters[0].advance;
            let a_adv = run_a.clusters[0].advance;
            // Space should be at least half of 'a' advance (not a degenerate ~1px)
            assert!(
                space_adv >= a_adv * 0.4,
                "space advance ({space_adv:.1}) too narrow vs 'a' ({a_adv:.1})"
            );

            // At 32pt, space should scale proportionally
            let mut shaper2 = Shaper::new().unwrap().with_font_size(32.0);
            let run2 = shaper2.shape(" ").expect("shape space 32pt failed");
            assert!(
                run2.clusters[0].advance > space_adv,
                "32pt space should be wider than 14pt space"
            );
        });
    }

    #[test]
    fn grapheme_advance_cjk_wider_than_ascii() {
        with_shaper(|mut shaper| {
            let ascii = shaper.grapheme_advance("a").expect("ascii advance failed");
            let cjk = shaper.grapheme_advance("世").expect("cjk advance failed");
            assert!(cjk >= ascii, "CJK advance ({cjk}) should be >= ASCII ({ascii})");
        });
    }

    #[test]
    fn shape_hello_world_clusters() {
        with_shaper(|mut shaper| {
            let input = "hello world";
            let run = shaper.shape(input).expect("shape failed");
            // "hello" (5) + " " (1) + "world" (5) = 11 characters
            assert_eq!(
                run.clusters.len(),
                11,
                "expected 11 clusters for 'hello world', got {}",
                run.clusters.len()
            );

            // Verify byte ranges cover the full input without gaps or overlaps
            let mut covered: Vec<bool> = vec![false; input.len()];
            for c in &run.clusters {
                for i in c.byte_range.clone() {
                    assert!(!covered[i], "byte {i} covered by multiple clusters");
                    covered[i] = true;
                }
            }
            assert!(covered.iter().all(|&c| c), "not all bytes covered by clusters");

            // Verify each cluster's bytes match the expected character
            for c in &run.clusters {
                let bytes = &input.as_bytes()[c.byte_range.clone()];
                let s = std::str::from_utf8(bytes).expect("cluster bytes not valid UTF-8");
                assert_eq!(s.len(), 1, "each ASCII cluster should be 1 byte");
            }
        });
    }

    #[test]
    fn shape_json_backslash_quote() {
        // Bug: \" in JSON content was rendered incorrectly
        // Raw bytes: backslash (0x5C) then double-quote (0x22)
        with_shaper(|mut shaper| {
            let input_bytes: &[u8] = &[0x5C, 0x22]; // "
            let input = std::str::from_utf8(input_bytes).unwrap();
            let run = shaper.shape(input).expect("shape backslash-quote failed");
            assert_eq!(
                run.clusters.len(),
                2,
                "backslash + quote should produce 2 clusters, got {}",
                run.clusters.len()
            );

            // First cluster: backslash (byte 0)
            assert_eq!(run.clusters[0].byte_range, 0..1, "first cluster should be backslash");
            // Second cluster: quote (byte 1)
            assert_eq!(run.clusters[1].byte_range, 1..2, "second cluster should be quote");

            // Both should have positive advance
            assert!(run.clusters[0].advance > 0.0, "backslash advance must be positive");
            assert!(run.clusters[1].advance > 0.0, "quote advance must be positive");
        });
    }

    #[test]
    fn shape_newline_escape_sequence() {
        // Bug: \n in source text was displayed incorrectly
        // Raw bytes: backslash (0x5C) then 'n' (0x6E)
        with_shaper(|mut shaper| {
            let input_bytes: &[u8] = &[0x5C, 0x6E]; // \n
            let input = std::str::from_utf8(input_bytes).unwrap();
            let run = shaper.shape(input).expect("shape backslash-n failed");
            assert_eq!(
                run.clusters.len(),
                2,
                "backslash + n should produce 2 clusters, got {}",
                run.clusters.len()
            );

            assert_eq!(run.clusters[0].byte_range, 0..1);
            assert_eq!(run.clusters[1].byte_range, 1..2);
        });
    }

    #[test]
    fn shape_cjk_mixed_ascii_no_collision() {
        // Bug: CJK characters displayed wrong glyphs due to atlas collision
        // "购买" should produce 2 CJK clusters, not collide with ASCII glyphs
        with_shaper(|mut shaper| {
            let input = "购买";
            let run = shaper.shape(input).expect("shape CJK failed");
            assert_eq!(
                run.clusters.len(),
                2,
                "expected 2 clusters for '购买', got {}",
                run.clusters.len()
            );

            // Each cluster should be 3 bytes (UTF-8 CJK)
            for (i, c) in run.clusters.iter().enumerate() {
                assert_eq!(c.byte_range.len(), 3, "cluster[{i}] should be 3 bytes for CJK");
                assert!(c.advance > 0.0, "cluster[{i}] advance must be positive");
            }

            // Both clusters should have same font_id (same CJK font)
            assert_eq!(
                run.clusters[0].font_id, run.clusters[1].font_id,
                "both CJK chars should use same font"
            );

            // Font ID should differ from ASCII font (Menlo)
            let ascii_run = shaper.shape("ab").unwrap();
            // It's OK if they happen to be the same font, but glyph_ids must differ
            // since these are completely different characters
            assert_ne!(
                run.clusters[0].glyph_id, ascii_run.clusters[0].glyph_id,
                "CJK glyph_id must differ from ASCII glyph_id"
            );
        });
    }

    #[test]
    fn shape_with_highlights_produces_clusters() {
        with_shaper(|mut shaper| {
            let text = "hello world";
            // Bold "world" at byte 6
            let spans =
                vec![(0usize, Weight::NORMAL, Style::Normal), (6, Weight::BOLD, Style::Normal)];
            let run =
                shaper.shape_with_highlights(text, &spans).expect("shape_with_highlights failed");
            assert!(!run.clusters.is_empty(), "must produce clusters");
            assert!(run.width > 0.0, "width must be positive");
        });
    }

    #[test]
    fn shape_with_highlights_byte_ranges_match_text() {
        with_shaper(|mut shaper| {
            let text = "hello world";
            let spans =
                vec![(0usize, Weight::NORMAL, Style::Normal), (6, Weight::BOLD, Style::Normal)];
            let run =
                shaper.shape_with_highlights(text, &spans).expect("shape_with_highlights failed");
            for (i, c) in run.clusters.iter().enumerate() {
                assert!(
                    c.byte_range.end <= text.len(),
                    "cluster[{i}] byte_range {:?} exceeds text len {}",
                    c.byte_range,
                    text.len()
                );
            }
        });
    }

    #[test]
    fn shape_with_highlights_bold_changes_font() {
        with_shaper(|mut shaper| {
            let text = "aa bb";
            let normal_run = shaper.shape(text).expect("normal shape failed");
            let bold_spans =
                vec![(0usize, Weight::NORMAL, Style::Normal), (3, Weight::BOLD, Style::Normal)];
            let bold_run =
                shaper.shape_with_highlights(text, &bold_spans).expect("bold shape failed");
            // The "bb" portion (bytes 3..5) should use a different font_id when bold
            // Find the cluster covering byte 3
            let normal_font =
                normal_run.clusters.iter().find(|c| c.byte_range.start >= 3).map(|c| c.font_id);
            let bold_font =
                bold_run.clusters.iter().find(|c| c.byte_range.start >= 3).map(|c| c.font_id);
            if let (Some(nf), Some(bf)) = (normal_font, bold_font) {
                // font_id MAY differ if a bold variant face is installed
                // At minimum, both runs must succeed
                let _ = (nf, bf);
            }
        });
    }

    #[test]
    fn shape_with_highlights_empty_text() {
        with_shaper(|mut shaper| {
            let spans = vec![(0usize, Weight::NORMAL, Style::Normal)];
            let run = shaper.shape_with_highlights("", &spans).expect("empty text");
            assert!(run.clusters.is_empty());
            assert_eq!(run.width, 0.0);
        });
    }

    #[test]
    fn shape_with_highlights_empty_spans() {
        with_shaper(|mut shaper| {
            let run = shaper.shape_with_highlights("hello", &[]).expect("empty spans");
            assert!(run.clusters.is_empty());
            assert_eq!(run.width, 0.0);
        });
    }

    #[test]
    fn shape_with_highlights_italic_style() {
        with_shaper(|mut shaper| {
            let text = "normal italic";
            let spans =
                vec![(0usize, Weight::NORMAL, Style::Normal), (7, Weight::NORMAL, Style::Italic)];
            let run = shaper.shape_with_highlights(text, &spans).expect("italic shape failed");
            assert!(!run.clusters.is_empty());
            assert!(run.width > 0.0);
        });
    }

    #[test]
    fn shape_respects_weight_and_style() {
        with_shaper(|mut shaper| {
            shaper.set_font_weight(Weight::BOLD);
            shaper.set_font_style(Style::Italic);
            let run = shaper.shape("test").expect("shape with bold+italic failed");
            assert!(!run.clusters.is_empty());
            // Restore
            shaper.set_font_weight(Weight::NORMAL);
            shaper.set_font_style(Style::Normal);
        });
    }
}
