use super::*;

// ─── 分类结果 → 具体 augmentation ────────────────────────────────────────

pub(in crate::augmenter) fn enter_context_augmentation(
    source: &str,
    current_byte: usize,
    context: EnterContext,
) -> Option<EditAugmentation> {
    match context {
        EnterContext::TopLevelParagraphEnd | EnterContext::ParagraphInterior => {
            Some(paragraph_enter_augmentation(source, current_byte))
        }
        EnterContext::ParagraphStart { insert_at } => {
            Some(emit_paragraph_before(source, current_byte, insert_at, ""))
        }
        EnterContext::Heading { level: _, position, continuation_prefix } => {
            heading_enter_augmentation(source, current_byte, position, &continuation_prefix)
        }
        EnterContext::SetextHeading { position, underline_end, continuation_prefix } => {
            match position {
                HeadingPosition::Start { insert_at } => Some(emit_paragraph_before(
                    source,
                    current_byte,
                    insert_at,
                    &continuation_prefix,
                )),
                HeadingPosition::Interior | HeadingPosition::End { .. } => Some(
                    setext_heading_enter_augmentation(source, underline_end, &continuation_prefix),
                ),
            }
        }
        EnterContext::ListItem {
            indent,
            continuation_marker,
            content_prefix,
            empty,
            at_end: _,
        } => list_item_enter_augmentation(
            source,
            current_byte,
            &indent,
            &continuation_marker,
            &content_prefix,
            empty,
        ),
        EnterContext::BlockQuoteLine { continuation_prefix, empty_parent_prefix, empty } => {
            blockquote_enter_augmentation(
                source,
                current_byte,
                &continuation_prefix,
                &empty_parent_prefix,
                empty,
            )
        }
        EnterContext::TableCell {
            next_cell_start,
            column_count,
            row_is_empty,
            is_header_row,
            row_line_end,
            container_prefix,
            ..
        } => {
            if let Some(next_cell_start) = next_cell_start {
                Some(EditAugmentation {
                    insert_text: Some(String::new()),
                    replace_range: None,
                    cursor_byte_after: next_cell_start,
                })
            } else if row_is_empty && !is_header_row {
                table_exit_empty_row_augmentation(source, row_line_end, &container_prefix)
            } else {
                Some(table_insert_row_augmentation(
                    source,
                    row_line_end,
                    column_count,
                    &container_prefix,
                ))
            }
        }
        EnterContext::EmptyBlockSeparatorLine => Some(emit_source_newline(source, current_byte)),
        EnterContext::HorizontalRuleEnd => Some(emit_block_break(source, current_byte)),
        EnterContext::IndentedCodeBlock { continuation_prefix } => {
            Some(indented_code_block_enter_augmentation(source, current_byte, &continuation_prefix))
        }
        EnterContext::LiteralBlockLine { continuation_prefix } => {
            Some(emit_literal_source_newline(source, current_byte, &continuation_prefix))
        }
        EnterContext::CodeBlockFenceLine { is_opening, line_content_end } => {
            Some(fence_line_enter_augmentation(source, line_content_end, is_opening))
        }
        EnterContext::CodeBlock => fenced_continuation_prefix(source, current_byte)
            .filter(|prefix| !prefix.is_empty())
            .map(|prefix| emit_literal_source_newline(source, current_byte, &prefix)),
        EnterContext::Other => None,
    }
}

pub(in crate::augmenter) fn paragraph_enter_augmentation(
    source: &str,
    current_byte: usize,
) -> EditAugmentation {
    if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
        return emit_block_break_replacing(source, boundary);
    }

    // 段尾/段中：Typora 语义 = 切两半（`\n\n`）；跨已有 `\n` 时用
    // emit_block_break 保留光标下方的原源码行。
    if cursor_touches_source_newline(source, current_byte) {
        if newline_sequence_width_at(source, current_byte).is_some() {
            return emit_block_break(source, current_byte);
        }
        return emit_source_newline(source, current_byte);
    }
    let split_at = reversible_split_point(source, current_byte);
    let augmentation = if split_at == current_byte {
        emit_block_break(source, current_byte)
    } else {
        emit_block_break_at(source, split_at)
    };
    preserve_inline_elements_at_split(source, split_at, augmentation)
}

