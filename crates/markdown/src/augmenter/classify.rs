use super::*;

// ─── 分类器 ────────────────────────────────────────────────────────────────

/// Enter 键的上下文分类。分类结果决定 `enter_context_augmentation` 的分支。
#[derive(Clone, Copy, Debug)]
pub enum HeadingPosition {
    Start { insert_at: usize },
    Interior,
    End { insert_at: usize },
}

#[derive(Debug)]
pub enum EnterContext {
    TopLevelParagraphEnd,
    ParagraphInterior,
    ParagraphStart {
        insert_at: usize,
    },
    Heading {
        level: u8,
        position: HeadingPosition,
        continuation_prefix: String,
    },
    /// Setext 标题；`underline_end` 不含下划线后的源码换行。
    SetextHeading {
        position: HeadingPosition,
        underline_end: usize,
        continuation_prefix: String,
    },
    ListItem {
        indent: String,
        continuation_marker: String,
        content_prefix: String,
        empty: bool,
        at_end: bool,
    },
    BlockQuoteLine {
        continuation_prefix: String,
        empty_parent_prefix: String,
        empty: bool,
    },
    CodeBlock,
    /// 围栏代码块的围栏行(开头或闭合);`line_content_end` 是该围栏行的
    /// 源码内容末尾(不含换行)。
    CodeBlockFenceLine {
        is_opening: bool,
        line_content_end: usize,
    },
    /// 缩进代码块及 Enter 后新行应继承的源码前缀。
    IndentedCodeBlock {
        continuation_prefix: String,
    },
    /// Metadata or block HTML: Markdown hard-break syntax is not valid here.
    LiteralBlockLine {
        continuation_prefix: String,
    },
    TableCell {
        cell_range: std::ops::Range<usize>,
        next_cell_start: Option<usize>,
        previous_tab_cell_start: Option<usize>,
        next_tab_cell_start: Option<usize>,
        column_count: usize,
        row_is_empty: bool,
        is_header_row: bool,
        is_last_cell: bool,
        row_line_end: usize,
        container_prefix: String,
    },
    EmptyBlockSeparatorLine,
    HorizontalRuleEnd,
    Other,
}

pub(in crate::augmenter) struct ItemFrame {
    pub(in crate::augmenter) start: usize,
    pub(in crate::augmenter) marker_end: usize,
    pub(in crate::augmenter) indent: String,
    pub(in crate::augmenter) continuation_marker: String,
    pub(in crate::augmenter) content_prefix: String,
    pub(in crate::augmenter) saw_content: bool,
}

pub(in crate::augmenter) struct TableFrame {
    pub(in crate::augmenter) cell_ranges: Vec<Vec<std::ops::Range<usize>>>,
    pub(in crate::augmenter) table_end: usize,
    pub(in crate::augmenter) container_prefix: String,
}

pub(in crate::augmenter) struct CodeBlockFrame {
    pub(in crate::augmenter) range: std::ops::Range<usize>,
    pub(in crate::augmenter) is_indented: bool,
    pub(in crate::augmenter) container_path: ContainerPath,
}

pub(in crate::augmenter) struct QuoteFrame {
    pub(in crate::augmenter) start: usize,
    pub(in crate::augmenter) continuation_prefix: String,
    pub(in crate::augmenter) empty_parent_prefix: String,
}

fn build_quote_frame(source: &str, range_start: usize) -> QuoteFrame {
    let bytes = source.as_bytes();
    let mut probe = range_start;
    while matches!(bytes.get(probe), Some(b' ' | b'\t')) {
        probe += 1;
    }
    let marker_end = if bytes.get(probe) == Some(&b'>') {
        let after_marker = probe + 1;
        if matches!(bytes.get(after_marker), Some(b' ' | b'\t')) {
            after_marker + 1
        } else {
            after_marker
        }
    } else {
        locate_blockquote_line(source, range_start)
            .map_or(range_start, |(_, content_start, _)| content_start)
    };
    QuoteFrame {
        start: range_start,
        continuation_prefix: canonical_container_prefix(source, marker_end),
        empty_parent_prefix: blockquote_parent_prefix(source, marker_end),
    }
}

fn explicit_blockquote_line_context(source: &str, current_byte: usize) -> Option<EnterContext> {
    let (_line_start, content_start, line_end) = locate_blockquote_line(source, current_byte)?;
    let content_end = source_line_content_end(source, line_end);
    (current_byte >= content_start && current_byte <= content_end).then(|| {
        EnterContext::BlockQuoteLine {
            continuation_prefix: canonical_container_prefix(source, content_start),
            empty_parent_prefix: blockquote_parent_prefix(source, content_start),
            empty: content_start == content_end,
        }
    })
}

fn classify_blockquote_frame(
    source: &str,
    current_byte: usize,
    frame: &QuoteFrame,
    range_end: usize,
) -> Option<(usize, EnterContext)> {
    let end = content_end_without_trailing_newline(source, frame.start..range_end);
    if let Some((line_start, content_start, line_end)) =
        locate_blockquote_line(source, current_byte)
    {
        let in_frame_range = (current_byte >= frame.start && current_byte <= end)
            || (content_start > frame.start && line_start <= range_end);
        let content_end = source_line_content_end(source, line_end);
        if !in_frame_range || current_byte < content_start || current_byte > content_end {
            return None;
        }
        return Some((
            frame.start,
            EnterContext::BlockQuoteLine {
                continuation_prefix: canonical_container_prefix(source, content_start),
                empty_parent_prefix: blockquote_parent_prefix(source, content_start),
                empty: content_start == content_end,
            },
        ));
    }

    if current_byte < frame.start || current_byte > end {
        return None;
    }
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let content_end = source_line_content_end(source, line_end);
    if current_byte < line_start
        || current_byte > content_end
        || source_line_is_empty(source, current_byte)
    {
        return None;
    }
    Some((
        frame.start,
        EnterContext::BlockQuoteLine {
            continuation_prefix: frame.continuation_prefix.clone(),
            empty_parent_prefix: frame.empty_parent_prefix.clone(),
            empty: false,
        },
    ))
}

