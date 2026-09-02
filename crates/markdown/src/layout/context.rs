//! Layout context and width estimation.

use std::collections::HashSet;

use core::unicode::{
    ucd_grapheme_cluster_joins, ucd_grapheme_cluster_joins_done, ucd_grapheme_cluster_lookup,
};
use shaping::Shaper;

use super::types::{FlatLine, LaidOutBlock, LaidOutBlockKind, WrappedLine};

// ===== Width estimation helpers for wrap_text =====

/// Returns true if the character is CJK, fullwidth, or another script
/// where each character occupies roughly `font_size` pixels (monospaced square).
pub fn is_cjk_or_fullwidth(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Extension A
        | '\u{20000}'..='\u{2A6DF}' // CJK Extension B
        | '\u{2A700}'..='\u{2B73F}' // CJK Extension C
        | '\u{2B740}'..='\u{2B81F}' // CJK Extension D
        | '\u{2B820}'..='\u{2CEAF}' // CJK Extension E
        | '\u{2CEB0}'..='\u{2EBEF}' // CJK Extension F
        | '\u{3000}'..='\u{303F}' // CJK Symbols & Punctuation
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{FF01}'..='\u{FF5E}' // Fullwidth Forms
        | '\u{3300}'..='\u{33FF}' // CJK Compatibility
        | '\u{2E80}'..='\u{2EFF}' // CJK Radicals Supplement
        | '\u{2F00}'..='\u{2FDF}' // Kangxi Radicals
        | '\u{31C0}'..='\u{31EF}' // CJK Strokes
        | '\u{3200}'..='\u{32FF}' // Enclosed CJK Letters
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
        | '\u{A000}'..='\u{A48F}' // Yi Syllables
    )
}

/// Find grapheme cluster index at a given x offset within a [`FlatLine`].
/// Uses shaped clusters when available, falls back to width estimation.
pub(crate) fn grapheme_at_x(flat_line: &FlatLine, rel_x: f32) -> usize {
    let text = &flat_line.text;
    if text.is_empty() {
        return 0;
    }
    if let Some(ref shaped) = flat_line.shaped {
        let mut cum_x = 0.0f32;
        for cluster in &shaped.clusters {
            let mid = cum_x + cluster.advance * 0.5;
            if rel_x < mid {
                return crate::grapheme_map::grapheme_index_at_byte(text, cluster.byte_range.start);
            }
            cum_x += cluster.advance;
        }
        return crate::grapheme_map::grapheme_index_at_byte(text, text.len());
    }
    let font_size = flat_line.font_size;
    let mut cum_x = 0.0f32;
    let mut g_idx = 0usize;
    // Iterate grapheme clusters (not chars) so combining marks and ZWJ
    // emoji are counted as single visual positions.
    let mut chars = text.char_indices().peekable();
    while let Some((_byte, ch)) = chars.next() {
        let cw = if is_cjk_or_fullwidth(ch) { font_size } else { font_size * 0.55 };
        if rel_x < cum_x + cw * 0.5 {
            return g_idx;
        }
        cum_x += cw;
        g_idx += 1;
        // Skip continuation chars within the same grapheme cluster.
        let mut prev_props = ucd_grapheme_cluster_lookup(ch);
        let mut state = 0u32;
        while let Some(&(_, next_ch)) = chars.peek() {
            let next_props = ucd_grapheme_cluster_lookup(next_ch);
            state = ucd_grapheme_cluster_joins(state, prev_props, next_props);
            if ucd_grapheme_cluster_joins_done(state) {
                break;
            }
            prev_props = next_props;
            chars.next();
        }
    }
    g_idx
}

/// Get the pixel x-coordinate for a grapheme position within a [`FlatLine`].
pub(crate) fn grapheme_x(flat_line: &FlatLine, visual_grapheme: usize) -> f32 {
    let text = &flat_line.text;
    if text.is_empty() || visual_grapheme == 0 {
        return 0.0;
    }
    if let Some(ref shaped) = flat_line.shaped {
        // Find the byte position of the visual_grapheme'th grapheme cluster.
        let mut g_count = 0usize;
        let mut target_byte = text.len();
        let mut chars = text.char_indices().peekable();
        while let Some((byte, ch)) = chars.next() {
            if g_count == visual_grapheme {
                target_byte = byte;
                break;
            }
            g_count += 1;
            let mut prev_props = ucd_grapheme_cluster_lookup(ch);
            let mut state = 0u32;
            while let Some(&(_, next_ch)) = chars.peek() {
                let next_props = ucd_grapheme_cluster_lookup(next_ch);
                state = ucd_grapheme_cluster_joins(state, prev_props, next_props);
                if ucd_grapheme_cluster_joins_done(state) {
                    break;
                }
                prev_props = next_props;
                chars.next();
            }
        }
        let mut cum_x = 0.0f32;
        for cluster in &shaped.clusters {
            if cluster.byte_range.start >= target_byte {
                break;
            }
            cum_x += cluster.advance;
        }
        return cum_x;
    }
    let font_size = flat_line.font_size;
    let mut cum_x = 0.0f32;
    let mut g_idx = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((_byte, ch)) = chars.next() {
        if g_idx >= visual_grapheme {
            break;
        }
        cum_x += if is_cjk_or_fullwidth(ch) { font_size } else { font_size * 0.55 };
        g_idx += 1;
        let mut prev_props = ucd_grapheme_cluster_lookup(ch);
        let mut state = 0u32;
        while let Some(&(_, next_ch)) = chars.peek() {
            let next_props = ucd_grapheme_cluster_lookup(next_ch);
            state = ucd_grapheme_cluster_joins(state, prev_props, next_props);
            if ucd_grapheme_cluster_joins_done(state) {
                break;
            }
            prev_props = next_props;
            chars.next();
        }
    }
    cum_x
}

