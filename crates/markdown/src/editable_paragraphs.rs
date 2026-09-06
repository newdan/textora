//! Editor-only paragraphs have real layout geometry and zero-width source anchors.

use super::{BlockKind, BlockNode, BlockSource};
use crate::projection::ProjectedText;
use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Clone, Debug)]
pub(crate) struct EditableParagraphMap {
    runs: Vec<EditableParagraphRun>,
}

#[derive(Clone, Debug)]
pub(crate) struct EditableParagraphRun {
    pub(crate) owner_path: Vec<usize>,
    pub(crate) continuation_source_start: Option<usize>,
    pub(crate) lines: Vec<EditableParagraphLine>,
    pub(crate) hidden_separator_count: usize,
    pub(crate) has_preceding_block: bool,
    pub(crate) has_following_block: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct EditableParagraphLine {
    pub(crate) source_byte: usize,
    pub(crate) source_range: Range<usize>,
    pub(crate) newline_range: Range<usize>,
}

impl EditableParagraphMap {
    pub(crate) fn from_blocks(blocks: &[BlockNode], source: &str) -> Self {
        let candidates = collect_candidates(blocks, source);
        let mut runs = Vec::new();
        let mut run_start = 0;
        while run_start < candidates.len() {
            let first = &candidates[run_start];
            let mut run_end = run_start + 1;
            while run_end < candidates.len()
                && candidates[run_end].owner == first.owner
                && candidates[run_end].line_index == candidates[run_end - 1].line_index + 1
            {
                run_end += 1;
            }
            runs.push(describe_run(blocks, &candidates[run_start..run_end]));
            run_start = run_end;
        }
        Self { runs }
    }

    pub(crate) fn runs(&self) -> &[EditableParagraphRun] {
        &self.runs
    }