pub(in crate::augmenter) fn classify_heading_hit(
    source: &str,
    current_byte: usize,
    level: u8,
    start: usize,
    range: &std::ops::Range<usize>,
) -> EnterContext {
    // pulldown-cmark 对 ATX 与 setext 标题都发 `Tag::Heading`；两种源码形态必须
    // 分流，否则 setext 会被错误套用 `# ` 前缀语义。
    let end = content_end_without_trailing_newline(source, start..range.end);
    if !heading_source_is_atx(source, start) {
        if current_byte >= start && current_byte <= end {
            return EnterContext::SetextHeading {
                position: if is_visible_content_start(source, start, current_byte) {
                    HeadingPosition::Start { insert_at: start }
                } else {
                    HeadingPosition::Interior
                },
                underline_end: end,
                continuation_prefix: canonical_container_prefix(source, start),
            };
        }
        return EnterContext::Other;
    }
    // 内容起点按实际源码扫描:至多 3 个前导空格 + `#` 序列 + 任意空白(空格/Tab),
    // 不能假定 `#` 后恰好一个空格(`#  Title`、`#\tTitle` 会落空在空白内)。
    let bytes = source.as_bytes();
    let mut probe = start;
    let mut leading_spaces = 0;
    while bytes.get(probe) == Some(&b' ') && leading_spaces < MAX_LEADING_BLOCK_INDENT {
        probe += 1;
        leading_spaces += 1;
    }
    probe += bytes[probe..].iter().take_while(|&&byte| byte == b'#').count();
    while matches!(bytes.get(probe), Some(b' ' | b'\t')) {
        probe += 1;
    }
    let content_start = probe.min(end);
    let position = if current_byte == end || heading_ends_at(source, start, current_byte) {
        HeadingPosition::End { insert_at: end }
    } else if is_visible_content_start(source, content_start, current_byte) {
        HeadingPosition::Start { insert_at: start }
    } else {
        HeadingPosition::Interior
    };
    if current_byte >= content_start && current_byte <= end {
        EnterContext::Heading {
            level,
            position,
            continuation_prefix: canonical_container_prefix(source, start),
        }
    } else {
        EnterContext::Other
    }
}

pub(in crate::augmenter) fn heading_ends_at(
    source: &str,
    heading_start: usize,
    cursor: usize,
) -> bool {
    fn find_heading(blocks: &[crate::builder::BlockNode], start: usize, cursor: usize) -> bool {
        for block in blocks {
            if block.block_range.start == start
                && matches!(block.kind, crate::builder::BlockKind::Heading { .. })
            {
                return editable_paragraph_navigation::has_text_boundary(
                    std::slice::from_ref(block),
                    cursor,
                    editable_paragraph_navigation::Boundary::End,
                );
            }
            if find_heading(&block.children, start, cursor) {
                return true;
            }
        }
        false
    }
    let document =
        crate::builder::MarkdownDoc::build_structure(&crate::parser::parse_markdown(source));
    find_heading(&document.blocks, heading_start, cursor)
}

/// 标题 range 起始处的源码必须是合法的 ATX marker：至多 3 个前导空格，
/// 1-6 个 `#`，后跟空格/制表符/行尾。
pub(crate) fn heading_source_is_atx(source: &str, heading_start: usize) -> bool {
    let bytes = source.as_bytes();
    let mut marker_probe = heading_start;
    let mut leading_spaces = 0;
    while bytes.get(marker_probe) == Some(&b' ') && leading_spaces < MAX_LEADING_BLOCK_INDENT {
        marker_probe += 1;
        leading_spaces += 1;
    }
    let hash_count = bytes[marker_probe..].iter().take_while(|&&byte| byte == b'#').count();
    if !(1..=6).contains(&hash_count) {
        return false;
    }
    matches!(bytes.get(marker_probe + hash_count), None | Some(b' ' | b'\t' | b'\r' | b'\n'))
}

/// 光标位于围栏代码块的围栏行时返回对应上下文,否则返回 `None`。
///
/// 开头围栏行 = 块起始所在源码行;闭合围栏行 = 块内容末尾所在行,且必须
/// 确实是合法闭合围栏(未闭合、延伸到 EOF 的代码块没有闭合围栏行)。
pub(in crate::augmenter) fn classify_fence_line_hit(
    source: &str,
    current_byte: usize,
    block_start: usize,
    block_content_end: usize,
    container_path: &ContainerPath,
) -> Option<EnterContext> {
    let (opening_line_start, _, opening_line_end) = locate_source_line_bounds(source, block_start)?;
    let opening_content_end = source_line_content_end(source, opening_line_end);
    let opening_position = LineColumnPosition::at_byte(source, opening_line_start, block_start)?;
    let (fence_char, opening_fence_len) =
        opening_fence_signature(source, opening_position, opening_content_end)?;
    if current_byte >= opening_line_start && current_byte <= opening_content_end {
        return Some(EnterContext::CodeBlockFenceLine {
            is_opening: true,
            line_content_end: opening_content_end,
        });
    }

    let (closing_line_start, _, closing_line_end) =
        locate_source_line_bounds(source, block_content_end)?;
    if closing_line_start == opening_line_start {
        return None;
    }
    let closing_content_end = source_line_content_end(source, closing_line_end);
    let closing_position = matching_container_position(
        source,
        closing_line_start,
        closing_content_end,
        container_path,
    )?;
    if !line_is_closing_fence(
        source,
        closing_position,
        closing_content_end,
        fence_char,
        opening_fence_len,
    ) {
        return None;
    }
    if current_byte >= closing_line_start && current_byte <= closing_content_end {
        return Some(EnterContext::CodeBlockFenceLine {
            is_opening: false,
            line_content_end: closing_content_end,
        });
    }
    None
}