/// Single-character pixel advance using the shaper when needed (cached).
/// CJK/fullwidth chars use the pre-measured `cjk_advance`; everything else
/// falls back to `shaper.grapheme_advance`.
#[allow(dead_code)] // Used by measure_char_widths indirectly
fn char_width(ch: char, cjk_advance: f32, font_size: f32, shaper: &mut Shaper) -> f32 {
    if is_cjk_or_fullwidth(ch) {
        return cjk_advance;
    }
    let mut buf = [0u8; 4];
    let s = ch.encode_utf8(&mut buf);
    shaper.grapheme_advance(s).unwrap_or(font_size * 0.55)
}

/// Estimate the pixel width of a token without full shaping.
///
/// `cjk_advance` should be the measured advance of one CJK ideograph at the
/// current font — measured once per `wrap_text` call, not once per token.
///
/// Strategy:
/// - Pure CJK → `cjk_advance × char_count` (zero shaping)
/// - Pure ASCII → sum of per-char `char_width` (cache hits after first use)
/// - Mixed/other → shape once
fn estimate_token_width(
    token: &str,
    cjk_advance: f32,
    ascii_widths: &[f32; 128],
    font_size_scale: f32,
    shaper: &mut Shaper,
) -> f32 {
    if token.is_empty() {
        return 0.0;
    }
    // Fast path: pure ASCII via lookup table (zero HarfBuzz)
    let mut all_ascii = true;
    let mut ascii_sum = 0.0f32;
    for b in token.bytes() {
        if !(0x20..0x7f).contains(&b) {
            all_ascii = false;
            break;
        }
        ascii_sum += ascii_widths[b as usize];
    }
    if all_ascii {
        return ascii_sum * font_size_scale;
    }
    // Fast path: pure CJK
    let all_cjk = token.chars().all(is_cjk_or_fullwidth);
    if all_cjk {
        return cjk_advance * token.chars().count() as f32;
    }
    // Mixed: shape once
    shaper.shape(token).map(|r| r.width).unwrap_or_else(|_| {
        ascii_widths[0x58] * font_size_scale * token.chars().count() as f32 // 'x' width as fallback
    })
}

/// Measure per-character widths in one shape() call.
/// Returns (char_widths, total_width).
fn measure_char_widths(
    token: &str,
    cjk_advance: f32,
    ascii_widths: &[f32; 128],
    font_size: f32,
    font_size_scale: f32,
    shaper: &mut Shaper,
) -> (Vec<f32>, f32) {
    let chars: Vec<char> = token.chars().collect();
    let n = chars.len();
    let mut widths = vec![0.0f32; n];
    // Fast path: pure CJK
    if chars.iter().all(|c| is_cjk_or_fullwidth(*c)) {
        widths.fill(cjk_advance);
        return (widths, cjk_advance * n as f32);
    }
    // Fast path: pure ASCII via lookup table
    let all_ascii = token.bytes().all(|b| (0x20..0x7f).contains(&b));
    if all_ascii {
        for (i, b) in token.bytes().enumerate() {
            widths[i] = ascii_widths[b as usize] * font_size_scale;
        }
        let total: f32 = widths.iter().sum();
        return (widths, total);
    }
    // Shape the whole token, then distribute cluster widths to char slots
    if let Ok(run) = shaper.shape(token) {
        let mut char_idx = 0usize;
        for cluster in &run.clusters {
            let cluster_chars: Vec<char> = token[cluster.byte_range.clone()].chars().collect();
            let cluster_w = cluster.advance;
            let cc = cluster_chars.len().max(1);
            let per_char = cluster_w / cc as f32;
            for _ in 0..cc {
                if char_idx < n {
                    widths[char_idx] = per_char;
                    char_idx += 1;
                }
            }
        }
        // Fill any remaining (shouldn't happen for well-formed text)
        while char_idx < n {
            widths[char_idx] = font_size * 0.55;
            char_idx += 1;
        }
        let total: f32 = widths.iter().sum();
        (widths, total)
    } else {
        // Fallback: estimate
        let w = font_size * 0.55;
        widths.fill(w);
        (widths, w * n as f32)
    }
}

