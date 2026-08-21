use crate::core::Rect;

use super::{TreeRowEditorInput, TreeRowInput};

pub const TREE_ROW_HEIGHT_LOGICAL: f32 = 28.0;
pub const TREE_ROW_FONT_SIZE_LOGICAL: f32 = 13.0;
pub const TREE_ROW_HORIZONTAL_PADDING_LOGICAL: f32 = 8.0;
pub const TREE_ROW_INDENT_LOGICAL: f32 = 16.0;
pub const TREE_EXPANDER_SIZE_LOGICAL: f32 = 14.0;
pub const TREE_ICON_SIZE_LOGICAL: f32 = 16.0;
pub const TREE_ICON_GAP_LOGICAL: f32 = 6.0;
pub const TREE_BADGE_HORIZONTAL_PADDING_LOGICAL: f32 = 6.0;
pub const TREE_BADGE_MINIMUM_WIDTH_LOGICAL: f32 = 20.0;
pub const TREE_BADGE_DIGIT_WIDTH_RATIO: f32 = 0.65;
pub const TREE_ACTION_SIZE_LOGICAL: f32 = 22.0;
pub const TREE_ACTION_ICON_SIZE_LOGICAL: f32 = 14.0;
pub const TREE_ACTION_GAP_LOGICAL: f32 = 2.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeRowLayout {
    pub row_rect: Rect,
    pub expander_rect: Rect,
    pub icon_rect: Option<Rect>,
    pub label_rect: Rect,
    pub badge_rect: Option<Rect>,
    pub action_rects: Vec<Rect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeListLayout {
    pub rows: Vec<TreeRowLayout>,
    pub editor: Option<TreeRowEditorLayout>,
    pub content_height_px: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeRowEditorLayout {
    pub row_rect: Rect,
    pub text_box_rect: Rect,
}

pub(super) fn build_tree_layout(
    rows: &[TreeRowInput],
    editor: Option<&TreeRowEditorInput>,
    rect: Rect,
    scroll_offset_px: f32,
    dpi: f32,
) -> TreeListLayout {
    let row_height = TREE_ROW_HEIGHT_LOGICAL * dpi;
    let horizontal_padding = TREE_ROW_HORIZONTAL_PADDING_LOGICAL * dpi;
    let expander_size = TREE_EXPANDER_SIZE_LOGICAL * dpi;
    let icon_size = TREE_ICON_SIZE_LOGICAL * dpi;
    let icon_gap = TREE_ICON_GAP_LOGICAL * dpi;
    let badge_height = (TREE_ROW_FONT_SIZE_LOGICAL + 6.0) * dpi;
    let badge_padding = TREE_BADGE_HORIZONTAL_PADDING_LOGICAL * dpi;
    let requested_action_size = TREE_ACTION_SIZE_LOGICAL * dpi;
    let action_gap = TREE_ACTION_GAP_LOGICAL * dpi;
    let editor_insert_index = editor.and_then(|editor| {
        rows.iter().position(|row| row.key == editor.parent_key).map(|index| index + 1)
    });

    let row_layouts = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let visual_index = index
                + usize::from(
                    editor_insert_index.is_some_and(|insert_index| index >= insert_index),
                );
            let row_rect = Rect::new(
                rect.x,
                rect.y + visual_index as f32 * row_height - scroll_offset_px,
                rect.w,
                row_height,
            );
            let mut cursor_x =
                rect.x + horizontal_padding + row.depth as f32 * TREE_ROW_INDENT_LOGICAL * dpi;
            let expander_rect = Rect::new(
                cursor_x,
                row_rect.y + (row_height - expander_size) * 0.5,
                expander_size,
                expander_size,
            );
            cursor_x += expander_size + icon_gap;

            let icon_rect = row.icon.as_ref().map(|_| {
                let icon_rect = Rect::new(
                    cursor_x,
                    row_rect.y + (row_height - icon_size) * 0.5,
                    icon_size,
                    icon_size,
                );
                cursor_x += icon_size + icon_gap;
                icon_rect
            });

            let action_count = row.trailing_actions.len();
            let available_action_width = (rect.w - horizontal_padding * 2.0).max(0.0);
            let action_size = if action_count == 0 {
                0.0
            } else {
                let gaps_width = action_gap * action_count.saturating_sub(1) as f32;
                requested_action_size.min(
                    ((available_action_width - gaps_width).max(0.0) / action_count as f32).max(0.0),
                )
            };
            let actions_width = action_size * action_count as f32
                + action_gap * action_count.saturating_sub(1) as f32;
            let actions_left = rect.right() - horizontal_padding - actions_width;
            let action_rects = (0..action_count)
                .map(|action_index| {
                    Rect::new(
                        actions_left + action_index as f32 * (action_size + action_gap),
                        row_rect.y + (row_height - action_size) * 0.5,
                        action_size,
                        action_size,
                    )
                })
                .collect::<Vec<_>>();
            let trailing_content_left = if action_count == 0 {
                rect.right() - horizontal_padding
            } else {
                actions_left - action_gap
            };

            let badge_rect = row.badge.map(|badge| {
                let requested_badge_width = (badge.to_string().len() as f32
                    * TREE_ROW_FONT_SIZE_LOGICAL
                    * TREE_BADGE_DIGIT_WIDTH_RATIO
                    * dpi
                    + badge_padding * 2.0)
                    .max(TREE_BADGE_MINIMUM_WIDTH_LOGICAL * dpi);
                let available_badge_width = (trailing_content_left - cursor_x).max(0.0);
                let badge_width = requested_badge_width.min(available_badge_width);
                Rect::new(
                    trailing_content_left - badge_width,
                    row_rect.y + (row_height - badge_height) * 0.5,
                    badge_width,
                    badge_height,
                )
            });
            let label_right =
                badge_rect.map(|badge| badge.x - badge_padding).unwrap_or(trailing_content_left);
            let label_left = cursor_x.min(label_right);
            let label_rect =
                Rect::new(label_left, row_rect.y, (label_right - label_left).max(0.0), row_height);

            TreeRowLayout {
                row_rect,
                expander_rect,
                icon_rect,
                label_rect,
                badge_rect,
                action_rects,
            }
        })
        .collect();

    let editor_layout = editor.zip(editor_insert_index).map(|(editor, insert_index)| {
        let row_rect = Rect::new(
            rect.x,
            rect.y + insert_index as f32 * row_height - scroll_offset_px,
            rect.w,
            row_height,
        );
        let text_box_left =
            rect.x + horizontal_padding + editor.depth as f32 * TREE_ROW_INDENT_LOGICAL * dpi;
        let text_box_rect = Rect::new(
            text_box_left,
            row_rect.y + 2.0 * dpi,
            (rect.right() - horizontal_padding - text_box_left).max(0.0),
            (row_height - 4.0 * dpi).max(0.0),
        );
        TreeRowEditorLayout { row_rect, text_box_rect }
    });

    let visual_row_count = rows.len() + usize::from(editor_layout.is_some());
    TreeListLayout {
        rows: row_layouts,
        editor: editor_layout,
        content_height_px: visual_row_count as f32 * row_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::tree_list::{TreeRowExpansion, TreeRowKey, TreeRowSelection};

    fn row(depth: usize) -> TreeRowInput {
        TreeRowInput {
            key: TreeRowKey(1),
            label: "Nested".to_owned(),
            icon: None,
            depth,
            expansion: TreeRowExpansion::Leaf,
            selection: TreeRowSelection::Unselected,
            badge: Some(10_000),
            tooltip: None,
            trailing_actions: Vec::new(),
        }
    }

    #[test]
    fn layout_keeps_rows_inside_their_scrolled_content_space() {
        let layout =
            build_tree_layout(&[row(3)], None, Rect::new(20.0, 10.0, 180.0, 100.0), 8.0, 1.5);

        assert_eq!(layout.content_height_px, TREE_ROW_HEIGHT_LOGICAL * 1.5);
        assert_eq!(layout.rows[0].row_rect.y, 2.0);
        assert!(layout.rows[0].label_rect.x > 90.0);
        assert!(
            layout.rows[0].badge_rect.expect("badge input should produce badge geometry").w
                > TREE_BADGE_MINIMUM_WIDTH_LOGICAL * 1.5
        );
    }

    #[test]
    fn leaf_icon_reserves_an_empty_expander_slot_for_sibling_alignment() {
        let mut leaf = row(0);
        leaf.icon = Some("file".to_owned());
        let list_rect = Rect::new(12.0, 20.0, 180.0, 100.0);

        let layout = build_tree_layout(&[leaf], None, list_rect, 0.0, 1.0);

        assert_eq!(
            layout.rows[0].icon_rect.expect("leaf icon should have layout").x,
            list_rect.x
                + TREE_ROW_HORIZONTAL_PADDING_LOGICAL
                + TREE_EXPANDER_SIZE_LOGICAL
                + TREE_ICON_GAP_LOGICAL
        );
    }

    #[test]
    fn trailing_actions_are_right_anchored_and_keep_label_clear_at_narrow_widths() {
        let mut input = row(0);
        input.trailing_actions = vec![
            crate::widgets::tree_list::TreeRowActionInput::enabled(1, "plus", "新建"),
            crate::widgets::tree_list::TreeRowActionInput::enabled(2, "folder-open", "打开"),
            crate::widgets::tree_list::TreeRowActionInput::enabled(3, "folder-plus", "新建目录"),
        ];
        let list_rect = Rect::new(12.0, 20.0, 88.0, 100.0);

        let layout = build_tree_layout(&[input], None, list_rect, 0.0, 2.0);
        let row_layout = &layout.rows[0];

        assert_eq!(row_layout.action_rects.len(), 3);
        assert!(
            row_layout.action_rects.iter().all(|rect| {
                rect.left() >= list_rect.left() && rect.right() <= list_rect.right()
            })
        );
        assert!(row_layout.label_rect.right() <= row_layout.action_rects[0].left());
        assert!(row_layout.action_rects.windows(2).all(|pair| pair[0].x < pair[1].x));
    }

    #[test]
    fn editor_is_inserted_immediately_after_its_parent_and_shifts_following_rows() {
        let parent = row(0);
        let mut sibling = row(0);
        sibling.key = TreeRowKey(2);
        let editor = TreeRowEditorInput {
            key: TreeRowKey(99),
            parent_key: parent.key,
            depth: 1,
            value: String::new(),
            placeholder: "新目录名称".to_owned(),
        };

        let layout = build_tree_layout(
            &[parent, sibling],
            Some(&editor),
            Rect::new(0.0, 0.0, 240.0, 100.0),
            0.0,
            1.0,
        );

        assert_eq!(layout.rows[0].row_rect.y, 0.0);
        assert_eq!(layout.editor.expect("editor should be laid out").row_rect.y, 28.0);
        assert_eq!(layout.rows[1].row_rect.y, 56.0);
        assert_eq!(layout.content_height_px, 84.0);
    }
}
