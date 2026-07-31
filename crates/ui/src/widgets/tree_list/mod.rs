//! 与产品领域无关的分层树列表。

mod layout;

use std::any::Any;

use crate::core::{
    DrawCmd, Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::widgets::icon::draw_icon;

use self::layout::{TreeListLayout, build_tree_layout};

/// 仅在单帧 UI 输入中有效的树行键。
///
/// 产品层负责将此键映射到自己的领域动作；键不承载路径或领域 ID 语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeRowKey(pub u64);

/// 树行的可展开状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TreeRowExpansion {
    #[default]
    Leaf,
    Collapsed,
    Expanded,
}

/// 树行的选择状态，由调用方用稳定键维护。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TreeRowSelection {
    #[default]
    Unselected,
    Selected,
}

/// 一行树列表的纯展示输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRowInput {
    pub key: TreeRowKey,
    pub label: String,
    pub icon: Option<String>,
    pub depth: usize,
    pub expansion: TreeRowExpansion,
    pub selection: TreeRowSelection,
    pub badge: Option<u32>,
}

/// 树列表的每帧输入。滚动和选择均由产品层独立持有。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeListInput {
    pub rows: Vec<TreeRowInput>,
    pub scroll_offset_px: f32,
}

/// 树列表向调用方返回的通用 UI 动作。
#[derive(Clone, Debug, PartialEq)]
pub enum TreeListAction {
    Selected(TreeRowKey),
    ExpansionToggled(TreeRowKey),
    ScrollOffsetChanged(f32),
    HoverChanged(Option<TreeRowKey>),
}

/// 分层树列表组件。
pub struct TreeListWidget {
    rect: Rect,
    input: TreeListInput,
    layout: TreeListLayout,
    selected_key: Option<TreeRowKey>,
    hovered_key: Option<TreeRowKey>,
}

impl Default for TreeListWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeListWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            input: TreeListInput::default(),
            layout: TreeListLayout::default(),
            selected_key: None,
            hovered_key: None,
        }
    }

    /// 覆盖当前帧展示输入，并丢弃已不存在行的悬停状态。
    pub fn set_input(&mut self, mut input: TreeListInput) {
        debug_assert!(has_unique_keys(&input.rows), "tree row keys must be unique per frame");
        let input_selected_key = input
            .rows
            .iter()
            .find(|row| row.selection == TreeRowSelection::Selected)
            .map(|row| row.key);
        let preserved_selected_key =
            self.selected_key.filter(|key| input.rows.iter().any(|row| row.key == *key));
        self.selected_key = input_selected_key.or(preserved_selected_key);
        for row in &mut input.rows {
            row.selection = if Some(row.key) == self.selected_key {
                TreeRowSelection::Selected
            } else {
                TreeRowSelection::Unselected
            };
        }
        self.hovered_key =
            self.hovered_key.filter(|key| input.rows.iter().any(|row| row.key == *key));
        self.input = input;
    }

    pub fn input(&self) -> &TreeListInput {
        &self.input
    }

    pub fn layout(&self) -> &TreeListLayout {
        &self.layout
    }

    pub fn hovered_key(&self) -> Option<TreeRowKey> {
        self.hovered_key
    }

    pub fn selected_key(&self) -> Option<TreeRowKey> {
        self.selected_key
    }

    fn max_scroll_offset(&self) -> f32 {
        (self.layout.content_height_px - self.rect.h).max(0.0)
    }

    fn row_at(&self, px: f32, py: f32) -> Option<usize> {
        self.rect
            .contains(px, py)
            .then(|| self.layout.rows.iter().position(|row| row.row_rect.contains(px, py)))?
    }

    fn update_hover(&mut self, px: f32, py: f32) -> Option<TreeListAction> {
        let hovered_key =
            self.row_at(px, py).and_then(|index| self.input.rows.get(index)).map(|row| row.key);
        if hovered_key == self.hovered_key {
            return None;
        }
        self.hovered_key = hovered_key;
        Some(TreeListAction::HoverChanged(hovered_key))
    }

    fn select_adjacent_row(&self, direction: i32) -> Option<TreeListAction> {
        let selected_index = self
            .input
            .rows
            .iter()
            .position(|row| row.selection == TreeRowSelection::Selected)
            .unwrap_or(0);
        let next_index = if direction.is_negative() {
            selected_index.saturating_sub(1)
        } else {
            (selected_index + 1).min(self.input.rows.len().saturating_sub(1))
        };
        self.input.rows.get(next_index).map(|row| TreeListAction::Selected(row.key))
    }

    fn select_row(&mut self, key: TreeRowKey) {
        self.selected_key = Some(key);
        for row in &mut self.input.rows {
            row.selection = if row.key == key {
                TreeRowSelection::Selected
            } else {
                TreeRowSelection::Unselected
            };
        }
    }
}