#[derive(Clone, Debug)]
pub(in crate::augmenter) struct FencedBoundaryFrame {
    pub(in crate::augmenter) marker_start: usize,
    pub(in crate::augmenter) opening_position: LineColumnPosition,
    pub(in crate::augmenter) container_path: ContainerPath,
}

pub(in crate::augmenter) struct EnclosingFencedBoundary {
    pub(in crate::augmenter) frame: FencedBoundaryFrame,
    pub(in crate::augmenter) block_end: usize,
}

/// 该冷路径只在 CodeBlock 行尾 Delete 上触发。公开分类 API 不携带 parser frame，
/// 因此这里独立解析一次以取得 opener 与容器栈，避免扩大 EnterContext 公共形状。
pub(in crate::augmenter) fn line_closes_enclosing_fenced_block(
    source: &str,
    current_byte: usize,
    candidate_line_start: usize,
) -> bool {
    let Some(boundary) = enclosing_fenced_boundary(source, current_byte) else {
        return false;
    };
    candidate_line_closes_fenced_boundary(source, candidate_line_start, &boundary)
}

pub(in crate::augmenter) fn enclosing_fenced_boundary(
    source: &str,
    current_byte: usize,
) -> Option<EnclosingFencedBoundary> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

    let parser = Parser::new_ext(source, crate::parser::markdown_options());
    let mut item_container_paths = Vec::new();
    let mut fenced_block = None;
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Item) => {
                let container_path = parse_list_marker(source, range.start)
                    .map(|(_, content_start)| {
                        container_path(&canonical_container_prefix(source, content_start))
                    })
                    .or_else(|| item_container_paths.last().cloned())
                    .unwrap_or_default();
                item_container_paths.push(container_path);
            }
            Event::End(TagEnd::Item) => {
                item_container_paths.pop();
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                fenced_block =
                    fenced_boundary_frame(source, range.start, item_container_paths.last());
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => fenced_block = None,
            Event::End(TagEnd::CodeBlock) => {
                let Some(frame) = fenced_block.take() else {
                    continue;
                };
                let content_end =
                    content_end_without_trailing_newline(source, frame.marker_start..range.end);
                if current_byte >= frame.marker_start && current_byte <= content_end {
                    return Some(EnclosingFencedBoundary { frame, block_end: range.end });
                }
            }
            _ => {}
        }
    }
    None
}

pub(in crate::augmenter) fn fenced_boundary_frame(
    source: &str,
    marker_start: usize,
    item_container_path: Option<&ContainerPath>,
) -> Option<FencedBoundaryFrame> {
    let (opening_line_start, _, opening_line_end) =
        locate_source_line_bounds(source, marker_start)?;
    let opening_content_end = source_line_content_end(source, opening_line_end);
    let base_path = item_container_path.cloned().unwrap_or_default();
    let container_path =
        source_container_path(source, opening_line_start, marker_start, &base_path);
    let opening_position = matching_container_position(
        source,
        opening_line_start,
        opening_content_end,
        &container_path,
    )
    .or_else(|| LineColumnPosition::at_byte(source, opening_line_start, marker_start))?;
    Some(FencedBoundaryFrame { marker_start, opening_position, container_path })
}

pub(in crate::augmenter) fn candidate_line_closes_fenced_boundary(
    source: &str,
    candidate_line_start: usize,
    boundary: &EnclosingFencedBoundary,
) -> bool {
    let Some((_, _, opening_line_end)) =
        locate_source_line_bounds(source, boundary.frame.marker_start)
    else {
        return false;
    };
    let opening_content_end = source_line_content_end(source, opening_line_end);
    let Some((fence_char, opening_fence_len)) =
        opening_fence_signature(source, boundary.frame.opening_position, opening_content_end)
    else {
        return false;
    };
    let Some((actual_line_start, _, candidate_line_end)) =
        locate_source_line_bounds(source, candidate_line_start)
    else {
        return false;
    };
    if actual_line_start != candidate_line_start
        || candidate_line_start < boundary.frame.marker_start
    {
        return false;
    }
    let candidate_content_end = source_line_content_end(source, candidate_line_end);
    if candidate_content_end > boundary.block_end {
        return false;
    }
    let Some(candidate_position) = matching_container_position(
        source,
        candidate_line_start,
        candidate_content_end,
        &boundary.frame.container_path,
    ) else {
        return false;
    };
    line_is_closing_fence(
        source,
        candidate_position,
        candidate_content_end,
        fence_char,
        opening_fence_len,
    )
}

