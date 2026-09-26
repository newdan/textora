use ui::plugin::{SemanticEditCommand, SemanticEditPlan};

mod table_structure;
pub use table_structure::{TableStructureCapabilities, table_structure_capabilities};
#[cfg(test)]
use table_structure::{format_table_row, source_table_at};

pub fn plan_semantic_edit(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    command: SemanticEditCommand,
) -> SemanticEditPlan {
    if cursor_byte > source.len() || !source.is_char_boundary(cursor_byte) {
        return SemanticEditPlan::NoChange;
    }
    if selection.as_ref().is_some_and(|range| {
        range.start > range.end
            || range.end > source.len()
            || !source.is_char_boundary(range.start)
            || !source.is_char_boundary(range.end)
    }) {
        return SemanticEditPlan::NoChange;
    }

    match command {
        SemanticEditCommand::Undo | SemanticEditCommand::Redo => SemanticEditPlan::Unsupported,
        SemanticEditCommand::SetHeadingLevel(level) => {
            plan_heading(source, source_generation, cursor_byte, selection, level)
        }
        SemanticEditCommand::ToggleBold => {
            plan_inline_toggle(source, source_generation, cursor_byte, selection, "**")
        }
        SemanticEditCommand::ToggleItalic => {
            plan_inline_toggle(source, source_generation, cursor_byte, selection, "*")
        }
        SemanticEditCommand::ToggleStrikethrough => {
            plan_inline_toggle(source, source_generation, cursor_byte, selection, "~~")
        }
        SemanticEditCommand::ToggleInlineCode => {
            plan_inline_code(source, source_generation, cursor_byte, selection)
        }
        SemanticEditCommand::UnorderedList => plan_line_prefix(
            source,
            source_generation,
            cursor_byte,
            selection,
            LinePrefix::Unordered,
        ),
        SemanticEditCommand::OrderedList => {
            plan_line_prefix(source, source_generation, cursor_byte, selection, LinePrefix::Ordered)
        }
        SemanticEditCommand::TaskList => {
            plan_line_prefix(source, source_generation, cursor_byte, selection, LinePrefix::Task)
        }
        SemanticEditCommand::Quote => {
            plan_line_prefix(source, source_generation, cursor_byte, selection, LinePrefix::Quote)
        }
        SemanticEditCommand::CodeBlock => {
            plan_code_block(source, source_generation, cursor_byte, selection)
        }
        SemanticEditCommand::InsertLink => {
            plan_link(source, source_generation, cursor_byte, selection)
        }
        SemanticEditCommand::InsertTable { columns, rows } => {
            plan_insert_table(source, source_generation, cursor_byte, selection, columns, rows)
        }
        SemanticEditCommand::TableStructure(operation) => {
            table_structure::plan_table_structure(source, source_generation, cursor_byte, operation)
        }
        SemanticEditCommand::PromoteObject | SemanticEditCommand::DemoteObject => {
            SemanticEditPlan::Unsupported
        }
    }
}

const CODE_MARKER: &str = "\x60";
const LF_SEQUENCE: &str = "\n";
const CRLF_SEQUENCE: &str = "\r\n";
const MAX_TABLE_COLUMNS: usize = 64;
const MAX_TABLE_ROWS: usize = 1_000;

fn plan_insert_table(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    columns: usize,
    rows: usize,
) -> SemanticEditPlan {
    if selection.is_some_and(|range| range.start < range.end)
        || columns == 0
        || columns > MAX_TABLE_COLUMNS
        || !(2..=MAX_TABLE_ROWS).contains(&rows)
    {
        return SemanticEditPlan::Unsupported;
    }

    let newline = if source.contains(CRLF_SEQUENCE) { CRLF_SEQUENCE } else { LF_SEQUENCE };
    let table_source = generate_empty_table(columns, rows, newline);
    if !parsed_table_has_columns(&table_source, columns) {
        return SemanticEditPlan::Unsupported;
    }
    if let Some(context) = container_paragraph_context(source, cursor_byte) {
        if cursor_byte < context.content_start {
            return SemanticEditPlan::Unsupported;
        }
        let paragraph_prefix = &source[context.range.start..context.content_start];
        let paragraph_before = &source[context.content_start..cursor_byte];
        let paragraph_after = &source[cursor_byte..context.range.end];
        let container_table = prefix_table_rows(&table_source, &context.row_prefix, newline);
        let separator = if context.separator_prefix.is_empty() {
            format!("{newline}{newline}")
        } else {
            format!("{newline}{}{newline}", context.separator_prefix)
        };
        let before_paragraph = format!("{paragraph_prefix}{paragraph_before}");
        let after_paragraph = if paragraph_after.is_empty() {
            String::new()
        } else {
            format!("{}{paragraph_after}", context.row_prefix)
        };
        let replacement = if after_paragraph.is_empty() {
            format!("{before_paragraph}{separator}{container_table}")
        } else {
            format!("{before_paragraph}{separator}{container_table}{separator}{after_paragraph}")
        };
        let result = format!(
            "{}{replacement}{}",
            &source[..context.range.start],
            &source[context.range.end..]
        );
        if parsed_table_count(&result, columns) != parsed_table_count(source, columns) + 1 {
            return SemanticEditPlan::Unsupported;
        }
        let cursor_after = context.range.start
            + before_paragraph.len()
            + separator.len()
            + context.row_prefix.len()
            + 2;
        return apply_transaction(source_generation, context.range, replacement, cursor_after);
    }
    let replacement_range = if source.is_empty() {
        0..0
    } else if let Some(range) = top_level_paragraph_range(source, cursor_byte) {
        range
    } else if let Some(range) = empty_line_range(source, cursor_byte) {
        let replacement = format!("{newline}{table_source}{newline}");
        let result = format!("{}{replacement}{}", &source[..range.start], &source[range.end..]);
        if parsed_table_count(&result, columns) != parsed_table_count(source, columns) + 1 {
            return SemanticEditPlan::Unsupported;
        }
        let cursor_after = range.start + newline.len() + 2;
        return apply_transaction(source_generation, range, replacement, cursor_after);
    } else {
        return SemanticEditPlan::Unsupported;
    };
    let paragraph_prefix = &source[replacement_range.start..cursor_byte];
    let paragraph_suffix = &source[cursor_byte..replacement_range.end];
    let replacement = match (paragraph_prefix.is_empty(), paragraph_suffix.is_empty()) {
        (true, true) => table_source.clone(),
        (true, false) => format!("{table_source}{newline}{newline}{paragraph_suffix}"),
        (false, true) => format!("{paragraph_prefix}{newline}{newline}{table_source}"),
        (false, false) => format!(
            "{paragraph_prefix}{newline}{newline}{table_source}{newline}{newline}{paragraph_suffix}"
        ),
    };
    let result = format!(
        "{}{replacement}{}",
        &source[..replacement_range.start],
        &source[replacement_range.end..]
    );
    if parsed_table_count(&result, columns) != parsed_table_count(source, columns) + 1 {
        return SemanticEditPlan::Unsupported;
    }
    let cursor_after = replacement_range.start
        + paragraph_prefix.len()
        + if paragraph_prefix.is_empty() { 0 } else { newline.len() * 2 }
        + 2;
    apply_transaction(source_generation, replacement_range, replacement, cursor_after)
}

