use super::*;

// ─── 光标位置附近的源码切分 ───────────────────────────────────────────────

pub(in crate::augmenter) fn get_marker_delete_range(
    source: &str,
    current_byte: usize,
) -> Option<std::ops::Range<usize>> {
    let (line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    if line_start == current_byte {
        return None;
    }
    let prefix = &source[line_start..current_byte];
    let trimmed = prefix.trim_start_matches(' ');

    let is_match = match trimmed {
        _ if trimmed.starts_with('>') => {
            trimmed.chars().all(|character| matches!(character, '>' | ' ' | '\t'))
        }
        _ if trimmed.starts_with('#') => {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            hashes <= 6 && hashes > 0 && marker_separator_is_complete(&trimmed[hashes..])
        }
        _ if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with('+') => {
            unordered_marker_suffix_is_complete(&trimmed[1..])
        }
        _ => {
            let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
            digits > 0 && {
                let rest = &trimmed[digits..];
                rest.strip_prefix(['.', ')']).is_some_and(marker_separator_is_complete)
            }
        }
    };

    if is_match { Some(line_start..current_byte) } else { None }
}

pub(in crate::augmenter) fn marker_separator_is_complete(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
}

pub(in crate::augmenter) fn unordered_marker_suffix_is_complete(suffix: &str) -> bool {
    let separator_end = suffix.bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
    if separator_end == 0 {
        return false;
    }
    let after_separator = &suffix[separator_end..];
    if after_separator.is_empty() {
        return true;
    }

    ["[ ]", "[x]", "[X]"].into_iter().any(|task_marker| {
        after_separator.strip_prefix(task_marker).is_some_and(|trailing| {
            trailing.is_empty() || trailing.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
        })
    })
}

pub(in crate::augmenter) fn list_continuation_marker(
    source: &str,
    marker_start: usize,
    marker_end: usize,
    bullet: ListBullet,
) -> String {
    let source_marker = &source[marker_start..marker_end];
    match bullet {
        ListBullet::Bullet => source_marker.to_owned(),
        ListBullet::Ordered(number) => {
            let suffix_start = source_marker.bytes().take_while(u8::is_ascii_digit).count();
            format!("{}{}", number + 1, &source_marker[suffix_start..])
        }
        ListBullet::TaskList(checked) => {
            let mut continuation_marker = if checked {
                source_marker.replacen("[x]", "[ ]", 1).replacen("[X]", "[ ]", 1)
            } else {
                source_marker.to_owned()
            };
            if !matches!(continuation_marker.as_bytes().last(), Some(b' ' | b'\t')) {
                continuation_marker.push(TASK_LIST_CONTENT_SEPARATOR);
            }
            continuation_marker
        }
    }
}

pub(in crate::augmenter) fn locate_source_line_bounds(
    source: &str,
    byte: usize,
) -> Option<(usize, usize, usize)> {
    let bytes = source.as_bytes();
    let start = bytes[..byte].iter().rposition(|&b| b == b'\n').map(|p| p + 1).unwrap_or(0);
    let end =
        bytes[byte..].iter().position(|&b| b == b'\n').map(|p| byte + p).unwrap_or(source.len());
    Some((start, byte, end))
}

pub(crate) fn parse_list_marker(source: &str, marker_start: usize) -> Option<(ListBullet, usize)> {
    let bytes = source.as_bytes();
    if matches!(bytes.get(marker_start), Some(b'-' | b'+' | b'*')) {
        let content_start = list_marker_separator_end(source, marker_start + 1)?;

        let task_start = content_start;
        let Some(task_marker) = bytes.get(task_start..task_start + 3) else {
            return Some((ListBullet::Bullet, content_start));
        };
        let checked = match task_marker {
            b"[ ]" => false,
            b"[x]" | b"[X]" => true,
            _ => return Some((ListBullet::Bullet, content_start)),
        };
        let task_content_start =
            list_marker_separator_end(source, task_start + 3).unwrap_or(task_start + 3);
        return Some((ListBullet::TaskList(checked), task_content_start));
    }

    let mut marker_end = marker_start;
    while let Some(&b) = bytes.get(marker_end) {
        if b.is_ascii_digit() {
            marker_end += 1;
        } else {
            break;
        }
    }
    let digit_count = marker_end - marker_start;
    if !(1..=9).contains(&digit_count) || !matches!(bytes.get(marker_end), Some(b'.' | b')')) {
        return None;
    }

    let number = source[marker_start..marker_end].parse::<u64>().ok()?;
    let content_start = list_marker_separator_end(source, marker_end + 1)?;
    Some((ListBullet::Ordered(number), content_start))
}

pub(in crate::augmenter) fn list_marker_separator_end(
    source: &str,
    separator_start: usize,
) -> Option<usize> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, separator_start)?;
    let marker_column = markdown_column_after(&source[line_start..separator_start], 0);
    let maximum_content_column = marker_column + MARKDOWN_TAB_STOP_WIDTH;
    let mut column = marker_column;
    let mut probe = separator_start;

    while probe < line_end {
        let next_column = match source.as_bytes()[probe] {
            b' ' => column + 1,
            b'\t' => next_markdown_tab_stop(column),
            _ => break,
        };
        if next_column > maximum_content_column {
            break;
        }
        column = next_column;
        probe += 1;
    }

    (probe > separator_start).then_some(probe)
}

