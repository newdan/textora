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
    if line_index >= run.hidden_separator_count
        && let Some(augmentation) = cross_owner_deletion(source, &document, run, cursor_byte_after)
    {
        return Some(augmentation);
    }
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

pub(super) fn backspace_boundary(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    let (line_start, _, _) = super::locate_source_line_bounds(source, current_byte)?;
    if line_start == 0 || !boundary_prefix_candidate(source.get(line_start..current_byte)?) {
        return None;
    }
    let document = parse_structure(source);
    if !has_text_boundary(&document.blocks, current_byte, Boundary::Start) {
        return None;
    }
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    let run = paragraphs
        .runs()
        .iter()
        .find(|run| run.lines.last().is_some_and(|line| line.newline_range.end == line_start))?;
    if let Some(augmentation) = cross_owner_deletion(source, &document, run, current_byte) {
        return Some(augmentation);
    }
    let line = run.lines.get(run.hidden_separator_count..)?.last()?;
    let deletion = container_start_deletion_range(source, run, line)
        .or_else(|| line_deletion_range(source, line))?;
    let cursor = current_byte.checked_sub(deletion.len())?;
    deletion_augmentation(source, deletion, cursor)
}

pub(super) fn delete_forward(source: &str, current_byte: usize) -> Option<EditAugmentation> {
    if current_byte >= source.len() {
        return None;
    }
    let (_, _, line_end) = super::locate_source_line_bounds(source, current_byte)?;
    let content_end = super::source_line_content_end(source, line_end);
    let suffix = source.get(current_byte..content_end)?;
    if !boundary_suffix_candidate(suffix) && !is_blank_or_quote_only_line(source, current_byte) {
        return None;
    }
    let document = parse_structure(source);
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    if let Some((run, index)) = line_at(&paragraphs, current_byte) {
        return delete_from_empty_paragraph(source, &document, run, index);
    }
    if !has_text_boundary(&document.blocks, current_byte, Boundary::End) {
        return None;
    }
    let next_line_start = source.get(content_end..)?.find('\n')? + content_end + 1;
    let run = paragraphs.runs().iter().find(|run| {
        run.lines.first().is_some_and(|line| line.source_range.start == next_line_start)
    })?;
    let editable = run.lines.get(run.hidden_separator_count..)?;
    let first = editable.first()?;
    if let Some(augmentation) = cross_owner_deletion(source, &document, run, current_byte) {
        return Some(augmentation);
    }
    let deletion = if run.lines.last()?.source_range.end == source.len() && editable.len() == 1 {
        content_end..run.lines.last()?.source_range.end
    } else {
        line_deletion_range(source, first)?
    };
    deletion_augmentation(source, deletion, current_byte)
}

fn delete_from_empty_paragraph(
    source: &str,
    document: &MarkdownDoc,
    run: &EditableParagraphRun,
    index: usize,
) -> Option<EditAugmentation> {
    if index < run.hidden_separator_count || is_container_exit_primitive(run) {
        return None;
    }
    let line = &run.lines[index];
    if line.newline_range.is_empty() {
        return None;
    }
    let deletion = container_start_deletion_range(source, run, line)
        .or_else(|| line_deletion_range(source, line))?;
    let next_cursor = run
        .lines
        .get(index + 1)
        .map(|next| next.source_byte)
        .or_else(|| text_start_on_line(&document.blocks, source, line.newline_range.end))
        .unwrap_or(line.newline_range.end);
    if let Some(augmentation) = cross_owner_deletion(source, document, run, next_cursor) {
        return Some(augmentation);
    }
    deletion_augmentation(source, deletion.clone(), next_cursor - deletion.len())
}

#[derive(Clone, Copy)]
pub(super) enum Boundary {
    Start,
    End,
}

pub(super) fn hidden_separator_range(
    source: &str,
    cursor: usize,
    boundary: Boundary,
) -> Option<Range<usize>> {
    let document = parse_structure(source);
    let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
    for run in paragraphs.runs() {
        if run.hidden_separator_count != run.lines.len() {
            continue;
        }
        let first = run.lines.first()?;
        let last = run.lines.last()?;
        let newline_width = super::newline_sequence_width_before(source, first.source_range.start)?;
        let range = first.source_range.start - newline_width..last.newline_range.end;
        let matches_boundary = match boundary {
            Boundary::Start => range.end == cursor,
            Boundary::End => range.start == cursor,
        };
        if matches_boundary {
            return Some(range);
        }
    }
    None
}

