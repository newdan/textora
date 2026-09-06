//! Preserve literal paragraph whitespace without enabling Markdown code indentation.

use crate::builder::{BlockKind, EditableParagraphMap, MarkdownDoc};
use crate::parser::parse_markdown;
use ui::plugin::EditAugmentation;

const LITERAL_PARAGRAPH_SPACE: char = '\u{a0}';

pub(super) fn insert_selected_text(
    source: &str,
    selection: std::ops::Range<usize>,
    text: &str,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = super::locate_source_line_bounds(source, selection.start)?;
    let prefix = source.get(line_start..selection.start)?;
    let suffix = source.get(selection.end..)?.split(['\r', '\n']).next().unwrap_or_default();
    if !requires_space_protection(prefix, text, suffix) {
        return None;
    }
    let content_end = super::source_line_content_end(source, line_end);
    if !is_plain_paragraph_line(source, selection.start, line_start, content_end) {
        return None;
    }
    let mut source_after_delete = source.to_owned();
    source_after_delete.replace_range(selection.clone(), "");
    insert_text(&source_after_delete, selection.start, text)
}

pub(super) fn insert_text(
    source: &str,
    current_byte: usize,
    text: &str,
) -> Option<EditAugmentation> {
    let (line_start, _, line_end) = super::locate_source_line_bounds(source, current_byte)?;
    let content_end = super::source_line_content_end(source, line_end);
    let prefix = source.get(line_start..current_byte)?;
    let suffix = source.get(current_byte..content_end)?;
    if !requires_space_protection(prefix, text, suffix) {
        return None;
    }
    if !is_plain_paragraph_line(source, current_byte, line_start, content_end) {
        return None;
    }

    let mut insertion = protect_spaces(prefix);
    let leading_spaces = text.len() - text.trim_start_matches(' ').len();
    insertion.push_str(&protect_spaces(&text[..leading_spaces]));
    insertion.push_str(&text[leading_spaces..]);
    if suffix.chars().all(is_paragraph_space) {
        return Some(insert_into_blank_line(source, current_byte, line_start, suffix, insertion));
    }
    Some(EditAugmentation {
        cursor_byte_after: line_start + insertion.len(),
        replace_range: Some(line_start..current_byte),
        insert_text: Some(insertion),
    })
}

fn requires_space_protection(prefix: &str, text: &str, suffix: &str) -> bool {
    if text.is_empty()
        || text.contains(['\r', '\n', '\t'])
        || !prefix.chars().all(is_paragraph_space)
    {
        return false;
    }
    prefix.contains(' ')
        || text.starts_with(' ')
        || (suffix.contains(' ') && suffix.chars().all(is_paragraph_space))
}

fn insert_into_blank_line(
    source: &str,
    current_byte: usize,
    line_start: usize,
    suffix: &str,
    mut insertion: String,
) -> EditAugmentation {
    let protected_suffix = protect_spaces(suffix);
    let cursor_byte_after = line_start + insertion.len();
    insertion.push_str(&protected_suffix);
    if let Some(mut augmentation) =
        super::editable_paragraph_edit::insert_text(source, current_byte, &insertion)
    {
        augmentation.cursor_byte_after -= protected_suffix.len();
        return augmentation;
    }
    EditAugmentation {
        cursor_byte_after,
        replace_range: Some(line_start..current_byte + suffix.len()),
        insert_text: Some(insertion),
    }
}

fn is_plain_paragraph_line(
    source: &str,
    current_byte: usize,
    line_start: usize,
    content_end: usize,
) -> bool {
    #[cfg(test)]
    super::CLASSIFY_PARSE_COUNT.with(|count| count.set(count.get() + 1));
    let document = MarkdownDoc::build_structure(&parse_markdown(source));
    if document.blocks.iter().any(|block| {
        matches!(block.kind, BlockKind::Paragraph)
            && block.block_range.start <= content_end
            && block.block_range.end > line_start
    }) {
        return true;
    }
    EditableParagraphMap::from_blocks(&document.blocks, source)
        .run_at_byte(current_byte)
        .is_some_and(|run| run.owner_path.is_empty())
}

fn is_paragraph_space(character: char) -> bool {
    character == ' ' || character == LITERAL_PARAGRAPH_SPACE
}

fn protect_spaces(text: &str) -> String {
    text.chars()
        .map(|character| if character == ' ' { LITERAL_PARAGRAPH_SPACE } else { character })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_selection_replacement_does_not_parse_the_document() {
        for (source, selection, text) in [
            ("普通段落", 0.."普".len(), "新"),
            ("普通段落", "普通".len().."普通段".len(), "新"),
            ("普通段落", 0.."普".len(), "多\n行"),
        ] {
            super::super::CLASSIFY_PARSE_COUNT.with(|count| count.set(0));
            assert!(super::insert_selected_text(source, selection, text).is_none());
            super::super::CLASSIFY_PARSE_COUNT.with(|count| assert_eq!(count.get(), 0));
        }
    }
}
