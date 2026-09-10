use crate::builder::BlockKind;
use crate::style::MarkdownStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockSpacing {
    Paragraph,
    Heading { level: u8 },
    CodeBlock,
    BlockQuote,
    ListItem { tight: bool, blank_line_before: bool },
    TableWrapper,
    HorizontalRule,
    MetadataBlock,
}

impl BlockSpacing {
    pub(crate) fn from_block_kind(kind: &BlockKind) -> Option<Self> {
        match kind {
            BlockKind::Container | BlockKind::TableRow_ | BlockKind::TableCell_ { .. } => None,
            BlockKind::Heading { level } => Some(Self::Heading { level: *level }),
            BlockKind::Paragraph => Some(Self::Paragraph),
            BlockKind::CodeBlock { .. } => Some(Self::CodeBlock),
            BlockKind::BlockQuote => Some(Self::BlockQuote),
            BlockKind::ListItem { tight, blank_line_before, .. } => {
                Some(Self::ListItem { tight: *tight, blank_line_before: *blank_line_before })
            }
            BlockKind::TableWrapper { .. } => Some(Self::TableWrapper),
            BlockKind::HorizontalRule => Some(Self::HorizontalRule),
            BlockKind::MetadataBlock => Some(Self::MetadataBlock),
        }
    }
}

pub(crate) fn resolve_block_gap(
    previous: BlockSpacing,
    next: BlockSpacing,
    style: &MarkdownStyle,
) -> f32 {
    match (previous, next) {
        (BlockSpacing::HorizontalRule, BlockSpacing::HorizontalRule) => style.rule_spacing,
        (BlockSpacing::HorizontalRule, BlockSpacing::Paragraph)
        | (BlockSpacing::Paragraph, BlockSpacing::HorizontalRule) => {
            style.rule_spacing.max(style.paragraph_spacing)
        }
        (BlockSpacing::Heading { .. }, BlockSpacing::HorizontalRule) => {
            style.heading_spacing_bottom.max(style.rule_spacing)
        }
        (BlockSpacing::HorizontalRule, BlockSpacing::Heading { level }) => {
            style.rule_spacing.max(heading_top_gap(level, style))
        }
        (BlockSpacing::ListItem { .. }, BlockSpacing::HorizontalRule)
        | (BlockSpacing::HorizontalRule, BlockSpacing::ListItem { .. }) => {
            style.list_group_spacing.max(style.rule_spacing)
        }
        (BlockSpacing::ListItem { .. }, BlockSpacing::ListItem { .. }) => style.list_item_spacing,
        (
            BlockSpacing::Paragraph,
            BlockSpacing::ListItem { tight: true, blank_line_before: false },
        ) => style.list_item_spacing,
        (BlockSpacing::Heading { .. }, BlockSpacing::Heading { .. }) => {
            style.heading_spacing_bottom
        }
        (BlockSpacing::ListItem { .. }, BlockSpacing::Heading { level }) => {
            style.list_group_spacing.max(heading_top_gap(level, style))
        }
        (BlockSpacing::ListItem { .. }, _) => style.list_group_spacing,
        (previous, BlockSpacing::Heading { level }) => {
            trailing_gap(previous, style).max(heading_top_gap(level, style))
        }
        (previous, BlockSpacing::HorizontalRule) => {
            trailing_gap(previous, style).max(style.rule_spacing)
        }
        (BlockSpacing::HorizontalRule, next) => style.rule_spacing.max(trailing_gap(next, style)),
        (previous, _) => trailing_gap(previous, style),
    }
}

pub(crate) fn document_start_gap(block: BlockSpacing, style: &MarkdownStyle) -> f32 {
    match block {
        BlockSpacing::Heading { level } => heading_top_gap(level, style) * 0.5,
        BlockSpacing::HorizontalRule => style.rule_spacing,
        _ => 0.0,
    }
}

pub(crate) fn document_end_gap(block: BlockSpacing, style: &MarkdownStyle) -> f32 {
    trailing_gap(block, style)
}

pub(crate) fn list_item_child_gap(child: BlockSpacing, style: &MarkdownStyle) -> f32 {
    match child {
        BlockSpacing::ListItem { .. } => 0.0,
        BlockSpacing::HorizontalRule => style.paragraph_spacing.max(style.rule_spacing),
        _ => style.paragraph_spacing,
    }
}

fn heading_top_gap(level: u8, style: &MarkdownStyle) -> f32 {
    style.heading_spacing_top * super::heading_spacing_scale(level)
}