pub(in crate::augmenter) fn container_path(prefix: &str) -> ContainerPath {
    let bytes = prefix.as_bytes();
    let mut segments = Vec::new();
    let mut probe = 0;
    let mut absolute_column = 0;
    while probe < bytes.len() {
        let whitespace_start = probe;
        let whitespace_start_column = absolute_column;
        while matches!(bytes.get(probe), Some(b' ' | b'\t')) {
            absolute_column = if bytes[probe] == b'\t' {
                next_markdown_tab_stop(absolute_column)
            } else {
                absolute_column + 1
            };
            probe += 1;
        }
        if whitespace_start < probe {
            let width = absolute_column - whitespace_start_column;
            let precedes_quote = bytes.get(probe) == Some(&b'>');
            let is_optional_quote_indent = precedes_quote
                && width <= MAX_LEADING_BLOCK_INDENT
                && bytes[whitespace_start..probe].iter().all(|byte| *byte == b' ');
            if !is_optional_quote_indent {
                segments.push(ContainerSegment::IndentColumns(width));
            }
        }
        if bytes.get(probe) != Some(&b'>') {
            break;
        }
        segments.push(ContainerSegment::BlockQuote);
        probe += 1;
        absolute_column += 1;
        if matches!(bytes.get(probe), Some(b' ' | b'\t')) {
            absolute_column = if bytes[probe] == b'\t' {
                next_markdown_tab_stop(absolute_column)
            } else {
                absolute_column + 1
            };
            probe += 1;
        }
    }
    ContainerPath(segments)
}

pub(in crate::augmenter) fn source_container_path(
    source: &str,
    line_start: usize,
    prefix_end: usize,
    base_path: &ContainerPath,
) -> ContainerPath {
    if !base_path.0.is_empty()
        && let Some(position) =
            matching_container_position(source, line_start, prefix_end, base_path)
    {
        return append_source_block_quotes(source, prefix_end, base_path.clone(), position);
    }
    explicit_source_container_path(source, line_start, prefix_end)
}

pub(in crate::augmenter) fn append_source_block_quotes(
    source: &str,
    prefix_end: usize,
    mut path: ContainerPath,
    mut position: LineColumnPosition,
) -> ContainerPath {
    while position.byte_offset < prefix_end {
        let Some(next_position) = match_block_quote_segment(source, prefix_end, position) else {
            break;
        };
        if next_position.byte_offset <= position.byte_offset {
            break;
        }
        path.0.push(ContainerSegment::BlockQuote);
        position = next_position;
    }
    path
}

pub(in crate::augmenter) fn explicit_source_container_path(
    source: &str,
    line_start: usize,
    prefix_end: usize,
) -> ContainerPath {
    let bytes = source.as_bytes();
    let mut path = ContainerPath::default();
    let mut position = LineColumnPosition::at_line_start(line_start);
    while position.byte_offset < prefix_end {
        let mut marker_probe = position.byte_offset;
        let mut leading_spaces = 0;
        while marker_probe < prefix_end
            && bytes.get(marker_probe) == Some(&b' ')
            && leading_spaces < MAX_LEADING_BLOCK_INDENT
        {
            marker_probe += 1;
            leading_spaces += 1;
        }

        if let Some((_, content_start)) = parse_list_marker(source, marker_probe)
            && content_start <= prefix_end
            && let Some(content_position) =
                LineColumnPosition::at_byte(source, line_start, content_start)
            && let Some(required_columns) =
                content_position.absolute_column.checked_sub(position.absolute_column)
        {
            path.0.push(ContainerSegment::IndentColumns(required_columns));
            position = content_position;
            continue;
        }

        let Some(next_position) = match_block_quote_segment(source, prefix_end, position) else {
            break;
        };
        path.0.push(ContainerSegment::BlockQuote);
        position = next_position;
    }
    path
}

pub(in crate::augmenter) fn matching_container_position(
    source: &str,
    line_start: usize,
    line_end: usize,
    path: &ContainerPath,
) -> Option<LineColumnPosition> {
    let mut position = LineColumnPosition::at_line_start(line_start);
    for segment in &path.0 {
        position = match *segment {
            ContainerSegment::BlockQuote => match_block_quote_segment(source, line_end, position)?,
            ContainerSegment::IndentColumns(required_width) => {
                advance_container_columns(source, line_end, position, required_width)?
            }
        };
    }
    Some(position)
}

pub(in crate::augmenter) fn match_block_quote_segment(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
) -> Option<LineColumnPosition> {
    let mut leading_spaces = position.leaf_indent_columns()?;
    if leading_spaces > MAX_LEADING_BLOCK_INDENT {
        return None;
    }
    // 上一容器段的跨界 Tab residual 等价于引用 marker 前的可选缩进；
    // 从此处起它已属于容器路径，不能继续作为叶块缩进传播。
    position.container_target_column = position.absolute_column;

    let bytes = source.as_bytes();
    while position.byte_offset < line_end
        && bytes.get(position.byte_offset) == Some(&b' ')
        && leading_spaces < MAX_LEADING_BLOCK_INDENT
    {
        position.byte_offset += 1;
        position.absolute_column += 1;
        position.container_target_column += 1;
        leading_spaces += 1;
    }
    if bytes.get(position.byte_offset) != Some(&b'>') {
        return None;
    }
    position.byte_offset += 1;
    position.absolute_column += 1;
    position.container_target_column += 1;

    if position.byte_offset < line_end {
        match bytes[position.byte_offset] {
            b' ' => {
                position.byte_offset += 1;
                position.absolute_column += 1;
                position.container_target_column += 1;
            }
            b'\t' => {
                position.byte_offset += 1;
                position.absolute_column = next_markdown_tab_stop(position.absolute_column);
                position.container_target_column += 1;
            }
            _ => {}
        }
    }
    Some(position)
}

