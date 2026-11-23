use crate::source_snapshot::MarkdownSource;
use ui::plugin::{SemanticEditCommand, SemanticEditPlan};

mod inline_formatting;
mod table_structure;
use inline_formatting::InlineFormat;
pub(crate) use table_structure::snapshot_table_capabilities;
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
    plan_semantic_source(
        &MarkdownSource::borrowed(source),
        source_generation,
        cursor_byte,
        selection,
        command,
    )
}

pub(crate) fn plan_semantic_source(
    source: &MarkdownSource<'_>,
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
    source: &MarkdownSource<'_>,
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
        if parsed_table_count(&MarkdownSource::owned(result), columns)
            != parsed_table_count(source, columns) + 1
        {
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
        if parsed_table_count(&MarkdownSource::owned(result), columns)
            != parsed_table_count(source, columns) + 1
        {
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
    if parsed_table_count(&MarkdownSource::owned(result), columns)
        != parsed_table_count(source, columns) + 1
    {
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

fn parsed_table_count(source: &MarkdownSource<'_>, columns: usize) -> usize {
    use crate::parser::{MarkdownEvent, MarkdownTag};
    source.parsed()
        .events
        .iter()
        .filter(|event| matches!(event, MarkdownEvent::Start(MarkdownTag::Table(alignments)) if alignments.len() == columns))
        .count()
}

fn top_level_paragraph_range(
    source: &MarkdownSource<'_>,
    cursor_byte: usize,
) -> Option<std::ops::Range<usize>> {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
    let parsed = source.parsed();
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
    source: &MarkdownSource<'_>,
    cursor_byte: usize,
) -> Option<ContainerParagraphContext> {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
    let parsed = source.parsed();
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

fn empty_line_range(
    source: &MarkdownSource<'_>,
    cursor_byte: usize,
) -> Option<std::ops::Range<usize>> {
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

fn cursor_is_inside_container(source: &MarkdownSource<'_>, cursor_byte: usize) -> bool {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ContainerKind {
        Quote,
        List,
        Item,
    }

    let parsed = source.parsed();
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
    source: &MarkdownSource<'_>,
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
    source: &MarkdownSource<'_>,
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
    source: &MarkdownSource<'_>,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<std::ops::Range<usize>>,
    level: u8,
) -> SemanticEditPlan {
    if !(1..=6).contains(&level) {
        return SemanticEditPlan::Unsupported;
    }
    let mut target = line_selection_range(source, cursor_byte, selection.as_ref());
    let setext_ranges = setext_heading_ranges(source);
    for setext in &setext_ranges {
        if ranges_overlap(&target, setext) {
            target.start = target.start.min(setext.start);
            target.end = target.end.max(setext.end);
        }
    }
    if let Some(title_range) = first_h1_range(source)
        && ranges_overlap(&target, &title_range)
    {
        return SemanticEditPlan::NoChange;
    }
    let replacement = transform_heading_target(source, &target, &setext_ranges, level);
    apply_line_transaction(source, source_generation, target, replacement)
}

fn setext_heading_ranges(source: &MarkdownSource<'_>) -> Vec<std::ops::Range<usize>> {
    use crate::parser::{MarkdownEvent, MarkdownTag};

    source
        .parsed()
        .source_events()
        .filter_map(|(event, range)| {
            let MarkdownEvent::Start(MarkdownTag::Heading { .. }) = event else {
                return None;
            };
            if crate::augmenter::heading_source_is_atx(source, range.start) {
                return None;
            }
            let start = line_start(source, range.start);
            let mut end = range.end.min(source.len());
            while end > start && matches!(source.as_bytes()[end - 1], b'\n' | b'\r') {
                end -= 1;
            }
            Some(start..end)
        })
        .collect()
}

fn transform_heading_target(
    source: &str,
    target: &std::ops::Range<usize>,
    setext_ranges: &[std::ops::Range<usize>],
    level: u8,
) -> String {
    if target.is_empty() {
        return format_single_heading_line("", level);
    }
    let mut result = String::with_capacity(target.len());
    let mut offset = target.start;
    let mut trailing_newline = false;
    while offset < target.end {
        if let Some(setext) = setext_ranges.iter().find(|range| range.start == offset) {
            result.push_str(&format_setext_heading(&source[setext.clone()], level));
            offset = setext.end;
        } else {
            let next_nl =
                source[offset..target.end].find('\n').map_or(target.end, |delta| offset + delta);
            let content_end = if next_nl > offset && source.as_bytes()[next_nl - 1] == b'\r' {
                next_nl - 1
            } else {
                next_nl
            };
            result.push_str(&format_single_heading_line(&source[offset..content_end], level));
            offset = content_end;
        }
        trailing_newline = false;
        if let Some(newline) = next_eol_slice(&source[offset..target.end]) {
            result.push_str(newline);
            offset += newline.len();
            trailing_newline = true;
        }
    }
    if trailing_newline {
        result.push_str(&format_single_heading_line("", level));
    }
    result
}

fn next_eol_slice(rest: &str) -> Option<&str> {
    if rest.starts_with(CRLF_SEQUENCE) {
        Some(CRLF_SEQUENCE)
    } else if rest.starts_with(LF_SEQUENCE) {
        Some(LF_SEQUENCE)
    } else {
        None
    }
}

fn format_setext_heading(block: &str, level: u8) -> String {
    let lines: Vec<&str> =
        block.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
    let content_count = lines.len().saturating_sub(1);
    let first_line = lines.first().copied().unwrap_or("");
    let indentation = leading_whitespace(first_line);
    let title_parts: Vec<&str> = lines[..content_count]
        .iter()
        .map(|line| line.trim())
        .filter(|part| !part.is_empty())
        .collect();
    format!(
        "{}{} {}",
        &first_line[..indentation],
        "#".repeat(level as usize),
        title_parts.join(" ")
    )
}

fn format_single_heading_line(line: &str, level: u8) -> String {
    let indentation = leading_whitespace(line);
    let content = &line[indentation..];
    let title = if crate::augmenter::heading_source_is_atx(line, 0) {
        strip_atx_closing_hashes(content.trim_start_matches('#').trim_start_matches([' ', '\t']))
    } else {
        content
    };
    format!("{}{} {}", &line[..indentation], "#".repeat(level as usize), title)
}

fn strip_atx_closing_hashes(content: &str) -> &str {
    let trimmed = content.trim_end_matches([' ', '\t']);
    let before_hashes = trimmed.trim_end_matches('#');
    if before_hashes.len() == trimmed.len() {
        return content;
    }
    if before_hashes.is_empty() {
        return "";
    }
    if before_hashes.ends_with([' ', '\t']) {
        return before_hashes.trim_end_matches([' ', '\t']);
    }
    content
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

fn first_h1_range(source: &MarkdownSource<'_>) -> Option<std::ops::Range<usize>> {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};

    let mut container_depth = 0usize;
    for (event, range) in source.parsed().source_events() {
        match event {
            MarkdownEvent::Start(
                MarkdownTag::BlockQuote
                | MarkdownTag::List(..)
                | MarkdownTag::Item
                | MarkdownTag::Table(_),
            ) => {
                container_depth += 1;
            }
            MarkdownEvent::End(
                MarkdownTagEnd::BlockQuote
                | MarkdownTagEnd::List
                | MarkdownTagEnd::Item
                | MarkdownTagEnd::Table,
            ) => {
                container_depth = container_depth.saturating_sub(1);
            }
            MarkdownEvent::Start(MarkdownTag::Heading { level: 1 })
                if container_depth == 0
                    && crate::augmenter::heading_source_is_atx(source, range.start) =>
            {
                let start = line_start(source, range.start);
                return Some(start..line_end(source, range.start));
            }
            _ => {}
        }
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

fn task_prefix_len(content: &str) -> usize {
    let Some(first) = content.as_bytes().first() else {
        return 0;
    };
    if !matches!(first, b'-' | b'*' | b'+') || content.as_bytes().get(1) != Some(&b' ') {
        return 0;
    }
    let rest = &content[2..];
    if rest.starts_with("[ ] ") || rest.starts_with("[x] ") || rest.starts_with("[X] ") {
        6
    } else {
        0
    }
}

fn unordered_prefix_len(content: &str) -> usize {
    if task_prefix_len(content) > 0 {
        return 0;
    }
    let Some(first) = content.as_bytes().first() else {
        return 0;
    };
    if matches!(first, b'-' | b'*' | b'+') && content.as_bytes().get(1) == Some(&b' ') {
        2
    } else {
        0
    }
}

fn ordered_prefix_len(content: &str) -> usize {
    let digits = content.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return 0;
    }
    let bytes = content.as_bytes();
    if matches!(bytes.get(digits), Some(b'.' | b')')) && bytes.get(digits + 1) == Some(&b' ') {
        digits + 2
    } else {
        0
    }
}

fn any_list_prefix_len(content: &str) -> usize {
    let task_len = task_prefix_len(content);
    if task_len > 0 {
        return task_len;
    }
    let unordered_len = unordered_prefix_len(content);
    if unordered_len > 0 {
        return unordered_len;
    }
    ordered_prefix_len(content)
}

fn has_line_prefix(line: &str, prefix: LinePrefix) -> bool {
    let content = &line[leading_whitespace(line)..];
    match prefix {
        LinePrefix::Unordered => unordered_prefix_len(content) > 0,
        LinePrefix::Ordered => ordered_prefix_len(content) > 0,
        LinePrefix::Task => task_prefix_len(content) > 0,
        LinePrefix::Quote => content.starts_with("> ") || content == ">",
    }
}

fn remove_line_prefix(line: &str, prefix: LinePrefix) -> String {
    let indentation = leading_whitespace(line);
    let content = &line[indentation..];
    let marker_length = match prefix {
        LinePrefix::Unordered => unordered_prefix_len(content),
        LinePrefix::Ordered => ordered_prefix_len(content),
        LinePrefix::Task => task_prefix_len(content),
        LinePrefix::Quote if content.starts_with("> ") => 2,
        LinePrefix::Quote if content == ">" => 1,
        _ => 0,
    };
    format!("{}{}", &line[..indentation], &content[marker_length..])
}

fn add_line_prefix(line: &str, prefix: LinePrefix) -> String {
    let indentation = leading_whitespace(line);
    let content = &line[indentation..];
    let stripped_len = match prefix {
        LinePrefix::Unordered | LinePrefix::Ordered | LinePrefix::Task => {
            any_list_prefix_len(content)
        }
        LinePrefix::Quote => 0,
    };
    let marker = match prefix {
        LinePrefix::Unordered => "- ",
        LinePrefix::Ordered => "1. ",
        LinePrefix::Task => "- [ ] ",
        LinePrefix::Quote => "> ",
    };
    format!("{}{}{}", &line[..indentation], marker, &content[stripped_len..])
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
