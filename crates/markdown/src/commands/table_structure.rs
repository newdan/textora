use super::{
    CRLF_SEQUENCE, LF_SEQUENCE, MAX_TABLE_COLUMNS, apply_transaction, list_marker_content_start,
};
use ui::plugin::SemanticEditPlan;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableStructureCapabilities {
    pub can_delete_row: bool,
    pub can_delete_column: bool,
}

pub fn table_structure_capabilities(
    source: &str,
    cursor_byte: usize,
) -> Option<TableStructureCapabilities> {
    let table = source_table_at(source, cursor_byte)?;
    let can_delete_row = table
        .rows
        .iter()
        .skip(1)
        .any(|row| row.range.start <= cursor_byte && cursor_byte <= row.range.end);
    Some(TableStructureCapabilities { can_delete_row, can_delete_column: table.column_count > 1 })
}

pub(super) fn plan_table_structure(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    operation: ui::plugin::TableStructureCommand,
) -> SemanticEditPlan {
    let Some(table) = source_table_at(source, cursor_byte) else {
        return SemanticEditPlan::Unsupported;
    };
    let Some(active_row) = table.rows.iter().position(|row| row.range.contains(&cursor_byte))
    else {
        return SemanticEditPlan::Unsupported;
    };
    let Some(active_column) = table.rows[active_row]
        .cells
        .iter()
        .position(|cell| cell.start <= cursor_byte && cursor_byte <= cell.end)
    else {
        return SemanticEditPlan::Unsupported;
    };
    if let ui::plugin::TableStructureCommand::SetColumnAlignment(alignment) = operation {
        let separator_cell = &table.rows[1].cells[active_column];
        let replacement = alignment_separator(alignment).to_owned();
        if source[separator_cell.clone()] == replacement {
            return SemanticEditPlan::NoChange;
        }
        return apply_transaction(
            source_generation,
            separator_cell.clone(),
            replacement.clone(),
            separator_cell.start + replacement.len(),
        );
    }
    let body_row_count = table.rows.len().saturating_sub(2);
    let (replacement, cursor_in_replacement) = match operation {
        ui::plugin::TableStructureCommand::DeleteTable => (String::new(), 0),
        ui::plugin::TableStructureCommand::DeleteRow if active_row == 0 => {
            return SemanticEditPlan::Unsupported;
        }
        ui::plugin::TableStructureCommand::DeleteRow => {
            let mut rows = table.row_texts.clone();
            if body_row_count == 1 {
                rows[active_row] = format!(
                    "{}{}",
                    table.row_prefixes[active_row],
                    empty_table_row_with_style(
                        table.column_count,
                        table.row_leading_pipes[active_row],
                        table.row_trailing_pipes[active_row],
                    )
                );
            } else {
                rows.remove(active_row);
            }
            let cursor_line = active_row.min(rows.len().saturating_sub(1));
            (rows.join(table.newline), cursor_line)
        }
        ui::plugin::TableStructureCommand::InsertRowBefore
        | ui::plugin::TableStructureCommand::InsertRowAfter => {
            let mut rows = table.row_texts.clone();
            let insertion =
                if matches!(operation, ui::plugin::TableStructureCommand::InsertRowBefore) {
                    active_row.max(2)
                } else {
                    (active_row + 1).max(2)
                };
            let prefix_row = insertion.saturating_sub(1).min(table.row_prefixes.len() - 1);
            let inserted_row = format!(
                "{}{}",
                table.row_prefixes[prefix_row],
                empty_table_row_with_style(
                    table.column_count,
                    table.row_leading_pipes[prefix_row],
                    table.row_trailing_pipes[prefix_row],
                )
            );
            rows.insert(insertion, inserted_row);
            (rows.join(table.newline), insertion)
        }
        ui::plugin::TableStructureCommand::DeleteColumn if table.column_count <= 1 => {
            return SemanticEditPlan::Unsupported;
        }
        ui::plugin::TableStructureCommand::DeleteColumn
        | ui::plugin::TableStructureCommand::InsertColumnBefore
        | ui::plugin::TableStructureCommand::InsertColumnAfter
        | ui::plugin::TableStructureCommand::SetColumnAlignment(_) => {
            let mut rows = table.row_cells.clone();
            let target_column = match operation {
                ui::plugin::TableStructureCommand::InsertColumnBefore => active_column,
                ui::plugin::TableStructureCommand::InsertColumnAfter => active_column + 1,
                _ => active_column,
            };
            for (row_index, cells) in rows.iter_mut().enumerate() {
                match operation {
                    ui::plugin::TableStructureCommand::InsertColumnBefore
                    | ui::plugin::TableStructureCommand::InsertColumnAfter => {
                        if table.column_count >= MAX_TABLE_COLUMNS {
                            return SemanticEditPlan::Unsupported;
                        }
                        let inserted_value = if row_index == 1 { "---" } else { "" };
                        cells.insert(target_column, inserted_value.to_owned());
                    }
                    ui::plugin::TableStructureCommand::DeleteColumn => {
                        cells.remove(active_column);
                    }
                    ui::plugin::TableStructureCommand::SetColumnAlignment(alignment)
                        if row_index == 1 =>
                    {
                        cells[active_column] = alignment_separator(alignment).to_owned();
                    }
                    _ => {}
                }
            }
            let rewritten = rows
                .into_iter()
                .enumerate()
                .map(|(row_index, cells)| {
                    // GFM treats a final pipe as a trailing edge marker. When a
                    // newly inserted final cell is empty, keep that marker so
                    // the empty cell remains represented in the parsed table.
                    let trailing_pipe = table.row_trailing_pipes[row_index]
                        || cells.last().is_some_and(String::is_empty);
                    format!(
                        "{}{}",
                        table.row_prefixes[row_index],
                        format_table_row_with_style(
                            &cells,
                            table.row_leading_pipes[row_index],
                            trailing_pipe,
                        )
                    )
                })
                .collect::<Vec<_>>()
                .join(table.newline);
            (rewritten, active_row)
        }
    };
    let edit_range = if matches!(operation, ui::plugin::TableStructureCommand::DeleteTable) {
        let mut expanded = table.range.clone();
        if expanded.start >= table.newline.len() * 2
            && source[..expanded.start].ends_with(&format!("{}{}", table.newline, table.newline))
        {
            expanded.start -= table.newline.len();
        }
        if source[expanded.end..].starts_with(&format!("{}{}", table.newline, table.newline)) {
            expanded.end += table.newline.len();
        }
        expanded
    } else {
        table.range.clone()
    };
    let cursor_after = if matches!(operation, ui::plugin::TableStructureCommand::DeleteTable) {
        edit_range.start
    } else {
        let target_column =
            if matches!(operation, ui::plugin::TableStructureCommand::InsertColumnAfter) {
                active_column + 1
            } else {
                active_column
            };
        let target_row = replacement.lines().nth(cursor_in_replacement).unwrap_or("");
        let target_cell_ranges =
            scan_table_row(target_row, 0, Some(table.column_count)).map(|(_, cells)| cells);
        let target_cell = target_cell_ranges
            .as_ref()
            .and_then(|cells| cells.get(target_column.min(cells.len().saturating_sub(1))))
            .map_or(0, |cell| cell.start);
        table.range.start
            + replacement.lines().take(cursor_in_replacement).map(str::len).sum::<usize>()
            + table.newline.len().saturating_mul(cursor_in_replacement)
            + target_cell
    };
    apply_transaction(source_generation, edit_range, replacement, cursor_after)
}