pub(in crate::augmenter) fn advance_container_columns(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
    required_columns: usize,
) -> Option<LineColumnPosition> {
    position.container_target_column =
        position.container_target_column.checked_add(required_columns)?;
    // Tab 可以一次越过目标列；多出的 1-3 列由叶块缩进继续使用。
    while position.absolute_column < position.container_target_column {
        position = advance_whitespace_byte(source, line_end, position)?;
    }
    Some(position)
}

pub(in crate::augmenter) fn advance_whitespace_byte(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
) -> Option<LineColumnPosition> {
    if position.byte_offset >= line_end {
        return None;
    }
    match source.as_bytes()[position.byte_offset] {
        b' ' => position.absolute_column += 1,
        b'\t' => position.absolute_column = next_markdown_tab_stop(position.absolute_column),
        _ => return None,
    }
    position.byte_offset += 1;
    Some(position)
}

pub(in crate::augmenter) fn marker_position_after_optional_indent(
    source: &str,
    line_end: usize,
    mut position: LineColumnPosition,
) -> Option<LineColumnPosition> {
    let mut leaf_indent_columns = position.leaf_indent_columns()?;
    if leaf_indent_columns > MAX_LEADING_BLOCK_INDENT {
        return None;
    }
    while position.byte_offset < line_end
        && matches!(source.as_bytes()[position.byte_offset], b' ' | b'\t')
    {
        // 容器目标已经满足后才开始的 Tab 属于叶块，必须从叶块列重新展开为 4 列；
        // 只有上面的 residual 才能表示同一 Tab 跨容器后留下的虚拟列。
        leaf_indent_columns = if source.as_bytes()[position.byte_offset] == b'\t' {
            next_markdown_tab_stop(leaf_indent_columns)
        } else {
            leaf_indent_columns + 1
        };
        position = advance_whitespace_byte(source, line_end, position)?;
        if leaf_indent_columns > MAX_LEADING_BLOCK_INDENT {
            return None;
        }
    }
    Some(position)
}

/// 扫描开头围栏行,返回(围栏字符, 围栏长度)。非法围栏返回 `None`。
pub(in crate::augmenter) fn opening_fence_signature(
    source: &str,
    line_position: LineColumnPosition,
    content_end: usize,
) -> Option<(u8, usize)> {
    let bytes = source.as_bytes();
    let fence_start =
        marker_position_after_optional_indent(source, content_end, line_position)?.byte_offset;
    let fence_char = *bytes.get(fence_start).filter(|byte| matches!(byte, b'`' | b'~'))?;
    let fence_len =
        bytes[fence_start..content_end].iter().take_while(|&&byte| byte == fence_char).count();
    (fence_len >= MIN_FENCE_MARKER_LENGTH).then_some((fence_char, fence_len))
}

/// 该行是否为合法闭合围栏:至多 3 个前导空格,≥ 开头围栏长度的相同围栏字符,
/// 其余位置只允许空白。未闭合代码块的末行(代码内容)不会通过此检查。
pub(in crate::augmenter) fn line_is_closing_fence(
    source: &str,
    line_position: LineColumnPosition,
    content_end: usize,
    fence_char: u8,
    opening_fence_len: usize,
) -> bool {
    let bytes = source.as_bytes();
    let Some(fence_start) =
        marker_position_after_optional_indent(source, content_end, line_position)
            .map(|position| position.byte_offset)
    else {
        return false;
    };
    let fence_len =
        bytes[fence_start..content_end].iter().take_while(|&&byte| byte == fence_char).count();
    if fence_len < opening_fence_len {
        return false;
    }
    bytes[fence_start + fence_len..content_end].iter().all(|byte| matches!(byte, b' ' | b'\t'))
}

