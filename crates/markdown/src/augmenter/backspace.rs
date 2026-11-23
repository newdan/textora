use super::*;

pub(in crate::augmenter) fn augment_backspace(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    if let Some(aug) = editable_paragraph_navigation::backspace_boundary(source, current_byte) {
        return Some(aug);
    }
    if let Some(fence_line) = fenced_line_span(source, current_byte) {
        if !fence_line.is_opening {
            return Some(consume_edit(current_byte));
        }
        return (current_byte <= fence_line.fence_end).then(|| consume_edit(current_byte));
    }
    let context = backspace_literal_context(source, current_byte);
    if matches!(context, EnterContext::CodeBlock | EnterContext::IndentedCodeBlock { .. }) {
        return backspace_code_boundary(source, current_byte);
    }
    if let Some(aug) = backspace_remove_inline_html_break(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_join_reopened_inline_elements(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_empty_source_line(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_at_atx_heading_marker_start(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_join_hard_break_line(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_paragraph_boundary(source, current_byte) {
        return Some(aug);
    }
    if let Some(aug) = backspace_last_isolated_paragraph_grapheme(source, current_byte) {
        return Some(aug);
    }
    let range = get_marker_delete_range(source, current_byte)?;
    Some(EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(range.clone()),
        cursor_byte_after: range.start,
    })
}

pub(in crate::augmenter) fn consume_edit(cursor: usize) -> EditAugmentation {
    EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: None,
        cursor_byte_after: cursor,
    }
}

pub(in crate::augmenter) fn backspace_literal_context(
    source: &MarkdownSource<'_>,
    cursor: usize,
) -> EnterContext {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, cursor) else {
        return EnterContext::Other;
    };
    let prefix = &source[line_start..cursor];
    if !prefix.chars().all(char::is_whitespace) && get_marker_delete_range(source, cursor).is_none()
    {
        return EnterContext::Other;
    }
    classify_source(source, cursor)
}

pub(in crate::augmenter) fn backspace_code_boundary(
    source: &MarkdownSource<'_>,
    cursor: usize,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, cursor)?;
    let boundary = enclosing_fenced_boundary(source, cursor)?;
    let position = matching_container_position(
        source,
        line_start,
        source_line_content_end(source, line_end),
        &boundary.frame.container_path,
    )?;
    if cursor != position.byte_offset {
        return None;
    }
    if position.absolute_column > position.container_target_column {
        return backspace_container_tab_residue(source, line_start, position);
    }
    let previous_end =
        line_start.checked_sub(newline_sequence_width_before(source, line_start)?)?;
    if matches!(
        classify_source(source, previous_end),
        EnterContext::CodeBlockFenceLine { is_opening: true, .. }
    ) {
        return Some(consume_edit(cursor));
    }
    if cursor == line_start {
        return None;
    }
    Some(EditAugmentation {
        replace_range: Some(previous_end..cursor),
        insert_text: Some(String::new()),
        cursor_byte_after: previous_end,
    })
}

pub(in crate::augmenter) fn backspace_container_tab_residue(
    source: &str,
    line_start: usize,
    position: LineColumnPosition,
) -> Option<EditAugmentation> {
    let tab_start = position.byte_offset.checked_sub(1)?;
    if source.as_bytes().get(tab_start) != Some(&b'\t') {
        return None;
    }
    let column_before_tab = markdown_column_after(source.get(line_start..tab_start)?, 0);
    let replacement = " ".repeat(position.container_target_column.checked_sub(column_before_tab)?);
    Some(EditAugmentation {
        replace_range: Some(tab_start..position.byte_offset),
        cursor_byte_after: tab_start + replacement.len(),
        insert_text: Some(replacement),
    })
}

pub(in crate::augmenter) fn delete_forward_code_row_join(
    source: &MarkdownSource<'_>,
    cursor: usize,
    next_line_start: usize,
) -> Option<EditAugmentation> {
    let boundary = enclosing_fenced_boundary(source, cursor)?;
    let (_, _, line_end) = locate_source_line_bounds(source, next_line_start)?;
    let position = matching_container_position(
        source,
        next_line_start,
        source_line_content_end(source, line_end),
        &boundary.frame.container_path,
    )?;
    if position.byte_offset == next_line_start {
        return None;
    }
    let literal_indent = " ".repeat(position.leaf_indent_columns()?);
    Some(EditAugmentation {
        replace_range: Some(cursor..position.byte_offset),
        insert_text: Some(literal_indent),
        cursor_byte_after: cursor,
    })
}

struct FencedLineSpan {
    fence_end: usize,
    line_content_end: usize,
    is_opening: bool,
}

fn fenced_line_span(source: &MarkdownSource<'_>, current_byte: usize) -> Option<FencedLineSpan> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let current_line = &source[line_start..source_line_content_end(source, line_end)];
    if !current_line.contains("```") && !current_line.contains("~~~") {
        return None;
    }
    let EnterContext::CodeBlockFenceLine { is_opening, line_content_end } =
        classify_source(source, current_byte)
    else {
        return None;
    };
    let boundary = enclosing_fenced_boundary(source, line_content_end)?;
    let line_position = if is_opening {
        boundary.frame.opening_position
    } else {
        matching_container_position(
            source,
            line_start,
            line_content_end,
            &boundary.frame.container_path,
        )?
    };
    let fence_start =
        marker_position_after_optional_indent(source, line_content_end, line_position)?.byte_offset;
    let fence_char =
        *source.as_bytes().get(fence_start).filter(|byte| matches!(byte, b'`' | b'~'))?;
    let fence_len = source.as_bytes()[fence_start..line_content_end]
        .iter()
        .take_while(|&&byte| byte == fence_char)
        .count();
    Some(FencedLineSpan { fence_end: fence_start + fence_len, line_content_end, is_opening })
}

pub(crate) fn augment_delete_forward(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    if let Some(augmentation) = editable_paragraph_navigation::delete_forward(source, current_byte)
    {
        return Some(augmentation);
    }
    if let Some(fence_line) = fenced_line_span(source, current_byte) {
        if fence_line.is_opening && current_byte < fence_line.fence_end {
            return Some(consume_edit(current_byte));
        }
        if !fence_line.is_opening && current_byte < fence_line.line_content_end {
            return Some(consume_edit(current_byte));
        }
        if current_byte < fence_line.line_content_end {
            return None;
        }
    }
    delete_forward_remove_inline_html_break(source, current_byte)
        .or_else(|| delete_forward_join_reopened_inline_elements(source, current_byte))
        .or_else(|| delete_forward_join_hard_break_line(source, current_byte))
        .or_else(|| {
            let grapheme = source.get(current_byte..)?.graphemes(true).next()?;
            editable_paragraph_edit::erase_last_grapheme(source, current_byte + grapheme.len())
        })
        .or_else(|| delete_forward_block_boundary(source, current_byte))
}

pub(crate) fn augment_erase_range(
    source: &MarkdownSource<'_>,
    erased: std::ops::Range<usize>,
) -> Option<EditAugmentation> {
    if erased.is_empty() || source.get(erased.clone()).is_none() {
        return None;
    }
    protect_fenced_selection(source, erased.clone())
        .or_else(|| protect_table_selection(source, erased.clone()))
        .or_else(|| editable_paragraph_edit::erase_range(source, erased))
}

fn protect_fenced_selection(
    source: &MarkdownSource<'_>,
    erased: std::ops::Range<usize>,
) -> Option<EditAugmentation> {
    let mut open_starts = Vec::new();
    for (event, range) in source.parsed().source_events() {
        match event {
            MarkdownEvent::Start(MarkdownTag::CodeBlock { .. })
                if source.parsed().is_fenced_code_start(range.start) =>
            {
                open_starts.push(range.start);
            }
            MarkdownEvent::End(MarkdownTagEnd::CodeBlock) => {
                let Some(marker_start) = open_starts.pop() else {
                    continue;
                };
                if let Some(aug) =
                    clip_partial_fenced_erasure(source, marker_start..range.end, &erased)
                {
                    return Some(aug);
                }
            }
            _ => {}
        }
    }
    None
}

fn clip_partial_fenced_erasure(
    source: &str,
    block_range: std::ops::Range<usize>,
    erased: &std::ops::Range<usize>,
) -> Option<EditAugmentation> {
    let marker_start = block_range.start;
    let (open_line_start, _, open_line_end) = locate_source_line_bounds(source, marker_start)?;
    let open_content_end = source_line_content_end(source, open_line_end);
    let close_content_end = content_end_without_trailing_newline(source, block_range);
    let (close_line_start, _, _) = locate_source_line_bounds(source, close_content_end)?;
    if close_line_start == open_line_start {
        return None;
    }
    let body_start = (open_line_end + LF_SEQUENCE.len()).min(source.len());
    let body_content_end = if body_start == close_line_start {
        body_start
    } else {
        close_line_start
            .saturating_sub(newline_sequence_width_before(source, close_line_start).unwrap_or(0))
    };
    if erased.end <= open_line_start || erased.start >= close_content_end {
        return None;
    }
    if erased.start <= open_line_start && erased.end >= close_content_end {
        return None;
    }
    if erased.start >= body_start && erased.end <= body_content_end {
        return None;
    }
    let open_fence_end = opening_fence_marker_end(source, marker_start, open_content_end);
    if erased.start >= open_fence_end && erased.end <= open_content_end {
        return None;
    }

    let mut sub_ranges = Vec::new();
    if erased.start < open_line_start {
        let starts_at_line_boundary = erased.start == 0 || source[..erased.start].ends_with('\n');
        let pre_end = if starts_at_line_boundary {
            open_line_start
        } else {
            open_line_start
                .saturating_sub(newline_sequence_width_before(source, open_line_start).unwrap_or(0))
        };
        if erased.start < pre_end {
            sub_ranges.push(erased.start..pre_end);
        }
    }
    let in_start = erased.start.max(body_start);
    let in_end = erased.end.min(body_content_end);
    if in_start < in_end {
        sub_ranges.push(in_start..in_end);
    }
    if erased.end > close_content_end {
        let post_start =
            close_content_end + newline_sequence_width_at(source, close_content_end).unwrap_or(0);
        if post_start < erased.end {
            sub_ranges.push(post_start..erased.end);
        }
    }
    Some(merge_disjoint_erasure_ranges(source, &sub_ranges, erased.start))
}

fn opening_fence_marker_end(source: &str, marker_start: usize, line_content_end: usize) -> usize {
    let bytes = source.as_bytes();
    let Some(fence_char) = bytes.get(marker_start).filter(|byte| matches!(byte, b'`' | b'~'))
    else {
        return marker_start;
    };
    let fence_len = bytes[marker_start..line_content_end]
        .iter()
        .take_while(|&&byte| byte == *fence_char)
        .count();
    marker_start + fence_len
}

fn protect_table_selection(
    source: &MarkdownSource<'_>,
    erased: std::ops::Range<usize>,
) -> Option<EditAugmentation> {
    let mut table_start: Option<usize> = None;
    let mut cell_starts: Vec<usize> = Vec::new();
    let mut cells: Vec<std::ops::Range<usize>> = Vec::new();

    for (event, range) in source.parsed().source_events() {
        match event {
            MarkdownEvent::Start(MarkdownTag::Table(_)) => {
                table_start = Some(range.start);
                cells.clear();
                cell_starts.clear();
            }
            MarkdownEvent::Start(MarkdownTag::TableCell) if table_start.is_some() => {
                cell_starts.push(range.start);
            }
            MarkdownEvent::End(MarkdownTagEnd::TableCell) if table_start.is_some() => {
                if let Some(start) = cell_starts.pop() {
                    cells.push(start..range.end);
                }
            }
            MarkdownEvent::End(MarkdownTagEnd::Table) => {
                let Some(start) = table_start.take() else {
                    continue;
                };
                if let Some(aug) =
                    clip_partial_table_erasure(source, start..range.end, &cells, &erased)
                {
                    return Some(aug);
                }
            }
            _ => {}
        }
    }
    None
}

fn clip_partial_table_erasure(
    source: &str,
    table_range: std::ops::Range<usize>,
    cells: &[std::ops::Range<usize>],
    erased: &std::ops::Range<usize>,
) -> Option<EditAugmentation> {
    let table_line_start = locate_source_line_bounds(source, table_range.start)
        .map_or(table_range.start, |(s, _, _)| s);
    let table_content_end = content_end_without_trailing_newline(source, table_range);
    if erased.end <= table_line_start || erased.start >= table_content_end {
        return None;
    }
    if erased.start <= table_line_start && erased.end >= table_content_end {
        return None;
    }
    if cells.iter().any(|cell| erased.start >= cell.start && erased.end <= cell.end) {
        return None;
    }

    let mut sub_ranges = Vec::new();
    if erased.start < table_line_start {
        let starts_at_line_boundary = erased.start == 0 || source[..erased.start].ends_with('\n');
        let pre_end = if starts_at_line_boundary {
            table_line_start
        } else {
            table_line_start.saturating_sub(
                newline_sequence_width_before(source, table_line_start).unwrap_or(0),
            )
        };
        if erased.start < pre_end {
            sub_ranges.push(erased.start..pre_end);
        }
    }
    for cell in cells {
        let content_start = table_cell_content_start(source, cell);
        let content_end = trim_trailing_cell_delimiters(source, content_start, cell.end);
        let del_start = erased.start.max(content_start);
        let del_end = erased.end.min(content_end);
        if del_start < del_end {
            sub_ranges.push(del_start..del_end);
        }
    }
    if erased.end > table_content_end {
        let post_start =
            table_content_end + newline_sequence_width_at(source, table_content_end).unwrap_or(0);
        if post_start < erased.end {
            sub_ranges.push(post_start..erased.end);
        }
    }
    Some(merge_disjoint_erasure_ranges(source, &sub_ranges, erased.start))
}

fn trim_trailing_cell_delimiters(source: &str, min_start: usize, cell_end: usize) -> usize {
    let mut end = cell_end.min(source.len());
    let bytes = source.as_bytes();
    while end > min_start && matches!(bytes[end - 1], b' ' | b'\t' | b'\r' | b'\n' | b'|') {
        end -= 1;
    }
    end
}

fn merge_disjoint_erasure_ranges(
    source: &str,
    sub_ranges: &[std::ops::Range<usize>],
    fallback_cursor: usize,
) -> EditAugmentation {
    let Some(first) = sub_ranges.first() else {
        return consume_edit(fallback_cursor);
    };
    let last = sub_ranges.last().expect("non-empty sub_ranges has last");
    let mut preserved = String::new();
    let mut previous_end = first.end;
    for next in &sub_ranges[1..] {
        preserved.push_str(&source[previous_end..next.start]);
        previous_end = next.end;
    }
    let augmentation = EditAugmentation {
        replace_range: Some(first.start..last.end),
        insert_text: Some(preserved),
        cursor_byte_after: first.start,
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

const MAX_INLINE_HTML_BREAK_LEN: usize = 16;

fn cursor_preceded_by_inline_opening(source: &str, cursor: usize) -> bool {
    let Some(prev) = source.as_bytes().get(cursor.wrapping_sub(1)).copied() else {
        return false;
    };
    matches!(prev, b'*' | b'_' | b'~' | b'[' | b'`')
        || (prev == b' ' && source[..cursor].trim_end_matches(' ').ends_with('`'))
}

fn cursor_followed_by_inline_closing(source: &str, cursor: usize) -> bool {
    let Some(next) = source.as_bytes().get(cursor).copied() else {
        return false;
    };
    matches!(next, b'*' | b'_' | b'~' | b']' | b'`')
        || (next == b' ' && source[cursor..].trim_start_matches(' ').starts_with('`'))
}

pub(in crate::augmenter) fn inline_html_break_ending_at(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<std::ops::Range<usize>> {
    if !cfg!(feature = "rich-markdown") {
        return None;
    }
    let prefix = source.get(..current_byte)?;
    if !prefix.ends_with('>') {
        return None;
    }
    let scan_start = prefix.len().saturating_sub(MAX_INLINE_HTML_BREAK_LEN);
    let open_rel = prefix.as_bytes()[scan_start..].iter().rposition(|&byte| byte == b'<')?;
    let break_start = scan_start + open_rel;
    if !crate::parser::is_inline_html_break(&source[break_start..current_byte]) {
        return None;
    }
    let candidate = break_start..current_byte;
    is_inline_html_break_range(source, candidate.clone()).then_some(candidate)
}

pub(in crate::augmenter) fn inline_html_break_starting_at(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<std::ops::Range<usize>> {
    if !cfg!(feature = "rich-markdown") {
        return None;
    }
    let suffix = source.get(current_byte..)?;
    if !suffix.starts_with('<') {
        return None;
    }
    let scan_end = suffix.len().min(MAX_INLINE_HTML_BREAK_LEN);
    let close_rel = suffix.as_bytes()[..scan_end].iter().position(|&byte| byte == b'>')?;
    let break_end = current_byte + close_rel + 1;
    if !crate::parser::is_inline_html_break(&source[current_byte..break_end]) {
        return None;
    }
    let candidate = current_byte..break_end;
    is_inline_html_break_range(source, candidate.clone()).then_some(candidate)
}

pub(in crate::augmenter) fn delete_forward_remove_inline_html_break(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let break_range = inline_html_break_starting_at(source, current_byte)?;
    let augmentation = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(break_range),
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(in crate::augmenter) fn delete_forward_join_reopened_inline_elements(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    if !cursor_followed_by_inline_closing(source, current_byte) {
        return None;
    }
    let frames = inline_wrapper_frames(source);
    let mut previous_frames = frames
        .iter()
        .filter(|frame| {
            frame.content_end == current_byte
                && frame.source_range.start < current_byte
                && frame.source_range.end > current_byte
        })
        .collect::<Vec<_>>();
    previous_frames.sort_by_key(|frame| frame.source_range.start);
    let previous_outer = previous_frames.first()?;
    let mut boundary_start = previous_outer.source_range.end;
    let separator = if source.as_bytes().get(boundary_start) == Some(&b' ') {
        boundary_start += 1;
        " "
    } else {
        ""
    };

    let (next_line_end, opening_start) = if let Some(br_range) =
        inline_html_break_starting_at(source, boundary_start)
    {
        let opening_start = br_range.end;
        let (_, _, line_end) = locate_source_line_bounds(source, opening_start)?;
        (line_end, opening_start)
    } else {
        let next_line_start =
            if let Some(hard_break) = hard_break_boundary_after(source, boundary_start) {
                hard_break.end
            } else {
                let mut probe = boundary_start;
                let mut newline_count = 0;
                while let Some(newline_width) = newline_sequence_width_at(source, probe) {
                    probe += newline_width;
                    newline_count += 1;
                }
                if newline_count == 0 {
                    return None;
                }
                probe
            };
        let (_, _, next_line_end) = locate_source_line_bounds(source, next_line_start)?;
        let opening_start = container_content_start_on_line(source, next_line_start, next_line_end);
        (next_line_end, opening_start)
    };
    let mut current_frames = frames
        .iter()
        .filter(|frame| {
            frame.source_range.start >= opening_start
                && frame.source_range.start < next_line_end
                && frame.source_range.start < frame.content_start
                && frame.content_start > opening_start
                && frame.content_start <= next_line_end
        })
        .collect::<Vec<_>>();
    current_frames.sort_by_key(|frame| frame.source_range.start);
    let current_outer = current_frames.first()?;
    if current_outer.source_range.start != opening_start
        || current_frames.iter().any(|frame| frame.content_start != current_outer.content_start)
        || !inline_paths_match(&previous_frames, &current_frames)
    {
        return None;
    }

    let augmentation = EditAugmentation {
        insert_text: Some(separator.to_owned()),
        replace_range: Some(current_byte..current_outer.content_start),
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

fn delete_forward_join_hard_break_line(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let boundary = hard_break_boundary_after(source, current_byte)?;
    if boundary.start != current_byte {
        return None;
    }
    if boundary.end < source.len() && newline_sequence_width_at(source, boundary.end).is_some() {
        return None;
    }
    let (cur_line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    let current_content_start =
        container_content_start_on_line(source, cur_line_start, current_byte);
    let current_prefix = canonical_container_prefix(source, current_content_start);
    if boundary.end < source.len() {
        let (_, _, next_line_end) = locate_source_line_bounds(source, boundary.end)?;
        let next_is_blank = blank_source_line_width(source, boundary.end).is_some();
        let is_terminal_container_continuation =
            next_line_end == source.len() && !current_prefix.is_empty();
        if next_is_blank && !is_terminal_container_continuation {
            return None;
        }
    }

    let context = classify_source(source, current_byte);
    if matches!(
        context,
        EnterContext::CodeBlock
            | EnterContext::IndentedCodeBlock { .. }
            | EnterContext::CodeBlockFenceLine { .. }
            | EnterContext::LiteralBlockLine { .. }
            | EnterContext::Heading { .. }
            | EnterContext::SetextHeading { .. }
            | EnterContext::TableCell { .. }
            | EnterContext::HorizontalRuleEnd
    ) {
        return None;
    }

    if boundary.end == source.len() {
        let aug = EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: Some(boundary),
            cursor_byte_after: current_byte,
        };
        debug_assert_augmentation(&aug, source);
        return Some(aug);
    }

    let next_line_start = boundary.end;
    let (_, _, next_line_end) = locate_source_line_bounds(source, next_line_start)?;
    let next_content_end = source_line_content_end(source, next_line_end);
    let container_path = container_path(&current_prefix);
    if line_starts_new_sibling_block(source, next_line_start)
        || line_starts_independent_block(source, next_line_start, &container_path)
    {
        return Some(consume_edit(current_byte));
    }

    let next_prefix_width = source[next_line_start..next_content_end]
        .bytes()
        .take_while(|&byte| matches!(byte, b' ' | b'\t' | b'>'))
        .count();
    let next_prefix_end = next_line_start + next_prefix_width;
    let next_joinable_prefix = &source[next_line_start..next_prefix_end];
    if !continuation_prefix_matches_prefix(Some(&current_prefix), next_joinable_prefix) {
        return Some(consume_edit(current_byte));
    }
    let delete_end = next_prefix_end.min(next_content_end);
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(boundary.start..delete_end),
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

/// Delete(前向删除)的块边界护栏,与 Backspace 侧的
/// [`backspace_paragraph_boundary`] 对称。
///
/// 光标位于非空源码行内容末尾、紧邻换行 run 时:
/// - 下一物理行自成独立块(ATX/围栏/HR/列表 marker/setext 下划线/引用/表格行),
///   默认逐字符删除会把两行并线、破坏结构(如段落成 setext 标题)——
///   返回消费型空操作(不删任何字节);
/// - 当前块为围栏/缩进代码块时,只有下一行是闭合围栏才拦截,
///   普通代码行允许默认删除;
/// - 段落/标题末尾且下一行是普通段落文本:删除整个换行 run,合并两段
///   (与段首 Backspace 删整个边界对称)。
pub(in crate::augmenter) fn delete_forward_block_boundary(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let boundary_end = text_block_source_end(source, current_byte)?;
    let consume_boundary = || {
        Some(EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: None,
            cursor_byte_after: current_byte,
        })
    };
    if boundary_end == source.len() && current_byte != boundary_end {
        return consume_boundary();
    }
    let context = classify_source(source, boundary_end);
    // ATX 标题尾部空白不产生段内硬换行。
    if !matches!(
        context,
        EnterContext::Heading { .. }
            | EnterContext::CodeBlock
            | EnterContext::IndentedCodeBlock { .. }
            | EnterContext::CodeBlockFenceLine { .. }
    ) && hard_break_marker_ending_at(source, boundary_end).is_some()
    {
        return None;
    }
    let first_newline_width = newline_sequence_width_at(source, boundary_end)?;
    let mut newline_run_end = boundary_end + first_newline_width;
    while let Some(newline_width) = newline_sequence_width_at(source, newline_run_end) {
        newline_run_end += newline_width;
    }
    if let Some(separator) = editable_paragraph_navigation::hidden_separator_range(
        source,
        boundary_end,
        editable_paragraph_navigation::Boundary::End,
    ) {
        newline_run_end = separator.end;
    }
    if newline_run_end >= source.len() {
        return None;
    }

    let container_path = delete_context_container_path(source, boundary_end, &context);
    match context {
        EnterContext::CodeBlock => {
            if newline_run_end == boundary_end + first_newline_width {
                if line_closes_enclosing_fenced_block(source, boundary_end, newline_run_end) {
                    return consume_boundary();
                }
                return delete_forward_code_row_join(source, boundary_end, newline_run_end);
            }
            None
        }
        EnterContext::CodeBlockFenceLine { .. } | EnterContext::HorizontalRuleEnd => {
            // 围栏行与相邻行并线必然破坏围栏;仅在边界恰好一个换行序列时拦截,
            // 有空行兜底(≥2 个换行)时默认删除一个仍保持结构合法。
            if newline_run_end == boundary_end + first_newline_width {
                return consume_boundary();
            }
            None
        }
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::ParagraphStart { .. }
        | EnterContext::Heading { .. } => {
            if line_starts_independent_block(source, newline_run_end, &container_path) {
                return consume_boundary();
            }
            let delete_start = if matches!(context, EnterContext::Heading { .. }) {
                atx_heading_merge_end(source, boundary_end)
            } else {
                boundary_end
            };
            let delete_end = delete_forward_folded_leading_whitespace_end(
                source,
                newline_run_end,
                matches!(context, EnterContext::Heading { .. }),
            );
            let augmentation = EditAugmentation {
                insert_text: Some(String::new()),
                replace_range: Some(delete_start..delete_end),
                cursor_byte_after: current_byte.min(delete_start),
            };
            debug_assert_augmentation(&augmentation, source);
            Some(augmentation)
        }
        EnterContext::BlockQuoteLine { .. } | EnterContext::ListItem { .. } => {
            if line_starts_independent_block(source, newline_run_end, &container_path) {
                return consume_boundary();
            }
            let (_, _, next_line_end) = locate_source_line_bounds(source, newline_run_end)?;
            let next_content_end = source_line_content_end(source, next_line_end);
            let container_content = matching_container_position(
                source,
                newline_run_end,
                next_content_end,
                &container_path,
            )
            .map_or(newline_run_end, |position| position.byte_offset);
            let delete_end =
                delete_forward_folded_leading_whitespace_end(source, container_content, false);
            let augmentation = EditAugmentation {
                insert_text: Some(String::new()),
                replace_range: Some(boundary_end..delete_end),
                cursor_byte_after: current_byte.min(boundary_end),
            };
            debug_assert_augmentation(&augmentation, source);
            Some(augmentation)
        }
        _ => {
            if line_starts_independent_block(source, newline_run_end, &container_path) {
                return consume_boundary();
            }
            None
        }
    }
}

fn delete_forward_folded_leading_whitespace_end(
    source: &str,
    newline_run_end: usize,
    from_heading: bool,
) -> usize {
    let Some((_line_start, _, line_end)) = locate_source_line_bounds(source, newline_run_end)
    else {
        return newline_run_end;
    };
    let content_end = source_line_content_end(source, line_end);
    let Some(offset) = source[newline_run_end..content_end]
        .find(|character: char| !matches!(character, ' ' | '\t'))
    else {
        return newline_run_end;
    };
    let after_spaces = newline_run_end + offset;
    if from_heading && markdown_column_after(&source[newline_run_end..after_spaces], 0) >= 4 {
        return newline_run_end;
    }
    after_spaces
}

/// Resolve a visible text endpoint to the end of its complete source line.
pub(in crate::augmenter) fn text_block_source_end(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<usize> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let boundary_end = source_line_content_end(source, line_end);
    if line_start == current_byte {
        return None;
    }
    if current_byte != boundary_end {
        if source.get(current_byte..boundary_end)?.chars().next().is_some_and(char::is_alphanumeric)
        {
            return None;
        }
        let document = source.structure();
        if !editable_paragraph_navigation::has_text_boundary(
            &document.blocks,
            current_byte,
            editable_paragraph_navigation::Boundary::End,
        ) {
            return None;
        }
    }
    Some(boundary_end)
}

/// Remove only the optional ATX closing sequence when serializing a joined heading.
pub(in crate::augmenter) fn atx_heading_merge_end(source: &str, line_end: usize) -> usize {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, line_end) else {
        return line_end;
    };
    let marker_start = container_content_start_on_line(source, line_start, line_end);
    let line = &source[marker_start..line_end];
    let trimmed = line.trim_end_matches([' ', '\t']);
    let before_hashes = trimmed.trim_end_matches('#');
    if before_hashes.len() == trimmed.len() || !before_hashes.ends_with([' ', '\t']) {
        return line_end;
    }
    let opening_end = line.len() - line.trim_start_matches('#').len();
    let after_opening = &line[opening_end..];
    let content_start =
        opening_end + after_opening.len() - after_opening.trim_start_matches([' ', '\t']).len();
    let closing_start = before_hashes.trim_end_matches([' ', '\t']).len();
    marker_start + closing_start.max(content_start)
}

pub(in crate::augmenter) fn delete_context_container_path(
    source: &str,
    current_byte: usize,
    context: &EnterContext,
) -> ContainerPath {
    let prefix = match context {
        EnterContext::Heading { continuation_prefix, .. }
        | EnterContext::SetextHeading { continuation_prefix, .. }
        | EnterContext::BlockQuoteLine { continuation_prefix, .. }
        | EnterContext::IndentedCodeBlock { continuation_prefix }
        | EnterContext::LiteralBlockLine { continuation_prefix } => continuation_prefix,
        EnterContext::ListItem { content_prefix, .. } => content_prefix,
        _ => "",
    };
    let base_path = container_path(prefix);
    let Some((line_start, _, _)) = locate_source_line_bounds(source, current_byte) else {
        return base_path;
    };
    source_container_path(source, line_start, current_byte, &base_path)
}

/// Delete 侧判定:下一物理行是否自成独立块,默认前向删除并线后会破坏结构。
/// 在 [`line_starts_new_sibling_block`] 基础上补充 setext 下划线、
/// 引用行(`>`)与表格行(`|`);后两者可被前一块合法中断,因此不并入
/// `line_starts_new_sibling_block`,避免影响 Backspace 侧既有行为。
pub(in crate::augmenter) fn line_starts_independent_block(
    source: &str,
    line_start: usize,
    container_path: &ContainerPath,
) -> bool {
    let Some(marker_line) = block_marker_line(source, line_start, container_path)
        .or_else(|| block_marker_line(source, line_start, &ContainerPath::default()))
    else {
        return false;
    };
    line_starts_new_sibling_marker(source, &marker_line)
        || matches!(source.as_bytes().get(marker_line.marker_start), Some(b'>' | b'|'))
        || content_is_setext_underline(marker_line.content)
}

pub(in crate::augmenter) struct BlockMarkerLine<'a> {
    pub(in crate::augmenter) marker_start: usize,
    pub(in crate::augmenter) content: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::augmenter) struct LineColumnPosition {
    pub(in crate::augmenter) byte_offset: usize,
    /// 候选物理行按 CommonMark tab stop 展开后的绝对列。
    pub(in crate::augmenter) absolute_column: usize,
    /// 当前容器路径要求到达的绝对列；两列之差是跨界 Tab 留给叶块的虚拟缩进。
    pub(in crate::augmenter) container_target_column: usize,
}

impl LineColumnPosition {
    pub(in crate::augmenter) fn at_line_start(line_start: usize) -> Self {
        Self { byte_offset: line_start, absolute_column: 0, container_target_column: 0 }
    }

    pub(in crate::augmenter) fn at_byte(
        source: &str,
        line_start: usize,
        byte_offset: usize,
    ) -> Option<Self> {
        let prefix = source.get(line_start..byte_offset)?;
        let absolute_column = markdown_column_after(prefix, 0);
        Some(Self { byte_offset, absolute_column, container_target_column: absolute_column })
    }

    pub(in crate::augmenter) fn leaf_indent_columns(self) -> Option<usize> {
        self.absolute_column.checked_sub(self.container_target_column)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::augmenter) struct ContainerPath(pub(in crate::augmenter) Vec<ContainerSegment>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::augmenter) enum ContainerSegment {
    BlockQuote,
    IndentColumns(usize),
}

/// 先精确消费容器必需续行前缀，再定位至多 3 空格缩进的块 marker。
pub(in crate::augmenter) fn block_marker_line<'a>(
    source: &'a str,
    line_start: usize,
    container_path: &ContainerPath,
) -> Option<BlockMarkerLine<'a>> {
    let (actual_line_start, _, line_end) = locate_source_line_bounds(source, line_start)?;
    if actual_line_start != line_start {
        return None;
    }
    let bytes = source.as_bytes();
    let content_end = source_line_content_end(source, line_end);
    let container_position =
        matching_container_position(source, line_start, content_end, container_path)?;
    let marker_position =
        marker_position_after_optional_indent(source, content_end, container_position)?;
    let marker_start = marker_position.byte_offset;
    let mut marker_end = content_end;
    while marker_end > marker_start && matches!(bytes[marker_end - 1], b' ' | b'\t') {
        marker_end -= 1;
    }
    Some(BlockMarkerLine { marker_start, content: source.get(marker_start..marker_end)? })
}

pub(in crate::augmenter) fn content_is_setext_underline(content: &str) -> bool {
    let Some(marker) = content.as_bytes().first().copied() else {
        return false;
    };
    matches!(marker, b'=' | b'-') && content.bytes().all(|byte| byte == marker)
}

pub(in crate::augmenter) fn content_is_thematic_break(content: &str) -> bool {
    let mut marker = None;
    let mut marker_count = 0;
    for byte in content.bytes() {
        if matches!(byte, b' ' | b'\t') {
            continue;
        }
        if !matches!(byte, b'*' | b'-' | b'_') || marker.is_some_and(|marker| marker != byte) {
            return false;
        }
        marker = Some(byte);
        marker_count += 1;
    }
    marker_count >= 3
}

pub(in crate::augmenter) fn backspace_remove_inline_html_break(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let break_range = inline_html_break_ending_at(source, current_byte)?;
    let cursor_after = break_range.start;
    let augmentation = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(break_range),
        cursor_byte_after: cursor_after,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(in crate::augmenter) fn is_inline_html_break_range(
    source: &MarkdownSource<'_>,
    candidate_range: std::ops::Range<usize>,
) -> bool {
    if !cfg!(feature = "rich-markdown") {
        return false;
    }
    let parsed = source.parsed();
    parsed.events.iter().zip(&parsed.event_ranges).any(|(event, event_range)| {
        matches!(
            event,
            crate::parser::MarkdownEvent::InlineHtml(html)
                if crate::parser::is_inline_html_break(html)
        ) && *event_range == candidate_range
    })
}

pub(in crate::augmenter) fn backspace_join_reopened_inline_elements(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    if !cursor_preceded_by_inline_opening(source, current_byte) {
        return None;
    }
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    if current_byte == line_start {
        return None;
    }

    let frames = inline_wrapper_frames(source);
    let mut current_frames = frames
        .iter()
        .filter(|frame| {
            frame.content_start == current_byte
                && frame.source_range.start >= line_start
                && frame.source_range.start < current_byte
        })
        .collect::<Vec<_>>();
    current_frames.sort_by_key(|frame| frame.source_range.start);
    let current_outer = current_frames.first()?;
    let opening_start = current_outer.source_range.start;

    let (previous_wrapper_end, separator) =
        if let Some(br_range) = inline_html_break_ending_at(source, opening_start) {
            let mut prev_end = br_range.start;
            let sep = if source.as_bytes().get(prev_end.wrapping_sub(1)) == Some(&b' ') {
                prev_end -= 1;
                " "
            } else {
                ""
            };
            (prev_end, sep)
        } else {
            if container_content_start_on_line(source, line_start, line_end) != opening_start {
                return None;
            }
            let mut prev_end = line_start;
            let mut newline_count = 0;
            while let Some(newline_width) = newline_sequence_width_before(source, prev_end) {
                prev_end -= newline_width;
                newline_count += 1;
            }
            if newline_count == 0 {
                return None;
            }
            if let Some(marker) = hard_break_marker_ending_at(source, prev_end) {
                prev_end = marker.start;
            }
            let sep = if source.as_bytes().get(prev_end.wrapping_sub(1)) == Some(&b' ') {
                prev_end -= 1;
                " "
            } else {
                ""
            };
            (prev_end, sep)
        };

    let previous_outer = frames
        .iter()
        .filter(|frame| frame.source_range.end == previous_wrapper_end)
        .min_by_key(|frame| frame.source_range.start)?;
    let closing_start = previous_outer.content_end;
    let mut previous_frames = frames
        .iter()
        .filter(|frame| {
            frame.content_end == closing_start
                && frame.source_range.start >= previous_outer.source_range.start
                && frame.source_range.end <= previous_outer.source_range.end
        })
        .collect::<Vec<_>>();
    previous_frames.sort_by_key(|frame| frame.source_range.start);

    if !inline_paths_match(&previous_frames, &current_frames) {
        return None;
    }

    let augmentation = EditAugmentation {
        insert_text: Some(separator.to_owned()),
        replace_range: Some(closing_start..current_byte),
        cursor_byte_after: closing_start + separator.len(),
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(in crate::augmenter) fn inline_paths_match(
    previous_frames: &[&InlineWrapperFrame<'_>],
    current_frames: &[&InlineWrapperFrame<'_>],
) -> bool {
    previous_frames.len() == current_frames.len()
        && previous_frames.iter().zip(current_frames).all(|(previous, current)| {
            if previous.kind != current.kind {
                return false;
            }
            if matches!(previous.kind, InlineFrameKind::Link | InlineFrameKind::Code) {
                return previous.opening == current.opening && previous.closing == current.closing;
            }
            true
        })
}

pub(in crate::augmenter) fn backspace_at_atx_heading_marker_start(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let line_content_end = source_line_content_end(source, line_end);
    let line = source.as_bytes().get(line_start..line_content_end)?;
    let leading_spaces =
        line.iter().take(MAX_LEADING_BLOCK_INDENT).take_while(|&&byte| byte == b' ').count();
    let marker_start = line_start + leading_spaces;
    if current_byte != marker_start {
        return None;
    }

    let marker_width = line[leading_spaces..].iter().take_while(|&&byte| byte == b'#').count();
    if !(1..=6).contains(&marker_width)
        || !matches!(line.get(leading_spaces + marker_width), None | Some(b' ' | b'\t'))
    {
        return None;
    }

    let augmentation = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: None,
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(in crate::augmenter) fn backspace_last_isolated_paragraph_grapheme(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    editable_paragraph_edit::erase_last_grapheme(source, current_byte)
}

pub(in crate::augmenter) fn newline_sequence_width_before(
    source: &str,
    byte: usize,
) -> Option<usize> {
    let prefix = source.as_bytes().get(..byte)?;
    if prefix.ends_with(b"\r\n") {
        return Some(2);
    }
    prefix.ends_with(b"\n").then_some(1)
}

pub(in crate::augmenter) fn newline_sequence_width_at(source: &str, byte: usize) -> Option<usize> {
    let suffix = source.as_bytes().get(byte..)?;
    if suffix.starts_with(b"\r\n") {
        return Some(2);
    }
    suffix.starts_with(b"\n").then_some(1)
}

/// 返回紧邻 `content_end` 结束的 Markdown 硬换行标记范围。
///
/// 空格形式消耗全部行尾空格。反斜杠形式按奇偶性解释，奇数个连续反斜杠中的
/// 最后一个是硬换行标记，前面的偶数个是转义后的正文。
pub(in crate::augmenter) fn hard_break_marker_ending_at(
    source: &str,
    content_end: usize,
) -> Option<std::ops::Range<usize>> {
    let prefix = source.as_bytes().get(..content_end)?;
    let trailing_backslash_count =
        prefix.iter().rev().take_while(|source_byte| **source_byte == b'\\').count();
    if !trailing_backslash_count.is_multiple_of(2) {
        return Some(content_end - 1..content_end);
    }

    let trailing_space_count =
        prefix.iter().rev().take_while(|source_byte| **source_byte == b' ').count();
    (trailing_space_count >= HARD_BREAK_MIN_SPACES)
        .then_some(content_end - trailing_space_count..content_end)
}

/// 光标位于硬换行源码标记之前时，返回应升级为块边界的完整源码范围。
///
/// 对奇数个连续反斜杠只替换最后一个标记及换行，保留前面的转义反斜杠。
pub(in crate::augmenter) fn hard_break_boundary_after(
    source: &str,
    current_byte: usize,
) -> Option<std::ops::Range<usize>> {
    let suffix = source.as_bytes().get(current_byte..)?;
    let trailing_backslash_count =
        suffix.iter().take_while(|source_byte| **source_byte == b'\\').count();
    if trailing_backslash_count > 0 {
        if trailing_backslash_count.is_multiple_of(2) {
            return None;
        }
        let marker_start = current_byte + trailing_backslash_count - 1;
        let newline_start = marker_start + 1;
        let newline_width = newline_sequence_width_at(source, newline_start)?;
        return Some(marker_start..newline_start + newline_width);
    }

    let trailing_space_count =
        suffix.iter().take_while(|source_byte| **source_byte == b' ').count();
    if trailing_space_count < HARD_BREAK_MIN_SPACES {
        return None;
    }
    let newline_start = current_byte + trailing_space_count;
    let newline_width = newline_sequence_width_at(source, newline_start)?;
    Some(current_byte..newline_start + newline_width)
}

pub(in crate::augmenter) fn preferred_newline_sequence(
    source: &str,
    current_byte: usize,
) -> &'static str {
    let bytes = source.as_bytes();
    let cursor = current_byte.min(bytes.len());
    let prefix = &bytes[..cursor];
    let suffix = &bytes[cursor..];

    if suffix.starts_with(CRLF_SEQUENCE.as_bytes())
        || prefix.ends_with(CRLF_SEQUENCE.as_bytes())
        || (prefix.ends_with(b"\r") && suffix.starts_with(b"\n"))
    {
        return CRLF_SEQUENCE;
    }
    if suffix.starts_with(LF_SEQUENCE.as_bytes()) || prefix.ends_with(LF_SEQUENCE.as_bytes()) {
        return LF_SEQUENCE;
    }

    let Some(first_line_feed) = bytes.iter().position(|source_byte| *source_byte == b'\n') else {
        return LF_SEQUENCE;
    };
    if first_line_feed > 0 && bytes[first_line_feed - 1] == b'\r' {
        CRLF_SEQUENCE
    } else {
        LF_SEQUENCE
    }
}