fn generate_empty_table(columns: usize, rows: usize, newline: &str) -> String {
    let empty_cells = "| ".repeat(columns);
    let row_suffix = "|";
    let header = format!("{empty_cells}{row_suffix}");
    let separator = format!("{}|", "| --- ".repeat(columns));
    let body_rows = (0..rows - 1).map(|_| header.as_str()).collect::<Vec<_>>().join(newline);
    format!("{header}{newline}{separator}{newline}{body_rows}")
}

fn prefix_table_rows(table_source: &str, row_prefix: &str, newline: &str) -> String {
    table_source
        .split(newline)
        .map(|row| format!("{row_prefix}{row}"))
        .collect::<Vec<_>>()
        .join(newline)
}

fn parsed_table_has_columns(source: &str, columns: usize) -> bool {
    parsed_table_count(source, columns) > 0
}

fn parsed_table_count(source: &str, columns: usize) -> usize {
    use crate::parser::{MarkdownEvent, MarkdownTag};
    crate::parser::parse_markdown(source)
        .events
        .iter()
        .filter(|event| matches!(event, MarkdownEvent::Start(MarkdownTag::Table(alignments)) if alignments.len() == columns))
        .count()
}

fn top_level_paragraph_range(source: &str, cursor_byte: usize) -> Option<std::ops::Range<usize>> {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
    let parsed = crate::parser::parse_markdown(source);
    let mut block_depth = 0_usize;
    let mut paragraph_start = None;
    for (index, event) in parsed.events.iter().enumerate() {
        match event {
            MarkdownEvent::Start(MarkdownTag::Paragraph) if block_depth == 0 => {
                paragraph_start = Some(index);
            }
            MarkdownEvent::End(MarkdownTagEnd::Paragraph) => {
                if let Some(start_index) = paragraph_start.take() {
                    let start = parsed.event_ranges[start_index].start;
                    let end = parsed.event_ranges[index].end;
                    let line_start = source[..start].rfind('\n').map_or(0, |newline| newline + 1);
                    let mut line_end =
                        source[end..].find('\n').map_or(source.len(), |offset| end + offset);
                    if line_end > line_start && source.as_bytes()[line_end - 1] == b'\n' {
                        line_end -= 1;
                    }
                    if line_end > line_start && source.as_bytes()[line_end - 1] == b'\r' {
                        line_end -= 1;
                    }
                    if cursor_byte >= line_start && cursor_byte <= line_end {
                        return Some(line_start..line_end);
                    }
                }
            }
            MarkdownEvent::Start(
                MarkdownTag::BlockQuote
                | MarkdownTag::List(..)
                | MarkdownTag::Item
                | MarkdownTag::Table(_),
            ) => block_depth += 1,
            MarkdownEvent::End(
                MarkdownTagEnd::BlockQuote
                | MarkdownTagEnd::List
                | MarkdownTagEnd::Item
                | MarkdownTagEnd::Table,
            ) => block_depth = block_depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

struct ContainerParagraphContext {
    range: std::ops::Range<usize>,
    row_prefix: String,
    separator_prefix: String,
    content_start: usize,
}

fn container_paragraph_context(
    source: &str,
    cursor_byte: usize,
) -> Option<ContainerParagraphContext> {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
    let parsed = crate::parser::parse_markdown(source);
    for (start_index, event) in parsed.events.iter().enumerate() {
        if !matches!(event, MarkdownEvent::Start(MarkdownTag::Paragraph)) {
            continue;
        }
        let Some(end_index) = (start_index + 1..parsed.events.len()).find(|index| {
            matches!(parsed.events[*index], MarkdownEvent::End(MarkdownTagEnd::Paragraph))
        }) else {
            continue;
        };
        let paragraph_start = parsed.event_ranges[start_index].start;
        let paragraph_end = parsed.event_ranges[end_index].end;
        let line_start = source[..paragraph_start].rfind('\n').map_or(0, |newline| newline + 1);
        let mut line_end = source[paragraph_end..]
            .find('\n')
            .map_or(source.len(), |offset| paragraph_end + offset);
        if line_end > line_start && source.as_bytes()[line_end - 1] == b'\n' {
            line_end -= 1;
        }
        if line_end > line_start && source.as_bytes()[line_end - 1] == b'\r' {
            line_end -= 1;
        }
        if cursor_byte < line_start || cursor_byte > line_end {
            continue;
        }
        let line = &source[line_start..line_end];
        let (row_prefix, separator_prefix, content_offset) = container_prefixes(line)?;
        return Some(ContainerParagraphContext {
            range: line_start..line_end,
            row_prefix,
            separator_prefix,
            content_start: line_start + content_offset,
        });
    }
    let line_start = source[..cursor_byte].rfind('\n').map_or(0, |newline| newline + 1);
    let mut line_end =
        source[cursor_byte..].find('\n').map_or(source.len(), |offset| cursor_byte + offset);
    if line_end > line_start && source.as_bytes()[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let line = &source[line_start..line_end];
    let (row_prefix, separator_prefix, content_offset) = container_prefixes(line)?;
    Some(ContainerParagraphContext {
        range: line_start..line_end,
        row_prefix,
        separator_prefix,
        content_start: line_start + content_offset,
    })
}

fn container_prefixes(line: &str) -> Option<(String, String, usize)> {
    let indentation = line.len() - line.trim_start().len();
    let mut remaining = &line[indentation..];
    let mut quote_prefix = " ".repeat(indentation);
    let mut has_quote = false;
    while let Some(after_marker) = remaining.strip_prefix('>') {
        has_quote = true;
        quote_prefix.push('>');
        if let Some(after_space) = after_marker.strip_prefix(' ') {
            quote_prefix.push(' ');
            remaining = after_space;
        } else {
            remaining = after_marker;
        }
        let continuation_indentation = remaining.len() - remaining.trim_start().len();
        quote_prefix.push_str(&" ".repeat(continuation_indentation));
        remaining = &remaining[continuation_indentation..];
    }
    if let Some(marker_end) = list_marker_content_start(remaining) {
        let continuation_prefix =
            format!("{}{spaces}", quote_prefix, spaces = " ".repeat(marker_end));
        let separator_prefix = if has_quote { quote_prefix } else { String::new() };
        let content_offset = line.len() - remaining.len() + marker_end;
        return Some((continuation_prefix, separator_prefix, content_offset));
    }
    has_quote.then(|| {
        let content_offset = line.len() - remaining.len();
        (quote_prefix.clone(), quote_prefix, content_offset)
    })
}

fn list_marker_content_start(line: &str) -> Option<usize> {
    let first = line.chars().next()?;
    if matches!(first, '-' | '+' | '*') {
        let marker_end = first.len_utf8();
        return line[marker_end..].starts_with(char::is_whitespace).then(|| marker_end + 1);
    }
    let digit_end = line.bytes().position(|byte| !byte.is_ascii_digit())?;
    if digit_end == 0 || !matches!(line.as_bytes().get(digit_end), Some(b'.' | b')')) {
        return None;
    }
    let marker_end = digit_end + 1;
    line[marker_end..].starts_with(char::is_whitespace).then(|| marker_end + 1)
}

fn empty_line_range(source: &str, cursor_byte: usize) -> Option<std::ops::Range<usize>> {
    if cursor_is_inside_container(source, cursor_byte) {
        return None;
    }
    let line_start = source[..cursor_byte].rfind('\n').map_or(0, |newline| newline + 1);
    let mut line_end =
        source[cursor_byte..].find('\n').map_or(source.len(), |offset| cursor_byte + offset);
    if line_end > line_start && source.as_bytes()[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let line_range = line_start..line_end;
    source[line_range.clone()].trim().is_empty().then_some(line_range)
}

fn cursor_is_inside_container(source: &str, cursor_byte: usize) -> bool {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ContainerKind {
        Quote,
        List,
        Item,
    }

    let parsed = crate::parser::parse_markdown(source);
    let mut open_containers = Vec::new();
    for (index, event) in parsed.events.iter().enumerate() {
        let range = &parsed.event_ranges[index];
        let opening = match event {
            MarkdownEvent::Start(MarkdownTag::BlockQuote) => Some(ContainerKind::Quote),
            MarkdownEvent::Start(MarkdownTag::List(..)) => Some(ContainerKind::List),
            MarkdownEvent::Start(MarkdownTag::Item) => Some(ContainerKind::Item),
            _ => None,
        };
        if let Some(kind) = opening {
            open_containers.push((kind, range.start));
            continue;
        }
        let closing = match event {
            MarkdownEvent::End(MarkdownTagEnd::BlockQuote) => Some(ContainerKind::Quote),
            MarkdownEvent::End(MarkdownTagEnd::List) => Some(ContainerKind::List),
            MarkdownEvent::End(MarkdownTagEnd::Item) => Some(ContainerKind::Item),
            _ => None,
        };
        let Some(kind) = closing else {
            continue;
        };
        if let Some(open_index) =
            open_containers.iter().rposition(|(open_kind, _)| *open_kind == kind)
        {
            let (_, start) = open_containers.remove(open_index);
            if cursor_byte >= start && cursor_byte <= range.end {
                return true;
            }
        }
    }
    false
}

#[derive(Clone, Copy)]
enum LinePrefix {
    Unordered,
    Ordered,
    Task,
    Quote,
}

fn plan_inline_toggle(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    marker: &str,
) -> SemanticEditPlan {
    let Some(range) = selection.filter(|range| range.start < range.end) else {
        let replacement = format!("{marker}{marker}");
        return apply_transaction(
            source_generation,
            cursor_byte..cursor_byte,
            replacement,
            cursor_byte + marker.len(),
        );
    };
    let selected = &source[range.clone()];
    if selected.len() >= marker.len() * 2
        && selected.starts_with(marker)
        && selected.ends_with(marker)
    {
        let unwrapped = selected[marker.len()..selected.len() - marker.len()].to_owned();
        return apply_transaction(
            source_generation,
            range.clone(),
            unwrapped.clone(),
            range.start + unwrapped.len(),
        );
    }
    let replacement = format!("{marker}{selected}{marker}");
    apply_transaction(
        source_generation,
        range.clone(),
        replacement.clone(),
        range.start + replacement.len(),
    )
}

fn plan_inline_code(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
) -> SemanticEditPlan {
    let Some(range) = selection.filter(|range| range.start < range.end) else {
        return apply_transaction(
            source_generation,
            cursor_byte..cursor_byte,
            CODE_MARKER.repeat(2),
            cursor_byte + CODE_MARKER.len(),
        );
    };
    let selected = &source[range.clone()];
    let replacement = unwrap_inline_code(selected).unwrap_or_else(|| wrap_inline_code(selected));
    apply_transaction(
        source_generation,
        range.clone(),
        replacement.clone(),
        range.start + replacement.len(),
    )
}

fn wrap_inline_code(content: &str) -> String {
    let delimiter = CODE_MARKER.repeat(longest_backtick_run(content).saturating_add(1));
    let needs_padding = inline_code_needs_boundary_padding(content);
    let padding = if needs_padding { " " } else { "" };
    format!("{delimiter}{padding}{content}{padding}{delimiter}")
}

fn unwrap_inline_code(source: &str) -> Option<String> {
    let opening_length = source.bytes().take_while(|byte| *byte == b'`').count();
    let closing_length = source.bytes().rev().take_while(|byte| *byte == b'`').count();
    if opening_length == 0 || opening_length != closing_length || source.len() < opening_length * 2
    {
        return None;
    }
    let closing_start = source.len() - opening_length;
    if !source[closing_start..].bytes().all(|byte| byte == b'`') {
        return None;
    }
    let content = &source[opening_length..closing_start];
    if longest_backtick_run(content) >= opening_length {
        return None;
    }
    if content.len() >= 2
        && content.starts_with(' ')
        && content.ends_with(' ')
        && !content.bytes().all(|byte| byte == b' ')
    {
        return Some(content[1..content.len() - 1].to_owned());
    }
    Some(content.to_owned())
}

fn inline_code_needs_boundary_padding(content: &str) -> bool {
    if content.is_empty() || content.chars().all(char::is_whitespace) {
        return false;
    }
    matches!(content.chars().next(), Some('`' | ' '))
        || matches!(content.chars().next_back(), Some('`' | ' '))
}

fn longest_backtick_run(content: &str) -> usize {
    content.split(|character| character != '`').map(str::len).max().unwrap_or(0)
}

fn plan_link(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
) -> SemanticEditPlan {
    let Some(range) = selection.filter(|range| range.start < range.end) else {
        return apply_transaction(
            source_generation,
            cursor_byte..cursor_byte,
            "[](https://)".to_owned(),
            cursor_byte + 1,
        );
    };
    let replacement = format!("[{}](https://)", &source[range.clone()]);
    apply_transaction(
        source_generation,
        range.clone(),
        replacement.clone(),
        range.start + replacement.len(),
    )
}

fn plan_heading(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    level: u8,
) -> SemanticEditPlan {
    if !(1..=6).contains(&level) {
        return SemanticEditPlan::Unsupported;
    }
    let target = line_selection_range(source, cursor_byte, selection.as_ref());
    if let Some(title_range) = first_h1_range(source)
        && ranges_overlap(&target, &title_range)
    {
        return SemanticEditPlan::NoChange;
    }
    let replacement = transform_lines(&source[target.clone()], |line| {
        let indentation = leading_whitespace(line);
        let content = &line[indentation..];
        let content = if crate::augmenter::heading_source_is_atx(line, 0) {
            content.trim_start_matches('#').trim_start_matches([' ', '\t'])
        } else {
            content
        };
        format!("{}{} {}", &line[..indentation], "#".repeat(level as usize), content)
    });
    apply_line_transaction(source, source_generation, target, replacement)
}

fn plan_line_prefix(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    prefix: LinePrefix,
) -> SemanticEditPlan {
    let target = line_selection_range(source, cursor_byte, selection.as_ref());
    let segment = &source[target.clone()];
    let remove_existing = segment
        .split('\n')
        .all(|line| has_line_prefix(line.strip_suffix('\r').unwrap_or(line), prefix));
    let replacement = transform_lines(segment, |line| {
        if remove_existing {
            remove_line_prefix(line, prefix)
        } else {
            add_line_prefix(line, prefix)
        }
    });
    apply_line_transaction(source, source_generation, target, replacement)
}

fn plan_code_block(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
) -> SemanticEditPlan {
    let target = line_selection_range(source, cursor_byte, selection.as_ref());
    let segment = &source[target.clone()];
    let replacement = unwrap_code_fence(segment).unwrap_or_else(|| {
        let fence = CODE_MARKER.repeat(longest_backtick_run(segment).saturating_add(1).max(3));
        let newline = if source.find('\n').is_some_and(|index| source[..index].ends_with('\r')) {
            CRLF_SEQUENCE
        } else {
            LF_SEQUENCE
        };
        format!("{fence}{newline}{segment}{newline}{fence}")
    });
    apply_line_transaction(source, source_generation, target, replacement)
}

fn unwrap_code_fence(segment: &str) -> Option<String> {
    let (opening_line, after_opening) = segment.split_once('\n')?;
    let opening_marker = opening_line.trim_start();
    let fence_length = opening_marker.bytes().take_while(|byte| *byte == b'`').count();
    if fence_length < 3 {
        return None;
    }
    let through_closing = after_opening.trim_end_matches(['\r', '\n']);
    let closing_start = through_closing.rfind('\n').map_or(0, |newline| newline + 1);
    let closing_marker = through_closing[closing_start..].trim();
    if closing_marker.len() < fence_length || !closing_marker.bytes().all(|byte| byte == b'`') {
        return None;
    }
    let content = &after_opening[..closing_start];
    let content = content
        .strip_suffix(CRLF_SEQUENCE)
        .or_else(|| content.strip_suffix(LF_SEQUENCE))
        .unwrap_or(content);
    let after_closing = &after_opening[through_closing.len()..];
    Some(format!("{content}{after_closing}"))
}

fn apply_line_transaction(
    source: &str,
    source_generation: u32,
    range: std::ops::Range<usize>,
    replacement: String,
) -> SemanticEditPlan {
    if source[range.clone()] == replacement {
        return SemanticEditPlan::NoChange;
    }
    let cursor_after = range.start + replacement.len();
    apply_transaction(source_generation, range, replacement, cursor_after)
}

fn apply_transaction(
    source_generation: u32,
    range: std::ops::Range<usize>,
    replacement: String,
    cursor_after: usize,
) -> SemanticEditPlan {
    SemanticEditPlan::Apply(ui::plugin::EditTransaction::replace(
        source_generation,
        range,
        replacement,
        cursor_after,
    ))
}

fn line_selection_range(
    source: &str,
    cursor_byte: usize,
    selection: Option<&std::ops::Range<usize>>,
) -> std::ops::Range<usize> {
    let start_byte = selection.map_or(cursor_byte, |range| range.start);
    let end_byte = selection.map_or(cursor_byte, |range| range.end);
    let start = line_start(source, start_byte);
    let end_probe = if end_byte > start_byte && source.as_bytes().get(end_byte - 1) == Some(&b'\n')
    {
        end_byte - 1
    } else {
        end_byte
    };
    start..line_end(source, end_probe)
}

fn line_start(source: &str, byte: usize) -> usize {
    source[..byte].rfind('\n').map_or(0, |newline| newline + 1)
}

fn line_end(source: &str, byte: usize) -> usize {
    let Some(newline) = source[byte..].find('\n') else {
        return source.len();
    };
    let newline_byte = byte + newline;
    if source[..newline_byte].ends_with('\r') { newline_byte - 1 } else { newline_byte }
}

fn first_h1_range(source: &str) -> Option<std::ops::Range<usize>> {
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let indentation = leading_whitespace(content);
        if content[indentation..].starts_with("# ") {
            return Some(offset..offset + content.len());
        }
        offset += line.len();
    }
    None
}

fn ranges_overlap(left: &std::ops::Range<usize>, right: &std::ops::Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn leading_whitespace(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn transform_lines(segment: &str, transform: impl Fn(&str) -> String) -> String {
    if segment.is_empty() {
        return transform(segment);
    }
    let mut result = String::with_capacity(segment.len());
    for line in segment.split_inclusive('\n') {
        let (content, newline) = if let Some(content) = line.strip_suffix(CRLF_SEQUENCE) {
            (content, CRLF_SEQUENCE)
        } else {
            line.strip_suffix(LF_SEQUENCE).map_or((line, ""), |content| (content, LF_SEQUENCE))
        };
        result.push_str(&transform(content));
        result.push_str(newline);
    }
    if segment.ends_with(LF_SEQUENCE) {
        result.push_str(&transform(""));
    }
    result
}

fn has_line_prefix(line: &str, prefix: LinePrefix) -> bool {
    let content = &line[leading_whitespace(line)..];
    match prefix {
        LinePrefix::Unordered => content.starts_with("- ") || content.starts_with("* "),
        LinePrefix::Ordered => content.split_once(". ").is_some_and(|(number, _)| {
            !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
        }),
        LinePrefix::Task => {
            content.starts_with("- [ ] ")
                || content.starts_with("- [x] ")
                || content.starts_with("- [X] ")
        }
        LinePrefix::Quote => content.starts_with("> ") || content == ">",
    }
}

fn remove_line_prefix(line: &str, prefix: LinePrefix) -> String {
    let indentation = leading_whitespace(line);
    let content = &line[indentation..];
    let marker_length = match prefix {
        LinePrefix::Unordered if content.starts_with("- ") || content.starts_with("* ") => 2,
        LinePrefix::Ordered => content
            .find(". ")
            .filter(|index| {
                *index > 0 && content[..*index].chars().all(|character| character.is_ascii_digit())
            })
            .map_or(0, |index| index + 2),
        LinePrefix::Task
            if content.starts_with("- [ ] ")
                || content.starts_with("- [x] ")
                || content.starts_with("- [X] ") =>
        {
            6
        }
        LinePrefix::Quote if content.starts_with("> ") => 2,
        LinePrefix::Quote if content == ">" => 1,
        _ => 0,
    };
    format!("{}{}", &line[..indentation], &content[marker_length..])
}

fn add_line_prefix(line: &str, prefix: LinePrefix) -> String {
    let indentation = leading_whitespace(line);
    let marker = match prefix {
        LinePrefix::Unordered => "- ",
        LinePrefix::Ordered => "1. ",
        LinePrefix::Task => "- [ ] ",
        LinePrefix::Quote => "> ",
    };
    format!("{}{}{}", &line[..indentation], marker, &line[indentation..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_structure_row_insertion_preserves_blockquote_prefix_and_adjacent_blocks() {
        let source = "before\n\n> | name | value |\n> | --- | --- |\n> | a | b |\n\n# after";
        let cursor = source.find("a | b").expect("blockquote table body exists");
        let (result, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowAfter),
            cursor,
            None,
        );
        assert!(result.contains("> | a | b |\n> |  |  |"), "result: {result:?}");
        assert!(result.starts_with("before\n\n"));
        assert!(result.ends_with("\n\n# after"));
    }

    #[test]
    fn deleting_last_container_table_row_clears_cells_without_losing_prefix() {
        let source = "> | heading | value |\n> | --- | --- |\n> | a | b |";
        let cursor = source.find("a | b").expect("container table body exists");
        let (result, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
            cursor,
            None,
        );
        assert!(result.ends_with("> |  |  |"), "result: {result:?}");
        assert_eq!(parsed_table_column_count(&result), Some(2));
    }

    #[test]
    fn container_row_insertion_preserves_crlf_and_adjacent_heading() {
        let source =
            "before\r\n\r\n> | heading | value |\r\n> | --- | --- |\r\n> | a | b |\r\n\r\n# after";
        let cursor = source.find("a | b").expect("container table body exists");
        assert!(source_table_at(source, cursor).is_some(), "container table should be modeled");
        let (result, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowAfter),
            cursor,
            None,
        );
        assert!(result.contains("> | a | b |\r\n> |  |  |"), "result: {result:?}");
        assert!(!result.replace("\r\n", "").contains('\r'));
        assert!(result.ends_with("\r\n\r\n# after"));
    }

    #[test]
    fn table_structure_column_edit_preserves_list_continuation_prefix() {
        let source = "- item\n\n  | name | value |\n  | --- | --- |\n  | a | b |";
        let cursor = source.find("a | b").expect("list table body exists");
        let (result, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::InsertColumnAfter,
            ),
            cursor,
            None,
        );
        assert!(result.contains("  | a |  | b |"), "result: {result:?}");
        assert!(result.contains("- item\n\n  | name |  | value |"), "result: {result:?}");
        assert_eq!(parsed_table_column_count(&result), Some(3));
    }

    #[test]
    fn table_structure_conservatively_rejects_non_table_pipe_text() {
        let source = "A | B\n---x---\nx | y";
        assert_eq!(parsed_table_column_count(source), None);
        let cursor = source.find("x | y").expect("table body exists");
        assert_eq!(
            table_structure_capabilities(source, cursor),
            None,
            "pipe-like text rejected by the parser must remain unavailable"
        );
        assert_eq!(
            plan_semantic_edit(
                source,
                3,
                cursor,
                None,
                SemanticEditCommand::TableStructure(
                    ui::plugin::TableStructureCommand::InsertColumnAfter
                )
            ),
            SemanticEditPlan::Unsupported
        );
    }

    #[test]
    fn table_structure_alignment_edits_a_parser_recognized_table_without_outer_pipes() {
        let source = "A | B\n---|---\nx | y";
        assert_eq!(parsed_table_column_count(source), Some(2));
        let cursor = source.find("y").expect("second body cell exists");
        let capability = table_structure_capabilities(source, cursor);
        assert!(capability.is_some(), "the parsed GFM table should expose structure operations");
        let (result, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::SetColumnAlignment(
                    ui::plugin::TableColumnAlignment::Right,
                ),
            ),
            cursor,
            None,
        );
        assert_eq!(result, "A | B\n---|---:\nx | y");
    }

    #[test]
    fn table_structure_cell_scanning_treats_unclosed_backticks_as_plain_text() {
        let source = "A | B\n---|---\nleft` | right";
        assert_eq!(parsed_table_column_count(source), Some(2));
        let cursor = source.find("right").expect("second body cell exists");
        assert!(table_structure_capabilities(source, cursor).is_some());
        let (result, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::SetColumnAlignment(
                    ui::plugin::TableColumnAlignment::Right,
                ),
            ),
            cursor,
            None,
        );
        assert_eq!(result, "A | B\n---|---:\nleft` | right");
    }

    #[test]
    fn table_structure_row_and_column_edits_preserve_omitted_outer_pipe_style() {
        let source = "A | B\n---|---\nx | y";
        let cursor = source.find("y").expect("second body cell exists");
        let (inserted_row, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowAfter),
            cursor,
            None,
        );
        assert!(inserted_row.ends_with(" | "), "row insertion style: {inserted_row:?}");
        assert_eq!(parsed_table_column_count(&inserted_row), Some(2));

        let (inserted_column, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::InsertColumnAfter,
            ),
            cursor,
            None,
        );
        assert_eq!(
            parsed_table_column_count(&inserted_column),
            Some(3),
            "inserted output: {inserted_column:?}"
        );
        assert!(
            inserted_column.lines().all(|line| !line.trim_start().starts_with('|')),
            "column edits must keep leading pipes omitted: {inserted_column:?}"
        );
        let inserted_rows = inserted_column.lines().collect::<Vec<_>>();
        assert!(inserted_rows[0].trim_end().ends_with('|'));
        assert!(inserted_rows[2].trim_end().ends_with('|'));

        let inserted_body_cursor = inserted_column.find("y").expect("body cell remains");
        let (deleted_column, _) = applied_text(
            &inserted_column,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteColumn),
            inserted_body_cursor,
            None,
        );
        assert_eq!(parsed_table_column_count(&deleted_column), Some(2));
        assert!(deleted_column.lines().all(|line| !line.trim_start().starts_with('|')));
    }

    #[test]
    fn large_table_context_lookup_and_column_edit_complete_within_budget() {
        use std::time::{Duration, Instant};

        let column_count = 24;
        let body_row_count = 500;
        let make_row = |cells: Vec<String>| format_table_row(&cells);
        let header = make_row((0..column_count).map(|column| format!("h{column}")).collect());
        let separator = make_row(vec!["---".to_owned(); column_count]);
        let body = (0..body_row_count)
            .map(|row| {
                make_row((0..column_count).map(|column| format!("r{row}c{column}")).collect())
            })
            .collect::<Vec<_>>();
        let source = std::iter::once(header)
            .chain(std::iter::once(separator))
            .chain(body)
            .collect::<Vec<_>>()
            .join("\n");
        let cursor = source.find("r250c12").expect("middle table cell exists");
        let started = Instant::now();
        let capabilities = table_structure_capabilities(&source, cursor);
        let plan = plan_semantic_edit(
            &source,
            5,
            cursor,
            None,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::InsertColumnAfter,
            ),
        );
        assert!(capabilities.is_some(), "the large table should be recognized");
        assert!(matches!(plan, SemanticEditPlan::Apply(_)), "column edit should be planned");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "table context lookup and structure planning exceeded two seconds: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn menu_table_creation_keeps_blockquote_context() {
        let source = "> paragraph";
        let cursor = source.len();
        let (result, cursor_after) = applied_text(
            source,
            SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
            cursor,
            None,
        );
        assert_eq!(parsed_table_count(&result, 2), 1, "result: {result:?}");
        assert!(result.starts_with("> paragraph\n> \n> | | |"), "result: {result:?}");
        assert_eq!(
            cursor_after,
            ui::plugin::EditSelection::Caret(
                result.find("| | |").expect("table header exists") + 2
            )
        );
    }

    #[test]
    fn menu_table_creation_keeps_list_item_context() {
        let source = "- paragraph";
        let cursor = source.len();
        let expected = "- paragraph\n\n  | | |\n  | --- | --- |\n  | | |";
        assert_eq!(parsed_table_count(expected, 2), 1, "expected candidate must parse");
        assert!(container_paragraph_context(source, cursor).is_some());
        let (result, _) = applied_text(
            source,
            SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
            cursor,
            None,
        );
        assert_eq!(parsed_table_count(&result, 2), 1, "result: {result:?}");
        assert!(result.starts_with("- paragraph\n\n  | | |"), "result: {result:?}");
    }

    #[test]
    fn menu_table_creation_in_blockquote_splits_at_start_middle_and_end() {
        let source = "> abcd";
        for (cursor, before, after) in [(2, "> ", "> abcd"), (4, "> ab", "> cd"), (6, "> abcd", "")]
        {
            let (result, cursor_after) = applied_text(
                source,
                SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
                cursor,
                None,
            );
            assert_eq!(parsed_table_count(&result, 2), 1, "cursor={cursor}, result={result:?}");
            assert!(result.starts_with(before), "cursor={cursor}, result: {result:?}");
            if !after.is_empty() {
                assert!(result.ends_with(after), "cursor={cursor}, result: {result:?}");
            }
            assert!(
                result.lines().filter(|line| line.contains('|')).all(|line| line.starts_with("> "))
            );
            assert_eq!(
                cursor_after,
                ui::plugin::EditSelection::Caret(
                    result.find("| | |").expect("table header exists") + 2
                )
            );
        }
    }

    #[test]
    fn menu_table_creation_in_list_splits_at_start_middle_and_end() {
        let source = "- abcd";
        for (cursor, before, after) in [(2, "- ", "  abcd"), (4, "- ab", "  cd"), (6, "- abcd", "")]
        {
            let (result, cursor_after) = applied_text(
                source,
                SemanticEditCommand::InsertTable { columns: 2, rows: 2 },
                cursor,
                None,
            );
            assert_eq!(parsed_table_count(&result, 2), 1, "cursor={cursor}, result={result:?}");
            assert!(
                result.contains("  | | |"),
                "table rows need list continuation indentation: {result:?}"
            );
            assert!(result.starts_with(before), "cursor={cursor}, result: {result:?}");
            if !after.is_empty() {
                assert!(result.ends_with(after), "cursor={cursor}, result: {result:?}");
            }
            assert_eq!(
                cursor_after,
                ui::plugin::EditSelection::Caret(
                    result.find("| | |").expect("table header exists") + 2
                )
            );
        }
    }

    #[test]
    fn table_structure_capabilities_disable_header_and_only_column_deletion() {
        let source = "| only |\n| --- |\n| value |";
        let header =
            table_structure_capabilities(source, source.find("only").expect("header exists"))
                .expect("header lies in table");
        assert!(!header.can_delete_row);
        assert!(!header.can_delete_column);

        let body = table_structure_capabilities(source, source.find("value").expect("body exists"))
            .expect("body lies in table");
        assert!(body.can_delete_row);
        assert!(!body.can_delete_column);
    }

    #[test]
    fn table_structure_commands_preserve_escaped_and_code_pipes() {
        let source = "| 名称 | 说明 |\n| --- | --- |\n| a\\|b | `x|y` |";
        let cursor = source.find("a\\|b").expect("table body cell exists");
        let (inserted, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::InsertColumnAfter,
            ),
            cursor,
            None,
        );
        assert_eq!(parsed_table_column_count(&inserted), Some(3));
        assert!(inserted.contains("a\\|b"));
        assert!(inserted.contains("`x|y`"));
    }

    #[test]
    fn row_and_column_insertions_and_deletions_keep_a_parseable_table() {
        let source = "| a | b |\r\n| --- | --- |\r\n| c | d |\r\n| e | f |";
        let cursor = source.find("c").expect("first body cell exists");
        let (row_inserted, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::InsertRowBefore),
            cursor,
            None,
        );
        assert_eq!(parsed_table_column_count(&row_inserted), Some(2));
        assert!(row_inserted.contains("|  |\r\n| c | d |"));

        let (row_deleted, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
            cursor,
            None,
        );
        assert_eq!(row_deleted.matches("| --- | --- |").count(), 1);
        assert!(!row_deleted.contains("| c | d |"));
        assert!(row_deleted.contains("| e | f |"));

        let (column_deleted, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteColumn),
            cursor,
            None,
        );
        assert_eq!(parsed_table_column_count(&column_deleted), Some(1));
        assert!(
            column_deleted.contains("| b |\r\n| --- |\r\n| d |\r\n| f |"),
            "result: {column_deleted:?}"
        );
    }

    #[test]
    fn table_structure_commands_protect_header_and_only_column() {
        let source = "| only |\n| --- |\n| value |";
        let cursor = source.find("value").expect("table body exists");
        let delete_column = plan_semantic_edit(
            source,
            7,
            cursor,
            None,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteColumn),
        );
        assert_eq!(delete_column, SemanticEditPlan::Unsupported);

        let header_cursor = source.find("only").expect("header exists");
        let delete_header = plan_semantic_edit(
            source,
            7,
            header_cursor,
            None,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
        );
        assert_eq!(delete_header, SemanticEditPlan::Unsupported);
    }

    #[test]
    fn alignment_updates_only_its_separator_cell_and_accepts_short_body_rows() {
        let source = "| a | b |\n| --- | --- |\n| short |\n\nafter";
        let cursor = source.find("short").expect("body cell exists");
        let plan = plan_semantic_edit(
            source,
            7,
            cursor,
            None,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::SetColumnAlignment(
                    ui::plugin::TableColumnAlignment::Right,
                ),
            ),
        );
        let SemanticEditPlan::Apply(transaction) = plan else {
            panic!("column alignment should produce a transaction");
        };
        assert_eq!(transaction.replacements.len(), 1);
        assert_eq!(&source[transaction.replacements[0].range.clone()], "---");
        assert_eq!(transaction.replacements[0].text, "---:");

        let (expanded, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(
                ui::plugin::TableStructureCommand::InsertColumnAfter,
            ),
            cursor,
            None,
        );
        assert_eq!(parsed_table_column_count(&expanded), Some(3));
        assert!(expanded.ends_with("\n\nafter"));
    }

    #[test]
    fn deleting_the_only_body_row_clears_it_and_table_delete_is_atomic() {
        let source = "before\n\n| h |\n| --- |\n| value |\n\nafter";
        let cursor = source.find("value").expect("table body exists");
        let (cleared, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteRow),
            cursor,
            None,
        );
        assert!(cleared.contains("| h |\n| --- |\n|  |"));
        assert!(cleared.starts_with("before\n\n") && cleared.ends_with("\n\nafter"));

        let (deleted, _) = applied_text(
            source,
            SemanticEditCommand::TableStructure(ui::plugin::TableStructureCommand::DeleteTable),
            cursor,
            None,
        );
        assert_eq!(deleted, "before\n\nafter");
    }

    fn parsed_table_column_count(source: &str) -> Option<usize> {
        crate::parser::parse_markdown(source).events.into_iter().find_map(|event| match event {
            crate::parser::MarkdownEvent::Start(crate::parser::MarkdownTag::Table(alignments)) => {
                Some(alignments.len())
            }
            _ => None,
        })
    }

    fn applied_text(
        source: &str,
        command: SemanticEditCommand,
        cursor_byte: usize,
        selection: Option<std::ops::Range<usize>>,
    ) -> (String, ui::plugin::EditSelection) {
        let plan = plan_semantic_edit(source, 7, cursor_byte, selection, command);
        let SemanticEditPlan::Apply(transaction) = plan else {
            panic!("semantic command should produce an edit transaction");
        };
        let replacement =
            transaction.replacements.first().expect("semantic command should have one replacement");
        let mut result = source.to_owned();
        result.replace_range(replacement.range.clone(), &replacement.text);
        (result, transaction.selection_after)
    }

    #[test]
    fn toggle_bold_without_selection_inserts_a_pair_and_places_the_caret_inside() {
        let (result, selection) =
            applied_text("标题", SemanticEditCommand::ToggleBold, "标题".len(), None);

        assert_eq!(result, "标题****");
        assert_eq!(selection, ui::plugin::EditSelection::Caret("标题".len() + 2));
    }

    #[test]
    fn toggle_bold_wraps_a_single_line_selection() {
        let (result, _) = applied_text("中文文本", SemanticEditCommand::ToggleBold, 6, Some(0..6));

        assert_eq!(result, "**中文**文本");
    }

    #[test]
    fn unordered_list_formats_every_line_in_a_multiline_selection() {
        let (result, _) = applied_text(
            "第一行\n第二行",
            SemanticEditCommand::UnorderedList,
            0,
            Some(0.."第一行\n第二行".len()),
        );

        assert_eq!(result, "- 第一行\n- 第二行");
    }

    #[test]
    fn toggling_bold_again_removes_existing_nested_markers() {
        let (result, _) = applied_text(
            "**重点**",
            SemanticEditCommand::ToggleBold,
            "**重点**".len(),
            Some(0.."**重点**".len()),
        );

        assert_eq!(result, "重点");
    }

    #[test]
    fn heading_command_does_not_demote_the_notora_title_h1() {
        let plan = plan_semantic_edit(
            "# 标题\n\n正文",
            7,
            2,
            Some(0..8),
            SemanticEditCommand::SetHeadingLevel(2),
        );

        assert_eq!(plan, SemanticEditPlan::NoChange);
    }

    #[test]
    fn inline_toggle_wraps_a_selection_that_contains_only_one_marker() {
        for (command, marker) in [
            (SemanticEditCommand::ToggleBold, "**"),
            (SemanticEditCommand::ToggleItalic, "*"),
            (SemanticEditCommand::ToggleStrikethrough, "~~"),
        ] {
            let (result, _) = applied_text(marker, command, marker.len(), Some(0..marker.len()));

            assert_eq!(result, marker.repeat(3));
        }
    }

    #[test]
    fn inline_code_command_preserves_embedded_backticks_and_boundary_spaces() {
        for selected in ["a`b", "`edge", "edge`", " leading", "trailing ", " both "] {
            let (formatted, _) = applied_text(
                selected,
                SemanticEditCommand::ToggleInlineCode,
                selected.len(),
                Some(0..selected.len()),
            );
            let parsed_code: Vec<String> = crate::parser::parse_markdown(&formatted)
                .events
                .into_iter()
                .filter_map(|event| match event {
                    crate::parser::MarkdownEvent::Code(code) => Some(code),
                    _ => None,
                })
                .collect();

            assert_eq!(parsed_code, [selected], "generated Markdown: {formatted:?}");

            let formatted_length = formatted.len();
            let (restored, _) = applied_text(
                &formatted,
                SemanticEditCommand::ToggleInlineCode,
                formatted_length,
                Some(0..formatted_length),
            );
            assert_eq!(restored, selected, "failed to toggle {formatted:?}");
        }
    }

    #[test]
    fn inline_code_command_uses_safe_delimiters_for_a_single_backtick() {
        let (formatted, _) = applied_text(
            CODE_MARKER,
            SemanticEditCommand::ToggleInlineCode,
            CODE_MARKER.len(),
            Some(0..CODE_MARKER.len()),
        );

        assert_eq!(formatted, "`` ` ``");
    }

    #[test]
    fn code_block_command_uses_a_fence_longer_than_content_runs() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("before{newline}```{newline}after");
            let (formatted, _) = applied_text(
                &source,
                SemanticEditCommand::CodeBlock,
                source.len(),
                Some(0..source.len()),
            );

            assert!(formatted.starts_with(&format!("````{newline}")));
            assert!(formatted.ends_with(&format!("{newline}````")));
            let parsed = crate::parser::parse_markdown(&formatted);
            assert_eq!(
                parsed
                    .events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        crate::parser::MarkdownEvent::Start(
                            crate::parser::MarkdownTag::CodeBlock { .. }
                        )
                    ))
                    .count(),
                1,
                "generated Markdown: {formatted:?}"
            );

            let formatted_length = formatted.len();
            let (restored, _) = applied_text(
                &formatted,
                SemanticEditCommand::CodeBlock,
                formatted_length,
                Some(0..formatted_length),
            );
            assert_eq!(restored, source);
        }
    }

    #[test]
    fn code_block_toggle_accepts_a_longer_closing_fence() {
        for source in ["```\na\n````", "```rust\r\na\r\n````"] {
            let (restored, _) =
                applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));
            assert_eq!(restored, "a");
        }
    }

    #[test]
    fn code_block_toggle_removes_an_empty_fenced_block() {
        let source = "```\n```";
        let (result, _) =
            applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));

        assert_eq!(result, "");
    }

    #[test]
    fn line_commands_format_an_empty_document() {
        for (command, expected) in [
            (SemanticEditCommand::SetHeadingLevel(2), "## "),
            (SemanticEditCommand::UnorderedList, "- "),
            (SemanticEditCommand::OrderedList, "1. "),
            (SemanticEditCommand::TaskList, "- [ ] "),
            (SemanticEditCommand::Quote, "> "),
            (SemanticEditCommand::CodeBlock, "```\n\n```"),
        ] {
            let (result, _) = applied_text("", command, 0, None);

            assert_eq!(result, expected);
        }
    }

    #[test]
    fn line_commands_format_a_blank_line_between_paragraphs() {
        for (command, expected) in [
            (SemanticEditCommand::SetHeadingLevel(2), "before\n## \nafter"),
            (SemanticEditCommand::UnorderedList, "before\n- \nafter"),
            (SemanticEditCommand::OrderedList, "before\n1. \nafter"),
            (SemanticEditCommand::TaskList, "before\n- [ ] \nafter"),
            (SemanticEditCommand::Quote, "before\n> \nafter"),
            (SemanticEditCommand::CodeBlock, "before\n```\n\n```\nafter"),
        ] {
            let (result, _) = applied_text("before\n\nafter", command, "before\n".len(), None);

            assert_eq!(result, expected);
        }
    }

    #[test]
    fn heading_command_preserves_hashes_that_are_part_of_plain_text() {
        for source in ["#hashtag", "##正文", "####### seven hashes"] {
            let (result, _) =
                applied_text(source, SemanticEditCommand::SetHeadingLevel(2), 0, None);

            assert_eq!(result, format!("## {source}"));
        }
    }

    #[test]
    fn line_command_keeps_the_crlf_sequence_outside_its_replacement_range() {
        let plan = plan_semantic_edit("first\r\nsecond", 7, 0, None, SemanticEditCommand::Quote);
        let SemanticEditPlan::Apply(transaction) = plan else {
            panic!("quote command should produce an edit transaction");
        };

        assert_eq!(transaction.replacements[0].range, 0.."first".len());
        assert_eq!(transaction.replacements[0].text, "> first");
    }

    #[test]
    fn code_block_toggle_preserves_crlf_when_adding_fences() {
        let source = "first\r\nsecond";
        let (result, _) =
            applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..source.len()));

        assert_eq!(result, "```\r\nfirst\r\nsecond\r\n```");
    }

    #[test]
    fn code_block_toggle_preserves_crlf_when_removing_fences() {
        let source = "```rust\r\nfirst\r\nsecond\r\n```\r\nafter";
        let closing_end =
            source.find("\r\nafter").expect("fixture has a paragraph after the fence");
        let (result, _) =
            applied_text(source, SemanticEditCommand::CodeBlock, 0, Some(0..closing_end));

        assert_eq!(result, "first\r\nsecond\r\nafter");
    }

    #[test]
    fn code_block_toggle_preserves_a_selected_blank_line_after_the_closing_fence() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("```{newline}a{newline}```{newline}{newline}last");
            let selection_end = source.find("last").expect("fixture ends with a paragraph");
            let (result, _) =
                applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

            assert_eq!(result, format!("a{newline}{newline}last"));
        }
    }

    #[test]
    fn empty_code_block_toggle_preserves_a_selected_blank_line_after_the_closing_fence() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("```{newline}```{newline}{newline}last");
            let selection_end = source.find("last").expect("fixture ends with a paragraph");
            let (result, _) =
                applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

            assert_eq!(result, format!("{newline}{newline}last"));
        }
    }

    #[test]
    fn code_block_toggle_preserves_multiple_selected_blank_lines_after_the_closing_fence() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("```{newline}a{newline}```{newline}{newline}{newline}last");
            let selection_end = source.find("last").expect("fixture ends with a paragraph");
            let (result, _) =
                applied_text(&source, SemanticEditCommand::CodeBlock, 0, Some(0..selection_end));

            assert_eq!(result, format!("a{newline}{newline}{newline}last"));
        }
    }

    #[test]
    fn multiline_heading_selection_formats_its_last_blank_line() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("first{newline}{newline}last");
            let selection_end = format!("first{newline}{newline}").len();
            let (result, _) = applied_text(
                &source,
                SemanticEditCommand::SetHeadingLevel(2),
                0,
                Some(0..selection_end),
            );

            assert_eq!(result, format!("## first{newline}## {newline}last"));
        }
    }

    #[test]
    fn multiline_quote_selection_counts_its_last_blank_line_when_toggling() {
        for newline in [LF_SEQUENCE, CRLF_SEQUENCE] {
            let source = format!("> first{newline}{newline}last");
            let selection_end = format!("> first{newline}{newline}").len();
            let (result, _) =
                applied_text(&source, SemanticEditCommand::Quote, 0, Some(0..selection_end));

            assert_eq!(result, format!("> > first{newline}> {newline}last"));
        }
    }
}