    pub(crate) fn run_at_byte(&self, byte: usize) -> Option<&EditableParagraphRun> {
        self.runs.iter().find(|run| {
            run.lines
                .iter()
                .any(|line| line.source_range.start <= byte && byte <= line.source_range.end)
        })
    }
}

struct EmptySourceLine {
    line_index: usize,
    line_start: usize,
    line_end: usize,
    newline_end: usize,
    source_byte: usize,
    quote_depth: usize,
    owner: Vec<usize>,
}

pub(super) fn insert(blocks: &mut Vec<BlockNode>, source: &str) {
    let paragraphs = EditableParagraphMap::from_blocks(blocks, source);
    let mut anchors_by_owner = BTreeMap::<Vec<usize>, Vec<usize>>::new();
    for run in paragraphs.runs() {
        anchors_by_owner
            .entry(run.owner_path.clone())
            .or_default()
            .extend(run.lines[run.hidden_separator_count..].iter().map(|line| line.source_byte));
    }
    insert_owned_anchors(blocks, &mut Vec::new(), &mut anchors_by_owner);
}

fn collect_candidates(blocks: &[BlockNode], source: &str) -> Vec<EmptySourceLine> {
    let mut candidates = Vec::new();
    let mut line_start = 0;
    for (line_index, physical_line) in source.split('\n').enumerate() {
        let content = physical_line.trim_end_matches('\r');
        if let Some((content_offset, quote_depth)) = empty_content_offset(content) {
            let mut line = EmptySourceLine {
                line_index,
                line_start,
                line_end: line_start + content.len(),
                newline_end: (line_start + physical_line.len() + 1).min(source.len()),
                source_byte: line_start + content_offset,
                quote_depth,
                owner: Vec::new(),
            };
            let terminal_code_content = line.source_byte == source.len()
                && blocks.last().is_some_and(|block| owns_terminal_code_line(block, source));
            if !terminal_code_content && resolve_owner(blocks, &mut line, 0) {
                candidates.push(line);
            }
        }
        line_start += physical_line.len() + 1;
    }
    candidates
}

fn owns_terminal_code_line(block: &BlockNode, source: &str) -> bool {
    if block.block_range.end != source.len() {
        return false;
    }
    if !matches!(block.kind, BlockKind::CodeBlock { .. }) {
        return block.children.last().is_some_and(|child| owns_terminal_code_line(child, source));
    }
    // Parser-owned content reaches the end of an unclosed block. A closing fence
    // remains outside that projection, so no second Markdown fence parser is needed.
    let content_end =
        block.projected_lines.last().map(|projected| projected.source_extent().end).or_else(|| {
            source[block.block_range.clone()]
                .find('\n')
                .map(|newline| block.block_range.start + newline + 1)
        });
    content_end.is_none_or(|content_end| source[content_end..].trim().is_empty())
}

fn empty_content_offset(content: &str) -> Option<(usize, usize)> {
    if content.chars().all(char::is_whitespace) {
        return Some((0, 0));
    }
    let mut remaining = content.trim_start_matches([' ', '\t']);
    let mut quote_depth = 0;
    while let Some(after_marker) = remaining.strip_prefix('>') {
        quote_depth += 1;
        remaining = after_marker.trim_start_matches([' ', '\t']);
    }
    (remaining.is_empty() && quote_depth > 0).then_some((content.len(), quote_depth))
}

fn resolve_owner(blocks: &[BlockNode], line: &mut EmptySourceLine, quote_depth: usize) -> bool {
    let insertion = blocks.partition_point(|block| block.block_range.start <= line.source_byte);
    let Some(block_index) = insertion.checked_sub(1) else {
        return line.quote_depth == quote_depth;
    };
    let block = &blocks[block_index];
    let owns_line = line.source_byte < block.block_range.end
        || (line.line_start < block.block_range.end && line.source_byte == block.block_range.end);
    if !owns_line {
        return line.quote_depth == quote_depth;
    }
    if !matches!(
        block.kind,
        BlockKind::Container | BlockKind::BlockQuote | BlockKind::ListItem { .. }
    ) {
        return false;
    }
    // Container ranges can include the separator after their final content.
    // Only an explicit quote prefix keeps such a trailing line inside the container.
    if line.quote_depth == quote_depth && content_end(block) <= line.source_byte {
        return true;
    }
    if block.projected_lines.iter().any(|projected| {
        let extent = projected.source_extent();
        extent.start <= line.source_byte && line.source_byte <= extent.end
    }) {
        return false;
    }
    line.owner.push(block_index);
    let child_quote_depth = quote_depth + usize::from(matches!(block.kind, BlockKind::BlockQuote));
    resolve_owner(&block.children, line, child_quote_depth)
}

fn content_end(block: &BlockNode) -> usize {
    let own_end = block.projected_lines.iter().map(|projected| projected.source_extent().end).max();
    let child_end = block.children.last().map(content_end);
    own_end.into_iter().chain(child_end).max().unwrap_or(block.block_range.end)
}

fn describe_run(blocks: &[BlockNode], run: &[EmptySourceLine]) -> EditableParagraphRun {
    let first = &run[0];
    let last = &run[run.len() - 1];
    let mut siblings = blocks;
    let mut owner_text_precedes = false;
    let mut continuation_source_start = None;
    for &owner_index in &first.owner {
        let owner = &siblings[owner_index];
        owner_text_precedes = owner.projected_lines.iter().any(|projected| {
            !projected.text.is_empty() && projected.source_extent().end <= first.source_byte
        });
        continuation_source_start = owner
            .projected_lines
            .first()
            .map(|projected| projected.source_extent().start)
            .or_else(|| owner.children.first().map(|child| child.block_range.start))
            .or(Some(first.source_byte));
        siblings = &owner.children;
    }
    let preceding_count =
        siblings.partition_point(|block| block.block_range.start < first.source_byte);
    let preceding_block = owner_text_precedes || preceding_count > 0;
    let following_block =
        siblings.last().is_some_and(|block| block.block_range.start > last.source_byte);
    EditableParagraphRun {
        owner_path: first.owner.clone(),
        continuation_source_start,
        lines: run
            .iter()
            .map(|line| EditableParagraphLine {
                source_byte: line.source_byte,
                source_range: line.line_start..line.line_end,
                newline_range: line.line_end..line.newline_end,
            })
            .collect(),
        hidden_separator_count: usize::from(preceding_block && (following_block || run.len() > 1)),
        has_preceding_block: preceding_block,
        has_following_block: following_block,
    }
}

fn insert_owned_anchors(
    blocks: &mut Vec<BlockNode>,
    owner: &mut Vec<usize>,
    anchors_by_owner: &mut BTreeMap<Vec<usize>, Vec<usize>>,
) {
    for (block_index, block) in blocks.iter_mut().enumerate() {
        owner.push(block_index);
        insert_owned_anchors(&mut block.children, owner, anchors_by_owner);
        owner.pop();
    }
    if let Some(anchors) = anchors_by_owner.remove(owner) {
        blocks.extend(anchors.into_iter().map(empty_paragraph));
        blocks.sort_by_key(|block| block.block_range.start);
    }
}

fn empty_paragraph(source_byte: usize) -> BlockNode {
    BlockNode {
        kind: BlockKind::Paragraph,
        children: Vec::new(),
        source_range: BlockSource::Continuous(source_byte..source_byte),
        text_styles: vec![Vec::new()],
        text_lines: vec![String::new()],
        projected_lines: vec![ProjectedText::direct("", source_byte)],
        block_range: source_byte..source_byte,
        code_line_source_starts: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{BlockKind, BlockNode, MarkdownDoc};
    use crate::parser::parse_markdown;
    use crate::test_utils::default_style;

    fn build(source: &str) -> MarkdownDoc {
        MarkdownDoc::build_for_editing(&parse_markdown(source), &default_style(), source)
    }

    #[test]
    fn editable_paragraph_map_preserves_space_lines_and_crlf_boundaries() {
        let source = "head\r\n \r\n \r\ntail";
        let document = MarkdownDoc::build(&parse_markdown(source), &default_style());
        let paragraphs = super::EditableParagraphMap::from_blocks(&document.blocks, source);
        assert_eq!(paragraphs.runs().len(), 1);
        let run = paragraphs.run_at_byte(9).expect("second blank line must belong to a run");
        assert!(run.owner_path.is_empty());
        assert!(run.continuation_source_start.is_none());
        assert_eq!(run.hidden_separator_count, 1);
        assert!(run.has_preceding_block);
        assert!(run.has_following_block);
        assert_eq!(run.lines[1].source_byte, 9);
        assert_eq!(run.lines[1].source_range, 9..10);
        assert_eq!(run.lines[1].newline_range, 10..12);
    }

    #[test]
    fn editable_paragraph_map_exposes_actual_container_content_start() {
        for (source, anchor, content_start) in
            [("- first\n\n\n  second", 9, 2), ("> first\n>\n>\n> second", 11, 2)]
        {
            let document = MarkdownDoc::build(&parse_markdown(source), &default_style());
            let paragraphs = super::EditableParagraphMap::from_blocks(&document.blocks, source);
            let run = paragraphs.run_at_byte(anchor).expect("container slot must be represented");
            assert_eq!(run.owner_path, [0]);
            assert_eq!(run.continuation_source_start, Some(content_start));
            assert_eq!(run.hidden_separator_count, 1);
            assert!(run.has_preceding_block && run.has_following_block);
        }
    }

    fn empty_anchors(blocks: &[BlockNode]) -> Vec<usize> {
        blocks
            .iter()
            .flat_map(|block| {
                let own = block.is_editable_empty_paragraph().then_some(block.block_range.start);
                own.into_iter().chain(empty_anchors(&block.children))
            })
            .collect()
    }

    #[test]
    fn editable_paragraphs_inter_block_runs_keep_one_hidden_separator() {
        for newline in ["\n", "\r\n"] {
            for follower in ["tail", "---", "> tail", "- tail"] {
                let source = format!("head{newline}{newline}{newline}{follower}");
                let document = build(&source);
                let anchor = "head".len() + newline.len() * 2;
                assert_eq!(empty_anchors(&document.blocks), [anchor], "{source:?}: {document:#?}");
                assert!(document.blocks.iter().any(|block| block.block_range == (anchor..anchor)));
            }
        }
    }

    #[test]
    fn editable_paragraphs_document_edges_have_explicit_anchors() {
        for (source, expected) in [
            ("", vec![0]),
            ("\n", vec![0, 1]),
            ("\n\n", vec![0, 1, 2]),
            ("\n\nhead", vec![0, 1]),
            ("head\n", vec![5]),
            ("head\n\n", vec![6]),
            ("head\n\n\n", vec![6, 7]),
            ("head\r\n\r\n", vec![8]),
        ] {
            let document = build(source);
            assert_eq!(empty_anchors(&document.blocks), expected, "{source:?}: {document:#?}");
        }
    }

    #[test]
    fn editable_paragraphs_loose_list_owns_internal_empty_paragraph() {
        let source = "- first\n\n\n  second";
        let document = build(source);
        assert_eq!(empty_anchors(&document.blocks), [9], "{document:#?}");
        let item = &document.blocks[0];
        assert!(matches!(item.kind, BlockKind::ListItem { .. }));
        assert_eq!(empty_anchors(&item.children), [9]);
    }

    #[test]
    fn editable_paragraphs_list_trailing_syntax_gap_belongs_to_parent() {
        let source = "- item\n\n\n后段";
        let document = build(source);
        let anchor = "- item\n\n".len();
        assert_eq!(empty_anchors(&document.blocks), [anchor], "{document:#?}");
        assert!(document.blocks[1].is_editable_empty_paragraph(), "{document:#?}");
        assert!(empty_anchors(&document.blocks[0].children).is_empty());
        let typed = build("- item\n\n新\n\n后段");
        assert!(empty_anchors(&typed.blocks).is_empty(), "{typed:#?}");
    }

    #[test]
    fn editable_paragraphs_quote_markers_keep_their_container() {
        let source = "> first\n>\n>\n> second";
        let document = build(source);
        assert_eq!(empty_anchors(&document.blocks), [11], "{document:#?}");
        assert_eq!(empty_anchors(&document.blocks[0].children), [11]);
    }

    #[test]
    fn editable_paragraphs_existing_block_content_is_not_duplicated() {
        for source in ["```\n\n\n```", "---\na: b\n\n\n---", "| a |\n| - |\n|   |", "- "] {
            let document = build(source);
            assert!(empty_anchors(&document.blocks).is_empty(), "{source:?}: {document:#?}");
        }
    }

    #[test]
    fn editable_paragraphs_unclosed_code_keeps_its_terminal_content_line() {
        for source in ["```\n", "```\n\n", "```\ncode\n\n", "> ```\n> code\n", "```\n    ```\n"] {
            let document = build(source);
            assert!(empty_anchors(&document.blocks).is_empty(), "{source:?}: {document:#?}");
        }
        let closed = "```\n```\n";
        assert_eq!(empty_anchors(&build(closed).blocks), [closed.len()]);
    }

    #[test]
    fn editable_paragraphs_outer_gap_is_not_consumed_by_nested_quotes() {
        let source = "head\n\n\n> > tail";
        let document = build(source);
        assert_eq!(empty_anchors(&document.blocks), [6]);
        assert!(document.blocks[1].is_editable_empty_paragraph());
        assert!(empty_anchors(&document.blocks[2].children).is_empty());
    }

    #[test]
    fn editable_paragraphs_nested_quote_has_one_owner() {
        let source = "> > first\n> >\n> >\n> > second";
        let document = build(source);
        let expected_anchor =
            source.find("\n> >\n> > second").expect("fixture contains the second blank quote line")
                + "\n> >".len();
        assert_eq!(empty_anchors(&document.blocks), [expected_anchor], "{document:#?}");
        let inner_quote = &document.blocks[0].children[0];
        assert!(matches!(inner_quote.kind, BlockKind::BlockQuote));
        assert_eq!(empty_anchors(&inner_quote.children), [expected_anchor]);
    }

    #[test]
    fn editable_paragraphs_zero_width_projection_preserves_source_anchor() {
        let source = "head\n\n\n---";
        let parsed = parse_markdown(source);
        let preview = MarkdownDoc::build(&parsed, &default_style());
        assert!(empty_anchors(&preview.blocks).is_empty());
        let document = build(source);
        let paragraph = &document.blocks[1];
        assert!(paragraph.is_editable_empty_paragraph());
        assert_eq!(paragraph.line_count(), 1);
        assert_eq!(paragraph.text_styles.len(), 1);
        let projected = &paragraph.projected_lines[0];
        assert!(projected.text.is_empty());
        assert_eq!(projected.boundaries.len(), 1);
        assert_eq!(projected.boundaries[0].byte, 6);
        assert_eq!(projected.source_extent(), 6..6);
    }
}
