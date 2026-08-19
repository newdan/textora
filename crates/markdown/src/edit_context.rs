use crate::builder::ListBullet;
use crate::layout::source_line_map::{
    EmptyRunPosition, SourceLineEntry, SourceLineMap, SourceLineRole,
};
use std::ops::Range;
use ui::plugin::EditRequest;

const MAX_LEADING_BLOCK_INDENT: usize = 3;

#[derive(Debug)]
pub enum MarkdownBlockContext {
    Paragraph {
        content_range: Range<usize>,
    },
    Heading {
        level: u8,
        content_range: Range<usize>,
    },
    ListItem {
        marker_range: Range<usize>,
        content_range: Range<usize>,
        indent: String,
        bullet: ListBullet,
    },
    BlockQuote {
        marker_ranges: Vec<Range<usize>>,
        content_range: Range<usize>,
    },
    CodeBlock {
        content_range: Range<usize>,
    },
    TableCell {
        content_range: Range<usize>,
        next_row_same_column: Option<usize>,
    },
    Other,
}

pub fn classify_markdown_edit_context(
    source: &str,
    source_map: &SourceLineMap,
    request: &EditRequest,
) -> MarkdownEditContext {
    let mut source_line = source_map.line_at_byte(request.cursor_byte).unwrap_or(
        crate::layout::source_line_map::SourceLineEntry {
            index: 0,
            start: 0,
            end: 0,
            is_blank: true,
            role: crate::layout::source_line_map::SourceLineRole::Other,
            y_top: 0.0,
            height: 0.0,
        },
    );

    if source_line.is_empty() {
        source_line.role = match source_line.role {
            SourceLineRole::EditableEmpty | SourceLineRole::HiddenBlockSeparator => {
                source_line.role
            }
            _ => {
                let has_previous = source_map.previous_non_empty(source_line.index).is_some();
                let has_next = source_map.next_non_empty(source_line.index).is_some();
                if has_previous
                    && has_next
                    && source_map
                        .empty_run_position(source_line.index)
                        .is_some_and(|position| position.index_in_run == 0)
                {
                    SourceLineRole::HiddenBlockSeparator
                } else {
                    SourceLineRole::EditableEmpty
                }
            }
        };
    }

    let mut block = MarkdownBlockContext::Other;

    if !source_line.is_empty() {
        let frames = collect_context_frames(source, request.cursor_byte);
        block = choose_block_context(source, request.cursor_byte, frames);
    }

    MarkdownEditContext {
        empty_run_position: source_map.empty_run_position(source_line.index),
        source_line,
        block,
        cursor_byte: request.cursor_byte,
        selection: request.selection.clone(),
    }
}

#[derive(Default)]
struct ContextFrames {
    list_item: Option<Range<usize>>,
    block_quote: bool,
    heading: Option<(u8, Range<usize>)>,
    code_block: Option<Range<usize>>,
    paragraph: Option<Range<usize>>,
}

fn collect_context_frames(source: &str, cursor: usize) -> ContextFrames {
    use pulldown_cmark::{Event, Parser, Tag};

    let mut frames = ContextFrames::default();
    for (event, range) in
        Parser::new_ext(source, crate::parser::markdown_options()).into_offset_iter()
    {
        if !range_contains_cursor(&range, cursor) {
            continue;
        }

        match event {
            Event::Start(Tag::Item) => frames.list_item = Some(range),
            Event::Start(Tag::BlockQuote(_)) => frames.block_quote = true,
            Event::Start(Tag::Heading { level, .. }) => {
                frames.heading = Some((level as u8, range));
            }
            Event::Start(Tag::CodeBlock(_)) => frames.code_block = Some(range),
            Event::Start(Tag::Paragraph) => frames.paragraph = Some(range),
            _ => {}
        }
    }
    frames
}

