//! PreviewEngine — shared cache/render/scroll/selection/search core.
//!
//! MarkdownView and NovelView each wrap a PreviewEngine, providing different
//! doc-building strategies (markdown parse vs. txt→MarkdownDoc) and styles
//! (from_theme vs. novel).

use crate::builder::{CodeHighlighter, HighlightSpan, MarkdownDoc};
use crate::layout::{BlockSource, HeadingEntry, LazyLayout};
use crate::projection::{
    CursorAffinity, HorizontalDirection, LineBoundary, SourceProjectionIndex, VisualPosition,
};
use crate::search::SearchHighlightCache;
use crate::selection::{self, SelectionState, ViewPos};
use crate::style::MarkdownStyle;
use core::document::DocView;
use core::highlight::{Highlighter, find_language, highlight_kind_scope};
use render::GlyphVertex;
use std::path::Path;
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use stdext::arena::scratch_arena;
use ui::Theme;
use ui::core::paint::DrawList;
use ui::plugin::{PluginFactory, PluginMessage, PluginQuery, PluginResponse, ViewPlugin};
use unicode_segmentation::UnicodeSegmentation;

// ===== Syntax highlighter =====

struct ByteSliceDoc<'a> {
    data: &'a [u8],
}

impl<'a> core::document::ReadableDocument for ByteSliceDoc<'a> {
    fn read_forward(&self, off: usize) -> &[u8] {
        if off >= self.data.len() { &[] } else { &self.data[off..] }
    }
    fn read_backward(&self, off: usize) -> &[u8] {
        if off == 0 { &[] } else { &self.data[..off] }
    }
}

struct AppCodeHighlighter<'a> {
    theme: &'a Theme,
}

const WYSIWYG_CURSOR_ASCENT_RATIO: f32 = 0.8;
const HIT_TEST_SNAP_MAX_LINE_HEIGHTS: f32 = 3.0;
const PERF_LOG_ENV: &str = "EDIT_PLUS_PERF_LOG";
const PERF_LOG_THRESHOLD_US_ENV: &str = "EDIT_PLUS_PERF_LOG_THRESHOLD_US";
const DEFAULT_PERF_LOG_THRESHOLD_US: u128 = 1_000;
const ASCII_METRIC_COUNT: usize = 128;
const DEFAULT_ASCII_ADVANCE_RATIO: f32 = 0.55;
const DEFAULT_CJK_ADVANCE_RATIO: f32 = 1.0;
const TAB_SPACE_COUNT: f32 = 4.0;

#[derive(Clone)]
struct NavigationFontMetrics {
    ascii_advance_ratios: [f32; ASCII_METRIC_COUNT],
    cjk_advance_ratio: f32,
    fallback_advance_ratio: f32,
}

impl Default for NavigationFontMetrics {
    fn default() -> Self {
        Self {
            ascii_advance_ratios: [DEFAULT_ASCII_ADVANCE_RATIO; ASCII_METRIC_COUNT],
            cjk_advance_ratio: DEFAULT_CJK_ADVANCE_RATIO,
            fallback_advance_ratio: DEFAULT_CJK_ADVANCE_RATIO,
        }
    }
}

impl NavigationFontMetrics {
    fn measure(shaper: &mut shaping::Shaper, font_size: f32, font_family: Option<&str>) -> Self {
        let old_size = shaper.font_size();
        let old_weight = shaper.font_weight();
        let old_style = shaper.font_style();
        let old_family = shaper.font_family().map(str::to_owned);
        let font_size = font_size.max(f32::EPSILON);
        shaper.set_font_size(font_size);
        shaper.set_font_weight(shaping::Weight::NORMAL);
        shaper.set_font_style(shaping::Style::Normal);
        shaper.set_font_family(font_family);

        let mut metrics = Self::default();
        for byte in 0x20u8..0x7f {
            let mut encoded = [0; 4];
            let grapheme = char::from(byte).encode_utf8(&mut encoded);
            metrics.ascii_advance_ratios[byte as usize] = shaper
                .grapheme_advance(grapheme)
                .unwrap_or(font_size * DEFAULT_ASCII_ADVANCE_RATIO)
                / font_size;
        }
        metrics.ascii_advance_ratios[b'\t' as usize] =
            metrics.ascii_advance_ratios[b' ' as usize] * TAB_SPACE_COUNT;
        metrics.cjk_advance_ratio =
            shaper.grapheme_advance("中").unwrap_or(font_size * DEFAULT_CJK_ADVANCE_RATIO)
                / font_size;
        metrics.fallback_advance_ratio =
            shaper.grapheme_advance("�").unwrap_or(font_size * metrics.cjk_advance_ratio)
                / font_size;

        shaper.set_font_size(old_size);
        shaper.set_font_weight(old_weight);
        shaper.set_font_style(old_style);
        shaper.set_font_family(old_family.as_deref());
        metrics
    }

    fn grapheme_advance(&self, grapheme: &str, font_size: f32) -> f32 {
        if grapheme.is_ascii() {
            return grapheme
                .bytes()
                .map(|byte| self.ascii_advance_ratios[byte as usize] * font_size)
                .sum();
        }
        let Some(first_character) = grapheme.chars().next() else {
            return 0.0;
        };
        if first_character.is_ascii() {
            return self.ascii_advance_ratios[first_character as usize] * font_size;
        }
        let ratio = if crate::layout::is_cjk_or_fullwidth(first_character) {
            self.cjk_advance_ratio
        } else {
            self.fallback_advance_ratio
        };
        ratio * font_size
    }

    fn grapheme_x(&self, text: &str, grapheme_position: usize, font_size: f32) -> f32 {
        UnicodeSegmentation::graphemes(text, true)
            .take(grapheme_position)
            .map(|grapheme| self.grapheme_advance(grapheme, font_size))
            .sum()
    }

    fn grapheme_at_x(&self, text: &str, relative_x: f32, font_size: f32) -> usize {
        let mut cumulative_x = 0.0;
        for (grapheme_index, grapheme) in UnicodeSegmentation::graphemes(text, true).enumerate() {
            let advance = self.grapheme_advance(grapheme, font_size);
            if relative_x < cumulative_x + advance * 0.5 {
                return grapheme_index;
            }
            cumulative_x += advance;
        }
        crate::grapheme_map::grapheme_count(text)
    }
}

const DISABLE_INCREMENTAL_LAYOUT_REUSE_ENV: &str =
    "TEXTORA_DISABLE_INCREMENTAL_MARKDOWN_LAYOUT_REUSE";

fn incremental_layout_reuse_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(DISABLE_INCREMENTAL_LAYOUT_REUSE_ENV).is_none())
}

fn perf_logging_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(PERF_LOG_ENV).is_ok_and(|value| {
            !matches!(value.as_str(), "" | "0" | "false" | "FALSE" | "off" | "OFF")
        })
    })
}

fn perf_log_threshold_us() -> u128 {
    static THRESHOLD_US: OnceLock<u128> = OnceLock::new();
    *THRESHOLD_US.get_or_init(|| {
        std::env::var(PERF_LOG_THRESHOLD_US_ENV)
            .ok()
            .and_then(|value| value.parse::<u128>().ok())
            .unwrap_or(DEFAULT_PERF_LOG_THRESHOLD_US)
    })
}

