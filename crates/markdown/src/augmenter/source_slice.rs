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
