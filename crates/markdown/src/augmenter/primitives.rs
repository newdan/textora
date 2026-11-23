use super::*;

// ─── emit_* 原语 ──────────────────────────────────────────────────────────

/// 在当前位置插入单个 `\n`。用于块间空行 run 里"加空行"的语义（Typora 风格）。
pub(in crate::augmenter) fn emit_source_newline(
    source: &str,
    current_byte: usize,
) -> EditAugmentation {
    let insertion = String::from(preferred_newline_sequence(source, current_byte));
    let aug = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    aug
}

pub(in crate::augmenter) fn emit_literal_source_newline(
    source: &str,
    current_byte: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = format!("{newline}{continuation_prefix}");
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        replace_range: Some(current_byte..current_byte),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

pub(in crate::augmenter) fn emit_fenced_source_newline(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> EditAugmentation {
    let continuation_prefix = fenced_continuation_prefix(source, current_byte).unwrap_or_default();
    emit_literal_source_newline(source, current_byte, &continuation_prefix)
}

pub(in crate::augmenter) fn fenced_continuation_prefix(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<String> {
    let boundary = enclosing_fenced_boundary(source, current_byte)?;
    let mut prefix = String::new();
    for segment in boundary.frame.container_path.0 {
        match segment {
            ContainerSegment::BlockQuote => prefix.push_str("> "),
            ContainerSegment::IndentColumns(columns) => prefix.push_str(&" ".repeat(columns)),
        }
    }
    Some(prefix)
}

/// Create one editable paragraph using the same separator roles as the layout tree.
pub(in crate::augmenter) fn emit_block_break(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = newline.repeat(block_break_newline_count(source, current_byte));
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + newline.len() * BLOCK_BOUNDARY_NEWLINE_COUNT,
        insert_text: Some(insertion),
        ..Default::default()
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

pub(in crate::augmenter) fn block_break_newline_count(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> usize {
    let Some(newline_width) = newline_sequence_width_at(source, current_byte) else {
        return BLOCK_BOUNDARY_NEWLINE_COUNT;
    };
    let next_line_start = current_byte + newline_width;
    if blank_source_line_width(source, next_line_start).is_none() {
        return BLOCK_BOUNDARY_NEWLINE_COUNT;
    }
    let document = source.structure();
    let paragraphs = crate::builder::EditableParagraphMap::from_blocks(&document.blocks, source);
    let Some(run) = paragraphs.run_at_byte(next_line_start) else {
        return BLOCK_BOUNDARY_NEWLINE_COUNT;
    };
    let separator_becomes_hidden = run.has_preceding_block && run.hidden_separator_count == 0;
    1 + usize::from(separator_becomes_hidden)
}

pub(in crate::augmenter) fn emit_paragraph_before(
    source: &MarkdownSource<'_>,
    current_byte: usize,
    insert_at: usize,
    continuation_prefix: &str,
) -> EditAugmentation {
    let list_start = leading_heading_list_start(source, insert_at);
    let container_prefix;
    let (insert_at, continuation_prefix) = if let Some(marker_start) = list_start {
        container_prefix = canonical_container_prefix(source, marker_start);
        (marker_start, container_prefix.as_str())
    } else {
        (insert_at, continuation_prefix)
    };
    let insertion_unit =
        format!("{}{continuation_prefix}", preferred_newline_sequence(source, insert_at));
    let insertion = insertion_unit.repeat(preceding_paragraph_insertion_count(
        source,
        insert_at,
        &insertion_unit,
    ));
    let augmentation = EditAugmentation {
        cursor_byte_after: current_byte + insertion.len(),
        replace_range: Some(insert_at..insert_at),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&augmentation, source);
    augmentation
}

pub(in crate::augmenter) fn preceding_paragraph_insertion_count(
    source: &str,
    insert_at: usize,
    insertion: &str,
) -> usize {
    let mut prospective = source.to_owned();
    prospective.insert_str(insert_at, insertion);
    let prospective = MarkdownSource::owned(prospective);
    let document = prospective.structure();
    let paragraphs =
        crate::builder::EditableParagraphMap::from_blocks(&document.blocks, &prospective);
    let hidden = paragraphs.run_at_byte(insert_at).is_some_and(|run| {
        run.lines
            .iter()
            .position(|line| {
                line.source_range.start <= insert_at && insert_at <= line.source_range.end
            })
            .is_some_and(|index| index < run.hidden_separator_count)
    });
    1 + usize::from(hidden)
}

pub(in crate::augmenter) fn leading_heading_list_start(
    source: &MarkdownSource<'_>,
    heading_start: usize,
) -> Option<usize> {
    struct LeadingListItem {
        marker_start: usize,
        has_content: bool,
    }
    let mut owners = Vec::<LeadingListItem>::new();
    for (event, range) in source.parsed().source_events() {
        if matches!(event, MarkdownEvent::Start(MarkdownTag::Heading { .. }))
            && range.start == heading_start
        {
            return owners.iter().find(|owner| !owner.has_content).map(|owner| owner.marker_start);
        }
        match event {
            MarkdownEvent::Start(MarkdownTag::Item) => {
                owners.push(LeadingListItem { marker_start: range.start, has_content: false })
            }
            MarkdownEvent::End(MarkdownTagEnd::Item) => {
                owners.pop();
            }
            MarkdownEvent::Text(_)
            | MarkdownEvent::Code(_)
            | MarkdownEvent::HtmlBlock(_)
            | MarkdownEvent::InlineHtml(_)
            | MarkdownEvent::Rule
            | MarkdownEvent::Start(
                MarkdownTag::CodeBlock { .. }
                | MarkdownTag::Heading { .. }
                | MarkdownTag::Table(_)
                | MarkdownTag::Image { .. },
            ) => {
                for owner in &mut owners {
                    owner.has_content = true;
                }
            }
            _ => {}
        }
        if range.start > heading_start {
            return None;
        }
    }
    None
}

pub(in crate::augmenter) fn is_visible_content_start(
    source: &MarkdownSource<'_>,
    source_start: usize,
    cursor: usize,
) -> bool {
    cursor == source_start
        || inline_wrapper_frames(source)
            .iter()
            .any(|frame| frame.source_range.start == source_start && frame.content_start == cursor)
}

/// 用一个块边界替换现有硬换行源码，光标落在新块起点。
pub(in crate::augmenter) fn emit_block_break_replacing(
    source: &str,
    replaced: std::ops::Range<usize>,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, replaced.start);
    let insertion = newline.repeat(BLOCK_BOUNDARY_NEWLINE_COUNT);
    let aug = EditAugmentation {
        cursor_byte_after: replaced.start + insertion.len(),
        replace_range: Some(replaced),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    aug
}

/// 与 [`emit_block_break`] 相同，但插入点由上下文指定而不是使用原光标。
pub(in crate::augmenter) fn emit_block_break_at(
    source: &MarkdownSource<'_>,
    insert_at: usize,
) -> EditAugmentation {
    let mut aug = emit_block_break(source, insert_at);
    aug.replace_range = Some(insert_at..insert_at);
    debug_assert_augmentation(&aug, source);
    aug
}

pub(in crate::augmenter) fn backspace_empty_source_line(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    editable_paragraph_navigation::backspace(source, current_byte)
}

pub(in crate::augmenter) fn contiguous_newline_count_before(source: &str, byte: usize) -> usize {
    let mut newline_count = 0;
    let mut run_start = byte.min(source.len());
    while let Some(newline_width) = newline_sequence_width_before(source, run_start) {
        run_start -= newline_width;
        newline_count += 1;
    }
    newline_count
}

fn quote_marker_count(prefix: &str) -> usize {
    prefix.bytes().filter(|&byte| byte == b'>').count()
}

fn merge_boundary_delete_start(
    source: &str,
    boundary_start: usize,
    context: &EnterContext,
) -> usize {
    if matches!(context, EnterContext::Heading { .. }) {
        atx_heading_merge_end(source, boundary_start)
    } else {
        hard_break_marker_ending_at(source, boundary_start)
            .map_or(boundary_start, |marker| marker.start)
    }
}

fn backspace_quote_boundary(
    source: &str,
    boundary_start: usize,
    current_byte: usize,
    current_prefix: &str,
    context: &EnterContext,
) -> Option<EditAugmentation> {
    let prev_prefix = match context {
        EnterContext::BlockQuoteLine { continuation_prefix, .. }
        | EnterContext::Heading { continuation_prefix, .. } => Some(continuation_prefix),
        _ => None,
    }?;
    if quote_marker_count(prev_prefix) != quote_marker_count(current_prefix)
        || prev_prefix != current_prefix
    {
        return None;
    }
    let delete_start = merge_boundary_delete_start(source, boundary_start, context);
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(delete_start..current_byte),
        cursor_byte_after: delete_start,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

pub(in crate::augmenter) fn backspace_paragraph_boundary(
    source: &MarkdownSource<'_>,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    if line_start == 0 || line_start == line_end {
        return None;
    }
    let at_visual_line_start = current_byte == line_start
        || (current_byte < line_end
            && !source[current_byte..].starts_with([' ', '\t', '\r', '\n'])
            && source[line_start..current_byte].bytes().all(|byte| matches!(byte, b' ' | b'\t')));
    let at_quote_content_start = !at_visual_line_start
        && line_start < current_byte
        && current_byte < line_end
        && !source[current_byte..].starts_with([' ', '\t', '\r', '\n'])
        && source[line_start..current_byte].bytes().any(|byte| byte == b'>')
        && source[line_start..current_byte].bytes().all(|byte| matches!(byte, b' ' | b'\t' | b'>'));
    if !at_visual_line_start && !at_quote_content_start {
        return None;
    }

    let current_context = classify_source(source, current_byte);
    let current_quote_prefix = if at_quote_content_start {
        let EnterContext::BlockQuoteLine { continuation_prefix, .. } = &current_context else {
            return None;
        };
        Some(continuation_prefix.clone())
    } else if matches!(
        current_context,
        EnterContext::TopLevelParagraphEnd
            | EnterContext::ParagraphInterior
            | EnterContext::ParagraphStart { .. }
    ) {
        None
    } else {
        return None;
    };

    let source_bytes = source.as_bytes();
    let mut boundary_start = line_start;
    while boundary_start > 0 && source_bytes[boundary_start - 1] == b'\n' {
        boundary_start -= 1;
        if boundary_start > 0 && source_bytes[boundary_start - 1] == b'\r' {
            boundary_start -= 1;
        }
    }
    if let Some(separator) = editable_paragraph_navigation::hidden_separator_range(
        source,
        line_start,
        editable_paragraph_navigation::Boundary::Start,
    ) {
        boundary_start = separator.start;
    }
    if boundary_start == line_start {
        return None;
    }

    let context = classify_source(source, boundary_start);
    if let Some(current_prefix) = current_quote_prefix {
        return backspace_quote_boundary(
            source,
            boundary_start,
            current_byte,
            &current_prefix,
            &context,
        );
    }

    match context {
        EnterContext::TopLevelParagraphEnd
        | EnterContext::ParagraphInterior
        | EnterContext::ParagraphStart { .. }
        | EnterContext::Heading { .. } => {
            let delete_start = merge_boundary_delete_start(source, boundary_start, &context);
            let aug = EditAugmentation {
                insert_text: Some(String::new()),
                replace_range: Some(delete_start..current_byte),
                cursor_byte_after: delete_start,
            };
            debug_assert_augmentation(&aug, source);
            Some(aug)
        }
        EnterContext::BlockQuoteLine { continuation_prefix, .. } => Some(
            merge_into_preceding_block(source, boundary_start, current_byte, &continuation_prefix),
        ),
        EnterContext::ListItem { content_prefix, .. } => {
            Some(merge_into_preceding_block(source, boundary_start, current_byte, &content_prefix))
        }
        _ => guard_unmergeable_leaf_boundary(source, boundary_start, current_byte),
    }
}

/// 段首退格并入引用块/列表项：删除块间空行分隔，为并入行补显式行首前缀
/// （引用的 `> ` marker，或列表延续行的内容列缩进），避免产生无 marker 的
/// 脆弱 lazy continuation 中间态。单条 augmentation 保证一次 undo 完整还原。
pub(in crate::augmenter) fn merge_into_preceding_block(
    source: &str,
    boundary_start: usize,
    current_byte: usize,
    line_prefix: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insert_text = format!("{newline}{line_prefix}");
    let cursor_byte_after = boundary_start + insert_text.len();
    let aug = EditAugmentation {
        insert_text: Some(insert_text),
        replace_range: Some(boundary_start..current_byte),
        cursor_byte_after,
    };
    debug_assert_augmentation(&aug, source);
    aug
}

/// 段首退格且前块为代码块/分隔线/setext 标题等不可安全合并的叶块时：
/// - 边界含空行（≥2 个换行序列）：默认计划只删一个换行，段落上移到叶块下一行，
///   结构保持合法，交回默认计划（返回 `None`）；
/// - 边界只剩单个换行：再删会把段落并到叶块所在行（` ```para `、`---para`、
///   `Title\n===para`），必须拦截。返回一个空操作 augmentation——
///   `insert_text` 为空、无 `replace_range`、光标不动——`view.rs` 的
///   `augmentation_edit_plan` 会把它映射为 `EditPlan::Consume`，
///   阻止默认的逐 grapheme 删除。
pub(in crate::augmenter) fn guard_unmergeable_leaf_boundary(
    source: &str,
    boundary_start: usize,
    current_byte: usize,
) -> Option<EditAugmentation> {
    debug_assert!(boundary_start < current_byte);
    let line_start =
        locate_source_line_bounds(source, current_byte).map_or(current_byte, |(s, _, _)| s);
    if contiguous_newline_count_before(source, line_start) > 1 {
        return None;
    }
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: None,
        cursor_byte_after: current_byte,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

pub(in crate::augmenter) fn matching_hard_break_boundary_before(
    source: &str,
    current_byte: usize,
) -> Option<(std::ops::Range<usize>, bool)> {
    let (line_start, _, _) = locate_source_line_bounds(source, current_byte)?;
    let continuation_prefix = source.get(line_start..current_byte)?;
    if !continuation_prefix_is_joinable(continuation_prefix)
        || line_starts_new_sibling_block(source, line_start)
    {
        return None;
    }

    let newline_width = newline_sequence_width_before(source, line_start)?;
    let previous_content_end = line_start - newline_width;
    let marker = hard_break_marker_ending_at(source, previous_content_end)?;
    let previous_prefix = previous_container_prefix(source, line_start);
    let matches_previous =
        continuation_prefix_matches_prefix(previous_prefix.as_deref(), continuation_prefix)
            && (continuation_prefix.len() == previous_prefix.as_deref().map_or(0, str::len)
                || (!source[current_byte..].starts_with([' ', '\t'])
                    && !line_starts_independent_block(
                        source,
                        line_start,
                        &ContainerPath::default(),
                    )));
    Some((marker.start..current_byte, matches_previous))
}

pub(in crate::augmenter) fn hard_break_boundary_before(
    source: &str,
    current_byte: usize,
) -> Option<std::ops::Range<usize>> {
    matching_hard_break_boundary_before(source, current_byte)
        .and_then(|(boundary, matched)| matched.then_some(boundary))
}

pub(in crate::augmenter) fn backspace_join_hard_break_line(
    source: &str,
    current_byte: usize,
) -> Option<EditAugmentation> {
    let (boundary, matches_previous) = matching_hard_break_boundary_before(source, current_byte)?;
    if !matches_previous {
        return Some(EditAugmentation {
            insert_text: Some(String::new()),
            replace_range: Some(current_byte..current_byte),
            cursor_byte_after: current_byte,
        });
    }
    let cursor_after = boundary.start;
    let aug = EditAugmentation {
        insert_text: Some(String::new()),
        replace_range: Some(boundary),
        cursor_byte_after: cursor_after,
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

fn previous_container_prefix(source: &str, line_start: usize) -> Option<String> {
    let newline_width = newline_sequence_width_before(source, line_start)?;
    let previous_line_end = line_start - newline_width;
    let previous_line_start = source[..previous_line_end]
        .bytes()
        .rposition(|source_byte| source_byte == b'\n')
        .map_or(0, |newline| newline + 1);
    let previous_content_start =
        container_content_start_on_line(source, previous_line_start, previous_line_end);
    Some(canonical_container_prefix(source, previous_content_start))
}

pub(in crate::augmenter) fn continuation_prefix_matches_prefix(
    previous_prefix: Option<&str>,
    continuation_prefix: &str,
) -> bool {
    if continuation_prefix.is_empty() {
        return true;
    }
    let Some(previous_prefix) = previous_prefix else {
        return false;
    };
    if previous_prefix == continuation_prefix {
        return true;
    }
    continuation_prefix
        .strip_prefix(previous_prefix)
        .is_some_and(|extra| extra.bytes().all(|byte| matches!(byte, b' ' | b'\t')))
}

pub(in crate::augmenter) fn container_content_start_on_line(
    source: &str,
    line_start: usize,
    line_end: usize,
) -> usize {
    let bytes = source.as_bytes();
    let mut probe = line_start;
    while probe < line_end && matches!(bytes.get(probe), Some(b' ' | b'\t')) {
        probe += 1;
    }

    loop {
        if let Some((_, content_start)) = parse_list_marker(source, probe)
            && content_start <= line_end
        {
            probe = content_start;
        } else if bytes.get(probe) == Some(&b'>') {
            probe += 1;
            if matches!(bytes.get(probe), Some(b' ' | b'\t')) {
                probe += 1;
            }
        } else {
            return probe;
        }
    }
}

pub(in crate::augmenter) fn continuation_prefix_is_joinable(prefix: &str) -> bool {
    prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t' | b'>'))
}

pub(in crate::augmenter) fn line_starts_new_sibling_block(source: &str, line_start: usize) -> bool {
    block_marker_line(source, line_start, &ContainerPath::default())
        .is_some_and(|marker_line| line_starts_new_sibling_marker(source, &marker_line))
}

pub(in crate::augmenter) fn line_starts_new_sibling_marker(
    source: &str,
    marker_line: &BlockMarkerLine<'_>,
) -> bool {
    parse_list_marker(source, marker_line.marker_start).is_some()
        || heading_source_is_atx(source, marker_line.marker_start)
        || content_is_thematic_break(marker_line.content)
        || marker_line.content.starts_with("```")
        || marker_line.content.starts_with("~~~")
}

/// 用于 list / blockquote 的"续 marker"：插入 `"\n{indent}{marker}"`。
pub(in crate::augmenter) fn emit_marker_break(
    source: &str,
    current_byte: usize,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, current_byte);
    let insertion = format!("{newline}{indent}{marker}");
    let cursor_after = current_byte + insertion.len();
    let aug = EditAugmentation {
        insert_text: Some(insertion),
        cursor_byte_after: cursor_after,
        ..Default::default()
    };
    debug_assert_augmentation(&aug, source);
    aug
}

pub(in crate::augmenter) fn emit_marker_break_at(
    source: &str,
    current_byte: usize,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let mut augmentation = emit_marker_break(source, current_byte, indent, marker);
    augmentation.replace_range = Some(current_byte..current_byte);
    augmentation
}

pub(in crate::augmenter) fn emit_marker_break_replacing(
    source: &str,
    replaced: std::ops::Range<usize>,
    indent: &str,
    marker: &str,
) -> EditAugmentation {
    let newline = preferred_newline_sequence(source, replaced.start);
    let insertion = format!("{newline}{indent}{marker}");
    let aug = EditAugmentation {
        cursor_byte_after: replaced.start + insertion.len(),
        replace_range: Some(replaced),
        insert_text: Some(insertion),
    };
    debug_assert_augmentation(&aug, source);
    aug
}

/// 将当前源码行替换为父容器前缀。空前缀表示退出顶层容器。
pub(in crate::augmenter) fn emit_replace_current_line(
    source: &str,
    current_byte: usize,
    parent_prefix: &str,
) -> Option<EditAugmentation> {
    let (start, _, line_end) = locate_source_line_bounds(source, current_byte)?;
    let content_end = source_line_content_end(source, line_end);
    let aug = EditAugmentation {
        insert_text: Some(parent_prefix.to_owned()),
        replace_range: Some(start..content_end),
        cursor_byte_after: start + parent_prefix.len(),
    };
    debug_assert_augmentation(&aug, source);
    Some(aug)
}

pub(in crate::augmenter) fn debug_assert_augmentation(aug: &EditAugmentation, source: &str) {
    debug_assert!(
        aug.insert_text.is_some() || aug.replace_range.is_some(),
        "augmentation must produce at least one of insert_text / replace_range"
    );
    if let Some(range) = &aug.replace_range {
        debug_assert!(
            range.end <= source.len(),
            "replace_range.end {} exceeds source.len() {}",
            range.end,
            source.len()
        );
        debug_assert!(range.start <= range.end);
    }
}