fn should_log_perf(elapsed_us: u128) -> bool {
    perf_logging_enabled() && elapsed_us >= perf_log_threshold_us()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceLineAtByte {
    index: usize,
    start: usize,
    end: usize,
    is_blank: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmptySourceLineRole {
    HiddenBlockSeparator,
    EditableLine,
}

#[cfg(test)]
static SOURCE_LINE_AT_BYTE_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
fn reset_source_line_at_byte_call_count() {
    SOURCE_LINE_AT_BYTE_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
fn source_line_at_byte_call_count() -> usize {
    SOURCE_LINE_AT_BYTE_CALLS.load(Ordering::Relaxed)
}

type RenderedLineRef<'a> = (usize, &'a crate::layout::FlatLine);
type SurroundingRenderedLines<'a> = (Option<RenderedLineRef<'a>>, Option<RenderedLineRef<'a>>);

struct StandalonePreeditRenderData<'a> {
    text: &'a str,
    cursor: Option<(usize, usize)>,
    x: f32,
    baseline_y: f32,
    font_size: f32,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct FlatLineProjectionBoundaries {
    pub flat_idx: usize,
    pub boundaries: Vec<usize>,
}

impl SourceLineAtByte {
    fn is_empty(self) -> bool {
        self.is_blank
    }
}

#[inline]
fn source_line_span_to_at_byte(
    span: crate::layout::source_line_map::SourceLineEntry,
) -> SourceLineAtByte {
    SourceLineAtByte {
        index: span.index,
        start: span.start,
        end: span.end,
        is_blank: span.is_blank,
    }
}

impl<'a> CodeHighlighter for AppCodeHighlighter<'a> {
    fn highlight(&self, language_tag: &str, code: &str) -> Vec<Vec<HighlightSpan>> {
        let language = match find_language(language_tag) {
            Some(lang) => lang,
            None => return vec![vec![]; code.lines().count().max(1)],
        };
        let doc = ByteSliceDoc { data: code.as_bytes() };
        let mut hl = Highlighter::new(&doc, language);
        let arena = scratch_arena(None);
        let mut result = Vec::new();
        let mut line_start = 0usize;
        for line in code.split('\n') {
            let line_end = line_start + line.len();
            let bvec = hl.parse_line(&arena, line_start);
            let mut spans = Vec::new();
            for (i, h) in bvec.iter().enumerate() {
                let span_start = line.floor_char_boundary(h.start);
                let span_end = if i + 1 < bvec.len() {
                    line.floor_char_boundary(bvec[i + 1].start)
                } else {
                    line.len()
                };
                if span_end <= span_start {
                    continue;
                }
                let color = self.theme.scope_color(highlight_kind_scope(h.kind));
                spans.push(HighlightSpan { start: span_start, len: span_end - span_start, color });
            }
            result.push(spans);
            line_start = line_end + 1;
        }
        result
    }
}

// ===== MarkdownRenderSettings =====

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkdownRenderSettings {
    pub font_size: f32,
    pub line_height: f32,
    pub toc_max_depth: u8,
}

impl MarkdownRenderSettings {
    pub fn from_metrics(
        settings: &ui::settings::Settings,
        metrics: &ui::settings::UiMetrics,
    ) -> Self {
        Self {
            font_size: metrics.font_size,
            line_height: metrics.line_height,
            toc_max_depth: settings.toc_max_depth,
        }
    }

    pub(crate) fn style(self, theme: &Theme) -> MarkdownStyle {
        MarkdownStyle::from_theme(theme, self.font_size, self.line_height)
    }
}

// ===== BlockAnchor (scroll stability) =====

#[derive(Debug, Clone)]
struct BlockAnchor {
    block_idx: usize,
    offset_in_block: f32,
}

// ===== EngineDirty =====

#[derive(Clone, Debug, PartialEq, Eq)]
enum EngineDirty {
    Clean,
    SourceChanged,
    /// Reserved for future style-change event (e.g. theme toggle).
    /// Currently unused: style changes are detected via `cached_style_hash` comparison
    /// in `needs_rebuild()`, not via this variant.
    StyleChanged,
    /// Reserved for future viewport-resize event.
    /// Currently unused: viewport changes are detected via `cached_viewport_w` comparison
    /// in `needs_rebuild()`, not via this variant.
    ViewportChanged,
    /// Selection endpoints changed; reuse the existing source and block layout.
    SelectionChanged,
    CursorMoved {
        old_byte: Option<usize>,
        new_byte: usize,
    },
}

// ===== PreviewEngine =====

/// Shared cache/render/scroll/selection/search engine.
/// Used by both MarkdownView and NovelView.
/// 共享缓存/渲染/滚动/选择/搜索引擎。泛型 S 允许 MarkdownView (MarkdownDoc)
/// 和 NovelView (NovelStructure) 共用同一套渲染管线。
pub struct PreviewEngine<S: BlockSource = MarkdownDoc> {
    lazy: Option<LazyLayout<S>>,
    dirty: EngineDirty,
    cached_style_hash: u64,
    cached_viewport_w: f32,

    pub scroll_y: f32,
    pub content_height: f32,
    headings: Vec<HeadingEntry>,
    pending_heading_jump: Option<usize>,

    sel: SelectionState,
    sel_anchor_byte: Option<usize>,
    sel_cursor_byte: Option<usize>,
    search: SearchHighlightCache,

    cached_dl: Option<DrawList>,
    cached_dl_scroll_y: f32,
    cached_dl_viewport: (f32, f32),
    cached_vertices: Option<Vec<GlyphVertex>>,
    cached_offset_x: f32,
    cached_offset_y: f32,

    pub base_font_size: f32,
    pub base_line_height: f32,
    rendered_body_font_size: f32,
    rendered_line_height: f32,
    body_navigation_font_metrics: NavigationFontMetrics,
    code_navigation_font_metrics: NavigationFontMetrics,
    navigation_metrics_style_hash: u64,
    pub paragraph_spacing: f32,
    pub toc_max_depth: u8,

    /// WYSIWYG 编辑上下文。None 表示纯预览模式 (快速路径)。
    pub edit_ctx: Option<crate::edit::EditContext>,
    /// Full source text for WYSIWYG span expansion (materialize_line).
    edit_source: Option<String>,
    /// 源码行 ↔ 视觉坐标桥接。与 `edit_source` 同步更新。None 时用旧路径兜底。
    source_line_map: Option<crate::layout::source_line_map::SourceLineMap>,
    /// Generation of the source text currently rendered by this engine.
    source_generation: u32,
    /// Whether the cursor blink phase is currently visible (controlled by app).
    pub cursor_visible: bool,
    /// Actual shaped advance from an editable empty line's start to the IME caret.
    standalone_preedit_cursor_advance: Option<f32>,
}

impl Default for PreviewEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: BlockSource> PreviewEngine<S> {
    pub fn new() -> Self {
        Self {
            lazy: None,
            dirty: EngineDirty::SourceChanged,
            cached_style_hash: 0,
            cached_viewport_w: 0.0,
            scroll_y: 0.0,
            content_height: 0.0,
            headings: Vec::new(),
            pending_heading_jump: None,
            sel: SelectionState::new(),
            sel_anchor_byte: None,
            sel_cursor_byte: None,
            search: SearchHighlightCache::new(),
            cached_dl: None,
            cached_dl_scroll_y: -1.0,
            cached_dl_viewport: (0.0, 0.0),
            cached_vertices: None,
            cached_offset_x: 0.0,
            cached_offset_y: 0.0,
            base_font_size: 15.0,
            base_line_height: 24.0,
            rendered_body_font_size: 15.0,
            rendered_line_height: 24.0,
            body_navigation_font_metrics: NavigationFontMetrics::default(),
            code_navigation_font_metrics: NavigationFontMetrics::default(),
            navigation_metrics_style_hash: 0,
            paragraph_spacing: 12.0,
            toc_max_depth: 3,
            edit_ctx: None,
            edit_source: None,
            source_line_map: None,
            source_generation: 0,
            cursor_visible: true,
            standalone_preedit_cursor_advance: None,
        }
    }

    /// Mark the engine cache as dirty (source changed). Full rebuild.
    pub fn mark_source_dirty(&mut self) {
        self.dirty = EngineDirty::SourceChanged;
        self.sel.clear();
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    /// Mark only the cursor position as changed — no full rebuild needed.
    fn mark_cursor_moved(&mut self, new_byte: usize) {
        let old_byte = self.edit_ctx.as_ref().map(|ctx| ctx.cursor_byte);
        if old_byte == Some(new_byte) {
            return;
        }
        let preedit_text = self.edit_ctx.as_ref().and_then(|ctx| ctx.preedit_text.clone());
        let preedit_cursor = self.edit_ctx.as_ref().and_then(|ctx| ctx.preedit_cursor);
        self.edit_ctx =
            Some(crate::edit::EditContext { cursor_byte: new_byte, preedit_text, preedit_cursor });
        if old_byte.is_none() {
            self.dirty = EngineDirty::SourceChanged;
        } else if !matches!(self.dirty, EngineDirty::SourceChanged) {
            self.dirty = EngineDirty::CursorMoved { old_byte, new_byte };
        }
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    /// 设置 IME preedit 文本。**严格保证不改动 `edit_ctx.cursor_byte`**（方案
    /// 2026-07-06 阶段 3a）：如果尚未收到 cursor，直接忽略 preedit——不能凭空
    /// 造出 cursor_byte=0 的假状态。
    fn set_preedit_text(&mut self, text: String, cursor: Option<(usize, usize)>) {
        let Some(existing_ctx) = self.edit_ctx.as_ref() else {
            // 没有 cursor 时，preedit 无处附着；直接丢弃，避免制造 cursor_byte=0
            // 的伪状态被后续 visual_move / augment_edit 读到。
            return;
        };
        let cursor_byte = existing_ctx.cursor_byte;
        let preedit_text = if text.is_empty() { None } else { Some(text) };
        let preedit_cursor = preedit_text.as_ref().and(cursor);
        if existing_ctx.preedit_text == preedit_text
            && existing_ctx.preedit_cursor == preedit_cursor
        {
            return;
        }
        self.edit_ctx =
            Some(crate::edit::EditContext { cursor_byte, preedit_text, preedit_cursor });
        if !matches!(self.dirty, EngineDirty::SourceChanged) {
            self.dirty =
                EngineDirty::CursorMoved { old_byte: Some(cursor_byte), new_byte: cursor_byte };
        }
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    // ── Heading collection ──

    fn collect_headings(&mut self) {
        self.headings.clear();
        let Some(ref lazy) = self.lazy else { return };
        for bi in 0..lazy.estimated_heights.len() {
            let doc_idx = match lazy.laid_to_doc(bi) {
                Some(i) => i,
                None => continue,
            };
            let kind = match lazy.source.blocks().get(doc_idx) {
                Some(b) => &b.kind,
                None => continue,
            };
            if let crate::builder::BlockKind::Heading { level } = kind {
                if *level > self.toc_max_depth {
                    continue;
                }
                // Get heading text from source blocks' headings or from materialized block.
                let text = if let Some(src_block) = lazy.source.blocks().get(doc_idx) {
                    src_block.text_lines.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                let y = lazy.estimated_positions[bi] + lazy.y_delta.get(bi).copied().unwrap_or(0.0);
                self.headings.push(HeadingEntry { text, level: *level, y_offset: y });
            }
        }
    }

    pub fn headings(&self) -> &[HeadingEntry] {
        &self.headings
    }

    pub fn current_heading_index(&self, scroll_y: f32) -> Option<usize> {
        if self.headings.is_empty() {
            return None;
        }
        match self.headings.binary_search_by(|h| {
            h.y_offset.partial_cmp(&scroll_y).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(i) => Some(i),
            Err(0) => Some(0),
            Err(i) => Some(i - 1),
        }
    }

    pub fn scroll_to_heading(&mut self, index: usize) {
        if let Some(h) = self.headings.get(index) {
            self.scroll_y = h.y_offset.clamp(0.0, self.content_height);
            self.pending_heading_jump = Some(index);
        }
    }

    // ── Layout rebuild ──

    fn needs_rebuild(&self, style_hash: u64, viewport_w: f32) -> bool {
        matches!(
            self.dirty,
            EngineDirty::SourceChanged | EngineDirty::StyleChanged | EngineDirty::ViewportChanged
        ) || self.lazy.is_none()
            || style_hash != self.cached_style_hash
            || viewport_w != self.cached_viewport_w
    }

    fn rebuild_layout(
        &mut self,
        doc: S,
        style: &MarkdownStyle,
        viewport_w: f32,
        viewport_h: f32,
        shaper: Option<&mut shaping::Shaper>,
        highlighter: &AppCodeHighlighter<'_>,
        doc_view: &dyn core::document::DocView,
        full_flat_lines: bool,
        full_layout: bool,
    ) {
        self.paragraph_spacing = style.paragraph_spacing;
        let selection_range = self.byte_selection_range().map(|(start, end)| start..end);
        let previous_layout = (incremental_layout_reuse_enabled()
            && matches!(self.dirty, EngineDirty::SourceChanged)
            && style_hash_quick(style) == self.cached_style_hash
            && viewport_w == self.cached_viewport_w)
            .then(|| self.lazy.take())
            .flatten();
        let mut lazy = LazyLayout::new(doc, style, viewport_w, doc_view);
        lazy.set_source_generation(self.source_generation);
        lazy.set_edit_source(self.edit_source.clone());
        lazy.set_edit_ctx(self.edit_ctx.clone());
        lazy.set_selection_range(selection_range);
        lazy.reserve_extra_blank_source_lines(style.line_height, style.paragraph_spacing);
        if let Some(previous_layout) = previous_layout {
            lazy.reuse_unchanged_blocks_from(previous_layout);
        }
        if full_layout {
            // Editing mode: materialize all blocks with full shaping.
            lazy.ensure_all_blocks(style, viewport_w, shaper, Some(highlighter), doc_view);
        } else if full_flat_lines {
            // WYSIWYG editing needs whole-document flat line mappings for
            // selection/navigation, but only visible blocks need precise shaping.
            lazy.ensure_all_blocks(style, viewport_w, None, None, doc_view);
            if let Some(s) = shaper {
                lazy.refresh_precise_range(
                    self.scroll_y,
                    viewport_h,
                    style,
                    s,
                    Some(highlighter),
                    doc_view,
                );
            }
        } else if let Some(s) = shaper {
            // Preview with shaper: viewport-driven lazy layout.
            lazy.ensure_visible(
                self.scroll_y,
                viewport_h,
                style,
                viewport_w,
                s,
                Some(highlighter),
                doc_view,
            );
        } else {
            // No shaper: materialize all blocks without text shaping.
            lazy.ensure_all_blocks(style, viewport_w, None, None, doc_view);
        }
        lazy.build_flat_lines(doc_view);
        self.content_height = lazy.total_height;
        self.lazy = Some(lazy);
        self.dirty = EngineDirty::Clean;
        self.collect_headings();
        self.cached_dl = None;
        self.cached_vertices = None;
    }

    // ── Precision pass ──

    fn precision_pass_on_scroll(
        &mut self,
        style: &MarkdownStyle,
        viewport_h: f32,
        shaper: Option<&mut shaping::Shaper>,
        highlighter: &AppCodeHighlighter<'_>,
        doc_view: &dyn core::document::DocView,
    ) {
        let scroll_anchor = self.lazy.as_ref().and_then(|lazy| {
            if lazy.estimated_heights.is_empty() {
                return None;
            }
            let idx = lazy.block_at_y(self.scroll_y);
            let block_y =
                lazy.estimated_positions[idx] + lazy.y_delta.get(idx).copied().unwrap_or(0.0);
            Some(BlockAnchor { block_idx: idx, offset_in_block: self.scroll_y - block_y })
        });
        let had_deltas = if let Some(ref mut lazy) = self.lazy {
            if let Some(s) = shaper {
                let deltas = lazy.ensure_precise_range(
                    self.scroll_y,
                    viewport_h,
                    style,
                    s,
                    Some(highlighter),
                    doc_view,
                );
                if !deltas.is_empty() {
                    self.content_height = lazy.total_height;
                }
                !deltas.is_empty()
            } else {
                false
            }
        } else {
            false
        };
        if had_deltas {
            self.collect_headings();
            if let Some(idx) = self.pending_heading_jump.take() {
                if let Some(h) = self.headings.get(idx) {
                    self.scroll_y = h.y_offset.clamp(0.0, self.content_height);
                }
            } else if let Some(ref anchor) = scroll_anchor {
                self.restore_anchor(anchor);
            }
        }
    }

    fn restore_anchor(&mut self, anchor: &BlockAnchor) {
        let Some(ref lazy) = self.lazy else {
            return;
        };
        if anchor.block_idx >= lazy.estimated_heights.len() {
            return;
        }
        let block_y = lazy.estimated_positions[anchor.block_idx]
            + lazy.y_delta.get(anchor.block_idx).copied().unwrap_or(0.0);
        self.scroll_y = (block_y + anchor.offset_in_block).clamp(0.0, self.content_height);
    }

    // ── Core render ──

    /// Core render method. `build_doc` produces a `S: BlockSource` from the given style.
    /// Returns (DrawList, needs_drain).
    pub fn render(
        &mut self,
        theme: &Theme,
        viewport_w: f32,
        viewport_h: f32,
        offset_x: f32,
        offset_y: f32,
        style: &MarkdownStyle,
        build_doc: impl FnOnce(&MarkdownStyle) -> S,
        mut shaper: Option<&mut shaping::Shaper>,
        doc_view: &dyn core::document::DocView,
        full_flat_lines: bool,
        full_layout: bool,
    ) -> (DrawList, bool) {
        let perf_total_started_at = Instant::now();
        let mut perf_build_doc_us = 0;
        let mut perf_rebuild_us = 0;
        let mut perf_cursor_us = 0;
        let mut render_path = "draw";
        self.rendered_body_font_size = style.body_font_size;
        self.rendered_line_height = style.line_height;
        let style_hash = style_hash_quick(style);
        let highlighter = AppCodeHighlighter { theme };
        if style_hash != self.navigation_metrics_style_hash
            && let Some(active_shaper) = shaper.as_deref_mut()
        {
            self.body_navigation_font_metrics = NavigationFontMetrics::measure(
                active_shaper,
                style.body_font_size,
                style.body_font_family.first().map(String::as_str),
            );
            self.code_navigation_font_metrics = NavigationFontMetrics::measure(
                active_shaper,
                style.code_font_size,
                style.code_font_family.as_deref(),
            );
            self.navigation_metrics_style_hash = style_hash;
        }

        // If cursor moved but no shaper available, escalate to full rebuild
        // so that needs_rebuild / rebuild_layout (below) will handle it.
        if matches!(&self.dirty, EngineDirty::CursorMoved { .. }) && shaper.is_none() {
            self.dirty = EngineDirty::SourceChanged;
            self.cached_dl = None;
            self.cached_vertices = None;
        }

        if self.needs_rebuild(style_hash, viewport_w) {
            render_path = "rebuild";
            let build_doc_started_at = Instant::now();
            let doc = build_doc(style);
            perf_build_doc_us = build_doc_started_at.elapsed().as_micros();
            let rebuild_started_at = Instant::now();
            self.rebuild_layout(
                doc,
                style,
                viewport_w,
                viewport_h,
                shaper.as_deref_mut(),
                &highlighter,
                doc_view,
                full_flat_lines,
                full_layout,
            );
            perf_rebuild_us = rebuild_started_at.elapsed().as_micros();
            self.cached_style_hash = style_hash;
            self.cached_viewport_w = viewport_w;
        }

        // Handle cursor-only changes: invalidate affected blocks without full rebuild.
        if let EngineDirty::CursorMoved { old_byte, new_byte } = self.dirty.clone() {
            render_path = "cursor";
            let cursor_started_at = Instant::now();
            let selection_range = self.byte_selection_range().map(|(start, end)| start..end);
            // shaper is guaranteed Some here (None case was promoted above).
            if let Some(ref mut s) = shaper {
                if let Some(lazy) = self.lazy.as_mut() {
                    lazy.set_edit_ctx(self.edit_ctx.clone());
                    lazy.set_selection_range(selection_range);
                    // 2026-07-06 阶段 4b：full_layout 分支不再对整个文档重 shape，
                    // 而是走"invalidate 视口内命中块 → ensure_all_blocks 结构性重排
                    // （不 shape）→ refresh_precise_range 只 shape 可见范围"。
                    // 与 full_flat_lines 分支共享同一路径，避免长文档下光标一次移动
                    // 就把全部块都 reshape 一遍。
                    if full_layout || full_flat_lines {
                        lazy.invalidate_visible_lines_for_source_bytes(
                            old_byte.into_iter().chain(std::iter::once(new_byte)),
                            self.scroll_y,
                            viewport_h,
                        );
                        lazy.ensure_all_blocks(style, viewport_w, None, None, doc_view);
                        lazy.refresh_precise_range(
                            self.scroll_y,
                            viewport_h,
                            style,
                            s,
                            Some(&highlighter),
                            doc_view,
                        );
                    } else {
                        lazy.invalidate_lines_for_source_bytes(
                            old_byte.into_iter().chain(std::iter::once(new_byte)),
                        );
                        lazy.ensure_visible(
                            self.scroll_y,
                            viewport_h,
                            style,
                            viewport_w,
                            s,
                            Some(&highlighter),
                            doc_view,
                        );
                    }
                    lazy.build_flat_lines(doc_view);
                    self.content_height = lazy.total_height;
                }
                self.dirty = EngineDirty::Clean;
            }
            perf_cursor_us = cursor_started_at.elapsed().as_micros();
        }

        if self.dirty == EngineDirty::SelectionChanged {
            render_path = "selection";
            let selection_range = self.byte_selection_range().map(|(start, end)| start..end);
            if let Some(lazy) = self.lazy.as_mut() {
                lazy.set_selection_range(selection_range);
            }
            self.dirty = EngineDirty::Clean;
        }

        let vp_key = (viewport_w, viewport_h);
        if let Some(ref cached) = self.cached_dl
            && self.cached_dl_scroll_y == self.scroll_y
            && self.cached_dl_viewport == vp_key
        {
            let elapsed_us = perf_total_started_at.elapsed().as_micros();
            if should_log_perf(elapsed_us) {
                eprintln!(
                    "[perf:md_engine] path=cache total={}us cmds={} vertices_cached={} viewport=({:.0}x{:.0}) scroll_y={:.1}",
                    elapsed_us,
                    cached.cmds.len(),
                    self.cached_vertices.is_some(),
                    viewport_w,
                    viewport_h,
                    self.scroll_y,
                );
            }
            return if self.cached_vertices.is_some() {
                (DrawList::new(), false)
            } else {
                (cached.clone(), true)
            };
        }

        let precision_started_at = Instant::now();
        self.precision_pass_on_scroll(
            style,
            viewport_h,
            shaper.as_deref_mut(),
            &highlighter,
            doc_view,
        );
        let perf_precision_us = precision_started_at.elapsed().as_micros();

        let lazy = self.lazy.as_ref().expect("lazy layout must exist after precision pass");
        let mut dl = DrawList::new();
        let (visible, visible_yd) = lazy.materialized_blocks();
        let block_count = lazy.source.blocks().len();
        let flat_line_count = lazy.flat_lines.len();
        let visible_block_count = visible.blocks.len();
        let draw_started_at = Instant::now();
        crate::render::render_doc_with_offset_and_ascii_diagrams(
            &visible,
            style,
            &mut dl,
            self.scroll_y,
            viewport_h,
            offset_x,
            offset_y,
            shaper,
            &visible_yd,
            Some(lazy.ascii_diagrams()),
        );
        let perf_draw_us = draw_started_at.elapsed().as_micros();
        let command_count = dl.cmds.len();
        self.cached_dl = Some(dl.clone());
        self.cached_dl_scroll_y = self.scroll_y;
        self.cached_dl_viewport = vp_key;
        self.cached_vertices = None;
        self.cached_offset_x = offset_x;
        self.cached_offset_y = offset_y;
        let elapsed_us = perf_total_started_at.elapsed().as_micros();
        if should_log_perf(elapsed_us) {
            eprintln!(
                "[perf:md_engine] path={} total={}us build_doc={}us rebuild={}us cursor={}us precision={}us draw={}us blocks={} visible_blocks={} flat_lines={} cmds={} viewport=({:.0}x{:.0}) scroll_y={:.1} full_flat={} full_layout={}",
                render_path,
                elapsed_us,
                perf_build_doc_us,
                perf_rebuild_us,
                perf_cursor_us,
                perf_precision_us,
                perf_draw_us,
                block_count,
                visible_block_count,
                flat_line_count,
                command_count,
                viewport_w,
                viewport_h,
                self.scroll_y,
                full_flat_lines,
                full_layout,
            );
        }
        (dl, true)
    }

    // ── Scroll ──

    pub fn scroll(&mut self, delta: f32, viewport_h: f32) -> bool {
        let max_scroll = (self.content_height - viewport_h).max(0.0);
        let old = self.scroll_y;
        self.scroll_y = (self.scroll_y + delta).clamp(0.0, max_scroll);
        if (self.scroll_y - old).abs() > 0.5 {
            self.pending_heading_jump = None;
            true
        } else {
            false
        }
    }

    // ── Selection ──

    pub fn hit_test(&self, px: f32, py: f32, offset_x: f32, offset_y: f32) -> Option<ViewPos> {
        let lazy = self.lazy.as_ref()?;
        selection::hit_test(&lazy.flat_lines, self.scroll_y, px, py, offset_x, offset_y)
    }

    pub fn selection_range(&self) -> Option<(ViewPos, ViewPos)> {
        self.sel.range()
    }
    pub fn clear_selection(&mut self) {
        self.sel.clear();
        self.sel_anchor_byte = None;
        self.sel_cursor_byte = None;
        self.mark_selection_changed();
    }

    pub fn select_all(&mut self) {
        let Some(ref lazy) = self.lazy else { return };
        self.sel.select_all(&lazy.flat_lines);
        self.sel_anchor_byte = None;
        self.sel_cursor_byte = None;
        self.mark_selection_changed();
    }

    pub fn word_at_pos(&self, pos: ViewPos) -> (ViewPos, ViewPos) {
        let Some(ref lazy) = self.lazy else { return (pos, pos) };
        selection::word_at_pos(&lazy.flat_lines, pos)
    }

    pub fn line_range_at_pos(&self, pos: ViewPos) -> (ViewPos, ViewPos) {
        let Some(ref lazy) = self.lazy else {
            return (
                ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: 0 },
                ViewPos { flat_line_idx: pos.flat_line_idx, grapheme_pos: 0 },
            );
        };
        selection::line_range_at_pos(&lazy.flat_lines, pos)
    }

    pub fn selected_text(&self) -> Option<String> {
        let lazy = self.lazy.as_ref()?;
        self.sel.selected_text(&lazy.flat_lines)
    }

    fn byte_selection_range(&self) -> Option<(usize, usize)> {
        if let (Some(anchor), Some(cursor)) = (self.sel_anchor_byte, self.sel_cursor_byte)
            && anchor != cursor
        {
            return Some((anchor.min(cursor), anchor.max(cursor)));
        }

        let (start, end) = self.sel.range()?;
        let start_byte =
            self.byte_from_flat_line_and_visual_grapheme(start.flat_line_idx, start.grapheme_pos)?;
        let end_byte =
            self.byte_from_flat_line_and_visual_grapheme(end.flat_line_idx, end.grapheme_pos)?;

        if start_byte <= end_byte {
            Some((start_byte, end_byte))
        } else {
            Some((end_byte, start_byte))
        }
    }

    pub fn selection_source_range(&self) -> Option<(usize, usize)> {
        self.byte_selection_range()
    }

    pub fn selection_highlights(&self, sel_color: [f32; 4]) -> DrawList {
        let Some(ref lazy) = self.lazy else { return DrawList::new() };
        let Some((start, end)) =
            self.sel.range().or_else(|| self.visual_range_for_byte_selection())
        else {
            return DrawList::new();
        };
        let selection = SelectionState { anchor: Some(start), cursor: Some(end) };
        selection.highlights(
            &lazy.flat_lines,
            self.scroll_y,
            self.cached_offset_x,
            self.cached_offset_y,
            self.cached_dl_viewport.1,
            sel_color,
        )
    }

    pub fn has_selection(&self) -> bool {
        self.sel.has_selection() || self.byte_selection_range().is_some()
    }

    pub fn set_sel_cursor(&mut self, pos: Option<(usize, usize)>) {
        self.sel.cursor = pos.map(|(l, c)| ViewPos { flat_line_idx: l, grapheme_pos: c });
        self.sel_cursor_byte = None;
        self.mark_selection_changed();
    }

    pub fn set_sel_anchor(&mut self, pos: Option<(usize, usize)>) {
        self.sel.anchor = pos.map(|(l, c)| ViewPos { flat_line_idx: l, grapheme_pos: c });
        self.sel_anchor_byte = None;
        self.mark_selection_changed();
    }

    pub fn set_sel_cursor_byte(&mut self, byte: Option<usize>) {
        if let Some(b) = byte {
            self.sel_cursor_byte = Some(b);
            self.sel.cursor = self
                .find_flat_and_grapheme_for_byte(b)
                .map(|(l, c)| ViewPos { flat_line_idx: l, grapheme_pos: c });
        } else {
            self.sel.cursor = None;
            self.sel_cursor_byte = None;
        }
        self.mark_selection_changed();
    }

    pub fn set_sel_anchor_byte(&mut self, byte: Option<usize>) {
        if let Some(b) = byte {
            self.sel_anchor_byte = Some(b);
            self.sel.anchor = self
                .find_flat_and_grapheme_for_byte(b)
                .map(|(l, c)| ViewPos { flat_line_idx: l, grapheme_pos: c });
        } else {
            self.sel.anchor = None;
            self.sel_anchor_byte = None;
        }
        self.mark_selection_changed();
    }

    fn mark_selection_changed(&mut self) {
        if matches!(self.dirty, EngineDirty::Clean | EngineDirty::SelectionChanged) {
            self.dirty = EngineDirty::SelectionChanged;
        }
        self.cached_dl = None;
        self.cached_vertices = None;
        self.standalone_preedit_cursor_advance = None;
    }

    // ── Search ──

    pub fn search_highlights(
        &self,
        query: &str,
        case_sensitive: bool,
        _use_regex: bool,
        active_match_idx: usize,
        match_color: [f32; 4],
        inactive_color: [f32; 4],
    ) -> DrawList {
        if query.is_empty() || self.lazy.is_none() {
            return DrawList::new();
        }
        // Search uses its own internal update; we pass generation=0 as search
        // invalidation is handled by the view layer.
        self.search.update_if_needed(
            query,
            case_sensitive,
            0,
            &self.lazy.as_ref().expect("lazy layout must exist for search").flat_lines,
        );
        self.search.highlights(
            self.scroll_y,
            self.cached_dl_viewport.1,
            self.cached_offset_x,
            self.cached_offset_y,
            active_match_idx,
            match_color,
            inactive_color,
        )
    }

    pub fn scroll_to_search_match(&mut self, query: &str, case_sensitive: bool, active_idx: usize) {
        if query.is_empty() {
            return;
        }
        if let Some(ref lazy) = self.lazy {
            self.search.update_if_needed(query, case_sensitive, 0, &lazy.flat_lines);
        }
        let viewport_h = self.cached_dl_viewport.1;
        self.scroll_y = self.search.scroll_to(active_idx, viewport_h, self.scroll_y);
    }

    // ── Vertex cache ──

    pub fn cache_vertices(&mut self, verts: Vec<GlyphVertex>) {
        self.cached_vertices = Some(verts);
    }

    pub fn get_cached_vertices(&self) -> Option<&Vec<GlyphVertex>> {
        self.cached_vertices.as_ref()
    }

    // ── Flat lines ──

    pub fn flat_lines(&self) -> &[crate::layout::FlatLine] {
        self.lazy.as_ref().map_or(&[], |l| &l.flat_lines)
    }

    /// Expose canonical projection boundaries for tests.
    #[cfg(test)]
    pub fn flat_line_projection_boundaries(&self) -> Vec<FlatLineProjectionBoundaries> {
        self.lazy
            .as_ref()
            .map(|lazy| {
                lazy.flat_lines
                    .iter()
                    .filter_map(|line| {
                        line.source_projection.as_ref().map(|projection| {
                            FlatLineProjectionBoundaries {
                                flat_idx: line.flat_idx,
                                boundaries: projection
                                    .boundaries
                                    .iter()
                                    .map(|anchor| anchor.byte)
                                    .collect(),
                            }
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn sel_mut(&mut self) -> &mut SelectionState {
        &mut self.sel
    }
    pub(crate) fn search_mut(&mut self) -> &mut SearchHighlightCache {
        &mut self.search
    }

    // ── WYSIWYG edit support ──

    /// 接收来自 app 层的光标位置变更通知。
    pub fn handle_set_cursor_byte(&mut self, byte: usize) {
        self.mark_cursor_moved(byte);
    }

    /// 设置 WYSIWYG 编辑的完整源码文本。用于 span 展开时读取原始 markers。
    pub fn set_edit_source(&mut self, source: Option<String>) {
        self.source_line_map =
            source.as_deref().map(crate::layout::source_line_map::SourceLineMap::from_source);
        self.edit_source = source;
    }

    pub(crate) fn set_source_generation(&mut self, source_generation: u32) {
        if self.source_generation == source_generation {
            return;
        }

        self.source_generation = source_generation;
        if let Some(lazy) = self.lazy.as_mut() {
            lazy.set_source_generation(source_generation);
        }
        self.mark_source_dirty();
    }

    /// 返回当前源码的行映射（若已设置 edit_source）。
    pub(crate) fn source_line_map(&self) -> Option<&crate::layout::source_line_map::SourceLineMap> {
        self.source_line_map.as_ref()
    }

    /// 查询：返回光标在插件渲染空间中的 (x, y, w, h)。
    /// y = fl.rect.y - scroll_y，与 render_line_with_offset 的投影一致。
    /// 调用方加 plugin_render_bounds().origin 即得窗口像素坐标。
    pub fn cursor_screen_pos(&self) -> Option<(f32, f32, f32, f32)> {
        let ctx = self.edit_ctx.as_ref()?;
        self.cursor_screen_pos_for_byte(ctx.cursor_byte)
    }

    fn grapheme_x_for_line(&self, line: &crate::layout::FlatLine, grapheme_position: usize) -> f32 {
        if line.shaped.is_some() {
            return crate::layout::grapheme_x(line, grapheme_position);
        }
        self.navigation_font_metrics_for_line(line).grapheme_x(
            &line.text,
            grapheme_position,
            line.font_size,
        )
    }

    fn grapheme_at_x_for_line(&self, line: &crate::layout::FlatLine, relative_x: f32) -> usize {
        if line.shaped.is_some() {
            return crate::layout::grapheme_at_x(line, relative_x);
        }
        self.navigation_font_metrics_for_line(line).grapheme_at_x(
            &line.text,
            relative_x,
            line.font_size,
        )
    }

    fn navigation_font_metrics_for_line(
        &self,
        line: &crate::layout::FlatLine,
    ) -> &NavigationFontMetrics {
        if line.is_code {
            &self.code_navigation_font_metrics
        } else {
            &self.body_navigation_font_metrics
        }
    }

    fn cursor_screen_pos_for_byte(&self, cursor_byte: usize) -> Option<(f32, f32, f32, f32)> {
        let lazy = self.lazy.as_ref()?;

        if let Some(rect) = self.empty_source_line_cursor_screen_pos(cursor_byte) {
            return Some(rect);
        }

        let (visual_line_idx, visual_grapheme) = self
            .cursor_visual_position_for_byte(cursor_byte, CursorAffinity::Downstream)
            .map(|position| (position.flat_line_idx, position.grapheme_pos))?;
        let flat_idx = lazy.flat_line_idx_for_projection(visual_line_idx)?;
        let fl = lazy.flat_lines.get(flat_idx)?;
        let x = fl.rect.x
            + crate::layout::grapheme_x(fl, visual_grapheme)
            + self.trailing_stripped_space_advance(flat_idx, cursor_byte);
        let cursor_height = fl.font_size.min(fl.rect.h);
        let text_baseline_y = fl.rect.y + fl.font_size - self.scroll_y;
        let cursor_y = text_baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO;
        Some((x, cursor_y, 2.0, cursor_height))
    }

    #[cfg(test)]
    pub(crate) fn projection_index(&self) -> &SourceProjectionIndex {
        self.lazy
            .as_ref()
            .and_then(|lazy| lazy.source_projection_index.as_ref())
            .expect("rendered WYSIWYG test view must publish a source projection index")
    }

    pub(crate) fn cursor_visual_position_for_byte(
        &self,
        source_byte: usize,
        affinity: CursorAffinity,
    ) -> Option<VisualPosition> {
        self.lazy
            .as_ref()?
            .source_projection_index
            .as_ref()?
            .visual_position_for_source(source_byte, affinity)
    }

    /// Extra x advance when the cursor sits past bytes that were present in the
    /// source but stripped by the markdown parser (e.g. trailing whitespace after
    /// a paragraph). Without this compensation, typing a trailing space visually
    /// stops the cursor at the end of the last shaped grapheme.
    fn trailing_stripped_space_advance(&self, flat_idx: usize, cursor_byte: usize) -> f32 {
        let lazy = match self.lazy.as_ref() {
            Some(l) => l,
            None => return 0.0,
        };
        let sentinel = match lazy
            .flat_lines
            .get(flat_idx)
            .and_then(|line| line.source_projection.as_ref())
            .and_then(|projection| projection.boundaries.last())
        {
            Some(anchor) => anchor.byte,
            None => return 0.0,
        };
        if cursor_byte < sentinel {
            return 0.0;
        }
        let source = match self.edit_source.as_deref() {
            Some(s) => s,
            None => return 0.0,
        };
        let stripped_start = if cursor_byte > sentinel {
            sentinel
        } else {
            source[..cursor_byte]
                .char_indices()
                .rev()
                .find(|(_, character)| !matches!(character, ' ' | '\t'))
                .map_or(0, |(byte, character)| byte + character.len_utf8())
        };
        let stripped = match source.get(stripped_start..cursor_byte) {
            Some(s) => s,
            None => return 0.0,
        };
        // Only whitespace gets stripped by pulldown-cmark from paragraph text;
        // bail out for any other content so we don't misplace the cursor.
        if !stripped.chars().all(|c| c == ' ' || c == '\t') {
            return 0.0;
        }
        let fl = match lazy.flat_lines.get(flat_idx) {
            Some(fl) => fl,
            None => return 0.0,
        };
        if stripped.is_empty() || fl.text.ends_with(stripped) {
            return 0.0;
        }
        // Prefer the shaped space advance from the flat_line if a trailing space
        // survived shaping; otherwise fall back to the same estimate the wrap
        // pipeline uses.
        let per_space = fl
            .shaped
            .as_ref()
            .and_then(|shaped| {
                shaped.clusters.iter().rev().find_map(|c| {
                    let bytes = fl.text.as_bytes().get(c.byte_range.clone())?;
                    (bytes == b" ").then_some(c.advance)
                })
            })
            .unwrap_or(fl.font_size * 0.3);
        stripped.chars().count() as f32 * per_space
    }

    fn visual_cursor_screen_pos(&self) -> Option<(f32, f32, f32, f32)> {
        let ctx = self.edit_ctx.as_ref()?;
        let preedit_text = ctx.preedit_text.as_deref().filter(|text| !text.is_empty())?;
        let lazy = self.lazy.as_ref()?;
        let source = self.edit_source.as_ref()?;
        let source_line = source_line_at_byte(source, ctx.cursor_byte)?;
        if source_line.is_empty() {
            if self.empty_source_line_role(source_line, lazy, source)
                == EmptySourceLineRole::HiddenBlockSeparator
            {
                return None;
            }
            return self.empty_source_line_preedit_cursor_screen_pos(
                source_line,
                preedit_text,
                ctx.preedit_cursor,
            );
        }

        let preedit_cursor_grapheme =
            preedit_cursor_grapheme_index(preedit_text, ctx.preedit_cursor);
        let virtual_position = lazy
            .source_projection_index
            .as_ref()?
            .virtual_position_for_source(ctx.cursor_byte, preedit_cursor_grapheme);
        let Some(virtual_position) = virtual_position else {
            let (x, y, width, height) = self.cursor_screen_pos_for_byte(ctx.cursor_byte)?;
            return Some((
                x + self.standalone_preedit_cursor_advance.unwrap_or_default(),
                y,
                width,
                height,
            ));
        };
        let flat_idx = lazy.flat_line_idx_for_projection(virtual_position.flat_line_idx)?;
        let fl = lazy.flat_lines.get(flat_idx)?;
        let x = fl.rect.x
            + crate::layout::grapheme_x(fl, virtual_position.grapheme_pos)
            + self.trailing_stripped_space_advance(flat_idx, ctx.cursor_byte);
        let cursor_height = fl.font_size.min(fl.rect.h);
        let text_baseline_y = fl.rect.y + fl.font_size - self.scroll_y;
        let cursor_y = text_baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO;
        Some((x, cursor_y, 2.0, cursor_height))
    }

    fn empty_source_line_preedit_cursor_screen_pos(
        &self,
        source_line: SourceLineAtByte,
        preedit_text: &str,
        preedit_cursor: Option<(usize, usize)>,
    ) -> Option<(f32, f32, f32, f32)> {
        let source = self.edit_source.as_ref()?;
        let lazy = self.lazy.as_ref()?;
        let (x, line_top, font_size, line_height) =
            self.empty_source_line_metrics(source_line, lazy, source);
        let cursor_height = font_size.min(line_height);
        let cursor_grapheme = preedit_cursor_grapheme_index(preedit_text, preedit_cursor);
        let fallback_advance = cursor_grapheme as f32 * font_size * 0.3;
        let cursor_x = x + self.standalone_preedit_cursor_advance.unwrap_or(fallback_advance);
        let baseline_y = line_top + cursor_height;
        let cursor_y = baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO - self.scroll_y;
        Some((cursor_x, cursor_y, 2.0, cursor_height))
    }

    fn standalone_preedit_render_data(&self) -> Option<StandalonePreeditRenderData<'_>> {
        let ctx = self.edit_ctx.as_ref()?;
        let preedit_text = ctx.preedit_text.as_deref().filter(|text| !text.is_empty())?;
        let lazy = self.lazy.as_ref()?;
        let source = self.edit_source.as_deref()?;
        let source_line = source_line_at_byte(source, ctx.cursor_byte)?;
        if !source_line.is_empty() {
            let preedit_cursor_grapheme =
                preedit_cursor_grapheme_index(preedit_text, ctx.preedit_cursor);
            if lazy
                .source_projection_index
                .as_ref()?
                .virtual_position_for_source(ctx.cursor_byte, preedit_cursor_grapheme)
                .is_some()
            {
                return None;
            }
            let (x, cursor_y, _, cursor_height) =
                self.cursor_screen_pos_for_byte(ctx.cursor_byte)?;
            return Some(StandalonePreeditRenderData {
                text: preedit_text,
                cursor: ctx.preedit_cursor,
                x,
                baseline_y: cursor_y + cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO,
                font_size: cursor_height,
            });
        }
        if self.empty_source_line_role(source_line, lazy, source)
            == EmptySourceLineRole::HiddenBlockSeparator
        {
            return None;
        }

        let (x, line_top, font_size, _) = self.empty_source_line_metrics(source_line, lazy, source);
        Some(StandalonePreeditRenderData {
            text: preedit_text,
            cursor: ctx.preedit_cursor,
            x,
            baseline_y: line_top + font_size - self.scroll_y,
            font_size,
        })
    }

    fn set_standalone_preedit_cursor_advance(&mut self, advance: f32) {
        self.standalone_preedit_cursor_advance = Some(advance);
    }

    fn empty_source_line_cursor_screen_pos(
        &self,
        cursor_byte: usize,
    ) -> Option<(f32, f32, f32, f32)> {
        let source = self.edit_source.as_ref()?;
        let source_line = source_line_at_byte(source, cursor_byte)?;
        if !source_line.is_empty() {
            return None;
        }

        let lazy = self.lazy.as_ref()?;
        if self.empty_source_line_role(source_line, lazy, source)
            == EmptySourceLineRole::HiddenBlockSeparator
        {
            return None;
        }

        let (x, line_top, font_size, line_height) =
            self.empty_source_line_metrics(source_line, lazy, source);
        let cursor_height = font_size.min(line_height);
        let baseline_y = line_top + cursor_height;
        let cursor_y = baseline_y - cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO - self.scroll_y;
        Some((x, cursor_y, 2.0, cursor_height))
    }

    fn empty_source_line_metrics(
        &self,
        source_line: SourceLineAtByte,
        lazy: &LazyLayout<S>,
        source: &str,
    ) -> (f32, f32, f32, f32) {
        if let Some(position) = lazy.source_projection_index.as_ref().and_then(|index| {
            index.visual_position_for_source(source_line.start, CursorAffinity::Downstream)
        }) && let Some(empty_line) =
            lazy.projected_empty_line_for_projection(position.flat_line_idx)
            && empty_line.source_byte == source_line.start
        {
            let (x, font_size, line_height) = self.empty_source_line_typography(source_line, lazy);
            return (x, empty_line.y_top, font_size, line_height);
        }

        // 块内部空行自带渲染行：直接使用该渲染行几何，保证光标落在看到的空行上。
        if let Some(own_line) = self.own_rendered_line(source_line, lazy) {
            return (own_line.rect.x, own_line.rect.y, own_line.font_size, own_line.rect.h);
        }

        let (previous_line, next_line) = self.surrounding_rendered_lines(source_line, lazy);

        if let (Some(previous), Some(next)) = (previous_line, next_line)
            && let Some(metrics) =
                self.empty_source_line_metrics_between(source_line, source, previous, next)
        {
            return metrics;
        }

        if let Some((previous_byte, previous_flat_line)) = previous_line {
            let newline_count = count_newlines_between(source, previous_byte, source_line.start);
            let gap = if newline_count <= 1 {
                0.0
            } else if newline_count == 2 {
                self.paragraph_spacing
            } else {
                self.paragraph_spacing + (newline_count - 2) as f32 * previous_flat_line.rect.h
            };
            return (
                previous_flat_line.rect.x,
                previous_flat_line.rect.y + previous_flat_line.rect.h + gap,
                previous_flat_line.font_size,
                previous_flat_line.rect.h,
            );
        }

        if let Some((next_byte, next_flat_line)) = next_line {
            let newline_count = count_newlines_between(source, source_line.end, next_byte);
            let gap = if newline_count <= 1 {
                0.0
            } else if newline_count == 2 {
                self.paragraph_spacing
            } else {
                self.paragraph_spacing + (newline_count - 2) as f32 * next_flat_line.rect.h
            };
            return (
                next_flat_line.rect.x,
                next_flat_line.rect.y - next_flat_line.rect.h - gap,
                next_flat_line.font_size,
                next_flat_line.rect.h,
            );
        }

        (
            0.0,
            self.rendered_line_height * source_line.index as f32,
            self.rendered_body_font_size,
            self.rendered_line_height,
        )
    }

    fn empty_source_line_typography(
        &self,
        source_line: SourceLineAtByte,
        lazy: &LazyLayout<S>,
    ) -> (f32, f32, f32) {
        let (previous_line, next_line) = self.surrounding_rendered_lines(source_line, lazy);

        if let Some((_, previous_flat_line)) = previous_line {
            return (
                previous_flat_line.rect.x,
                previous_flat_line.font_size,
                previous_flat_line.rect.h,
            );
        }

        if let Some((_, next_flat_line)) = next_line {
            return (next_flat_line.rect.x, next_flat_line.font_size, next_flat_line.rect.h);
        }

        (0.0, self.rendered_body_font_size, self.rendered_line_height)
    }

    /// 空源码行自身的渲染行（代码块/metadata 块会为内部空行生成投影）。
    /// 判定依据：投影的源码范围完整落在该空行的源码区间内。
    fn own_rendered_line<'a>(
        &self,
        source_line: SourceLineAtByte,
        lazy: &'a LazyLayout<S>,
    ) -> Option<&'a crate::layout::FlatLine> {
        lazy.flat_lines.iter().find(|flat_line| {
            flat_line.source_projection.as_ref().is_some_and(|projection| {
                projection.source_extent.start >= source_line.start
                    && projection.source_extent.end <= source_line.end
            })
        })
    }

    fn surrounding_rendered_lines<'a>(
        &self,
        source_line: SourceLineAtByte,
        lazy: &'a LazyLayout<S>,
    ) -> SurroundingRenderedLines<'a> {
        let mut previous_line = None;
        let mut next_line = None;

        for flat_line in &lazy.flat_lines {
            let Some(projection) = flat_line.source_projection.as_ref() else {
                continue;
            };
            let Some(line_start_byte) = projection.boundaries.first().map(|anchor| anchor.byte)
            else {
                continue;
            };
            let Some(line_end_byte) = projection.boundaries.last().map(|anchor| anchor.byte) else {
                continue;
            };

            if line_end_byte <= source_line.start {
                previous_line = Some((line_end_byte, flat_line));
            }

            if line_start_byte >= source_line.end {
                next_line = Some((line_start_byte, flat_line));
                break;
            }
        }

        (previous_line, next_line)
    }

    fn empty_source_line_metrics_between(
        &self,
        source_line: SourceLineAtByte,
        source: &str,
        previous_line: (usize, &crate::layout::FlatLine),
        next_line: (usize, &crate::layout::FlatLine),
    ) -> Option<(f32, f32, f32, f32)> {
        let (previous_byte, previous_flat_line) = previous_line;
        let (next_byte, next_flat_line) = next_line;
        let (empty_line_index, empty_line_count) =
            empty_source_line_rank(source, previous_byte, next_byte, source_line)?;
        let gap_top = previous_flat_line.rect.y + previous_flat_line.rect.h;
        let gap_height = next_flat_line.rect.y - gap_top;
        if gap_height <= 0.0 {
            return None;
        }

        let editable_line_height = previous_flat_line.rect.h;
        let editable_line_count = empty_line_count.saturating_sub(1);
        let separator_height =
            (gap_height - editable_line_height * editable_line_count as f32).max(0.0);
        let (line_top, line_height) = if empty_line_index == 0 {
            (gap_top, separator_height)
        } else {
            (
                gap_top + separator_height + editable_line_height * (empty_line_index - 1) as f32,
                editable_line_height,
            )
        };
        Some((previous_flat_line.rect.x, line_top, previous_flat_line.font_size, line_height))
    }

    /// 扩展被 Markdown 布局剥离的行尾空格，令其光标仍归属原文本行。
    fn hit_test_line_right(&self, flat_line: &crate::layout::FlatLine) -> f32 {
        let text_right = flat_line.rect.x
            + crate::layout::grapheme_x(
                flat_line,
                crate::grapheme_map::grapheme_count(&flat_line.text),
            );
        let trailing_space_advance = flat_line
            .source_projection
            .as_ref()
            .and_then(|projection| projection.boundaries.last())
            .map_or(0.0, |anchor| {
                let trailing_space_end =
                    self.edit_source.as_deref().map_or(anchor.byte, |source| {
                        source.get(anchor.byte..).map_or(anchor.byte, |suffix| {
                            anchor.byte
                                + suffix
                                    .chars()
                                    .take_while(|character| matches!(character, ' ' | '\t'))
                                    .map(char::len_utf8)
                                    .sum::<usize>()
                        })
                    });
                self.trailing_stripped_space_advance(flat_line.flat_idx, trailing_space_end)
            });
        (flat_line.rect.x + flat_line.rect.w).max(text_right) + trailing_space_advance
    }

    /// 屏幕坐标 → 源码字节偏移。用于鼠标点击。
    /// 输入 y/offset_y 为插件渲染空间坐标；内部加回 scroll_y 以匹配
    /// flat_line 的文档绝对 y（与 render_line_with_offset 的投影互逆）。
    pub fn hit_test_byte(&self, x: f32, y: f32, offset_x: f32, offset_y: f32) -> Option<usize> {
        let lazy = self.lazy.as_ref()?;
        let doc_x = x - offset_x;
        let doc_y = y - offset_y + self.scroll_y;

        let same_row = |flat_line: &&crate::layout::FlatLine| {
            doc_y >= flat_line.rect.y && doc_y <= flat_line.rect.y + flat_line.rect.h
        };
        let horizontal_distance = |flat_line: &&crate::layout::FlatLine| {
            let left = flat_line.rect.x;
            let right = self.hit_test_line_right(flat_line);
            if doc_x < left {
                left - doc_x
            } else if doc_x > right {
                doc_x - right
            } else {
                0.0
            }
        };
        let flat_line = lazy
            .flat_lines
            .iter()
            .filter(|flat_line| same_row(flat_line))
            .find(|flat_line| {
                doc_x >= flat_line.rect.x && doc_x <= self.hit_test_line_right(flat_line)
            })
            .or_else(|| {
                lazy.flat_lines.iter().filter(|flat_line| same_row(flat_line)).min_by(
                    |left, right| {
                        horizontal_distance(left)
                            .total_cmp(&horizontal_distance(right))
                            .then_with(|| right.rect.x.total_cmp(&left.rect.x))
                    },
                )
            });

        if let Some(flat_line) = flat_line {
            let line_x = doc_x - flat_line.rect.x;
            let char_offset = crate::layout::grapheme_at_x(flat_line, line_x);
            return self.byte_from_flat_line_and_visual_grapheme(flat_line.flat_idx, char_offset);
        }

        if let Some(byte) = self.visible_empty_source_line_byte_at_doc_y(doc_y, lazy) {
            return Some(byte);
        }

        let above = lazy.flat_lines.iter().rfind(|fl| fl.rect.y + fl.rect.h <= doc_y);
        let below = lazy.flat_lines.iter().find(|fl| fl.rect.y > doc_y);

        if let (Some(above), Some(below)) = (above, below) {
            let above_bottom = above.rect.y + above.rect.h;
            let gap_mid_y = (above_bottom + below.rect.y) * 0.5;
            let target = if doc_y < gap_mid_y { above } else { below };
            return if doc_y < gap_mid_y {
                self.byte_from_flat_line_and_visual_grapheme(
                    target.flat_idx,
                    crate::grapheme_map::grapheme_count(&target.text),
                )
            } else {
                self.byte_from_flat_line_and_visual_grapheme(target.flat_idx, 0)
            };
        }

        // Click landed below the final flat line. Only snap when the gap is
        // reasonable (≤ 3× line height); clicks far outside content return None.
        if let Some(above) = above {
            let gap = doc_y - (above.rect.y + above.rect.h);
            if gap > above.rect.h * HIT_TEST_SNAP_MAX_LINE_HEIGHTS {
                return None; // far below content
            }
            return self.byte_from_flat_line_and_visual_grapheme(
                above.flat_idx,
                crate::grapheme_map::grapheme_count(&above.text),
            );
        }

        // No line above — click above first line; snap to start of first line.
        if let Some(below) = lazy.flat_lines.first() {
            let gap = below.rect.y - doc_y;
            if gap > below.rect.h * HIT_TEST_SNAP_MAX_LINE_HEIGHTS {
                return None; // far above content
            }
            return self.byte_from_flat_line_and_visual_grapheme(below.flat_idx, 0);
        }

        None
    }

    fn visible_empty_source_line_byte_at_doc_y(
        &self,
        doc_y: f32,
        lazy: &LazyLayout<S>,
    ) -> Option<usize> {
        let source = self.edit_source.as_ref()?;
        let map = self.source_line_map.as_ref()?;
        for span in map.lines().iter().filter(|line| line.is_empty()).copied() {
            let source_line = source_line_span_to_at_byte(span);
            if self.empty_source_line_role(source_line, lazy, source)
                == EmptySourceLineRole::HiddenBlockSeparator
            {
                continue;
            }
            let (_x, line_top, _font_size, line_height) =
                self.empty_source_line_metrics(source_line, lazy, source);
            if doc_y >= line_top && doc_y < line_top + line_height {
                return Some(source_line.start);
            }
        }
        None
    }

    fn byte_from_flat_line_and_visual_grapheme(
        &self,
        flat_line_idx: usize,
        visual_grapheme: usize,
    ) -> Option<usize> {
        let lazy = self.lazy.as_ref()?;
        let index = lazy.source_projection_index.as_ref()?;
        let projection_flat_line_idx =
            lazy.projection_visual_line_idx_for_flat_line(flat_line_idx)?;
        let position = VisualPosition {
            layout_revision: index.layout_revision(),
            flat_line_idx: projection_flat_line_idx,
            grapheme_pos: visual_grapheme,
        };
        index.source_anchor_at(self.source_generation, position).ok().map(|anchor| anchor.byte)
    }

    /// 找到 byte 所在的 flat_line 索引和该 byte 在行内的 x 像素位置。
    /// Uses canonical source projections to find the correct wrapped segment.
    fn flat_line_and_x_for_byte(&self, byte: usize) -> Option<(usize, f32)> {
        let lazy = self.lazy.as_ref()?;
        let (flat_idx, visual_grapheme) = self.find_flat_and_grapheme_for_byte(byte)?;
        let fl = lazy.flat_lines.get(flat_idx)?;
        let x = crate::layout::grapheme_x(fl, visual_grapheme);
        Some((flat_idx, x))
    }

    fn find_flat_and_grapheme_for_byte(&self, byte: usize) -> Option<(usize, usize)> {
        let position = self.cursor_visual_position_for_byte(byte, CursorAffinity::Downstream)?;
        let flat_line_idx =
            self.lazy.as_ref()?.flat_line_idx_for_projection(position.flat_line_idx)?;
        Some((flat_line_idx, position.grapheme_pos))
    }

    fn visual_range_for_byte_selection(&self) -> Option<(ViewPos, ViewPos)> {
        let (start_byte, end_byte) = self.byte_selection_range()?;
        let start = self.selection_position_for_byte(start_byte, false)?;
        let end = self.selection_position_for_byte(end_byte, true)?;
        let start = ViewPos { flat_line_idx: start.0, grapheme_pos: start.1 };
        let end = ViewPos { flat_line_idx: end.0, grapheme_pos: end.1 };
        if (start.flat_line_idx, start.grapheme_pos) <= (end.flat_line_idx, end.grapheme_pos) {
            Some((start, end))
        } else {
            Some((end, start))
        }
    }

    fn selection_position_for_byte(
        &self,
        source_byte: usize,
        prefer_previous_rendered_line: bool,
    ) -> Option<(usize, usize)> {
        let lazy = self.lazy.as_ref()?;
        let index = lazy.source_projection_index.as_ref()?;
        let position = index.visual_position_for_source(source_byte, CursorAffinity::Downstream)?;
        if let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(position.flat_line_idx) {
            return Some((flat_line_idx, position.grapheme_pos));
        }

        if prefer_previous_rendered_line {
            for projection_idx in (0..position.flat_line_idx).rev() {
                let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(projection_idx) else {
                    continue;
                };
                let grapheme_pos =
                    index.visual_lines()[projection_idx].boundaries.len().saturating_sub(1);
                return Some((flat_line_idx, grapheme_pos));
            }
            return None;
        }

        for projection_idx in position.flat_line_idx.saturating_add(1)..index.visual_lines().len() {
            if let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(projection_idx) {
                return Some((flat_line_idx, 0));
            }
        }
        None
    }

    /// 视觉方向导航。Uses canonical source projections when translating between visual rows and
    /// source bytes.
    pub fn visual_move(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<usize> {
        use ui::plugin::MoveDirection;
        let lazy = self.lazy.as_ref()?;
        let source = self.edit_source.as_ref();
        let current_source_line = source.and_then(|text| source_line_at_byte(text, current_byte));

        if matches!(
            direction,
            MoveDirection::Left | MoveDirection::Right | MoveDirection::Up | MoveDirection::Down
        ) && current_source_line.is_some_and(|source_line| {
            source.is_some_and(|source| {
                self.empty_source_line_role(source_line, lazy, source)
                    == EmptySourceLineRole::HiddenBlockSeparator
            })
        }) {
            return self.move_from_hidden_block_separator(current_byte, direction);
        }

        let index = lazy.source_projection_index.as_ref()?;

        match direction {
            MoveDirection::Left => {
                if current_byte == 0 {
                    return Some(0);
                }
                if let Some(anchor) =
                    index.move_horizontal(current_byte, HorizontalDirection::Previous)
                {
                    return Some(anchor.byte);
                }
                Some(0)
            }
            MoveDirection::Right => {
                if let Some(anchor) = index.move_horizontal(current_byte, HorizontalDirection::Next)
                {
                    return Some(anchor.byte);
                }
                Some(current_byte)
            }
            MoveDirection::Up | MoveDirection::Down => self.visual_move_in_projection_sequence(
                lazy,
                index,
                current_byte,
                direction,
                target_x,
            ),
            MoveDirection::LineStart => {
                if let Some(line) = current_source_line
                    && line.is_empty()
                {
                    return Some(line.start);
                }
                index.line_boundary(current_byte, LineBoundary::Start).map(|anchor| anchor.byte)
            }
            MoveDirection::LineEnd => {
                if let Some(line) = current_source_line
                    && line.is_empty()
                {
                    return Some(line.start);
                }
                index.line_boundary(current_byte, LineBoundary::End).map(|anchor| anchor.byte)
            }
        }
    }

    fn visual_move_in_projection_sequence(
        &self,
        lazy: &LazyLayout<S>,
        index: &SourceProjectionIndex,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
        target_x: Option<f32>,
    ) -> Option<usize> {
        let current_position =
            self.projection_position_for_vertical_move(lazy, index, current_byte)?;
        let target_visual_line_idx = match direction {
            ui::plugin::MoveDirection::Up => {
                let Some(target) = current_position.flat_line_idx.checked_sub(1) else {
                    return Some(0);
                };
                target
            }
            ui::plugin::MoveDirection::Down => {
                let Some(target) = current_position.flat_line_idx.checked_add(1) else {
                    return Some(current_byte);
                };
                if target >= index.visual_lines().len() {
                    return self.edit_source.as_deref().map(str::len);
                }
                target
            }
            _ => return None,
        };
        let screen_x = match target_x {
            Some(screen_x) => screen_x,
            None => self.projection_screen_x(lazy, current_position)?,
        };
        let target_grapheme =
            self.projection_grapheme_at_screen_x(lazy, target_visual_line_idx, screen_x)?;
        let target_position = VisualPosition {
            layout_revision: index.layout_revision(),
            flat_line_idx: target_visual_line_idx,
            grapheme_pos: target_grapheme,
        };
        index
            .source_anchor_at(self.source_generation, target_position)
            .ok()
            .map(|anchor| anchor.byte)
    }

    fn projection_position_for_vertical_move(
        &self,
        lazy: &LazyLayout<S>,
        index: &SourceProjectionIndex,
        current_byte: usize,
    ) -> Option<VisualPosition> {
        let source_line = self
            .edit_source
            .as_deref()
            .and_then(|source| source_line_at_byte(source, current_byte));
        if let Some(source_line) = source_line.filter(|line| line.is_empty())
            && let Some(visual_line_idx) =
                lazy.projection_visual_line_idx_for_empty_source_byte(source_line.start)
        {
            return Some(VisualPosition {
                layout_revision: index.layout_revision(),
                flat_line_idx: visual_line_idx,
                grapheme_pos: 0,
            });
        }

        index.visual_position_for_source(current_byte, CursorAffinity::Downstream)
    }

    fn projection_screen_x(&self, lazy: &LazyLayout<S>, position: VisualPosition) -> Option<f32> {
        if let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(position.flat_line_idx) {
            let line = lazy.flat_lines.get(flat_line_idx)?;
            return Some(line.rect.x + self.grapheme_x_for_line(line, position.grapheme_pos));
        }

        let empty_line = lazy.projected_empty_line_for_projection(position.flat_line_idx)?;
        let source = self.edit_source.as_deref()?;
        let source_line = source_line_at_byte(source, empty_line.source_byte)?;
        let (x, _, _) = self.empty_source_line_typography(source_line, lazy);
        Some(x)
    }

    fn projection_grapheme_at_screen_x(
        &self,
        lazy: &LazyLayout<S>,
        visual_line_idx: usize,
        screen_x: f32,
    ) -> Option<usize> {
        if let Some(flat_line_idx) = lazy.flat_line_idx_for_projection(visual_line_idx) {
            let line = lazy.flat_lines.get(flat_line_idx)?;
            return Some(self.grapheme_at_x_for_line(line, screen_x - line.rect.x));
        }

        lazy.projected_empty_line_for_projection(visual_line_idx).map(|_| 0)
    }

    fn move_from_hidden_block_separator(
        &self,
        current_byte: usize,
        direction: ui::plugin::MoveDirection,
    ) -> Option<usize> {
        let index = self.lazy.as_ref()?.source_projection_index.as_ref()?;
        match direction {
            ui::plugin::MoveDirection::Left => {
                index.line_boundary(current_byte, LineBoundary::End).map(|anchor| anchor.byte)
            }
            ui::plugin::MoveDirection::Up | ui::plugin::MoveDirection::LineStart => {
                index.line_boundary(current_byte, LineBoundary::Start).map(|anchor| anchor.byte)
            }
            ui::plugin::MoveDirection::Right | ui::plugin::MoveDirection::Down => index
                .move_horizontal(current_byte, HorizontalDirection::Next)
                .map(|anchor| anchor.byte),
            ui::plugin::MoveDirection::LineEnd => {
                index.line_boundary(current_byte, LineBoundary::End).map(|anchor| anchor.byte)
            }
        }
    }

    fn empty_source_line_role(
        &self,
        source_line: SourceLineAtByte,
        lazy: &LazyLayout<S>,
        _source: &str,
    ) -> EmptySourceLineRole {
        if !source_line.is_empty() {
            return EmptySourceLineRole::EditableLine;
        }
        // 块内部空行（代码块/metadata 块）自带渲染投影，属于块内容而非块间分隔。
        if self.own_rendered_line(source_line, lazy).is_some() {
            return EmptySourceLineRole::EditableLine;
        }
        let (previous_line, next_line) = self.surrounding_rendered_lines(source_line, lazy);
        // HiddenBlockSeparator 只在"上下都有渲染块"时生效——首行/末行的空行仍视作可编辑。
        if previous_line.is_none() || next_line.is_none() {
            return EmptySourceLineRole::EditableLine;
        }
        let Some(pos) =
            self.source_line_map.as_ref().and_then(|map| map.empty_run_position(source_line.index))
        else {
            return EmptySourceLineRole::EditableLine;
        };
        if pos.index_in_run == 0 {
            EmptySourceLineRole::HiddenBlockSeparator
        } else {
            EmptySourceLineRole::EditableLine
        }
    }

    /// 编辑干预：Enter/Backspace/Tab/InsertText 的 markdown 感知行为。委派到
    /// [`crate::augmenter`]（2026-07-06 方案阶段 2b/2c 抽出）。
    pub fn augment_edit(
        &self,
        current_byte: usize,
        kind: ui::plugin::AugmentKind,
    ) -> Option<ui::plugin::EditAugmentation> {
        let source = self.edit_source.as_deref()?;
        crate::augmenter::augment_edit(source, current_byte, kind)
    }

    /// 判断光标是否在某个 span 内。
    pub fn cursor_in_span(&self, span: &crate::builder::StyleSpan) -> bool {
        let Some(ctx) = self.edit_ctx.as_ref() else {
            return false;
        };
        crate::edit::cursor_in_span(span, ctx.cursor_byte)
    }

    // ── Common query/message helpers ──

    /// Handle queries common to all markdown views (preview + editor).
    /// Returns `Some(response)` for recognized queries, `None` for view-specific ones.
    pub fn query_common(&self, q: &PluginQuery) -> Option<PluginResponse> {
        match q {
            PluginQuery::ScrollY => Some(PluginResponse::Float(self.scroll_y)),
            PluginQuery::ContentHeight => Some(PluginResponse::Float(self.content_height)),
            PluginQuery::NeedsSourceUpdate(_) => None,
            PluginQuery::TOCHeadings => Some(PluginResponse::Headings(
                self.headings()
                    .iter()
                    .map(|h| ui::plugin::HeadingEntry {
                        title: h.text.clone(),
                        y: h.y_offset,
                        level: h.level,
                    })
                    .collect(),
            )),
            PluginQuery::CurrentHeadingIndex(scroll_y) => Some(PluginResponse::Position(
                self.current_heading_index(*scroll_y).map(|i| (i, 0)),
            )),
            PluginQuery::HasSelection => Some(PluginResponse::Bool(self.has_selection())),
            PluginQuery::SelectedText => {
                Some(PluginResponse::String(self.selected_text().unwrap_or_default()))
            }
            PluginQuery::SelCursor => Some(PluginResponse::Position(
                self.sel.cursor.map(|p| (p.flat_line_idx, p.grapheme_pos)),
            )),
            PluginQuery::SelectionRange => Some(PluginResponse::PositionPair(
                self.selection_source_range().map(|(start, end)| ((start, 0), (end, 0))),
            )),
            PluginQuery::HitTest { x, y, offset_x, offset_y } => Some(PluginResponse::Position(
                self.hit_test(*x, *y, *offset_x, *offset_y)
                    .map(|p| (p.flat_line_idx, p.grapheme_pos)),
            )),
            PluginQuery::SelectionHighlights(color) => {
                Some(PluginResponse::DrawList(self.selection_highlights(*color)))
            }
            PluginQuery::FlatLines => Some(PluginResponse::FlatLines(
                self.flat_lines()
                    .iter()
                    .map(|line| {
                        let grapheme_count = crate::grapheme_map::grapheme_count(&line.text);
                        ui::plugin::FlatLine {
                            text: line.text.clone(),
                            grapheme_count,
                            rect: line.rect,
                            grapheme_x: (0..=grapheme_count)
                                .map(|grapheme| crate::layout::grapheme_x(line, grapheme))
                                .collect(),
                        }
                    })
                    .collect(),
            )),
            PluginQuery::WordAtPos(li, gp) => {
                let (s, e) = self.word_at_pos(ViewPos { flat_line_idx: *li, grapheme_pos: *gp });
                Some(PluginResponse::PositionPair(Some((
                    (s.flat_line_idx, s.grapheme_pos),
                    (e.flat_line_idx, e.grapheme_pos),
                ))))
            }
            PluginQuery::LineRangeAtPos(li, gp) => {
                let (s, e) =
                    self.line_range_at_pos(ViewPos { flat_line_idx: *li, grapheme_pos: *gp });
                Some(PluginResponse::PositionPair(Some((
                    (s.flat_line_idx, s.grapheme_pos),
                    (e.flat_line_idx, e.grapheme_pos),
                ))))
            }
            PluginQuery::CursorScreenPos(byte) => Some(PluginResponse::CursorScreenRect(
                self.edit_ctx
                    .as_ref()
                    .filter(|ctx| ctx.cursor_byte == *byte)
                    .and_then(|_| self.visual_cursor_screen_pos())
                    .or_else(|| self.cursor_screen_pos_for_byte(*byte)),
            )),
            PluginQuery::HitTestByte { x, y, offset_x, offset_y } => {
                Some(PluginResponse::BytePosition(self.hit_test_byte(*x, *y, *offset_x, *offset_y)))
            }
            PluginQuery::VisualMove { current_byte, direction, target_x } => {
                Some(PluginResponse::BytePosition(self.visual_move(
                    *current_byte,
                    *direction,
                    *target_x,
                )))
            }

            _ => None,
        }
    }

    /// Handle messages common to all markdown views.
    /// Returns `Some(result)` for recognized messages, `None` for view-specific ones.
    pub fn handle_message_common(&mut self, msg: &PluginMessage) -> Option<bool> {
        match msg {
            PluginMessage::Scroll { delta, viewport_h } => Some(self.scroll(*delta, *viewport_h)),
            PluginMessage::ScrollToHeading(index) => {
                self.scroll_to_heading(*index);
                Some(true)
            }
            PluginMessage::UpdateSource { .. } => None,
            PluginMessage::SetSelCursor(pos) => {
                self.set_sel_cursor(*pos);
                Some(true)
            }
            PluginMessage::SetSelAnchor(pos) => {
                self.set_sel_anchor(*pos);
                Some(true)
            }
            PluginMessage::SetSelCursorByte(byte) => {
                self.set_sel_cursor_byte(*byte);
                Some(true)
            }
            PluginMessage::SetSelAnchorByte(byte) => {
                self.set_sel_anchor_byte(*byte);
                Some(true)
            }
            PluginMessage::ClearSelection => {
                self.clear_selection();
                Some(true)
            }
            PluginMessage::SelectAll => {
                self.select_all();
                Some(true)
            }
            PluginMessage::SetRenderSettings { font_size, line_height, toc_max_depth } => {
                self.base_font_size = *font_size;
                self.base_line_height = *line_height;
                self.toc_max_depth = *toc_max_depth;
                Some(true)
            }
            PluginMessage::SetCursorByte(byte) => {
                self.handle_set_cursor_byte(*byte);
                Some(true)
            }
            PluginMessage::SetPreedit { text, cursor } => {
                self.set_preedit_text(text.clone(), *cursor);
                Some(true)
            }
            PluginMessage::SetCursorVisible(visible) => {
                self.cursor_visible = *visible;
                Some(true)
            }
            _ => None,
        }
    }
}

// ===== MarkdownView =====

/// Markdown preview view — wraps PreviewEngine with markdown-specific doc building.
pub struct MarkdownView {
    engine: PreviewEngine,
    source: String,
    cached_source_hash: u64,
    cached_generation: u32,
}

impl Default for MarkdownView {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownView {
    pub fn new() -> Self {
        Self {
            engine: PreviewEngine::new(),
            source: String::new(),
            cached_source_hash: 0,
            cached_generation: 0,
        }
    }

    pub fn set_source(&mut self, text: String, generation: u32) {
        let hash = fxhash(&text);
        if hash != self.cached_source_hash {
            self.source = text;
            self.cached_source_hash = hash;
            self.engine.mark_source_dirty();
        }
        self.engine.set_source_generation(generation);
        self.cached_generation = generation;
    }

    pub fn needs_source_update(&self, generation: u32) -> bool {
        generation != self.cached_generation
    }

    fn build_doc(&self, style: &MarkdownStyle) -> crate::builder::MarkdownDoc {
        let parsed = crate::parser::parse_markdown(&self.source);
        crate::builder::MarkdownDoc::build(&parsed, style)
    }

    pub fn render(
        &mut self,
        theme: &Theme,
        viewport_w: f32,
        viewport_h: f32,
        offset_x: f32,
        offset_y: f32,
        settings: MarkdownRenderSettings,
        shaper: Option<&mut shaping::Shaper>,
    ) -> (DrawList, bool) {
        let style = settings.style(theme);
        self.engine.toc_max_depth = settings.toc_max_depth;
        let source = &self.source;
        let engine = &mut self.engine;
        let string_doc = core::document::StringDocView::new(source);
        engine.render(
            theme,
            viewport_w,
            viewport_h,
            offset_x,
            offset_y,
            &style,
            |s| {
                let parsed = crate::parser::parse_markdown(source);
                crate::builder::MarkdownDoc::build(&parsed, s)
            },
            shaper,
            &string_doc,
            true,  // preview still needs whole-document flat lines for selection/copy.
            false, // precise shaping/highlighting stays viewport-driven for responsiveness.
        )
    }

    pub fn engine(&self) -> &PreviewEngine {
        &self.engine
    }
    pub fn engine_mut(&mut self) -> &mut PreviewEngine {
        &mut self.engine
    }
    pub fn scroll_y(&self) -> f32 {
        self.engine.scroll_y
    }
    pub fn headings(&self) -> &[HeadingEntry] {
        self.engine.headings()
    }
}

impl ViewPlugin for MarkdownView {
    fn name(&self) -> &str {
        "markdown_view"
    }
    fn allows_editing(&self) -> bool {
        false
    }
    fn shows_cursor(&self) -> bool {
        false
    }
    fn shows_gutter(&self) -> bool {
        false
    }

    fn render(
        &mut self,
        _doc: &dyn DocView,
        bounds: ui::core::geom::Rect,
        theme: &Theme,
        shaper: &mut shaping::Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let settings = MarkdownRenderSettings {
            font_size: self.engine.base_font_size * dpi_scale,
            line_height: self.engine.base_line_height * dpi_scale,
            toc_max_depth: self.engine.toc_max_depth,
        };
        let (dl, _) =
            self.render(theme, bounds.w, bounds.h, bounds.x, bounds.y, settings, Some(shaper));
        dl
    }

    fn handle_message(
        &mut self,
        msg: PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        if let Some(result) = self.engine.handle_message_common(&msg) {
            return result;
        }
        match msg {
            PluginMessage::ScrollToSearchMatch { query, match_case, active_idx } => {
                self.engine.scroll_to_search_match(&query, match_case, active_idx);
                true
            }
            PluginMessage::UpdateSource { text, generation } => {
                self.set_source(text, generation);
                true
            }
            _ => false,
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        if let Some(resp) = self.engine.query_common(&q) {
            return resp;
        }
        match q {
            PluginQuery::NeedsSourceUpdate(gen_id) => {
                PluginResponse::Bool(self.needs_source_update(gen_id))
            }
            PluginQuery::SearchHighlights {
                query,
                match_case,
                use_regex,
                active_idx,
                match_color,
                inactive_color,
            } => PluginResponse::DrawList(self.engine.search_highlights(
                &query,
                match_case,
                use_regex,
                active_idx,
                match_color,
                inactive_color,
            )),
            _ => PluginResponse::None,
        }
    }
}

// ===== MarkdownViewFactory =====

pub struct MarkdownViewFactory;
fn is_plain_markdown_path(path: Option<&Path>) -> bool {
    path.and_then(|p| p.to_str()).is_some_and(|s| {
        (s.ends_with(".md") || s.ends_with(".markdown")) && !s.ends_with(".mmap.md")
    })
}

fn source_line_at_byte(source: &str, byte: usize) -> Option<SourceLineAtByte> {
    #[cfg(test)]
    SOURCE_LINE_AT_BYTE_CALLS.fetch_add(1, Ordering::Relaxed);

    let source_bytes = source.as_bytes();
    if byte > source_bytes.len() {
        return None;
    }

    // 单次 lookup 场景仍走原地扫描，避免为一次调用现造 SourceLineMap。
    // 缓存版本由 PreviewEngine::source_line_map 提供（迁移中）。
    let line_start = source_bytes[..byte]
        .iter()
        .rposition(|&source_byte| source_byte == b'\n')
        .map_or(0, |newline_index| newline_index + 1);
    let line_end = source_bytes[line_start..]
        .iter()
        .position(|&source_byte| source_byte == b'\n')
        .map_or(source_bytes.len(), |newline_offset| line_start + newline_offset);
    let line_index =
        source_bytes[..line_start].iter().filter(|&&source_byte| source_byte == b'\n').count();
    let is_blank = source[line_start..line_end].chars().all(char::is_whitespace);

    Some(SourceLineAtByte { index: line_index, start: line_start, end: line_end, is_blank })
}

fn empty_source_line_rank(
    source: &str,
    lower_byte: usize,
    upper_byte: usize,
    source_line: SourceLineAtByte,
) -> Option<(usize, usize)> {
    // 保留旧签名；内部走 SourceLineMap（2026-07-06 方案 1b）。
    // lower_byte/upper_byte 限定"块间空行 run"的字节范围。
    let map = crate::layout::source_line_map::SourceLineMap::from_source(source);
    let empty_lines: Vec<_> = map.empty_lines_in_byte_range(lower_byte..upper_byte).collect();
    let empty_line_index = empty_lines.iter().position(|line| {
        line.index == source_line.index
            && line.start == source_line.start
            && line.end == source_line.end
    })?;
    Some((empty_line_index, empty_lines.len()))
}

fn count_newlines_between(source: &str, start: usize, end: usize) -> usize {
    let start = start.min(source.len());
    let end = end.min(source.len());
    if start >= end {
        return 0;
    }
    source.as_bytes()[start..end].iter().filter(|&&source_byte| source_byte == b'\n').count()
}

// `augment_enter` / `augment_backspace` / `augment_insert_text` 及分类器与
// 各 `*_augmentation` 自由函数已在 2026-07-06 方案阶段 2b/2c 迁至
// `crate::augmenter`。仅测试代码需要重导出 EnterContext / classify_enter_context。
#[cfg(test)]
pub(crate) use crate::augmenter::{EnterContext, classify_enter_context};

impl PluginFactory for MarkdownViewFactory {
    fn name(&self) -> &str {
        "markdown_view"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        is_plain_markdown_path(path)
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownView::new())
    }
}

// ===== NovelView =====

/// Novel reading view — wraps PreviewEngine with txt→NovelStructure conversion.
/// Uses `MarkdownStyle::novel()` for independent theme section.
pub struct NovelView {
    engine: PreviewEngine<crate::builder::NovelStructure>,
}

impl Default for NovelView {
    fn default() -> Self {
        Self::new()
    }
}

impl NovelView {
    pub fn new() -> Self {
        Self { engine: PreviewEngine::new() }
    }

    fn find_chapter_heading<S: BlockSource>(
        engine: &PreviewEngine<S>,
        forward: bool,
    ) -> Option<usize> {
        let cur = engine.current_heading_index(engine.scroll_y).unwrap_or(0);
        let headings = engine.headings();
        if headings.is_empty() {
            return None;
        }
        if forward {
            if cur + 1 < headings.len() { Some(cur + 1) } else { None }
        } else if cur > 0 {
            Some(cur - 1)
        } else if engine.scroll_y > 0.0 {
            Some(0)
        } else {
            None
        }
    }
}

impl ViewPlugin for NovelView {
    fn name(&self) -> &str {
        "novel_view"
    }
    fn allows_editing(&self) -> bool {
        false
    }
    fn shows_cursor(&self) -> bool {
        false
    }
    fn shows_gutter(&self) -> bool {
        false
    }

    fn render(
        &mut self,
        doc: &dyn DocView,
        bounds: ui::core::geom::Rect,
        theme: &Theme,
        shaper: &mut shaping::Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let font_size = self.engine.base_font_size * dpi_scale;
        let line_height = self.engine.base_line_height * dpi_scale;
        let style = MarkdownStyle::novel(theme, font_size, line_height);
        self.engine.toc_max_depth = 3;

        let (dl, _) = self.engine.render(
            theme,
            bounds.w,
            bounds.h,
            bounds.x,
            bounds.y,
            &style,
            |_| crate::builder::NovelStructure::scan(doc),
            Some(shaper),
            doc,
            false,
            false, // preview: viewport-driven layout
        );
        dl
    }

    fn handle_message(
        &mut self,
        msg: PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        match msg {
            PluginMessage::Scroll { delta, viewport_h } => self.engine.scroll(delta, viewport_h),
            PluginMessage::ScrollToHeading(index) => {
                self.engine.scroll_to_heading(index);
                true
            }
            PluginMessage::ScrollToNextChapter => {
                if let Some(idx) = Self::find_chapter_heading(&self.engine, true) {
                    self.engine.scroll_to_heading(idx);
                }
                true
            }
            PluginMessage::ScrollToPrevChapter => {
                if let Some(idx) = Self::find_chapter_heading(&self.engine, false) {
                    self.engine.scroll_to_heading(idx);
                } else {
                    self.engine.scroll_y = 0.0;
                }
                true
            }
            PluginMessage::UpdateSource { .. } => {
                self.engine.mark_source_dirty();
                true
            }
            PluginMessage::SetRenderSettings { font_size, line_height, .. } => {
                self.engine.base_font_size = font_size;
                self.engine.base_line_height = line_height;
                true
            }
            _ => false,
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        match q {
            PluginQuery::ScrollY => PluginResponse::Float(self.engine.scroll_y),
            PluginQuery::ContentHeight => PluginResponse::Float(self.engine.content_height),
            PluginQuery::TOCHeadings => PluginResponse::Headings(
                self.engine
                    .headings()
                    .iter()
                    .map(|h| ui::plugin::HeadingEntry {
                        title: h.text.clone(),
                        y: h.y_offset,
                        level: h.level,
                    })
                    .collect(),
            ),
            PluginQuery::CurrentHeadingIndex(scroll_y) => PluginResponse::Position(
                self.engine.current_heading_index(scroll_y).map(|i| (i, 0)),
            ),
            PluginQuery::HasSelection => PluginResponse::Bool(self.engine.has_selection()),
            PluginQuery::SelectedText => {
                PluginResponse::String(self.engine.selected_text().unwrap_or_default())
            }
            PluginQuery::SelCursor => PluginResponse::Position(
                self.engine.sel.cursor.map(|p| (p.flat_line_idx, p.grapheme_pos)),
            ),
            PluginQuery::SelectionRange => PluginResponse::PositionPair(None),
            PluginQuery::HitTest { x, y, offset_x, offset_y } => PluginResponse::Position(
                self.engine
                    .hit_test(x, y, offset_x, offset_y)
                    .map(|p| (p.flat_line_idx, p.grapheme_pos)),
            ),
            PluginQuery::FlatLines => PluginResponse::FlatLines(
                self.engine
                    .flat_lines()
                    .iter()
                    .map(|line| {
                        let grapheme_count = crate::grapheme_map::grapheme_count(&line.text);
                        ui::plugin::FlatLine {
                            text: line.text.clone(),
                            grapheme_count,
                            rect: line.rect,
                            grapheme_x: (0..=grapheme_count)
                                .map(|grapheme| crate::layout::grapheme_x(line, grapheme))
                                .collect(),
                        }
                    })
                    .collect(),
            ),
            _ => PluginResponse::None,
        }
    }
}

// ===== NovelViewFactory =====

pub struct NovelViewFactory;

impl PluginFactory for NovelViewFactory {
    fn name(&self) -> &str {
        "novel_view"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|p| p.extension()).is_some_and(|e| e == "txt")
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(NovelView::new())
    }
}

// ===== MarkdownEditorView =====

/// WYSIWYG markdown editor view — wraps PreviewEngine with editing support.
/// Differs from MarkdownView in that it accepts cursor position updates
/// and provides layout-aware editing queries.
pub struct MarkdownEditorView {
    engine: PreviewEngine,
    source: String,
    cached_source_hash: u64,
    cached_generation: u32,
}

impl Default for MarkdownEditorView {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        let mut engine = PreviewEngine::new();
        engine.set_edit_source(Some(String::new()));
        Self { engine, source: String::new(), cached_source_hash: 0, cached_generation: 0 }
    }

    pub fn set_source(&mut self, text: String, generation: u32) {
        let hash = fxhash(&text);
        if hash != self.cached_source_hash {
            self.source = text;
            self.cached_source_hash = hash;
            self.engine.mark_source_dirty();
        }
        self.engine.set_edit_source(Some(self.source.clone()));
        self.engine.set_source_generation(generation);
        self.cached_generation = generation;
    }

    pub fn needs_source_update(&self, generation: u32) -> bool {
        generation != self.cached_generation
    }

    pub fn engine(&self) -> &PreviewEngine {
        &self.engine
    }
}

fn preedit_cursor_offset(preedit_text: &str, preedit_cursor: Option<(usize, usize)>) -> usize {
    let cursor_offset = preedit_cursor.map(|(_, cursor)| cursor).unwrap_or(preedit_text.len());
    let mut valid_offset = cursor_offset.min(preedit_text.len());
    while valid_offset > 0 && !preedit_text.is_char_boundary(valid_offset) {
        valid_offset -= 1;
    }
    valid_offset
}

/// Convert the IME preedit cursor's byte offset into a grapheme index within `preedit_text`.
fn preedit_cursor_grapheme_index(
    preedit_text: &str,
    preedit_cursor: Option<(usize, usize)>,
) -> usize {
    let byte_offset = preedit_cursor_offset(preedit_text, preedit_cursor);
    crate::grapheme_map::grapheme_index_at_byte(preedit_text, byte_offset)
}

impl ui::plugin::EditAugmenter for MarkdownEditorView {
    fn augment(&self, ctx: &ui::plugin::AugmentContext) -> Option<ui::plugin::EditAugmentation> {
        self.engine.augment_edit(ctx.current_byte, ctx.kind.clone())
    }
}

fn request_augment_kind(intent: &ui::plugin::EditIntent) -> Option<ui::plugin::AugmentKind> {
    match intent {
        ui::plugin::EditIntent::InsertText(text) => {
            Some(ui::plugin::AugmentKind::InsertText(text.clone()))
        }
        ui::plugin::EditIntent::InsertParagraphBreak => Some(ui::plugin::AugmentKind::Enter),
        ui::plugin::EditIntent::DeleteBackward => Some(ui::plugin::AugmentKind::Backspace),
        ui::plugin::EditIntent::Indent => Some(ui::plugin::AugmentKind::Tab),
        ui::plugin::EditIntent::DeleteForward
        | ui::plugin::EditIntent::Outdent
        | ui::plugin::EditIntent::PromoteObject
        | ui::plugin::EditIntent::DemoteObject
        | ui::plugin::EditIntent::SelectObject => None,
    }
}

fn augmentation_edit_plan(
    request: &ui::plugin::EditRequest,
    augmentation: ui::plugin::EditAugmentation,
) -> ui::plugin::EditPlan {
    let range = augmentation.replace_range.unwrap_or(request.cursor_byte..request.cursor_byte);
    let text = augmentation.insert_text.unwrap_or_default();
    if range.is_empty() && text.is_empty() {
        if augmentation.cursor_byte_after == request.cursor_byte {
            return ui::plugin::EditPlan::Consume;
        }
        return ui::plugin::EditPlan::MoveCursor(ui::plugin::CursorUpdate {
            cursor_after: augmentation.cursor_byte_after,
        });
    }
    ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
        request.source_generation,
        range,
        text,
        augmentation.cursor_byte_after,
    ))
}

/// 把基于"选区已删除"虚拟源码计算的增强映射回真实文档的单条替换。
///
/// 虚拟源码 = 真实源码删除 `selection` 后的文本，删除点（= `selection.start`）
/// 即增强计算的光标位置。偏移映射规则：虚拟偏移 `v < selection.start` 时真实
/// 偏移相同，否则 `v + 选区长度`；range 起点位于删除点左侧时保持不变，因此
/// 映射后的 range 恰好覆盖整个选区。
/// `cursor_byte_after` 是增强后文本的坐标——真实替换与虚拟替换产生的最终
/// 文本是同一字符串，无需映射。
///
/// 仅当增强的 `replace_range` 覆盖删除点时映射成立（Enter 的各增强均满足）；
/// 否则返回 `UseDefault` 回落默认计划。
fn selection_augmentation_edit_plan(
    request: &ui::plugin::EditRequest,
    selection: &std::ops::Range<usize>,
    augmentation: ui::plugin::EditAugmentation,
) -> ui::plugin::EditPlan {
    let virtual_range = augmentation.replace_range.unwrap_or(selection.start..selection.start);
    let deleted_len = selection.end - selection.start;
    let replacement_text = augmentation.insert_text.unwrap_or_default();

    if virtual_range.start <= selection.start && virtual_range.end >= selection.start {
        return ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
            request.source_generation,
            virtual_range.start..virtual_range.end + deleted_len,
            replacement_text,
            augmentation.cursor_byte_after,
        ));
    }

    let mapped_range = if virtual_range.end < selection.start {
        virtual_range
    } else if virtual_range.start > selection.start {
        virtual_range.start + deleted_len..virtual_range.end + deleted_len
    } else {
        return ui::plugin::EditPlan::UseDefault;
    };
    ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction {
        source_generation: request.source_generation,
        replacements: vec![
            ui::plugin::TextReplacement { range: selection.clone(), text: String::new() },
            ui::plugin::TextReplacement { range: mapped_range, text: replacement_text },
        ],
        selection_after: ui::plugin::EditSelection::Caret(augmentation.cursor_byte_after),
    })
}

impl ui::plugin::EditPolicy for MarkdownEditorView {
    fn plan_edit(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
        // 零宽选区视为无选区。
        let selection = request.selection.as_ref().filter(|range| range.start < range.end);
        if let Some(selection) = selection {
            return self.plan_selection_edit(request, selection);
        }
        let Some(kind) = request_augment_kind(&request.intent) else {
            return ui::plugin::EditPlan::UseDefault;
        };
        self.engine
            .augment_edit(request.cursor_byte, kind)
            .map_or(ui::plugin::EditPlan::UseDefault, |augmentation| {
                augmentation_edit_plan(request, augmentation)
            })
    }
}

impl MarkdownEditorView {
    /// 带选区编辑的计划：仅回车做块级增强（删选区 + 删除点上下文增强），
    /// 其余 intent 维持默认计划（替换/删除选区）。
    fn plan_selection_edit(
        &self,
        request: &ui::plugin::EditRequest,
        selection: &std::ops::Range<usize>,
    ) -> ui::plugin::EditPlan {
        if !matches!(request.intent, ui::plugin::EditIntent::InsertParagraphBreak) {
            return ui::plugin::EditPlan::UseDefault;
        }
        // 选区越界或落在非字符边界时无法构造虚拟源码，交回默认计划
        // （默认计划产出的非法事务会在执行侧被校验拒绝，而不是在此 panic）。
        if selection.end > self.source.len()
            || !self.source.is_char_boundary(selection.start)
            || !self.source.is_char_boundary(selection.end)
        {
            return ui::plugin::EditPlan::UseDefault;
        }
        let mut source_after_delete = self.source.clone();
        source_after_delete.replace_range(selection.clone(), "");
        let Some(augmentation) = crate::augmenter::augment_edit(
            &source_after_delete,
            selection.start,
            ui::plugin::AugmentKind::Enter,
        ) else {
            return ui::plugin::EditPlan::UseDefault;
        };
        selection_augmentation_edit_plan(request, selection, augmentation)
    }
}

impl ViewPlugin for MarkdownEditorView {
    fn handles_own_rendering(&self) -> bool {
        true
    }

    fn augmenter(&self) -> &dyn ui::plugin::EditAugmenter {
        self
    }

    fn edit_policy(&self) -> &dyn ui::plugin::EditPolicy {
        self
    }

    fn name(&self) -> &str {
        "markdown_editor"
    }

    fn allows_editing(&self) -> bool {
        // WYSIWYG editor — app_renderer will enable
        // Copy/Paste/Undo context menu and scrollbar behaviour.
        true
    }

    fn shows_cursor(&self) -> bool {
        // WYSIWYG draws its own cursor via render() — app should not overlay one.
        false
    }

    fn shows_gutter(&self) -> bool {
        false
    }

    fn needs_cursor_blink_wakeup(&self) -> bool {
        // App must still compute blink phase and forward it via SetCursorVisible.
        true
    }

    fn render(
        &mut self,
        _doc: &dyn DocView,
        bounds: ui::core::geom::Rect,
        theme: &Theme,
        shaper: &mut shaping::Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let _t0 = std::time::Instant::now();
        let settings = MarkdownRenderSettings {
            font_size: self.engine.base_font_size * dpi_scale,
            line_height: self.engine.base_line_height * dpi_scale,
            toc_max_depth: self.engine.toc_max_depth,
        };
        let style = settings.style(theme);
        self.engine.toc_max_depth = settings.toc_max_depth;
        let source = &self.source;
        let string_doc = core::document::StringDocView::new(source);
        let (mut dl, _) = self.engine.render(
            theme,
            bounds.w,
            bounds.h,
            bounds.x,
            bounds.y,
            &style,
            |s| {
                let parsed = crate::parser::parse_markdown(source);
                crate::builder::MarkdownDoc::build(&parsed, s)
            },
            Some(shaper),
            &string_doc,
            true,  // editing keeps whole-document flat lines for selection/navigation.
            false, // precise shaping/highlighting stays viewport-driven for responsiveness.
        );
        let _dur = _t0.elapsed().as_micros();
        #[cfg(debug_assertions)]
        {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/perf.log")
                .and_then(|mut f| {
                    use std::io::Write;
                    writeln!(f, "[md render] {} us", _dur)
                });
            println!("[md render] {} us", _dur);
        }
        if let Some(preedit) = self.engine.standalone_preedit_render_data() {
            let preedit_text = preedit.text.to_owned();
            let preedit_cursor = preedit.cursor;
            let preedit_x = preedit.x;
            let preedit_baseline_y = preedit.baseline_y;
            let preedit_font_size = preedit.font_size;
            let cursor_offset = preedit_cursor_offset(&preedit_text, preedit_cursor);
            let cursor_advance = ui::core::text_layout::UiTextLayout::new(
                &preedit_text[..cursor_offset],
                preedit_font_size,
                None,
                shaping::Weight::NORMAL,
                shaping::Style::Normal,
                false,
                shaper,
            )
            .map_or(0.0, |layout| layout.shaped.width);
            self.engine.set_standalone_preedit_cursor_advance(cursor_advance);
            dl.text_shaped(
                bounds.x + preedit_x,
                bounds.y + preedit_baseline_y,
                preedit_font_size,
                theme.editor.foreground,
                &preedit_text,
                shaper,
            );
        }
        // Draw cursor at the WYSIWYG position (only when blink phase is visible)
        if self.engine.cursor_visible
            && let Some((cx, cy, cw, ch)) =
                self.engine.visual_cursor_screen_pos().or_else(|| self.engine.cursor_screen_pos())
        {
            let cursor_x = bounds.x + cx;
            let cursor_y = bounds.y + cy;
            let visual_cw = cw * dpi_scale;
            let cursor_rect =
                ui::core::geom::Rect::new(cursor_x - visual_cw * 0.5, cursor_y, visual_cw, ch);
            dl.fill(cursor_rect, theme.editor.cursor);
        }
        dl
    }

    fn handle_message(
        &mut self,
        msg: PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        if let Some(result) = self.engine.handle_message_common(&msg) {
            return result;
        }
        match msg {
            PluginMessage::UpdateSource { text, generation } => {
                self.set_source(text, generation);
                true
            }
            _ => false,
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        if let Some(resp) = self.engine.query_common(&q) {
            return resp;
        }
        match q {
            PluginQuery::NeedsSourceUpdate(gen_id) => {
                PluginResponse::Bool(self.needs_source_update(gen_id))
            }
            PluginQuery::SearchHighlights { .. } => {
                // WYSIWYG editor delegates search to the app layer.
                PluginResponse::DrawList(DrawList::new())
            }
            PluginQuery::PlanSemanticEdit {
                command,
                source_generation,
                cursor_byte,
                selection,
            } => PluginResponse::SemanticEdit(crate::commands::plan_semantic_edit(
                &self.source,
                source_generation,
                cursor_byte,
                selection,
                command,
            )),
            _ => PluginResponse::None,
        }
    }
}

// ===== MarkdownEditorViewFactory =====

pub struct MarkdownEditorViewFactory;

impl PluginFactory for MarkdownEditorViewFactory {
    fn name(&self) -> &str {
        "markdown_editor"
    }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        // .md files default to the editor view.
        path.is_some_and(|p| p.extension().is_some_and(|e| e == "md" || e == "markdown"))
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownEditorView::new())
    }
}

/// Notora 使用的 Markdown 工厂；正文 H1 与产品元数据标题彼此独立。
pub struct NotoraMarkdownEditorViewFactory;

impl PluginFactory for NotoraMarkdownEditorViewFactory {
    fn name(&self) -> &str {
        "markdown_editor"
    }

    fn can_handle(&self, path: Option<&Path>) -> bool {
        MarkdownEditorViewFactory.can_handle(path)
    }

    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownEditorView::new())
    }
}

fn style_hash_quick(style: &MarkdownStyle) -> u64 {
    let mut h: u64 = 0x517cc1b727220a95;
    for &fs in &style.heading_font_sizes {
        h = h.rotate_left(5) ^ (fs.to_bits() as u64);
    }
    h = h.rotate_left(5) ^ (style.body_font_size.to_bits() as u64);
    h = h.rotate_left(5) ^ (style.code_font_size.to_bits() as u64);
    h = h.rotate_left(5) ^ (style.line_height.to_bits() as u64);
    h = h.rotate_left(5) ^ (style.paragraph_spacing.to_bits() as u64);
    h = h.rotate_left(5) ^ (style.list_indent.to_bits() as u64);
    h = h.rotate_left(5) ^ (style.list_item_spacing.to_bits() as u64);
    for &c in &[
        style.text_color,
        style.code_color,
        style.code_bg,
        style.link_color,
        style.heading_color,
        style.blockquote_bg,
        style.blockquote_border,
    ] {
        for &v in &c {
            h = h.rotate_left(5) ^ (v.to_bits() as u64);
        }
    }
    h
}

fn fxhash(s: &str) -> u64 {
    let mut h: u64 = 0x517cc1b727220a95;
    for byte in s.bytes() {
        h = h.rotate_left(5) ^ (byte as u64);
    }
    h
}

// ===== Tests =====

#[cfg(test)]
mod heading_tests {
    use super::*;

    fn make_headings(entries: Vec<(u8, f32)>) -> Vec<HeadingEntry> {
        entries
            .into_iter()
            .map(|(level, y_offset)| HeadingEntry { text: format!("H{level}"), level, y_offset })
            .collect()
    }

    fn make_engine_with_headings(headings: Vec<HeadingEntry>) -> PreviewEngine {
        let mut e = PreviewEngine::new();
        e.headings = headings;
        e
    }

    #[test]
    fn current_heading_index_empty() {
        let e = make_engine_with_headings(vec![]);
        assert_eq!(e.current_heading_index(0.0), None);
    }

    #[test]
    fn current_heading_index_single_at_zero() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0)]));
        assert_eq!(e.current_heading_index(0.0), Some(0));
    }

    #[test]
    fn current_heading_index_single_below() {
        let e = make_engine_with_headings(make_headings(vec![(1, 100.0)]));
        assert_eq!(e.current_heading_index(50.0), Some(0));
    }

    #[test]
    fn current_heading_index_single_above() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0)]));
        assert_eq!(e.current_heading_index(50.0), Some(0));
    }

    #[test]
    fn current_heading_index_multiple_exact_match() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (1, 200.0)]));
        assert_eq!(e.current_heading_index(100.0), Some(1));
    }

    #[test]
    fn current_heading_index_multiple_between() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (1, 200.0)]));
        assert_eq!(e.current_heading_index(150.0), Some(1));
    }

    #[test]
    fn current_heading_index_at_last_heading() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (3, 200.0)]));
        assert_eq!(e.current_heading_index(200.0), Some(2));
    }

    #[test]
    fn current_heading_index_beyond_last() {
        let e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0)]));
        assert_eq!(e.current_heading_index(500.0), Some(1));
    }

    #[test]
    fn scroll_to_heading_valid_index() {
        let mut e =
            make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0), (1, 200.0)]));
        e.content_height = 300.0;
        e.scroll_to_heading(1);
        assert_eq!(e.scroll_y, 100.0);
    }

    #[test]
    fn scroll_to_heading_first() {
        let mut e = make_engine_with_headings(make_headings(vec![(1, 0.0), (2, 100.0)]));
        e.content_height = 200.0;
        e.scroll_y = 150.0;
        e.scroll_to_heading(0);
        assert_eq!(e.scroll_y, 0.0);
    }

    #[test]
    fn scroll_to_heading_out_of_bounds() {
        let mut e = make_engine_with_headings(make_headings(vec![(1, 0.0)]));
        e.content_height = 100.0;
        e.scroll_y = 50.0;
        e.scroll_to_heading(99);
        assert_eq!(e.scroll_y, 50.0);
    }

    #[test]
    fn scroll_to_heading_clamps_to_content_height() {
        let mut e = make_engine_with_headings(make_headings(vec![(1, 500.0)]));
        e.content_height = 300.0;
        e.scroll_to_heading(0);
        assert_eq!(e.scroll_y, 300.0);
    }

    #[test]
    fn scroll_y_accessor() {
        let mut v = MarkdownView::new();
        v.engine.scroll_y = 42.0;
        assert_eq!(v.scroll_y(), 42.0);
    }

    #[test]
    fn headings_accessor_empty() {
        let v = MarkdownView::new();
        assert!(v.headings().is_empty());
    }

    #[test]
    fn headings_accessor_returns_slice() {
        let mut v = MarkdownView::new();
        v.engine.headings = make_headings(vec![(1, 0.0), (2, 50.0)]);
        assert_eq!(v.headings().len(), 2);
        assert_eq!(v.headings()[0].level, 1);
        assert_eq!(v.headings()[1].level, 2);
    }

    #[test]
    fn markdown_render_settings_take_physical_metrics() {
        let settings = ui::settings::Settings::new();
        let metrics = ui::settings::UiMetrics::from_settings(&settings, 2.0);
        let input = MarkdownRenderSettings::from_metrics(&settings, &metrics);
        assert_eq!(input.font_size, metrics.font_size);
        assert_eq!(input.line_height, metrics.line_height);
        assert_eq!(input.toc_max_depth, settings.toc_max_depth);
    }

    #[test]
    fn render_settings_control_style_and_toc_depth() {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings =
            MarkdownRenderSettings { font_size: 36.0, line_height: 58.0, toc_max_depth: 5 };
        let style = settings.style(&theme);
        assert_eq!(style.body_font_size, 36.0);
        assert_eq!(style.line_height, 58.0);

        let mut v = MarkdownView::new();
        v.set_source("# H1\n\n##### H5\n\n###### H6".into(), 1);
        let _ = v.render(&theme, 600.0, 400.0, 0.0, 0.0, settings, None);
        assert_eq!(v.headings().len(), 2);
        assert_eq!(v.headings()[1].level, 5);
    }
}