pub(in crate::augmenter) fn heading_enter_augmentation(
    source: &str,
    current_byte: usize,
    position: HeadingPosition,
    continuation_prefix: &str,
) -> Option<EditAugmentation> {
    if let HeadingPosition::Start { insert_at } = position {
        return Some(emit_paragraph_before(source, current_byte, insert_at, continuation_prefix));
    }
    if let HeadingPosition::End { insert_at } = position {
        if !continuation_prefix.is_empty() {
            return Some(emit_container_line_break(source, insert_at, continuation_prefix));
        }
        return Some(emit_block_break_at(source, insert_at));
    }

    // Heading 中间：在当前光标处分割标题，后半段成为普通段落。
    let split_at = reversible_split_point(source, current_byte);
    let insertion =
        format!("{}{}", preferred_newline_sequence(source, split_at), continuation_prefix);
    let aug = EditAugmentation {
        insert_text: Some(insertion.clone()),
        cursor_byte_after: split_at + insertion.len(),
        replace_range: Some(split_at..split_at),
    };
    debug_assert_augmentation(&aug, source);
    Some(preserve_inline_elements_at_split(source, split_at, aug))
}

pub(in crate::augmenter) fn emit_container_line_break(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let insertion =
        format!("{}{}", preferred_newline_sequence(source, current_byte), continuation_prefix);
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        replace_range: Some(current_byte..current_byte),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

/// Setext 标题内的无选区 Enter：保留标题源码，在下划线之后建立块边界。
pub(in crate::augmenter) fn setext_heading_enter_augmentation(
    source: &str,
    underline_end: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    if continuation_prefix.is_empty() {
        emit_block_break_at(source, underline_end)
    } else {
        emit_container_line_break(source, underline_end, continuation_prefix)
    }
}

/// 缩进代码块内 Enter：插入换行并续上分类阶段确定的代码内容前缀。
pub(in crate::augmenter) fn indented_code_block_enter_augmentation(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = format!("{newline}{continuation_prefix}");
    let aug = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    aug
}

/// 围栏代码块围栏行 Enter(光标可在围栏行任意位置,插入点恒为该行行尾):
/// - 开头围栏行(含 info string):行尾插入单个换行,光标进入代码体第一行;
/// - 闭合围栏行:行尾建立块边界,光标落到围栏外的新空段(退出代码块)。
pub(in crate::augmenter) fn fence_line_enter_augmentation(
    source: &str,
    line_content_end: usize,
    is_opening: bool,
) -> EditAugmentation {
    if !is_opening {
        return emit_block_break_at(source, line_content_end);
    }
    if let Some(augmentation) = close_new_fence_augmentation(source, line_content_end) {
        return augmentation;
    }
    emit_fenced_source_newline(source, line_content_end)
}

pub(in crate::augmenter) fn close_new_fence_augmentation(
    source: &str,
    line_content_end: usize,
) -> Option<EditAugmentation> {
    let trailing = source.get(line_content_end..)?;
    if !matches!(trailing, "" | LF_SEQUENCE | CRLF_SEQUENCE) {
        return None;
    }
    let (line_start, _, _) = locate_source_line_bounds(source, line_content_end)?;
    let marker_start = container_content_start_on_line(source, line_start, line_content_end);
    let fence_character = *source.as_bytes().get(marker_start)?;
    if !matches!(fence_character, b'`' | b'~') {
        return None;
    }
    let fence_length = source.as_bytes()[marker_start..line_content_end]
        .iter()
        .take_while(|byte| **byte == fence_character)
        .count();
    if fence_length < MIN_FENCE_MARKER_LENGTH {
        return None;
    }
    let continuation_prefix = canonical_container_prefix(source, marker_start);
    let fence = char::from(fence_character).to_string().repeat(fence_length);
    let newline = preferred_newline_sequence(source, line_content_end);
    let insertion = format!("{newline}{continuation_prefix}{newline}{continuation_prefix}{fence}");
    let augmentation = EditAugmentation {
        cursor_byte_after: line_content_end + newline.len() + continuation_prefix.len(),
        replace_range: Some(line_content_end..source.len()),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(in crate::augmenter) fn list_item_enter_augmentation(
    source: &str,
    current_byte: usize,
    indent: &str,
    continuation_marker: &str,
    continuation_content_prefix: &str,
    empty: bool,
) -> Option<EditAugmentation> {
    if empty {
        return emit_replace_current_line(source, current_byte, indent);
    }

    if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
        let replaced =
            extend_range_over_prefix_or_indent(source, boundary, continuation_content_prefix);
        return Some(emit_marker_break_replacing(source, replaced, indent, continuation_marker));
    }

    if let Some(newline_width) = newline_sequence_width_at(source, current_byte) {
        let next_line_start = current_byte + newline_width;
        if !is_only_blank_separator_before_content(source, next_line_start)
            && !next_source_line_has_list_marker(source, next_line_start)
        {
            let replaced = extend_range_over_prefix_or_indent(
                source,
                current_byte..next_line_start,
                continuation_content_prefix,
            );
            return Some(emit_marker_break_replacing(
                source,
                replaced,
                indent,
                continuation_marker,
            ));
        }
    }

    if newline_sequence_width_before(source, current_byte).is_some()
        && !next_source_line_has_list_marker(source, current_byte)
    {
        let replaced = extend_range_over_prefix_or_indent(
            source,
            current_byte..current_byte,
            continuation_content_prefix,
        );
        return Some(emit_inline_marker_replacing(source, replaced, indent, continuation_marker));
    }

    let split_at = reversible_split_point(source, current_byte);
    let augmentation = if split_at == current_byte {
        emit_marker_break(source, current_byte, indent, continuation_marker)
    } else {
        emit_marker_break_at(source, split_at, indent, continuation_marker)
    };
    Some(preserve_inline_elements_at_split(source, split_at, augmentation))
}

pub(in crate::augmenter) fn is_only_blank_separator_before_content(
    source: &str,
    line_start: usize,
) -> bool {
    let Some(blank_width) = blank_source_line_width(source, line_start) else {
        return false;
    };
    let line_end = line_start + blank_width;
    let Some(newline_width) = newline_sequence_width_at(source, line_end) else {
        return false;
    };
    let following_start = line_end + newline_width;
    following_start < source.len() && blank_source_line_width(source, following_start).is_none()
}

pub(in crate::augmenter) fn blockquote_enter_augmentation(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
    empty_parent_prefix: &str,
    empty: bool,
) -> Option<EditAugmentation> {
    if empty {
        return emit_replace_current_line(source, current_byte, empty_parent_prefix);
    }

    if let Some(boundary) = hard_break_boundary_after(source, current_byte) {
        let replaced = extend_range_over_prefix_or_indent(source, boundary, continuation_prefix);
        return Some(emit_marker_break_replacing(source, replaced, "", continuation_prefix));
    }

    if let Some(newline_width) = newline_sequence_width_at(source, current_byte) {
        let next_line_start = current_byte + newline_width;
        if !next_source_line_has_blockquote_marker(source, next_line_start) {
            let replaced = extend_range_over_prefix_or_indent(
                source,
                current_byte..next_line_start,
                continuation_prefix,
            );
            return Some(emit_marker_break_replacing(source, replaced, "", continuation_prefix));
        }
    }

    if newline_sequence_width_before(source, current_byte).is_some()
        && !next_source_line_has_blockquote_marker(source, current_byte)
    {
        let replaced = extend_range_over_prefix_or_indent(
            source,
            current_byte..current_byte,
            continuation_prefix,
        );
        return Some(emit_inline_marker_replacing(source, replaced, "", continuation_prefix));
    }

    let split_at = reversible_split_point(source, current_byte);
    let augmentation = if split_at == current_byte {
        emit_marker_break(source, current_byte, "", continuation_prefix)
    } else {
        emit_marker_break_at(source, split_at, "", continuation_prefix)
    };
    Some(preserve_inline_elements_at_split(source, split_at, augmentation))
}

pub(in crate::augmenter) fn extend_range_over_prefix(
    source: &str,
    mut range: std::ops::Range<usize>,
    prefix: &str,
) -> std::ops::Range<usize> {
    if source.get(range.end..).is_some_and(|suffix| suffix.starts_with(prefix)) {
        range.end += prefix.len();
    }
    range
}

pub(in crate::augmenter) fn extend_range_over_prefix_or_indent(
    source: &str,
    range: std::ops::Range<usize>,
    prefix: &str,
) -> std::ops::Range<usize> {
    let extended = extend_range_over_prefix(source, range.clone(), prefix);
    if extended.end > range.end {
        return extended;
    }

    let indentation_width = source[range.end..]
        .bytes()
        .take_while(|source_byte| matches!(*source_byte, b' ' | b'\t'))
        .count();
    range.start..range.end + indentation_width
}

pub(in crate::augmenter) fn emit_inline_marker_replacing(
    source: &str,
    replace_range: std::ops::Range<usize>,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let insertion = format!("{indent}{marker}");
    let augmentation = EditAugmentation {
        cursor_byte_after: replace_range.start + insertion.len(),
        replace_range: Some(replace_range),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

pub(in crate::augmenter) fn next_source_line_marker_start(
    source: &str,
    line_start: usize,
) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut marker_start = line_start;
    let mut leading_space_count = 0;
    while bytes.get(marker_start) == Some(&b' ') && leading_space_count < MAX_LEADING_BLOCK_INDENT {
        marker_start += 1;
        leading_space_count += 1;
    }
    (marker_start <= source.len()).then_some(marker_start)
}

pub(in crate::augmenter) fn next_source_line_has_list_marker(
    source: &str,
    line_start: usize,
) -> bool {
    next_source_line_marker_start(source, line_start)
        .is_some_and(|marker_start| parse_list_marker(source, marker_start).is_some())
}

pub(in crate::augmenter) fn next_source_line_has_blockquote_marker(
    source: &str,
    line_start: usize,
) -> bool {
    next_source_line_marker_start(source, line_start)
        .is_some_and(|marker_start| source.as_bytes().get(marker_start) == Some(&b'>'))
}

pub(in crate::augmenter) fn reversible_split_point(source: &str, current_byte: usize) -> usize {
    let bytes = source.as_bytes();
    if bytes.get(current_byte) == Some(&b' ') {
        let belongs_to_space_run = current_byte > 0 && bytes.get(current_byte - 1) == Some(&b' ')
            || bytes.get(current_byte + 1) == Some(&b' ');
        if !belongs_to_space_run {
            return current_byte + 1;
        }
    }
    current_byte
}

#[derive(Debug)]
pub(in crate::augmenter) struct InlineFrameDraft {
    pub(in crate::augmenter) source_range: std::ops::Range<usize>,
    pub(in crate::augmenter) content_start: Option<usize>,
    pub(in crate::augmenter) content_end: usize,
    pub(in crate::augmenter) kind: InlineFrameKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::augmenter) enum InlineFrameKind {
    Emphasis,
    Strong,
    Strikethrough,
    Link,
    Code,
}

#[derive(Debug)]
pub(in crate::augmenter) struct InlineSplitFrame<'a> {
    pub(in crate::augmenter) opening: &'a str,
    pub(in crate::augmenter) closing: &'a str,
}

#[derive(Debug)]
pub(in crate::augmenter) struct InlineWrapperFrame<'a> {
    pub(in crate::augmenter) source_range: std::ops::Range<usize>,
    pub(in crate::augmenter) content_start: usize,
    pub(in crate::augmenter) content_end: usize,
    pub(in crate::augmenter) opening: &'a str,
    pub(in crate::augmenter) closing: &'a str,
    pub(in crate::augmenter) kind: InlineFrameKind,
}

pub(in crate::augmenter) fn preserve_inline_elements_at_split(
    source: &str,
    split_at: usize,
    mut augmentation: EditAugmentation,
) -> EditAugmentation {
    let inline_split_at = inline_safe_split_point(source, split_at);
    let frames = inline_split_frames(source, inline_split_at);
    if frames.is_empty() {
        return augmentation;
    }

    let closing_len = frames.iter().map(|frame| frame.closing.len()).sum::<usize>();
    let opening_len = frames.iter().map(|frame| frame.opening.len()).sum::<usize>();
    let original_insertion = augmentation.insert_text.take().unwrap_or_default();
    let preserved_separator = source.get(inline_split_at..split_at).unwrap_or_default();
    let mut insertion = String::with_capacity(
        closing_len + preserved_separator.len() + original_insertion.len() + opening_len,
    );
    for frame in &frames {
        insertion.push_str(frame.closing);
    }
    insertion.push_str(preserved_separator);
    insertion.push_str(&original_insertion);
    for frame in frames.iter().rev() {
        insertion.push_str(frame.opening);
    }

    augmentation.insert_text = Some(insertion);
    if inline_split_at < split_at {
        let replace_range = augmentation.replace_range.get_or_insert(split_at..split_at);
        if replace_range.start == split_at {
            replace_range.start = inline_split_at;
        }
    }
    augmentation.cursor_byte_after += closing_len + opening_len;
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

pub(in crate::augmenter) fn inline_safe_split_point(source: &str, split_at: usize) -> usize {
    let Some(previous_byte) = split_at.checked_sub(1) else {
        return split_at;
    };
    let bytes = source.as_bytes();
    if bytes.get(previous_byte) != Some(&b' ') {
        return split_at;
    }
    let belongs_to_space_run = previous_byte > 0 && bytes.get(previous_byte - 1) == Some(&b' ')
        || bytes.get(split_at) == Some(&b' ');
    if belongs_to_space_run { split_at } else { previous_byte }
}

pub(in crate::augmenter) fn inline_split_frames(
    source: &str,
    split_at: usize,
) -> Vec<InlineSplitFrame<'_>> {
    inline_wrapper_frames(source)
        .into_iter()
        .filter(|frame| split_at >= frame.content_start && split_at <= frame.content_end)
        .map(|frame| InlineSplitFrame { opening: frame.opening, closing: frame.closing })
        .collect()
}

pub(in crate::augmenter) fn inline_wrapper_frames(source: &str) -> Vec<InlineWrapperFrame<'_>> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut drafts = Vec::<InlineFrameDraft>::new();
    let mut completed = Vec::new();
    for (event, source_range) in
        Parser::new_ext(source, crate::parser::markdown_options()).into_offset_iter()
    {
        match event {
            Event::Start(Tag::Emphasis) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Emphasis,
            }),
            Event::Start(Tag::Strong) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Strong,
            }),
            Event::Start(Tag::Strikethrough) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Strikethrough,
            }),
            Event::Start(Tag::Link { .. }) => drafts.push(InlineFrameDraft {
                source_range,
                content_start: None,
                content_end: 0,
                kind: InlineFrameKind::Link,
            }),
            Event::Text(_) | Event::InlineHtml(_) | Event::Html(_) => {
                record_inline_content(&mut drafts, &source_range);
            }
            Event::Code(code) => {
                record_inline_content(&mut drafts, &source_range);
                if let Some(frame) = inline_code_wrapper_frame(source, source_range, &code) {
                    completed.push(frame);
                }
            }
            Event::End(
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link,
            ) => {
                let Some(draft) = drafts.pop() else {
                    continue;
                };
                let Some(content_start) = draft.content_start else {
                    continue;
                };
                let Some(opening) = source.get(draft.source_range.start..content_start) else {
                    continue;
                };
                let Some(closing) = source.get(draft.content_end..draft.source_range.end) else {
                    continue;
                };
                if !opening.is_empty() && !closing.is_empty() {
                    completed.push(InlineWrapperFrame {
                        source_range: draft.source_range,
                        content_start,
                        content_end: draft.content_end,
                        opening,
                        closing,
                        kind: draft.kind,
                    });
                }
            }
            _ => {}
        }
    }
    completed
}

