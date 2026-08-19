use crate::builder::BlockNode;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlockReconcilePlan {
    pub common_prefix_len: usize,
    pub old_changed: Range<usize>,
    pub new_changed: Range<usize>,
    pub common_suffix_len: usize,
}

impl BlockReconcilePlan {
    pub(crate) fn between(
        old_blocks: &[BlockNode],
        old_source: &str,
        new_blocks: &[BlockNode],
        new_source: &str,
    ) -> Self {
        let comparable_len = old_blocks.len().min(new_blocks.len());
        let common_prefix_len = (0..comparable_len)
            .take_while(|&block_index| {
                block_identity_matches(
                    &old_blocks[block_index],
                    old_source,
                    &new_blocks[block_index],
                    new_source,
                )
            })
            .count();

        let remaining_old_len = old_blocks.len() - common_prefix_len;
        let remaining_new_len = new_blocks.len() - common_prefix_len;
        let comparable_suffix_len = remaining_old_len.min(remaining_new_len);
        let common_suffix_len = (0..comparable_suffix_len)
            .take_while(|suffix_offset| {
                let old_index = old_blocks.len() - suffix_offset - 1;
                let new_index = new_blocks.len() - suffix_offset - 1;
                block_identity_matches(
                    &old_blocks[old_index],
                    old_source,
                    &new_blocks[new_index],
                    new_source,
                )
            })
            .count();

        Self {
            common_prefix_len,
            old_changed: common_prefix_len..old_blocks.len() - common_suffix_len,
            new_changed: common_prefix_len..new_blocks.len() - common_suffix_len,
            common_suffix_len,
        }
    }

    pub(crate) fn old_index_for_unchanged_new_block(&self, new_index: usize) -> Option<usize> {
        if new_index < self.common_prefix_len {
            return Some(new_index);
        }
        if self.new_changed.contains(&new_index) {
            return None;
        }

        let suffix_offset = new_index.checked_sub(self.new_changed.end)?;
        Some(self.old_changed.end + suffix_offset)
    }
}

fn block_identity_matches(
    old_block: &BlockNode,
    old_source: &str,
    new_block: &BlockNode,
    new_source: &str,
) -> bool {
    old_block.kind == new_block.kind
        && source_slice(old_source, &old_block.block_range)
            == source_slice(new_source, &new_block.block_range)
}

fn source_slice<'a>(source: &'a str, source_range: &Range<usize>) -> Option<&'a str> {
    source.get(source_range.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::MarkdownDoc;
    use crate::parser::parse_markdown;
    use crate::test_utils::default_style;

    fn build_document(source: &str) -> MarkdownDoc {
        MarkdownDoc::build(&parse_markdown(source), &default_style())
    }

    #[test]
    fn paragraph_edit_preserves_surrounding_blocks() {
        let old_source = "# Title\n\nalpha\n\nomega";
        let new_source = "# Title\n\nalpha changed\n\nomega";
        let old_document = build_document(old_source);
        let new_document = build_document(new_source);

        let plan = BlockReconcilePlan::between(
            &old_document.blocks,
            old_source,
            &new_document.blocks,
            new_source,
        );

        assert_eq!(plan.common_prefix_len, 1);
        assert_eq!(plan.old_changed, 1..2);
        assert_eq!(plan.new_changed, 1..2);
        assert_eq!(plan.common_suffix_len, 1);
    }

    #[test]
    fn inserted_block_matches_suffix_after_index_shift() {
        let old_source = "first\n\nlast";
        let new_source = "first\n\ninserted\n\nlast";
        let old_document = build_document(old_source);
        let new_document = build_document(new_source);

        let plan = BlockReconcilePlan::between(
            &old_document.blocks,
            old_source,
            &new_document.blocks,
            new_source,
        );

        assert_eq!(plan.common_prefix_len, 1);
        assert_eq!(plan.old_changed, 1..1);
        assert_eq!(plan.new_changed, 1..2);
        assert_eq!(plan.common_suffix_len, 1);
        assert_eq!(plan.old_index_for_unchanged_new_block(0), Some(0));
        assert_eq!(plan.old_index_for_unchanged_new_block(1), None);
        assert_eq!(plan.old_index_for_unchanged_new_block(2), Some(1));
    }

    #[test]
    fn unchanged_document_does_not_overlap_prefix_and_suffix() {
        let source = "first\n\nsecond\n\nthird";
        let old_document = build_document(source);
        let new_document = build_document(source);

        let plan =
            BlockReconcilePlan::between(&old_document.blocks, source, &new_document.blocks, source);

        assert_eq!(plan.common_prefix_len, old_document.blocks.len());
        assert_eq!(plan.old_changed, old_document.blocks.len()..old_document.blocks.len());
        assert_eq!(plan.new_changed, new_document.blocks.len()..new_document.blocks.len());
        assert_eq!(plan.common_suffix_len, 0);
    }

    #[test]
    fn source_text_match_requires_the_same_block_kind() {
        let old_source = "paragraph";
        let new_source = "# paragraph";
        let old_document = build_document(old_source);
        let new_document = build_document(new_source);

        let plan = BlockReconcilePlan::between(
            &old_document.blocks,
            old_source,
            &new_document.blocks,
            new_source,
        );

        assert_eq!(plan.common_prefix_len, 0);
        assert_eq!(plan.common_suffix_len, 0);
    }
}