fn choose_block_context(
    source: &str,
    cursor: usize,
    frames: ContextFrames,
) -> MarkdownBlockContext {
    if let Some((content_range, next_row_same_column)) = table_cell_context(source, cursor) {
        return MarkdownBlockContext::TableCell { content_range, next_row_same_column };
    }

    if let Some(item_range) = frames.list_item
        && let Some((marker_start, bullet, content_start)) = list_item_marker(source, &item_range)
    {
        return MarkdownBlockContext::ListItem {
            marker_range: marker_start..content_start,
            content_range: content_start..item_range.end,
            indent: crate::augmenter::list_item_indent(source, marker_start),
            bullet,
        };
    }

    if frames.block_quote {
        let (marker_ranges, content_range) = block_quote_line_ranges(source, cursor);
        return MarkdownBlockContext::BlockQuote { marker_ranges, content_range };
    }

    if let Some((level, tag_range)) = frames.heading {
        return MarkdownBlockContext::Heading {
            level,
            content_range: heading_content_range(source, tag_range),
        };
    }

    if let Some(tag_range) = frames.code_block {
        return MarkdownBlockContext::CodeBlock {
            content_range: fenced_code_content_range(source, tag_range),
        };
    }

    if let Some(content_range) = frames.paragraph {
        return MarkdownBlockContext::Paragraph { content_range };
    }

    MarkdownBlockContext::Other
}

fn range_contains_cursor(range: &Range<usize>, cursor: usize) -> bool {
    range.contains(&cursor) || range.end == cursor
}

fn block_quote_line_ranges(source: &str, cursor: usize) -> (Vec<Range<usize>>, Range<usize>) {
    let line_start =
        source[..cursor.min(source.len())].rfind('\n').map_or(0, |newline| newline + 1);
    let line_end = source[cursor.min(source.len())..]
        .find('\n')
        .map_or(source.len(), |newline| cursor.min(source.len()) + newline);
    let bytes = source.as_bytes();
    let mut marker_ranges = Vec::new();
    let mut content_start =
        line_start + leading_space_count(source, line_start).min(MAX_LEADING_BLOCK_INDENT);

    while bytes.get(content_start) == Some(&b'>') {
        let marker_end = if matches!(bytes.get(content_start + 1), Some(b' ' | b'\t')) {
            content_start + 2
        } else {
            content_start + 1
        };
        marker_ranges.push(content_start..marker_end);
        content_start = marker_end;
    }

    (marker_ranges, content_start..line_end)
}

fn list_item_marker(source: &str, item_range: &Range<usize>) -> Option<(usize, ListBullet, usize)> {
    source[item_range.clone()].char_indices().find_map(|(offset, _)| {
        let marker_start = item_range.start + offset;
        crate::augmenter::parse_list_marker(source, marker_start)
            .map(|(bullet, content_start)| (marker_start, bullet, content_start))
    })
}

fn leading_space_count(source: &str, line_start: usize) -> usize {
    source[line_start..].bytes().take_while(|byte| *byte == b' ').count()
}

fn heading_content_range(source: &str, tag_range: Range<usize>) -> Range<usize> {
    let tag_source = &source[tag_range.clone()];
    if let Some(newline_offset) = tag_source.find('\n') {
        return tag_range.start..tag_range.start + newline_offset;
    }

    let hash_count = tag_source.bytes().take_while(|byte| *byte == b'#').count();
    let whitespace_count =
        tag_source[hash_count..].bytes().take_while(|byte| matches!(*byte, b' ' | b'\t')).count();
    (tag_range.start + hash_count + whitespace_count)..tag_range.end
}

fn fenced_code_content_range(source: &str, tag_range: Range<usize>) -> Range<usize> {
    let bytes = source.as_bytes();
    let opening_indent = leading_space_count(source, tag_range.start);
    if opening_indent > MAX_LEADING_BLOCK_INDENT {
        return tag_range;
    }
    let opening_fence_start = tag_range.start + opening_indent;
    let opening_fence = bytes.get(opening_fence_start).copied();
    let Some(fence_byte @ (b'`' | b'~')) = opening_fence else {
        return tag_range;
    };
    let fence_length = bytes[opening_fence_start..tag_range.end]
        .iter()
        .take_while(|byte| **byte == fence_byte)
        .count();
    if fence_length < 3 {
        return tag_range;
    }

    let Some(opening_newline) = source[tag_range.clone()].find('\n') else {
        return tag_range;
    };
    let content_start = tag_range.start + opening_newline + 1;
    let mut line_start = content_start;
    while line_start < tag_range.end {
        let line_end = source[line_start..tag_range.end]
            .find('\n')
            .map_or(tag_range.end, |newline| line_start + newline);
        let line = &source[line_start..line_end];
        let closing_indent = line.bytes().take_while(|byte| *byte == b' ').count();
        let fence_start = closing_indent.min(MAX_LEADING_BLOCK_INDENT);
        let fence_count =
            line[fence_start..].bytes().take_while(|byte| *byte == fence_byte).count();
        if closing_indent <= MAX_LEADING_BLOCK_INDENT
            && fence_count >= fence_length
            && line[fence_start + fence_count..].chars().all(char::is_whitespace)
        {
            return content_start..line_start;
        }
        line_start = line_end.saturating_add(1);
    }

    content_start..tag_range.end
}

