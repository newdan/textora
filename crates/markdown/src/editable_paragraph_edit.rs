//! Source edits use the same empty paragraph ownership as the layout builder.

use super::{canonical_container_prefix, preferred_newline_sequence};
use crate::builder::{
    BlockKind, BlockNode, EditableParagraphMap, EditableParagraphRun, MarkdownDoc,
};
use crate::parser::parse_markdown;
use ui::plugin::EditAugmentation;
use unicode_segmentation::UnicodeSegmentation;

pub(super) fn insert_text(
    source: &str,
    current_byte: usize,
    text: &str,
) -> Option<EditAugmentation> {
    if text.bytes().all(|byte| matches!(byte, b' ' | b'\t')) || text.contains(['\r', '\n']) {
        return None;
    }
    let (line_start, _, line_end) = super::locate_source_line_bounds(source, current_byte)?;
    if !source[line_start..line_end]
        .chars()
        .all(|character| character.is_whitespace() || character == '>')
    {
        return None;
    }
    let document = parse_structure(source);
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    let run = paragraphs.run_at_byte(current_byte)?;
    let line_index = run.lines.iter().position(|line| {
        line.source_range.contains(&current_byte) || line.source_range.end == current_byte
    })?;
    let line = &run.lines[line_index];
    if current_byte < line.source_byte {
        return None;
    }
    Some(materialize_paragraph(source, current_byte, text, run, line_index))
}

fn materialize_paragraph(
    source: &str,
    current_byte: usize,
    text: &str,
    run: &EditableParagraphRun,
    line_index: usize,
) -> EditAugmentation {
    let line = &run.lines[line_index];
    let prefix = run
        .continuation_source_start
        .map(|start| canonical_container_prefix(source, start))
        .unwrap_or_default();
    let newline = preferred_newline_sequence(source, current_byte);
    let needs_left_separator = line_index == 0 && run.has_preceding_block;
    let needs_right_separator = !line.newline_range.is_empty();
    let mut insertion = String::new();
    if needs_left_separator {
        insertion.push_str(&prefix);
        insertion.push_str(newline);
    }
    insertion.push_str(&prefix);
    insertion.push_str(text);
    let cursor_byte_after = line.source_range.start + insertion.len();
    if needs_right_separator {
        insertion.push_str(newline);
        insertion.push_str(&prefix);
    }
    EditAugmentation {
        replace_range: Some(line.source_range.clone()),
        insert_text: Some(insertion),
        cursor_byte_after,
    }
}

pub(super) fn erase_last_grapheme(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (_, _, line_end) = super::locate_source_line_bounds(source, current_byte)?;
    if current_byte != super::source_line_content_end(source, line_end) {
        return None;
    }
    let newline_width = super::newline_sequence_width_at(source, current_byte)?;
    let document = parse_structure(source);
    let (paragraph, owner_path) = single_grapheme_paragraph(&document.blocks, current_byte, &[])?;
    let projection = paragraph.projected_lines.first()?;
    let content_start = projection.source_extent().start;
    if source.get(content_start..current_byte)? != paragraph.text_lines.first()? {
        return None;
    }
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    let next_line_start = current_byte + newline_width;
    let run = paragraphs.run_at_byte(next_line_start)?;
    let separator = run.lines.first()?;
    if run.hidden_separator_count == 0
        || run.owner_path != owner_path
        || separator.source_range.start != next_line_start
    {
        return None;
    }
    let (line_start, _, _) = super::locate_source_line_bounds(source, content_start)?;
    let prefix = &source[line_start..content_start];
    let cursor_byte_after = if prefix.contains('>') { content_start } else { line_start };
    Some(EditAugmentation {
        replace_range: Some(content_start..separator.source_range.end),
        insert_text: Some(String::new()),
        cursor_byte_after,
    })
}

fn single_grapheme_paragraph<'a>(
    blocks: &'a [BlockNode],
    cursor: usize,
    owner: &[usize],
) -> Option<(&'a BlockNode, Vec<usize>)> {
    for (index, block) in blocks.iter().enumerate() {
        if !(block.block_range.start <= cursor && cursor <= block.block_range.end) {
            continue;
        }
        if matches!(block.kind, BlockKind::Paragraph)
            && block.text_lines.len() == 1
            && block.text_lines[0].graphemes(true).count() == 1
        {
            return Some((block, owner.to_vec()));
        }
        let mut child_owner = owner.to_vec();
        child_owner.push(index);
        if let Some(paragraph) = single_grapheme_paragraph(&block.children, cursor, &child_owner) {
            return Some(paragraph);
        }
    }
    None
}

fn parse_structure(source: &str) -> MarkdownDoc {
    #[cfg(test)]
    super::CLASSIFY_PARSE_COUNT.with(|count| count.set(count.get() + 1));
    MarkdownDoc::build_structure(&parse_markdown(source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_before_an_expanded_quote_marker_preserves_source_editing() {
        let source = "> first\n>\n>\n> second";
        let marker_start = "> first\n>\n".len();
        assert!(insert_text(source, marker_start, "x").is_none());
        assert!(insert_text(source, marker_start + 1, "x").is_some());
    }
}
