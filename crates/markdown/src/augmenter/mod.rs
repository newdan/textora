//! Markdown 感知的编辑增强（Enter / Backspace / Delete / InsertText）。
//!
//! 方案 2026-07-06 阶段 2b/2c：把原本散落在 `view.rs` 里的
//! - `classify_enter_context` 分类器
//! - `augment_enter` / `augment_backspace` / `augment_insert_text` 入口
//! - 7 个 `*_augmentation` 自由函数
//!
//! 收敛到本模块，并把三条通用产物（source_newline / block_break / marker_break）
//! 抽成 `emit_*` 原语，让所有分支共享同一份不变量断言。
//!
//! 不变量（对每一次成功返回的 [`ui::plugin::EditAugmentation`]）：
//! 1. `replace_range.end <= source.len()`
//! 2. `cursor_byte_after` 位于最终字符串的合法字节位置
//! 3. `insert_text` 与 `replace_range` 至少有一个存在
//! 4. `emit_*` 三原语内部用 `debug_assert!` 强制以上约束

pub(in crate::augmenter) mod editable_paragraph_edit;
pub(in crate::augmenter) mod editable_paragraph_navigation;
pub(in crate::augmenter) mod paragraph_leading_spaces;

use crate::builder::ListBullet;
#[cfg(test)]
use std::cell::Cell;
use ui::plugin::{AugmentKind, EditAugmentation};
use unicode_segmentation::UnicodeSegmentation;

pub(in crate::augmenter) const LF_SEQUENCE: &str = "\n";
pub(in crate::augmenter) const CRLF_SEQUENCE: &str = "\r\n";
pub(in crate::augmenter) const BLOCK_BOUNDARY_NEWLINE_COUNT: usize = 2;
pub(in crate::augmenter) const MAX_LEADING_BLOCK_INDENT: usize = 3;
/// CommonMark 代码围栏(``` 或 ~~~)的最少字符数。
pub(in crate::augmenter) const MIN_FENCE_MARKER_LENGTH: usize = 3;
/// CommonMark 空格形式硬换行所需的最少行尾空格数。
pub(in crate::augmenter) const HARD_BREAK_MIN_SPACES: usize = 2;
pub(in crate::augmenter) const TASK_LIST_CONTENT_SEPARATOR: char = ' ';
pub(in crate::augmenter) const MARKDOWN_TAB_STOP_WIDTH: usize = 4;

#[cfg(test)]
thread_local! {
    pub(in crate::augmenter) static CLASSIFY_PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
}

// ─── 入口 ──────────────────────────────────────────────────────────────────

/// 根据当前光标位置和键类型，计算一个 markdown 感知的编辑增强。
///
/// 返回 `None` 表示当前上下文没有特殊行为，调用方应回落到普通 Enter/Backspace/InsertText。
pub fn augment_edit(
    source: &str,
    current_byte: usize,
    kind: AugmentKind,
) -> Option<EditAugmentation> {
    match kind {
        AugmentKind::Enter => augment_enter(source, current_byte),
        AugmentKind::LineBreak => Some(augment_line_break(source, current_byte)),
        AugmentKind::Backspace => augment_backspace(source, current_byte),
        AugmentKind::Tab => augment_table_tab(source, current_byte, false),
        AugmentKind::ShiftTab => augment_table_tab(source, current_byte, true),
        AugmentKind::InsertText(ref text) => augment_insert_text(source, current_byte, text),
    }
}

pub(in crate::augmenter) fn augment_line_break(
    source: &str,
    current_byte: usize,
) -> EditAugmentation {
    let split_at = reversible_split_point(source, current_byte);
    let context = classify_enter_context(source, current_byte);
    let continuation_prefix = match &context {
        EnterContext::ListItem { content_prefix, .. } => content_prefix.clone(),
        EnterContext::BlockQuoteLine { continuation_prefix, .. }
        | EnterContext::IndentedCodeBlock { continuation_prefix } => continuation_prefix.clone(),
        EnterContext::LiteralBlockLine { continuation_prefix } => {
            return emit_literal_source_newline(source, current_byte, continuation_prefix);
        }
        EnterContext::Heading { .. }
        | EnterContext::SetextHeading { .. }
        | EnterContext::TableCell { .. } => {
            if cfg!(feature = "rich-markdown") {
                return emit_inline_html_break(source, current_byte);
            }
            return emit_source_newline(source, current_byte);
        }
        EnterContext::CodeBlock | EnterContext::CodeBlockFenceLine { .. } => {
            return emit_fenced_source_newline(source, current_byte);
        }
        EnterContext::HorizontalRuleEnd
        | EnterContext::EmptyBlockSeparatorLine
        | EnterContext::Other => {
            return emit_source_newline(source, current_byte);
        }
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::ParagraphStart { .. } => String::new(),
    };
    let newline = preferred_newline_sequence(source, split_at);
    let insertion = format!("\\{newline}{continuation_prefix}");
    let augmentation = EditAugmentation {
        cursor_byte_after: split_at + insertion.len(),
        replace_range: Some(split_at..split_at),
        insert_text: Some(insertion),
    };
    preserve_inline_elements_at_split(source, split_at, augmentation)
}