pub(in crate::augmenter) fn record_inline_content(
    drafts: &mut [InlineFrameDraft],
    source_range: &std::ops::Range<usize>,
) {
    for draft in drafts {
        draft.content_start = Some(
            draft.content_start.map_or(source_range.start, |start| start.min(source_range.start)),
        );
        draft.content_end = draft.content_end.max(source_range.end);
    }
}

pub(in crate::augmenter) fn inline_code_wrapper_frame<'a>(
    source: &'a str,
    source_range: std::ops::Range<usize>,
    code: &str,
) -> Option<InlineWrapperFrame<'a>> {
    let code_source = source.get(source_range.clone())?;
    let content_offset = code_source.find(code)?;
    let content_start = source_range.start + content_offset;
    let content_end = content_start + code.len();
    let opening = source.get(source_range.start..content_start)?;
    let closing = source.get(content_end..source_range.end)?;
    (!opening.is_empty() && !closing.is_empty()).then_some(InlineWrapperFrame {
        source_range,
        content_start,
        content_end,
        opening,
        closing,
        kind: InlineFrameKind::Code,
    })
}

pub(in crate::augmenter) fn cursor_touches_source_newline(
    source: &str,
    current_byte: usize,
) -> bool {
    newline_sequence_width_at(source, current_byte).is_some()
        || newline_sequence_width_before(source, current_byte).is_some()
}