// ===== Layout context =====

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LastBlockKind {
    Paragraph,
    Heading,
    CodeBlock,
    BlockQuote,
    HorizontalRule,
    TableWrapper,
    MetadataBlock,
    ListItem,
}

/// 块型的 trailing 间距 —— 块间距规则的单一事实源。
/// `layout_block` 每块排完后、增量重排路径恢复上一块上下文、以及 debug
/// 间距展示都取这里的值；新增/调整间距规则只需改此函数。
pub(crate) fn trailing_spacing_for(
    kind: LastBlockKind,
    style: &crate::style::MarkdownStyle,
) -> f32 {
    match kind {
        LastBlockKind::Paragraph
        | LastBlockKind::CodeBlock
        | LastBlockKind::BlockQuote
        | LastBlockKind::TableWrapper
        | LastBlockKind::MetadataBlock => style.paragraph_spacing,
        LastBlockKind::Heading => style.heading_spacing_bottom,
        LastBlockKind::ListItem => style.list_item_spacing,
        LastBlockKind::HorizontalRule => style.rule_spacing,
    }
}

/// 标题顶端间距(margin collapsing):首块减半;前块已是标题则不再叠加;
/// 否则只补超出前块 trailing 的部分。`layout_block` 的 Heading 分支、
/// 单块重排预扣与 debug 间距展示共用此公式。
pub(crate) fn heading_top_spacing(
    style: &crate::style::MarkdownStyle,
    level: u8,
    is_first_block: bool,
    prev_was_heading: bool,
    prev_trailing_spacing: f32,
) -> f32 {
    if prev_was_heading {
        return 0.0;
    }
    let desired_top = style.heading_spacing_top * super::heading_spacing_scale(level);
    if is_first_block { desired_top * 0.5 } else { (desired_top - prev_trailing_spacing).max(0.0) }
}

/// 由上一块的布局产物重建间距上下文块型:Text 需借文档块类型区分标题与
/// 段落(active 的 HR 也排成 Text,按段落处理,与 layout_block 的 HR
/// active 分支一致)。
pub(crate) fn spacing_kind_of_laid_block(
    laid_kind: &LaidOutBlockKind,
    doc_kind: Option<&crate::builder::BlockKind>,
) -> LastBlockKind {
    match laid_kind {
        LaidOutBlockKind::Text { .. } => match doc_kind {
            Some(crate::builder::BlockKind::Heading { .. }) => LastBlockKind::Heading,
            _ => LastBlockKind::Paragraph,
        },
        LaidOutBlockKind::ListItem { .. } => LastBlockKind::ListItem,
        LaidOutBlockKind::CodeBlock { .. } => LastBlockKind::CodeBlock,
        LaidOutBlockKind::BlockQuote { .. } => LastBlockKind::BlockQuote,
        LaidOutBlockKind::HorizontalRule => LastBlockKind::HorizontalRule,
        LaidOutBlockKind::Table { .. } => LastBlockKind::TableWrapper,
        LaidOutBlockKind::MetadataBlock { .. } => LastBlockKind::MetadataBlock,
    }
}

pub struct LayoutCtx<'a> {
    pub(crate) doc: &'a dyn core::document::DocView,
    pub(crate) style: &'a crate::style::MarkdownStyle,
    pub(crate) viewport_w: f32,
    pub(crate) y: f32,
    pub(crate) indent: f32,
    pub(crate) output: Vec<LaidOutBlock>,
    pub(crate) shaper: Option<&'a mut Shaper>,
    /// Track whether the previous block was a heading (for margin collapsing).
    pub(crate) last_block_was_heading: bool,
    /// Track whether the previous block was a list item (for inter-item spacing).
    pub(crate) last_block_was_list: bool,
    /// Count of blocks processed (for first-block special handling).
    pub(crate) block_count: usize,
    /// Color fade ratio for text in nested contexts (e.g. blockquote).
    /// 0.0 = normal, 1.0 = fully background color.
    pub(crate) color_fade: f32,
    /// Font size override for nested contexts (e.g. blockquote).
    /// When set, child text blocks use this instead of body_font_size.
    pub(crate) font_size_override: Option<f32>,
    /// Current list nesting depth (0 = top-level list).
    pub(crate) list_depth: usize,
    /// Pre-computed ASCII character widths (indexed by char value).
    /// Built once at layout start; makes per-char width lookups O(1).
    pub(crate) ascii_widths: [f32; 128],
    pub(crate) highlighter: Option<&'a dyn crate::builder::CodeHighlighter>,
    /// Track the kind of the previous block for spacing decisions.
    pub(crate) last_block_kind: Option<LastBlockKind>,
    /// Trailing spacing added by the previous block (for margin collapsing with headings).
    pub(crate) last_trailing_spacing: f32,
    /// Shaped runs from the last wrap_text_with_width call, one per input line (split by newline).
    /// Used by layout_text_block to create per-segment text_layout without re-shaping.
    pub(crate) last_wrap_shaped: Vec<Option<shaping::ShapedRun>>,
    /// Full source text, used by materialize_line for WYSIWYG span expansion.
    pub(crate) source_text: Option<&'a str>,
    /// WYSIWYG edit context; None means pure preview (no cursor span expansion).
    pub(crate) edit_ctx: Option<&'a crate::edit::EditContext>,
    /// Non-empty source selection range used for block-level editing fallbacks.
    pub(crate) selection_range: Option<&'a std::ops::Range<usize>>,
    /// Crate-private code-block render metadata, kept outside public layout output structs.
    pub(crate) ascii_diagrams: super::ascii_diagram::AsciiDiagramRegistry,
    /// Active block marker when cursor is in a heading/list/blockquote source range.
    /// Set by the block handler before layout, cleared after.
    pub(crate) active_block_marker: Option<crate::edit::ActiveBlockMarker>,
    /// Physical source lines whose preceding blank run has already been reserved.
    compensated_blank_run_line_starts: HashSet<usize>,
}