#[cfg(test)]
mod wysiwyg_tests {
    use super::*;
    use crate::projection::{CursorAffinity, ProjectionOwnerId, SourceAnchor};
    use ui::plugin::AugmentKind;
    use ui::plugin::MoveDirection;

    fn default_settings() -> MarkdownRenderSettings {
        MarkdownRenderSettings { font_size: 15.0, line_height: 24.0, toc_max_depth: 5 }
    }

    #[test]
    fn notora_markdown_editor_keeps_the_first_h1_after_titles_become_independent() {
        let source = "# 页面标题\n\n正文内容\n\n# 正文章节";
        let document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);

        render_editor_once(&mut view, &document);

        let rendered_text =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        assert!(rendered_text.iter().any(|line| line.contains("页面标题")));
        assert!(rendered_text.iter().any(|line| line.contains("正文内容")));
        assert!(rendered_text.iter().any(|line| line.contains("正文章节")));
        assert!(
            view.engine()
                .projection_index()
                .visual_position_for_source(
                    source.find("正文内容").expect("body must exist"),
                    CursorAffinity::Downstream
                )
                .is_some(),
            "body source bytes must remain mapped while showing the independent H1"
        );
    }

    #[test]
    fn default_markdown_editor_keeps_the_first_h1_for_textora() {
        let source = "# 页面标题\n\n正文内容";
        let document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);

        render_editor_once(&mut view, &document);

        assert!(
            view.engine().flat_lines().iter().any(|line| line.text.contains("页面标题")),
            "the shared textora Markdown editor must retain its default rendering"
        );
    }

    /// Build a MarkdownView, set source, render, and return it.
    fn make_view(src: &str) -> MarkdownView {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings = default_settings();
        let mut v = MarkdownView::new();
        v.set_source(src.into(), 1);
        v.engine_mut().set_edit_source(Some(src.into()));
        v.render(&theme, 800.0, 600.0, 0.0, 0.0, settings, None);
        v
    }

    fn make_projected_view(src: &str) -> MarkdownView {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings = default_settings();
        let mut view = MarkdownView::new();
        view.set_source(src.into(), 1);
        view.engine_mut().set_edit_source(Some(src.into()));
        let mut shaper = shaping::Shaper::new().expect("WYSIWYG test view requires a shaper");
        view.render(&theme, 800.0, 600.0, 0.0, 0.0, settings, Some(&mut shaper));
        view
    }

    #[test]
    fn editable_empty_line_is_a_zero_grapheme_projection_line() {
        let source = "paragraph\n\n\nnext";
        let second_empty = "paragraph\n\n".len();
        let view = make_view(source);
        assert_eq!(
            view.engine().flat_lines().len(),
            2,
            "source-only empty lines must not alter the public rendered-line collection"
        );
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(second_empty, CursorAffinity::Downstream)
            .expect("editable empty line must be projected");
        let line = &view.engine().projection_index().visual_lines()[position.flat_line_idx];

        assert_eq!(line.owner, ProjectionOwnerId::EmptyLine { source_byte: second_empty });
        assert_eq!(line.boundaries, vec![SourceAnchor::downstream(second_empty)]);
    }

    #[test]
    fn trailing_editable_empty_line_is_a_zero_grapheme_projection_line() {
        let source = "paragraph\n\n";
        let trailing_empty = source.len();
        let view = make_view(source);
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(trailing_empty, CursorAffinity::Downstream)
            .expect("trailing editable empty line must be projected");
        let line = &view.engine().projection_index().visual_lines()[position.flat_line_idx];

        assert_eq!(line.owner, ProjectionOwnerId::EmptyLine { source_byte: trailing_empty });
        assert_eq!(line.boundaries, vec![SourceAnchor::downstream(trailing_empty)]);
    }

    #[test]
    fn wysiwyg_can_scroll_to_trailing_empty_source_lines() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "paragraph\n\n\n";
        let viewport_height = 48.0;
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_with_offset_y(&mut view, &document, viewport_height, 0.0);

        let did_scroll = view.handle_message(
            PluginMessage::Scroll { delta: 100.0, viewport_h: viewport_height },
            &mut document,
        );

        assert!(did_scroll, "trailing empty source lines must extend the scrollable range");
        assert!(view.engine().scroll_y > 0.0, "scroll position must advance past the text block");
    }

    fn click_point_for_visible_text(
        engine: &PreviewEngine<MarkdownDoc>,
        needle: &str,
    ) -> (f32, f32) {
        let line = engine
            .flat_lines()
            .iter()
            .find(|line| line.text.contains(needle))
            .expect("needle must be rendered");
        let byte = line.text.find(needle).expect("needle must be in selected line");
        let grapheme = crate::grapheme_map::grapheme_index_at_byte(&line.text, byte);
        (line.rect.x + crate::layout::grapheme_x(line, grapheme), line.rect.y + line.rect.h * 0.5)
    }

    #[test]
    fn promotion_line_three_real_rect_hits_line_three_source_range() {
        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches and campaigns.\n> Style anchor: Apple Keynote and exhibitions.\n";
        let line_three = source.find("Applicable").expect("fixture must contain line three");
        let line_four = source.find("\n> Style").expect("fixture must contain line four") + 1;
        let view = make_projected_view(source);
        let (x, y) = click_point_for_visible_text(view.engine(), "Applicable");
        let hit = view.engine().hit_test_byte(x, y, 0.0, 0.0).expect("hit required");
        assert!((line_three..line_four).contains(&hit));
    }

    #[test]
    fn promotion_line_three_cursor_position_matches_its_visible_text_line() {
        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches and campaigns.\n> Style anchor: Apple Keynote and exhibitions.\n";
        let line_three = source.find("Applicable").expect("fixture must contain line three");
        let view = make_projected_view(source);
        let visible_line = view
            .engine()
            .flat_lines()
            .iter()
            .position(|line| line.text.contains("Applicable"))
            .expect("line three text must be visible");
        let position = view
            .engine()
            .cursor_visual_position_for_byte(line_three, CursorAffinity::Downstream)
            .expect("line three source byte must have a visual position");
        assert_eq!(position.flat_line_idx, visible_line);
        assert_eq!(position.grapheme_pos, 0);
    }

    #[test]
    fn promotion_blockquote_click_roundtrip_and_vertical_navigation_reach_line_three() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        const NARROW_VIEWPORT_WIDTH: f32 = 180.0;
        const WIDE_VIEWPORT_WIDTH: f32 = 800.0;
        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches, marketing campaigns, art/fashion/culture showcases, product introductions, etc.\n> Style anchor: Apple Keynote / Xiaomi product launch / High-end fashion brands / Art exhibitions / Cultural promotion / Premium brand visual systems\n\n## Design Philosophy\n\n- **Top-tier visual impact**: Large images and bold typography, using extreme visual tension to convey brand tone, artistic quality, or cultural depth — every page should leave the audience breathless\n";
        let line_three_start =
            source.find("Applicable scenarios").expect("fixture must contain real line three text");
        let line_four_start =
            source.find("\n> Style anchor").expect("fixture must contain real line four") + 1;
        let title_start = source.find("Promotion").expect("fixture must contain title text");

        for width in [NARROW_VIEWPORT_WIDTH, WIDE_VIEWPORT_WIDTH] {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (flat_idx, grapheme_idx) = view
                .engine()
                .flat_line_projection_boundaries()
                .iter()
                .find_map(|map| {
                    map.boundaries
                        .iter()
                        .position(|&byte| byte == line_three_start)
                        .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
                })
                .expect("initial layout must expose real line three source byte");
            let flat_line = &view.engine().flat_lines()[flat_idx];
            let click_x = flat_line.rect.x + crate::layout::grapheme_x(flat_line, grapheme_idx);
            let click_y = flat_line.rect.y + flat_line.rect.h * 0.5;

            let first_hit = view
                .engine()
                .hit_test_byte(click_x, click_y, 0.0, 0.0)
                .expect("first click on real line three must produce a source byte");
            view.handle_message(PluginMessage::SetCursorByte(first_hit), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
                .engine()
                .cursor_screen_pos()
                .expect("selected line three byte must have a cursor screen position");
            let second_hit = view
                .engine()
                .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
                .expect("cursor-position click must produce a source byte");

            assert_eq!(first_hit, line_three_start);
            assert_eq!(second_hit, line_three_start);

            view.handle_message(PluginMessage::SetCursorByte(title_start), &mut document);
            render_editor_narrow(&mut view, &document, width);
            let (title_x, _title_y, _title_width, _title_height) = view
                .engine()
                .cursor_screen_pos()
                .expect("title must have a cursor screen position");
            let maximum_visual_moves = view.engine().flat_lines().len();
            let mut moved = title_start;
            if width == NARROW_VIEWPORT_WIDTH {
                let title_flat_idx = view
                    .engine()
                    .flat_line_projection_boundaries()
                    .iter()
                    .find(|map| map.boundaries.contains(&title_start))
                    .map(|map| map.flat_idx)
                    .expect("active heading must expose the title source byte");
                let heading_projections = view.engine().flat_line_projection_boundaries();
                let second_heading_segment = heading_projections
                    .get(title_flat_idx + 1)
                    .expect("narrow active heading must wrap to a second visual segment");
                let first_down = view
                    .engine()
                    .visual_move(title_start, MoveDirection::Down, Some(title_x))
                    .expect("first Down from title must produce a source byte");

                assert!(
                    second_heading_segment.boundaries.contains(&first_down),
                    "first Down must enter the active heading's second visual segment, got {first_down}",
                );
                assert!(
                    first_down < source.find('\n').expect("title fixture must end with a newline"),
                    "first Down must remain within the heading source line, got {first_down}",
                );
                moved = first_down;
            }
            for _ in 0..maximum_visual_moves {
                moved = view
                    .engine()
                    .visual_move(moved, MoveDirection::Down, Some(title_x))
                    .expect("Down from title must produce a source byte");
                if (line_three_start..line_four_start).contains(&moved) {
                    break;
                }
            }

            assert!(
                (line_three_start..line_four_start).contains(&moved),
                "Down from title must reach real line three source bytes {line_three_start}..{line_four_start} at width {width}, got {moved}",
            );
        }
    }

    #[test]
    fn promotion_em_dash_click_roundtrip_never_lands_inside_utf8_sequence() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches, marketing campaigns, art/fashion/culture showcases, product introductions, etc.\n> Style anchor: Apple Keynote / Xiaomi product launch / High-end fashion brands / Art exhibitions / Cultural promotion / Premium brand visual systems\n\n## Design Philosophy\n\n- **Top-tier visual impact**: Large images and bold typography, using extreme visual tension to convey brand tone, artistic quality, or cultural depth — every page should leave the audience breathless\n";
        let em_dash = source.find('—').expect("fixture must contain real line eight em dash");
        let after_em_dash = em_dash + '—'.len_utf8();

        for width in [180.0, 800.0] {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (flat_idx, grapheme_idx) = view
                .engine()
                .flat_line_projection_boundaries()
                .iter()
                .find_map(|map| {
                    map.boundaries
                        .iter()
                        .position(|&byte| byte == after_em_dash)
                        .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
                })
                .expect("initial layout must expose byte after real em dash");
            let flat_line = &view.engine().flat_lines()[flat_idx];
            let click_x = flat_line.rect.x + crate::layout::grapheme_x(flat_line, grapheme_idx);
            let click_y = flat_line.rect.y + flat_line.rect.h * 0.5;

            let first_hit = view
                .engine()
                .hit_test_byte(click_x, click_y, 0.0, 0.0)
                .expect("first click after real em dash must produce a source byte");
            view.handle_message(PluginMessage::SetCursorByte(first_hit), &mut document);
            render_editor_narrow(&mut view, &document, width);

            let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
                .engine()
                .cursor_screen_pos()
                .expect("selected em-dash-adjacent byte must have a cursor screen position");
            let second_hit = view
                .engine()
                .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
                .expect("cursor-position click must produce a source byte");

            assert_eq!(first_hit, after_em_dash);
            assert_eq!(second_hit, after_em_dash);
            assert!(source.is_char_boundary(second_hit));
            assert_eq!(
                view.engine().visual_move(after_em_dash, MoveDirection::Left, None),
                Some(em_dash),
                "Left must cross the complete real em dash at width {width}",
            );
            assert_eq!(
                view.engine().visual_move(em_dash, MoveDirection::Right, None),
                Some(after_em_dash),
                "Right must cross the complete real em dash at width {width}",
            );
        }
    }

    fn styled_line_for_source_byte(
        block: &crate::layout::LaidOutBlock,
        source_byte: usize,
    ) -> Option<&crate::layout::LaidOutLine> {
        fn line_for_source_byte(
            lines: &[crate::layout::LaidOutLine],
            source_byte: usize,
        ) -> Option<&crate::layout::LaidOutLine> {
            lines.iter().find(|line| {
                line.source_projection.as_ref().is_some_and(|projection| {
                    projection.boundaries.iter().any(|anchor| anchor.byte == source_byte)
                })
            })
        }

        match &block.kind {
            crate::layout::LaidOutBlockKind::Text { lines }
            | crate::layout::LaidOutBlockKind::MetadataBlock { lines } => {
                line_for_source_byte(lines, source_byte)
            }
            crate::layout::LaidOutBlockKind::ListItem { lines, blocks, .. } => {
                line_for_source_byte(lines, source_byte).or_else(|| {
                    blocks.iter().find_map(|child| styled_line_for_source_byte(child, source_byte))
                })
            }
            crate::layout::LaidOutBlockKind::BlockQuote { blocks } => {
                blocks.iter().find_map(|child| styled_line_for_source_byte(child, source_byte))
            }
            crate::layout::LaidOutBlockKind::Table { header, rows, .. } => {
                header.iter().flatten().chain(rows.iter().flatten().flatten()).find(|line| {
                    line.source_projection.as_ref().is_some_and(|projection| {
                        projection.boundaries.iter().any(|anchor| anchor.byte == source_byte)
                    })
                })
            }
            crate::layout::LaidOutBlockKind::CodeBlock { .. }
            | crate::layout::LaidOutBlockKind::HorizontalRule => None,
        }
    }

    #[test]
    fn styled_prefix_cursor_and_hit_test_use_rendered_advance() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# Promotion & Marketing\n\n> Applicable scenarios: Brand launches, marketing campaigns, art/fashion/culture showcases, product introductions, etc.\n> Style anchor: Apple Keynote / Xiaomi product launch / High-end fashion brands / Art exhibitions / Cultural promotion / Premium brand visual systems\n\n## Design Philosophy\n\n- **Top-tier visual impact**: Large images and bold typography, using extreme visual tension to convey brand tone, artistic quality, or cultural depth — every page should leave the audience breathless\n";
        let colon_byte = source.find("**:").expect("fixture must contain bold closing marker") + 2;
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(colon_byte), &mut document);
        render_editor_once(&mut view, &document);

        let (flat_idx, grapheme_idx) = view
            .engine()
            .flat_line_projection_boundaries()
            .iter()
            .find_map(|map| {
                map.boundaries
                    .iter()
                    .position(|&byte| byte == colon_byte)
                    .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
            })
            .expect("materialized list line must expose the colon source byte");
        let flat_line = &view.engine().flat_lines()[flat_idx];
        let styled_line = view
            .engine()
            .lazy
            .as_ref()
            .and_then(|lazy| {
                lazy.laid_out
                    .iter()
                    .flatten()
                    .find_map(|block| styled_line_for_source_byte(block, colon_byte))
            })
            .expect("precise layout must retain the materialized list line");
        let bold = styled_line
            .style_segments
            .iter()
            .find(|segment| matches!(segment.style, crate::builder::InlineStyle::Bold))
            .expect("precise layout must retain the bold prefix segment");
        let closing_marker = styled_line
            .style_segments
            .iter()
            .find(|segment| {
                matches!(segment.style, crate::builder::InlineStyle::SourceMarker)
                    && segment.start == bold.start + bold.len
            })
            .expect("precise layout must retain the bold closing marker");
        assert!(
            (closing_marker.x_offset - (bold.x_offset + bold.width)).abs() < 0.01,
            "bold closing marker must start at the styled visual boundary"
        );
        let expected_x = flat_line.rect.x + closing_marker.x_offset + closing_marker.width;
        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("colon cursor must resolve to the materialized list line");
        assert!(
            (cursor_x - expected_x).abs() < 0.01,
            "cursor x {cursor_x} must match the bold visual boundary {expected_x}"
        );

        let hit = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
            .expect("click at the styled boundary must produce a source byte");
        assert_eq!(hit, colon_byte);
        assert_eq!(
            crate::layout::grapheme_x(flat_line, grapheme_idx) + flat_line.rect.x,
            cursor_x,
            "cursor must use the same visual grapheme geometry as hit-testing"
        );
    }

    #[test]
    fn active_heading_marker_cursor_and_hit_test_use_heading_advance() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# Title";
        let marker_end = "# ".len();
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(marker_end), &mut document);
        render_editor_once(&mut view, &document);

        let (flat_idx, grapheme_idx) = view
            .engine()
            .flat_line_projection_boundaries()
            .iter()
            .find_map(|map| {
                map.boundaries
                    .iter()
                    .position(|&byte| byte == marker_end)
                    .map(|grapheme_idx| (map.flat_idx, grapheme_idx))
            })
            .expect("active heading marker must expose its source boundary");
        let flat_line = &view.engine().flat_lines()[flat_idx];
        let styled_line = view
            .engine()
            .lazy
            .as_ref()
            .and_then(|lazy| {
                lazy.laid_out
                    .iter()
                    .flatten()
                    .find_map(|block| styled_line_for_source_byte(block, marker_end))
            })
            .expect("precise layout must retain the active heading line");
        let marker = styled_line
            .style_segments
            .iter()
            .find(|segment| matches!(segment.style, crate::builder::InlineStyle::SourceMarker))
            .expect("active heading marker must retain a source-marker segment");

        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let style = default_settings().style(&theme);
        shaper.set_font_size(styled_line.font_size);
        shaper.set_font_weight(styled_line.font_weight);
        shaper.set_font_family(style.body_font_family.first().map(String::as_str));
        let expected_marker_width = shaper
            .shape("# ")
            .expect("heading marker should shape with its line font weight")
            .width;
        let expected_x = flat_line.rect.x + expected_marker_width;

        assert!(
            (marker.width - expected_marker_width).abs() < 0.01,
            "marker width {} must match the heading weight advance {expected_marker_width}",
            marker.width
        );

        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("heading marker boundary must have a cursor position");
        assert!(
            (cursor_x - expected_x).abs() < 0.01,
            "cursor x {cursor_x} must match the heading marker advance {expected_x}"
        );

        let hit = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0)
            .expect("click at the heading marker boundary must produce a source byte");
        assert_eq!(hit, marker_end);
        assert_eq!(
            crate::layout::grapheme_x(flat_line, grapheme_idx) + flat_line.rect.x,
            cursor_x,
            "cursor and hit-testing must share the heading marker geometry"
        );
    }

    // ── augment_edit ─────────────────────────────────────────────────────

    #[test]
    fn augment_edit_bullet_list_inserts_marker() {
        let mut v = make_view("- item");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n- "));
    }

    #[test]
    fn augment_edit_nested_bullet_list_preserves_indent() {
        let source = "- parent\n  - child";
        let mut v = make_view(source);
        let cursor_byte = source.len();
        v.engine_mut().handle_set_cursor_byte(cursor_byte);
        let aug = v.engine().augment_edit(cursor_byte, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n  - "));
    }

    #[test]
    fn augment_edit_ordered_list_increments_number() {
        let mut v = make_view("1. item");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n2. "));
    }

    #[test]
    fn augment_edit_task_list_inserts_checkbox() {
        let mut v = make_view("- [ ] task");
        v.engine_mut().handle_set_cursor_byte(7);
        let aug = v.engine().augment_edit(7, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n- [ ] "));
    }

    #[test]
    fn augment_edit_backspace_list_item_marker() {
        let mut v = make_view("- hello");
        v.engine_mut().handle_set_cursor_byte(2); // "- |hello"
        let aug = v.engine().augment_edit(2, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..2));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_ordered_list_item_marker() {
        let mut v = make_view("12. hello");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..4));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_blockquote_marker() {
        let mut v = make_view("> hello");
        v.engine_mut().handle_set_cursor_byte(2);
        let aug = v.engine().augment_edit(2, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..2));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_heading_marker() {
        let mut v = make_view("### hello");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..4));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_backspace_inside_heading_marker_returns_none() {
        let mut v = make_view("### hello");
        v.engine_mut().handle_set_cursor_byte(2); // "##|# hello"
        let aug = v.engine().augment_edit(2, AugmentKind::Backspace);
        assert!(aug.is_none());
    }

    #[test]
    fn augment_edit_backspace_indented_list_item_marker() {
        let mut v = make_view("  - hello");
        v.engine_mut().handle_set_cursor_byte(4); // "  - |hello"
        let aug = v.engine().augment_edit(4, AugmentKind::Backspace).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some(""));
        assert_eq!(aug.replace_range, Some(0..4));
        assert_eq!(aug.cursor_byte_after, 0);
    }

    #[test]
    fn augment_edit_paragraph_middle_inserts_paragraph_break() {
        let mut v = make_view("hello world");
        v.engine_mut().handle_set_cursor_byte(4);
        let aug = v.engine().augment_edit(4, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.replace_range, None);
        assert_eq!(aug.cursor_byte_after, 6);
    }

    #[test]
    fn augment_edit_paragraph_before_timestamp_inserts_visible_break() {
        let source = "主要使用外部模型进行写作辅助。29:49";
        let timestamp_start = source.find("29:49").expect("fixture should contain timestamp");
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(timestamp_start);

        let aug = view.engine.augment_edit(timestamp_start, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.replace_range, None);
        assert_eq!(aug.cursor_byte_after, timestamp_start + 2);
    }

    #[test]
    fn augment_edit_heading_middle_splits_at_cursor_without_empty_line() {
        let source = "# hello world";
        let cursor_byte = 4;
        let mut v = make_view(source);
        v.engine_mut().handle_set_cursor_byte(cursor_byte); // "# he|llo world"

        let aug = v.engine().augment_edit(cursor_byte, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.replace_range, Some(cursor_byte..cursor_byte));
        assert_eq!(aug.cursor_byte_after, cursor_byte + 1);

        let mut edited_source = source.to_owned();
        let replace_range =
            aug.replace_range.expect("heading interior Enter must edit at the current cursor");
        edited_source.replace_range(
            replace_range,
            aug.insert_text
                .as_deref()
                .expect("heading interior Enter must insert one logical newline"),
        );
        assert_eq!(edited_source, "# he\nllo world");
    }

    #[test]
    fn augment_edit_blockquote_middle_inserts_prefix() {
        let mut v = make_view("> hello world");
        v.engine_mut().handle_set_cursor_byte(5); // "> hel|lo world"
        let aug = v.engine().augment_edit(5, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n> "));
        assert_eq!(aug.replace_range, None);
        assert_eq!(aug.cursor_byte_after, 8); // 5 + 3
    }

    #[test]
    fn augment_edit_paragraph_end_inserts_paragraph_break() {
        let source = "hello world";
        let mut v = make_view(source);
        v.engine_mut().handle_set_cursor_byte(source.len());
        let aug = v.engine().augment_edit(source.len(), AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.cursor_byte_after, source.len() + 2);
    }

    #[test]
    fn augment_edit_paragraph_end_before_existing_newline_inserts_paragraph_break() {
        let source = "hello world\n";
        let paragraph_end = "hello world".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(paragraph_end);

        let aug = view.engine.augment_edit(paragraph_end, AugmentKind::Enter).unwrap();
        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, paragraph_end + 2);
    }

    #[test]
    fn augment_edit_paragraph_end_before_next_block_inserts_single_editable_line() {
        let source = "hello world\n\n# Next";
        let paragraph_end = "hello world".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(paragraph_end);

        let aug = view.engine.augment_edit(paragraph_end, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, paragraph_end + 2);
    }

    #[test]
    fn augment_edit_heading_end_before_next_block_places_cursor_on_visible_empty_line() {
        let source = "# Heading\n\nparagraph";
        let heading_end = "# Heading".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(heading_end);

        let aug = view
            .engine
            .augment_edit(heading_end, AugmentKind::Enter)
            .expect("heading-end Enter must create an editable paragraph");

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, heading_end + 2);
    }

    #[test]
    fn heading_enter_before_next_block_cursor_rect_is_visible() {
        let source = "# Heading\n\nparagraph";
        let heading_end = "# Heading".len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(heading_end);

        let aug = view.engine.augment_edit(heading_end, AugmentKind::Enter).unwrap();
        if let Some(insert_text) = aug.insert_text.as_deref() {
            doc.text.replace_range(heading_end..heading_end, insert_text);
        }
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        assert!(
            view.engine().cursor_screen_pos().is_some(),
            "cursor after heading Enter should land on a visible editable empty line"
        );
    }

    #[test]
    fn heading_enter_before_adjacent_block_creates_visible_empty_line() {
        let source = "# Heading\nparagraph";
        let heading_end = "# Heading".len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(heading_end);

        let aug = view.engine.augment_edit(heading_end, AugmentKind::Enter).unwrap();
        doc.text.replace_range(
            heading_end..heading_end,
            aug.insert_text.as_deref().expect("heading Enter should insert a block break"),
        );
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        assert_eq!(doc.text, "# Heading\n\n\nparagraph");
        assert!(
            view.engine().cursor_screen_pos().is_some(),
            "cursor should land on the editable empty line, not the next block"
        );
    }

    #[test]
    fn augment_edit_softbreak_boundary_before_newline_inserts_source_newline() {
        let source = "first line\nsecond line";
        let softbreak_byte = "first line".len();
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(softbreak_byte);

        let aug = view.engine.augment_edit(softbreak_byte, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.cursor_byte_after, softbreak_byte + 2);

        let mut edited_source = source.to_owned();
        edited_source.replace_range(
            softbreak_byte..softbreak_byte,
            aug.insert_text
                .as_deref()
                .expect("paragraph Enter before a soft break must insert a block break"),
        );
        assert_eq!(edited_source, "first line\n\n\nsecond line");
    }

    #[test]
    fn augment_edit_softbreak_boundary_after_newline_inserts_source_newline() {
        let source = "first line\nsecond line";
        let second_line_start = "first line\n".len();
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(second_line_start);

        let aug = view.engine.augment_edit(second_line_start, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, second_line_start + 1);
    }

    #[test]
    fn augment_edit_paragraph_end_uses_current_source_before_next_render() {
        let source = "fresh paragraph";
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(source.len());

        let aug = view.engine.augment_edit(source.len(), AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n\n"));
        assert_eq!(aug.cursor_byte_after, source.len() + 2);
    }

    #[test]
    fn augment_edit_backspace_returns_none() {
        let mut v = make_view("- item");
        v.engine_mut().handle_set_cursor_byte(4);
        assert!(v.engine().augment_edit(4, AugmentKind::Backspace).is_none());
    }

    #[test]
    fn augment_edit_backspace_on_empty_line_between_blocks_keeps_cursor_visible() {
        let source = "first\n\nsecond";
        let first_end = "first".len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(first_end);

        let enter_aug = view
            .engine
            .augment_edit(first_end, AugmentKind::Enter)
            .expect("paragraph-end Enter should create a trailing empty paragraph");
        doc.text.replace_range(
            first_end..first_end,
            enter_aug.insert_text.as_deref().expect("Enter augmentation should insert text"),
        );
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(enter_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        let backspace_aug = view
            .engine
            .augment_edit(enter_aug.cursor_byte_after, AugmentKind::Backspace)
            .expect("backspace on editable empty line should be handled by markdown augmenter");
        let replace_range = backspace_aug
            .replace_range
            .clone()
            .expect("empty-line backspace should delete one newline");
        doc.text.replace_range(replace_range, backspace_aug.insert_text.as_deref().unwrap_or(""));
        view.set_source(doc.text.clone(), 3);
        view.engine.handle_set_cursor_byte(backspace_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        assert_eq!(doc.text, source, "backspace should restore the original block separator");
        assert_eq!(backspace_aug.cursor_byte_after, first_end);
        assert!(
            view.engine.cursor_screen_pos().is_some(),
            "cursor should remain visible after deleting the editable empty line"
        );
    }

    #[test]
    fn augment_edit_backspace_on_trailing_empty_paragraph_removes_blank_line() {
        let source = "first";
        let first_end = source.len();
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);
        view.engine.handle_set_cursor_byte(first_end);

        let enter_aug = view
            .engine
            .augment_edit(first_end, AugmentKind::Enter)
            .expect("paragraph-end Enter should create a trailing empty paragraph");
        doc.text.replace_range(
            first_end..first_end,
            enter_aug.insert_text.as_deref().expect("Enter augmentation should insert text"),
        );
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(enter_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        let insert_aug = view
            .engine
            .augment_edit(enter_aug.cursor_byte_after, AugmentKind::InsertText(String::from("x")))
            .expect("typing into trailing empty paragraph should normalize markdown newlines");
        let insert_range = insert_aug
            .replace_range
            .clone()
            .unwrap_or(enter_aug.cursor_byte_after..enter_aug.cursor_byte_after);
        doc.text.replace_range(insert_range, insert_aug.insert_text.as_deref().unwrap_or(""));
        view.set_source(doc.text.clone(), 3);
        view.engine.handle_set_cursor_byte(insert_aug.cursor_byte_after);
        render_editor_once(&mut view, &doc);

        doc.text.replace_range(insert_aug.cursor_byte_after - 1..insert_aug.cursor_byte_after, "");
        view.set_source(doc.text.clone(), 4);
        view.engine.handle_set_cursor_byte(insert_aug.cursor_byte_after - 1);
        render_editor_once(&mut view, &doc);

        let backspace_aug = view
            .engine
            .augment_edit(insert_aug.cursor_byte_after - 1, AugmentKind::Backspace)
            .expect("backspace on trailing empty paragraph should be handled");
        let replace_range = backspace_aug
            .replace_range
            .clone()
            .expect("trailing empty paragraph backspace should delete source newlines");
        doc.text.replace_range(replace_range, backspace_aug.insert_text.as_deref().unwrap_or(""));

        assert_eq!(doc.text, source, "backspace should remove the trailing empty paragraph");
        assert_eq!(backspace_aug.cursor_byte_after, first_end);
    }

    #[test]
    fn augment_edit_insert_text_in_empty_separator_line() {
        use ui::plugin::AugmentKind;
        let v = make_view("para1\n\npara2");
        let aug = v.engine().augment_edit(6, AugmentKind::InsertText(String::from("A"))).unwrap();

        assert_eq!(aug.replace_range, Some(5..7));
        assert_eq!(aug.insert_text, Some(String::from("\n\nA\n\n")));
        assert_eq!(aug.cursor_byte_after, 5 + 3);
    }

    #[test]
    fn augment_edit_insert_text_in_triple_empty_separator_line() {
        use ui::plugin::AugmentKind;
        let v = make_view("para1\n\n\npara2");
        let aug = v.engine().augment_edit(6, AugmentKind::InsertText(String::from("A"))).unwrap();

        assert_eq!(aug.replace_range, Some(5..8));
        assert_eq!(aug.insert_text, Some(String::from("\n\nA\n\n")));
        assert_eq!(aug.cursor_byte_after, 5 + 3);
    }

    #[test]
    fn augment_edit_tab_returns_none() {
        let mut v = make_view("- item");
        v.engine_mut().handle_set_cursor_byte(4);
        assert!(v.engine().augment_edit(4, AugmentKind::Tab).is_none());
    }

    #[test]
    fn augment_edit_cursor_after_insert_text() {
        let mut v = make_view("- item");
        v.engine_mut().handle_set_cursor_byte(3);
        let aug = v.engine().augment_edit(3, AugmentKind::Enter).unwrap();
        assert_eq!(aug.cursor_byte_after, 3 + "\n- ".len());
    }

    // ── Phase-0 guardrails (2026-07-06 plan) ─────────────────────────────
    // These pin down invariants the L1/L2/L3 refactor must preserve.

    /// L3 guardrail: setting preedit MUST NOT move the source-of-truth cursor.
    /// If this ever fails, IME composition has started overwriting
    /// `edit_ctx.cursor_byte`, which breaks arrow-key navigation during IME.
    #[test]
    fn set_preedit_does_not_move_edit_ctx_cursor() {
        let mut v = make_view("hello world");
        v.engine_mut().handle_set_cursor_byte(5);
        assert_eq!(
            v.engine().edit_ctx.as_ref().map(|c| c.cursor_byte),
            Some(5),
            "precondition: cursor should be at 5"
        );

        v.engine_mut().set_preedit_text("ab".into(), Some((1, 1)));
        assert_eq!(
            v.engine().edit_ctx.as_ref().map(|c| c.cursor_byte),
            Some(5),
            "SetPreedit must not overwrite cursor_byte (source of truth stays on DocumentView)"
        );

        v.engine_mut().set_preedit_text(String::new(), None);
        assert_eq!(
            v.engine().edit_ctx.as_ref().map(|c| c.cursor_byte),
            Some(5),
            "clearing preedit must also leave cursor_byte untouched"
        );
    }

    /// L3 guardrail: setting preedit without a prior cursor MUST NOT fabricate
    /// a phantom cursor at byte 0. Before 2026-07-06, `set_preedit_text` would
    /// synthesize an edit_ctx with cursor_byte=0 whenever no cursor was set,
    /// then visual_move/augment_edit would read this bogus state.
    #[test]
    fn set_preedit_without_prior_cursor_does_not_fabricate_zero_cursor() {
        // Fresh view: no handle_set_cursor_byte issued yet.
        let mut v = make_view("hello world");
        assert!(v.engine().edit_ctx.is_none(), "precondition: no cursor set");

        v.engine_mut().set_preedit_text("中".into(), Some((3, 3)));
        assert!(
            v.engine().edit_ctx.is_none(),
            "preedit with no prior cursor must be dropped, not stored with cursor_byte=0"
        );
    }

    /// L1 guardrail: empty source lines' declared vertical span must fit
    /// between the surrounding rendered flat_lines' rects. Any drift means
    /// `empty_source_line_metrics` and `reserve_extra_blank_source_lines`
    /// have diverged again.
    #[test]
    fn empty_source_line_metrics_align_with_layout() {
        let v = make_view("para1\n\npara2");
        let lazy = v.engine().lazy.as_ref().expect("layout should be built");
        let source = v.engine().edit_source.as_deref().expect("edit source should be set");

        // "para1\n\npara2" — byte 6 is the empty separator line (start == end).
        let source_line = source_line_at_byte(source, 6).expect("byte 6 should map to a line");
        assert!(source_line.is_empty(), "byte 6 should sit on an empty source line");

        let (_, line_top, _, line_height) =
            v.engine().empty_source_line_metrics(source_line, lazy, source);
        let line_bottom = line_top + line_height;

        // Find the surrounding rendered lines.
        let (prev, next) = v.engine().surrounding_rendered_lines(source_line, lazy);
        let (_, prev_fl) = prev.expect("first paragraph should be laid out");
        let (_, next_fl) = next.expect("second paragraph should be laid out");
        let prev_bottom = prev_fl.rect.y + prev_fl.rect.h;
        let next_top = next_fl.rect.y;

        assert!(
            line_top >= prev_bottom - 0.5,
            "empty line top ({line_top}) must not overlap previous flat_line bottom ({prev_bottom})"
        );
        assert!(
            line_bottom <= next_top + 0.5,
            "empty line bottom ({line_bottom}) must not overlap next flat_line top ({next_top})"
        );
    }

    // ── visual_move ──────────────────────────────────────────────────────

    #[test]
    fn visual_move_left_at_byte_zero() {
        let v = make_view("hello");
        assert_eq!(v.engine().visual_move(0, MoveDirection::Left, None), Some(0));
    }

    #[test]
    fn visual_move_right_at_end_returns_current() {
        let v = make_view("ab");
        // Block text is "ab" at source bytes 0..2; last char is byte 2.
        let result = v.engine().visual_move(2, MoveDirection::Right, None);
        assert_eq!(result, Some(2));
    }

    #[test]
    fn visual_move_right_advances_one_utf8_char() {
        let v = make_view("- item");
        // Block text "item" starts at source byte 2; byte 2 → byte 3.
        let result = v.engine().visual_move(2, MoveDirection::Right, None);
        assert_eq!(result, Some(3));
    }

    #[test]
    fn visual_move_left_retreats_one_utf8_char() {
        let v = make_view("- item");
        // byte 3 (second char of "item") → byte 2 (first char of "item").
        let result = v.engine().visual_move(3, MoveDirection::Left, None);
        assert_eq!(result, Some(2));
    }

    #[test]
    fn visual_move_right_skips_combining_mark() {
        let source = "**e\u{0301}x**";
        let mut v = make_view(source);
        v.engine_mut().handle_set_cursor_byte(2);

        let result = v.engine().visual_move(2, MoveDirection::Right, None);

        // Should skip the entire é grapheme (3 bytes) and land at byte 5 (x).
        assert_eq!(result, Some(2 + "e\u{0301}".len()));
    }

    #[test]
    fn visual_move_right_skips_zwj_emoji_cluster() {
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let source = format!("**{emoji}x**");
        let mut v = make_view(&source);
        v.engine_mut().handle_set_cursor_byte(2);

        let result = v.engine().visual_move(2, MoveDirection::Right, None);

        // Should skip the entire ZWJ emoji cluster and land at byte 2 + emoji.len().
        assert_eq!(result, Some(2 + emoji.len()));
    }

    #[test]
    fn promotion_blockquote_left_right_traversal_is_ordered_and_terminates() {
        let source = "> first physical line\n> second physical line";
        let view = make_view(source);
        let mut byte = source.find("second").expect("fixture must contain second");
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..=source.len() * 2 {
            assert!(visited.insert(byte), "horizontal navigation must not loop at byte {byte}");
            let next =
                view.engine().visual_move(byte, MoveDirection::Left, None).expect("move left");
            if next == 0 {
                return;
            }
            byte = next;
        }
        panic!("left traversal did not reach document start");
    }

    #[test]
    fn virtual_preedit_roundtrip_keeps_committed_source_byte() {
        let source = "ab";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(1);
        view.engine.set_preedit_text("中文".to_string(), Some((3, 3)));
        render_editor_once(&mut view, &doc);

        let _index = view.engine.projection_index();
        let rect = view.engine().cursor_screen_pos().expect("preedit cursor rect");
        let hit = view.engine().hit_test_byte(rect.0, rect.1 + rect.3 * 0.5, 0.0, 0.0);
        assert_eq!(hit, Some(1));
    }

    // ── cursor_in_span (PreviewEngine wrapper) ───────────────────────────

    #[test]
    fn cursor_in_span_inside_returns_true() {
        let mut v = make_view("**bold**");
        // Bold marker "**" is 2 bytes; text "bold" is source bytes 2..6.
        let span = crate::builder::StyleSpan {
            start: 0,
            len: 4,
            style: crate::builder::InlineStyle::Bold,
            source_range: std::ops::Range { start: 2, end: 6 },
        };
        v.engine_mut().handle_set_cursor_byte(3);
        assert!(v.engine().cursor_in_span(&span));
    }

    #[test]
    fn cursor_in_span_outside_returns_false() {
        let mut v = make_view("**bold** text");
        let span = crate::builder::StyleSpan {
            start: 0,
            len: 4,
            style: crate::builder::InlineStyle::Bold,
            source_range: std::ops::Range { start: 2, end: 6 },
        };
        v.engine_mut().handle_set_cursor_byte(8);
        assert!(!v.engine().cursor_in_span(&span));
    }

    #[test]
    fn cursor_in_span_no_edit_ctx_returns_false() {
        let v = make_view("**bold**");
        let span = crate::builder::StyleSpan {
            start: 0,
            len: 4,
            style: crate::builder::InlineStyle::Bold,
            source_range: std::ops::Range { start: 2, end: 6 },
        };
        assert!(!v.engine().cursor_in_span(&span));
    }

    // ── byte mapping: expanded span roundtrip ──────────────────────────

    #[test]
    fn hit_test_byte_roundtrip_at_expanded_cursor_position() {
        // "**bold**" → bold span is expanded when cursor is inside.
        // Source: "**bold**" = 8 bytes; bold text "bold" is at bytes 2..6.
        let mut v = make_view("**bold**");
        v.engine_mut().handle_set_cursor_byte(3); // cursor inside "bold"

        let pos = v.engine().cursor_screen_pos().unwrap();
        let (cx, cy, _cw, ch) = pos;
        // hit_test_byte roundtrip: screen pos → byte should map back to 3.
        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        assert_eq!(result, Some(3), "roundtrip at expanded cursor should return same byte");
    }

    #[test]
    fn visual_move_within_single_line_expanded_span() {
        // When cursor is inside a bold span, the span is expanded to show markers.
        // visual_move Right should advance one source byte within the expanded text.
        let mut v = make_view("**bold**");
        v.engine_mut().handle_set_cursor_byte(3); // inside "bold"

        let result = v.engine().visual_move(3, MoveDirection::Right, None);
        assert_eq!(result, Some(4), "Right inside expanded span should advance one byte");
    }

    #[test]
    fn hit_test_byte_at_end_of_single_line_expanded_span() {
        // Cursor at byte 5 (last char of "bold" in "**bold**")
        let mut v = make_view("**bold** text");
        v.engine_mut().handle_set_cursor_byte(5);

        let pos = v.engine().cursor_screen_pos().unwrap();
        let (cx, cy, _cw, ch) = pos;
        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        // Should map back to a byte within the bold text range
        assert!(result.is_some(), "hit_test at expanded span tail should return a byte");
    }

    #[test]
    fn hit_test_byte_roundtrip_second_plain_paragraph() {
        // Regression test for P1: plain-text source maps starting from 0.
        // Two plain paragraphs: "first" at bytes 0..5, "second" at bytes 7..13.
        // Cursor at byte 7 (start of "second") must roundtrip correctly.
        let mut v = make_view("first\n\nsecond");
        v.engine_mut().handle_set_cursor_byte(7);

        let pos = v
            .engine()
            .cursor_screen_pos()
            .expect("cursor at byte 7 (start of second paragraph) should resolve to screen pos");
        let (cx, cy, _cw, ch) = pos;

        // Cursor on second paragraph: y should be > first line height
        assert!(cy > 20.0, "second paragraph cursor y ({cy}) should be below first line");

        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        assert_eq!(result, Some(7), "roundtrip should return byte 7 for second plain paragraph");
    }

    #[test]
    fn extra_empty_source_line_reserves_vertical_space_before_next_paragraph() {
        let baseline = make_editor_view_with_cursor("first\n\nsecond", 7);
        let expanded = make_editor_view_with_cursor("first\n\n\nsecond", 7);

        let baseline_second_y = flat_line_y_for_source_byte(&baseline, 7)
            .expect("baseline second paragraph should be laid out");
        let expanded_second_y = flat_line_y_for_source_byte(&expanded, 8)
            .expect("expanded second paragraph should be laid out");

        assert!(
            expanded_second_y >= baseline_second_y + baseline.engine.base_line_height - 1.0,
            "extra empty source line should push following paragraph down by one line: \
             baseline={baseline_second_y}, expanded={expanded_second_y}"
        );
    }

    #[test]
    fn editable_empty_line_between_paragraphs_keeps_spacing_on_both_sides() {
        let source = "first\n\n\nsecond";
        let second_start = source.find("second").expect("fixture should contain second paragraph");
        let view = make_editor_view_with_cursor(source, second_start);
        let first_line = view.engine().flat_lines().first().expect("first paragraph should render");
        let second_y = flat_line_y_for_source_byte(&view, second_start)
            .expect("second paragraph should render");
        let actual_gap = second_y - (first_line.rect.y + first_line.rect.h);
        let expected_gap = view.engine().paragraph_spacing * 2.0 + view.engine().base_line_height;

        assert!(
            (actual_gap - expected_gap).abs() < 1.0,
            "an editable blank paragraph needs paragraph spacing on both sides: \
             actual={actual_gap}, expected={expected_gap}"
        );
    }

    #[test]
    fn editable_empty_line_before_list_does_not_add_paragraph_spacing() {
        let baseline_source = "first\n\n- item";
        let expanded_source = "first\n\n\n- item";
        let baseline_list_start =
            baseline_source.find("- item").expect("fixture should contain list");
        let expanded_list_start =
            expanded_source.find("- item").expect("fixture should contain list");
        let baseline = make_editor_view_with_cursor(baseline_source, baseline_list_start);
        let expanded = make_editor_view_with_cursor(expanded_source, expanded_list_start);
        let baseline_y = flat_line_y_for_source_byte(&baseline, baseline_list_start)
            .expect("list should render");
        let expanded_y = flat_line_y_for_source_byte(&expanded, expanded_list_start)
            .expect("list should render");

        assert!(
            ((expanded_y - baseline_y) - baseline.engine().base_line_height).abs() < 1.0,
            "a retained empty line before a list should add only its line height: \
             baseline={baseline_y}, expanded={expanded_y}"
        );
    }

    #[test]
    fn source_update_with_new_empty_line_reflows_next_paragraph_immediately() {
        let mut view = MarkdownEditorView::new();
        let mut doc = StubDoc::new("first\n\nsecond");
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let baseline_second_y = flat_line_y_for_source_byte(&view, 7)
            .expect("baseline second paragraph should be laid out");

        doc.text = String::from("first\n\n\nsecond");
        view.set_source(doc.text.clone(), 2);
        view.engine.handle_set_cursor_byte(7);
        render_editor_once(&mut view, &doc);

        let expanded_second_y = flat_line_y_for_source_byte(&view, 8)
            .expect("expanded second paragraph should be laid out after source update");

        assert!(
            expanded_second_y >= baseline_second_y + view.engine.base_line_height - 1.0,
            "source update should rebuild layout in the next render, not wait for cursor blink: \
             baseline={baseline_second_y}, expanded={expanded_second_y}"
        );
    }

    #[test]
    fn editable_empty_source_line_between_paragraphs_uses_full_line_height() {
        let source = "hello\n\n\nnext";
        let cursor_byte = "hello\n\n".len();
        let next_byte = "hello\n\n\n".len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);

        let source_line =
            source_line_at_byte(source, cursor_byte).expect("cursor should be on an empty line");
        let lazy = view.engine.lazy.as_ref().expect("layout should be built");
        let (_x, line_top, _font_size, line_height) =
            view.engine.empty_source_line_metrics(source_line, lazy, source);
        let next_y = flat_line_y_for_source_byte(&view, next_byte)
            .expect("following paragraph should be laid out");

        assert!(
            (line_height - view.engine.base_line_height).abs() < 1.0,
            "editable empty line should use full line height, got {line_height}"
        );
        assert!(
            next_y >= line_top + line_height - 1.0,
            "following line should stay below the editable empty line: top={line_top}, \
             height={line_height}, next_y={next_y}"
        );
    }

    #[test]
    fn cursor_after_paragraph_enter_uses_paragraph_spacing_immediately() {
        let source = "hello\n\n";
        let cursor_byte = source.len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);

        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("cursor on the newly inserted paragraph line should be visible");
        let first_line =
            view.engine().flat_lines().first().expect("first paragraph line should be laid out");
        let cursor_doc_y = cursor_y + view.engine.scroll_y;
        let cursor_top_in_line = cursor_height * (1.0 - WYSIWYG_CURSOR_ASCENT_RATIO);
        let cursor_line_top = cursor_doc_y - cursor_top_in_line;
        let expected_line_top =
            first_line.rect.y + first_line.rect.h + view.engine.paragraph_spacing;

        assert_eq!(cursor_x, first_line.rect.x, "empty paragraph cursor should keep line x");
        assert!(
            (cursor_line_top - expected_line_top).abs() < 1.0,
            "new paragraph cursor should include paragraph spacing immediately: \
             got line_top={cursor_line_top}, expected={expected_line_top}"
        );
    }

    #[test]
    fn leading_empty_source_lines_reserve_vertical_space_before_first_block() {
        let baseline = make_editor_view_with_cursor("## title", 0);
        let expanded = make_editor_view_with_cursor("\n\n\n## title", 3);

        let baseline_heading_y =
            flat_line_y_for_source_byte(&baseline, 0).expect("baseline heading should be laid out");
        let expanded_heading_y =
            flat_line_y_for_source_byte(&expanded, 3).expect("expanded heading should be laid out");

        assert!(
            expanded_heading_y >= baseline_heading_y + baseline.engine.base_line_height * 3.0 - 1.0,
            "leading empty source lines should push first block down: \
             baseline={baseline_heading_y}, expanded={expanded_heading_y}"
        );
    }

    #[test]
    fn cursor_after_trailing_newline_moves_to_empty_paragraph_line() {
        let source = "hello\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.engine.handle_set_cursor_byte(source.len());
        let (_x, cursor_y, _w, _h) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve to the new paragraph line");

        assert!(
            cursor_y > 24.0,
            "cursor after trailing newline should be below old paragraph, got y={cursor_y}"
        );
    }

    #[test]
    fn cursor_after_single_trailing_newline_uses_one_line_gap() {
        let source = "hello\n";
        let cursor_byte = source.len();
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);

        let (_cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve");
        let first_line =
            view.engine().flat_lines().first().expect("first paragraph line should be laid out");
        let cursor_doc_y = cursor_y + view.engine.scroll_y;
        let cursor_top_in_line = cursor_height * (1.0 - WYSIWYG_CURSOR_ASCENT_RATIO);
        let cursor_line_top = cursor_doc_y - cursor_top_in_line;
        let expected_line_top = first_line.rect.y + first_line.rect.h;

        assert!(
            (cursor_line_top - expected_line_top).abs() < 1.0,
            "cursor after a single trailing newline should advance by one line only: \
             got line_top={cursor_line_top}, expected={expected_line_top}"
        );
    }

    #[test]
    fn cursor_after_paragraph_end_enter_y_position() {
        let source = "hello\n\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        // Act as if we pressed enter at end of paragraph and \n\n was inserted
        view.engine.handle_set_cursor_byte(source.len());
        let (_x, cursor_y, _w, cursor_height) =
            view.engine.cursor_screen_pos().expect("cursor should resolve");

        let baseline_y = cursor_y + cursor_height * WYSIWYG_CURSOR_ASCENT_RATIO;

        // the previous line's bottom is roughly base_line_height (24.0)
        // the new line's top should be prev_bottom + paragraph_spacing (24.0 + 12.0 = 36.0)
        // the new line's baseline is top + cursor_height
        let expected_top = view.engine.base_line_height + view.engine.paragraph_spacing;
        assert!(
            baseline_y >= expected_top && baseline_y <= expected_top + cursor_height,
            "cursor baseline_y should be exactly paragraph_spacing below the first line (expected ~{}, got {})",
            expected_top + cursor_height,
            baseline_y
        );
    }

    #[test]
    fn enter_on_empty_block_separator_keeps_inserting_source_newlines() {
        let source = "para1\n\npara2";
        let ctx = classify_enter_context(source, 6);
        match ctx {
            EnterContext::EmptyBlockSeparatorLine => {}
            _ => panic!("Expected EmptyBlockSeparatorLine at byte 6, got {:?}", ctx),
        }

        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(6);

        let aug = view.engine.augment_edit(6, AugmentKind::Enter).unwrap();

        assert_eq!(aug.insert_text.as_deref(), Some("\n"));
        assert_eq!(aug.cursor_byte_after, 7);
    }

    #[test]
    fn cursor_after_trailing_newline_uses_physical_line_metrics() {
        let source = "hello\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        view.engine.base_font_size = 15.0;
        view.engine.base_line_height = 24.0;
        render_editor_viewport_with_dpi(&mut view, &doc, 800.0, 600.0, 2.0);

        view.engine.handle_set_cursor_byte(source.len());
        let (_x, cursor_y, _w, cursor_height) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve to the new paragraph line");

        assert!(
            cursor_y > 48.0,
            "2x cursor after trailing newline should advance by a physical line, got y={cursor_y}"
        );
        assert!(
            cursor_height > 24.0,
            "2x cursor height should use physical font metrics, got h={cursor_height}"
        );
    }

    #[test]
    fn cursor_after_trailing_newline_roundtrips_to_empty_line_byte() {
        let source = "hello\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.engine.handle_set_cursor_byte(source.len());
        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing newline should resolve to the new source line");

        let hit_byte =
            view.engine.hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0);

        assert_eq!(
            hit_byte,
            Some(source.len()),
            "cursor rect on the empty source line should map back to the empty line byte"
        );
    }

    #[test]
    fn selection_endpoint_after_trailing_newline_preserves_empty_line_byte() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut doc);

        let selection_range = match view.query(PluginQuery::SelectionRange, &doc) {
            PluginResponse::PositionPair(range) => range,
            other => panic!("expected selection range, got {other:?}"),
        };

        assert_eq!(
            selection_range,
            Some(((0, 0), (source.len(), 0))),
            "selection endpoint on trailing empty line should preserve the source end byte"
        );
    }

    #[test]
    fn backward_selection_from_trailing_newline_preserves_empty_line_anchor() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len())), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(0)), &mut doc);

        let selection_range = match view.query(PluginQuery::SelectionRange, &doc) {
            PluginResponse::PositionPair(range) => range,
            other => panic!("expected selection range, got {other:?}"),
        };

        assert_eq!(
            selection_range,
            Some(((0, 0), (source.len(), 0))),
            "backward selection from trailing empty line should preserve the source end byte"
        );
    }

    #[test]
    fn set_selection_byte_clears_visual_endpoint_when_byte_cannot_map() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);

        view.handle_message(PluginMessage::SetSelCursor(Some((0, 1))), &mut doc);
        assert!(matches!(
            view.query(PluginQuery::SelCursor, &doc),
            PluginResponse::Position(Some((0, 1)))
        ));

        view.handle_message(PluginMessage::SetSelCursorByte(Some(0)), &mut doc);

        assert!(matches!(view.query(PluginQuery::SelCursor, &doc), PluginResponse::Position(None)));
    }

    #[test]
    fn cursor_screen_pos_query_uses_requested_byte() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "abc\n\ndef";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        let first = match view.query(PluginQuery::CursorScreenPos(0), &doc) {
            PluginResponse::CursorScreenRect(Some(rect)) => rect,
            response => panic!("expected first cursor rect, got {response:?}"),
        };
        let second = match view.query(PluginQuery::CursorScreenPos(5), &doc) {
            PluginResponse::CursorScreenRect(Some(rect)) => rect,
            response => panic!("expected second cursor rect, got {response:?}"),
        };

        assert_ne!(first.1, second.1, "different requested bytes should resolve different rows");
    }

    #[test]
    fn trailing_newline_selection_keeps_range_without_empty_line_highlight() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len() - 1)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut doc);

        assert!(
            matches!(view.query(PluginQuery::HasSelection, &doc), PluginResponse::Bool(true)),
            "trailing newline byte range should count as an active selection"
        );

        let selection_range = match view.query(PluginQuery::SelectionRange, &doc) {
            PluginResponse::PositionPair(range) => range,
            other => panic!("expected selection range, got {other:?}"),
        };
        assert_eq!(
            selection_range,
            Some(((source.len() - 1, 0), (source.len(), 0))),
            "trailing newline selection should preserve the source byte range"
        );

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let has_empty_line_highlight = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. } if rect.y > view.engine.base_line_height * 0.5
            )
        });

        assert!(
            !has_empty_line_highlight,
            "trailing newline selection should not synthesize an empty-line highlight"
        );
    }

    #[test]
    fn select_all_highlights_last_physical_line_before_trailing_blanks() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "\
first physical line
second physical line
third physical line
final visible physical line