pub(in crate::augmenter) fn list_item_content_prefix(
    source: &str,
    marker_start: usize,
    content_start: usize,
) -> String {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, marker_start) else {
        return String::new();
    };
    let indent = &source[line_start..marker_start];
    let marker_start_column = markdown_column_after(indent, 0);
    let content_column =
        markdown_column_after(&source[marker_start..content_start], marker_start_column);
    format!("{indent}{}", " ".repeat(content_column - marker_start_column))
}

pub(in crate::augmenter) fn markdown_column_after(text: &str, starting_column: usize) -> usize {
    text.bytes().fold(starting_column, |column, byte| {
        if byte == b'\t' { next_markdown_tab_stop(column) } else { column + 1 }
    })
}

pub(in crate::augmenter) fn next_markdown_tab_stop(column: usize) -> usize {
    (column / MARKDOWN_TAB_STOP_WIDTH + 1) * MARKDOWN_TAB_STOP_WIDTH
}

pub(crate) fn list_item_indent(source: &str, marker_start: usize) -> String {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, marker_start) else {
        return String::new();
    };
    source[line_start..marker_start].to_string()
}

/// 返回叶块所在行的容器续行前缀。
///
/// 引用 marker 原样保留；列表 marker 改写为等宽空格，使新行留在原列表项内，
/// 而不会意外创建同级列表项。该表示可组合处理 `> - `、`- > ` 等嵌套路径。
pub(in crate::augmenter) fn canonical_container_prefix(source: &str, leaf_start: usize) -> String {
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, leaf_start) else {
        return String::new();
    };
    let prefix_end = leaf_start.min(line_end);
    let mut prefix = String::with_capacity(prefix_end.saturating_sub(line_start));
    let mut probe = line_start;

    while probe < prefix_end {
        if let Some((_, content_start)) = parse_list_marker(source, probe)
            && content_start <= prefix_end
        {
            let marker_start_column = markdown_column_after(&prefix, 0);
            let content_column =
                markdown_column_after(&source[probe..content_start], marker_start_column);
            prefix.extend(std::iter::repeat_n(' ', content_column - marker_start_column));
            probe = content_start;
            continue;
        }

        let Some(character) = source[probe..prefix_end].chars().next() else {
            break;
        };
        prefix.push(character);
        probe += character.len_utf8();
    }

    prefix
}

pub(in crate::augmenter) fn blockquote_parent_prefix(source: &str, content_start: usize) -> String {
    let Some((line_start, _, _)) = locate_source_line_bounds(source, content_start) else {
        return String::new();
    };
    let structural_prefix = &source[line_start..content_start];
    let Some(last_quote_offset) = structural_prefix.rfind('>') else {
        return String::new();
    };
    structural_prefix[..last_quote_offset].to_owned()
}

pub(in crate::augmenter) fn locate_blockquote_line(
    source: &str,
    byte: usize,
) -> Option<(usize, usize, usize)> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, byte)?;
    let bytes = source.as_bytes();
    let leading_spaces =
        bytes[line_start..line_end].iter().take_while(|source_byte| **source_byte == b' ').count();
    if leading_spaces > MAX_LEADING_BLOCK_INDENT {
        return None;
    }
    let mut content_start = line_start + leading_spaces;
    let mut innermost_marker_is_quote = false;

    loop {
        if let Some((_, list_content_start)) = parse_list_marker(source, content_start)
            && list_content_start <= line_end
        {
            innermost_marker_is_quote = false;
            content_start = list_content_start;
            continue;
        }

        if bytes.get(content_start) == Some(&b'>') {
            innermost_marker_is_quote = true;
            content_start += 1;
            if matches!(bytes.get(content_start), Some(b' ' | b'\t')) {
                content_start += 1;
            }
            continue;
        }

        break;
    }

    innermost_marker_is_quote.then_some((line_start, content_start, line_end))
}