impl<'a> LayoutCtx<'a> {
    pub fn new(
        doc: &'a dyn core::document::DocView,
        style: &'a crate::style::MarkdownStyle,
        viewport_w: f32,
        mut shaper: Option<&'a mut Shaper>,
        highlighter: Option<&'a dyn crate::builder::CodeHighlighter>,
        source_text: Option<&'a str>,
        edit_ctx: Option<&'a crate::edit::EditContext>,
    ) -> Self {
        let mut ascii_widths = [0.0f32; 128];
        if let Some(ref mut s) = shaper {
            let old_size = s.font_size();
            let old_weight = s.font_weight();
            let old_style = s.font_style();
            let old_family = s.font_family().map(|family| family.to_string());
            s.set_font_size(style.body_font_size);
            s.set_font_weight(shaping::Weight::NORMAL);
            s.set_font_style(shaping::Style::Normal);
            s.set_font_family(style.body_font_family.first().map(|family| family.as_str()));
            for c in 0x20..0x7f {
                let mut buf = [0u8; 4];
                let ch = char::from_u32(c).unwrap();
                let s_str = ch.encode_utf8(&mut buf);
                ascii_widths[c as usize] =
                    s.grapheme_advance(s_str).unwrap_or(style.body_font_size * 0.55);
            }
            s.set_font_size(old_size);
            s.set_font_weight(old_weight);
            s.set_font_style(old_style);
            s.set_font_family(old_family.as_deref());
        } else {
            let w = style.body_font_size * 0.55;
            for c in 0x20..0x7f {
                ascii_widths[c as usize] = w;
            }
        }
        Self {
            doc,
            style,
            viewport_w,
            y: 0.0,
            indent: 0.0,
            output: vec![],
            shaper,
            last_block_was_heading: false,
            last_block_was_list: false,
            block_count: 0,
            color_fade: 0.0,
            font_size_override: None,
            list_depth: 0,
            ascii_widths,
            highlighter,
            last_block_kind: None,
            last_trailing_spacing: 0.0,
            last_wrap_shaped: Vec::new(),
            source_text,
            edit_ctx,
            selection_range: None,
            ascii_diagrams: super::ascii_diagram::AsciiDiagramRegistry::default(),
            active_block_marker: None,
            compensated_blank_run_line_starts: HashSet::new(),
        }
    }

    pub(crate) fn reserve_blank_source_run(
        &mut self,
        following_line_start: usize,
        blank_line_count: usize,
    ) -> f32 {
        if !self.compensated_blank_run_line_starts.insert(following_line_start) {
            return 0.0;
        }
        blank_line_count.saturating_sub(1) as f32
            * (self.style.line_height + self.style.paragraph_spacing)
    }

    pub(crate) fn available_width(&self) -> f32 {
        (self.viewport_w - self.indent).max(20.0)
    }

    /// 块排完后记录间距上下文,返回本块的 trailing 间距(调用方据此推进
    /// `ctx.y`;HR 等已把间距烘进块高的块型不再推进)。
    pub(crate) fn finish_block_spacing(&mut self, kind: LastBlockKind) -> f32 {
        let trailing = trailing_spacing_for(kind, self.style);
        self.last_trailing_spacing = trailing;
        self.last_block_kind = Some(kind);
        self.last_block_was_heading = kind == LastBlockKind::Heading;
        self.last_block_was_list = kind == LastBlockKind::ListItem;
        self.block_count += 1;
        trailing
    }