";
        let final_line_byte =
            source.find("final visible").expect("fixture should contain final line");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(final_line_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(final_line_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on final physical line, got {other:?}"),
            };

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(0)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut doc);

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let final_line_is_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_y + cursor_height * 0.5
                        && rect.y + rect.h >= cursor_y + cursor_height * 0.5
            )
        });

        assert!(
            final_line_is_highlighted,
            "select-all should visibly highlight the final physical text line"
        );
    }

    #[test]
    fn selecting_real_last_physical_text_line_reports_highlight() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "\
C608-03 武昌职业第03组：2025 最低 389，且是民办、计划 4 人，不应太靠前，除非非常想去海军水面方向。
C503-01、T105-01：疑似非军士，建议后置或剔除。
C501 武汉船舶、C523 湖北交通、C537 武汉交通：如果体检类别匹配，本地公办、计划较多，价值更高，应该优先级更清晰。
C608-01 武昌职业第01组：计划 68，历史低线较低，是表里最像“保底”的组之一，但民办学费高，要接受成本。


";
        let line_start = source.find("C608-01").expect("fixture should contain final text line");
        let line_end = source[line_start..]
            .find('\n')
            .map(|offset| line_start + offset)
            .expect("fixture should end final text line with newline");
        let probe_byte = source.find("要接受成本").expect("fixture should contain probe text");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(probe_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(probe_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on final text line, got {other:?}"),
            };

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(line_start)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(line_end)), &mut doc);

        assert!(
            matches!(view.query(PluginQuery::HasSelection, &doc), PluginResponse::Bool(true)),
            "the final physical text line byte range should count as an active selection"
        );

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let final_line_is_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_y + cursor_height * 0.5
                        && rect.y + rect.h >= cursor_y + cursor_height * 0.5
            )
        });

        assert!(
            final_line_is_highlighted,
            "selection should visibly highlight the real final physical text line"
        );
    }

    #[test]
    fn backward_selection_from_trailing_blank_highlights_real_final_text_line() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "\
C608-03 武昌职业第03组：2025 最低 389，且是民办、计划 4 人，不应太靠前，除非非常想去海军水面方向。
C503-01、T105-01：疑似非军士，建议后置或剔除。
C501 武汉船舶、C523 湖北交通、C537 武汉交通：如果体检类别匹配，本地公办、计划较多，价值更高，应该优先级更清晰。
C608-01 武昌职业第01组：计划 68，历史低线较低，是表里最像“保底”的组之一，但民办学费高，要接受成本。