struct ActiveListItemContext {
    outer_quote_depth: usize,
    item_range: std::ops::Range<usize>,
    marker_start: usize,
    prev_sibling_marker_start: Option<usize>,
    parent_marker_start: Option<usize>,
}

enum SubtreeIndentChange {
    Indent(usize),
    Outdent(usize),
}

enum ListIndentAstFrame {
    BlockQuote,
    List {
        outer_quote_depth: usize,
        prev_item_marker_start: Option<usize>,
    },
    Item {
        outer_quote_depth: usize,
        marker_start: Option<usize>,
        prev_sibling_marker_start: Option<usize>,
        parent_marker_start: Option<usize>,
    },
}

pub(in crate::augmenter) fn augment_list_item_tab(
    source: &str,
    current_byte: usize,
    reverse: bool,
) -> Option<EditAugmentation> {
    let context = find_active_list_item_context(source, current_byte)?;
    let (first_line_start, _, _) = locate_source_line_bounds(source, context.item_range.start)?;
    let first_line_end = locate_source_line_bounds(source, context.marker_start)?.2;
    let first_origin = first_line_start
        + line_indent_origin(&source[first_line_start..first_line_end], context.outer_quote_depth);
    let curr_marker_col = markdown_column_after(&source[first_origin..context.marker_start], 0);

    if reverse {
        let target_marker_col = parent_item_marker_column(
            source,
            context.parent_marker_start,
            context.outer_quote_depth,
        );
        let dedent_cols = curr_marker_col.saturating_sub(target_marker_col);
        if dedent_cols == 0 {
            return Some(consume_edit(current_byte));
        }
        return Some(rewrite_list_item_subtree_indent(
            source,
            current_byte,
            &context.item_range,
            context.outer_quote_depth,
            SubtreeIndentChange::Outdent(dedent_cols),
        ));
    }

    let Some(prev_marker_start) = context.prev_sibling_marker_start else {
        return Some(consume_edit(current_byte));
    };
    let (prev_line_start, _, prev_line_end) = locate_source_line_bounds(source, prev_marker_start)?;
    let prev_origin = prev_line_start
        + line_indent_origin(&source[prev_line_start..prev_line_end], context.outer_quote_depth);
    let prev_content_start = list_item_structural_content_start(source, prev_marker_start)?;
    let prev_content_col = markdown_column_after(&source[prev_origin..prev_content_start], 0);
    let delta_cols = prev_content_col.saturating_sub(curr_marker_col);
    if delta_cols == 0 {
        return Some(consume_edit(current_byte));
    }
    Some(rewrite_list_item_subtree_indent(
        source,
        current_byte,
        &context.item_range,
        context.outer_quote_depth,
        SubtreeIndentChange::Indent(delta_cols),
    ))
}

fn parent_item_marker_column(
    source: &str,
    parent_marker_start: Option<usize>,
    outer_quote_depth: usize,
) -> usize {
    let Some(parent_start) = parent_marker_start else {
        return 0;
    };
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, parent_start) else {
        return 0;
    };
    let origin = line_start + line_indent_origin(&source[line_start..line_end], outer_quote_depth);
    markdown_column_after(&source[origin..parent_start.max(origin)], 0)
}