pub fn classify_enter_context(source: &str, current_byte: usize) -> EnterContext {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

    #[cfg(test)]
    CLASSIFY_PARSE_COUNT.with(|parse_count| parse_count.set(parse_count.get() + 1));

    let mark_item_content_seen = |stack: &mut Vec<ItemFrame>| {
        if let Some(frame) = stack.last_mut() {
            frame.saw_content = true;
        }
    };

    let parser = Parser::new_ext(source, crate::parser::markdown_options());
    let mut item_stack: Vec<ItemFrame> = Vec::new();
    let mut quote_stack: Vec<QuoteFrame> = Vec::new();
    let mut table: Option<TableFrame> = None;
    let mut heading_level: Option<u8> = None;
    let mut heading_start: Option<usize> = None;
    let mut paragraph_start: Option<(usize, usize, usize)> = None;
    let mut code_block: Option<CodeBlockFrame> = None;
    let mut matched_list_item: Option<(usize, EnterContext)> = None;
    let mut matched_blockquote: Option<(usize, EnterContext)> = None;
    let mut matched_literal_block: Option<EnterContext> = None;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Item) => {
                // 子 item 本身也是父 item 的内容:只挂子列表的父 item 不算空,
                // 否则父行回车会走"空 item 退出"删掉 marker、把子列表提升为顶层。
                mark_item_content_seen(&mut item_stack);
                let indent = list_item_indent(source, range.start);
                if let Some((bullet, marker_end)) = parse_list_marker(source, range.start) {
                    item_stack.push(ItemFrame {
                        start: range.start,
                        marker_end,
                        indent,
                        continuation_marker: list_continuation_marker(
                            source,
                            range.start,
                            marker_end,
                            bullet,
                        ),
                        content_prefix: list_item_content_prefix(source, range.start, marker_end),
                        saw_content: false,
                    });
                } else {
                    let content_prefix = format!("{indent}  ");
                    item_stack.push(ItemFrame {
                        start: range.start,
                        marker_end: range.start,
                        indent,
                        continuation_marker: String::from("- "),
                        content_prefix,
                        saw_content: false,
                    });
                }
            }
            Event::End(TagEnd::Item) => {
                if let Some(frame) = item_stack.pop() {
                    // item range 含尾部换行 run（块间空行分隔也算在内）；
                    // 命中测试用去尾换行的内容末尾，否则紧随其后的顶层块首字节
                    // 会被误判为仍属于该 item。
                    let end =
                        content_end_without_trailing_newline(source, frame.marker_end..range.end);
                    // 光标在 marker 内部(`-| item`)时按内容起点处理,而不是落到
                    // Other 产生 `-\n item` 这样的懒延续残留。
                    if frame.marker_end > frame.start
                        && current_byte > frame.start
                        && current_byte <= end
                    {
                        // 光标所在行为纯空白(如嵌套空 item 退出后的残留缩进行)时,
                        // 即使 item 其他行有内容也按空 item 处理:回车应继续退出,
                        // 而不是在空白行下意外创建同级 item。
                        let blank_line_width = blank_source_line_width(source, current_byte);
                        let empty = !frame.saw_content || blank_line_width.is_some();
                        // 残留空白行可能同时落在多级 item 的 range 内;选择缩进严格
                        // 窄于该行空白宽度的最深帧——帧缩进与行内容等宽时原地替换
                        // 是不动点,无法真正"退一层"。
                        let frame_is_exit_target = match blank_line_width {
                            Some(width) => frame.indent.len() < width,
                            None => true,
                        };
                        let at_end = current_byte == end;
                        if frame_is_exit_target && matched_list_item.is_none() {
                            matched_list_item = Some((
                                frame.start,
                                EnterContext::ListItem {
                                    indent: frame.indent,
                                    continuation_marker: frame.continuation_marker,
                                    content_prefix: frame.content_prefix,
                                    empty,
                                    at_end,
                                },
                            ));
                        }
                    }
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                mark_item_content_seen(&mut item_stack);
                quote_stack.push(build_quote_frame(source, range.start));
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if let Some(frame) = quote_stack.pop()
                    && matched_blockquote.is_none()
                {
                    matched_blockquote =
                        classify_blockquote_frame(source, current_byte, &frame, range.end);
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                mark_item_content_seen(&mut item_stack);
                heading_level = Some(level as u8);
                heading_start = Some(range.start);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let (Some(level), Some(start)) = (heading_level.take(), heading_start.take())
                    && current_byte >= start
                    && current_byte <= range.end
                {
                    let res = classify_heading_hit(source, current_byte, level, start, &range);
                    if !matches!(res, EnterContext::Other) {
                        return res;
                    }
                }
            }
            Event::Start(Tag::Paragraph) => {
                mark_item_content_seen(&mut item_stack);
                paragraph_start = Some((range.start, item_stack.len(), quote_stack.len()));
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some((start, p_item_depth, p_bq_depth)) = paragraph_start.take()
                    && current_byte >= start
                    && current_byte <= range.end
                    && p_item_depth == 0
                    && p_bq_depth == 0
                {
                    let end = content_end_without_trailing_newline(source, start..range.end);
                    if current_byte == end {
                        return EnterContext::TopLevelParagraphEnd;
                    }
                    if is_visible_content_start(source, start, current_byte) {
                        return EnterContext::ParagraphStart { insert_at: start };
                    }
                    if source_line_is_empty(source, current_byte) {
                        return EnterContext::EmptyBlockSeparatorLine;
                    }
                    return EnterContext::ParagraphInterior;
                }
            }
            Event::Rule
                if item_stack.is_empty()
                    && quote_stack.is_empty()
                    && current_byte
                        == content_end_without_trailing_newline(source, range.clone()) =>
            {
                return EnterContext::HorizontalRuleEnd;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                mark_item_content_seen(&mut item_stack);
                code_block = Some(CodeBlockFrame {
                    range: range.clone(),
                    is_indented: matches!(kind, CodeBlockKind::Indented),
                    container_path: {
                        let base = item_stack
                            .last()
                            .map(|item| container_path(&item.content_prefix))
                            .unwrap_or_default();
                        let opening_line_start = locate_source_line_bounds(source, range.start)
                            .map_or(range.start, |bounds| bounds.0);
                        source_container_path(source, opening_line_start, range.start, &base)
                    },
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(frame) = code_block.take() {
                    // pulldown-cmark 对缩进代码块的 range 包含结尾换行
                    // （fenced 则不包含）；用去尾换行的 content_end 统一两种边界，
                    // 否则紧邻的下一块首字节会被误判为 CodeBlock。
                    let block_range = frame.range.start..range.end;
                    let end = content_end_without_trailing_newline(source, block_range.clone());
                    if current_byte >= frame.range.start && current_byte <= end {
                        if frame.is_indented {
                            let continuation_prefix = indented_code_continuation_prefix(
                                source,
                                current_byte,
                                &block_range,
                                &frame.container_path,
                            );
                            return EnterContext::IndentedCodeBlock { continuation_prefix };
                        }
                        if let Some(fence_context) = classify_fence_line_hit(
                            source,
                            current_byte,
                            frame.range.start,
                            end,
                            &frame.container_path,
                        ) {
                            return fence_context;
                        }
                        return EnterContext::CodeBlock;
                    }
                }
            }
            Event::Start(Tag::MetadataBlock(_))
                if current_byte >= range.start && current_byte <= range.end =>
            {
                matched_literal_block = Some(EnterContext::LiteralBlockLine {
                    continuation_prefix: literal_line_continuation_prefix(source, current_byte),
                });
            }
            Event::Start(Tag::Table(_)) => {
                mark_item_content_seen(&mut item_stack);
                table = Some(TableFrame {
                    cell_ranges: Vec::new(),
                    table_end: range.end,
                    container_prefix: canonical_container_prefix(source, range.start),
                });
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let Some(t) = table.as_mut() {
                    t.cell_ranges.push(Vec::new());
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(t) = table.as_mut()
                    && let Some(row) = t.cell_ranges.last_mut()
                {
                    row.push(range.start..range.end);
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = table.as_mut()
                    && let Some(row) = t.cell_ranges.last_mut()
                    && let Some(cell) = row.last_mut()
                {
                    cell.end = range.end;
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(table_frame) = table.as_mut() {
                    table_frame.table_end = range.end;
                }
            }
            Event::Html(text) if !text.is_empty() => {
                mark_item_content_seen(&mut item_stack);
                if current_byte >= range.start && current_byte <= range.end {
                    matched_literal_block = Some(EnterContext::LiteralBlockLine {
                        continuation_prefix: literal_line_continuation_prefix(source, current_byte),
                    });
                }
            }
            Event::Text(text) | Event::Code(text) | Event::InlineHtml(text) if !text.is_empty() => {
                mark_item_content_seen(&mut item_stack);
            }
            _ => {}
        }
    }

    if let Some(t) = table {
        for (row_idx, row) in t.cell_ranges.iter().enumerate() {
            for (col_idx, cell) in row.iter().enumerate() {
                if current_byte >= cell.start && current_byte <= cell.end {
                    let next_cell_start = t
                        .cell_ranges
                        .get(row_idx + 1)
                        .and_then(|next_row| next_row.get(col_idx))
                        .map(|next_cell| table_cell_content_start(source, next_cell));
                    let previous_tab_cell_start = if col_idx > 0 {
                        row.get(col_idx - 1)
                    } else {
                        row_idx.checked_sub(1).and_then(|previous_row| {
                            t.cell_ranges
                                .get(previous_row)
                                .and_then(|previous_cells| previous_cells.last())
                        })
                    }
                    .map(|previous_cell| table_cell_content_start(source, previous_cell));
                    let next_tab_cell_start = row
                        .get(col_idx + 1)
                        .or_else(|| {
                            t.cell_ranges.get(row_idx + 1).and_then(|next_row| next_row.first())
                        })
                        .map(|next_cell| table_cell_content_start(source, next_cell));
                    let column_count = row.len();
                    let row_is_empty = row.iter().all(|row_cell| {
                        source[row_cell.clone()]
                            .bytes()
                            .all(|byte| matches!(byte, b' ' | b'\t' | b'|' | b'\r' | b'\n'))
                    });
                    let is_header_row = row_idx == 0;
                    let last_cell_end = row.last().map_or(cell.end, |last_cell| last_cell.end);
                    let last_row_content_end = content_end_without_trailing_newline(
                        source,
                        0..t.table_end.max(last_cell_end),
                    );
                    let row_probe = if next_cell_start.is_none() {
                        last_row_content_end.saturating_sub(1)
                    } else {
                        last_cell_end.saturating_sub(1)
                    };
                    let row_line_end = locate_source_line_bounds(source, row_probe)
                        .map(|(_, _, line_end)| source_line_content_end(source, line_end))
                        .unwrap_or(last_cell_end);
                    return EnterContext::TableCell {
                        cell_range: cell.clone(),
                        next_cell_start,
                        previous_tab_cell_start,
                        next_tab_cell_start,
                        column_count,
                        row_is_empty,
                        is_header_row,
                        is_last_cell: row_idx + 1 == t.cell_ranges.len()
                            && col_idx + 1 == row.len(),
                        row_line_end,
                        container_prefix: t.container_prefix.clone(),
                    };
                }
            }
        }
    }

    if let Some(literal_block) = matched_literal_block {
        return literal_block;
    }

    match (matched_blockquote, matched_list_item) {
        (Some((quote_start, quote_ctx)), Some((item_start, item_ctx))) => {
            if item_start > quote_start {
                return item_ctx;
            }
            return quote_ctx;
        }
        (Some((_, quote_ctx)), None) => return quote_ctx,
        (None, Some((_, item_ctx))) => return item_ctx,
        (None, None) => {
            if let Some(explicit_quote) = explicit_blockquote_line_context(source, current_byte) {
                return explicit_quote;
            }
        }
    }

    if source_line_is_empty(source, current_byte) {
        return EnterContext::EmptyBlockSeparatorLine;
    }

    EnterContext::Other
}

pub(in crate::augmenter) fn literal_line_continuation_prefix(
    source: &str,
    current_byte: usize,
) -> String {
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, current_byte) else {
        return String::new();
    };
    let content_start = container_content_start_on_line(source, line_start, line_end);
    let indentation_end = source.as_bytes()[content_start..line_end]
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count()
        + content_start;
    format!(
        "{}{}",
        canonical_container_prefix(source, content_start),
        &source[content_start..indentation_end]
    )
}

pub(in crate::augmenter) fn source_line_is_empty(source: &str, byte: usize) -> bool {
    let Some((start, _, end)) = locate_source_line_bounds(source, byte) else {
        return false;
    };
    start == source_line_content_end(source, end)
}

/// 光标所在行为纯空白行(去 `\r` 后仅含空格/Tab)时返回其空白宽度,否则返回 `None`。
pub(in crate::augmenter) fn blank_source_line_width(source: &str, byte: usize) -> Option<usize> {
    let (start, _, end) = locate_source_line_bounds(source, byte)?;
    let content_end = source_line_content_end(source, end);
    let is_blank = source[start..content_end].bytes().all(|byte| matches!(byte, b' ' | b'\t'));
    is_blank.then_some(content_end - start)
}

pub(in crate::augmenter) fn source_line_content_end(source: &str, line_end: usize) -> usize {
    source[..line_end].strip_suffix('\r').map_or(line_end, |_| line_end - 1)
}

pub(in crate::augmenter) fn content_end_without_trailing_newline(
    source: &str,
    range: std::ops::Range<usize>,
) -> usize {
    let mut end = range.end.min(source.len());
    let start = range.start.min(end);
    let source_bytes = source.as_bytes();
    while end > start && matches!(source_bytes[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    end
}

pub(in crate::augmenter) fn line_indent_and_content(
    source: &str,
    line_start: usize,
    line_end: usize,
    block_range: &std::ops::Range<usize>,
    container_path: &ContainerPath,
) -> (String, bool) {
    let content_end = source_line_content_end(source, line_end);
    let container_end =
        matching_container_position(source, line_start, content_end, container_path)
            .map(|position| position.byte_offset)
            .unwrap_or_else(|| {
                if line_start <= block_range.start && block_range.start <= content_end {
                    block_range.start
                } else {
                    line_start
                }
            });
    let indent_width = source[container_end..content_end]
        .bytes()
        .take_while(|source_byte| matches!(*source_byte, b' ' | b'\t'))
        .count();
    let indent_end = container_end + indent_width;
    (canonical_container_prefix(source, indent_end), indent_end < content_end)
}

/// 计算缩进代码块续行前缀。空白行从同一块最近的非空行继承缩进。
pub(in crate::augmenter) fn indented_code_continuation_prefix(
    source: &str,
    current_byte: usize,
    block_range: &std::ops::Range<usize>,
    container_path: &ContainerPath,
) -> String {
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, current_byte) else {
        return String::new();
    };
    let (current_indent, current_has_content) =
        line_indent_and_content(source, line_start, line_end, block_range, container_path);
    if current_has_content {
        return current_indent;
    }

    let mut previous_line_start = line_start;
    while let Some(newline_width) = newline_sequence_width_before(source, previous_line_start) {
        let previous_line_end = previous_line_start - newline_width;
        if previous_line_end <= block_range.start {
            break;
        }
        let candidate_start = source[..previous_line_end]
            .bytes()
            .rposition(|source_byte| source_byte == b'\n')
            .map_or(0, |newline| newline + 1);
        let (candidate_indent, candidate_has_content) = line_indent_and_content(
            source,
            candidate_start,
            previous_line_end,
            block_range,
            container_path,
        );
        if candidate_has_content {
            return candidate_indent;
        }
        previous_line_start = candidate_start;
    }

    let mut next_line_start = line_end + newline_sequence_width_at(source, line_end).unwrap_or(0);
    while next_line_start < block_range.end && next_line_start < source.len() {
        let next_line_end = source[next_line_start..]
            .bytes()
            .position(|source_byte| source_byte == b'\n')
            .map_or(source.len(), |newline| next_line_start + newline);
        let (candidate_indent, candidate_has_content) = line_indent_and_content(
            source,
            next_line_start,
            next_line_end,
            block_range,
            container_path,
        );
        if candidate_has_content {
            return candidate_indent;
        }
        let newline_width = newline_sequence_width_at(source, next_line_end).unwrap_or(0);
        if newline_width == 0 {
            break;
        }
        next_line_start = next_line_end + newline_width;
    }

    current_indent
}

/// 单元格正文起点；空单元格落在单元格源码范围末端。
pub(crate) fn table_cell_content_start(source: &str, cell_range: &std::ops::Range<usize>) -> usize {
    let leading_non_content = source[cell_range.clone()]
        .bytes()
        .take_while(|source_byte| matches!(*source_byte, b' ' | b'\t' | b'\r' | b'\n' | b'|'))
        .count();
    cell_range.start + leading_non_content
}

pub(in crate::augmenter) fn table_insert_row_augmentation(
    source: &str,
    row_line_end: usize,
    column_count: usize,
    container_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, row_line_end);
    let mut row_source = String::from(container_prefix);
    row_source.push_str(&"|  ".repeat(column_count));
    row_source.push('|');

    let (insert_at, insertion, first_cell_content_offset) =
        if let Some(newline_width) = newline_sequence_width_at(source, row_line_end) {
            (row_line_end + newline_width, row_source, container_prefix.len() + 2)
        } else {
            let insertion = format!("{newline}{row_source}");
            (row_line_end, insertion, newline.len() + container_prefix.len() + 2)
        };
    let aug = EditAugmentation {
        cursor_byte_after: insert_at + first_cell_content_offset,
        replace_range: Some(insert_at..insert_at),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    aug
}

pub(in crate::augmenter) fn table_exit_empty_row_augmentation(
    source: &str,
    row_line_end: usize,
    container_prefix: &str,
) -> Option<EditAugmentation> {
    let row_probe = row_line_end.saturating_sub(1);
    let (row_start, _, _) = locate_source_line_bounds(source, row_probe)?;
    let preceding_newline_width = newline_sequence_width_before(source, row_start)?;
    let replace_start = row_start - preceding_newline_width;
    let newline = preferred_newline_sequence(source, replace_start);
    let insertion = if container_prefix.is_empty() {
        newline.repeat(BLOCK_BOUNDARY_NEWLINE_COUNT)
    } else {
        format!("{newline}{container_prefix}")
    };
    let aug = EditAugmentation {
        cursor_byte_after: replace_start + insertion.len(),
        replace_range: Some(replace_start..row_line_end),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}