pub(super) struct SourceTable {
    range: std::ops::Range<usize>,
    rows: Vec<SourceTableRow>,
    row_texts: Vec<String>,
    row_prefixes: Vec<String>,
    row_leading_pipes: Vec<bool>,
    row_trailing_pipes: Vec<bool>,
    row_cells: Vec<Vec<String>>,
    column_count: usize,
    newline: &'static str,
}

struct SourceTableRow {
    range: std::ops::Range<usize>,
    cells: Vec<std::ops::Range<usize>>,
}

pub(super) fn source_table_at(source: &str, cursor_byte: usize) -> Option<SourceTable> {
    use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
    let parsed = crate::parser::parse_markdown(source);
    let mut table_start = None;
    let mut table_range = None;
    let mut table_column_count = None;
    for (index, event) in parsed.events.iter().enumerate() {
        match event {
            MarkdownEvent::Start(MarkdownTag::Table(alignments)) => {
                table_start = Some(parsed.event_ranges[index].start);
                table_column_count = Some(alignments.len());
            }
            MarkdownEvent::End(MarkdownTagEnd::Table) => {
                if let Some(start) = table_start.take() {
                    let end = parsed.event_ranges[index].end;
                    if start <= cursor_byte && cursor_byte <= end {
                        table_range = Some(start..end);
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    let parsed_range = table_range?;
    let parser_column_count = table_column_count?;
    let range_start = source[..parsed_range.start].rfind('\n').map_or(0, |offset| offset + 1);
    let range_end = if source[..parsed_range.end].ends_with('\n') {
        parsed_range.end
    } else {
        source[parsed_range.end..]
            .find('\n')
            .map_or(source.len(), |offset| parsed_range.end + offset)
    };
    let range_end = if range_end > range_start && source.as_bytes()[range_end - 1] == b'\n' {
        range_end - 1
    } else {
        range_end
    };
    let range_end = if range_end > range_start && source.as_bytes()[range_end - 1] == b'\r' {
        range_end - 1
    } else {
        range_end
    };
    let range = range_start..range_end;
    let raw = &source[range.clone()];
    let newline = if raw.contains(CRLF_SEQUENCE) { CRLF_SEQUENCE } else { LF_SEQUENCE };
    let mut rows = Vec::new();
    let mut row_texts = Vec::new();
    let mut row_prefixes = Vec::new();
    let mut row_leading_pipes = Vec::new();
    let mut row_trailing_pipes = Vec::new();
    let mut row_cells = Vec::new();
    let mut offset = range.start;
    for line in raw.split(newline) {
        let style = table_row_pipe_style(line, Some(parser_column_count))?;
        let (cells, cell_ranges) = scan_table_row(line, offset, Some(parser_column_count))?;
        row_prefixes.push(style.prefix);
        row_leading_pipes.push(style.leading_pipe);
        row_trailing_pipes.push(style.trailing_pipe);
        rows.push(SourceTableRow { range: offset..offset + line.len(), cells: cell_ranges });
        row_texts.push(line.to_owned());
        row_cells.push(cells);
        offset += line.len() + newline.len();
    }
    if rows.len() < 2 || row_cells[0].len() != row_cells[1].len() {
        return None;
    }
    let column_count = row_cells[0].len();
    for (row_index, cells) in row_cells.iter_mut().enumerate() {
        if cells.len() > column_count {
            return None;
        }
        let missing_cells = column_count - cells.len();
        if missing_cells > 0 {
            cells.extend(std::iter::repeat_n(String::new(), missing_cells));
            let row_end = rows[row_index].range.end;
            rows[row_index].cells.extend(std::iter::repeat_n(row_end..row_end, missing_cells));
        }
    }
    let active_row = rows.iter().position(|row| row.range.contains(&cursor_byte))?;
    if active_row == 1 {
        return None;
    }
    Some(SourceTable {
        range,
        rows,
        row_texts,
        row_prefixes,
        row_leading_pipes,
        row_trailing_pipes,
        row_cells,
        column_count,
        newline,
    })
}

struct TableRowPipeStyle {
    prefix: String,
    content_start: usize,
    leading_pipe: bool,
    trailing_pipe: bool,
}

fn table_row_pipe_style(
    line: &str,
    expected_column_count: Option<usize>,
) -> Option<TableRowPipeStyle> {
    let trimmed = line.trim_start();
    let indentation = line.len() - trimmed.len();
    let mut prefix = line[..indentation].to_owned();
    let mut remaining = trimmed;
    while let Some(after_quote) = remaining.strip_prefix('>') {
        prefix.push('>');
        remaining = after_quote;
        if let Some(after_space) = remaining.strip_prefix(' ') {
            prefix.push(' ');
            remaining = after_space;
        }
        let quote_indent = remaining.len() - remaining.trim_start().len();
        prefix.push_str(&" ".repeat(quote_indent));
        remaining = &remaining[quote_indent..];
    }
    if let Some(marker_end) = list_marker_content_start(remaining) {
        prefix.push_str(&remaining[..marker_end]);
        remaining = &remaining[marker_end..];
    }
    let content_start = line.len() - remaining.len();
    let trimmed_content_end = line.trim_end().len();
    if content_start >= trimmed_content_end {
        return None;
    }
    let separators = unescaped_pipe_positions(line);
    let has_only_cell_separators =
        expected_column_count.is_some_and(|count| separators.len() == count.saturating_sub(1));
    let leading_pipe = !has_only_cell_separators && separators.first() == Some(&content_start);
    let trailing_pipe =
        !has_only_cell_separators && separators.last() == Some(&(trimmed_content_end - 1));
    Some(TableRowPipeStyle { prefix, content_start, leading_pipe, trailing_pipe })
}

fn scan_table_row(
    line: &str,
    line_start: usize,
    expected_column_count: Option<usize>,
) -> Option<(Vec<String>, Vec<std::ops::Range<usize>>)> {
    let separators = unescaped_pipe_positions(line);
    let mut values = Vec::new();
    let mut ranges = Vec::new();
    let style = table_row_pipe_style(line, expected_column_count)?;
    let content_end = line.trim_end().len();
    let start_after_leading = style.content_start + usize::from(style.leading_pipe);
    let end_before_trailing = content_end - usize::from(style.trailing_pipe);
    let inner_separators = separators
        .into_iter()
        .filter(|separator| *separator >= start_after_leading && *separator < end_before_trailing)
        .collect::<Vec<_>>();
    let mut cell_start = start_after_leading;
    for cell_end in inner_separators.into_iter().chain(std::iter::once(end_before_trailing)) {
        let end = cell_end;
        let start = cell_start;
        let cell = &line[start..end];
        let leading = cell.len() - cell.trim_start().len();
        let value = cell.trim();
        let absolute_start = line_start + start + leading;
        values.push(value.to_owned());
        ranges.push(absolute_start..absolute_start + value.len());
        cell_start = end + 1;
    }
    Some((values, ranges))
}

fn unescaped_pipe_positions(line: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut separators = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == b'`' {
            let run_end = bytes[index..]
                .iter()
                .position(|byte| *byte != b'`')
                .map_or(bytes.len(), |length| index + length);
            let run_length = run_end - index;
            if let Some(code_end) = matching_code_span_end(bytes, run_end, run_length) {
                index = code_end;
            } else {
                index = run_end;
            }
            continue;
        }
        if bytes[index] == b'|' {
            separators.push(index);
        }
        index += 1;
    }
    separators
}

fn matching_code_span_end(bytes: &[u8], mut index: usize, tick_count: usize) -> Option<usize> {
    while index < bytes.len() {
        if bytes[index] != b'`' {
            index += 1;
            continue;
        }
        let run_end = bytes[index..]
            .iter()
            .position(|byte| *byte != b'`')
            .map_or(bytes.len(), |length| index + length);
        if run_end - index == tick_count {
            return Some(run_end);
        }
        index = run_end;
    }
    None
}

pub(super) fn format_table_row(cells: &[String]) -> String {
    format_table_row_with_style(cells, true, true)
}

fn format_table_row_with_style(
    cells: &[String],
    leading_pipe: bool,
    trailing_pipe: bool,
) -> String {
    let mut row = cells.join(" | ");
    if leading_pipe {
        row.insert_str(0, "| ");
    }
    if trailing_pipe {
        row.push_str(" |");
    }
    row
}

fn empty_table_row_with_style(columns: usize, leading_pipe: bool, trailing_pipe: bool) -> String {
    format_table_row_with_style(&vec![String::new(); columns], leading_pipe, trailing_pipe)
}

fn alignment_separator(alignment: ui::plugin::TableColumnAlignment) -> &'static str {
    match alignment {
        ui::plugin::TableColumnAlignment::Default => "---",
        ui::plugin::TableColumnAlignment::Left => ":---",
        ui::plugin::TableColumnAlignment::Center => ":---:",
        ui::plugin::TableColumnAlignment::Right => "---:",
    }
}
