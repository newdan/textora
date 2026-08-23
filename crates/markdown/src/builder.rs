//! Builder — converts parser events into a BlockNode tree.
//! Inspired by zed's MarkdownElementBuilder.

use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd, ParsedMarkdown};
use crate::projection::{ProjectedText, TextProjectionBuilder};
use crate::style::MarkdownStyle;
use pulldown_cmark::Alignment;
use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

// ===== Block 节点 =====

#[derive(Clone, Debug)]
pub enum BlockSource {
    Continuous(Range<usize>),
    Fragmented(Vec<Range<usize>>),
}

#[derive(Clone, Debug)]
pub struct BlockNode {
    pub kind: BlockKind,
    pub children: Vec<BlockNode>,
    /// The source range containing the text for this block.
    pub source_range: BlockSource,
    /// Style spans for each logical text line (this assumes text formatting is independent).
    pub text_styles: Vec<Vec<StyleSpan>>,
    /// Parsed text lines (Markdown path only). Populated by the builder from
    /// pending_line text so style span offsets match the content, not the
    /// markup source. Empty for Novel zero-copy path.
    pub text_lines: Vec<String>,
    /// Source-to-visual projection for each parsed Markdown text line.
    /// Empty for Novel zero-copy path.
    pub(crate) projected_lines: Vec<ProjectedText>,
    /// 此 Block 在原始源码中的整体字节范围 (含标记符)。用于增量 diff 和字节定位。
    pub block_range: Range<usize>,
    /// CodeBlock 每一行内容在 source 中的起始位置
    pub code_line_source_starts: Option<Vec<usize>>,
}