pub(in crate::augmenter) fn emit_inline_html_break(
    source: &str,
    current_byte: usize,
) -> EditAugmentation {
    const INLINE_HTML_BREAK: &str = "<br>";
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + INLINE_HTML_BREAK.len(),
        replace_range: Some(current_byte..current_byte),
        insert_text: Some(INLINE_HTML_BREAK.to_owned()),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

pub(in crate::augmenter) fn augment_enter(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let pipe_candidate_context = if has_pipe_header_shape_at_cursor(source, current_byte) {
        let context = classify_enter_context(source, current_byte);
        if let Some(augmentation) = pipe_header_table_augmentation(source, current_byte, &context) {
            return Some(augmentation);
        }
        Some(context)
    } else {
        None
    };
    if let Some(augmentation) = editable_paragraph_navigation::enter(source, current_byte) {
        return Some(augmentation);
    }
    let context =
        pipe_candidate_context.unwrap_or_else(|| classify_enter_context(source, current_byte));
    enter_context_augmentation(source, current_byte, context)
}

pub(in crate::augmenter) fn has_pipe_header_shape_at_cursor(
    source: &str,
    current_byte: usize,
) -> bool {
    let Some((line_start, _, line_end)) = locate_source_line_bounds(source, current_byte) else {
        return false;
    };
    let content_end = source_line_content_end(source, line_end);
    if current_byte != content_end {
        return false;
    }
    let mut content_start = line_start;
    loop {
        let leading_spaces = source[content_start..content_end]
            .bytes()
            .take(MAX_LEADING_BLOCK_INDENT)
            .take_while(|byte| *byte == b' ')
            .count();
        let marker_start = content_start + leading_spaces;
        match source.as_bytes().get(marker_start) {
            Some(b'>') => {
                content_start = marker_start + 1;
                if matches!(source.as_bytes().get(content_start), Some(b' ' | b'\t')) {
                    content_start += 1;
                }
            }
            _ => {
                if let Some((_, after_marker)) = parse_list_marker(source, marker_start) {
                    content_start = after_marker;
                } else {
                    content_start = marker_start;
                    break;
                }
            }
        }
    }
    let Some(header) = source.get(content_start..content_end) else {
        return false;
    };
    pipe_header_column_count(header.trim_start_matches([' ', '\t'])).is_some()
}

pub(in crate::augmenter) fn pipe_header_table_augmentation(
    source: &str,
    current_byte: usize,
    context: &EnterContext,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let content_end = source_line_content_end(source, line_end);
    if current_byte != content_end {
        return None;
    }
    let (header_content_start, container_prefix) = match context {
        EnterContext::TopLevelParagraphEnd => (line_start, ""),
        EnterContext::BlockQuoteLine { continuation_prefix, .. } => {
            let (_, content_start, _) = locate_blockquote_line(source, current_byte)?;
            (content_start, continuation_prefix.as_str())
        }
        EnterContext::ListItem { content_prefix, .. } => {
            let content_start = parse_list_marker(source, line_start)
                .map_or(line_start + content_prefix.len(), |(_, content_start)| content_start);
            (content_start, content_prefix.as_str())
        }
        _ => return None,
    };
    let header = source.get(header_content_start..content_end)?;
    let column_count = pipe_header_column_count(header)?;
    let escaped_header = escape_inline_code_pipes(header);
    let original_newline = preferred_newline_sequence(source, current_byte);
    let divider = (0..column_count).map(|_| " --- |").collect::<String>();
    let divider = format!("|{divider}");
    let body = format!("|{}", "  |".repeat(column_count));
    let divider = format!("{container_prefix}{divider}");
    let body = format!("{container_prefix}{body}");
    let trailing_separator = if suffix_needs_blank_line_after_table(source, current_byte) {
        original_newline
    } else {
        ""
    };
    let table_suffix =
        format!("{original_newline}{divider}{original_newline}{body}{trailing_separator}");
    let original_header_prefix = source.get(line_start..header_content_start)?;
    let replacement_text = format!("{original_header_prefix}{escaped_header}{table_suffix}");
    let cursor_byte_after = line_start
        + original_header_prefix.len()
        + escaped_header.len()
        + original_newline.len()
        + divider.len()
        + original_newline.len()
        + container_prefix.len()
        + 2;
    let mut prospective = source.to_owned();
    prospective.replace_range(line_start..current_byte, &replacement_text);
    let table_count_before = parsed_table_count(source, column_count);
    let table_count_after = parsed_table_count(&prospective, column_count);
    if table_count_after != table_count_before + 1 {
        return None;
    }
    Some(EditAugmentation {
        insert_text: Some(replacement_text),
        replace_range: Some(line_start..current_byte),
        cursor_byte_after,
    })
}

pub(in crate::augmenter) fn escape_inline_code_pipes(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut result = String::with_capacity(line.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'`' {
            let next_delimiter = bytes[index..]
                .iter()
                .position(|byte| *byte == b'`')
                .map_or(bytes.len(), |offset| index + offset);
            result.push_str(&line[index..next_delimiter]);
            index = next_delimiter;
            continue;
        }
        let opener_end = index + bytes[index..].iter().take_while(|byte| **byte == b'`').count();
        if is_escaped_by_backslash(bytes, index) {
            result.push_str(&line[index..opener_end]);
            index = opener_end;
            continue;
        }
        let delimiter_length = opener_end - index;
        let Some(closer_start) = matching_code_span_close(bytes, opener_end, delimiter_length)
        else {
            result.push_str(&line[index..opener_end]);
            index = opener_end;
            continue;
        };
        result.push_str(&line[index..opener_end]);
        let mut content_start = opener_end;
        for pipe_index in opener_end..closer_start {
            if bytes[pipe_index] != b'|' {
                continue;
            }
            let preceding_backslashes = bytes[opener_end..pipe_index]
                .iter()
                .rev()
                .take_while(|byte| **byte == b'\\')
                .count();
            result.push_str(&line[content_start..pipe_index]);
            if preceding_backslashes % 2 == 0 {
                result.push('\\');
            }
            result.push('|');
            content_start = pipe_index + 1;
        }
        result.push_str(&line[content_start..closer_start + delimiter_length]);
        index = closer_start + delimiter_length;
    }
    result
}

pub(in crate::augmenter) fn suffix_needs_blank_line_after_table(
    source: &str,
    insertion_at: usize,
) -> bool {
    let Some(newline_width) = newline_sequence_width_at(source, insertion_at) else {
        return false;
    };
    let next_line_start = insertion_at + newline_width;
    !source[next_line_start..].is_empty()
        && newline_sequence_width_at(source, next_line_start).is_none()
}

pub(in crate::augmenter) fn pipe_header_column_count(header: &str) -> Option<usize> {
    let trimmed = header.trim_matches([' ', '\t']);
    let delimiters = unescaped_pipes_outside_code(trimmed);
    if delimiters.first() != Some(&0) || delimiters.last() != Some(&(trimmed.len() - 1)) {
        return None;
    }
    delimiters.len().checked_sub(1).filter(|columns| *columns > 0)
}

pub(in crate::augmenter) fn unescaped_pipes_outside_code(line: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut delimiters = Vec::new();
    while index < bytes.len() {
        if bytes[index] == b'`' {
            let run_end = index + bytes[index..].iter().take_while(|byte| **byte == b'`').count();
            if is_escaped_by_backslash(bytes, index) {
                index = run_end;
                continue;
            }
            let delimiter_length = run_end - index;
            if let Some(close_start) = matching_code_span_close(bytes, run_end, delimiter_length) {
                index = close_start + delimiter_length;
                continue;
            }
            index = run_end;
            continue;
        }
        if bytes[index] == b'|' && !is_escaped_by_backslash(bytes, index) {
            delimiters.push(index);
        }
        index += 1;
    }
    delimiters
}

pub(in crate::augmenter) fn is_escaped_by_backslash(bytes: &[u8], byte_index: usize) -> bool {
    bytes[..byte_index].iter().rev().take_while(|byte| **byte == b'\\').count() % 2 == 1
}

pub(in crate::augmenter) fn matching_code_span_close(
    bytes: &[u8],
    mut index: usize,
    delimiter_length: usize,
) -> Option<usize> {
    while index < bytes.len() {
        if bytes[index] != b'`' {
            index += 1;
            continue;
        }
        let run_end = index + bytes[index..].iter().take_while(|byte| **byte == b'`').count();
        if run_end - index == delimiter_length {
            return Some(index);
        }
        index = run_end;
    }
    None
}

pub(in crate::augmenter) fn parsed_table_count(source: &str, expected_columns: usize) -> usize {
    use pulldown_cmark::{Event, Parser, Tag};
    Parser::new_ext(source, crate::parser::markdown_options())
        .into_offset_iter()
        .filter(|(event, _)| {
            matches!(event, Event::Start(Tag::Table(alignments)) if alignments.len() == expected_columns)
        })
        .count()
}

pub(in crate::augmenter) fn augment_insert_text(
    source: &str,
    current_byte: usize,
    text: &str,
) -> Option<EditAugmentation> {
    if text.contains(['|', '\r', '\n'])
        && let EnterContext::TableCell { cell_range, .. } =
            classify_enter_context(source, current_byte)
    {
        return Some(table_cell_text_augmentation(source, current_byte, text, cell_range));
    }
    paragraph_leading_spaces::insert_text(source, current_byte, text)
        .or_else(|| editable_paragraph_edit::insert_text(source, current_byte, text))
}

pub(crate) fn augment_selected_text(
    source: &str,
    selection: std::ops::Range<usize>,
    text: &str,
) -> Option<EditAugmentation> {
    if text.contains(['|', '\r', '\n'])
        && let EnterContext::TableCell { cell_range, .. } =
            classify_enter_context(source, selection.start)
        && selection.start >= cell_range.start
        && selection.end <= cell_range.end
    {
        return Some(table_cell_text_augmentation(source, selection.start, text, cell_range));
    }
    paragraph_leading_spaces::insert_selected_text(source, selection, text)
}

pub(in crate::augmenter) fn table_cell_text_augmentation(
    source: &str,
    current_byte: usize,
    text: &str,
    cell_range: std::ops::Range<usize>,
) -> EditAugmentation {
    let normalized_newlines = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let preceding_backslashes =
        source[..current_byte].bytes().rev().take_while(|byte| *byte == b'\\').count();
    let mut inserted_text = String::with_capacity(normalized_newlines.len());
    for character in normalized_newlines.chars() {
        if character == '|'
            && inserted_text
                .bytes()
                .rev()
                .take_while(|byte| *byte == b'\\')
                .count()
                .checked_add(if inserted_text.bytes().all(|byte| byte == b'\\') {
                    preceding_backslashes
                } else {
                    0
                })
                .is_some_and(|backslash_count| backslash_count % 2 == 0)
        {
            inserted_text.push('\\');
        }
        inserted_text.push(character);
    }
    let augmentation = EditAugmentation {
        replace_range: Some(current_byte..current_byte),
        cursor_byte_after: current_byte + inserted_text.len(),
        insert_text: Some(inserted_text),
    };
    debug_assert!(cell_range.start <= current_byte && current_byte <= cell_range.end);
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

pub(in crate::augmenter) fn augment_table_tab(
    source: &str,
    current_byte: usize,
    reverse: bool,
) -> Option<EditAugmentation> {
    let EnterContext::TableCell {
        previous_tab_cell_start,
        next_tab_cell_start,
        column_count,
        is_last_cell,
        row_line_end,
        container_prefix,
        ..
    } = classify_enter_context(source, current_byte)
    else {
        return None;
    };
    let target_cell = if reverse { previous_tab_cell_start } else { next_tab_cell_start };
    if let Some(target_cell) = target_cell {
        return Some(EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: None,
            cursor_byte_after: target_cell,
        });
    }
    if reverse || !is_last_cell {
        return None;
    }
    Some(table_insert_row_augmentation(source, row_line_end, column_count, &container_prefix))
}

mod backspace;
mod classify;
mod enter_rules;
mod primitives;
mod source_slice;

#[allow(unused_imports)]
pub(crate) use backspace::{augment_delete_forward, augment_erase_range};
pub use classify::{EnterContext, HeadingPosition, classify_enter_context};
#[allow(unused_imports)]
pub(crate) use classify::{heading_source_is_atx, table_cell_content_start};
#[allow(unused_imports)]
pub(crate) use source_slice::{list_item_indent, parse_list_marker};

#[allow(unused_imports)]
pub(in crate::augmenter) use backspace::*;
#[allow(unused_imports)]
pub(in crate::augmenter) use classify::*;
#[allow(unused_imports)]
pub(in crate::augmenter) use enter_rules::*;
#[allow(unused_imports)]
pub(in crate::augmenter) use primitives::*;
#[allow(unused_imports)]
pub(in crate::augmenter) use source_slice::*;

#[cfg(test)]
#[path = "augmenter_tests.rs"]
mod tests;
