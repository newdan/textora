//! Enter and Backspace navigation for editor-only empty paragraphs.

use super::{debug_assert_augmentation, preferred_newline_sequence};
use crate::builder::{EditableParagraphMap, EditableParagraphRun, MarkdownDoc};
use crate::parser::parse_markdown;
use std::ops::Range;
use ui::plugin::EditAugmentation;

pub(super) fn enter(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    if !is_blank_or_quote_only_line(source, current_byte) {
        return None;
    }

    let document = parse_structure(source);
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    let (run, line_index) = line_at(&paragraphs, current_byte)?;
    if is_container_exit_primitive(run) || line_index < run.hidden_separator_count {
        return None;
    }
    let line = &run.lines[line_index];
    let prefix = source.get(line.source_range.start..line.source_byte)?;
    let newline = preferred_newline_sequence(source, current_byte);
    let inserted_line_count = inserted_line_count(run);
    let (insert_at, insertion, cursor_byte_after) = if line.newline_range.is_empty() {
        let insertion = format!("{newline}{prefix}").repeat(inserted_line_count);
        let cursor_byte_after = line.source_range.end + insertion.len();
        (line.source_range.end, insertion, cursor_byte_after)
    } else {
        let insertion = format!("{prefix}{newline}").repeat(inserted_line_count);
        let cursor_byte_after = line.newline_range.end + insertion.len() - newline.len();
        (line.newline_range.end, insertion, cursor_byte_after)
    };
    let augmentation = EditAugmentation {
        replace_range: Some(insert_at..insert_at),
        insert_text: Some(insertion),
        cursor_byte_after,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(super) fn backspace(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    if !is_blank_or_quote_only_line(source, current_byte) {
        return None;
    }

    let document = parse_structure(source);
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    let (run, line_index) = line_at(&paragraphs, current_byte)?;
    if is_container_exit_primitive(run) {
        return None;
    }
    let (delete_range, cursor_byte_after) = if line_index < run.hidden_separator_count {
        hidden_separator_backspace(source, run, line_index)?
    } else if line_index > run.hidden_separator_count {
        let previous_line = &run.lines[line_index - 1];
        (line_deletion_range(source, &run.lines[line_index])?, previous_line.source_byte)
    } else {
        first_line_backspace(source, run, line_index)?
    };
    let augmentation = EditAugmentation {
        replace_range: Some(delete_range),
        insert_text: Some(String::new()),
        cursor_byte_after,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

fn line_at(
    paragraphs: &EditableParagraphMap,
    current_byte: usize,
) -> Option<(&EditableParagraphRun, usize)> {
    let run = paragraphs.run_at_byte(current_byte)?;
    let line_index = run.lines.iter().position(|line| line.source_byte == current_byte)?;
    Some((run, line_index))
}

fn inserted_line_count(run: &EditableParagraphRun) -> usize {
    let hidden_count_after_one_line = usize::from(
        run.has_preceding_block && (run.has_following_block || run.lines.len() + 1 > 1),
    );
    1 + hidden_count_after_one_line.saturating_sub(run.hidden_separator_count)
}

fn is_container_exit_primitive(run: &EditableParagraphRun) -> bool {
    let editable_line_count = run.lines.len().saturating_sub(run.hidden_separator_count);
    !run.owner_path.is_empty()
        && !run.has_preceding_block
        && !run.has_following_block
        && editable_line_count == 1
}

fn hidden_separator_backspace(
    source: &str,
    run: &EditableParagraphRun,
    line_index: usize,
) -> Option<(Range<usize>, usize)> {
    let line = &run.lines[line_index];
    let first_line = run.lines.first()?;
    let preceding_newline_width =
        super::newline_sequence_width_before(source, first_line.source_range.start)?;
    let preceding_block_end = first_line.source_range.start - preceding_newline_width;
    Some((line_deletion_range(source, line)?, preceding_block_end))
}

fn first_line_backspace(
    source: &str,
    run: &EditableParagraphRun,
    line_index: usize,
) -> Option<(Range<usize>, usize)> {
    if !run.has_preceding_block {
        return None;
    }

    let first_line = run.lines.first()?;
    let preceding_newline_width =
        super::newline_sequence_width_before(source, first_line.source_range.start)?;
    let preceding_block_end = first_line.source_range.start - preceding_newline_width;
    let line = &run.lines[line_index];
    let delete_range = if line.newline_range.is_empty() {
        preceding_block_end..line.source_range.end
    } else {
        line.source_range.start..line.newline_range.end
    };
    Some((delete_range, preceding_block_end))
}

fn line_deletion_range(
    source: &str,
    line: &crate::builder::EditableParagraphLine,
) -> Option<Range<usize>> {
    if !line.newline_range.is_empty() {
        return Some(line.source_range.start..line.newline_range.end);
    }

    let preceding_newline_width =
        super::newline_sequence_width_before(source, line.source_range.start)?;
    Some(line.source_range.start - preceding_newline_width..line.source_range.end)
}

fn is_blank_or_quote_only_line(source: &str, current_byte: usize) -> bool {
    let Some((line_start, _, line_end)) = super::locate_source_line_bounds(source, current_byte)
    else {
        return false;
    };
    let content_end = super::source_line_content_end(source, line_end);
    let Some(line) = source.get(line_start..content_end) else {
        return false;
    };
    if line.chars().all(char::is_whitespace) {
        return true;
    }

    let mut remaining = line.trim_start_matches([' ', '\t']);
    let mut quote_depth = 0;
    while let Some(after_marker) = remaining.strip_prefix('>') {
        quote_depth += 1;
        remaining = after_marker.trim_start_matches([' ', '\t']);
    }
    quote_depth > 0 && remaining.is_empty()
}

fn parse_structure(source: &str) -> MarkdownDoc {
    #[cfg(test)]
    super::CLASSIFY_PARSE_COUNT.with(|count| count.set(count.get() + 1));
    MarkdownDoc::build_structure(&parse_markdown(source))
}
