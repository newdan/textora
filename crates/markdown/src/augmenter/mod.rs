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
use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
use crate::source_snapshot::{MarkdownSource, SourceSnapshot};
use ui::plugin::{AugmentKind, EditAugmentation};

pub(in crate::augmenter) const LF_SEQUENCE: &str = "\n";
pub(in crate::augmenter) const CRLF_SEQUENCE: &str = "\r\n";
pub(in crate::augmenter) const BLOCK_BOUNDARY_NEWLINE_COUNT: usize = 2;
pub(in crate::augmenter) const MAX_LEADING_BLOCK_INDENT: usize = 3;
/// CommonMark 代码围栏(``` 或 ~~~)的最少字符数。
pub(in crate::augmenter) const MIN_FENCE_MARKER_LENGTH: usize = 3;
/// CommonMark 空格形式硬换行所需的最少行尾空格数。
pub(crate) const HARD_BREAK_MIN_SPACES: usize = 2;
pub(in crate::augmenter) const TASK_LIST_CONTENT_SEPARATOR: char = ' ';
pub(in crate::augmenter) const MARKDOWN_TAB_STOP_WIDTH: usize = 4;

// ─── 入口 ──────────────────────────────────────────────────────────────────

/// 根据当前光标位置和键类型，计算一个 markdown 感知的编辑增强。
///
/// 返回 `None` 表示当前上下文没有特殊行为，调用方应回落到普通 Enter/Backspace/InsertText。
pub fn augment_edit(
    source: &str,
    current_byte: usize,
    kind: AugmentKind,
) -> Option<EditAugmentation> {
    augment_source(&MarkdownSource::borrowed(source), current_byte, kind)
}

pub fn classify_enter_context(source: &str, current_byte: usize) -> EnterContext {
    classify_source(&MarkdownSource::borrowed(source), current_byte)
}

pub(crate) fn augment_source(
    source: &MarkdownSource<'_>,
    current_byte: usize,
    kind: AugmentKind,
) -> Option<EditAugmentation> {
    match kind {
        AugmentKind::Enter => augment_enter(source, current_byte).map(|plan| plan.augmentation),
        AugmentKind::LineBreak => Some(augment_line_break(source, current_byte)),
        AugmentKind::Backspace => augment_backspace(source, current_byte),
        AugmentKind::Tab => augment_table_tab(source, current_byte, false)
            .or_else(|| augment_list_item_tab(source, current_byte, false)),
        AugmentKind::ShiftTab => augment_table_tab(source, current_byte, true)
            .or_else(|| augment_list_item_tab(source, current_byte, true)),
        AugmentKind::InsertText(ref text) => augment_insert_text(source, current_byte, text),
    }
}

enum LineBreakEncoding {
    Literal { continuation_prefix: String },
    PlainNewline,
    InlineHtml,
    BackslashHardBreak { continuation_prefix: String },
}

fn line_break_encoding_for_context(
    source: &MarkdownSource<'_>,
    current_byte: usize,
    context: &EnterContext,
) -> LineBreakEncoding {
    match context {
        EnterContext::IndentedCodeBlock { continuation_prefix }
        | EnterContext::LiteralBlockLine { continuation_prefix } => {
            LineBreakEncoding::Literal { continuation_prefix: continuation_prefix.clone() }
        }
        EnterContext::CodeBlock | EnterContext::CodeBlockFenceLine { .. } => {
            LineBreakEncoding::Literal {
                continuation_prefix: fenced_continuation_prefix(source, current_byte)
                    .unwrap_or_default(),
            }
        }
        EnterContext::Heading { .. }
        | EnterContext::SetextHeading { .. }
        | EnterContext::TableCell { .. } => LineBreakEncoding::InlineHtml,
        EnterContext::HorizontalRuleEnd
        | EnterContext::EmptyBlockSeparatorLine
        | EnterContext::Other => LineBreakEncoding::PlainNewline,
        EnterContext::ListItem { content_prefix, .. } => {
            LineBreakEncoding::BackslashHardBreak { continuation_prefix: content_prefix.clone() }
        }
        EnterContext::BlockQuoteLine { continuation_prefix, .. } => {
            LineBreakEncoding::BackslashHardBreak {
                continuation_prefix: continuation_prefix.clone(),
            }
        }
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::ParagraphStart { .. } => {
            LineBreakEncoding::BackslashHardBreak { continuation_prefix: String::new() }
        }
    }
}