fn table_cell_context(source: &str, cursor: usize) -> Option<(Range<usize>, Option<usize>)> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut rows: Vec<Vec<Range<usize>>> = Vec::new();
    for (event, range) in
        Parser::new_ext(source, crate::parser::markdown_options()).into_offset_iter()
    {
        match event {
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => rows.push(Vec::new()),
            Event::Start(Tag::TableCell) => {
                if let Some(row) = rows.last_mut() {
                    row.push(range);
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(cell) = rows.last_mut().and_then(|row| row.last_mut()) {
                    cell.end = range.end;
                }
            }
            _ => {}
        }
    }

    for (row_index, row) in rows.iter().enumerate() {
        for (column_index, cell_range) in row.iter().enumerate() {
            if range_contains_cursor(cell_range, cursor) {
                let next_row_same_column = rows
                    .get(row_index + 1)
                    .and_then(|next_row| next_row.get(column_index))
                    .map(|cell| crate::augmenter::table_cell_content_start(source, cell));
                return Some((cell_range.clone(), next_row_same_column));
            }
        }
    }

    None
}

pub struct MarkdownEditContext {
    pub source_line: SourceLineEntry,
    pub empty_run_position: Option<EmptyRunPosition>,
    pub block: MarkdownBlockContext,
    pub cursor_byte: usize,
    pub selection: Option<Range<usize>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::source_line_map::SourceLineRole;

    fn classify(source: &str, cursor_byte: usize) -> MarkdownEditContext {
        let source_map = SourceLineMap::from_source(source);
        let request = EditRequest {
            cursor_byte,
            selection: Some(cursor_byte..cursor_byte),
            source_generation: 0,
            intent: ui::plugin::EditIntent::InsertText(String::new()),
        };
        classify_markdown_edit_context(source, &source_map, &request)
    }

    fn assert_heading_range(source: &str, cursor_byte: usize, expected_range: Range<usize>) {
        let context = classify(source, cursor_byte);
        assert!(
            matches!(context.block, MarkdownBlockContext::Heading { content_range, .. } if content_range == expected_range)
        );
    }

    fn assert_code_range(source: &str, cursor_byte: usize, expected_range: Range<usize>) {
        let context = classify(source, cursor_byte);
        assert!(
            matches!(context.block, MarkdownBlockContext::CodeBlock { content_range } if content_range == expected_range)
        );
    }

    #[test]
    fn classifies_heading_interior_at_exact_cursor_byte() {
        let source = "# Title";
        let map = SourceLineMap::from_source(source);
        let request = EditRequest {
            cursor_byte: 4,
            selection: Some(4..4),
            source_generation: 0,
            intent: ui::plugin::EditIntent::InsertText(String::new()),
        };
        let ctx = classify_markdown_edit_context(source, &map, &request);
        assert!(
            matches!(ctx.block, MarkdownBlockContext::Heading { level: 1, content_range } if content_range == (2..7))
        );
    }

    #[test]
    fn classifies_cursor_on_second_trailing_empty_line_without_skipping_run_position() {
        let source = "Paragraph\n\n\n";
        let map = SourceLineMap::from_source(source);
        let request = EditRequest {
            cursor_byte: 11,
            selection: Some(11..11),
            source_generation: 0,
            intent: ui::plugin::EditIntent::InsertText(String::new()),
        };
        let ctx = classify_markdown_edit_context(source, &map, &request);
        assert_eq!(ctx.empty_run_position.unwrap().index_in_run, 1);
        assert!(matches!(ctx.block, MarkdownBlockContext::Other));
    }