impl BlockNode {
    pub fn lines<'a>(
        &'a self,
        doc: &'a dyn core::document::DocView,
    ) -> Vec<std::borrow::Cow<'a, str>> {
        // Markdown path: style offsets are relative to parsed text (no markup).
        if !self.text_lines.is_empty() {
            return self
                .text_lines
                .iter()
                .map(|s| std::borrow::Cow::Borrowed(s.as_str()))
                .collect();
        }
        // Novel zero-copy path: fetch text from source byte ranges.
        match &self.source_range {
            BlockSource::Continuous(r) => {
                let text = doc.doc_text_in_range(r.clone());
                if text.is_empty() {
                    vec![]
                } else {
                    // Strip trailing \n so split doesn't produce a spurious
                    // empty final line (the byte range includes the last
                    // line's newline).  Without this every paragraph would
                    // carry an extra blank line, making the visual gap
                    // between paragraphs ~1.5× line_height instead of the
                    // intended 0.5×.
                    match text {
                        std::borrow::Cow::Borrowed(s) => {
                            // s has lifetime 'a from doc_text_in_range — the
                            // returned Cow::Borrowed slices will be valid.
                            let s = s.strip_suffix('\n').unwrap_or(s);
                            s.split('\n')
                                .map(|line| std::borrow::Cow::Borrowed(line.trim_end_matches('\r')))
                                .collect()
                        }
                        std::borrow::Cow::Owned(mut s) => {
                            // Strip trailing \n before split so the owned copy
                            // doesn't carry the trailing newline.
                            if s.ends_with('\n') {
                                s.pop();
                            }
                            s.split('\n')
                                .map(|line| {
                                    let trimmed = line.trim_end_matches('\r');
                                    if trimmed.len() == line.len() {
                                        std::borrow::Cow::Owned(line.to_string())
                                    } else {
                                        std::borrow::Cow::Owned(trimmed.to_string())
                                    }
                                })
                                .collect()
                        }
                    }
                }
            }
            BlockSource::Fragmented(ranges) => {
                ranges.iter().map(|r| doc.doc_text_in_range(r.clone())).collect()
            }
        }
    }

    /// Number of logical source lines in this block.
    /// For Markdown blocks this equals `text_lines.len()`; for Novel zero-copy
    /// blocks `text_lines` is empty and the count comes from `text_styles.len()`.
    pub fn line_count(&self) -> usize {
        if matches!(self.kind, BlockKind::HorizontalRule) {
            return 1;
        }
        if !self.text_lines.is_empty() { self.text_lines.len() } else { self.text_styles.len() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    /// Root container (implicit top-level).
    Container,
    Heading {
        level: u8,
    },
    Paragraph,
    CodeBlock {
        language: Option<String>,
    },
    BlockQuote,
    ListItem {
        bullet: ListBullet,
        tight: bool,
        blank_line_before: bool,
    },
    TableWrapper {
        columns: usize,
        alignments: Vec<Alignment>,
    },
    /// Internal row container — only used inside TableWrapper; trailing underscore marks it as non-leaf.
    TableRow_,
    /// Internal cell container — children hold the cell's text blocks; trailing underscore marks it as non-leaf.
    TableCell_ {
        col: usize,
        row: usize,
        is_header: bool,
    },
    HorizontalRule,
    MetadataBlock,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListBullet {
    Bullet,
    Ordered(u64),
    TaskList(bool),
}

// ===== 文本样式 =====

/// A styled span within a line of text.
#[derive(Clone, Debug)]
pub struct StyleSpan {
    pub start: usize,
    pub len: usize,
    pub style: InlineStyle,
    /// 此 span 在原始源码中的字节范围 (含标记符, e.g. "**world**" → 6..15)
    pub source_range: Range<usize>,
}

/// Simplified inline style for rendering (no block-level styles).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineStyle {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link { url: String },
    SourceMarker,
}

/// A highlighted span within a code line.
/// `start` and `len` are **byte offsets** (not char indices),
/// matching the LSH runtime's offset convention and Rust's UTF-8 layout.
#[derive(Clone, Debug, PartialEq)]
pub struct HighlightSpan {
    pub start: usize,
    pub len: usize,
    pub color: [f32; 4],
}

/// Injected by the host application. Stateless — called once per code block.
pub trait CodeHighlighter {
    /// Highlight an entire code block. Returns per-line spans.
    fn highlight(&self, language: &str, code: &str) -> Vec<Vec<HighlightSpan>>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextStyleMod {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link { url: String },
    Heading { level: u8 },
    BlockQuote,
    CodeBlock,
}

// ===== 构建产物 =====

#[derive(Clone, Debug)]
pub struct MarkdownDoc {
    pub blocks: Vec<BlockNode>,
}

impl crate::layout::BlockSource for MarkdownDoc {
    fn blocks(&self) -> &[BlockNode] {
        &self.blocks
    }

    fn headings(&self) -> &[crate::layout::HeadingEntry] {
        &[]
    }
}

// ===== NovelStructure — 轻量级小说结构扫描器 =====

/// 行分类结果。仅记录元数据，不分配 String/Vec 用于块内容。
#[derive(Clone, Debug)]
struct LineMeta {
    kind: LineKind,
    /// 不含尾部换行的字节范围
    byte_range: std::ops::Range<usize>,
    /// 本 section 占几行（正文段落可能多行）
    line_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum LineKind {
    Empty,
    Heading { level: u8 },
    Body,
}

/// 轻量级小说结构 — 由 scan() 生成，实现 BlockSource。
/// 记录分类元数据 (LineMeta)，同时产生轻量 BlockNode（无 String/Vec 分配）。
#[derive(Default)]
pub struct NovelStructure {
    /// 分类元数据（Phase 4 视口驱动用）
    #[allow(dead_code)]
    sections: Vec<LineMeta>,
    /// 轻量 BlockNode 树（text_lines/text_styles/children 为空切片）
    blocks: Vec<BlockNode>,
    /// 从分类中直接提取的标题
    headings: Vec<crate::layout::HeadingEntry>,
}

impl NovelStructure {
    /// 扫描小说文档，产生 NovelStructure。
    /// 分类逻辑与 build_from_novel_doc 完全一致。
    pub fn scan(doc: &dyn core::document::DocView) -> Self {
        let mut sections: Vec<LineMeta> = Vec::new();
        let mut blocks = Vec::new();
        let mut headings = Vec::new();
        let total_lines = doc.line_count();
        let mut para_line_count: usize = 0;
        let mut para_start_byte: usize = 0;

        for i in 0..total_lines {
            let text = doc.doc_line_text(i);
            let trimmed = text.trim();
            let line_start = doc.line_byte_offset(i);
            // 当前行（含行尾换行符）之后的第一个字节位置
            let line_end = if i + 1 < total_lines {
                doc.line_byte_offset(i + 1)
            } else {
                line_start + doc.line_byte_length(i)
            };

            if trimmed.is_empty() {
                // 空行：结束当前段落
                if para_line_count > 0 {
                    let byte_range = para_start_byte..line_start;
                    let text_styles = vec![vec![]; para_line_count];
                    sections.push(LineMeta {
                        kind: LineKind::Body,
                        byte_range: byte_range.clone(),
                        line_count: para_line_count,
                    });
                    blocks.push(BlockNode {
                        kind: BlockKind::Paragraph,
                        children: vec![],
                        text_lines: vec![],
                        projected_lines: vec![],
                        text_styles,
                        source_range: BlockSource::Continuous(byte_range),
                        code_line_source_starts: None,
                        block_range: para_start_byte..line_start,
                    });
                    para_line_count = 0;
                }
                sections.push(LineMeta {
                    kind: LineKind::Empty,
                    byte_range: line_start..line_start,
                    line_count: 0,
                });
                para_start_byte = line_end;
                continue;
            }

            if let Some(heading_level) = classify_title(trimmed) {
                // 结束当前段落
                if para_line_count > 0 {
                    let byte_range = para_start_byte..line_start;
                    let text_styles = vec![vec![]; para_line_count];
                    sections.push(LineMeta {
                        kind: LineKind::Body,
                        byte_range: byte_range.clone(),
                        line_count: para_line_count,
                    });
                    blocks.push(BlockNode {
                        kind: BlockKind::Paragraph,
                        children: vec![],
                        text_lines: vec![],
                        projected_lines: vec![],
                        text_styles,
                        source_range: BlockSource::Continuous(byte_range),
                        code_line_source_starts: None,
                        block_range: para_start_byte..line_start,
                    });
                    para_line_count = 0;
                }

                let content_len = text.len();
                let byte_range = line_start..(line_start + content_len);
                sections.push(LineMeta {
                    kind: LineKind::Heading { level: heading_level },
                    byte_range: byte_range.clone(),
                    line_count: 1,
                });
                blocks.push(BlockNode {
                    kind: BlockKind::Heading { level: heading_level },
                    children: vec![],
                    text_lines: vec![],
                    projected_lines: vec![],
                    text_styles: vec![vec![]],
                    source_range: BlockSource::Continuous(byte_range),
                    code_line_source_starts: None,
                    block_range: line_start..(line_start + content_len),
                });
                headings.push(crate::layout::HeadingEntry {
                    text: trimmed.to_string(),
                    level: heading_level,
                    y_offset: 0.0, // 由布局阶段填充
                });
                para_start_byte = line_end;
                continue;
            }

            let last_char = trimmed.chars().last().unwrap_or(' ');
            para_line_count += 1;

            if is_paragraph_end_char(last_char) {
                let byte_range = para_start_byte..line_end;
                let text_styles = vec![vec![]; para_line_count];
                sections.push(LineMeta {
                    kind: LineKind::Body,
                    byte_range: byte_range.clone(),
                    line_count: para_line_count,
                });
                blocks.push(BlockNode {
                    kind: BlockKind::Paragraph,
                    children: vec![],
                    text_lines: vec![],
                    projected_lines: vec![],
                    text_styles,
                    source_range: BlockSource::Continuous(byte_range),
                    code_line_source_starts: None,
                    block_range: para_start_byte..line_end,
                });
                para_line_count = 0;
                para_start_byte = line_end;
            }
        }

        // 收尾未完成的段落
        if para_line_count > 0 {
            let last_line_end = if total_lines > 0 {
                let last = total_lines - 1;
                doc.line_byte_offset(last) + doc.line_byte_length(last)
            } else {
                0
            };
            let byte_range = para_start_byte..last_line_end;
            let text_styles = vec![vec![]; para_line_count];
            sections.push(LineMeta {
                kind: LineKind::Body,
                byte_range: byte_range.clone(),
                line_count: para_line_count,
            });
            blocks.push(BlockNode {
                kind: BlockKind::Paragraph,
                children: vec![],
                text_lines: vec![],
                projected_lines: vec![],
                text_styles,
                source_range: BlockSource::Continuous(byte_range),
                code_line_source_starts: None,
                block_range: para_start_byte..last_line_end,
            });
        }

        Self { sections, blocks, headings }
    }
}

impl Clone for NovelStructure {
    fn clone(&self) -> Self {
        Self {
            sections: self.sections.clone(),
            blocks: self.blocks.clone(),
            headings: self.headings.clone(),
        }
    }
}

impl std::fmt::Debug for NovelStructure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NovelStructure")
            .field("sections", &self.sections.len())
            .field("blocks", &self.blocks.len())
            .field("headings", &self.headings.len())
            .finish()
    }
}

impl crate::layout::BlockSource for NovelStructure {
    fn blocks(&self) -> &[BlockNode] {
        &self.blocks
    }

    fn headings(&self) -> &[crate::layout::HeadingEntry] {
        &self.headings
    }
}

// ===== Table 辅助 =====

#[derive(Default)]
struct TableState {
    alignments: Vec<Alignment>,
    in_head: bool,
    row_index: usize,
    col_index: usize,
}

impl TableState {
    fn start(&mut self, alignments: Vec<Alignment>) {
        self.alignments = alignments;
        self.in_head = false;
        self.row_index = 0;
        self.col_index = 0;
    }

    fn end(&mut self) {
        self.alignments.clear();
        self.in_head = false;
        self.row_index = 0;
        self.col_index = 0;
    }

    fn start_head(&mut self) {
        self.in_head = true;
    }
    fn end_head(&mut self) {
        self.in_head = false;
    }
    fn start_row(&mut self) {
        self.col_index = 0;
    }
    fn end_row(&mut self) {
        self.row_index += 1;
    }
    fn end_cell(&mut self) {
        self.col_index += 1;
    }
}

// ===== PendingLine =====

#[derive(Default)]
struct PendingLine {
    text: String,
    /// Style spans accumulated so far in this line.
    styles: Vec<StyleSpan>,
    projection: TextProjectionBuilder,
}

// ===== ListStackEntry =====

struct ListStackEntry {
    bullet_index: Option<u64>,
    tight: bool,
    blank_line_before: bool,
}

// ===== MarkdownBuilder =====

struct MarkdownBuilder {
    block_stack: Vec<BlockNode>,
    pending_line: PendingLine,
    text_style_stack: Vec<TextStyleMod>,
    code_block_depth: usize,
    link_depth: usize,
    list_stack: Vec<ListStackEntry>,
    table: TableState,
    /// 当前事件在源码中的字节范围，由 build 循环每轮更新。
    current_event_range: Range<usize>,
    /// 每层 block 的起始源码偏移，用于 pop_block 时计算 source_range。
    block_source_starts: Vec<usize>,
    /// 每层 inline style 的起始源码偏移，用于 push_text 时计算 StyleSpan::source_range。
    inline_start_offsets: Vec<usize>,
}

impl MarkdownBuilder {
    fn new(_style: &MarkdownStyle) -> Self {
        Self {
            block_stack: vec![BlockNode {
                kind: BlockKind::Container,
                children: vec![],
                text_lines: vec![],
                projected_lines: vec![],
                text_styles: vec![],
                source_range: BlockSource::Continuous(0..0),
                code_line_source_starts: None,
                block_range: 0..0,
            }],
            pending_line: PendingLine::default(),
            text_style_stack: vec![],
            code_block_depth: 0,
            link_depth: 0,
            list_stack: vec![],
            table: TableState::default(),
            current_event_range: 0..0,
            block_source_starts: vec![0],
            inline_start_offsets: vec![],
        }
    }

    fn push_text_style(&mut self, modifier: TextStyleMod) {
        if modifier_to_inline(&modifier).is_some() {
            self.inline_start_offsets.push(self.current_event_range.start);
        }
        self.text_style_stack.push(modifier);
    }

    fn pop_text_style(&mut self) {
        if let Some(top) = self.text_style_stack.last()
            && modifier_to_inline(top).is_some()
        {
            self.finalize_inline_source_range(self.current_event_range.end);
            self.inline_start_offsets.pop();
        }
        self.text_style_stack.pop();
    }

    /// 更新当前行最后一个 inline span 的 source_range.end，使其包含闭合标记。
    fn finalize_inline_source_range(&mut self, end: usize) {
        if let Some(&start) = self.inline_start_offsets.last()
            && let Some(span) = self.pending_line.styles.last_mut()
            && span.source_range.start == start
        {
            span.source_range.end = end;
        }
    }

    fn push_block(&mut self, kind: BlockKind) {
        self.flush_line();
        self.block_source_starts.push(self.current_event_range.start);
        let node = BlockNode {
            kind,
            children: vec![],
            text_styles: vec![],
            text_lines: vec![],
            projected_lines: vec![],
            source_range: BlockSource::Continuous(
                self.current_event_range.start..self.current_event_range.start,
            ),
            code_line_source_starts: None,
            block_range: self.current_event_range.start..self.current_event_range.start,
        };
        self.block_stack.push(node);
    }

    fn pop_block(&mut self) {
        let (flushed_text, flushed_styles, flushed_projection) = self.flush_line_to_vec();
        let block_end = self.current_event_range.end;
        if let Some(mut node) = self.block_stack.pop() {
            if !flushed_text.is_empty() {
                node.text_lines.extend(flushed_text);
            }
            if !flushed_styles.is_empty() {
                node.text_styles.extend(flushed_styles);
            }
            if !flushed_projection.is_empty() {
                node.projected_lines.extend(flushed_projection);
            }
            if node.text_lines.is_empty()
                && node.text_styles.is_empty()
                && needs_empty_text_line(&node)
            {
                node.text_lines.push(String::new());
                node.text_styles.push(Vec::new());
                node.projected_lines.push(TextProjectionBuilder::default().finish(block_end));
            }
            if let Some(&block_start) = self.block_source_starts.last() {
                node.source_range = BlockSource::Continuous(block_start..block_end);
                node.block_range = block_start..block_end;
            }
            self.block_source_starts.pop();
            if let Some(parent) = self.block_stack.last_mut() {
                parent.children.push(node);
            }
        }
    }

    fn push_text(&mut self, text: &str) {
        self.push_text_with_source(text, self.current_event_range.clone());
    }

    fn push_text_with_source(&mut self, text: &str, source_range: Range<usize>) {
        self.append_text_and_style(text);
        self.pending_line.projection.push_direct(text, source_range);
    }

    fn append_text_and_style(&mut self, text: &str) {
        let start = self.pending_line.text.len();
        let len = text.len();
        self.pending_line.text.push_str(text);
        // Record only the most specific inline style (top of stack).
        // Avoids creating overlapping spans for nested modifiers.
        if let Some(top_inline) = self.text_style_stack.iter().rev().find_map(modifier_to_inline) {
            let source_range = if let Some(&inline_start) = self.inline_start_offsets.last() {
                inline_start..self.current_event_range.end
            } else {
                self.current_event_range.clone()
            };
            self.pending_line.styles.push(StyleSpan {
                start,
                len,
                style: top_inline,
                source_range: source_range.clone(),
            });
        }
    }

    fn push_soft_break(&mut self, source_range: Range<usize>) {
        self.append_text_and_style(" ");
        self.pending_line.projection.push_soft_break(source_range);
    }

    fn is_inside_blockquote(&self) -> bool {
        self.block_stack.iter().any(|block| matches!(block.kind, BlockKind::BlockQuote))
    }

    /// Flush pending line, returning text, styles, and source projection.
    fn flush_line_to_vec(&mut self) -> (Vec<String>, Vec<Vec<StyleSpan>>, Vec<ProjectedText>) {
        let line = std::mem::take(&mut self.pending_line);
        if line.text.is_empty() {
            return (vec![], vec![], vec![]);
        }
        let merged = merge_style_spans(line.styles);
        let projection = line.projection.finish(self.current_event_range.end);
        (vec![line.text], vec![merged], vec![projection])
    }

    fn flush_line(&mut self) {
        self.flush_line_to_vec();
    }

    fn flush_line_into_current_block(&mut self) {
        let (text_lines, text_styles, projected_lines) = self.flush_line_to_vec();
        if text_lines.is_empty() {
            return;
        }

        if let Some(block) = self.block_stack.last_mut() {
            block.text_lines.extend(text_lines);
            block.text_styles.extend(text_styles);
            block.projected_lines.extend(projected_lines);
        }
    }

    fn push_list(&mut self, start: Option<u64>, tight: bool, blank_line_before: bool) {
        self.list_stack.push(ListStackEntry { bullet_index: start, tight, blank_line_before });
    }

    fn pop_list(&mut self) {
        self.list_stack.pop();
    }

    fn next_bullet_index(&mut self) -> Option<u64> {
        self.list_stack.last_mut().and_then(|entry| {
            let idx = entry.bullet_index.as_mut()?;
            let current = *idx;
            *idx += 1;
            Some(current)
        })
    }

    fn trim_trailing_newline(&mut self) {
        if self.pending_line.text.ends_with('\n') {
            let new_len = self.pending_line.text.len() - 1;
            self.pending_line.text.truncate(new_len);
            self.pending_line.projection.trim_trailing_newline();
        }
    }

    fn build(mut self) -> MarkdownDoc {
        self.flush_line();
        MarkdownDoc { blocks: std::mem::take(&mut self.block_stack.swap_remove(0).children) }
    }
}

fn needs_empty_text_line(node: &BlockNode) -> bool {
    match node.kind {
        BlockKind::Heading { .. } => true,
        BlockKind::ListItem { .. } => node.children.is_empty(),
        _ => false,
    }
}

// ===== 公开入口 =====

impl MarkdownDoc {
    /// Build a MarkdownDoc from parsed events + style configuration.
    pub fn build(parsed: &ParsedMarkdown, style: &MarkdownStyle) -> Self {
        let mut builder = MarkdownBuilder::new(style);

        for (event_idx, event) in parsed.events.iter().enumerate() {
            if event_idx < parsed.event_ranges.len() {
                builder.current_event_range = parsed.event_ranges[event_idx].clone();
            }
            match event {
                // ---- Block-level Start ----
                MarkdownEvent::Start(tag) => match tag {
                    MarkdownTag::Paragraph => {
                        builder.push_block(BlockKind::Paragraph);
                    }
                    MarkdownTag::Heading { level } => {
                        builder.push_text_style(TextStyleMod::Heading { level: *level as u8 });
                        builder.push_block(BlockKind::Heading { level: *level as u8 });
                    }
                    MarkdownTag::BlockQuote => {
                        builder.push_text_style(TextStyleMod::BlockQuote);
                        builder.push_block(BlockKind::BlockQuote);
                    }
                    MarkdownTag::CodeBlock { info } => {
                        builder.code_block_depth += 1;
                        builder.push_text_style(TextStyleMod::CodeBlock);
                        builder.push_block(BlockKind::CodeBlock { language: info.clone() });
                    }
                    MarkdownTag::List(start, tight, blank_line_before) => {
                        builder.flush_line_into_current_block();
                        builder.push_list(*start, *tight, *blank_line_before);
                    }
                    MarkdownTag::Item => {
                        let bullet = if let Some(idx) = builder.next_bullet_index() {
                            ListBullet::Ordered(idx)
                        } else {
                            ListBullet::Bullet
                        };
                        let is_tight = builder.list_stack.last().is_none_or(|e| e.tight);
                        // blank_line_before only applies to the first item in the group
                        let has_blank = !builder
                            .block_stack
                            .last()
                            .is_some_and(|b| matches!(b.kind, BlockKind::ListItem { .. }))
                            && builder.list_stack.last().is_some_and(|e| e.blank_line_before);
                        builder.push_block(BlockKind::ListItem {
                            bullet,
                            tight: is_tight,
                            blank_line_before: has_blank,
                        });
                    }
                    MarkdownTag::Emphasis => {
                        builder.push_text_style(TextStyleMod::Italic);
                    }
                    MarkdownTag::Strong => {
                        builder.push_text_style(TextStyleMod::Bold);
                    }
                    MarkdownTag::Strikethrough => {
                        builder.push_text_style(TextStyleMod::Strikethrough);
                    }
                    MarkdownTag::MetadataBlock(_) => {
                        builder.code_block_depth += 1;
                        builder.push_block(BlockKind::MetadataBlock);
                    }
                    MarkdownTag::Link { url, .. } => {
                        builder.link_depth += 1;
                        builder.push_text_style(TextStyleMod::Link { url: url.clone() });
                    }
                    MarkdownTag::Table(alignments) => {
                        builder.table.start(alignments.clone());
                        builder.push_block(BlockKind::TableWrapper {
                            columns: alignments.len(),
                            alignments: alignments.clone(),
                        });
                    }
                    MarkdownTag::TableHead => {
                        builder.table.start_head();
                    }
                    MarkdownTag::TableRow => {
                        builder.table.start_row();
                        builder.push_block(BlockKind::TableRow_);
                    }
                    MarkdownTag::TableCell => {
                        let col = builder.table.col_index;
                        let row = builder.table.row_index;
                        let is_header = builder.table.in_head;
                        builder.push_block(BlockKind::TableCell_ { col, row, is_header });
                    }
                    MarkdownTag::Image { url, title: _ } => {
                        builder.link_depth += 1;
                        builder.push_text(&format!("[Image: {}]", url));
                    }
                },

                // ---- Block-level End ----
                MarkdownEvent::End(tag_end) => match tag_end {
                    MarkdownTagEnd::Paragraph => builder.pop_block(),
                    MarkdownTagEnd::Heading => {
                        builder.pop_block();
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::BlockQuote => {
                        builder.pop_block();
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::CodeBlock => {
                        builder.trim_trailing_newline();
                        builder.pop_block();
                        builder.code_block_depth = builder.code_block_depth.saturating_sub(1);
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::List => {
                        builder.pop_list();
                    }
                    MarkdownTagEnd::Item => builder.pop_block(),
                    MarkdownTagEnd::Emphasis => builder.pop_text_style(),
                    MarkdownTagEnd::Strong => builder.pop_text_style(),
                    MarkdownTagEnd::Strikethrough => builder.pop_text_style(),
                    MarkdownTagEnd::MetadataBlock(_) => {
                        builder.trim_trailing_newline();
                        builder.pop_block();
                        builder.code_block_depth = builder.code_block_depth.saturating_sub(1);
                    }
                    MarkdownTagEnd::Link => {
                        builder.link_depth = builder.link_depth.saturating_sub(1);
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::Image => {
                        builder.link_depth = builder.link_depth.saturating_sub(1);
                    }
                    MarkdownTagEnd::Table => {
                        builder.pop_block();
                        builder.table.end();
                    }
                    MarkdownTagEnd::TableHead => builder.table.end_head(),
                    MarkdownTagEnd::TableRow => {
                        builder.pop_block();
                        builder.table.end_row();
                    }
                    MarkdownTagEnd::TableCell => {
                        builder.pop_block();
                        builder.table.end_cell();
                    }
                },

                // ---- Inline ----
                MarkdownEvent::Text(text) => {
                    let text_start = builder.current_event_range.start;
                    builder.push_text_with_source(text, builder.current_event_range.clone());
                    if builder.code_block_depth > 0
                        && let Some(block) = builder.block_stack.last_mut()
                        && matches!(block.kind, BlockKind::CodeBlock { .. })
                    {
                        let mut starts = block.code_line_source_starts.take().unwrap_or_default();
                        if starts.is_empty() {
                            starts.push(text_start);
                        }
                        for (i, b) in text.bytes().enumerate() {
                            if b == b'\n' {
                                starts.push(text_start + i + 1);
                            }
                        }
                        block.code_line_source_starts = Some(starts);
                    }
                }
                MarkdownEvent::Code(code) => {
                    // InlineCode 是原子事件 (无 Start/End)，手动管理 source_range。
                    let code_source = builder.current_event_range.clone();
                    builder.inline_start_offsets.push(code_source.start);
                    builder.push_text_style(TextStyleMod::InlineCode);
                    builder.push_text(code);
                    builder.finalize_inline_source_range(code_source.end);
                    builder.inline_start_offsets.pop();
                    builder.text_style_stack.pop();
                }
                MarkdownEvent::InlineHtml(html) => {
                    if inline_html_is_break(html) {
                        builder.flush_line_into_current_block();
                    } else {
                        // Other inline HTML remains literal text until the renderer has a
                        // dedicated, sanitized HTML representation.
                        builder.push_text(html);
                    }
                }
                MarkdownEvent::SoftBreak => {
                    builder.push_soft_break(builder.current_event_range.clone());
                }
                MarkdownEvent::HardBreak => {
                    builder.flush_line_into_current_block();
                }
                MarkdownEvent::Rule => {
                    builder.push_block(BlockKind::HorizontalRule);
                    builder.pop_block();
                }
                MarkdownEvent::TaskListMarker(checked) => {
                    // Update the last ListItem's bullet to TaskList
                    if let Some(last) = builder.block_stack.last_mut()
                        && let BlockKind::ListItem { tight, blank_line_before, .. } = last.kind
                    {
                        last.kind = BlockKind::ListItem {
                            bullet: ListBullet::TaskList(*checked),
                            tight,
                            blank_line_before,
                        };
                    }
                }
            }
        }

        builder.build()
    }

    /// Build a MarkdownDoc directly from a DocView, optimized for Novel Mode.
    /// This avoids creating a giant intermediate String and minimizes regex overhead.
    pub fn build_from_novel_doc(doc: &dyn core::document::DocView) -> Self {
        let mut blocks = Vec::new();
        let total_lines = doc.line_count();
        let mut para_lines = Vec::new();
        let mut para_start_byte = 0;

        for i in 0..total_lines {
            let text = doc.doc_line_text(i);
            let trimmed = text.trim();
            let line_start = doc.line_byte_offset(i);
            let line_end = if i + 1 < total_lines {
                doc.line_byte_offset(i + 1)
            } else {
                line_start + doc.line_byte_length(i)
            };

            if trimmed.is_empty() {
                if !para_lines.is_empty() {
                    let text_styles = para_lines.iter().map(|_| vec![]).collect();
                    blocks.push(BlockNode {
                        kind: BlockKind::Paragraph,
                        children: vec![],
                        text_lines: vec![],
                        projected_lines: vec![],
                        text_styles,
                        source_range: BlockSource::Continuous(para_start_byte..line_start),
                        code_line_source_starts: None,
                        block_range: para_start_byte..line_start,
                    });
                    para_lines.clear();
                }
                para_start_byte = line_end;
                continue;
            }

            if let Some(heading_level) = classify_title(trimmed) {
                if !para_lines.is_empty() {
                    let text_styles = para_lines.iter().map(|_| vec![]).collect();
                    blocks.push(BlockNode {
                        kind: BlockKind::Paragraph,
                        children: vec![],
                        text_lines: vec![],
                        projected_lines: vec![],
                        text_styles,
                        source_range: BlockSource::Continuous(para_start_byte..line_start),
                        code_line_source_starts: None,
                        block_range: para_start_byte..line_start,
                    });
                    para_lines.clear();
                }

                let content_len = text.len();
                blocks.push(BlockNode {
                    kind: BlockKind::Heading { level: heading_level },
                    children: vec![],
                    text_lines: vec![],
                    projected_lines: vec![],
                    text_styles: vec![vec![]],
                    source_range: BlockSource::Continuous(line_start..(line_start + content_len)),
                    code_line_source_starts: None,
                    block_range: line_start..(line_start + content_len),
                });
                para_start_byte = line_end;
                continue;
            }

            let last_char = trimmed.chars().last().unwrap_or(' ');
            para_lines.push(1); // just a placeholder to count lines

            if is_paragraph_end_char(last_char) {
                let text_styles = para_lines.iter().map(|_| vec![]).collect();
                blocks.push(BlockNode {
                    kind: BlockKind::Paragraph,
                    children: vec![],
                    text_lines: vec![],
                    projected_lines: vec![],
                    text_styles,
                    source_range: BlockSource::Continuous(para_start_byte..line_end),
                    code_line_source_starts: None,
                    block_range: para_start_byte..line_end,
                });
                para_lines.clear();
                para_start_byte = line_end;
            }
        }

        if !para_lines.is_empty() {
            let last_line_end = if total_lines > 0 {
                let last = total_lines - 1;
                doc.line_byte_offset(last) + doc.line_byte_length(last)
            } else {
                0
            };
            let text_styles = para_lines.iter().map(|_| vec![]).collect();
            blocks.push(BlockNode {
                kind: BlockKind::Paragraph,
                children: vec![],
                text_lines: vec![],
                projected_lines: vec![],
                text_styles,
                source_range: BlockSource::Continuous(para_start_byte..last_line_end),
                code_line_source_starts: None,
                block_range: para_start_byte..last_line_end,
            });
        }

        Self { blocks }
    }
}

fn inline_html_is_break(html: &str) -> bool {
    matches!(html.trim().to_ascii_lowercase().as_str(), "<br>" | "<br/>" | "<br />")
}

/// Convert TextStyleMod to InlineStyle (if applicable for rendering).
fn modifier_to_inline(m: &TextStyleMod) -> Option<InlineStyle> {
    match m {
        TextStyleMod::Bold => Some(InlineStyle::Bold),
        TextStyleMod::Italic => Some(InlineStyle::Italic),
        TextStyleMod::Strikethrough => Some(InlineStyle::Strikethrough),
        TextStyleMod::InlineCode => Some(InlineStyle::InlineCode),
        TextStyleMod::Link { url } => Some(InlineStyle::Link { url: url.clone() }),
        // Block-level styles don't produce inline runs
        TextStyleMod::Heading { .. } | TextStyleMod::BlockQuote | TextStyleMod::CodeBlock => None,
    }
}

/// Merge overlapping/adjacent style spans. Higher-priority styles win on overlap.
fn merge_style_spans(mut spans: Vec<StyleSpan>) -> Vec<StyleSpan> {
    if spans.is_empty() {
        return spans;
    }
    // Sort by start position
    spans.sort_by_key(|s| s.start);
    // Merge adjacent spans with identical styles
    let mut merged: Vec<StyleSpan> = Vec::new();
    for span in spans {
        if let Some(last) = merged.last_mut() {
            let last_end = last.start + last.len;
            if last.start + last.len >= span.start && last.style == span.style {
                // Extend last span
                let new_end = (span.start + span.len).max(last_end);
                last.len = new_end - last.start;
                // Merge source ranges
                last.source_range.end = last.source_range.end.max(span.source_range.end);
                continue;
            }
        }
        merged.push(span);
    }
    merged
}

// ===== Novel chapter detection =====

/// Maximum non-CJK character ratio for body text filtering.
const MAX_NON_CJK_RATIO: f32 = 0.4;
/// Maximum title length in characters.
const MAX_TITLE_LENGTH: usize = 120;
// Unicode CJK block ranges.
const CJK_UNIFIED_START: char = '\u{4E00}';
const CJK_UNIFIED_END: char = '\u{9FFF}';
const CJK_EXT_A_START: char = '\u{3400}';
const CJK_EXT_A_END: char = '\u{4DBF}';
const CJK_COMPAT_START: char = '\u{F900}';
const CJK_COMPAT_END: char = '\u{FAFF}';
const CJK_PUNCT_START: char = '\u{3000}';
const CJK_PUNCT_END: char = '\u{303F}';
const FULLWIDTH_START: char = '\u{FF00}';
const FULLWIDTH_END: char = '\u{FFEF}';

/// Special chapter markers that are always recognized as headings.
const SPECIAL_MARKERS: &[&str] =
    &["序章", "序幕", "楔子", "尾声", "终章", "番外", "引子", "后记", "前言"];

fn book_title_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^《.+》(\s+作者[：:].+)?$").expect("invalid book title regex")
    });
    &RE
}

fn volume_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^第[零一二三四五六七八九十百千万亿0-9]+[卷集部篇](\s+.*)?$")
            .expect("invalid volume regex")
    });
    &RE
}

