use ui::plugin::{SemanticEditCommand, SemanticEditPlan};

mod inline_formatting;
mod table_structure;
use inline_formatting::InlineFormat;
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
        SemanticEditCommand::ToggleBold => inline_formatting::plan_toggle(
            source,
            source_generation,
            cursor_byte,
            selection,
            InlineFormat::Bold,
        ),
        SemanticEditCommand::ToggleItalic => inline_formatting::plan_toggle(
            source,
            source_generation,
            cursor_byte,
            selection,
            InlineFormat::Italic,
        ),
        SemanticEditCommand::ToggleStrikethrough => inline_formatting::plan_toggle(
            source,
            source_generation,
            cursor_byte,
            selection,
            InlineFormat::Strikethrough,
        ),
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
    let targets = inline_formatting::selected_blocks(source, &range);
    let remove_existing =
        targets.iter().all(|target| unwrap_inline_code(&source[target.clone()]).is_some());
    inline_formatting::plan_block_transform(source, source_generation, &targets, |selected| {
        match unwrap_inline_code(selected) {
            Some(unwrapped) if remove_existing => unwrapped,
            Some(_) => selected.to_owned(),
            None => wrap_inline_code(selected),
        }
    })
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
    let targets = inline_formatting::selected_blocks(source, &range);
    inline_formatting::plan_block_transform(source, source_generation, &targets, |selected| {
        format!("[{selected}](https://)")
    })
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
        } else if has_line_prefix(line, prefix) {
            line.to_owned()
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
#[path = "commands_tests.rs"]
mod tests;