impl Widget for TreeListWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.layout =
            build_tree_layout(&self.input.rows, rect, self.input.scroll_offset_px, ctx.dpi);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let clip_rect = Rect::new(
            self.rect.x + ctx.list.offset.0,
            self.rect.y + ctx.list.offset.1,
            self.rect.w,
            self.rect.h,
        );
        ctx.list.cmds.push(DrawCmd::PushClip(clip_rect));

        for (row, row_layout) in self.input.rows.iter().zip(&self.layout.rows) {
            if row_layout.row_rect.bottom() <= self.rect.top()
                || row_layout.row_rect.top() >= self.rect.bottom()
            {
                continue;
            }

            let is_hovered = self.hovered_key == Some(row.key);
            let is_selected = row.selection == TreeRowSelection::Selected;
            if is_selected {
                ctx.list.fill_menu_hover(
                    row_layout.row_rect,
                    ctx.theme.palette.sidebar_active_bg,
                    ctx.dpi,
                );
            } else if is_hovered {
                ctx.list.fill_menu_hover(
                    row_layout.row_rect,
                    ctx.theme.palette.sidebar_hover_bg,
                    ctx.dpi,
                );
            }

            paint_expansion_indicator(ctx, row.expansion, row_layout.expander_rect);
            if let (Some(icon), Some(icon_rect)) = (&row.icon, row_layout.icon_rect) {
                draw_icon(
                    ctx.list,
                    icon,
                    icon_rect.x,
                    icon_rect.y,
                    icon_rect.w,
                    ctx.theme.palette.text_muted,
                );
            }

            let text_color = if is_selected {
                ctx.theme.palette.sidebar_active_fg
            } else {
                ctx.theme.palette.text_main
            };
            let font_size = layout::TREE_ROW_FONT_SIZE_LOGICAL * ctx.dpi;
            let baseline =
                row_layout.label_rect.y + row_layout.label_rect.h * 0.5 + font_size * 0.35;
            ctx.text(row_layout.label_rect.x, baseline, font_size, text_color, &row.label);

            if let (Some(badge), Some(badge_rect)) = (row.badge, row_layout.badge_rect) {
                ctx.list.fill_rounded(
                    badge_rect,
                    ctx.theme.palette.bg_elevated,
                    badge_rect.h * 0.5,
                );
                let badge_text = badge.to_string();
                let badge_baseline = badge_rect.y + badge_rect.h * 0.5 + font_size * 0.31;
                ctx.text(
                    badge_rect.x + layout::TREE_BADGE_HORIZONTAL_PADDING_LOGICAL * ctx.dpi,
                    badge_baseline,
                    font_size,
                    ctx.theme.palette.text_muted,
                    &badge_text,
                );
            }
        }

        ctx.list.cmds.push(DrawCmd::PopClip);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let action = match event {
            Event::MouseMove { px, py } => {
                if self.hit(*px, *py) {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                self.update_hover(*px, *py)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                let index = self.row_at(*px, *py)?;
                let row = self.input.rows.get(index)?;
                let row_layout = self.layout.rows.get(index)?;
                if row.expansion != TreeRowExpansion::Leaf
                    && row_layout.expander_rect.contains(*px, *py)
                {
                    Some(TreeListAction::ExpansionToggled(row.key))
                } else {
                    Some(TreeListAction::Selected(row.key))
                }
            }
            Event::Wheel { dy, px, py, .. } if self.hit(*px, *py) => {
                let next_offset =
                    (self.input.scroll_offset_px - *dy).clamp(0.0, self.max_scroll_offset());
                if (next_offset - self.input.scroll_offset_px).abs() <= f32::EPSILON {
                    None
                } else {
                    self.input.scroll_offset_px = next_offset;
                    Some(TreeListAction::ScrollOffsetChanged(next_offset))
                }
            }
            Event::KeyDown(KeyCode::Up, _) => self.select_adjacent_row(-1),
            Event::KeyDown(KeyCode::Down, _) => self.select_adjacent_row(1),
            Event::KeyDown(KeyCode::Left, _) => self
                .input
                .rows
                .iter()
                .find(|row| row.selection == TreeRowSelection::Selected)
                .filter(|row| row.expansion == TreeRowExpansion::Expanded)
                .map(|row| TreeListAction::ExpansionToggled(row.key)),
            Event::KeyDown(KeyCode::Right, _) => self
                .input
                .rows
                .iter()
                .find(|row| row.selection == TreeRowSelection::Selected)
                .filter(|row| row.expansion == TreeRowExpansion::Collapsed)
                .map(|row| TreeListAction::ExpansionToggled(row.key)),
            _ => None,
        }?;
        if let TreeListAction::Selected(key) = action {
            self.select_row(key);
        }
        Some(WidgetAction::TreeList(action))
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn has_unique_keys(rows: &[TreeRowInput]) -> bool {
    let mut keys = std::collections::HashSet::with_capacity(rows.len());
    rows.iter().all(|row| keys.insert(row.key))
}

fn paint_expansion_indicator(ctx: &mut PaintCtx, expansion: TreeRowExpansion, rect: Rect) {
    let color = ctx.theme.palette.text_muted;
    match expansion {
        TreeRowExpansion::Leaf => {}
        TreeRowExpansion::Collapsed => ctx.list.fill_triangle(
            [rect.x + rect.w * 0.35, rect.y + rect.h * 0.2],
            [rect.x + rect.w * 0.35, rect.y + rect.h * 0.8],
            [rect.x + rect.w * 0.75, rect.y + rect.h * 0.5],
            color,
        ),
        TreeRowExpansion::Expanded => ctx.list.fill_triangle(
            [rect.x + rect.w * 0.2, rect.y + rect.h * 0.35],
            [rect.x + rect.w * 0.8, rect.y + rect.h * 0.35],
            [rect.x + rect.w * 0.5, rect.y + rect.h * 0.75],
            color,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{EventCtx, LayoutCtx, Modifiers, NoopMeasure};

    fn row(key: u64, depth: usize, expansion: TreeRowExpansion) -> TreeRowInput {
        TreeRowInput {
            key: TreeRowKey(key),
            label: format!("Row {key}"),
            icon: Some("folder".to_owned()),
            depth,
            expansion,
            selection: TreeRowSelection::Unselected,
            badge: None,
        }
    }

    fn layout(widget: &mut TreeListWidget, rect: Rect, dpi: f32) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut context = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
        widget.set_rect(rect, &mut context);
    }

    fn event_context(theme: &crate::Theme) -> EventCtx<'_> {
        EventCtx { theme, dpi: 1.0, cursor_hint: None }
    }

    #[test]
    fn preserves_stable_selection_when_rows_are_replaced() {
        let mut widget = TreeListWidget::new();
        let mut selected = row(7, 0, TreeRowExpansion::Leaf);
        selected.selection = TreeRowSelection::Selected;
        widget.set_input(TreeListInput {
            rows: vec![row(3, 0, TreeRowExpansion::Leaf), selected],
            scroll_offset_px: 0.0,
        });
        widget.set_input(TreeListInput {
            rows: vec![row(9, 0, TreeRowExpansion::Leaf), row(7, 0, TreeRowExpansion::Leaf)],
            scroll_offset_px: 42.0,
        });

        assert_eq!(widget.selected_key(), Some(TreeRowKey(7)));
        assert_eq!(
            widget
                .input()
                .rows
                .iter()
                .find(|row| row.key == TreeRowKey(7))
                .map(|row| row.selection),
            Some(TreeRowSelection::Selected)
        );
        assert_eq!(widget.input().scroll_offset_px, 42.0);
    }

    #[test]
    fn deep_rows_use_dpi_scaled_indentation_and_badge_layout() {
        let mut widget = TreeListWidget::new();
        let mut deep_row = row(2, 4, TreeRowExpansion::Collapsed);
        deep_row.badge = Some(12);
        widget.set_input(TreeListInput { rows: vec![deep_row], scroll_offset_px: 0.0 });
        layout(&mut widget, Rect::new(0.0, 0.0, 300.0, 80.0), 2.0);

        let geometry = &widget.layout().rows[0];
        assert!(geometry.label_rect.x > 100.0);
        assert!(geometry.badge_rect.is_some());
    }

    #[test]
    fn expansion_and_selection_emit_distinct_typed_actions() {
        let mut widget = TreeListWidget::new();
        widget.set_input(TreeListInput {
            rows: vec![row(1, 0, TreeRowExpansion::Collapsed)],
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(20.0, 30.0, 240.0, 80.0), 1.0);
        let expander_rect = widget.layout().rows[0].expander_rect;
        let label_rect = widget.layout().rows[0].label_rect;
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(
                &Event::MouseDown {
                    px: expander_rect.x + 1.0,
                    py: expander_rect.y + 1.0,
                    button: MouseButton::Left
                },
                &mut context,
            ),
            Some(WidgetAction::TreeList(TreeListAction::ExpansionToggled(TreeRowKey(1))))
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseDown {
                    px: label_rect.x + 1.0,
                    py: label_rect.y + 1.0,
                    button: MouseButton::Left
                },
                &mut context,
            ),
            Some(WidgetAction::TreeList(TreeListAction::Selected(TreeRowKey(1))))
        );
    }

    #[test]
    fn keyboard_selection_stays_within_available_rows() {
        let mut widget = TreeListWidget::new();
        let mut selected = row(1, 0, TreeRowExpansion::Leaf);
        selected.selection = TreeRowSelection::Selected;
        widget.set_input(TreeListInput {
            rows: vec![selected, row(2, 0, TreeRowExpansion::Leaf)],
            scroll_offset_px: 0.0,
        });
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Up, Modifiers::NONE), &mut context),
            Some(WidgetAction::TreeList(TreeListAction::Selected(TreeRowKey(1))))
        );
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Down, Modifiers::NONE), &mut context),
            Some(WidgetAction::TreeList(TreeListAction::Selected(TreeRowKey(2))))
        );
    }
}