    /// 单块重排前恢复上一块的间距上下文(trailing 取间距规则单一事实源,
    /// 与 layout_block 排完该块后的状态逐项一致)。
    pub(crate) fn restore_spacing_context(&mut self, kind: LastBlockKind) {
        self.last_block_kind = Some(kind);
        self.last_trailing_spacing = trailing_spacing_for(kind, self.style);
        self.last_block_was_heading = kind == LastBlockKind::Heading;
        self.last_block_was_list = kind == LastBlockKind::ListItem;
    }

    /// 标题 margin collapsing 预扣:layout_block 的 Heading 分支会把同一
    /// 数值加回 `ctx.y`,预扣使单块重排与全文布局落点一致。
    pub(crate) fn presubtract_heading_top_spacing(&mut self, src_kind: &crate::builder::BlockKind) {
        let crate::builder::BlockKind::Heading { level } = src_kind else {
            return;
        };
        self.y -= heading_top_spacing(
            self.style,
            *level,
            self.block_count == 0,
            self.last_block_was_heading,
            self.last_trailing_spacing,
        );
    }

    /// 单块重排前预扣全部入口间距:estimated_positions 已包含 layout_block
    /// 入口的间距调整(列表组收尾 bump、tight 列表缩减、标题顶端 margin
    /// collapsing),而 layout_block 会再应用一次;此处预先反向扣除以抵消,
    /// 保证重排落点与全文布局一致。guard 与 trailing 变化镜像 layout_block
    /// 入口及 ListItem 分支,二者必须同步修改。
    pub(crate) fn presubtract_entry_spacing(&mut self, src_kind: &crate::builder::BlockKind) {
        // 列表组收尾 bump(镜像 layout_block 入口)
        if self.last_block_was_list
            && !matches!(src_kind, crate::builder::BlockKind::ListItem { .. })
        {
            self.y -= self.style.list_group_spacing - self.style.list_item_spacing;
            self.last_trailing_spacing = self.style.list_group_spacing;
        }
        // tight 列表紧跟段落的缩减(镜像 layout_block ListItem 分支的 guard)
        if let crate::builder::BlockKind::ListItem { tight, blank_line_before, .. } = src_kind
            && *tight
            && !*blank_line_before
            && !self.last_block_was_list
            && self.last_block_kind == Some(LastBlockKind::Paragraph)
            && self.last_trailing_spacing > self.style.list_item_spacing
        {
            self.y += self.last_trailing_spacing - self.style.list_item_spacing;
        }
        self.presubtract_heading_top_spacing(src_kind);
    }

    pub(crate) fn push_block(&mut self, kind: LaidOutBlockKind, h: f32) {
        use ui::core::geom::Rect;
        let rect = Rect::new(self.indent, self.y, self.available_width(), h);
        self.output.push(LaidOutBlock { kind, rect });
        self.y += h;
    }

    /// Word wrap to the full available viewport width (default).
    pub(crate) fn wrap_text(
        &mut self,
        text: &str,
        font_size: f32,
        font_weight: shaping::Weight,
    ) -> Vec<WrappedLine> {
        self.wrap_text_with_width(text, font_size, font_weight, self.available_width())
    }

