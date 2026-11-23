//! PreviewEngine — shared cache/render/scroll/selection/search core.
//!
//! MarkdownView and NovelView each wrap a PreviewEngine, providing different
//! doc-building strategies (markdown parse vs. txt→MarkdownDoc) and styles
//! (from_theme vs. novel).

use crate::builder::{CodeHighlighter, HighlightSpan, MarkdownDoc};
use crate::layout::{BlockSource, HeadingEntry, LazyLayout};
use crate::projection::{
    CursorAffinity, HorizontalDirection, LineBoundary, SourceAnchor, SourceProjectionIndex,
    VisualPosition,
};
use crate::search::SearchHighlightCache;
use crate::selection::{self, SelectionState, ViewPos};
use crate::source_snapshot::{MarkdownSource, SourceSnapshot};
use crate::style::MarkdownStyle;
use render::GlyphVertex;
use std::path::Path;
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use stdext::arena::scratch_arena;
use textora_core::document::DocView;
use textora_core::highlight::{Highlighter, find_language, highlight_kind_scope};
use ui::Theme;
use ui::core::paint::DrawList;
use ui::plugin::{
    PastePreference, PluginFactory, PluginMessage, PluginQuery, PluginResponse, ViewPlugin,
    VisualCaretPosition,
};
use unicode_segmentation::UnicodeSegmentation;

mod preedit;

mod source_search;

#[cfg(test)]
mod source_snapshot_tests;

mod code_viewport;

mod navigation;

mod split_history;

#[cfg(test)]
#[path = "view_empty_paragraph_tests.rs"]
mod empty_paragraph_tests;

#[cfg(test)]
#[path = "view_rule_spacing_tests.rs"]
mod rule_spacing_tests;

// ===== Syntax highlighter =====

struct ByteSliceDoc<'a> {
    data: &'a [u8],
}

impl<'a> textora_core::document::ReadableDocument for ByteSliceDoc<'a> {
    fn read_forward(&self, off: usize) -> &[u8] {
        if off >= self.data.len() { &[] } else { &self.data[off..] }
    }
    fn read_backward(&self, off: usize) -> &[u8] {
        if off == 0 { &[] } else { &self.data[..off] }
    }
}