fn chapter_re() -> &'static Regex {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^(第[零一二三四五六七八九十百千万亿0-9]+[章节回](\s+.*)?|Chapter\s+\d+(\s+.*)?|CHAPTER\s+\d+(\s+.*)?)$",
        )
        .expect("invalid chapter regex")
    });
    &RE
}

/// Classify a line as a chapter/section heading. Returns heading level (1-3) or None.
fn classify_title(text: &str) -> Option<u8> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_TITLE_LENGTH {
        return None;
    }

    // Fast path: Only run expensive checks if the string starts with common title prefixes
    // or looks like a book title (starts with 《).
    let first_char = trimmed.chars().next()?;
    let is_potential_title = first_char == '第'
        || first_char == '《'
        || trimmed.starts_with("Chapter")
        || trimmed.starts_with("CHAPTER")
        || SPECIAL_MARKERS.iter().any(|m| trimmed.starts_with(m));

    if !is_potential_title {
        return None;
    }

    // Special markers → level 2 (chapter)
    for marker in SPECIAL_MARKERS {
        if trimmed == *marker || trimmed.starts_with(&format!("{marker} ")) {
            return Some(2);
        }
    }
    // Book title → level 1
    if book_title_re().is_match(trimmed) {
        return Some(1);
    }
    // Filter body text by punctuation ratio
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let punct_count = chars.iter().filter(|c| is_cjk_punctuation(**c)).count();
    if punct_count as f32 / chars.len() as f32 > 0.2 {
        return None;
    }
    if !trimmed.chars().next().is_some_and(|c| is_cjk_char(c) || c.is_ascii_uppercase()) {
        return None;
    }
    let has_cjk = chars.iter().any(|c| is_cjk_char(*c));
    if has_cjk {
        let non_cjk = chars
            .iter()
            .filter(|c| !is_cjk_char(**c) && !c.is_ascii_digit() && !c.is_whitespace())
            .count();
        if chars.len() > 2 && non_cjk as f32 / chars.len() as f32 > MAX_NON_CJK_RATIO {
            return None;
        }
    }
    // Volume → level 1, Chapter → level 2
    if volume_re().is_match(trimmed) {
        return Some(1);
    }
    if chapter_re().is_match(trimmed) {
        return Some(2);
    }
    None
}