pub(in crate::augmenter) fn augment_line_break(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> EditAugmentation {
    let context = classify_source(source, current_byte);
    match line_break_encoding_for_context(source, current_byte, &context) {
        LineBreakEncoding::Literal { continuation_prefix } => {
            emit_literal_source_newline(source, current_byte, &continuation_prefix)
        }
        LineBreakEncoding::PlainNewline => emit_source_newline(source, current_byte),
        LineBreakEncoding::InlineHtml => preserve_inline_code_at_split(
            source,
            current_byte,
            emit_inline_html_break(source, current_byte),
        ),
        LineBreakEncoding::BackslashHardBreak { continuation_prefix } => {
            let split_at = reversible_split_point(source, current_byte);
            let newline = preferred_newline_sequence(source, split_at);
            let insertion = format!("\\{newline}{continuation_prefix}");
            let augmentation = EditAugmentation {
                cursor_byte_after: split_at + insertion.len(),
                replace_range: Some(split_at..split_at),
                insert_text: Some(insertion),
            };
            preserve_inline_elements_at_split(source, split_at, augmentation)
        }
    }
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

pub(crate) struct EnterAugmentation {
    pub(crate) augmentation: EditAugmentation,
    pub(crate) validated_source: Option<SourceSnapshot>,
}

pub(crate) fn augment_enter(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EnterAugmentation> {
    let pipe_candidate_context = if has_pipe_header_shape_at_cursor(source, current_byte) {
        let context = classify_source(source, current_byte);
        if let Some(augmentation) = pipe_header_table_augmentation(source, current_byte, &context) {
            return Some(augmentation);
        }
        Some(context)
    } else {
        None
    };
    if let Some(augmentation) = editable_paragraph_navigation::enter(source, current_byte) {
        return Some(EnterAugmentation { augmentation, validated_source: None });
    }
    let context = pipe_candidate_context.unwrap_or_else(|| classify_source(source, current_byte));
    enter_context_augmentation(source, current_byte, context)
        .map(|augmentation| EnterAugmentation { augmentation, validated_source: None })
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

fn pipe_header_table_augmentation(
    source: &MarkdownSource<'_>,
    current_byte: usize,
    context: &EnterContext,
) -> Option<EnterAugmentation> {
    let (line_start, _, line_end) = locate_snapshot_line_bounds(source, current_byte)?;
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
    let mut prospective = source.as_str().to_owned();
    prospective.replace_range(line_start..current_byte, &replacement_text);
    let table_count_before = parsed_table_count(source, column_count);
    let validated_source = MarkdownSource::snapshot(prospective);
    let table_count_after = parsed_table_count(&validated_source, column_count);
    if table_count_after != table_count_before + 1 {
        return None;
    }
    Some(EnterAugmentation {
        augmentation: EditAugmentation {
            insert_text: Some(replacement_text),
            replace_range: Some(line_start..current_byte),
            cursor_byte_after,
        },
        validated_source: Some(validated_source),
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

pub(in crate::augmenter) fn parsed_table_count(
    source: &MarkdownSource<'_>,
    expected_columns: usize,
) -> usize {
    source.parsed().source_events()
        .filter(|(event, _)| {
            matches!(event, MarkdownEvent::Start(MarkdownTag::Table(alignments)) if alignments.len() == expected_columns)
        })
        .count()
}

pub(in crate::augmenter) fn augment_insert_text(
    source: &MarkdownSource<'_>,
    current_byte: usize,
    text: &str,
) -> Option<EditAugmentation> {
    if text.contains(['|', '\r', '\n'])
        && let EnterContext::TableCell { cell_range, .. } = classify_source(source, current_byte)
    {
        return Some(table_cell_text_augmentation(source, current_byte, text, cell_range));
    }
    empty_fenced_code_insert_text(source, current_byte, text)
        .or_else(|| paragraph_leading_spaces::insert_text(source, current_byte, text))
        .or_else(|| editable_paragraph_edit::insert_text(source, current_byte, text))
        .or_else(|| container_multiline_insert_text(source, current_byte, text))
}

pub(in crate::augmenter) fn format_container_multiline_insertion(
    text: &str,
    continuation_prefix: &str,
    blank_line_prefix: &str,
) -> String {
    let mut formatted = String::with_capacity(text.len() + continuation_prefix.len() * 4);
    for (line_index, chunk) in text.split_inclusive('\n').enumerate() {
        let line_no_lf = chunk.strip_suffix(LF_SEQUENCE).unwrap_or(chunk);
        let content = line_no_lf.strip_suffix('\r').unwrap_or(line_no_lf);
        let eol = &chunk[content.len()..];
        if line_index > 0 {
            let prefix =
                if content.trim().is_empty() { blank_line_prefix } else { continuation_prefix };
            if !prefix.is_empty() && !content.starts_with(prefix) {
                formatted.push_str(prefix);
            }
        }
        formatted.push_str(content);
        formatted.push_str(eol);
    }
    if text.ends_with('\n') && !continuation_prefix.is_empty() {
        formatted.push_str(continuation_prefix);
    }
    formatted
}

fn container_multiline_insert_text(
    source: &MarkdownSource<'_>,
    current_byte: usize,
    text: &str,
) -> Option<EditAugmentation> {
    if !text.contains(['\r', '\n']) || text.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
        return None;
    }
    let continuation_prefix = match classify_source(source, current_byte) {
        EnterContext::BlockQuoteLine { continuation_prefix, .. } => continuation_prefix,
        EnterContext::ListItem { content_prefix, .. } => content_prefix,
        _ => return None,
    };
    if continuation_prefix.is_empty() {
        return None;
    }
    let blank_line_prefix = continuation_prefix.trim_end_matches([' ', '\t']);
    let formatted =
        format_container_multiline_insertion(text, &continuation_prefix, blank_line_prefix);
    let augmentation = EditAugmentation {
        replace_range: Some(current_byte..current_byte),
        cursor_byte_after: current_byte + formatted.len(),
        insert_text: Some(formatted),
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

fn empty_fenced_code_insert_text(
    source: &MarkdownSource<'_>,
    current_byte: usize,
    text: &str,
) -> Option<EditAugmentation> {
    if text.is_empty() {
        return None;
    }
    let (closing_line_start, _, closing_line_end) =
        locate_snapshot_line_bounds(source, current_byte)?;
    let closing_content_end = source_line_content_end(source, closing_line_end);
    let closing_line = &source[closing_line_start..closing_content_end];
    if !closing_line.contains("```") && !closing_line.contains("~~~") {
        return None;
    }
    let boundary = enclosing_fenced_boundary(source, current_byte)?;
    let (_, _, opening_line_end) =
        locate_snapshot_line_bounds(source, boundary.frame.marker_start)?;
    if closing_line_start != opening_line_end + LF_SEQUENCE.len() {
        return None;
    }
    let opening_content_end = source_line_content_end(source, opening_line_end);
    let (fence_char, opening_fence_len) =
        opening_fence_signature(source, boundary.frame.opening_position, opening_content_end)?;
    let closing_position = matching_container_position(
        source,
        closing_line_start,
        closing_content_end,
        &boundary.frame.container_path,
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
    let closing_fence_start =
        marker_position_after_optional_indent(source, closing_content_end, closing_position)?
            .byte_offset;
    if current_byte > closing_fence_start {
        return None;
    }

    let continuation_prefix = fenced_continuation_prefix(source, current_byte).unwrap_or_default();
    let closing_prefix = &source[closing_line_start..closing_fence_start];
    let newline = preferred_newline_sequence(source, current_byte);
    let replacement = format!("{continuation_prefix}{text}{newline}{closing_prefix}");
    let augmentation = EditAugmentation {
        cursor_byte_after: closing_line_start + continuation_prefix.len() + text.len(),
        replace_range: Some(closing_line_start..closing_fence_start),
        insert_text: Some(replacement),
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(crate) fn augment_selected_text(
    source: &MarkdownSource<'_>,
    selection: std::ops::Range<usize>,
    text: &str,
    erased_source: Option<&MarkdownSource<'_>>,
) -> Option<EditAugmentation> {
    if text.contains(['|', '\r', '\n'])
        && let EnterContext::TableCell { cell_range, .. } = classify_source(source, selection.start)
        && selection.start >= cell_range.start
        && selection.end <= cell_range.end
    {
        return Some(table_cell_text_augmentation(source, selection.start, text, cell_range));
    }
    if text.contains(['\r', '\n']) && !text.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
        let cursor = selection.start;
        return with_erased_selection(source, selection, erased_source, |candidate| {
            container_multiline_insert_text(candidate, cursor, text)
        });
    }
    paragraph_leading_spaces::insert_selected_text(source, selection, text, erased_source)
}

fn with_erased_selection(
    source: &str,
    selection: std::ops::Range<usize>,
    erased_source: Option<&MarkdownSource<'_>>,
    plan: impl FnOnce(&MarkdownSource<'_>) -> Option<EditAugmentation>,
) -> Option<EditAugmentation> {
    if let Some(candidate) = erased_source {
        return plan(candidate);
    }
    let mut candidate = source.to_owned();
    candidate.replace_range(selection, "");
    plan(&MarkdownSource::owned(candidate))
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
    source: &MarkdownSource<'_>,
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
    } = classify_source(source, current_byte)
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
pub(crate) use classify::classify_source;
pub use classify::{EnterContext, HeadingPosition};
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