fn boundary_prefix_candidate(prefix: &str) -> bool {
    !prefix.chars().next_back().is_some_and(char::is_alphanumeric)
}

fn container_start_deletion_range(
    source: &str,
    run: &EditableParagraphRun,
    line: &crate::builder::EditableParagraphLine,
) -> Option<Range<usize>> {
    if run.owner_path.is_empty()
        || run.has_preceding_block
        || line.source_range.start != run.lines.first()?.source_range.start
        || line.source_byte == line.source_range.start
    {
        return None;
    }
    let continuation = super::canonical_container_prefix(source, line.source_byte);
    let next_start = line.newline_range.end;
    source
        .get(next_start..)?
        .starts_with(&continuation)
        .then_some(line.source_byte..next_start + continuation.len())
}

fn boundary_suffix_candidate(suffix: &str) -> bool {
    !suffix.chars().next().is_some_and(char::is_alphanumeric)
}

fn cross_owner_deletion(
    source: &str,
    document: &MarkdownDoc,
    run: &EditableParagraphRun,
    cursor: usize,
) -> Option<EditAugmentation> {
    if run.has_following_block || run.lines.len() - run.hidden_separator_count != 1 {
        return None;
    }
    let next_line_start = run.lines.last()?.newline_range.end;
    if next_line_start >= source.len() {
        return None;
    }
    let mut siblings = document.blocks.as_slice();
    let leaving_owner = run.owner_path.iter().find_map(|&index| {
        let owner = siblings.get(index)?;
        siblings = &owner.children;
        (owner.block_range.end <= next_line_start).then_some(owner)
    })?;
    let parent_prefix = super::canonical_container_prefix(source, leaving_owner.block_range.start);
    let newline = preferred_newline_sequence(source, next_line_start);
    let replacement = format!("{parent_prefix}{newline}");
    let range = run.lines.first()?.source_range.start..next_line_start;
    let cursor_byte_after = if cursor >= range.end {
        cursor - range.len() + replacement.len()
    } else if cursor >= range.start {
        range.start + replacement.len()
    } else {
        cursor
    };
    let augmentation = EditAugmentation {
        replace_range: Some(range),
        insert_text: Some(replacement),
        cursor_byte_after,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

pub(super) fn has_text_boundary(
    blocks: &[crate::builder::BlockNode],
    cursor: usize,
    boundary: Boundary,
) -> bool {
    blocks.iter().any(|block| {
        use crate::builder::BlockKind;
        if matches!(
            block.kind,
            BlockKind::CodeBlock { .. } | BlockKind::TableWrapper { .. } | BlockKind::MetadataBlock
        ) {
            return false;
        }
        let line = match boundary {
            Boundary::Start => block.projected_lines.first(),
            Boundary::End => block.projected_lines.last(),
        };
        let own_boundary = line.is_some_and(|line| {
            let mut visible_spans = line.spans.iter().filter(|span| !span.visual_range.is_empty());
            let (extent_boundary, visible_boundary) = match boundary {
                Boundary::Start => (
                    line.source_extent().start,
                    visible_spans.next().map(|span| span.source_range.start),
                ),
                Boundary::End => (
                    line.source_extent().end,
                    visible_spans.next_back().map(|span| span.source_range.end),
                ),
            };
            extent_boundary == cursor || visible_boundary == Some(cursor)
        });
        own_boundary || has_text_boundary(&block.children, cursor, boundary)
    })
}

fn text_start_on_line(
    blocks: &[crate::builder::BlockNode],
    source: &str,
    line_start: usize,
) -> Option<usize> {
    for block in blocks {
        if let Some(first) = block.projected_lines.first() {
            let anchor = first.source_extent().start;
            if super::locate_source_line_bounds(source, anchor)?.0 == line_start {
                return Some(anchor);
            }
        }
        if let Some(anchor) = text_start_on_line(&block.children, source, line_start) {
            return Some(anchor);
        }
    }
    None
}

fn deletion_augmentation(
    source: &str,
    range: Range<usize>,
    cursor: usize,
) -> Option<EditAugmentation> {
    let augmentation = EditAugmentation {
        replace_range: Some(range),
        insert_text: Some(String::new()),
        cursor_byte_after: cursor,
    };
    debug_assert_augmentation(&augmentation, source);
    Some(augmentation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_edit(
        source: &str,
        cursor: usize,
        expected: &str,
        expected_cursor: usize,
        operation: fn(&str, usize) -> Option<EditAugmentation>,
    ) {
        let edit = operation(source, cursor).expect("editable neighbor must produce a deletion");
        let mut edited = source.to_owned();
        edited.replace_range(
            edit.replace_range.expect("deletion has a range"),
            edit.insert_text.as_deref().expect("deletion has replacement text"),
        );
        assert_eq!(edited, expected, "source: {source:?}");
        assert_eq!(edit.cursor_byte_after, expected_cursor, "source: {source:?}");
        assert!(edited.is_char_boundary(edit.cursor_byte_after));
    }

    #[test]
    fn cross_owner_empty_paragraph_deletion_preserves_neutral_separator() {
        for newline in ["\n", "\r\n"] {
            for blanks in [1, 2] {
                let source = format!("> a{newline}{}b", format!(">{newline}").repeat(blanks));
                let expected = format!("> a{newline}{newline}b");
                assert_edit(&source, 3, &expected, 3, delete_forward);
                assert_edit(
                    &source,
                    source.len() - 1,
                    &expected,
                    expected.len() - 1,
                    backspace_boundary,
                );
                let empty_cursor = source.len() - 1 - newline.len();
                assert_edit(&source, empty_cursor, &expected, expected.len() - 1, delete_forward);
            }
        }
    }

    #[test]
    fn backspace_on_last_container_slot_keeps_following_block_outside() {
        assert_edit("> a\n>\nb", 5, "> a\n\nb", 3, backspace);
        assert_edit("> a\n>\n>\nb", 7, "> a\n\nb", 3, backspace);
    }

    #[test]
    fn cross_owner_deletion_preserves_parent_and_reduces_exactly_one_slot() {
        for (source, cursor, expected) in [
            ("> > a\n> >\n> b", 5, "> > a\n> \n> b"),
            ("- > a\n  >\n  b", 5, "- > a\n  \n  b"),
            ("> a\n>\n>\n>\nb", 3, "> a\n>\n>\nb"),
        ] {
            assert_edit(source, cursor, expected, cursor, delete_forward);
            let before_document = parse_structure(source);
            let after_document = parse_structure(expected);
            let count_slots = |document: &MarkdownDoc, text: &str| {
                EditableParagraphMap::from_blocks(&document.blocks, text)
                    .runs()
                    .iter()
                    .map(|run| run.lines.len() - run.hidden_separator_count)
                    .sum::<usize>()
            };
            assert_eq!(
                count_slots(&before_document, source),
                count_slots(&after_document, expected) + 1
            );
        }
    }

    #[test]
    fn removing_container_start_slot_keeps_its_opening_marker() {
        for (source, cursor, expected, expected_cursor) in [
            ("- \n  # Title", 7, "- # Title", 4),
            ("> - \n>   # Title", 11, "> - # Title", 6),
            ("- > \n  > # Title", 11, "- > # Title", 6),
        ] {
            assert_edit(source, cursor, expected, expected_cursor, backspace_boundary);
        }
    }

    #[test]
    fn styled_paragraph_boundaries_remove_adjacent_empty_paragraphs() {
        for (styled, start_offset, end_offset) in
            [("_words_", 1, 6), ("~~words~~", 2, 7), ("**words**", 2, 7)]
        {
            let source = format!("a\n\n\n{styled}");
            let expected = format!("a\n\n{styled}");
            assert_edit(&source, 4 + start_offset, &expected, 3 + start_offset, backspace_boundary);
            let source = format!("{styled}\n\n\nb");
            let expected = format!("{styled}\n\nb");
            assert_edit(&source, end_offset, &expected, end_offset, delete_forward);
        }
    }

    #[test]
    fn list_continuation_markers_are_not_empty_paragraphs() {
        for source in [
            "123. title\n     =",
            "123. para\n     ***",
            "123. > title\n     > =",
            "123. > para\n     > ***",
            "> 123. title\n>      =",
            "> 123. para\n>      ***",
            "- title\n\t=",
            "- para\n\t***",
            "> - title\n>\t=",
        ] {
            let cursor = source.find('\n').expect("fixture has continuation line");
            let document = parse_structure(source);
            let paragraphs = EditableParagraphMap::from_blocks(&document.blocks, source);
            assert!(
                delete_forward(source, cursor).is_none(),
                "source {source:?}; blocks {:#?}; map {paragraphs:#?}",
                document.blocks
            );
        }
    }

    #[test]
    fn neighbor_deletion_is_direction_symmetric() {
        for newline in ["\n", "\r\n"] {
            for preceding in ["a", "# a", "> a", "- a", "---"] {
                let source = format!("{preceding}{}b", newline.repeat(4));
                let expected = format!("{preceding}{}b", newline.repeat(3));
                assert_edit(
                    &source,
                    source.len() - 1,
                    &expected,
                    expected.len() - 1,
                    backspace_boundary,
                );
                if preceding != "---" {
                    assert_edit(
                        &source,
                        preceding.len(),
                        &expected,
                        preceding.len(),
                        delete_forward,
                    );
                }
            }
        }
    }

    #[test]
    fn heading_start_backspace_removes_preceding_paragraph() {
        for heading in ["# Title", "Title\n===", "> Title", "- Title"] {
            for prefix in ["\n", "head\n\n\n"] {
                let source = format!("{prefix}{heading}");
                let content = source.find("Title").expect("fixture includes title");
                let expected = format!("{}{heading}", &prefix[..prefix.len() - 1]);
                assert_edit(&source, content, &expected, content - 1, backspace_boundary);
            }
        }
    }

    #[test]
    fn delete_removes_one_empty_paragraph_or_the_only_eof_slot() {
        for (source, cursor, expected, expected_cursor) in [
            ("a\n", 1, "a", 1),
            ("a\n\n", 1, "a", 1),
            ("a\n\n\n", 1, "a\n\n", 1),
            ("a\n\n\nb", 3, "a\n\nb", 3),
            ("a\n\n\n---", 3, "a\n\n---", 3),
            ("a\n \n\t\nb", 1, "a\n \nb", 1),
            ("> first\n>\n>\n> second", 7, "> first\n>\n> second", 7),
            ("- first\n\n\n  second", 7, "- first\n\n  second", 7),
        ] {
            assert_edit(source, cursor, expected, expected_cursor, delete_forward);
        }
    }

    #[test]
    fn interior_keys_do_not_parse_the_document() {
        for source in ["word", "xxxx", "12345"] {
            super::super::CLASSIFY_PARSE_COUNT.with(|count| count.set(0));
            assert!(backspace_boundary(source, 2).is_none());
            assert!(delete_forward(source, 2).is_none());
            assert_eq!(super::super::CLASSIFY_PARSE_COUNT.with(|count| count.get()), 0);
        }
        let source = "head\n\n\nxxxx";
        super::super::CLASSIFY_PARSE_COUNT.with(|count| count.set(0));
        assert!(backspace_boundary(source, source.len() - 1).is_none());
        assert_eq!(super::super::CLASSIFY_PARSE_COUNT.with(|count| count.get()), 0);
    }

    #[test]
    fn containers_and_following_blocks_keep_their_content() {
        for (source, cursor, expected, expected_cursor) in [
            ("> first\n>\n>\n> second", 14, "> first\n>\n> second", 12),
            ("- first\n\n\n  second", 12, "- first\n\n  second", 11),
            ("a\r\n \r\n\t\r\nb", 9, "a\r\n \r\nb", 6),
        ] {
            assert_edit(source, cursor, expected, expected_cursor, backspace_boundary);
        }
        for following in ["# tail", "> tail", "- tail", "---", "```\ncode\n```", "| a |\n| - |"] {
            let source = format!("a\n\n\n{following}");
            let expected = format!("a\n\n{following}");
            assert_edit(&source, 1, &expected, 1, delete_forward);
        }
    }

    #[test]
    fn boundaries_without_editable_neighbors_keep_existing_guards() {
        for (source, cursor) in [("a\n\nb", 3), ("```\n\nb\n```", 5)] {
            assert!(backspace_boundary(source, cursor).is_none());
        }
        for (source, cursor) in [("a\n\nb", 1), ("a\n\n", 3), ("```\na\n\n```", 5)] {
            assert!(delete_forward(source, cursor).is_none());
        }
    }
}