    /// Word wrap with an explicit maximum width in pixels.
    /// Used for table cells, blockquotes, and other constrained contexts.
    pub(crate) fn wrap_text_with_width(
        &mut self,
        text: &str,
        font_size: f32,
        font_weight: shaping::Weight,
        max_w: f32,
    ) -> Vec<WrappedLine> {
        let mut lines = Vec::new();
        self.last_wrap_shaped.clear();
        let mut input_offset = 0usize;

        for input_line in text.split('\n') {
            if input_line.is_empty() {
                lines.push(WrappedLine {
                    text: String::new(),
                    byte_start: input_offset,
                    byte_end: input_offset,
                });
                self.last_wrap_shaped.push(None);
                input_offset += 1; // account for the '\n'
                continue;
            }
            if let Some(ref mut shaper) = self.shaper {
                // Reuse the same wrapping algorithm as the editor (ui::layout::compute_visual_lines).
                // Shape the text, then use the shared cluster-based line-break algorithm.
                let old_size = shaper.font_size();
                let old_weight = shaper.font_weight();
                let old_style = shaper.font_style();
                let old_family = shaper.font_family().map(|family| family.to_string());
                shaper.set_font_size(font_size);
                shaper.set_font_weight(font_weight);
                shaper.set_font_style(shaping::Style::Normal);
                shaper.set_font_family(
                    self.style.body_font_family.first().map(|family| family.as_str()),
                );
                let shaped = shaper.shape(input_line);
                let char_width = shaper.grapheme_advance(" ").unwrap_or(font_size * 0.3);
                shaper.set_font_size(old_size);
                shaper.set_font_weight(old_weight);
                shaper.set_font_style(old_style);
                shaper.set_font_family(old_family.as_deref());

                if let Ok(shaped) = shaped {
                    self.last_wrap_shaped.push(Some(shaped.clone()));
                    let line_bytes = input_line.as_bytes();
                    let visual_lines = ui::layout::compute_visual_lines(
                        &shaped.clusters,
                        line_bytes,
                        char_width,
                        max_w,
                        0.0,
                    );
                    if visual_lines.is_empty() {
                        lines.push(WrappedLine {
                            text: String::new(),
                            byte_start: input_offset,
                            byte_end: input_offset,
                        });
                    } else {
                        // Convert cluster ranges to byte ranges within input_line.
                        // compute_visual_lines may skip leading whitespace on continuation
                        // lines; to preserve all bytes we track prev_end and include any
                        // gap (skipped whitespace) as trailing content of the current segment.
                        let line_start = input_offset;
                        let mut prev_cluster_end: usize = 0;
                        for (_, vl_end, _) in &visual_lines {
                            let cluster_end = if *vl_end > 0 {
                                shaped.clusters[*vl_end - 1].byte_range.end
                            } else {
                                prev_cluster_end
                            };
                            let seg_text = &input_line[prev_cluster_end..cluster_end];
                            lines.push(WrappedLine {
                                text: seg_text.to_string(),
                                byte_start: line_start + prev_cluster_end,
                                byte_end: line_start + cluster_end,
                            });
                            prev_cluster_end = cluster_end;
                        }
                        // Capture any trailing bytes after the last visual line
                        if prev_cluster_end < input_line.len() {
                            let trailing = &input_line[prev_cluster_end..];
                            if let Some(last) = lines.last_mut() {
                                last.text.push_str(trailing);
                                last.byte_end = line_start + input_line.len();
                            }
                        }
                        input_offset = line_start + input_line.len();
                    }
                } else {
                    // Shaping failed; treat as single unbroken line
                    self.last_wrap_shaped.push(None);
                    lines.push(WrappedLine {
                        text: input_line.to_string(),
                        byte_start: input_offset,
                        byte_end: input_offset + input_line.len(),
                    });
                    input_offset += input_line.len();
                }
            } else {
                // Fallback: character-count estimate (no shaper available)
                self.last_wrap_shaped.push(None);
                let char_w = font_size * 0.55;
                let max_bytes = ((max_w / char_w) as usize).max(10);
                let mut remaining = input_line;
                while !remaining.is_empty() {
                    if remaining.len() <= max_bytes {
                        lines.push(WrappedLine {
                            text: remaining.to_string(),
                            byte_start: input_offset,
                            byte_end: input_offset + remaining.len(),
                        });
                        input_offset += remaining.len();
                        break;
                    }
                    let safe_end = remaining.floor_char_boundary(max_bytes);
                    let split_at =
                        remaining[..safe_end].rfind(' ').map(|p| p + 1).unwrap_or(safe_end);
                    lines.push(WrappedLine {
                        text: remaining[..split_at].to_string(),
                        byte_start: input_offset,
                        byte_end: input_offset + split_at,
                    });
                    input_offset += split_at;
                    let next = &remaining[split_at..];
                    let trimmed_next = next.trim_start();
                    let skipped_len = next.len() - trimmed_next.len();
                    if skipped_len > 0
                        && let Some(last) = lines.last_mut()
                    {
                        // Keep skipped whitespace in the previous segment so its
                        // text and source byte range remain identical.  The
                        // fallback layout is later replaced by precise shaping,
                        // but it still supplies projections for the whole document.
                        last.text.push_str(&next[..skipped_len]);
                        last.byte_end += skipped_len;
                        input_offset += skipped_len;
                    }
                    remaining = trimmed_next;
                }
            }
            input_offset += 1; // account for the '\n' separator
        }
        if lines.is_empty() {
            lines.push(WrappedLine { text: String::new(), byte_start: 0, byte_end: 0 });
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_legacy_markdown_width_ranges() {
        assert!(is_cjk_or_fullwidth('：'));
        assert!(!is_cjk_or_fullwidth('｟'));
        assert!(!is_cjk_or_fullwidth('￦'));
        assert!(!is_cjk_or_fullwidth('ᄀ'));
        assert!(!is_cjk_or_fullwidth('A'));
    }

    #[test]
    fn ordinary_markdown_width_keeps_legacy_classifier_for_unicode_width_extensions() {
        for character in ['｟', '￦', 'ᄀ'] {
            assert!(
                !is_cjk_or_fullwidth(character),
                "ordinary Markdown must retain its existing narrow-width estimate for {character}"
            );
        }
    }
    use crate::test_utils::default_style;

    #[test]
    fn wrap_text_cjk_preserves_spaces_in_output() {
        let src = "";
        // Soft-break spaces between CJK tokens must stay in the output string
        // (for correct byte offset tracking in layout_text_block), even though
        // the gap width for fit decisions is zero.
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 800.0, Some(&mut shaper), None, None, None);
        // Wide viewport: both tokens should fit on one line
        let lines = ctx.wrap_text("第一行内容 第二行内容", 14.0, shaping::Weight::NORMAL);
        assert!(!lines.is_empty());
        let joined: String = lines.iter().map(|l| l.text.as_str()).collect();
        // The space char must be preserved
        assert!(
            joined.contains(' '),
            "space between CJK tokens must be preserved, got {:?}",
            lines
        );
    }

    #[test]
    fn wrap_text_cjk_narrow_viewport_fills_chars() {
        let src = "";
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 150.0, Some(&mut shaper), None, None, None);
        let long_cjk = "这是一段很长的中文文本用来测试自动换行功能";
        let lines = ctx.wrap_text(long_cjk, 14.0, shaping::Weight::NORMAL);
        assert!(lines.len() > 1, "narrow viewport should produce multiple lines, got {:?}", lines);
        for line in &lines {
            assert!(!line.text.is_empty(), "no empty lines expected from pure CJK char fill");
        }
    }

