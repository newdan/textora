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

pub(super) fn erase_range(
    source: &str,
    erased: std::ops::Range<usize>,
) -> Option<EditAugmentation> {
    if erased.is_empty() || source.get(erased.clone()).is_none() {
        return None;
    }
    let document = parse_structure(source);
    let (paragraph, owner_path) = erased_paragraph(&document.blocks, &erased, &[])?;
    let content_start = paragraph.block_range.start;
    let paragraph_end = paragraph.block_range.end;
    let trailing_newline_width =
        super::newline_sequence_width_before(source, paragraph_end).unwrap_or(0);
    let mut replacement_end = paragraph_end - trailing_newline_width;
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    if let Some(newline_width) = super::newline_sequence_width_at(source, replacement_end) {
        let next_line_start = replacement_end + newline_width;
        if let Some(run) = paragraphs.run_at_byte(next_line_start)
            && run.hidden_separator_count > 0
            && run.owner_path == owner_path
            && let Some(separator) = run.lines.first()
            && separator.source_range.start == next_line_start
        {
            replacement_end = separator.source_range.end;
        }
    }
    Some(EditAugmentation {
        replace_range: Some(content_start..replacement_end),
        insert_text: Some(String::new()),
        cursor_byte_after: content_start,
    })
}

pub(super) fn erase_last_grapheme(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (_, _, line_end) = super::locate_source_line_bounds(source, current_byte)?;
    let content_end = super::source_line_content_end(source, line_end);
    let suffix = source.get(current_byte..content_end)?;
    if !suffix.is_empty() && !suffix.starts_with(['*', '_', '~', '`', ']']) {
        return None;
    }
    let (grapheme_start, _) = source.get(..current_byte)?.grapheme_indices(true).next_back()?;
    erase_range(source, grapheme_start..current_byte)
}

fn erased_paragraph<'a>(
    blocks: &'a [BlockNode],
    erased: &std::ops::Range<usize>,
    owner: &[usize],
) -> Option<(&'a BlockNode, Vec<usize>)> {
    for (index, block) in blocks.iter().enumerate() {
        if erased.start < block.block_range.start || erased.end > block.block_range.end {
            continue;
        }
        if matches!(block.kind, BlockKind::Paragraph)
            && !block.projected_lines.is_empty()
            && block.projected_lines.iter().all(|line| {
                !line.text.is_empty()
                    && line.spans.iter().filter(|span| !span.visual_range.is_empty()).all(|span| {
                        erased.start <= span.source_range.start
                            && span.source_range.end <= erased.end
                    })
            })
        {
            return Some((block, owner.to_vec()));
        }
        let mut child_owner = owner.to_vec();
        child_owner.push(index);
        if let Some(paragraph) = erased_paragraph(&block.children, erased, &child_owner) {
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

#[cfg(test)]
#[path = "editable_paragraph_erasure_tests.rs"]
mod erasure_tests;