fn find_active_list_item_context(
    source: &str,
    current_byte: usize,
) -> Option<ActiveListItemContext> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut quote_depth = 0usize;
    let mut frames: Vec<ListIndentAstFrame> = Vec::new();
    let mut protected_block_starts: Vec<usize> = Vec::new();
    let mut matched: Option<ActiveListItemContext> = None;

    for (event, range) in
        Parser::new_ext(source, crate::parser::markdown_options()).into_offset_iter()
    {
        match event {
            Event::Start(Tag::BlockQuote(_)) => {
                quote_depth += 1;
                frames.push(ListIndentAstFrame::BlockQuote);
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                pop_quote_frame(&mut frames, &mut quote_depth);
            }
            Event::Start(Tag::List(_)) => {
                frames.push(ListIndentAstFrame::List {
                    outer_quote_depth: quote_depth,
                    prev_item_marker_start: None,
                });
            }
            Event::End(TagEnd::List(_)) => {
                pop_list_frame(&mut frames);
            }
            Event::Start(Tag::Item) => {
                push_item_frame(source, &mut frames, quote_depth, range.start);
            }
            Event::End(TagEnd::Item) => {
                finish_item_frame(source, &mut frames, range, current_byte, &mut matched);
            }
            Event::Start(Tag::CodeBlock(_) | Tag::Table(_)) => {
                let start = locate_source_line_bounds(source, range.start)
                    .map_or(range.start, |(line_start, _, _)| line_start);
                protected_block_starts.push(start);
            }
            Event::End(TagEnd::CodeBlock | TagEnd::Table) => {
                if let Some(block_start) = protected_block_starts.pop() {
                    let content_end =
                        content_end_without_trailing_newline(source, block_start..range.end);
                    if current_byte >= block_start && current_byte <= content_end {
                        return None;
                    }
                }
            }
            _ => {}
        }
    }
    matched
}

fn pop_quote_frame(frames: &mut Vec<ListIndentAstFrame>, quote_depth: &mut usize) {
    if let Some(pos) =
        frames.iter().rposition(|frame| matches!(frame, ListIndentAstFrame::BlockQuote))
    {
        frames.remove(pos);
        *quote_depth = quote_depth.saturating_sub(1);
    }
}

fn pop_list_frame(frames: &mut Vec<ListIndentAstFrame>) {
    if let Some(pos) =
        frames.iter().rposition(|frame| matches!(frame, ListIndentAstFrame::List { .. }))
    {
        frames.remove(pos);
    }
}

fn push_item_frame(
    source: &str,
    frames: &mut Vec<ListIndentAstFrame>,
    quote_depth: usize,
    range_start: usize,
) {
    let (list_quote_depth, prev_sibling_marker_start) = frames
        .iter()
        .rev()
        .find_map(|frame| match frame {
            ListIndentAstFrame::List { outer_quote_depth, prev_item_marker_start } => {
                Some((*outer_quote_depth, *prev_item_marker_start))
            }
            _ => None,
        })
        .unwrap_or((quote_depth, None));
    let parent_marker_start = frames.iter().rev().find_map(|frame| match frame {
        ListIndentAstFrame::Item { marker_start, .. } => *marker_start,
        _ => None,
    });
    let marker_start = find_list_marker_on_item_start(source, range_start, list_quote_depth);
    frames.push(ListIndentAstFrame::Item {
        outer_quote_depth: list_quote_depth,
        marker_start,
        prev_sibling_marker_start,
        parent_marker_start,
    });
}

fn finish_item_frame(
    source: &str,
    frames: &mut Vec<ListIndentAstFrame>,
    range: std::ops::Range<usize>,
    current_byte: usize,
    matched: &mut Option<ActiveListItemContext>,
) {
    let Some(pos) =
        frames.iter().rposition(|frame| matches!(frame, ListIndentAstFrame::Item { .. }))
    else {
        return;
    };
    let ListIndentAstFrame::Item {
        outer_quote_depth,
        marker_start,
        prev_sibling_marker_start,
        parent_marker_start,
    } = frames.remove(pos)
    else {
        return;
    };
    if let Some(ListIndentAstFrame::List { prev_item_marker_start, .. }) =
        frames.iter_mut().rev().find(|frame| matches!(frame, ListIndentAstFrame::List { .. }))
    {
        *prev_item_marker_start = marker_start;
    }
    if matched.is_none()
        && let Some(marker_start) = marker_start
        && let Some((first_line_start, _, _)) = locate_source_line_bounds(source, range.start)
    {
        let content_end = content_end_without_trailing_newline(source, range);
        if current_byte >= first_line_start && current_byte <= content_end {
            *matched = Some(ActiveListItemContext {
                outer_quote_depth,
                item_range: first_line_start..content_end,
                marker_start,
                prev_sibling_marker_start,
                parent_marker_start,
            });
        }
    }
}

fn find_list_marker_on_item_start(
    source: &str,
    start_byte: usize,
    outer_quote_depth: usize,
) -> Option<usize> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, start_byte)?;
    let origin = line_start + line_indent_origin(&source[line_start..line_end], outer_quote_depth);
    let mut probe = start_byte.max(origin);
    while probe <= line_end {
        if parse_list_marker(source, probe).is_some() {
            return Some(probe);
        }
        if !matches!(source.as_bytes().get(probe), Some(b' ' | b'\t')) {
            break;
        }
        probe += 1;
    }
    None
}