pub(in crate::view) struct AppCodeHighlighter<'a> {
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
pub(in crate::view) struct NavigationFontMetrics {
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
    pub(in crate::view) fn measure(
        shaper: &mut shaping::Shaper,
        font_size: f32,
        font_family: Option<&str>,
    ) -> Self {
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
pub(in crate::view) struct SourceLineAtByte {
    index: usize,
    start: usize,
    end: usize,
    is_blank: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::view) enum EmptySourceLineRole {
    HiddenBlockSeparator,
    EditableLine,
}

#[cfg(test)]
static SOURCE_LINE_AT_BYTE_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(in crate::view) fn reset_source_line_at_byte_call_count() {
    SOURCE_LINE_AT_BYTE_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(in crate::view) fn source_line_at_byte_call_count() -> usize {
    SOURCE_LINE_AT_BYTE_CALLS.load(Ordering::Relaxed)
}

pub(in crate::view) struct StandalonePreeditRenderData<'a> {
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
    pub(in crate::view) fn is_empty(self) -> bool {
        self.is_blank
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

const PARAGRAPH_FIRST_LINE_INDENT_EMS: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkdownRenderSettings<'a> {
    pub font_family: &'a str,
    pub font_size: f32,
    pub line_height: f32,
    pub toc_max_depth: u8,
    pub markdown_first_line_indent: bool,
    pub text_spacing_mode: ui::typography::TextSpacingMode,
}

impl<'a> MarkdownRenderSettings<'a> {
    pub fn from_metrics(
        settings: &'a ui::settings::Settings,
        metrics: &ui::settings::UiMetrics,
    ) -> Self {
        Self {
            font_family: &settings.font_family,
            font_size: metrics.font_size,
            line_height: metrics.line_height,
            toc_max_depth: settings.toc_max_depth,
            markdown_first_line_indent: settings.markdown_first_line_indent,
            text_spacing_mode: settings.text_spacing_mode,
        }
    }

    #[cfg(test)]
    pub(crate) fn style(self, theme: &Theme) -> MarkdownStyle {
        self.style_at_dpi(theme, 1.0)
    }

    pub(crate) fn style_at_dpi(self, theme: &Theme, dpi_scale: f32) -> MarkdownStyle {
        let mut style =
            MarkdownStyle::from_theme_at_dpi(theme, self.font_size, self.line_height, dpi_scale);
        if self.markdown_first_line_indent {
            style.paragraph_first_line_indent = PARAGRAPH_FIRST_LINE_INDENT_EMS * self.font_size;
        }
        style.body_font_family = vec![self.font_family.to_string()];
        style.text_spacing_mode = self.text_spacing_mode;
        style
    }
}

// ===== BlockAnchor (scroll stability) =====

#[derive(Debug, Clone)]
pub(in crate::view) struct BlockAnchor {
    block_idx: usize,
    offset_in_block: f32,
}

// ===== EngineDirty =====

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::view) enum EngineDirty {
    Clean,
    SourceChanged,
    /// Current source projection is ready; visible glyph shaping is deferred to paint.
    QueryLayoutReady,
    /// Reserved for future style-change event (e.g. theme toggle).
    /// Currently unused: style changes are detected via `cached_style_hash` comparison
    /// in `needs_rebuild()`, not via this variant.
    StyleChanged,
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
    pub(in crate::view) lazy: Option<LazyLayout<S>>,
    pub(in crate::view) dirty: EngineDirty,
    pub(in crate::view) cached_style_hash: u64,
    pub(in crate::view) cached_query_style: Option<MarkdownStyle>,
    pub(in crate::view) cached_viewport_w: f32,

    pub scroll_y: f32,
    pub content_height: f32,
    pub(in crate::view) headings: Vec<HeadingEntry>,
    pub(in crate::view) pending_heading_jump: Option<usize>,
    pub(in crate::view) pending_cursor_reveal: Option<usize>,
    pub(in crate::view) cursor_affinity: CursorAffinity,

    pub(in crate::view) sel: SelectionState,
    pub(in crate::view) sel_anchor_byte: Option<usize>,
    pub(in crate::view) sel_cursor_byte: Option<usize>,
    pub(in crate::view) search: SearchHighlightCache,

    pub(in crate::view) cached_dl: Option<DrawList>,
    pub(in crate::view) cached_dl_scroll_y: f32,
    pub(in crate::view) cached_dl_viewport: (f32, f32),
    pub(in crate::view) cached_vertices: Option<Vec<GlyphVertex>>,
    pub(in crate::view) cached_offset_x: f32,
    pub(in crate::view) cached_offset_y: f32,

    pub base_font_family: String,
    pub base_font_size: f32,
    pub base_line_height: f32,
    pub(in crate::view) rendered_body_font_size: f32,
    pub(in crate::view) rendered_line_height: f32,
    pub(in crate::view) body_navigation_font_metrics: NavigationFontMetrics,
    pub(in crate::view) code_navigation_font_metrics: NavigationFontMetrics,
    pub(in crate::view) navigation_metrics_style_hash: u64,
    pub paragraph_spacing: f32,
    pub toc_max_depth: u8,
    pub markdown_first_line_indent: bool,
    pub text_spacing_mode: ui::typography::TextSpacingMode,

    /// WYSIWYG 编辑上下文。None 表示纯预览模式 (快速路径)。
    pub edit_ctx: Option<crate::edit::EditContext>,
    /// Full source text for WYSIWYG span expansion (materialize_line).
    pub(in crate::view) edit_source: Option<SourceSnapshot>,
    /// Generation of the source text currently rendered by this engine.
    pub(in crate::view) source_generation: u32,
    /// Whether the cursor blink phase is currently visible (controlled by app).
    pub cursor_visible: bool,
    /// Actual shaped advance from an editable empty line's start to the IME caret.
    pub(in crate::view) standalone_preedit_cursor_advance: Option<f32>,
    pub(in crate::view) replacement_preedit: Option<preedit::ReplacementPreedit<S>>,
}

pub(in crate::view) fn mix_hash_value(hash: &mut u64, value: u64) {
    *hash = hash.rotate_left(5) ^ value;
}

mod editor_view;
mod engine;
mod engine_caret;
mod engine_navigation;
mod engine_protocol;
mod engine_selection;
mod markdown_view;
mod novel_view;

pub use editor_view::{
    MarkdownEditorView, MarkdownEditorViewFactory, NotoraMarkdownEditorViewFactory,
};
pub use markdown_view::{MarkdownView, MarkdownViewFactory};
pub use novel_view::{NovelView, NovelViewFactory};

#[cfg(test)]
#[path = "view_tests.rs"]
mod view_tests;

pub(in crate::view) fn is_plain_markdown_path(path: Option<&Path>) -> bool {
    path.and_then(|p| p.to_str()).is_some_and(|s| {
        (s.ends_with(".md") || s.ends_with(".markdown")) && !s.ends_with(".mmap.md")
    })
}

pub(in crate::view) fn source_line_at_byte(source: &str, byte: usize) -> Option<SourceLineAtByte> {
    #[cfg(test)]
    SOURCE_LINE_AT_BYTE_CALLS.fetch_add(1, Ordering::Relaxed);

    let source_bytes = source.as_bytes();
    if byte > source_bytes.len() {
        return None;
    }

    // 单次 lookup 场景仍走原地扫描，避免为一次调用现造 SourceLineMap。
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

// `augment_enter` / `augment_backspace` / `augment_insert_text` 及分类器与
// 各 `*_augmentation` 自由函数已在 2026-07-06 方案阶段 2b/2c 迁至
// `crate::augmenter`。仅测试代码需要重导出 EnterContext / classify_enter_context。
#[cfg(test)]
pub(crate) use crate::augmenter::{EnterContext, classify_enter_context};

pub(in crate::view) fn preedit_cursor_offset(
    preedit_text: &str,
    preedit_cursor: Option<(usize, usize)>,
) -> usize {
    let cursor_offset = preedit_cursor.map(|(_, cursor)| cursor).unwrap_or(preedit_text.len());
    let mut valid_offset = cursor_offset.min(preedit_text.len());
    while valid_offset > 0 && !preedit_text.is_char_boundary(valid_offset) {
        valid_offset -= 1;
    }
    valid_offset
}

/// Convert the IME preedit cursor's byte offset into a grapheme index within `preedit_text`.
pub(in crate::view) fn preedit_cursor_grapheme_index(
    preedit_text: &str,
    preedit_cursor: Option<(usize, usize)>,
) -> usize {
    let byte_offset = preedit_cursor_offset(preedit_text, preedit_cursor);
    crate::grapheme_map::grapheme_index_at_byte(preedit_text, byte_offset)
}

pub(in crate::view) fn style_hash_quick(style: &MarkdownStyle) -> u64 {
    let mut hash = 0x517cc1b727220a95;

    mix_hash_value(
        &mut hash,
        match style.text_spacing_mode {
            ui::typography::TextSpacingMode::Natural => 0,
            ui::typography::TextSpacingMode::Verbatim => 1,
        },
    );

    mix_hash_bytes(&mut hash, style.body_font_family.len().to_ne_bytes().as_slice());
    for family in &style.body_font_family {
        mix_hash_bytes(&mut hash, family.len().to_ne_bytes().as_slice());
        mix_hash_bytes(&mut hash, family.as_bytes());
    }
    match &style.code_font_family {
        Some(family) => {
            mix_hash_value(&mut hash, 1);
            mix_hash_bytes(&mut hash, family.as_bytes());
        }
        None => mix_hash_value(&mut hash, 0),
    }

    for dimension in style.heading_font_sizes.into_iter().chain([
        style.body_font_size,
        style.code_font_size,
        style.border_radius_base,
        style.border_radius_small,
        style.list_item_spacing,
        style.list_group_spacing,
        style.rule_spacing,
        style.paragraph_spacing,
        style.paragraph_first_line_indent,
        style.heading_spacing_top,
        style.heading_spacing_bottom,
        style.code_block_padding,
        style.blockquote_padding,
        style.list_indent,
        style.table_cell_padding,
        style.line_height,
        style.code_line_height,
        style.rule_thickness,
        style.rule_width_ratio,
    ]) {
        mix_hash_value(&mut hash, dimension.to_bits() as u64);
    }

    for color in [
        style.text_color,
        style.code_color,
        style.code_bg,
        style.inline_code_bg,
        style.heading_color,
        style.link_color,
        style.rule_color,
        style.blockquote_bg,
        style.blockquote_border,
        style.table_border,
        style.table_header_bg,
        style.table_stripe_bg,
        style.code_block_border,
        style.background_color,
    ] {
        for channel in color {
            mix_hash_value(&mut hash, channel.to_bits() as u64);
        }
    }

    hash
}

pub(in crate::view) fn mix_hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        mix_hash_value(hash, byte as u64);
    }
}

#[cfg(test)]
pub(in crate::view) use editor_view::augmentation_edit_plan;