    #[test]
    fn classifies_nested_blockquote_before_inner_paragraph() {
        let source = "> > quote";
        let context = classify(source, 5);
        assert!(matches!(context.block, MarkdownBlockContext::BlockQuote {
            marker_ranges,
            content_range,
        } if marker_ranges == vec![0..2, 2..4] && content_range == (4..9)));
    }

    #[test]
    fn classifies_indented_nested_blockquote_markers() {
        let source = "   > > quote";
        let context = classify(source, 8);
        assert!(matches!(context.block, MarkdownBlockContext::BlockQuote {
            marker_ranges,
            content_range,
        } if marker_ranges == vec![3..5, 5..7] && content_range == (7..12)));
    }

    #[test]
    fn classifies_ordered_and_tab_separated_list_markers() {
        let ordered = classify("42. item", 8);
        assert!(matches!(ordered.block, MarkdownBlockContext::ListItem {
            bullet: ListBullet::Ordered(42), marker_range, content_range, ..
        } if marker_range == (0..4) && content_range == (4..8)));

        let tabbed = classify("-\titem", 6);
        assert!(matches!(tabbed.block, MarkdownBlockContext::ListItem {
            marker_range, content_range, ..
        } if marker_range == (0..2) && content_range == (2..6)));
    }

    #[test]
    fn classifies_indented_and_nested_list_markers_from_physical_line_start() {
        let indented = classify("  - item", 8);
        assert!(matches!(indented.block, MarkdownBlockContext::ListItem {
            marker_range, content_range, indent, ..
        } if marker_range == (2..4) && content_range == (4..8) && indent == "  "));

        let source = "  - parent\n    - child";
        let nested = classify(source, source.len());
        assert!(matches!(nested.block, MarkdownBlockContext::ListItem {
            marker_range, content_range, indent, ..
        } if marker_range == (15..17) && content_range == (17..22) && indent == "    "));
    }

    #[test]
    fn classifies_list_item_inside_blockquote_from_item_range() {
        let context = classify("> - item", 8);
        assert!(matches!(context.block, MarkdownBlockContext::ListItem {
            marker_range, content_range, ..
        } if marker_range == (2..4) && content_range == (4..8)));
    }

    #[test]
    fn table_cell_points_to_next_row_same_column() {
        let source = "| A | B |\n|---|---|\n| C | D |";
        let context = classify(source, source.find('A').expect("first cell"));
        let next = source.find('C').expect("same column next row");
        assert!(matches!(context.block, MarkdownBlockContext::TableCell {
            next_row_same_column: Some(byte), ..
        } if byte == next));
    }

    #[test]
    fn heading_content_range_keeps_inline_markers_and_empty_atx_boundary() {
        assert_heading_range("# **Title**", 5, 2..11);
        assert_heading_range("#", 1, 1..1);
        assert_heading_range("标题\n====", 3, 0..6);
    }

    #[test]
    fn fenced_code_content_excludes_closing_fence_with_spaces_and_crlf() {
        assert_code_range("```rust\r\ncode\r\n```   \r\n", 10, 9..15);
    }

    #[test]
    fn fenced_code_content_excludes_indented_closing_fence_with_crlf_whitespace() {
        assert_code_range("   ```rust\r\ncode\r\n   ```   \r\n", 13, 12..18);
    }

    #[test]
    fn empty_document_and_trailing_blank_are_editable() {
        assert_eq!(classify("", 0).source_line.role, SourceLineRole::EditableEmpty);
        let source = "heading\n";
        assert_eq!(classify(source, source.len()).source_line.role, SourceLineRole::EditableEmpty);
    }

    #[test]
    fn middle_empty_line_is_a_hidden_block_separator() {
        let source = "alpha\n\nbeta";
        assert_eq!(classify(source, 6).source_line.role, SourceLineRole::HiddenBlockSeparator);
    }
}