    #[test]
    fn wrap_text_mixed_cjk_ascii_no_panic() {
        let src = "";
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 300.0, Some(&mut shaper), None, None, None);
        let mixed = "中文 English mixed 混合文本 test";
        let lines = ctx.wrap_text(mixed, 14.0, shaping::Weight::NORMAL);
        assert!(!lines.is_empty());
        // Mixed text: space between ASCII and CJK tokens should be preserved
        let joined = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("English"));
        assert!(joined.contains("test"));
    }

    #[test]
    fn wrap_text_softbreak_joined_paragraph_no_panic() {
        let src = "";
        // Regression: builder-joined paragraphs (2600+ chars with spaces)
        // must not panic and must finish quickly.
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 700.0, Some(&mut shaper), None, None, None);
        let parts: Vec<&str> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    "这是一段中文内容用来测试换行"
                } else {
                    "另外一段中文文本需要进行排版"
                }
            })
            .collect();
        let paragraph = parts.join(" ");
        assert!(paragraph.len() > 200);
        let _lines = ctx.wrap_text(&paragraph, 14.0, shaping::Weight::NORMAL);
        // If we got here without panicking, the char-boundary logic works
    }

    #[test]
    fn wrap_text_preserves_byte_length_with_cjk() {
        let src = "";
        // Regression: when a token wraps entirely to the next line (fill_end == 0),
        // the space before it was lost, causing byte_offset drift that led to
        // non-char-boundary slicing panics in render.rs.
        let text = "状图（├── mod.rs # comment）整行长度可能 > 200px 在 188px 的窄 cell 中既填不满也不会正确折行";
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        // Use a narrow width that forces wrapping mid-sentence
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 188.0, Some(&mut shaper), None, None, None);
        let wrapped = ctx.wrap_text(text, 14.0, shaping::Weight::NORMAL);
        // Simulate what layout_line_with_styles does: track byte_offset
        let mut byte_offset = 0usize;
        for w in &wrapped {
            let seg_start = byte_offset;
            let seg_end = seg_start + w.text.len();
            // seg_start..seg_end must be valid byte ranges in the original text
            assert!(
                text.get(seg_start..seg_end).is_some(),
                "byte range {}..{} is not valid for original text (len={}): wrapped={:?}",
                seg_start,
                seg_end,
                text.len(),
                wrapped
            );
            byte_offset = seg_end;
        }
        // Total must equal original text length (byte-preserving wrap)
        assert_eq!(
            byte_offset,
            text.len(),
            "total wrapped bytes ({}) != original ({}), wrapped={:?}",
            byte_offset,
            text.len(),
            wrapped
        );
    }

    #[test]
    fn wrap_text_uses_markdown_body_font_not_incoming_shaper_family() {
        let src = "";
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let text = "一. 定向军士:冲，不押宝.只填愿意去的军士院校和专业；优先技术型、军工/航空/信息/船舶/交通背景强的院校";

        let wrap_after_family = |family: Option<&str>| {
            let mut shaper = shaping::Shaper::new().unwrap();
            shaper.set_font_family(family);
            let mut ctx =
                LayoutCtx::new(&src_view, &style, 420.0, Some(&mut shaper), None, None, None);
            ctx.wrap_text(text, 15.0, shaping::Weight::NORMAL)
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            wrap_after_family(Some("monospace")),
            wrap_after_family(Some("PingFang SC")),
            "line wrapping must not depend on the shaper font family left by earlier draws"
        );
    }

    #[test]
    fn wrap_preserves_bytes_with_double_space() {
        let src = "";
        // Regression: consecutive spaces produce empty tokens from split(' ').
        // The wrapper must preserve these space bytes for correct byte tracking.
        let text = "状图（├── mod.rs  # comment）整行长度";
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 188.0, Some(&mut shaper), None, None, None);
        let wrapped = ctx.wrap_text(text, 14.0, shaping::Weight::NORMAL);
        let mut byte_offset = 0usize;
        for w in &wrapped {
            let seg_start = byte_offset;
            let seg_end = seg_start + w.text.len();
            assert!(
                text.get(seg_start..seg_end).is_some(),
                "byte range {}..{} invalid for text len {}",
                seg_start,
                seg_end,
                text.len()
            );
            byte_offset = seg_end;
        }
        assert_eq!(byte_offset, text.len(), "total bytes must match");
    }

    #[test]
    fn wrap_preserves_bytes_with_triple_space() {
        let src = "";
        let text = "mod.rs   # comment";
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 400.0, Some(&mut shaper), None, None, None);
        let wrapped = ctx.wrap_text(text, 14.0, shaping::Weight::NORMAL);
        let total: usize = wrapped.iter().map(|w| w.text.len()).sum();
        assert_eq!(total, text.len(), "triple space bytes must be preserved");
    }

    #[test]
    fn wrap_preserves_bytes_with_leading_space() {
        let src = "";
        let text = "  hello world";
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        let src_view = core::document::StringDocView::new(src);
        let mut ctx = LayoutCtx::new(&src_view, &style, 400.0, Some(&mut shaper), None, None, None);
        let wrapped = ctx.wrap_text(text, 14.0, shaping::Weight::NORMAL);
        let total: usize = wrapped.iter().map(|w| w.text.len()).sum();
        assert_eq!(
            total,
            text.len(),
            "leading space bytes must be preserved: wrapped={:?}",
            wrapped
        );
    }

    #[test]
    fn wrap_preserves_bytes_all_whitespace() {
        let src = "";
        // Regression: all-whitespace lines lost their space bytes because
        // split(' ') produces only empty tokens and leading_spaces was discarded.
        let texts = [" ", "  ", "   ", "    "];
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        for text in &texts {
            let src_view = core::document::StringDocView::new(src);
            let mut ctx =
                LayoutCtx::new(&src_view, &style, 400.0, Some(&mut shaper), None, None, None);
            let wrapped = ctx.wrap_text(text, 14.0, shaping::Weight::NORMAL);
            let total: usize = wrapped.iter().map(|w| w.text.len()).sum();
            assert_eq!(
                total,
                text.len(),
                "all-whitespace bytes lost: text={:?} wrapped={:?} total={} expected={}",
                text,
                wrapped,
                total,
                text.len()
            );
        }
    }

    #[test]
    fn wrap_preserves_bytes_across_viewport_widths() {
        let src = "";
        // Simulate viewport resize: wrapping at different widths must always
        // preserve total byte count of the original text.
        let texts = [
            "状图（├── mod.rs  # comment）整行长度可能 > 200px，在 188px 的窄 cell 中既填不满也不会正确折行——",
            "2），再绘制文字。",
            "中文测试 这是一段很长的中文文本用来测试换行功能是否正确",
            "mixed 混合 text 文本 with  double  spaces",
            "  leading spaces and trailing  ",
            " ",
            "  ",
        ];
        let widths = [50.0, 80.0, 120.0, 188.0, 250.0, 400.0, 800.0];
        let mut shaper = shaping::Shaper::new().unwrap();
        let style = default_style();
        for text in &texts {
            for &w in &widths {
                let src_view = core::document::StringDocView::new(src);
                let mut ctx =
                    LayoutCtx::new(&src_view, &style, w, Some(&mut shaper), None, None, None);
                let wrapped = ctx.wrap_text(text, 14.0, shaping::Weight::NORMAL);
                let total: usize = wrapped.iter().map(|l| l.byte_end - l.byte_start).sum();
                assert_eq!(
                    total,
                    text.len(),
                    "byte loss at width={}: original={} wrapped={} text={:?} lines={:?}",
                    w,
                    text.len(),
                    total,
                    text,
                    wrapped
                );
            }
        }
    }

    #[test]
    fn fallback_wrap_keeps_text_and_projection_ranges_in_sync() {
        let text = "provider/send 未配置时上传锁不释放";
        let style = default_style();
        let src_view = core::document::StringDocView::new("");
        let mut ctx = LayoutCtx::new(&src_view, &style, 100.0, None, None, None, None);
        let wrapped = ctx.wrap_text_with_width(text, 14.0, shaping::Weight::NORMAL, 100.0);

        let mut previous_end = 0;
        for line in &wrapped {
            assert_eq!(line.byte_start, previous_end);
            assert_eq!(text.get(line.byte_start..line.byte_end), Some(line.text.as_str()));
            previous_end = line.byte_end;
        }
        assert_eq!(previous_end, text.len());
    }
}