fn list_item_structural_content_start(source: &str, marker_start: usize) -> Option<usize> {
    if matches!(source.as_bytes().get(marker_start), Some(b'-' | b'+' | b'*')) {
        return list_marker_separator_end(source, marker_start + 1);
    }
    parse_list_marker(source, marker_start).map(|(_, content_start)| content_start)
}

fn line_indent_origin(line: &str, outer_quote_depth: usize) -> usize {
    let bytes = line.as_bytes();
    let mut probe = 0usize;
    for _ in 0..outer_quote_depth {
        let mut scan = probe;
        while scan < bytes.len() && matches!(bytes[scan], b' ' | b'\t') {
            scan += 1;
        }
        if bytes.get(scan) != Some(&b'>') {
            return probe;
        }
        probe = scan + 1;
        if matches!(bytes.get(probe), Some(b' ' | b'\t')) {
            probe += 1;
        }
    }
    probe
}

fn line_indent_edit_span(
    content: &str,
    outer_quote_depth: usize,
    change: &SubtreeIndentChange,
) -> Option<(usize, usize, String)> {
    let origin = line_indent_origin(content, outer_quote_depth);
    if content[origin..].trim_matches([' ', '\t']).is_empty() {
        return None;
    }
    match change {
        SubtreeIndentChange::Indent(delta_cols) => Some((origin, origin, " ".repeat(*delta_cols))),
        SubtreeIndentChange::Outdent(dedent_cols) => {
            let bytes = content.as_bytes();
            let mut column = 0usize;
            let mut probe = origin;
            while probe < bytes.len() && column < *dedent_cols {
                match bytes[probe] {
                    b' ' => {
                        column += 1;
                        probe += 1;
                    }
                    b'\t' => {
                        column = next_markdown_tab_stop(column);
                        probe += 1;
                    }
                    _ => break,
                }
            }
            let overshoot = column.saturating_sub(*dedent_cols);
            Some((origin, probe, " ".repeat(overshoot)))
        }
    }
}

fn rewrite_list_item_subtree_indent(
    source: &str,
    current_byte: usize,
    item_range: &std::ops::Range<usize>,
    outer_quote_depth: usize,
    change: SubtreeIndentChange,
) -> EditAugmentation {
    let block_start = item_range.start;
    let block_end = item_range.end;
    let segment = &source[block_start..block_end];
    let rel_cursor = current_byte.saturating_sub(block_start).min(segment.len());

    let mut replacement = String::with_capacity(segment.len() + 16);
    let mut chunk_start = 0usize;
    let mut mapped_cursor: Option<usize> = None;

    for chunk in segment.split_inclusive('\n') {
        let chunk_end = chunk_start + chunk.len();
        let line_no_lf = chunk.strip_suffix(LF_SEQUENCE).unwrap_or(chunk);
        let content = line_no_lf.strip_suffix('\r').unwrap_or(line_no_lf);
        let eol = &chunk[content.len()..];
        let out_line_start = replacement.len();
        let line_contains_cursor = mapped_cursor.is_none()
            && rel_cursor >= chunk_start
            && (rel_cursor < chunk_end || chunk_end == segment.len());

        if let Some((edit_start, edit_end, inserted)) =
            line_indent_edit_span(content, outer_quote_depth, &change)
        {
            if line_contains_cursor {
                let line_rel = rel_cursor - chunk_start;
                let adjusted = if line_rel < edit_start {
                    line_rel
                } else if line_rel >= edit_end {
                    edit_start + inserted.len() + (line_rel - edit_end)
                } else {
                    edit_start
                };
                mapped_cursor = Some(block_start + out_line_start + adjusted);
            }
            replacement.push_str(&content[..edit_start]);
            replacement.push_str(&inserted);
            replacement.push_str(&content[edit_end..]);
            replacement.push_str(eol);
        } else {
            if line_contains_cursor {
                mapped_cursor = Some(block_start + out_line_start + (rel_cursor - chunk_start));
            }
            replacement.push_str(chunk);
        }
        chunk_start = chunk_end;
    }

    let augmentation = EditAugmentation {
        replace_range: Some(block_start..block_end),
        insert_text: Some(replacement),
        cursor_byte_after: mapped_cursor.unwrap_or(block_start + rel_cursor),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}