";
        let probe_byte = source.find("要接受成本").expect("fixture should contain probe text");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(probe_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(probe_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on final text line, got {other:?}"),
            };

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len())), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(probe_byte)), &mut doc);

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let final_line_is_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_y + cursor_height * 0.5
                        && rect.y + rect.h >= cursor_y + cursor_height * 0.5
            )
        });

        assert!(
            final_line_is_highlighted,
            "backward selection from trailing blank should highlight the final text line"
        );
    }

    #[test]
    fn selection_highlight_at_bottom_respects_render_offset_y() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "top line\nbottom line\n\n";
        let bottom_byte = source.find("bottom").expect("fixture should contain bottom line");
        let line_end = source[bottom_byte..]
            .find('\n')
            .map(|offset| bottom_byte + offset)
            .expect("fixture should end bottom line with newline");
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.handle_message(PluginMessage::Scroll { delta: 48.0, viewport_h: 48.0 }, &mut doc);
        render_editor_with_offset_y(&mut view, &doc, 48.0, 80.0);

        view.handle_message(PluginMessage::SetCursorByte(bottom_byte), &mut doc);
        let (_cursor_x, cursor_y, _cursor_width, cursor_height) =
            match view.query(PluginQuery::CursorScreenPos(bottom_byte), &doc) {
                PluginResponse::CursorScreenRect(Some(rect)) => rect,
                other => panic!("expected cursor rect on bottom line, got {other:?}"),
            };
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(bottom_byte)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(line_end)), &mut doc);

        let highlights =
            match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
                PluginResponse::DrawList(draw_list) => draw_list,
                other => panic!("expected selection highlights, got {other:?}"),
            };
        let cursor_screen_y = 80.0 + cursor_y + cursor_height * 0.5;
        let bottom_line_highlighted = highlights.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, .. }
                    if rect.y <= cursor_screen_y && rect.y + rect.h >= cursor_screen_y
            )
        });

        assert!(
            bottom_line_highlighted,
            "selection highlight near viewport bottom should not be clipped by offset_y"
        );
    }

    #[test]
    fn blockquote_marker_byte_selection_highlight_uses_constant_source_line_lookups() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut source = String::new();
        for index in 0..180 {
            source.push_str(&format!("paragraph {index}\n\n"));
            if index == 40 || index == 140 {
                source.push_str(&format!("> quoted block {index}\n\n"));
            }
        }
        let first_quote = source.find("> quoted block 40").expect("fixture should contain quote");
        let second_quote = source.find("> quoted block 140").expect("fixture should contain quote");
        let mut doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);
        render_editor_once(&mut view, &doc);

        reset_source_line_at_byte_call_count();
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(first_quote)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(second_quote)), &mut doc);
        match view.query(PluginQuery::SelectionHighlights([0.1, 0.2, 0.3, 1.0]), &doc) {
            PluginResponse::DrawList(_) => {}
            other => panic!("expected selection highlights, got {other:?}"),
        }

        assert!(
            source_line_at_byte_call_count() <= 4,
            "blockquote marker byte fallback should not rescan source lines per mapped byte; calls={}",
            source_line_at_byte_call_count()
        );
    }

    #[test]
    fn cursor_after_trailing_blank_line_uses_nearby_paragraph_position() {
        let mut source =
            (0..40).map(|idx| format!("paragraph {idx}")).collect::<Vec<_>>().join("\n\n");
        source.push_str("\n\n");
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(&source);
        view.set_source(source.clone(), 1);
        render_editor_once(&mut view, &doc);

        let previous_line = view
            .engine
            .flat_lines()
            .last()
            .expect("fixture should render a previous paragraph")
            .clone();

        view.engine.handle_set_cursor_byte(source.len());
        let (_cursor_x, cursor_y, _cursor_width, _cursor_height) = view
            .engine
            .cursor_screen_pos()
            .expect("cursor after trailing blank line should resolve near previous paragraph");

        let max_expected_y = previous_line.rect.y + previous_line.rect.h * 3.0;
        assert!(
            cursor_y < max_expected_y,
            "cursor y should be near previous paragraph, got {cursor_y}, previous={:?}",
            previous_line.rect
        );
    }

    #[test]
    fn markdown_editor_reflows_immediately_when_viewport_width_changes() {
        let source = "这是一段很长的中文测试文本 mixed with english and some more text that \
                      should wrap narrowly but fit into fewer lines in a wide editor viewport.";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);

        render_editor_narrow(&mut view, &doc, 180.0);
        let narrow_line_count = view.engine().flat_lines().len();

        render_editor_narrow(&mut view, &doc, 800.0);
        let wide_line_count = view.engine().flat_lines().len();

        assert!(
            wide_line_count < narrow_line_count,
            "editor should reflow on width change without requiring scroll: narrow={narrow_line_count}, wide={wide_line_count}"
        );
    }

    #[test]
    fn hit_test_byte_after_horizontal_rule_keeps_adjacent_paragraphs_distinct() {
        let source = "# 党内法规发展历程回眸\n\n**作者：林希存**\n> 栏目：党内法规\n---\n第六行普通段落\n\n第八行普通段落";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let flat_lines = view.engine().flat_lines();
        let sixth_line = flat_lines
            .iter()
            .find(|line| line.text.contains("第六行普通段落"))
            .expect("rendered flat lines should contain source line 6 paragraph");
        let eighth_line = flat_lines
            .iter()
            .find(|line| line.text.contains("第八行普通段落"))
            .expect("rendered flat lines should contain source line 8 paragraph");

        let sixth_start = source.find("第六行普通段落").expect("source should contain line 6");
        let sixth_end = sixth_start + "第六行普通段落".len();
        let eighth_start = source.find("第八行普通段落").expect("source should contain line 8");
        let eighth_end = eighth_start + "第八行普通段落".len();

        let sixth_hit = view
            .engine()
            .hit_test_byte(
                sixth_line.rect.x + 1.0,
                sixth_line.rect.y + sixth_line.rect.h * 0.5,
                0.0,
                0.0,
            )
            .expect("hit-test on source line 6 should return a byte");
        let eighth_hit = view
            .engine()
            .hit_test_byte(
                eighth_line.rect.x + 1.0,
                eighth_line.rect.y + eighth_line.rect.h * 0.5,
                0.0,
                0.0,
            )
            .expect("hit-test on source line 8 should return a byte");

        assert!(
            (sixth_start..=sixth_end).contains(&sixth_hit),
            "click on source line 6 mapped to byte {sixth_hit}, expected {sixth_start}..={sixth_end}"
        );
        assert!(
            (eighth_start..=eighth_end).contains(&eighth_hit),
            "click on source line 8 mapped to byte {eighth_hit}, expected {eighth_start}..={eighth_end}"
        );
    }

    #[test]
    fn wrapped_bold_line_end_returns_correct_segment_end() {
        // LineEnd on a wrapped expanded line goes to the end of the current
        // visual segment (not the first segment of the logical line).
        // find_flat_and_grapheme_for_byte correctly identifies the segment
        // containing the cursor byte via per-segment exact matching.
        let long_text = "**".to_string() + &"word ".repeat(30) + "**";
        let mut v = make_view(&long_text);
        // Place cursor somewhere in the middle of the expanded span
        v.engine_mut().handle_set_cursor_byte(10);

        let result = v.engine().visual_move(10, MoveDirection::LineEnd, None);
        assert!(result.is_some(), "LineEnd on wrapped expanded line should return Some");
        let end_byte = result.unwrap();
        // The byte should be >= current position
        assert!(end_byte >= 10, "LineEnd byte should be >= current position");

        // Roundtrip: cursor_screen_pos at end_byte should hit back to end_byte
        v.engine_mut().handle_set_cursor_byte(end_byte);
        let (x, y, _w, h) = v.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = v.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);
        assert_eq!(hit, Some(end_byte), "LineEnd byte roundtrip should return same byte");
    }

    // ── visual_move: Up/Down across lines ───────────────────────────────

    #[test]
    fn visual_move_down_from_first_line() {
        let v = make_view("line one\nline two");
        // Start at byte 0 (first char of first line)
        let result = v.engine().visual_move(0, MoveDirection::Down, None);
        // Down should move to second line
        assert!(result.is_some(), "Down from first line should return Some");
    }

    #[test]
    fn visual_move_up_from_first_line_returns_same() {
        let v = make_view("hello");
        let result = v.engine().visual_move(0, MoveDirection::Up, None);
        // Up from first line should stay at same position
        assert_eq!(result, Some(0), "Up from first line should stay at byte 0");
    }

    #[test]
    fn visual_move_at_outer_visual_rows_reaches_document_boundaries() {
        let source = "hello\nworld";
        let view = make_view(source);

        assert_eq!(view.engine().visual_move(3, MoveDirection::Up, None), Some(0));
        assert_eq!(
            view.engine().visual_move(source.len() - 2, MoveDirection::Down, None),
            Some(source.len())
        );
    }

    #[test]
    fn projected_empty_line_screen_x_preserves_surrounding_indent() {
        let source = "> quoted\n\n";
        let view = make_view(source);
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(source.len(), CursorAffinity::Downstream)
            .expect("trailing empty line should have a projection position");
        let previous_line_x =
            view.engine().flat_lines().last().expect("quoted paragraph should render").rect.x;

        assert!(previous_line_x > 0.0, "fixture must provide a non-zero indentation");
        let lazy = view.engine().lazy.as_ref().expect("view should retain lazy layout state");
        assert_eq!(view.engine().projection_screen_x(lazy, position), Some(previous_line_x));
    }

    #[test]
    fn unshaped_navigation_preserves_proportional_font_advances() {
        let mut view = make_projected_view("Wi");
        view.engine_mut().lazy.as_mut().expect("projected view should retain layout").flat_lines
            [0]
        .shaped = None;
        let line = &view.engine().flat_lines()[0];
        let after_w = view.engine().grapheme_x_for_line(line, 1);
        let after_i = view.engine().grapheme_x_for_line(line, 2);
        let wide_advance = after_w;
        let narrow_advance = after_i - after_w;

        assert!(
            wide_advance > narrow_advance * 1.5,
            "unshaped navigation must retain proportional advances: W={wide_advance}, i={narrow_advance}"
        );
    }

    #[test]
    fn unshaped_code_navigation_preserves_monospace_advances() {
        let source = "```\nWi\n```";
        let mut view = make_projected_view(source);
        let code_line_index = view
            .engine()
            .flat_lines()
            .iter()
            .position(|line| line.text == "Wi")
            .expect("code content line should render");
        view.engine_mut().lazy.as_mut().expect("projected view should retain layout").flat_lines
            [code_line_index]
            .shaped = None;
        let line = &view.engine().flat_lines()[code_line_index];
        let after_w = view.engine().grapheme_x_for_line(line, 1);
        let after_i = view.engine().grapheme_x_for_line(line, 2);
        let wide_advance = after_w;
        let narrow_advance = after_i - after_w;

        assert!(
            (wide_advance - narrow_advance).abs() < wide_advance * 0.15,
            "unshaped code navigation must retain monospace advances: W={wide_advance}, i={narrow_advance}"
        );
    }

    #[test]
    fn visual_move_line_start_and_end() {
        let v = make_view("- list item");
        // byte 2 is 'l' (first char of "list item")
        let start = v.engine().visual_move(5, MoveDirection::LineStart, None);
        assert!(start.is_some(), "LineStart should return Some");
        let end = v.engine().visual_move(5, MoveDirection::LineEnd, None);
        assert!(end.is_some(), "LineEnd should return Some");
        // LineStart should be before LineEnd
        assert!(start.unwrap() <= end.unwrap(), "LineStart <= LineEnd");
    }

    // ── MarkdownEditorView (ViewPlugin impl) ─────────────────────────

    struct StubDoc {
        text: String,
    }

    impl StubDoc {
        fn new(text: &str) -> Self {
            Self { text: text.to_string() }
        }
    }

    impl core::document::DocView for StubDoc {
        fn line_count(&self) -> usize {
            self.text.lines().count().max(1)
        }

        fn doc_line_text(&self, line: usize) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Owned(self.text.lines().nth(line).unwrap_or("").to_string())
        }

        fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> std::borrow::Cow<'_, str> {
            let start = range.start.min(self.text.len());
            let end = range.end.min(self.text.len());
            std::borrow::Cow::Owned(self.text[start..end].to_string())
        }

        fn line_byte_offset(&self, line: usize) -> usize {
            let mut byte_offset = 0usize;
            for (idx, segment) in self.text.split_inclusive('\n').enumerate() {
                if idx == line {
                    return byte_offset;
                }
                byte_offset += segment.len();
            }
            self.text.len()
        }

        fn line_byte_length(&self, line: usize) -> usize {
            self.text.lines().nth(line).map(|s| s.len()).unwrap_or(0)
        }

        fn scroll_y(&self) -> f32 {
            0.0
        }

        fn viewport_height(&self) -> f32 {
            600.0
        }
    }

    impl core::document::DocViewMut for StubDoc {
        fn set_scroll_y(&mut self, _y: f32) {}

        fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
            self.text.replace_range(range, text);
        }
    }

    fn render_editor_once(view: &mut MarkdownEditorView, doc: &StubDoc) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn render_editor_with_offset_y(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        height: f32,
        offset_y: f32,
    ) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, offset_y, 800.0, height);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn make_editor_view_with_cursor(source: &str, cursor_byte: usize) -> MarkdownEditorView {
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        view.engine.handle_set_cursor_byte(cursor_byte);
        render_editor_once(&mut view, &doc);
        view
    }

    fn render_editor_draw_list(view: &mut MarkdownEditorView, doc: &StubDoc) -> DrawList {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0)
    }

    fn render_editor_draw_list_with_dpi(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        dpi_scale: f32,
    ) -> DrawList {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        <MarkdownEditorView as ViewPlugin>::render(
            view,
            doc,
            bounds,
            &theme,
            &mut shaper,
            dpi_scale,
        )
    }

    fn editor_cursor_rect_from_draw_list(dl: &DrawList) -> ui::core::geom::Rect {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        dl.cmds
            .iter()
            .filter_map(|cmd| match cmd {
                ui::core::paint::DrawCmd::FillRect { rect, color, .. }
                    if *color == theme.editor.cursor =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .next_back()
            .expect("editor cursor fill rect should be present")
    }

    fn flat_line_y_for_source_byte(view: &MarkdownEditorView, source_byte: usize) -> Option<f32> {
        let lazy = view.engine.lazy.as_ref()?;
        for flat_line in &lazy.flat_lines {
            if flat_line.source_projection.as_ref().is_some_and(|projection| {
                projection.boundaries.first().is_some_and(|anchor| anchor.byte == source_byte)
            }) {
                return Some(flat_line.rect.y);
            }
        }
        None
    }

    /// Render editor with narrow width to trigger soft wrapping of long lines.
    fn render_editor_narrow(view: &mut MarkdownEditorView, doc: &StubDoc, width: f32) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, width, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn render_editor_viewport(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        width: f32,
        height: f32,
    ) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, width, height);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ =
            <MarkdownEditorView as ViewPlugin>::render(view, doc, bounds, &theme, &mut shaper, 1.0);
    }

    fn render_editor_viewport_with_dpi(
        view: &mut MarkdownEditorView,
        doc: &StubDoc,
        width: f32,
        height: f32,
        dpi_scale: f32,
    ) {
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, width, height);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let _ = <MarkdownEditorView as ViewPlugin>::render(
            view,
            doc,
            bounds,
            &theme,
            &mut shaper,
            dpi_scale,
        );
    }

    #[test]
    fn markdown_editor_full_flat_lines_only_precisely_shapes_visible_blocks() {
        let source = (0..80)
            .map(|idx| format!("paragraph {idx} with enough text for layout\n\n"))
            .collect::<String>();
        let doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);

        render_editor_viewport(&mut view, &doc, 800.0, 80.0);

        let lazy = view.engine().lazy.as_ref().expect("editor render should build layout");
        let precise_count = lazy.precise.iter().filter(|is_precise| **is_precise).count();
        let materialized_count = lazy.laid_out.iter().filter(|block| block.is_some()).count();

        assert_eq!(materialized_count, lazy.estimated_heights.len());
        assert!(!lazy.flat_lines.is_empty());
        assert!(
            precise_count < lazy.estimated_heights.len(),
            "full-flat WYSIWYG render should keep precise shaping viewport-scoped"
        );
    }

    #[test]
    fn markdown_editor_table_resize_preserves_projection_boundaries() {
        let rows = [
            "| G-01 | P0 | 已确认 | LLM 写入路径未限制在 `wiki/` | 原始来源或状态文件可被覆盖 |",
            "| G-02 | P0 | 已确认 | 任意一个对象写入成功即可判定摄入成功 | 缺页、坏 frontmatter、截断输出被误报成功 |",
            "| G-03 | P0 | 已确认 | 增量批量部分失败后跳过全部后处理 | 页面已落盘但 index/search 不可见或陈旧 |",
            "| G-04 | P0 | 已确认 | 并行重建共享输出目录且没有跨文件上下文 | 同名页面覆盖、知识重复、跨文档关系缺失 |",
            "| G-05 | P0 | 已确认 | 重建只要一个文件成功就发布不完整 Wiki | 旧 Wiki 中对应失败来源的内容被整体丢弃 |",
            "| G-06 | P1 | 已确认 | 重试没有清理第一次尝试的部分输出 | 失败残留混入第二次结果，产生幽灵页/重复页 |",
            "| G-07 | P0 | 已确认 | 最大 10 MB 原文整段进入单次 prompt，无分块 | 上下文溢出、超时、高费用、重试放大成本 |",
            "| G-08 | P1 | 已确认 | index/hot/log/deadlink/search 后处理错误不向上返回 | UI 显示成功但派生数据不完整 |",
            "| G-09 | P1 | 已确认 | 影子重建时 topic registry 写入 live wiki | 破坏影子目录隔离，失败重建也会改线上状态 |",
            "| G-10 | P1 | 已确认 | 死链标记不可逆 | 后续目标页生成后仍保持删除线死链 |",
            "| G-11 | P2 | 已确认 | `log.md` 每次被重建为快照，不是追加日志 | 历史摄入记录丢失，审计能力失效 |",
            "| G-12 | P1 | 已确认 | provider/send 未配置时上传锁不释放 | 文件不生成，后续上传/重建永久被锁住 |",
            "| G-13 | P1 | 已确认 | 重建状态只存在于前端内存 | 断线/切换 persona 后无法恢复进度和结果 |",
            "| G-14 | P1 | 已确认 | 重试和失败也发送 `complete` 进度 | 重建进度可超过总文件数、提前显示完成 |",
            "| G-15 | P2 | 风险 | 原文直接嵌入 prompt，且输出路径权限过宽 | 文档内提示注入可能放大为错误写入 |",
        ];
        let source = format!(
            "| ID | 优先级 | 状态 | 问题 | 主要影响 |\n|---|---|---|---|---|\n{}",
            rows.join("\n")
        );
        let doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);
        for viewport_width in [200.0, 400.0, 800.0, 1200.0, 1600.0, 2000.0] {
            render_editor_viewport(&mut view, &doc, viewport_width, 600.0);
        }
    }

    #[test]
    fn markdown_editor_initial_render_highlights_only_visible_code_blocks() {
        let mut source = String::new();
        for idx in 0..300 {
            source.push_str(&format!("```rust\nfn function_{idx}() {{}}\n```\n\n"));
        }
        let doc = StubDoc::new(&source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source, 1);

        render_editor_viewport(&mut view, &doc, 800.0, 240.0);

        let lazy = view.engine().lazy.as_ref().expect("layout should exist");
        let mut code_blocks = 0usize;
        let mut highlighted_blocks = 0usize;
        for block in lazy.laid_out.iter().filter_map(|block| block.as_ref()) {
            if let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind {
                code_blocks += 1;
                if lines.iter().any(|line| !line.highlight_spans.is_empty()) {
                    highlighted_blocks += 1;
                }
            }
        }

        assert!(highlighted_blocks > 0, "visible code blocks should be highlighted");
        assert!(
            highlighted_blocks < code_blocks / 2,
            "initial editor render should not highlight the whole document: {highlighted_blocks}/{code_blocks}"
        );
    }

    fn render_preview_narrow(view: &mut MarkdownView, width: f32) {
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let settings = default_settings();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        view.render(&theme, width, 600.0, 0.0, 0.0, settings, Some(&mut shaper));
    }

    #[test]
    fn preview_select_all_includes_final_line_outside_initial_lazy_viewport() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut source = String::new();
        for idx in 0..80 {
            source.push_str(&format!("ordinary paragraph {idx}\n\n"));
        }
        source.push_str("final selectable line\n\n");

        let mut doc = StubDoc::new(&source);
        let mut view = MarkdownView::new();
        view.set_source(source, 1);
        render_preview_narrow(&mut view, 800.0);

        view.handle_message(PluginMessage::SelectAll, &mut doc);

        let selected_text = match view.query(PluginQuery::SelectedText, &doc) {
            PluginResponse::String(text) => text,
            other => panic!("expected selected text, got {other:?}"),
        };

        assert!(
            selected_text.contains("final selectable line"),
            "preview select-all must include the final visible line, got {selected_text:?}"
        );
    }

    fn visible_line_snapshots(engine: &PreviewEngine) -> Vec<(String, i32)> {
        engine
            .flat_lines()
            .iter()
            .map(|line| (line.text.clone(), line.rect.y.round() as i32))
            .collect()
    }

    fn layout_snapshot(engine: &PreviewEngine) -> (Vec<(String, i32)>, i32) {
        (visible_line_snapshots(engine), engine.content_height.round() as i32)
    }

    fn byte_offset_for_line(source: &str, line_number: usize) -> usize {
        source.split_inclusive('\n').take(line_number.saturating_sub(1)).map(str::len).sum()
    }

    #[test]
    fn editor_view_cursor_blink_ownership() {
        use ui::plugin::ViewPlugin;
        let v = MarkdownEditorView::new();
        assert!(!v.shows_cursor(), "WYSIWYG draws its own cursor — app must not overlay one");
        assert!(
            v.needs_cursor_blink_wakeup(),
            "WYSIWYG needs app to compute and forward blink phase"
        );
    }

    #[test]
    fn editor_view_render_draws_cursor_rect_when_cursor_set() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::ViewPlugin;

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut v = MarkdownEditorView::new();
        v.set_source("hello world".into(), 1);

        // First render to build the lazy layout (required for cursor_screen_pos).
        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut doc = StubDoc::new("hello world");
        let _ = <MarkdownEditorView as ViewPlugin>::render(
            &mut v,
            &doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        // Place cursor at byte 5 via plugin message.
        v.handle_message(ui::plugin::PluginMessage::SetCursorByte(5), &mut doc);
        assert!(v.engine().cursor_screen_pos().is_some(), "cursor must be positioned after render");

        // Second render: should include the cursor rect.
        let dl = <MarkdownEditorView as ViewPlugin>::render(
            &mut v,
            &doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        // The DrawList should contain at least one FillRect for the cursor.
        let has_cursor_fill = dl.cmds.iter().any(
            |cmd| matches!(cmd, DrawCmd::FillRect { color, .. } if *color == theme.editor.cursor),
        );
        assert!(
            has_cursor_fill,
            "render() must draw a cursor rect using theme.editor.cursor color"
        );
    }

    #[test]
    fn new_empty_editor_draws_cursor_without_source_update() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let mut document = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.handle_message(PluginMessage::SetCursorByte(0), &mut document);

        let draw_list = render_editor_draw_list(&mut view, &document);
        let cursor_rect = editor_cursor_rect_from_draw_list(&draw_list);

        assert!(
            cursor_rect.x + cursor_rect.w > 0.0,
            "empty document cursor must overlap editor bounds"
        );
        assert!(
            cursor_rect.y + cursor_rect.h > 0.0,
            "empty document cursor must overlap editor bounds"
        );
    }

    #[test]
    fn new_empty_editor_renders_preedit_without_source_update() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut document = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut document,
        );

        let draw_list = render_editor_draw_list(&mut view, &document);
        let (preedit_x, preedit_width) = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, x, .. } if layout.text == "拼音" => {
                    Some((*x, layout.shaped.width))
                }
                _ => None,
            })
            .expect("new empty editor should emit the IME preedit text layout");
        let cursor_x = match view.query(PluginQuery::CursorScreenPos(0), &document) {
            PluginResponse::CursorScreenRect(Some((x, _, _, _))) => x,
            other => panic!("expected preedit cursor rect, got {other:?}"),
        };

        assert!(
            (cursor_x - (preedit_x + preedit_width)).abs() < 0.01,
            "preedit cursor must follow shaped text: cursor={cursor_x}, text_end={}",
            preedit_x + preedit_width
        );
    }

    #[test]
    fn new_empty_editor_uses_physical_typography_at_high_dpi() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut document = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut document,
        );

        let dpi_scale = 2.0;
        let expected_font_size = view.engine().base_font_size * dpi_scale;
        let draw_list = render_editor_draw_list_with_dpi(&mut view, &document, dpi_scale);
        let cursor_rect = editor_cursor_rect_from_draw_list(&draw_list);
        let preedit_font_size = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text == "拼音" => {
                    Some(layout.font_size)
                }
                _ => None,
            })
            .expect("new empty editor should emit the IME preedit text layout");
        let queried_cursor_height = match view.query(PluginQuery::CursorScreenPos(0), &document) {
            PluginResponse::CursorScreenRect(Some((_, _, _, height))) => height,
            other => panic!("expected preedit cursor rect, got {other:?}"),
        };

        assert!((cursor_rect.h - expected_font_size).abs() < 0.01);
        assert!((preedit_font_size - expected_font_size).abs() < 0.01);
        assert!((queried_cursor_height - expected_font_size).abs() < 0.01);
    }

    #[test]
    fn editor_view_hides_cursor_when_blink_phase_hidden() {
        use ui::core::paint::DrawCmd;
        use ui::plugin::{PluginMessage, ViewPlugin};

        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut doc = StubDoc::new("hello world");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        view.handle_message(PluginMessage::SetCursorVisible(false), &mut doc);

        let bounds = ui::core::geom::Rect::new(0.0, 0.0, 800.0, 600.0);
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let dl = <MarkdownEditorView as ViewPlugin>::render(
            &mut view,
            &doc,
            bounds,
            &theme,
            &mut shaper,
            1.0,
        );

        let has_cursor_fill = dl.cmds.iter().any(
            |cmd| matches!(cmd, DrawCmd::FillRect { color, .. } if *color == theme.editor.cursor),
        );
        assert!(!has_cursor_fill, "hidden blink phase must not draw WYSIWYG cursor");
    }

    #[test]
    fn editor_cursor_rect_uses_text_height_not_line_box() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::CursorScreenPos(5), &doc);
        let Some((_x, _y, _w, height)) = (match response {
            PluginResponse::CursorScreenRect(rect) => rect,
            other => panic!("expected CursorScreenRect, got {other:?}"),
        }) else {
            panic!("cursor rect should resolve");
        };

        assert!(
            (height - view.engine().base_font_size).abs() < 0.01,
            "cursor height should match glyph height, got {height}"
        );
    }

    #[test]
    fn editor_preedit_cursor_follows_materialized_preedit_layout() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        let base_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        view.handle_message(
            PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((1, 1)) },
            &mut doc,
        );
        let preedit_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        view.handle_message(
            PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((4, 4)) },
            &mut doc,
        );
        let end_rect = editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        assert!(
            preedit_rect.x > base_rect.x,
            "preedit cursor should advance from base cursor; base={}, preedit={}",
            base_rect.x,
            preedit_rect.x
        );
        assert!(
            preedit_rect.x < end_rect.x,
            "preedit cursor at byte 1 should stay before end cursor; preedit={}, end={}",
            preedit_rect.x,
            end_rect.x
        );
    }

    #[test]
    fn editor_multiline_preedit_cursor_uses_the_virtual_second_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        let base_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        let preedit = "中\n文";
        view.handle_message(
            PluginMessage::SetPreedit {
                text: preedit.into(),
                cursor: Some((preedit.len(), preedit.len())),
            },
            &mut doc,
        );
        let preedit_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        assert!(
            preedit_rect.y > base_rect.y + base_rect.h * 0.5,
            "multiline preedit caret must move to its virtual second line: base={base_rect:?}, preedit={preedit_rect:?}"
        );
    }

    #[test]
    fn editor_preedit_cursor_uses_physical_font_size_at_high_dpi() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        fn actual_preedit_advance(dpi_scale: f32) -> f32 {
            let source = "hello world";
            let mut doc = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.into(), 1);

            let _ = render_editor_draw_list_with_dpi(&mut view, &doc, dpi_scale);
            view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
            let base_rect = editor_cursor_rect_from_draw_list(&render_editor_draw_list_with_dpi(
                &mut view, &doc, dpi_scale,
            ));

            view.handle_message(
                PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((1, 1)) },
                &mut doc,
            );
            let preedit_rect = editor_cursor_rect_from_draw_list(
                &render_editor_draw_list_with_dpi(&mut view, &doc, dpi_scale),
            );
            preedit_rect.x - base_rect.x
        }

        let advance_1x = actual_preedit_advance(1.0);
        let advance_2x = actual_preedit_advance(2.0);
        assert!(
            (advance_2x - advance_1x * 2.0).abs() < 0.05,
            "preedit cursor advance should scale with physical DPI; 1x={advance_1x}, 2x={advance_2x}"
        );
    }

    #[test]
    fn editor_cursor_screen_pos_includes_preedit_cursor_advance() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        let base_response = view.query(PluginQuery::CursorScreenPos(5), &doc);
        let Some((base_x, _base_y, _base_w, _base_h)) = (match base_response {
            PluginResponse::CursorScreenRect(rect) => rect,
            other => panic!("expected CursorScreenRect, got {other:?}"),
        }) else {
            panic!("cursor rect should resolve before preedit");
        };

        view.handle_message(
            PluginMessage::SetPreedit { text: "abcd".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let preedit_response = view.query(PluginQuery::CursorScreenPos(5), &doc);
        let Some((preedit_x, _preedit_y, _preedit_w, _preedit_h)) = (match preedit_response {
            PluginResponse::CursorScreenRect(rect) => rect,
            other => panic!("expected CursorScreenRect, got {other:?}"),
        }) else {
            panic!("cursor rect should resolve with preedit");
        };

        let cursor_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));
        let drawn_cursor_center_x = cursor_rect.x + cursor_rect.w * 0.5;

        assert!(
            preedit_x > base_x,
            "CursorScreenPos should advance into preedit; base={base_x}, preedit={preedit_x}"
        );
        assert!(
            (preedit_x - drawn_cursor_center_x).abs() < 0.01,
            "CursorScreenPos should match drawn preedit caret x; query={preedit_x}, drawn={drawn_cursor_center_x}"
        );
    }

    #[test]
    fn editor_preedit_is_materialized_at_cursor() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };

        assert_eq!(lines.first().map(|line| line.text.as_str()), Some("helloni world"));
    }

    #[test]
    fn editor_preedit_is_materialized_only_on_cursor_line() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "first line\n\nhello world\n\nthird line";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        let cursor_byte = "first line\n\nhello".len();
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let texts = lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert_eq!(texts, vec!["first line", "helloni world", "third line"]);
    }

    #[test]
    fn editor_renders_preedit_on_trailing_empty_source_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut doc,
        );

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let preedit_is_rendered = draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                ui::core::paint::DrawCmd::TextLayout { layout, .. } if layout.text == "拼音"
            )
        });

        assert!(preedit_is_rendered, "preedit text must render on a trailing empty source line");
    }

    #[test]
    fn editor_renders_preedit_after_trailing_space() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello ";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut doc,
        );

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let rendered_text = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let preedit_is_rendered = rendered_text.iter().any(|text| text.contains("拼音"));

        assert!(
            preedit_is_rendered,
            "preedit text must render after a trailing source space; rendered={rendered_text:?}"
        );
    }

    #[test]
    fn empty_line_preedit_cursor_uses_shaped_advance() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "拼音".into(), cursor: Some((6, 6)) },
            &mut doc,
        );

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let (preedit_x, preedit_width) = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, x, .. } if layout.text == "拼音" => {
                    Some((*x, layout.shaped.width))
                }
                _ => None,
            })
            .expect("preedit text layout should be emitted");
        let cursor_x = match view.query(PluginQuery::CursorScreenPos(source.len()), &doc) {
            PluginResponse::CursorScreenRect(Some((x, _, _, _))) => x,
            other => panic!("expected preedit cursor rect, got {other:?}"),
        };

        assert!(
            (cursor_x - (preedit_x + preedit_width)).abs() < 0.01,
            "preedit cursor must use the shaped text advance: cursor={cursor_x}, \
             text_end={}",
            preedit_x + preedit_width
        );
    }

    #[test]
    fn trailing_empty_source_line_inherits_preceding_line_metrics() {
        let source = "# Heading\n";
        let mut view = MarkdownEditorView::new();
        let doc = StubDoc::new(source);
        view.set_source(source.into(), 1);
        view.engine.handle_set_cursor_byte(source.len());
        render_editor_once(&mut view, &doc);

        let lazy = view.engine.lazy.as_ref().expect("layout should be built");
        let source_line = source_line_at_byte(source, source.len())
            .expect("document end should resolve to the trailing empty source line");
        let previous_line = lazy.flat_lines.first().expect("heading should be rendered");
        let (_, _, font_size, line_height) =
            view.engine.empty_source_line_metrics(source_line, lazy, source);

        assert_eq!(font_size, previous_line.font_size);
        assert_eq!(line_height, previous_line.rect.h);
    }

    #[test]
    fn editor_preedit_clear_restores_source_text() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        view.handle_message(
            PluginMessage::SetPreedit { text: "ni".into(), cursor: Some((2, 2)) },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        view.handle_message(
            PluginMessage::SetPreedit { text: String::new(), cursor: None },
            &mut doc,
        );
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };

        assert_eq!(lines.first().map(|line| line.text.as_str()), Some("hello world"));
    }

    /// 段落末尾输入空格后,光标应向前推进一个空格宽度,即使 pulldown-cmark
    /// 剥离了尾部空白。
    #[test]
    fn editor_cursor_advances_past_trailing_source_space() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let mut view = MarkdownEditorView::new();
        let mut doc = StubDoc::new("abc");
        view.set_source("abc".into(), 1);
        render_editor_once(&mut view, &doc);
        view.handle_message(PluginMessage::SetCursorByte(3), &mut doc);
        let before_space =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        doc = StubDoc::new("abc ");
        view.set_source("abc ".into(), 2);
        view.handle_message(PluginMessage::SetCursorByte(4), &mut doc);
        let after_draw_list = render_editor_draw_list(&mut view, &doc);
        let after_space = editor_cursor_rect_from_draw_list(&after_draw_list);

        assert!(
            (after_space.y - before_space.y).abs() < 0.5,
            "cursor after trailing space must stay on the same line; before_y={}, after_y={}",
            before_space.y,
            after_space.y,
        );
        assert!(
            after_space.x > before_space.x + 1.0,
            "cursor after trailing space must advance visibly; before_x={}, after_x={}",
            before_space.x,
            after_space.x,
        );
    }

    /// 段落末尾输入 IME 时,光标应留在同一 flat_line,而非跳到下一段落。
    #[test]
    fn editor_preedit_cursor_at_paragraph_end_stays_on_same_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "hello\n\nworld";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        let cursor_byte = "hello".len();
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        let base_rect =
            editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

        for (preedit, cursor) in [("n", (1, 1)), ("ni", (2, 2)), ("nih", (3, 3)), ("nihao", (5, 5))]
        {
            view.handle_message(
                PluginMessage::SetPreedit { text: preedit.into(), cursor: Some(cursor) },
                &mut doc,
            );
            let preedit_rect =
                editor_cursor_rect_from_draw_list(&render_editor_draw_list(&mut view, &doc));

            assert!(
                (preedit_rect.y - base_rect.y).abs() < 0.5,
                "preedit={preedit:?} cursor must stay on same y; base_y={}, preedit_y={}",
                base_rect.y,
                preedit_rect.y
            );
            assert!(
                preedit_rect.x > base_rect.x,
                "preedit={preedit:?} cursor must advance from base; base_x={}, preedit_x={}",
                base_rect.x,
                preedit_rect.x
            );
        }
    }

    #[test]
    fn editor_selection_range_query_returns_source_bytes() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "hello world";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetSelAnchorByte(Some(6)), &mut doc);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(11)), &mut doc);

        let response = view.query(PluginQuery::SelectionRange, &doc);
        match response {
            PluginResponse::PositionPair(Some(((start, _), (end, _)))) => {
                assert_eq!((start, end), (6, 11));
            }
            other => panic!("expected source byte selection range, got {other:?}"),
        }
    }

    #[test]
    fn selection_intersecting_ascii_diagram_rebuilds_code_block_without_grid() {
        let source = "```\n┌────┐\n│中文│\n└────┘\n```\noutside";
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &document);

        let selection_start = source.find("中文").expect("fixture has diagram text");
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(selection_start)), &mut document);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(source.len())), &mut document);
        render_editor_once(&mut view, &document);

        let lazy = view.engine.lazy.as_ref().expect("selection render must build layout");
        let code_block = lazy.laid_out[0].as_ref().expect("code block must remain materialized");
        let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(
            lazy.ascii_diagrams().diagram_for(lines).is_none(),
            "selection must disable the diagram grid path"
        );
    }

    #[test]
    fn reverse_selection_intersecting_ascii_diagram_rebuilds_code_block_without_grid() {
        let source = "```\n┌────┐\n│中文│\n└────┘\n```\noutside";
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_string(), 1);
        render_editor_once(&mut view, &document);

        let selection_end = source.find("中文").expect("fixture has diagram text");
        view.handle_message(PluginMessage::SetSelAnchorByte(Some(source.len())), &mut document);
        view.handle_message(PluginMessage::SetSelCursorByte(Some(selection_end)), &mut document);
        render_editor_once(&mut view, &document);

        let lazy = view.engine.lazy.as_ref().expect("selection render must build layout");
        let code_block = lazy.laid_out[0].as_ref().expect("code block must remain materialized");
        let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(
            lazy.ascii_diagrams().diagram_for(lines).is_none(),
            "reverse selection must disable the diagram grid path"
        );
    }

    #[test]
    fn selection_change_reuses_existing_layout_and_disables_intersecting_diagram_grid() {
        use std::cell::Cell;

        let source = "```\n┌────┐\n│中文│\n└────┘\n```\n\nunchanged paragraph";
        let document = StubDoc::new(source);
        let theme = ui::theme::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let style = crate::test_utils::default_style();
        let build_count = Cell::new(0usize);
        let mut engine = PreviewEngine::<crate::builder::MarkdownDoc>::new();
        engine.set_edit_source(Some(source.to_owned()));
        let mut shaper = shaping::Shaper::new().expect("selection test requires a shaper");

        engine.render(
            &theme,
            800.0,
            600.0,
            0.0,
            0.0,
            &style,
            |style| {
                build_count.set(build_count.get() + 1);
                let parsed = crate::parser::parse_markdown(source);
                crate::builder::MarkdownDoc::build(&parsed, style)
            },
            Some(&mut shaper),
            &document,
            true,
            false,
        );

        let selection_start = source.find("中文").expect("fixture has diagram text");
        engine.set_sel_anchor_byte(Some(selection_start));
        engine.set_sel_cursor_byte(Some(source.len()));
        engine.render(
            &theme,
            800.0,
            600.0,
            0.0,
            0.0,
            &style,
            |style| {
                build_count.set(build_count.get() + 1);
                let parsed = crate::parser::parse_markdown(source);
                crate::builder::MarkdownDoc::build(&parsed, style)
            },
            Some(&mut shaper),
            &document,
            true,
            false,
        );

        let lazy = engine.lazy.as_ref().expect("selection render must retain the lazy layout");
        let code_block = lazy.laid_out[0].as_ref().expect("code block must remain materialized");
        let crate::layout::LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(lazy.ascii_diagrams().diagram_for(lines).is_none());
        assert_eq!(build_count.get(), 1, "selection updates must not rebuild MarkdownDoc");
    }

    #[test]
    fn entering_editor_keeps_softbreak_paragraph_layout() {
        use ui::plugin::PluginMessage;

        let source = "\
你这个分数，别冲体育统考本科.

一. 定向军士:冲，不押宝.只填愿意去的军士院校和专业；优先技术型、军工/航空/信息/船舶/交通背景强的院校
二. 定体育高职高专批: 主线.\t
主报体育专科里“能考证、能升本、能接就业”的专业；学校上优先湖体职、武体科院专科组、荆州职院、黄冈职院这类稳妥组合，专业上优先体康、体教、体育运营，调剂尽量服从，专升本从进校第一天就开始准备。
体育保健与康复 / 运动健康指导 > 体育教育 > 体育运营与管理 / 健身指导与管理 / 社会体育 > 运动训练。
原因很简单：前两类要么接健康产业，要么接教师资格与校园岗位，路径更稳；后两类更吃城市、平台和个人能力；单纯运动训练则更吃资源和项目背景。
三. 高职高专普通批 : 兜底备选\t
若体育和军士都不理想，再考虑就业型普通专科，如护理康复、机电、轨道、计算机应用、汽车检测等

定向军士
如果考生真想走军士，我建议提前批这样排：
前 3–5 个：好学校好专业冲一下
长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。
中间 5–8 个：本省或相邻省份务实型军士院校
湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。

体育专科
体育高职高专批：主线兜底
前 3–4 个冲：武汉文理、荆州理工、湖北幼专、鄂州/咸宁等。
中间 8–10 个稳：湖北体育职业学院、武汉体育学院体育科技学院、黄冈职院、荆州职院等。
最后 6–8 个保：三峡旅游职院及其他往年线更低、计划数较多的体育专科组。";
        let width = 420.0;

        let mut preview = MarkdownView::new();
        preview.set_source(source.into(), 1);
        render_preview_narrow(&mut preview, width);

        let doc = StubDoc::new(source);
        let mut editor = MarkdownEditorView::new();
        editor.set_source(source.into(), 1);
        render_editor_narrow(&mut editor, &doc, width);

        assert_eq!(
            visible_line_snapshots(editor.engine()),
            visible_line_snapshots(preview.engine()),
            "clicking into WYSIWYG editor must not reflow ordinary softbreak paragraphs"
        );

        let before_click = visible_line_snapshots(editor.engine());
        for line_number in [3, 6, 8, 12, 16, 19, 23] {
            let byte_offset = byte_offset_for_line(source, line_number);
            editor.handle_message(
                PluginMessage::SetCursorByte(byte_offset),
                &mut StubDoc::new(source),
            );
            render_editor_narrow(&mut editor, &doc, width);
            assert_eq!(
                visible_line_snapshots(editor.engine()),
                before_click,
                "placing the cursor on physical line {line_number} must not reflow softbreak paragraphs"
            );
        }
    }

    #[test]
    fn first_cursor_focus_keeps_softbreak_layout_in_short_viewport() {
        use ui::plugin::PluginMessage;

        let source = "\
你这个分数，别冲体育统考本科.

一. 定向军士:冲，不押宝.只填愿意去的军士院校和专业；优先技术型、军工/航空/信息/船舶/交通背景强的院校
二. 定体育高职高专批: 主线.\t
主报体育专科里“能考证、能升本、能接就业”的专业；学校上优先湖体职、武体科院专科组、荆州职院、黄冈职院这类稳妥组合，专业上优先体康、体教、体育运营，调剂尽量服从，专升本从进校第一天就开始准备。
体育保健与康复 / 运动健康指导 > 体育教育 > 体育运营与管理 / 健身指导与管理 / 社会体育 > 运动训练。
原因很简单：前两类要么接健康产业，要么接教师资格与校园岗位，路径更稳；后两类更吃城市、平台和个人能力；单纯运动训练则更吃资源和项目背景。
三. 高职高专普通批 : 兜底备选\t
若体育和军士都不理想，再考虑就业型普通专科，如护理康复、机电、轨道、计算机应用、汽车检测等

定向军士
如果考生真想走军士，我建议提前批这样排：
前 3–5 个：好学校好专业冲一下
长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。
中间 5–8 个：本省或相邻省份务实型军士院校
湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。

体育专科
体育高职高专批：主线兜底
前 3–4 个冲：武汉文理、荆州理工、湖北幼专、鄂州/咸宁等。
中间 8–10 个稳：湖北体育职业学院、武汉体育学院体育科技学院、黄冈职院、荆州职院等。
最后 6–8 个保：三峡旅游职院及其他往年线更低、计划数较多的体育专科组。";
        let width = 420.0;
        let height = 260.0;
        let doc = StubDoc::new(source);

        for line_number in [3, 6, 8, 12, 16, 19, 23] {
            let mut editor = MarkdownEditorView::new();
            editor.set_source(source.into(), 1);
            render_editor_viewport(&mut editor, &doc, width, height);
            let before_focus = layout_snapshot(editor.engine());

            let byte_offset = byte_offset_for_line(source, line_number);
            editor.handle_message(
                PluginMessage::SetCursorByte(byte_offset),
                &mut StubDoc::new(source),
            );
            render_editor_viewport(&mut editor, &doc, width, height);

            assert_eq!(
                layout_snapshot(editor.engine()),
                before_focus,
                "first cursor focus on physical line {line_number} must not reflow softbreak layout"
            );
        }
    }

    #[test]
    fn editor_render_expands_cursor_span_source_markers() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut doc = StubDoc::new("hello **world** here");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };

        let joined = lines.into_iter().map(|line| line.text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("hello **world** here"),
            "cursor inside bold span must materialize markdown markers, got {joined:?}"
        );
    }

    #[test]
    fn hit_test_byte_roundtrip_inside_cjk_bold_span() {
        let mut view = make_view("前缀 **世界** 后缀");
        view.engine_mut().handle_set_cursor_byte("前缀 **世".len());

        let (cursor_x, cursor_y, _cursor_w, cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor should resolve");
        let result = view.engine().hit_test_byte(cursor_x, cursor_y + cursor_h * 0.5, 0.0, 0.0);

        assert_eq!(
            result,
            Some("前缀 **世".len()),
            "CJK cursor screen position must hit-test back to the same source byte"
        );
    }

    #[test]
    fn cursor_move_does_not_require_source_generation_change() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let mut doc = StubDoc::new("hello **world** here");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 7);
        render_editor_once(&mut view, &doc);

        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::NeedsSourceUpdate(7), &doc);
        assert!(matches!(response, PluginResponse::Bool(false)));
    }

    // ── Two-phase hit test ─────────────────────────────────────────────────

    /// Simulates the two-phase WYSIWYG hit test flow:
    /// 1. Hit-test on folded layout (cursor outside bold span) → candidate byte
    /// 2. SetCursorByte(candidate) + render → expand inline spans
    /// 3. Hit-test again on expanded layout → final byte
    ///
    /// Verifies that both phases land in the correct source range and that
    /// the expanded layout shows the markdown markers.
    #[test]
    fn two_phase_hit_test_from_folded_to_expanded_span() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        // Source: "hello **world** here" (21 bytes)
        // "hello "  = bytes 0..6
        // "**"      = bytes 6..8  (markers, folded when cursor not inside)
        // "world"   = bytes 8..13 (bold text)
        // "**"      = bytes 13..15 (markers)
        // " here"   = bytes 15..21
        let source = "hello **world** here";
        let world_source_range = 8..13;

        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // ---- Build FOLDED layout (cursor at byte 0, not in bold span) ----
        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        // ---- Find pixel position of "world" inside the FOLDED layout ----
        // In the folded text "hello world here", "world" starts at visual char 6
        // (after "hello " which is 6 visual chars).
        let flat_lines = view.engine().flat_lines();
        let fl = &flat_lines[0];
        assert!(
            fl.text.contains("hello"),
            "folded flat_line text should contain 'hello', got: {:?}",
            fl.text
        );
        // "world" starts at visual index 6 in "hello world here"
        let world_vis_start = 6;
        let px = fl.rect.x + crate::layout::grapheme_x(fl, world_vis_start) + 2.0;
        let py = fl.rect.y + fl.rect.h / 2.0;

        // ---- Phase 1: hit-test on folded layout ----
        let candidate = view
            .engine()
            .hit_test_byte(px, py, 0.0, 0.0)
            .expect("Phase 1 hit test on folded layout must find a byte");

        // ---- Phase 1.5: move cursor + sync refresh (expand the span) ----
        view.handle_message(PluginMessage::SetCursorByte(candidate), &mut doc);
        render_editor_once(&mut view, &doc);

        // Verify expanded layout: markers must be visible.
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|line| line.text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("hello **world** here"),
            "expanded layout must contain markdown markers after SetCursorByte + render, got: {joined:?}"
        );

        // ---- Phase 2: hit-test at cursor's visual position after expansion ----
        // After SetCursorByte + render, the cursor is at the candidate byte in
        // the expanded layout. We hit-test at the cursor's screen position to
        // get the final byte from the expanded source maps.
        let (cursor_x, cursor_y, _cw, cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor screen pos must resolve after render");
        let final_byte = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_h / 2.0, 0.0, 0.0)
            .expect("Phase 2 hit test on expanded layout must return a byte");

        // Both candidate and final should be within or very near the "world" source range.
        assert!(
            candidate >= world_source_range.start && candidate <= world_source_range.end,
            "Phase 1 candidate byte {candidate} should be in 'world' source range {world_source_range:?}",
        );
        assert!(
            final_byte >= world_source_range.start && final_byte <= world_source_range.end,
            "Phase 2 final byte {final_byte} should be in 'world' source range {world_source_range:?}",
        );

        // The two phases may produce slightly different bytes at pixel boundaries,
        // but they should be very close (within 2 bytes, the width of "**").
        let diff = (final_byte as isize - candidate as isize).abs();
        assert!(
            diff <= 2,
            "two-phase hit-test: final byte {final_byte} should be close to candidate {candidate} (diff={diff})"
        );
    }

    // ── Block marker editing ───────────────────────────────────────────────

    /// When cursor enters a heading's source range, the block marker
    /// (e.g. "# ") must become visible in FlatLines.
    #[test]
    fn heading_marker_visible_when_cursor_in_heading_source_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "# Title\n\nparagraph";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // Cursor in paragraph (byte 10) → heading marker NOT visible.
        view.handle_message(PluginMessage::SetCursorByte(10), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(
            !joined.contains("# "),
            "marker should NOT be visible when cursor is in paragraph, got: {joined:?}"
        );

        // Cursor at byte 3 (inside heading content "Title") → marker visible.
        view.handle_message(PluginMessage::SetCursorByte(3), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines after cursor move, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(
            joined.contains("# "),
            "marker '# ' must be visible when cursor is inside heading, got: {joined:?}"
        );
    }

    /// When cursor enters a blockquote's source range, the marker "> " must
    /// become visible in FlatLines.
    #[test]
    fn blockquote_marker_visible_when_cursor_in_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "> quoted";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // Cursor at byte 3 (inside content "quoted") → marker visible.
        view.handle_message(PluginMessage::SetCursorByte(3), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("> "), "blockquote marker '> ' must be visible, got: {joined:?}");
    }

    /// Empty document: FlatLines should be empty without crashing.
    #[test]
    fn empty_document_flat_lines_is_empty() {
        use ui::plugin::{PluginQuery, PluginResponse, ViewPlugin};

        let doc = StubDoc::new("");
        let mut view = MarkdownEditorView::new();
        view.set_source(String::new(), 1);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        assert!(lines.is_empty(), "empty document should produce no FlatLines");
    }

    /// Very long line: FlatLines still produces output without panic.
    #[test]
    fn very_long_line_does_not_panic() {
        use ui::plugin::{PluginQuery, PluginResponse, ViewPlugin};

        let long = "x".repeat(10_000);
        let doc = StubDoc::new(&long);
        let mut view = MarkdownEditorView::new();
        view.set_source(long, 1);
        render_editor_once(&mut view, &doc);

        let response = view.query(PluginQuery::FlatLines, &doc);
        assert!(matches!(response, PluginResponse::FlatLines(_)));
    }

    /// H2 heading marker "## " is visible.
    #[test]
    fn h2_marker_visible_when_cursor_in_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "## SubTitle";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("## "), "h2 marker must be visible, got: {joined:?}");
    }

    #[test]
    fn folded_h2_click_uses_expanded_cursor_rect_after_marker_appears() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "## SubTitle\n\nparagraph";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(source.len()), &mut doc);
        render_editor_once(&mut view, &doc);

        let folded_line = view.engine().flat_lines()[0].clone();
        assert!(
            !folded_line.text.starts_with("## "),
            "heading marker should start folded while cursor is outside, got {:?}",
            folded_line.text
        );

        let clicked_visual_grapheme = 2;
        let px =
            folded_line.rect.x + crate::layout::grapheme_x(&folded_line, clicked_visual_grapheme);
        let py = folded_line.rect.y + folded_line.rect.h * 0.5;
        let candidate = view
            .engine()
            .hit_test_byte(px, py, 0.0, 0.0)
            .expect("folded heading hit-test should return a source byte");

        view.handle_message(PluginMessage::SetCursorByte(candidate), &mut doc);
        render_editor_once(&mut view, &doc);

        let expanded_line = view.engine().flat_lines()[0].clone();
        assert!(
            expanded_line.text.starts_with("## "),
            "heading marker should appear after cursor enters heading, got {:?}",
            expanded_line.text
        );

        let stale_mouse_hit = view
            .engine()
            .hit_test_byte(px, py, 0.0, 0.0)
            .expect("expanded heading stale hit-test should still hit the line");
        assert_ne!(
            stale_mouse_hit, candidate,
            "reusing the folded mouse x after marker expansion should demonstrate the drift"
        );

        let (cursor_x, cursor_y, _cursor_w, cursor_h) =
            view.engine().cursor_screen_pos().expect("expanded cursor rect should resolve");
        let rect_hit = view
            .engine()
            .hit_test_byte(cursor_x, cursor_y + cursor_h * 0.5, 0.0, 0.0)
            .expect("cursor-rect hit-test should return a source byte");

        assert_eq!(
            rect_hit, candidate,
            "second phase should preserve the folded hit byte by using the expanded cursor rect"
        );
    }

    /// H3 heading marker "### " is visible.
    #[test]
    fn h3_marker_visible_when_cursor_in_range() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "### Small";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(6), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("### "), "h3 marker must be visible, got: {joined:?}");
    }

    /// Task list checked marker "- [x] " is visible.
    #[test]
    fn task_list_checked_marker_visible() {
        use ui::plugin::{PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

        let source = "- [x] done";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(8), &mut doc);
        render_editor_once(&mut view, &doc);
        let response = view.query(PluginQuery::FlatLines, &doc);
        let lines = match response {
            PluginResponse::FlatLines(lines) => lines,
            other => panic!("expected FlatLines, got {other:?}"),
        };
        let joined = lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("- [x] "), "checked task marker must be visible, got: {joined:?}");
    }

    /// HitTestByte returns None for a click far outside all content.
    #[test]
    fn hit_test_byte_returns_none_far_outside_content() {
        let v = make_view("- item");
        // Click at (99999, 99999) — way outside the rendered content.
        let result = v.engine().hit_test_byte(99999.0, 99999.0, 0.0, 0.0);
        assert!(result.is_none(), "HitTestByte far outside content should return None");
    }
    #[test]
    fn hit_test_byte_after_consecutive_empty_lines_stays_on_following_text() {
        let source = "\
### 真人感方面有做什么?
anchor 人物性格,背景描述,提示词来决定. 对他自己的了解和对你的了解

2 个


viebcoding 用过吗?

##  吴志全  
";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let target_start = source.find("viebcoding").expect("fixture should contain target line");
        let target_end = source.len();
        let target_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("viebcoding"))
            .expect("rendered flat lines should contain target line");

        for y_ratio in [0.1, 0.5, 0.9] {
            let hit = view
                .engine()
                .hit_test_byte(
                    target_line.rect.x + 1.0,
                    target_line.rect.y + target_line.rect.h * y_ratio,
                    0.0,
                    0.0,
                )
                .expect("click on target line should return a byte");

            assert!(
                (target_start..=target_end).contains(&hit),
                "click on target text at y ratio {y_ratio} mapped to byte {hit}, \
                 expected {target_start}..={target_end}"
            );
        }
    }

    #[test]
    fn hit_test_byte_on_visible_empty_line_after_heading_activates_empty_line() {
        let source = "\
## 李现民


对 AI Agent 产品形态有实际演进经验，对记忆、上下文和幻觉控制有较多工程实践";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        render_editor_once(&mut view, &doc);

        let editable_empty_line_byte = "## 李现民\n\n".len();
        view.handle_message(PluginMessage::SetCursorByte(editable_empty_line_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let (cursor_x, cursor_y, cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("visible empty source line should have a cursor rect");
        let hit = view.engine().hit_test_byte(
            cursor_x + cursor_width * 0.5,
            cursor_y + cursor_height * 0.5,
            0.0,
            0.0,
        );

        assert_eq!(
            hit,
            Some(editable_empty_line_byte),
            "clicking the visible empty line should activate its source byte"
        );
    }

    #[test]
    fn hit_test_byte_in_hidden_block_separator_snaps_nearby_text() {
        let source = "first\n\nsecond";
        let view = make_view(source);
        let first_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("first"))
            .expect("first paragraph should be laid out");
        let second_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("second"))
            .expect("second paragraph should be laid out");
        let separator_mid_y = (first_line.rect.y + first_line.rect.h + second_line.rect.y) * 0.5;

        let hit = view.engine().hit_test_byte(first_line.rect.x, separator_mid_y, 0.0, 0.0);

        assert!(
            hit.is_some(),
            "clicking the hidden block separator gap should snap to nearby text, not return None"
        );
        assert_ne!(hit, Some(0), "gap click must not degrade to document start");
    }

    #[test]
    fn whitespace_and_crlf_blank_lines_keep_empty_line_cursor_and_navigation() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        for (source, blank_byte) in [("first\n \n", 6), ("first\r\n\r\n", 7)] {
            let mut doc = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(source.to_owned(), 1);
            view.handle_message(PluginMessage::SetCursorByte(blank_byte), &mut doc);
            render_editor_once(&mut view, &doc);

            assert!(view.engine().cursor_screen_pos().is_some());
            assert_eq!(
                view.engine().visual_move(blank_byte, MoveDirection::LineStart, None),
                Some(blank_byte),
            );
        }
    }

    #[test]
    fn whitespace_only_block_separator_is_not_clickable_as_editable_line() {
        let source = "first\n \nsecond";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_once(&mut view, &doc);
        let first = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("first"))
            .expect("first paragraph must be rendered");
        let second = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("second"))
            .expect("second paragraph must be rendered");
        let separator_y = (first.rect.y + first.rect.h + second.rect.y) * 0.5;

        assert_ne!(view.engine().hit_test_byte(first.rect.x, separator_y, 0.0, 0.0), Some(6),);
    }

    #[test]
    fn visual_move_down_from_empty_source_line_goes_to_next_line_start() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Down, None)
            .expect("Down from empty line should return a byte");

        assert_eq!(moved, 7, "Down from empty line should land at next source line start");
    }

    #[test]
    fn visual_move_up_from_empty_source_line_goes_to_previous_line_start() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Up, None)
            .expect("Up from empty line should return a byte");

        assert_eq!(moved, 0, "Up from empty line should land at previous source line start");
    }

    #[test]
    fn visual_move_from_second_editable_empty_line_reaches_adjacent_rendered_lines() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let second_empty_line_start = "first\n\n".len();
        let second_line_start = source.find("second").expect("fixture should contain second line");
        view.handle_message(PluginMessage::SetCursorByte(second_empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(second_empty_line_start, MoveDirection::Up, None),
            Some(0),
            "Up from the second editable empty line should reach the previous rendered line"
        );
        assert_eq!(
            view.engine().visual_move(second_empty_line_start, MoveDirection::Down, None),
            Some(second_line_start),
            "Down from the second editable empty line should reach the next rendered line"
        );
    }

    #[test]
    fn visual_move_up_from_trailing_editable_empty_line_reaches_previous_rendered_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let trailing_empty_line_start = "first\n".len();
        view.handle_message(PluginMessage::SetCursorByte(trailing_empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(trailing_empty_line_start, MoveDirection::Up, None),
            Some(0),
            "Up from a trailing editable empty line should reach the previous rendered line"
        );
    }

    #[test]
    fn visual_move_right_skips_inter_paragraph_empty_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_line_end = source.find('\n').expect("fixture should contain blank line");
        view.handle_message(PluginMessage::SetCursorByte(first_line_end), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(first_line_end, MoveDirection::Right, None)
            .expect("Right from first line should return a byte");

        assert_eq!(moved, 7, "Right should skip hidden paragraph separator lines");
    }

    #[test]
    fn visual_move_left_skips_inter_paragraph_empty_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let second_line_start = source.find("second").expect("fixture should contain second line");
        view.handle_message(PluginMessage::SetCursorByte(second_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(second_line_start, MoveDirection::Left, None)
            .expect("Left from second line should return a byte");

        assert_eq!(moved, 5, "Left should skip hidden paragraph separator lines");
    }

    #[test]
    fn visual_move_right_from_empty_source_line_goes_to_next_line_start() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Right, None)
            .expect("Right from empty line should return a byte");

        assert_eq!(moved, 7, "Right from empty line should land at next source line start");
    }

    #[test]
    fn visual_move_left_from_empty_source_line_goes_to_previous_line_end() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let moved = view
            .engine()
            .visual_move(empty_line_start, MoveDirection::Left, None)
            .expect("Left from empty line should return a byte");

        assert_eq!(moved, 5, "Left from empty line should land at previous source line end");
    }

    #[test]
    fn visual_move_line_start_and_end_preserve_empty_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "first\n\nsecond";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let empty_line_start = 6;
        view.handle_message(PluginMessage::SetCursorByte(empty_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(empty_line_start, MoveDirection::LineStart, None),
            Some(empty_line_start),
            "Home on an empty source line should stay on that line"
        );
        assert_eq!(
            view.engine().visual_move(empty_line_start, MoveDirection::LineEnd, None),
            Some(empty_line_start),
            "End on an empty source line should stay on that line"
        );
    }

    #[test]
    fn cursor_on_blank_line_inside_code_block_stays_on_that_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "```rust\nfn a() {}\n\nfn b() {}\n```\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        view.handle_message(PluginMessage::SetCursorByte(blank_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        let (cursor_x, cursor_y, cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("blank line inside a code block should have a cursor rect");
        let hit = view.engine().hit_test_byte(
            cursor_x + cursor_width * 0.5,
            cursor_y + cursor_height * 0.5,
            0.0,
            0.0,
        );

        assert_eq!(
            hit,
            Some(blank_line_start),
            "cursor on the blank line inside a code block must map back to that line's byte"
        );
    }

    #[test]
    fn visual_move_down_passes_through_blank_line_inside_active_fenced_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "```rust\nfn a() {}\n\nfn b() {}\n```\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_code_line_start = source.find("fn a() {}").expect("fixture has first code line");
        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        let second_code_line_start =
            source.find("fn b() {}").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(first_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_code_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down from a code line should land on the blank line inside the same code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_code_line_start),
            "Down from the blank line inside a code block should reach the next code line"
        );
    }

    #[test]
    fn visual_move_up_passes_through_blank_line_inside_active_fenced_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "```rust\nfn a() {}\n\nfn b() {}\n```\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_code_line_start = source.find("fn a() {}").expect("fixture has first code line");
        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        let second_code_line_start =
            source.find("fn b() {}").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(second_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(second_code_line_start, MoveDirection::Up, None),
            Some(blank_line_start),
            "Up from a code line should land on the blank line inside the same code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_code_line_start),
            "Up from the blank line inside a code block should reach the previous code line"
        );
    }

    #[test]
    fn visual_move_passes_through_blank_line_inside_active_indented_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "intro\n\n    first\n\n    second\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // 缩进代码块的块范围从首个文本字节开始（不含首行缩进）。
        let first_code_line_start = source.find("first").expect("fixture has first code line");
        let blank_line_start =
            source.find("first\n\n").expect("fixture has blank line") + "first\n".len();
        let second_code_line_start =
            source.find("    second").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(first_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_code_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down should land on the blank line inside the same indented code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_code_line_start),
            "Down from the blank line should reach the next indented code line"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_code_line_start),
            "Up from the blank line should reach the previous indented code line"
        );
    }

    #[test]
    fn visual_move_passes_through_blank_line_inside_crlf_code_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "```\r\nalpha\r\n\r\nbeta\r\n```\r\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_code_line_start = source.find("alpha").expect("fixture has first code line");
        let blank_line_start =
            source.find("\r\n\r\n").expect("fixture has blank line") + "\r\n".len();
        let second_code_line_start = source.find("beta").expect("fixture has second code line");
        view.handle_message(PluginMessage::SetCursorByte(first_code_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_code_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down should land on the blank line inside the same CRLF code block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_code_line_start),
            "Down from the CRLF blank line should reach the next code line"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_code_line_start),
            "Up from the CRLF blank line should reach the previous code line"
        );
    }

    #[test]
    fn visual_move_passes_through_blank_line_inside_metadata_block() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "---\ntitle: a\n\ndesc: b\n---\nbody\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let first_line_start = source.find("title: a").expect("fixture has first metadata line");
        let blank_line_start = source.find("\n\n").expect("fixture has blank line") + 1;
        let second_line_start = source.find("desc: b").expect("fixture has second metadata line");
        view.handle_message(PluginMessage::SetCursorByte(first_line_start), &mut doc);
        render_editor_once(&mut view, &doc);

        assert_eq!(
            view.engine().visual_move(first_line_start, MoveDirection::Down, None),
            Some(blank_line_start),
            "Down should land on the blank line inside the same metadata block"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Down, None),
            Some(second_line_start),
            "Down from the blank line should reach the next metadata line"
        );
        assert_eq!(
            view.engine().visual_move(blank_line_start, MoveDirection::Up, None),
            Some(first_line_start),
            "Up from the blank line should reach the previous metadata line"
        );
    }

    #[test]
    fn wysiwyg_cursor_y_aligns_to_text_baseline() {
        let mut doc = StubDoc::new("hello markdown");
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(5), &mut doc);

        let draw_list = render_editor_draw_list(&mut view, &doc);
        let text_baseline = draw_list
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                ui::core::paint::DrawCmd::TextLayout { y_baseline, layout, .. }
                    if layout.text.contains("hello markdown") =>
                {
                    Some(*y_baseline)
                }
                _ => None,
            })
            .expect("rendered markdown text must expose its baseline");

        let (_cursor_x, cursor_y, _cursor_w, cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor should resolve after render");
        let expected_top = text_baseline - cursor_h * WYSIWYG_CURSOR_ASCENT_RATIO;

        assert!(
            (cursor_y - expected_top).abs() < 0.01,
            "cursor top {cursor_y} should align to text baseline {text_baseline} with height {cursor_h}, expected {expected_top}"
        );
    }

    // ── Grapheme hit-test roundtrip ─────────────────────────────────────────

    #[test]
    fn wysiwyg_hit_test_roundtrips_combining_grapheme() {
        // byte 2 is the start of 'e' (correct grapheme boundary).
        // byte 3 is inside the combining acute, so it snaps to grapheme start byte 2.
        let mut view = make_view("**e\u{0301}**");
        view.engine_mut().handle_set_cursor_byte(2);

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

        assert_eq!(hit, Some(2));
    }

    #[test]
    fn wysiwyg_hit_test_roundtrips_zwj_emoji() {
        // byte 2 is the start of the ZWJ emoji (on a grapheme boundary).
        let emoji = "👨\u{200D}👩\u{200D}👧";
        let source = format!("**{emoji}**");
        let target_byte = 2usize;
        let mut view = make_view(&source);
        view.engine_mut().handle_set_cursor_byte(target_byte);

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

        assert_eq!(hit, Some(target_byte));
    }

    #[test]
    fn hit_test_byte_roundtrip_inside_list_item_respects_indent() {
        // "- item": bytes 0-1 are "- ", byte 2 start of "i".
        // List items have flat_line.rect.x > 0 (indent).
        let mut view = make_view("- item");
        view.engine_mut().handle_set_cursor_byte(2);

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        assert!(x > 0.0, "cursor x inside list item should include indent offset, got {x}");

        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);
        assert_eq!(hit, Some(2));
    }

    #[test]
    fn active_code_block_cursor_uses_shaped_code_font_geometry() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "```text\nabcdefghij\n```";
        let code_start = source.find("abcdefghij").expect("fixture must contain code text");
        let cursor_byte = code_start + 8;
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut document);
        render_editor_once(&mut view, &document);

        let visual_position = view
            .engine()
            .cursor_visual_position_for_byte(cursor_byte, CursorAffinity::Downstream)
            .expect("active code byte must have a visual position");
        let flat_line_idx = view
            .engine()
            .lazy
            .as_ref()
            .and_then(|lazy| lazy.flat_line_idx_for_projection(visual_position.flat_line_idx))
            .expect("active code projection must map to a flat line");
        let flat_line = &view.engine().flat_lines()[flat_line_idx];
        let shaped = flat_line.shaped.as_ref().expect("active code line must retain shaping");
        let local_byte = cursor_byte - code_start;
        let expected_advance: f32 = shaped
            .clusters
            .iter()
            .take_while(|cluster| cluster.byte_range.start < local_byte)
            .map(|cluster| cluster.advance)
            .sum();
        let (cursor_x, cursor_y, _cursor_width, cursor_height) =
            view.engine().cursor_screen_pos().expect("active code cursor must resolve");

        assert!(
            (cursor_x - (flat_line.rect.x + expected_advance)).abs() < 0.01,
            "cursor x {cursor_x} must use shaped code advance {expected_advance}"
        );
        assert_eq!(
            view.engine().hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0,),
            Some(cursor_byte),
            "hit-testing at the shaped caret boundary must return the same source byte"
        );
    }

    #[test]
    fn hit_test_byte_roundtrips_table_middle_and_right_cells() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "| left | middle | right |\n| --- | --- | --- |\n| left cell | middle cell | right cell |";
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);

        for cell_text in ["middle cell", "right cell"] {
            let cell_byte = source.find(cell_text).expect("fixture must contain table cell text");
            view.handle_message(PluginMessage::SetCursorByte(cell_byte), &mut document);
            render_editor_once(&mut view, &document);

            let (cursor_x, cursor_y, _cursor_width, cursor_height) =
                view.engine().cursor_screen_pos().expect("table cell must have a cursor rect");
            let cell_line = view
                .engine()
                .flat_lines()
                .iter()
                .find(|line| {
                    line.source_projection.as_ref().is_some_and(|projection| {
                        projection.boundaries.iter().any(|anchor| anchor.byte == cell_byte)
                    })
                })
                .expect("table cell source byte must own a rendered line");
            assert!(
                cursor_x >= cell_line.rect.x && cursor_x <= cell_line.rect.x + cell_line.rect.w,
                "cursor x {cursor_x} must fall inside its table cell rect {:?}",
                cell_line.rect,
            );

            let hit =
                view.engine().hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0);
            assert_eq!(
                hit,
                Some(cell_byte),
                "table cell {cell_text:?} must hit-test to its own source byte",
            );
        }
    }

    #[test]
    fn hit_test_byte_roundtrips_trailing_stripped_whitespace() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let plain_source = "plain trailing \t";
        let table_source = "| left | right |\n| --- | --- |\n| left cell | right cell \t|";
        let table_trailing_whitespace =
            table_source.find("\t|").expect("fixture must contain trailing table tab") + 1;

        for (source, cursor_byte) in
            [(plain_source, plain_source.len()), (table_source, table_trailing_whitespace)]
        {
            let mut document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut document);
            render_editor_once(&mut view, &document);

            let expected = canonical_source_byte(&view, cursor_byte);
            let hit = hit_test_source_byte_at_cursor(&view, cursor_byte);
            assert_eq!(
                hit, expected,
                "source={source:?}, cursor={cursor_byte} must preserve its canonical anchor",
            );
        }
    }

    fn source_grapheme_boundaries(source: &str) -> Vec<usize> {
        let grapheme_count = crate::grapheme_map::grapheme_count(source);
        (0..=grapheme_count)
            .map(|grapheme_index| {
                crate::grapheme_map::byte_at_grapheme_index(source, grapheme_index)
            })
            .collect()
    }

    fn canonical_source_byte(view: &MarkdownEditorView, source_byte: usize) -> usize {
        let position = view
            .engine()
            .projection_index()
            .visual_position_for_source(source_byte, CursorAffinity::Downstream)
            .expect("every source grapheme boundary must have a canonical position");
        view.engine()
            .projection_index()
            .source_anchor_at(1, position)
            .expect("canonical position must map back to source")
            .byte
    }

    fn hit_test_source_byte_at_cursor(view: &MarkdownEditorView, source_byte: usize) -> usize {
        let (x, y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos_for_byte(source_byte)
            .expect("every canonical source position must have a cursor rect");
        view.engine()
            .hit_test_byte(x, y + cursor_height * 0.5, 0.0, 0.0)
            .expect("cursor rect center must be hittable")
    }

    #[test]
    fn projection_corpus_roundtrips_every_source_grapheme_boundary() {
        let corpus = [
            "plain paragraph",
            "# heading heading heading heading heading heading",
            "> outer\n> > **inner** — continuation",
            "- outer\n  - inner wrapped content wrapped content",
            "| left | middle | right |\n| --- | --- | --- |\n| 左侧内容 | middle content | 👨\u{200d}👩 |",
            "paragraph\n\n\nnext",
            "e\u{301} and 👨\u{200d}👩",
        ];

        for source in corpus {
            for width in [140.0, 320.0, 800.0] {
                let mut document = StubDoc::new(source);
                let mut view = MarkdownEditorView::new();
                view.set_source(document.text.clone(), 1);
                view.handle_message(PluginMessage::SetCursorByte(0), &mut document);
                render_editor_narrow(&mut view, &document, width);

                for source_byte in source_grapheme_boundaries(source) {
                    let has_static_position = view
                        .engine()
                        .projection_index()
                        .visual_position_for_source(source_byte, CursorAffinity::Downstream);
                    let (mut expected, mut hit) = match has_static_position {
                        Some(_) => (
                            canonical_source_byte(&view, source_byte),
                            hit_test_source_byte_at_cursor(&view, source_byte),
                        ),
                        None => {
                            view.handle_message(
                                PluginMessage::SetCursorByte(source_byte),
                                &mut document,
                            );
                            render_editor_narrow(&mut view, &document, width);
                            (
                                canonical_source_byte(&view, source_byte),
                                hit_test_source_byte_at_cursor(&view, source_byte),
                            )
                        }
                    };

                    if has_static_position.is_some() && hit != expected {
                        view.handle_message(
                            PluginMessage::SetCursorByte(source_byte),
                            &mut document,
                        );
                        render_editor_narrow(&mut view, &document, width);
                        expected = canonical_source_byte(&view, source_byte);
                        hit = hit_test_source_byte_at_cursor(&view, source_byte);
                    }

                    assert_eq!(
                        hit, expected,
                        "source={source:?}, width={width}, byte={source_byte}"
                    );
                }
            }
        }
    }

    #[test]
    fn preedit_corpus_roundtrips_committed_source_anchor() {
        let cases = [
            ("nested > quote with 中文", "中", "输入"),
            (
                "| left | middle | right |\n| --- | --- | --- |\n| left cell | middle cell | right cell |",
                "middle cell",
                "中文",
            ),
        ];

        for (source, cursor_text, preedit) in cases {
            let cursor_byte = source.find(cursor_text).expect("fixture must contain cursor text");
            let document = StubDoc::new(source);
            let mut view = MarkdownEditorView::new();
            view.set_source(document.text.clone(), 1);
            view.engine.handle_set_cursor_byte(cursor_byte);
            view.engine.set_preedit_text(preedit.to_string(), Some((preedit.len(), preedit.len())));
            render_editor_narrow(&mut view, &document, 140.0);

            let (x, y, _cursor_width, cursor_height) =
                view.engine().cursor_screen_pos().expect("preedit cursor must have a source rect");
            let hit = view.engine().hit_test_byte(x, y + cursor_height * 0.5, 0.0, 0.0);

            assert_eq!(
                hit,
                Some(cursor_byte),
                "source={source:?}, preedit={preedit:?} must retain its committed source anchor",
            );
        }
    }

    #[test]
    fn cursor_only_refresh_keeps_unaffected_block_projection_identical() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "# first heading\n\nmiddle paragraph stays stable\n\n> final quote";
        let first = source.find("first").expect("fixture must contain first heading text");
        let final_quote = source.find("final").expect("fixture must contain final quote text");
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(document.text.clone(), 1);
        view.handle_message(PluginMessage::SetCursorByte(first), &mut document);
        render_editor_narrow(&mut view, &document, 320.0);
        let before = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("middle paragraph"))
            .expect("middle paragraph must be visible")
            .source_projection
            .clone()
            .expect("middle paragraph must have a projection");

        view.handle_message(PluginMessage::SetCursorByte(final_quote), &mut document);
        render_editor_narrow(&mut view, &document, 320.0);
        let after = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text.contains("middle paragraph"))
            .expect("middle paragraph must stay visible")
            .source_projection
            .clone()
            .expect("middle paragraph must have a projection");

        assert_eq!(after, before);
    }

    #[test]
    fn hit_test_byte_roundtrip_inside_ordered_list_item_respects_marker_width() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "intro\n\n1. item";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        let folded_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text == "item")
            .expect("ordered list item should render folded content");
        let clicked_byte = source.find("item").expect("fixture should contain item") + 1;
        let folded_x = folded_line.rect.x + crate::layout::grapheme_x(folded_line, 1);
        let folded_y = folded_line.rect.y + folded_line.rect.h * 0.5;
        let candidate = view
            .engine()
            .hit_test_byte(folded_x, folded_y, 0.0, 0.0)
            .expect("folded ordered list hit-test should return a byte");
        assert_eq!(candidate, clicked_byte);

        view.handle_message(PluginMessage::SetCursorByte(candidate), &mut doc);
        render_editor_once(&mut view, &doc);

        let expanded_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| line.text == "1. item")
            .expect("ordered list item should render expanded marker");
        assert!(
            expanded_line.rect.x > 0.0,
            "ordered list content should preserve list indent after marker expands"
        );

        let (x, y, _w, h) = view.engine().cursor_screen_pos().expect("cursor should resolve");
        let hit = view.engine().hit_test_byte(x, y + h * 0.5, 0.0, 0.0);

        assert_eq!(hit, Some(clicked_byte));

        let shaped = expanded_line
            .shaped
            .as_ref()
            .expect("active ordered list line should keep shaped geometry");
        let prefix_len = "1. i".len();
        let prefix_width = shaped
            .clusters
            .iter()
            .take_while(|cluster| cluster.byte_range.start < prefix_len)
            .map(|cluster| cluster.advance)
            .sum::<f32>();
        let expected_x = expanded_line.rect.x + prefix_width;
        assert!(
            (x - expected_x).abs() < 0.01,
            "cursor x {x} should match shaped prefix x {expected_x} for active ordered list"
        );
    }

    #[test]
    fn active_list_with_lazy_continuation_lines_does_not_duplicate_flat_lines() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "- 前 3-5 个：好学校好专业冲一下\n长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。\n中间 5-8 个：本省或相邻省份务实型军士院校\n湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find("好学校").expect("fixture should contain first line text");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert_eq!(
            flat_texts.len(),
            4,
            "list lazy continuation should produce one flat line per source line, got {flat_texts:?}"
        );
        assert_eq!(flat_texts[0], "- 前 3-5 个：好学校好专业冲一下");
        assert_eq!(
            flat_texts[1],
            "长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。"
        );
        assert_eq!(flat_texts[2], "中间 5-8 个：本省或相邻省份务实型军士院校");
        assert_eq!(
            flat_texts[3],
            "湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。"
        );
    }

    #[test]
    fn active_last_list_item_ignores_trailing_blank_separator_lines() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "- a\n- b\n\n";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find('b').expect("fixture should contain last item text");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert_eq!(flat_texts, vec!["a", "- b"]);
    }

    #[test]
    fn active_list_down_moves_to_next_lazy_continuation_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "- 前 3-5 个：好学校好专业冲一下\n长沙航空、南京信息、江苏海事、成都航空、西安航空、重庆航天这类，有湖北计划就放前面。\n中间 5-8 个：本省或相邻省份务实型军士院校\n湖北交通、武汉船舶、长江工程、湖南汽车、湖南国防、张家界航空、江西航空等。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let current_byte = source.find("好学校").expect("fixture should contain first line text");
        view.handle_message(PluginMessage::SetCursorByte(current_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let next_line_start = source.find("长沙航空").expect("fixture should contain second line");
        let third_line_start = source.find("中间").expect("fixture should contain third line");
        let moved = view
            .engine()
            .visual_move(current_byte, MoveDirection::Down, None)
            .expect("Down from first list line should return a byte");

        assert!(
            (next_line_start..third_line_start).contains(&moved),
            "Down should move into the second source line byte range {next_line_start}..{third_line_start}, got {moved}"
        );
    }

    #[test]
    fn active_styled_list_lazy_continuation_preserves_source_lines() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "- 前 **好学校** 好专业冲一下\n长沙航空、南京 **信息**、江苏海事这类，有湖北计划就放前面。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let current_byte = source.find("好专业").expect("fixture should contain first line text");
        view.handle_message(PluginMessage::SetCursorByte(current_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        assert_eq!(
            flat_texts,
            vec![
                "- 前 好学校 好专业冲一下",
                "长沙航空、南京 信息、江苏海事这类，有湖北计划就放前面。"
            ]
        );

        let next_line_start = source.find("长沙航空").expect("fixture should contain second line");
        let moved = view
            .engine()
            .visual_move(current_byte, MoveDirection::Down, None)
            .expect("Down from styled list line should return a byte");

        assert!(
            moved >= next_line_start,
            "Down should move into the second source line starting at {next_line_start}, got {moved}"
        );
    }

    #[test]
    fn visual_move_down_in_indented_soft_wrap_uses_screen_x_as_absolute_coordinate() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "- alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let current_byte = source.find("gamma").expect("fixture should contain current word");
        view.handle_message(PluginMessage::SetCursorByte(current_byte), &mut doc);
        render_editor_narrow(&mut view, &doc, 170.0);

        let flat_lines = view.engine().flat_lines();
        assert!(
            flat_lines.len() >= 3,
            "narrow list item should soft-wrap into at least 3 visual lines, got {}",
            flat_lines.len()
        );
        let (current_flat_idx, _) = view
            .engine()
            .find_flat_and_grapheme_for_byte(current_byte)
            .expect("current byte should map to a flat line");
        let target_flat_idx = current_flat_idx + 1;
        let target_line = flat_lines.get(target_flat_idx).expect("next soft-wrap line must exist");
        assert!(
            target_line.rect.x > 0.0,
            "fixture must exercise non-zero target line x, got {}",
            target_line.rect.x
        );
        let (cursor_x, _cursor_y, _cursor_w, _cursor_h) =
            view.engine().cursor_screen_pos().expect("cursor rect should resolve");

        let expected_grapheme =
            view.engine().grapheme_at_x_for_line(target_line, cursor_x - target_line.rect.x);
        let expected_byte = view
            .engine()
            .byte_from_flat_line_and_visual_grapheme(target_flat_idx, expected_grapheme)
            .expect("expected target grapheme should map to source byte");
        let moved = view
            .engine()
            .visual_move(current_byte, MoveDirection::Down, Some(cursor_x))
            .expect("Down should move to the next soft-wrap line");

        assert_eq!(
            moved, expected_byte,
            "VisualMove target_x is in plugin coordinates and must be converted to target-line relative x"
        );
    }

    #[test]
    fn visual_move_up_from_empty_line_before_long_paragraph_returns_previous_source_line() {
        use ui::plugin::{MoveDirection, PluginMessage, ViewPlugin};

        let source = "\
# 版式部门 2026 OKR 战略陈述（新版）

**汇报口径：** 只回答两个问题：

1. **我的目标凭什么支撑战略？**
2. **目标的可达成性如何？**

**评审语境：** 这不是一次功能汇报，而是一次战略承接答辩。评委要集中业务判断力，判断版式 OKR 是否真正支撑公司战略、是否存在方向性问题、是否需要整改。CEO 最后需要定调：核心问题是什么、是否需要整改、方向是否认可。

---

## 0. 先给结论

版式部门 2026 OKR 的方向应当被认可。

它的核心价值不是“继续增强 PDF/OFD 工具”，而是把 PDF/OFD 这类**正式业务结果文档**，从传统阅读、编辑、转换工具，升级为 AI Office 和 WPS 365 中能够被理解、被调用、被处理、被交付、被合规归档的关键能力。

用一句话概括：

> **如果 AI Office 只能生成草稿、总结文档，却不能处理合同、公文、标书、审计报告、签章文件这些最终业务结果，那么它还没有真正进入用户的业务闭环。版式部门要补上的，正是 AI Office 通向正式业务结果的最后一公里。**

所以，版式 OKR 对公司战略的支撑关系是成立的。需要强化的不是方向，而是执行表达：要从“能力清单”进一步收敛为“正式文档处理场景闭环”和“可验证的业务结果”。

---

## 1. 汇报逻辑重构
";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let line_14_start =
            source.find("版式部门 2026 OKR 的方向").expect("fixture should contain line 14");
        let line_16_start = source.find("它的核心价值").expect("fixture should contain line 16");
        let line_26_start =
            source.find("## 1. 汇报逻辑重构").expect("fixture should contain line 26");

        view.handle_message(PluginMessage::SetCursorByte(line_16_start), &mut doc);
        render_editor_narrow(&mut view, &doc, 420.0);
        let (line_16_x, _line_16_y, _line_16_w, _line_16_h) =
            view.engine().cursor_screen_pos().expect("line 16 cursor should resolve");
        let moved_above = view
            .engine()
            .visual_move(line_16_start, MoveDirection::Up, Some(line_16_x))
            .expect("Up from line 16 should return a byte");

        assert!(
            (line_14_start..line_16_start).contains(&moved_above),
            "Up from line 16 should skip line 15 paragraph spacing and return to line 14 range {line_14_start}..{line_16_start}, got {moved_above}; line 26 starts at {line_26_start}"
        );
    }

    #[test]
    fn inactive_list_lazy_continuation_keeps_fast_collapsed_layout() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source =
            "intro\n\n- 前 3-5 个：好学校好专业冲一下\n长沙航空、南京信息、江苏海事、成都航空。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        view.handle_message(PluginMessage::SetCursorByte(0), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();

        assert!(
            flat_texts.iter().any(|line| line.contains("冲一下 长沙航空")),
            "inactive list should keep the parser-collapsed softbreak layout, got {flat_texts:?}"
        );
    }

    #[test]
    fn loose_list_items_do_not_render_parent_and_paragraph_twice() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "\
- 前 3-4 个冲：武汉文理、荆州理工、湖北幼专、鄂州/咸宁等。