fn is_cjk_char(c: char) -> bool {
    matches!(
        c,
        CJK_UNIFIED_START..=CJK_UNIFIED_END
            | CJK_EXT_A_START..=CJK_EXT_A_END
            | CJK_COMPAT_START..=CJK_COMPAT_END
    )
}

fn is_cjk_punctuation(c: char) -> bool {
    matches!(
        c,
        '。' | '，' | '、' | '；' | '：' | '？' | '！'
            | '「' | '」' | '『' | '』' | '（' | '）' | '《' | '》'
            | '…' | '—' | '～' | '・' | '．'
            | CJK_PUNCT_START..=CJK_PUNCT_END | FULLWIDTH_START..=FULLWIDTH_END
    )
}

fn is_paragraph_end_char(c: char) -> bool {
    matches!(
        c,
        '。' | '！'
            | '？'
            | '”'
            | '」'
            | '』'
            | '…'
            | '—'
            | '～'
            | '.'
            | '!'
            | '?'
            | '"'
            | '\''
            | '’'
            | ']'
            | ')'
            | '）'
            | '】'
            | '》'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::BlockSource;
    use crate::parser::parse_markdown;

    use crate::test_utils::default_style;

    #[test]
    fn build_simple_paragraph() {
        let parsed = parse_markdown("hello world");
        let doc = MarkdownDoc::build(&parsed, &default_style());
        // Should have a Paragraph block with text
        let has_paragraph = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Paragraph));
        assert!(has_paragraph, "should have a Paragraph block");
        // Check that text content is stored
        let string_doc = core::document::StringDocView::new("hello world");
        let text: Vec<std::borrow::Cow<'_, str>> =
            doc.blocks.iter().flat_map(|b| b.lines(&string_doc)).collect();
        assert!(
            text.iter().any(|t| t.contains("hello world")),
            "text 'hello world' should be extracted via doc"
        );
    }

    #[test]
    fn build_heading() {
        let parsed = parse_markdown("# Hello");
        let doc = MarkdownDoc::build(&parsed, &default_style());
        assert!(!doc.blocks.is_empty());
        assert!(matches!(doc.blocks[0].kind, BlockKind::Heading { level: 1 }));
    }

    #[test]
    fn build_bold_text() {
        let parsed = parse_markdown("**bold text**");
        let doc = MarkdownDoc::build(&parsed, &default_style());
        // Should have a paragraph with bold text content
        let has_paragraph = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Paragraph));
        assert!(has_paragraph, "should have a Paragraph block for bold text");
    }

    #[test]
    fn build_code_block() {
        let parsed = parse_markdown("```rust\nlet x = 1;\n```");
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let has_code = doc.blocks.iter().any(
            |b| matches!(b.kind, BlockKind::CodeBlock { language: Some(ref l) } if l == "rust"),
        );
        assert!(has_code);
    }

    #[test]
    fn build_unordered_list() {
        let parsed = parse_markdown("- one\n- two\n- three");
        let doc = MarkdownDoc::build(&parsed, &default_style());
        // Should have at least 3 ListItem blocks (nested under Container)
        fn count_items(blocks: &[BlockNode]) -> usize {
            let mut count = 0;
            for b in blocks {
                if matches!(b.kind, BlockKind::ListItem { .. }) {
                    count += 1;
                }
                count += count_items(&b.children);
            }
            count
        }
        assert_eq!(count_items(&doc.blocks), 3);
    }

    #[test]
    fn build_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let parsed = parse_markdown(md);
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let has_table = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::TableWrapper { .. }));
        assert!(has_table);
    }

    #[test]
    fn build_link() {
        let parsed = parse_markdown("[example](https://example.com)");
        let doc = MarkdownDoc::build(&parsed, &default_style());
        // Should have a paragraph block with link text
        let has_paragraph = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Paragraph));
        assert!(has_paragraph, "should have a Paragraph block for link");
    }

    #[test]
    fn build_blockquote() {
        let parsed = parse_markdown("> quoted text");
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let has_bq = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::BlockQuote));
        assert!(has_bq);
    }

    #[test]
    fn builder_collapses_explicit_blockquote_softbreak_and_preserves_source_jump() {
        let source = "> first\n> second";
        let parsed = crate::parser::parse_markdown(source);
        let style = crate::test_utils::default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let paragraph = &doc.blocks[0].children[0];
        assert_eq!(paragraph.projected_lines.len(), 1);
        assert_eq!(paragraph.projected_lines[0].text, "first second");
        assert_eq!(
            paragraph.projected_lines[0].boundaries.last().expect("collapsed line boundary").byte,
            source.len()
        );
        assert!(paragraph.projected_lines[0].boundaries.iter().any(|boundary| {
            boundary.byte == source.find("second").expect("fixture must contain second")
        }));
        assert!(paragraph.projected_lines[0].boundaries.iter().any(|boundary| {
            boundary.byte == source.find('\n').expect("fixture must contain newline")
        }));
        assert_eq!(
            paragraph.projected_lines[0].source_extent().end,
            source.find("second").expect("fixture must contain second") + "second".len()
        );
    }

    #[test]
    fn builder_collapses_each_explicit_blockquote_softbreak() {
        let source = "> 日期：2026-07-20\n> 状态：待评审\n> 目标：加载 Wiki";
        let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
        let paragraph = &doc.blocks[0].children[0];

        assert_eq!(paragraph.text_lines, ["日期：2026-07-20 状态：待评审 目标：加载 Wiki"]);
    }

    #[test]
    fn builder_keeps_lazy_blockquote_continuation_as_one_line() {
        let source = "> first\nsecond";
        let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());
        let paragraph = &doc.blocks[0].children[0];

        assert_eq!(paragraph.text_lines, ["first second"]);
    }

    #[test]
    fn builder_preserves_hard_break_lines() {
        let source = "first\\\nsecond";
        let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());

        assert_eq!(doc.blocks[0].text_lines, ["first", "second"]);
    }

    #[test]
    fn builder_renders_inline_html_break_as_a_hard_break() {
        for html_break in ["<br>", "<br/>", "<br />"] {
            let source = format!("# first{html_break}second");
            let doc = MarkdownDoc::build(&parse_markdown(&source), &default_style());

            assert_eq!(
                doc.blocks[0].text_lines,
                ["first", "second"],
                "inline HTML break {html_break:?} must create two visual lines"
            );
        }
    }

    #[test]
    fn builder_keeps_non_break_inline_html_as_literal_text() {
        let source = "# first<span>second</span>";
        let doc = MarkdownDoc::build(&parse_markdown(source), &default_style());

        assert_eq!(doc.blocks[0].text_lines, ["first<span>second</span>"]);
    }

    #[test]
    fn builder_nested_list_parent_projection_ends_at_parent_content() {
        let source = "- outer\n  - inner wrapped content";
        let parsed = crate::parser::parse_markdown(source);
        let style = crate::test_utils::default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let parent = doc.blocks.first().expect("fixture must produce a parent list item");
        let projection = parent
            .projected_lines
            .first()
            .expect("parent list item must retain its text projection");
        let parent_content_end = source.find('\n').expect("fixture must contain nested list line");

        assert_eq!(projection.spans[0].source_range.end, parent_content_end);
        assert_eq!(
            projection.boundaries.last().expect("projection needs a terminal anchor").byte,
            parent_content_end,
            "parent terminal anchor must not include nested child source"
        );
    }

    #[test]
    fn builder_plain_softbreak_maps_newline_to_one_visual_space() {
        let source = "first\nsecond";
        let parsed = crate::parser::parse_markdown(source);
        let style = crate::test_utils::default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let projected = &doc.blocks[0].projected_lines[0];
        assert_eq!(projected.text, "first second");
        assert_eq!(projected.boundaries[6].byte, 6);
    }

    #[test]
    fn builder_trims_fenced_code_projection_trailing_newline() {
        let source = "```\nhello\n```";
        let parsed = crate::parser::parse_markdown(source);
        let style = crate::test_utils::default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let code_block = &doc.blocks[0];
        let projected = &code_block.projected_lines[0];

        assert_eq!(code_block.text_lines, ["hello"]);
        assert_eq!(projected.text, "hello");
        assert_eq!(
            projected.boundaries.last().expect("projection needs a sentinel").byte,
            source.find("\n```").expect("fixture must contain closing fence")
        );
    }

    #[test]
    fn builder_empty_heading_has_an_empty_projection_line() {
        let source = "#";
        let parsed = crate::parser::parse_markdown(source);
        let style = crate::test_utils::default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let heading = &doc.blocks[0];

        assert_eq!(heading.text_lines, [""]);
        assert_eq!(heading.projected_lines.len(), heading.text_lines.len());
        assert_eq!(heading.projected_lines[0].text, "");
        assert_eq!(heading.projected_lines[0].boundaries[0].byte, source.len());
    }

    #[test]
    fn build_horizontal_rule() {
        // "---" after text is still a horizontal rule
        let parsed = parse_markdown(
            "text

---",
        );
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let has_rule = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::HorizontalRule));
        assert!(has_rule);
    }

    #[test]
    fn build_yaml_metadata_block() {
        let parsed = parse_markdown(
            "---
title: hello
---",
        );
        let doc = MarkdownDoc::build(&parsed, &default_style());
        let has_metadata = doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::MetadataBlock));
        assert!(has_metadata, "YAML --- at document start should produce MetadataBlock");
    }

    #[test]
    fn source_range_captures_bold_marker() {
        let src = "hello **world** here";
        let parsed = parse_markdown(src);
        let doc = MarkdownDoc::build(&parsed, &default_style());

        // 找到 Bold span
        let bold_span = doc
            .blocks
            .iter()
            .flat_map(|b| b.text_styles.iter().flatten())
            .find(|s| s.style == InlineStyle::Bold)
            .expect("should have a bold span");

        // source_range 应包含 ** 标记: "**world**" 在 src[6..15]
        assert_eq!(bold_span.source_range, 6..15);
        assert_eq!(&src[bold_span.source_range.clone()], "**world**");
    }

    #[test]
    fn source_range_captures_italic_marker() {
        let src = "hello *world* here";
        let parsed = parse_markdown(src);
        let doc = MarkdownDoc::build(&parsed, &default_style());

        let italic_span = doc
            .blocks
            .iter()
            .flat_map(|b| b.text_styles.iter().flatten())
            .find(|s| s.style == InlineStyle::Italic)
            .expect("should have an italic span");

        // "*world*" 在 src[6..13]
        assert_eq!(italic_span.source_range, 6..13);
        assert_eq!(&src[italic_span.source_range.clone()], "*world*");
    }

    #[test]
    fn source_range_inline_code() {
        let src = "use `println!` here";
        let parsed = parse_markdown(src);
        let doc = MarkdownDoc::build(&parsed, &default_style());

        let code_span = doc
            .blocks
            .iter()
            .flat_map(|b| b.text_styles.iter().flatten())
            .find(|s| s.style == InlineStyle::InlineCode)
            .expect("should have an inline code span");

        // "`println!`" 在 src[4..14]
        assert_eq!(code_span.source_range, 4..14);
        assert_eq!(&src[code_span.source_range.clone()], "`println!`");
    }

    #[test]
    fn plain_text_has_no_style_spans() {
        let src = "plain text";
        let parsed = parse_markdown(src);
        let doc = MarkdownDoc::build(&parsed, &default_style());

        let all_spans: Vec<&StyleSpan> =
            doc.blocks.iter().flat_map(|b| b.text_styles.iter().flatten()).collect();
        assert!(all_spans.is_empty(), "plain text should produce no style spans");
    }

    #[test]
    fn block_source_range_covers_paragraph() {
        let src = "hello world";
        let parsed = parse_markdown(src);
        let doc = MarkdownDoc::build(&parsed, &default_style());

        let para = doc
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Paragraph))
            .expect("should have a Paragraph block");

        // 整段文本在 src[0..11]
        assert_eq!(para.block_range, 0..11);
        assert_eq!(&src[para.block_range.clone()], "hello world");
    }

    #[test]
    fn block_source_range_covers_heading() {
        let src = "# Title";
        let parsed = parse_markdown(src);
        let doc = MarkdownDoc::build(&parsed, &default_style());

        let heading = doc
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Heading { level: 1 }))
            .expect("should have a Heading block");

        assert_eq!(heading.block_range, 0..7);
        assert_eq!(&src[heading.block_range.clone()], "# Title");
    }

    // ===== NovelStructure 测试 =====

    fn novel_scan(text: &str) -> NovelStructure {
        NovelStructure::scan(&core::document::StringDocView::new(text))
    }

    #[test]
    fn novel_scan_empty_input() {
        let ns = novel_scan("");
        assert!(ns.blocks().is_empty());
        assert!(ns.headings().is_empty());
    }

    #[test]
    fn novel_scan_single_paragraph() {
        let ns = novel_scan("hello world");
        assert_eq!(ns.blocks().len(), 1);
        assert!(matches!(ns.blocks()[0].kind, BlockKind::Paragraph));
    }

    #[test]
    fn novel_scan_heading_h1() {
        let ns = novel_scan("第1章 开始");
        assert_eq!(ns.blocks().len(), 1);
        assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 2 }));
    }

    #[test]
    fn novel_scan_heading_book_title() {
        let ns = novel_scan("《我的小说》 作者：某某");
        assert_eq!(ns.blocks().len(), 1);
        assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 1 }));
    }

    #[test]
    fn novel_scan_special_marker() {
        let ns = novel_scan("楔子");
        assert_eq!(ns.blocks().len(), 1);
        assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 2 }));
    }

    #[test]
    fn novel_scan_multiple_paragraphs() {
        let text = "第一段文字。\n\n第二段文字！\n\n第三段文字";
        let ns = novel_scan(text);
        assert_eq!(ns.blocks().len(), 3);
        for block in ns.blocks() {
            assert!(matches!(block.kind, BlockKind::Paragraph));
        }
    }

    #[test]
    fn novel_scan_heading_and_paragraph() {
        let text = "第1章 序\n\n这是正文内容。";
        let ns = novel_scan(text);
        assert_eq!(ns.blocks().len(), 2);
        assert!(matches!(ns.blocks()[0].kind, BlockKind::Heading { level: 2 }));
        assert!(matches!(ns.blocks()[1].kind, BlockKind::Paragraph));
    }

    #[test]
    fn novel_scan_headings_populated() {
        let text = "《书名》\n\n第1章 开始\n\n正文";
        let ns = novel_scan(text);
        assert_eq!(ns.headings().len(), 2);
        assert_eq!(ns.headings()[0].text, "《书名》");
        assert_eq!(ns.headings()[0].level, 1);
        assert_eq!(ns.headings()[1].text, "第1章 开始");
        assert_eq!(ns.headings()[1].level, 2);
    }

    #[test]
    fn novel_scan_body_text_not_heading() {
        let text = "这是一段普通的中文正文，不应被识别为标题。";
        let ns = novel_scan(text);
        assert_eq!(ns.blocks().len(), 1);
        assert!(matches!(ns.blocks()[0].kind, BlockKind::Paragraph));
    }

    #[test]
    fn novel_scan_paragraph_split_by_end_chars() {
        // 句号、感叹号、问号等结束符作为行末字符时产生段落分割
        let text = "第一句话。\n第二句话！\n第三句话？";
        let ns = novel_scan(text);
        assert_eq!(ns.blocks().len(), 3);
        for block in ns.blocks() {
            assert!(matches!(block.kind, BlockKind::Paragraph));
        }
    }

    #[test]
    fn novel_scan_matches_build_from_novel_doc() {
        // 验证 scan() 产生的 block kinds 与 byte ranges 和 build_from_novel_doc 一致
        let text =
            "《小说》\n\n第1章 序\n\n这是第一节内容。\n还有更多。\n\n第2章 开篇\n\n第二节的内容！";
        let ns = NovelStructure::scan(&core::document::StringDocView::new(text));
        let md = MarkdownDoc::build_from_novel_doc(&core::document::StringDocView::new(text));

        let ns_kinds: Vec<&BlockKind> = ns.blocks().iter().map(|b| &b.kind).collect();
        let md_kinds: Vec<&BlockKind> = md.blocks.iter().map(|b| &b.kind).collect();
        assert_eq!(
            ns_kinds, md_kinds,
            "scan() and build_from_novel_doc must produce identical block kinds"
        );

        let ns_ranges: Vec<_> = ns.blocks().iter().map(|b| b.block_range.clone()).collect();
        let md_ranges: Vec<_> = md.blocks.iter().map(|b| b.block_range.clone()).collect();
        assert_eq!(
            ns_ranges, md_ranges,
            "scan() and build_from_novel_doc must produce identical block ranges"
        );
    }

    #[test]
    fn novel_scan_implements_block_source() {
        let text = "第1章 标题\n\n正文内容。";
        let ns = novel_scan(text);
        // 通过 BlockSource trait 访问
        let blocks: &[BlockNode] = crate::layout::BlockSource::blocks(&ns);
        let headings: &[crate::layout::HeadingEntry] = crate::layout::BlockSource::headings(&ns);
        assert_eq!(blocks.len(), 2);
        assert_eq!(headings.len(), 1);
    }
}
