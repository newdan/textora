//! 纯数据驱动的 Markdown 表格选格面板。

use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use std::any::Any;

const CELL_SIZE_LOGICAL: f32 = 24.0;
const GRID_PADDING_LOGICAL: f32 = 12.0;
const HEADER_HEIGHT_LOGICAL: f32 = 34.0;
const INPUT_HEIGHT_LOGICAL: f32 = 30.0;
const PANEL_PADDING_LOGICAL: f32 = 12.0;
const FONT_SIZE_LOGICAL: f32 = 13.0;
const MINIMUM_COLUMNS: usize = 1;
const MINIMUM_ROWS: usize = 2;
const MAXIMUM_COLUMNS: usize = 64;
const MAXIMUM_ROWS: usize = 1_000;

pub const TABLE_PICKER_CONFIRM_ID: WidgetId = WidgetId(10_201);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TablePickerInput {
    pub open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableSize {
    pub columns: usize,
    pub rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TablePickerAction {
    PreviewChanged(Option<TableSize>),
    Confirmed(TableSize),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumericField {
    Columns,
    Rows,
}

pub struct TablePickerWidget {
    input: TablePickerInput,
    rect: Rect,
    selection: Option<TableSize>,
    columns_text: String,
    rows_text: String,
    focused_field: NumericField,
    replace_focused_field: bool,
    ime_composing: bool,
    dragging: bool,
    grid_columns: usize,
    grid_rows: usize,
}

impl TablePickerWidget {
    pub fn new() -> Self {
        Self {
            input: TablePickerInput::default(),
            rect: Rect::ZERO,
            selection: None,
            columns_text: String::new(),
            rows_text: String::new(),
            focused_field: NumericField::Columns,
            replace_focused_field: false,
            ime_composing: false,
            dragging: false,
            grid_columns: 6,
            grid_rows: 5,
        }
    }

    pub fn set_input(&mut self, input: TablePickerInput) {
        if self.input.open != input.open && input.open {
            self.clear_selection();
        }
        self.input = input;
    }

    pub fn selection(&self) -> Option<TableSize> {
        self.selection
    }

    pub fn event_action(&mut self, event: &Event, dpi: f32) -> Option<TablePickerAction> {
        if !self.input.open {
            return None;
        }
        match event {
            Event::KeyDown(KeyCode::Escape, _) => {
                self.dragging = false;
                Some(TablePickerAction::Cancelled)
            }
            Event::KeyDown(KeyCode::Enter, _) if self.ime_composing => {
                Some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::KeyDown(KeyCode::Enter, _) => self.confirm(),
            Event::KeyDown(KeyCode::Tab, modifiers) => {
                self.focused_field = match (self.focused_field, modifiers.shift) {
                    (NumericField::Columns, false) | (NumericField::Rows, true) => {
                        NumericField::Rows
                    }
                    _ => NumericField::Columns,
                };
                self.replace_focused_field = true;
                Some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::KeyDown(key, _)
                if matches!(key, KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right) =>
            {
                self.adjust_selection(*key);
                Some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::KeyDown(KeyCode::Char(character), _) if character.is_ascii_digit() => {
                if self.ime_composing {
                    return Some(TablePickerAction::PreviewChanged(self.selection));
                }
                self.edit_dimension(*character);
                Some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::KeyDown(KeyCode::Backspace, _) => {
                if self.ime_composing {
                    return Some(TablePickerAction::PreviewChanged(self.selection));
                }
                self.erase_dimension_digit();
                Some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::ImePreedit { .. } => {
                self.ime_composing = true;
                Some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::ImeCommit(_) | Event::ImeDisable => {
                self.ime_composing = false;
                Some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::MouseMove { px, py } => {
                let Some(size) = self.grid_size_at(*px, *py, dpi, self.dragging) else {
                    return self
                        .dragging
                        .then_some(TablePickerAction::PreviewChanged(self.selection));
                };
                let changed = self.set_selection(size);
                changed.then_some(TablePickerAction::PreviewChanged(self.selection))
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if let Some(field) = self.numeric_field_at(*px, *py, dpi) {
                    self.focused_field = field;
                    self.dragging = false;
                    self.replace_focused_field = true;
                    return Some(TablePickerAction::PreviewChanged(self.selection));
                }
                if let Some(size) = self.grid_size_at(*px, *py, dpi, false) {
                    self.dragging = true;
                    self.set_selection(size);
                    return Some(TablePickerAction::PreviewChanged(self.selection));
                }
                None
            }
            Event::MouseUp { px, py, button: MouseButton::Left } if self.dragging => {
                self.dragging = false;
                if let Some(size) = self.grid_size_at(*px, *py, dpi, true) {
                    self.set_selection(size);
                }
                self.confirm()
            }
            Event::MouseUp { px, py, button: MouseButton::Left } => {
                self.grid_size_at(*px, *py, dpi, false).and_then(|size| {
                    self.set_selection(size).then_some(TablePickerAction::Confirmed(size))
                })
            }
            Event::InteractionCancel => {
                self.dragging = false;
                Some(TablePickerAction::Cancelled)
            }
            _ => None,
        }
    }

    fn clear_selection(&mut self) {
        self.selection = None;
        self.columns_text.clear();
        self.rows_text.clear();
        self.dragging = false;
    }

    fn confirm(&self) -> Option<TablePickerAction> {
        if self.ime_composing {
            return None;
        }
        self.selection.map(TablePickerAction::Confirmed)
    }

    fn set_selection(&mut self, size: TableSize) -> bool {
        let size = TableSize {
            columns: size.columns.clamp(MINIMUM_COLUMNS, MAXIMUM_COLUMNS),
            rows: size.rows.clamp(MINIMUM_ROWS, MAXIMUM_ROWS),
        };
        if self.selection == Some(size) {
            return false;
        }
        self.selection = Some(size);
        self.columns_text = size.columns.to_string();
        self.rows_text = size.rows.to_string();
        true
    }

    fn grid_origin(&self, dpi: f32) -> (f32, f32) {
        (self.rect.x + GRID_PADDING_LOGICAL * dpi, self.rect.y + HEADER_HEIGHT_LOGICAL * dpi)
    }

    fn grid_size_at(
        &mut self,
        px: f32,
        py: f32,
        dpi: f32,
        allow_drag_expansion: bool,
    ) -> Option<TableSize> {
        let (origin_x, origin_y) = self.grid_origin(dpi);
        let cell_size = CELL_SIZE_LOGICAL * dpi;
        let grid_right = self.rect.x + self.rect.w - GRID_PADDING_LOGICAL * dpi;
        let grid_bottom =
            self.rect.y + self.rect.h - (INPUT_HEIGHT_LOGICAL + PANEL_PADDING_LOGICAL) * dpi;
        if px < origin_x
            || py < origin_y
            || (!allow_drag_expansion && (px >= grid_right || py >= grid_bottom))
        {
            return None;
        }
        let columns = (((px - origin_x) / cell_size).floor() as usize + 1).min(MAXIMUM_COLUMNS);
        let rows = (((py - origin_y) / cell_size).floor() as usize + 2).min(MAXIMUM_ROWS);
        self.grid_columns = self.grid_columns.max(columns).min(MAXIMUM_COLUMNS);
        self.grid_rows = self.grid_rows.max(rows.saturating_sub(1)).min(MAXIMUM_ROWS - 1);
        Some(TableSize { columns, rows })
    }

    fn numeric_field_at(&self, px: f32, py: f32, dpi: f32) -> Option<NumericField> {
        let input_y =
            self.rect.y + self.rect.h - (INPUT_HEIGHT_LOGICAL + PANEL_PADDING_LOGICAL) * dpi;
        if !self.rect.contains(px, py) || py < input_y || py >= input_y + INPUT_HEIGHT_LOGICAL * dpi
        {
            return None;
        }
        let split_x = self.rect.x + self.rect.w * 0.5;
        Some(if px < split_x { NumericField::Columns } else { NumericField::Rows })
    }

    fn adjust_selection(&mut self, key: KeyCode) {
        let current = self.selection.unwrap_or(TableSize { columns: 1, rows: 2 });
        let updated = match key {
            KeyCode::Left => TableSize {
                columns: current.columns.saturating_sub(1).max(MINIMUM_COLUMNS),
                ..current
            },
            KeyCode::Right => TableSize {
                columns: current.columns.saturating_add(1).min(MAXIMUM_COLUMNS),
                ..current
            },
            KeyCode::Up => {
                TableSize { rows: current.rows.saturating_sub(1).max(MINIMUM_ROWS), ..current }
            }
            KeyCode::Down => {
                TableSize { rows: current.rows.saturating_add(1).min(MAXIMUM_ROWS), ..current }
            }
            _ => current,
        };
        self.set_selection(updated);
    }

    fn edit_dimension(&mut self, digit: char) {
        if self.replace_focused_field {
            match self.focused_field {
                NumericField::Columns => self.columns_text.clear(),
                NumericField::Rows => self.rows_text.clear(),
            }
            self.replace_focused_field = false;
        }
        let text = match self.focused_field {
            NumericField::Columns => &mut self.columns_text,
            NumericField::Rows => &mut self.rows_text,
        };
        text.push(digit);
        self.apply_numeric_dimensions();
    }

    fn erase_dimension_digit(&mut self) {
        match self.focused_field {
            NumericField::Columns => self.columns_text.pop(),
            NumericField::Rows => self.rows_text.pop(),
        };
        self.apply_numeric_dimensions();
    }

    fn apply_numeric_dimensions(&mut self) {
        let columns = self.columns_text.parse::<usize>().ok();
        let rows = self.rows_text.parse::<usize>().ok();
        let size = match (columns, rows) {
            (Some(columns), Some(rows)) => Some(TableSize {
                columns: columns.clamp(MINIMUM_COLUMNS, MAXIMUM_COLUMNS),
                rows: rows.clamp(MINIMUM_ROWS, MAXIMUM_ROWS),
            }),
            _ => None,
        };
        if let Some(size) = size {
            self.set_selection(size);
        }
    }

    fn input_rect(&self, dpi: f32, field: NumericField) -> Rect {
        let input_y =
            self.rect.y + self.rect.h - (INPUT_HEIGHT_LOGICAL + PANEL_PADDING_LOGICAL) * dpi;
        let half_width = self.rect.w * 0.5 - PANEL_PADDING_LOGICAL * dpi;
        let input_x = self.rect.x
            + PANEL_PADDING_LOGICAL * dpi
            + if field == NumericField::Rows { self.rect.w * 0.5 } else { 0.0 };
        Rect::new(input_x, input_y, half_width, INPUT_HEIGHT_LOGICAL * dpi)
    }
}

impl Default for TablePickerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TablePickerWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = rect;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if !self.input.open || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let dpi = ctx.dpi;
        ctx.list.fill_rounded(self.rect, ctx.theme.palette.bg_surface, 8.0 * dpi);
        let size_label = self.selection.map_or_else(
            || "选择表格大小".to_owned(),
            |size| format!("{} 列 × {} 行", size.columns, size.rows),
        );
        ctx.text(
            self.rect.x + PANEL_PADDING_LOGICAL * dpi,
            self.rect.y + 22.0 * dpi,
            FONT_SIZE_LOGICAL * dpi,
            ctx.theme.palette.text_main,
            &size_label,
        );
        let (origin_x, origin_y) = self.grid_origin(dpi);
        let cell_size = CELL_SIZE_LOGICAL * dpi;
        for row in 0..self.grid_rows {
            for column in 0..self.grid_columns {
                let cell = Rect::new(
                    origin_x + column as f32 * cell_size,
                    origin_y + row as f32 * cell_size,
                    cell_size - 2.0 * dpi,
                    cell_size - 2.0 * dpi,
                );
                let selected =
                    self.selection.is_some_and(|size| column < size.columns && row + 1 < size.rows);
                let color =
                    if selected { ctx.theme.palette.accent } else { ctx.theme.palette.bg_base };
                ctx.list.fill_rounded(cell, color, 2.0 * dpi);
            }
        }
        for (field, label, value) in [
            (NumericField::Columns, "列", self.columns_text.as_str()),
            (NumericField::Rows, "行", self.rows_text.as_str()),
        ] {
            let field_rect = self.input_rect(dpi, field);
            ctx.list.stroke_rounded(
                field_rect,
                ctx.theme.palette.border_subtle,
                1.0 * dpi,
                3.0 * dpi,
            );
            ctx.text(
                field_rect.x + 6.0 * dpi,
                field_rect.y + 19.0 * dpi,
                FONT_SIZE_LOGICAL * dpi,
                ctx.theme.palette.text_main,
                &format!("{label}: {value}"),
            );
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.input.open && self.rect.contains(px, py)
    }

    fn is_capturing(&self) -> bool {
        self.dragging
    }

    fn id(&self) -> Option<WidgetId> {
        Some(TABLE_PICKER_CONFIRM_ID)
    }

    fn is_focusable(&self) -> bool {
        self.input.open
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let action = self.event_action(event, ctx.dpi)?;
        if matches!(event, Event::MouseDown { .. }) {
            return Some(WidgetAction::Control(ControlAction::FocusRequested {
                id: TABLE_PICKER_CONFIRM_ID,
            }));
        }
        Some(WidgetAction::TablePicker(action))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Modifiers;

    fn picker() -> TablePickerWidget {
        let mut picker = TablePickerWidget::new();
        picker.set_input(TablePickerInput { open: true });
        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        picker.set_rect(Rect::new(0.0, 0.0, 260.0, 220.0), &mut context);
        picker
    }

    #[test]
    fn starts_unselected_and_enter_does_not_choose_an_implicit_size() {
        let mut picker = picker();
        assert_eq!(picker.id(), Some(TABLE_PICKER_CONFIRM_ID));
        assert!(picker.is_focusable());
        assert_eq!(picker.selection(), None);
        assert_eq!(
            picker.event_action(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), 1.0),
            None
        );
    }

    #[test]
    fn hover_and_drag_preview_rectangular_dimensions_then_confirm_on_release() {
        let mut picker = picker();
        let column_x = GRID_PADDING_LOGICAL + CELL_SIZE_LOGICAL * 2.5;
        let row_y = HEADER_HEIGHT_LOGICAL + CELL_SIZE_LOGICAL * 1.5;
        assert_eq!(
            picker.event_action(&Event::MouseMove { px: column_x, py: row_y }, 1.0),
            Some(TablePickerAction::PreviewChanged(Some(TableSize { columns: 3, rows: 3 })))
        );
        picker.event_action(
            &Event::MouseDown {
                px: GRID_PADDING_LOGICAL + 2.0,
                py: HEADER_HEIGHT_LOGICAL + 2.0,
                button: MouseButton::Left,
            },
            1.0,
        );
        let action = picker.event_action(
            &Event::MouseUp { px: column_x, py: row_y, button: MouseButton::Left },
            1.0,
        );
        assert_eq!(action, Some(TablePickerAction::Confirmed(TableSize { columns: 3, rows: 3 })));
    }

    #[test]
    fn keyboard_and_numeric_inputs_adjust_dimensions_and_escape_cancels() {
        let mut picker = picker();
        assert_eq!(
            picker.event_action(&Event::KeyDown(KeyCode::Right, Modifiers::NONE), 1.0),
            Some(TablePickerAction::PreviewChanged(Some(TableSize { columns: 2, rows: 2 })))
        );
        picker.event_action(&Event::KeyDown(KeyCode::Tab, Modifiers::NONE), 1.0);
        picker.event_action(&Event::KeyDown(KeyCode::Char('4'), Modifiers::NONE), 1.0);
        assert_eq!(picker.selection(), Some(TableSize { columns: 2, rows: 4 }));
        assert_eq!(
            picker.event_action(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), 1.0),
            Some(TablePickerAction::Cancelled)
        );
    }

    #[test]
    fn dragging_beyond_the_visible_grid_expands_the_selection() {
        let mut picker = picker();
        picker.event_action(
            &Event::MouseDown {
                px: GRID_PADDING_LOGICAL + 2.0,
                py: HEADER_HEIGHT_LOGICAL + 2.0,
                button: MouseButton::Left,
            },
            1.0,
        );
        let action = picker.event_action(
            &Event::MouseMove {
                px: GRID_PADDING_LOGICAL + CELL_SIZE_LOGICAL * 10.5,
                py: HEADER_HEIGHT_LOGICAL + CELL_SIZE_LOGICAL * 7.5,
            },
            1.0,
        );
        assert_eq!(
            action,
            Some(TablePickerAction::PreviewChanged(Some(TableSize { columns: 11, rows: 9 })))
        );
        assert_eq!(picker.grid_columns, 11);
        assert_eq!(picker.grid_rows, 8);
    }

    #[test]
    fn pointer_coordinates_are_scaled_by_dpi() {
        let mut picker = picker();
        let mut picker_rect = picker.rect;
        picker_rect.w *= 2.0;
        picker_rect.h *= 2.0;
        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 2.0 };
        picker.set_rect(picker_rect, &mut context);
        let action = picker.event_action(
            &Event::MouseMove {
                px: 2.0 * (GRID_PADDING_LOGICAL + CELL_SIZE_LOGICAL * 1.5),
                py: 2.0 * (HEADER_HEIGHT_LOGICAL + CELL_SIZE_LOGICAL * 0.5),
            },
            2.0,
        );
        assert_eq!(
            action,
            Some(TablePickerAction::PreviewChanged(Some(TableSize { columns: 2, rows: 2 })))
        );
    }

    #[test]
    fn ime_preedit_blocks_numeric_changes_and_enter_confirmation() {
        let mut picker = picker();
        picker.set_selection(TableSize { columns: 2, rows: 4 });
        picker.event_action(&Event::ImePreedit { text: "4".to_owned(), cursor: None }, 1.0);

        picker.event_action(&Event::KeyDown(KeyCode::Char('9'), Modifiers::NONE), 1.0);
        let action = picker.event_action(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), 1.0);

        assert_eq!(picker.selection(), Some(TableSize { columns: 2, rows: 4 }));
        assert_eq!(
            action,
            Some(TablePickerAction::PreviewChanged(Some(TableSize { columns: 2, rows: 4 })))
        );
        picker.event_action(&Event::ImeCommit("4".to_owned()), 1.0);
        assert_eq!(
            picker.event_action(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), 1.0),
            Some(TablePickerAction::Confirmed(TableSize { columns: 2, rows: 4 }))
        );
    }
}