- 中间 8-10 个稳：湖北体育职业学院、武汉体育学院体育科技学院、黄冈职院、荆州职院等。

最后 6-8 个保：三峡旅游职院及其他往年线更低、计划数较多的体育专科组。";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find("武汉文理").expect("fixture should contain first list item");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        let first_item_count = flat_texts.iter().filter(|line| line.contains("武汉文理")).count();
        let second_item_count =
            flat_texts.iter().filter(|line| line.contains("湖北体育职业学院")).count();

        assert_eq!(first_item_count, 1, "first loose list item duplicated: {flat_texts:?}");
        assert_eq!(second_item_count, 1, "second loose list item duplicated: {flat_texts:?}");
        assert!(
            flat_texts.iter().any(|line| line.contains("最后 6-8 个保")),
            "following paragraph should still render, got {flat_texts:?}"
        );
    }

    #[test]
    fn active_parent_list_item_does_not_duplicate_nested_list_items() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "\
- parent
  - child one
  - child two
- sibling";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        let cursor_byte = source.find("parent").expect("fixture should contain parent item");
        view.handle_message(PluginMessage::SetCursorByte(cursor_byte), &mut doc);
        render_editor_once(&mut view, &doc);

        let flat_texts =
            view.engine().flat_lines().iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
        let first_child_count = flat_texts.iter().filter(|line| line.contains("child one")).count();
        let second_child_count =
            flat_texts.iter().filter(|line| line.contains("child two")).count();

        assert_eq!(first_child_count, 1, "first child duplicated: {flat_texts:?}");
        assert_eq!(second_child_count, 1, "second child duplicated: {flat_texts:?}");
    }

    // ── Soft-wrap cursor roundtrip ────────────────────────────────────────

    /// Cursor byte → screen position → hit-test back should roundtrip on a
    /// soft-wrapped line (CJK+ASCII mixed content with a narrow viewport).
    #[test]
    fn wysiwyg_cursor_roundtrips_on_second_soft_wrapped_line() {
        use ui::plugin::{PluginMessage, ViewPlugin};

        let source = "这是一段很长的中文测试文本 mixed with english and some more text that \
                      should definitely wrap across multiple visual lines when the viewport \
                      is narrow enough for testing purposes";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        // Render with narrow bounds to force soft wrapping.
        render_editor_narrow(&mut view, &doc, 160.0);

        let flat_lines = view.engine().flat_lines().to_vec();
        assert!(
            flat_lines.len() >= 3,
            "narrow viewport must wrap CJK+ASCII line into >= 3 flat lines, got {}",
            flat_lines.len()
        );

        // Use canonical projections to pick a source byte in the middle of the
        // second flat line's content (not the first or last entry).
        let source_maps = view.engine().flat_line_projection_boundaries();
        let map_1 = &source_maps[1];
        let bytes_1 = &map_1.boundaries;
        assert!(
            bytes_1.len() >= 3,
            "second flat line projection must have >= 3 boundaries, got {}",
            bytes_1.len()
        );
        let mid_idx = bytes_1.len() / 2;
        let mid_byte = bytes_1[mid_idx];

        // Set cursor to the middle byte of the second flat line.
        view.handle_message(PluginMessage::SetCursorByte(mid_byte), &mut doc);

        // Re-render so cursor position is computed from the updated layout.
        render_editor_narrow(&mut view, &doc, 160.0);

        // Verify cursor_screen_pos resolves.
        let (cx, cy, cw, ch) =
            view.engine().cursor_screen_pos().expect("cursor must resolve after render");

        // Cursor y must fall within the second flat line's rect (allow minor
        // rounding differences).
        let flat_lines = view.engine().flat_lines().to_vec();
        assert!(flat_lines.len() >= 3, "flat line count must remain >= 3 after cursor move");
        let fl1 = &flat_lines[1];
        assert!(
            cy >= fl1.rect.y - 0.5 && cy <= fl1.rect.y + fl1.rect.h + 0.5,
            "cursor y {cy} should be in second flat line y=[{}, {}], text={:?}",
            fl1.rect.y,
            fl1.rect.y + fl1.rect.h,
            fl1.text
        );

        // Hit-test at cursor horizontal midpoint + vertical midpoint.
        let hit_x = cx + cw * 0.5;
        let hit_y = cy + ch * 0.5;
        let roundtrip = view
            .engine()
            .hit_test_byte(hit_x, hit_y, 0.0, 0.0)
            .expect("hit-test at cursor position must return a byte");

        // The hit-test result should match the original byte. If it differs
        // due to pixel-snapping to a different grapheme boundary, verify
        // it is within the same projection boundary neighborhood (±1 grapheme).
        let source_maps = view.engine().flat_line_projection_boundaries();
        let map_1 = &source_maps[1];
        let bytes_after = &map_1.boundaries;
        let roundtrip_in_same_map = bytes_after.contains(&roundtrip);
        assert!(
            roundtrip_in_same_map || roundtrip.abs_diff(mid_byte) <= 4,
            "roundtrip byte {roundtrip} should match original byte {mid_byte} \
             or be within the same projection (boundaries: {bytes_after:?})",
        );
    }

    #[test]
    fn hit_test_byte_on_later_plain_soft_wrap_uses_segment_source_range() {
        let source = "段落00甲乙丙丁 段落01甲乙丙丁 段落02甲乙丙丁 段落03甲乙丙丁 \
                      段落04甲乙丙丁 段落05甲乙丙丁 段落06甲乙丙丁 段落07甲乙丙丁 \
                      段落08甲乙丙丁 段落09甲乙丙丁 段落10甲乙丙丁 段落11甲乙丙丁";
        let doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        render_editor_narrow(&mut view, &doc, 180.0);

        let flat_lines = view.engine().flat_lines();
        assert!(
            flat_lines.len() >= 3,
            "narrow viewport should soft-wrap plain paragraph into >= 3 flat lines, got {}",
            flat_lines.len()
        );
        let target_line = flat_lines
            .iter()
            .skip(2)
            .find(|line| !line.text.trim().is_empty())
            .expect("later soft-wrapped line should contain text");
        let expected_start = source.find(&target_line.text).unwrap_or_else(|| {
            panic!("source should contain target segment {:?}", target_line.text)
        });
        let expected_end = expected_start + target_line.text.len();

        let hit = view
            .engine()
            .hit_test_byte(
                target_line.rect.x + 1.0,
                target_line.rect.y + target_line.rect.h * 0.5,
                0.0,
                0.0,
            )
            .expect("hit-test on later soft-wrapped segment should return a byte");

        assert!(
            (expected_start..=expected_end).contains(&hit),
            "click on later soft-wrap segment {:?} mapped to byte {hit}, expected {expected_start}..={expected_end}",
            target_line.text
        );
    }

    /// 滚动后 cursor_screen_pos → hit_test_byte 仍 roundtrip。
    /// cursor_screen_pos 减去 scroll_y 使输出与渲染的 ly = rect.y - scroll_y + oy 一致；
    /// hit_test_byte 加回 scroll_y 使输入插件坐标映射到正确的文档行。
    #[test]
    fn wysiwyg_cursor_roundtrips_after_scroll_y() {
        let source = "这是一段很长的中文测试文本 mixed with english and some more text that \
                      should definitely wrap across multiple visual lines when the viewport \
                      is narrow enough for testing purposes";
        let mut doc = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(doc.text.clone(), 1);

        render_editor_narrow(&mut view, &doc, 160.0);

        let flat_lines = view.engine().flat_lines().to_vec();
        assert!(
            flat_lines.len() >= 3,
            "narrow viewport must wrap CJK+ASCII line into >= 3 flat lines, got {}",
            flat_lines.len()
        );

        // Pick a known byte in the third flat line (index 2).
        let (mid_byte, bytes_2) = {
            let source_maps = view.engine().flat_line_projection_boundaries();
            let map_2 = &source_maps[2];
            let b = &map_2.boundaries;
            assert!(
                b.len() >= 3,
                "third flat line projection must have >= 3 boundaries, got {}",
                b.len()
            );
            let mid_idx = b.len() / 2;
            (b[mid_idx], b.to_vec())
        };

        // Set cursor and scroll to a non-zero position.
        view.handle_message(PluginMessage::SetCursorByte(mid_byte), &mut doc);

        let scroll = 120.0;
        view.engine.scroll_y = scroll;

        // Re-render so layout reflects the cursor position.
        render_editor_narrow(&mut view, &doc, 160.0);

        // Forward: cursor_screen_pos must return something reasonable
        // (y reflects scroll projection, i.e. doc_y - scroll_y).
        let (cx, cy, _cw, ch) =
            view.engine().cursor_screen_pos().expect("cursor must resolve after render");

        // Cursor rect height must be non-zero.
        assert!(ch > 0.0, "cursor rect height must be positive, got {ch}");

        // Find the flat line our cursor landed on and verify scroll projection.
        {
            let flat_idx = {
                let lazy = view.engine().lazy.as_ref().unwrap();
                let mut found = None;
                for (fi, flat_line) in lazy.flat_lines.iter().enumerate() {
                    if flat_line.source_projection.as_ref().is_some_and(|projection| {
                        projection.boundaries.iter().any(|anchor| anchor.byte == mid_byte)
                    }) {
                        found = Some(fi);
                        break;
                    }
                }
                found.expect("mid_byte must be in some source map after re-render")
            };
            let fl = &view.engine().flat_lines()[flat_idx];
            let expected_cursor_y = fl.rect.y + (fl.rect.h - ch) * 0.5 - scroll;
            assert!(
                (cy - expected_cursor_y).abs() < 2.0,
                "cursor_screen_pos y={cy} should ≈ centered line y {} (fl.rect.y={}, scroll={scroll}, flat_idx={flat_idx})",
                expected_cursor_y,
                fl.rect.y,
            );
        }

        // Reverse: hit_test_byte at cursor position + container offset must
        // return the same source byte. offset_y models the app's bounds.y.
        let offset_y = 200.0;
        let hit_y = cy + ch * 0.5 + offset_y;
        let hit_x = cx;
        let roundtrip = view
            .engine()
            .hit_test_byte(hit_x, hit_y, 0.0, offset_y)
            .expect("hit-test at cursor position must return a byte");

        let roundtrip_matches = bytes_2.contains(&roundtrip) || roundtrip.abs_diff(mid_byte) <= 4;
        assert!(
            roundtrip_matches,
            "roundtrip byte {roundtrip} should match original byte {mid_byte} \
             or be within same source map (map bytes: {bytes_2:?})",
        );
    }

    #[test]
    fn code_block_hit_test_returns_code_content_byte() {
        let src = "```rust\nabc\ndef\n```\n\nparagraph\n";
        let mut v = make_view(src);

        // Code block starts at byte 0.
        // "```rust\n" is 8 bytes.
        // "abc\n" is 4 bytes (8..12).
        // "def\n" is 4 bytes (12..16).
        // "```\n" is 4 bytes.
        // We want to hit-test on 'e' in "def", which is byte 13.

        // Let's set cursor at 13, get screen pos, and hit_test it back.
        let mut doc = StubDoc::new(src);
        use ui::plugin::{PluginMessage, ViewPlugin};
        v.handle_message(PluginMessage::SetCursorByte(13), &mut doc);

        let pos = v.engine().cursor_screen_pos().unwrap();
        let (cx, cy, _cw, ch) = pos;

        let result = v.engine().hit_test_byte(cx, cy + ch / 2.0, 0.0, 0.0);
        assert_eq!(result, Some(13), "hit_test on 'e' in 'def' should return byte 13");
    }

    #[test]
    fn enter_on_empty_bullet_exits_list() {
        let src = "- ";
        let v = make_view(src);

        let aug = v.engine().augment_edit(2, ui::plugin::AugmentKind::Enter);

        let aug = aug.expect("Should return an augmentation");
        assert_eq!(aug.replace_range, Some(0..2), "Should replace the entire list marker");
        assert_eq!(
            aug.insert_text,
            Some(String::from("")),
            "Should insert empty text to exit list"
        );
    }

    #[test]
    fn enter_on_empty_ordered_item_exits_list() {
        let src = "1. ";
        let v = make_view(src);

        let aug = v.engine().augment_edit(3, ui::plugin::AugmentKind::Enter);

        let aug = aug.expect("Should return an augmentation");
        assert_eq!(aug.replace_range, Some(0..3), "Should replace the entire ordered list marker");
        assert_eq!(
            aug.insert_text,
            Some(String::from("")),
            "Should insert empty text to exit list"
        );
    }

    #[test]
    fn enter_on_non_empty_ordered_item_continues_numbering() {
        let src = "1. abc";
        let v = make_view(src);

        let aug = v.engine().augment_edit(6, ui::plugin::AugmentKind::Enter);

        let aug = aug.expect("Should return an augmentation");
        assert_eq!(aug.replace_range, None, "Should not replace anything");
        assert_eq!(aug.insert_text, Some(String::from("\n2. ")), "Should continue numbering");
    }

    #[test]
    fn empty_fenced_code_block_exposes_editable_content_line() {
        const OPENING_FENCE: &str = "```\n";
        let source = "before\n\n```\n```\n\nafter";
        let content_start =
            source.find(OPENING_FENCE).expect("fixture must contain an opening fence")
                + OPENING_FENCE.len();
        let mut document = StubDoc::new(source);
        let mut view = MarkdownEditorView::new();
        view.set_source(source.to_owned(), 1);
        render_editor_once(&mut view, &document);

        let content_line = view
            .engine()
            .flat_lines()
            .iter()
            .find(|line| {
                line.source_projection.as_ref().is_some_and(|projection| {
                    projection.boundaries.iter().any(|anchor| anchor.byte == content_start)
                })
            })
            .expect("empty fenced code block must retain an editable content projection");
        let hit = view.engine().hit_test_byte(
            content_line.rect.x,
            content_line.rect.y + content_line.rect.h * 0.5,
            0.0,
            0.0,
        );

        assert_eq!(hit, Some(content_start));

        view.handle_message(ui::plugin::PluginMessage::SetCursorByte(content_start), &mut document);
        render_editor_once(&mut view, &document);
        let (cursor_x, cursor_y, _cursor_width, cursor_height) = view
            .engine()
            .cursor_screen_pos()
            .expect("clicking the empty code content must reveal a cursor");

        assert_eq!(
            view.engine().hit_test_byte(cursor_x, cursor_y + cursor_height * 0.5, 0.0, 0.0,),
            Some(content_start),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::plugin::{
        EditAugmentation, EditIntent, EditPlan, EditPolicy, EditRequest, EditSelection,
        EditTransaction, TextReplacement,
    };

    fn test_enter_context(source: &str, byte: usize) -> EnterContext {
        classify_enter_context(source, byte)
    }

    #[test]
    fn table_cell_empty_insertion() {
        let src = "| A |\n|---|";
        let ctx = test_enter_context(src, 3);
        if let EnterContext::TableCell { next_cell_start } = ctx {
            assert_eq!(next_cell_start, None);
        } else {
            panic!("Expected TableCell");
        }
    }

    #[test]
    fn markdown_edit_policy_maps_cursor_only_augmentation_to_move_cursor() {
        let mut view = MarkdownEditorView::new();
        let source = "| a |\n|---|\n| b |";
        view.set_source(source.into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: source.find('a').expect("first cell"),
            selection: None,
            intent: EditIntent::InsertParagraphBreak,
        };

        assert!(matches!(view.plan_edit(&request), EditPlan::MoveCursor(_)));
    }

    #[test]
    fn augmentation_edit_plan_preserves_transaction_protocol_fields() {
        let request = EditRequest {
            source_generation: 42,
            cursor_byte: 9,
            selection: None,
            intent: EditIntent::InsertText("replacement".into()),
        };
        let augmentation = EditAugmentation {
            replace_range: Some(4..9),
            insert_text: Some("-".into()),
            cursor_byte_after: 5,
        };

        assert_eq!(
            augmentation_edit_plan(&request, augmentation),
            EditPlan::Apply(EditTransaction {
                source_generation: 42,
                replacements: vec![TextReplacement { range: 4..9, text: "-".into() }],
                selection_after: EditSelection::Caret(5),
            })
        );
    }

    #[test]
    fn blockquote_marker_byte_selection_highlight_uses_bounded_source_line_lookups() {
        let text = "> quoted text\n\nparagraph";
        let mut engine = PreviewEngine::<crate::builder::MarkdownDoc>::new();
        engine.set_edit_source(Some(text.to_string()));
        let style = crate::test_utils::default_style();
        let parsed = crate::parser::parse_markdown(text);
        let theme = ui::theme::test_theme();
        engine.rebuild_layout(
            crate::builder::MarkdownDoc::build(&parsed, &style),
            &style,
            800.0,
            600.0,
            None,
            &crate::view::AppCodeHighlighter { theme: &theme },
            &core::document::StringDocView::new(text),
            true,
            true,
        );

        reset_source_line_at_byte_call_count();
        // Select 'q' at byte index 2 (between > and q)
        engine.set_sel_anchor_byte(Some(2));
        engine.set_sel_cursor_byte(Some(2));
        engine.selection_highlights([1.0; 4]);

        assert!(
            source_line_at_byte_call_count() <= 2,
            "Bounded binary searches should strictly limit source_line_at_byte fallback calls (was {})",
            source_line_at_byte_call_count()
        );
    }

    fn selection_enter_plan(source: &str, selection: std::ops::Range<usize>) -> EditPlan {
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: selection.end,
            selection: Some(selection),
            intent: EditIntent::InsertParagraphBreak,
        };
        view.plan_edit(&request)
    }

    /// 应用单条替换计划到源码，返回（最终文本, 光标落点）。
    fn apply_single_replacement(source: &str, plan: &EditPlan) -> (String, usize) {
        let EditPlan::Apply(transaction) = plan else {
            panic!("expected a single Apply plan, got {plan:?}");
        };
        assert_eq!(transaction.replacements.len(), 1, "selection Enter must stay atomic");
        let replacement = &transaction.replacements[0];
        let mut edited = source.to_owned();
        edited.replace_range(replacement.range.clone(), &replacement.text);
        let EditSelection::Caret(cursor) = transaction.selection_after else {
            panic!("selection Enter must end in a caret");
        };
        (edited, cursor)
    }

    #[test]
    fn selection_enter_in_paragraph_splits_block_at_deletion_point() {
        let source = "hello world";
        let plan = selection_enter_plan(source, 2..8);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "he\n\nrld");
        assert_eq!(cursor, 4, "光标应落在新块开头");
    }

    #[test]
    fn selection_enter_across_paragraphs_uses_deletion_point_context() {
        let source = "foo\n\nbar";
        let plan = selection_enter_plan(source, 2..5);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "fo\n\nbar");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn selection_enter_in_list_item_continues_marker() {
        let source = "- hello world";
        let plan = selection_enter_plan(source, 4..9);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "- he\n- orld");
        assert_eq!(cursor, 7, "光标应落在续行 marker 之后");
    }

    #[test]
    fn selection_enter_in_heading_middle_splits_heading() {
        let source = "# hello world";
        let plan = selection_enter_plan(source, 4..9);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "# he\norld");
        assert_eq!(cursor, 5);
    }

    #[test]
    fn selection_enter_in_setext_heading_preserves_heading_and_creates_block_after_underline() {
        let source = "Title\n===\nafter";
        let plan = selection_enter_plan(source, 1..4);
        let EditPlan::Apply(transaction) = plan else {
            panic!("Setext selection Enter must produce one atomic transaction");
        };

        assert_eq!(transaction.replacements.len(), 2);
        let mut edited = source.to_owned();
        let mut replacements = transaction.replacements.clone();
        replacements.sort_by_key(|replacement| replacement.range.start);
        for replacement in replacements.iter().rev() {
            edited.replace_range(replacement.range.clone(), &replacement.text);
        }

        assert_eq!(edited, "Te\n===\n\n\nafter");
        assert_eq!(transaction.selection_after, EditSelection::Caret("Te\n===\n\n".len()));
    }

    #[test]
    fn selection_enter_leaving_empty_list_item_exits_list() {
        let source = "- a";
        let plan = selection_enter_plan(source, 2..3);

        let (edited, cursor) = apply_single_replacement(source, &plan);
        assert_eq!(edited, "");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn selection_enter_in_code_block_falls_back_to_default() {
        let source = "```\ncode\n```";
        let plan = selection_enter_plan(source, 4..8);

        assert_eq!(plan, EditPlan::UseDefault);
    }

    #[test]
    fn zero_width_selection_is_treated_as_no_selection() {
        let source = "hello world";
        let mut view = MarkdownEditorView::new();
        view.set_source(source.into(), 1);
        let zero_width = EditRequest {
            source_generation: 1,
            cursor_byte: 4,
            selection: Some(4..4),
            intent: EditIntent::InsertParagraphBreak,
        };
        let no_selection = EditRequest { selection: None, ..zero_width.clone() };

        assert_eq!(view.plan_edit(&zero_width), view.plan_edit(&no_selection));
    }

    #[test]
    fn selection_backspace_still_falls_back_to_default() {
        let mut view = MarkdownEditorView::new();
        view.set_source("hello world".into(), 1);
        let request = EditRequest {
            source_generation: 1,
            cursor_byte: 5,
            selection: Some(2..5),
            intent: EditIntent::DeleteBackward,
        };

        assert_eq!(view.plan_edit(&request), EditPlan::UseDefault);
    }
}