fn trailing_gap(block: BlockSpacing, style: &MarkdownStyle) -> f32 {
    match block {
        BlockSpacing::Paragraph
        | BlockSpacing::CodeBlock
        | BlockSpacing::BlockQuote
        | BlockSpacing::TableWrapper
        | BlockSpacing::MetadataBlock => style.paragraph_spacing,
        BlockSpacing::Heading { .. } => style.heading_spacing_bottom,
        BlockSpacing::ListItem { .. } => style.list_item_spacing,
        BlockSpacing::HorizontalRule => style.rule_spacing,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BlockSpacing, document_end_gap, document_start_gap, list_item_child_gap, resolve_block_gap,
    };
    use crate::test_utils::default_style;

    #[test]
    fn horizontal_rule_uses_the_larger_demand_on_both_sides() {
        let mut style = default_style();
        style.paragraph_spacing = 30.0;
        style.rule_spacing = 12.0;

        assert_eq!(
            resolve_block_gap(BlockSpacing::Paragraph, BlockSpacing::HorizontalRule, &style),
            30.0
        );
        assert_eq!(
            resolve_block_gap(BlockSpacing::HorizontalRule, BlockSpacing::Paragraph, &style),
            30.0
        );
        assert_eq!(
            resolve_block_gap(BlockSpacing::HorizontalRule, BlockSpacing::HorizontalRule, &style,),
            12.0
        );
        assert_eq!(
            resolve_block_gap(BlockSpacing::HorizontalRule, BlockSpacing::CodeBlock, &style),
            30.0
        );
    }

    #[test]
    fn heading_and_horizontal_rule_resolve_level_aware_gaps() {
        let mut style = default_style();
        style.heading_spacing_top = 40.0;
        style.heading_spacing_bottom = 8.0;
        style.rule_spacing = 12.0;

        assert_eq!(
            resolve_block_gap(
                BlockSpacing::Heading { level: 1 },
                BlockSpacing::HorizontalRule,
                &style,
            ),
            12.0
        );
        assert_eq!(
            resolve_block_gap(
                BlockSpacing::HorizontalRule,
                BlockSpacing::Heading { level: 2 },
                &style,
            ),
            32.0
        );
    }

    #[test]
    fn list_group_and_horizontal_rule_use_group_spacing() {
        let mut style = default_style();
        style.list_item_spacing = 4.0;
        style.list_group_spacing = 20.0;
        style.rule_spacing = 12.0;
        let list_item = BlockSpacing::ListItem { tight: true, blank_line_before: false };

        assert_eq!(resolve_block_gap(list_item, BlockSpacing::HorizontalRule, &style), 20.0);
        assert_eq!(resolve_block_gap(BlockSpacing::HorizontalRule, list_item, &style), 20.0);
    }

    #[test]
    fn existing_non_rule_spacing_exceptions_are_preserved() {
        let mut style = default_style();
        style.paragraph_spacing = 16.0;
        style.heading_spacing_top = 40.0;
        style.heading_spacing_bottom = 8.0;
        style.list_item_spacing = 4.0;

        assert_eq!(
            resolve_block_gap(
                BlockSpacing::Heading { level: 1 },
                BlockSpacing::Heading { level: 2 },
                &style,
            ),
            8.0
        );
        assert_eq!(
            resolve_block_gap(
                BlockSpacing::Paragraph,
                BlockSpacing::ListItem { tight: true, blank_line_before: false },
                &style,
            ),
            4.0
        );
        assert_eq!(
            resolve_block_gap(
                BlockSpacing::Paragraph,
                BlockSpacing::ListItem { tight: false, blank_line_before: true },
                &style,
            ),
            16.0
        );
    }

    #[test]
    fn document_edges_are_explicit() {
        let mut style = default_style();
        style.heading_spacing_top = 40.0;
        style.heading_spacing_bottom = 8.0;
        style.rule_spacing = 12.0;
        style.paragraph_spacing = 16.0;

        assert_eq!(document_start_gap(BlockSpacing::Heading { level: 1 }, &style), 20.0);
        assert_eq!(document_start_gap(BlockSpacing::HorizontalRule, &style), 12.0);
        assert_eq!(document_start_gap(BlockSpacing::Paragraph, &style), 0.0);
        assert_eq!(document_end_gap(BlockSpacing::Heading { level: 1 }, &style), 8.0);
        assert_eq!(document_end_gap(BlockSpacing::HorizontalRule, &style), 12.0);
        assert_eq!(document_end_gap(BlockSpacing::Paragraph, &style), 16.0);
    }

    #[test]
    fn list_item_content_uses_child_semantics_for_its_internal_boundary() {
        let mut style = default_style();
        style.paragraph_spacing = 8.0;
        style.rule_spacing = 20.0;

        assert_eq!(list_item_child_gap(BlockSpacing::HorizontalRule, &style), 20.0);
        assert_eq!(
            list_item_child_gap(
                BlockSpacing::ListItem { tight: true, blank_line_before: false },
                &style,
            ),
            0.0
        );
        assert_eq!(list_item_child_gap(BlockSpacing::Paragraph, &style), 8.0);
    }
}
