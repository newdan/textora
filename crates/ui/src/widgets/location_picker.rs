//! 工作区目录选择器；只处理通用目录行和稳定 row key。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use std::any::Any;

const PICKER_HEADER_HEIGHT_LOGICAL: f32 = 36.0;
const PICKER_ROW_HEIGHT_LOGICAL: f32 = 28.0;
const PICKER_HORIZONTAL_PADDING_LOGICAL: f32 = 12.0;
const PICKER_CHEVRON_WIDTH_LOGICAL: f32 = 24.0;
const PICKER_FONT_SIZE_LOGICAL: f32 = 13.0;

pub const LOCATION_PICKER_SELECT_ID: WidgetId = WidgetId(10_101);
pub const LOCATION_PICKER_TOGGLE_ID: WidgetId = WidgetId(10_102);
pub const LOCATION_PICKER_CANCEL_ID: WidgetId = WidgetId(10_103);
pub const LOCATION_PICKER_DISMISS_ID: WidgetId = WidgetId(10_104);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocationDirectoryInput {
    pub row_key: String,
    pub label: String,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocationPickerInput {
    pub workspace_name: String,
    pub current_relative_path: String,
    pub directories: Vec<LocationDirectoryInput>,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocationPickerAction {
    DirectorySelected { row_key: String },
    DirectoryToggled { row_key: String },
    Cancelled,
    Dismissed,
}

pub struct LocationPickerWidget {
    input: LocationPickerInput,
    rect: Rect,
}

impl LocationPickerWidget {
    pub fn new() -> Self {
        Self { input: LocationPickerInput::default(), rect: Rect::ZERO }
    }

    pub fn set_input(&mut self, input: LocationPickerInput) {
        self.input = input;
    }

    pub fn event_action(&mut self, _event: &Event, _dpi: f32) -> Option<LocationPickerAction> {
        if !self.input.open {
            return None;
        }
        match _event {
            Event::KeyDown(crate::core::KeyCode::Escape, _) => {
                Some(LocationPickerAction::Cancelled)
            }
            Event::MouseDown { px, py, button: crate::core::MouseButton::Left } => {
                if !self.rect.contains(*px, *py) {
                    return Some(LocationPickerAction::Dismissed);
                }
                let row_index = self.row_index_at(*py, _dpi)?;
                let row = self.input.directories.get(row_index)?;
                if !row.enabled {
                    return None;
                }
                let row_rect = self.row_rect(row_index, _dpi);
                if row.has_children && *px < row_rect.x + PICKER_CHEVRON_WIDTH_LOGICAL * _dpi {
                    return Some(LocationPickerAction::DirectoryToggled {
                        row_key: row.row_key.clone(),
                    });
                }
                Some(LocationPickerAction::DirectorySelected { row_key: row.row_key.clone() })
            }
            _ => None,
        }
    }

    fn row_rect(&self, index: usize, dpi: f32) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y
                + PICKER_HEADER_HEIGHT_LOGICAL * dpi
                + index as f32 * PICKER_ROW_HEIGHT_LOGICAL * dpi,
            self.rect.w,
            PICKER_ROW_HEIGHT_LOGICAL * dpi,
        )
    }

    fn row_index_at(&self, py: f32, dpi: f32) -> Option<usize> {
        self.input.directories.iter().enumerate().find_map(|(index, _)| {
            self.row_rect(index, dpi).contains(self.rect.x + 1.0, py).then_some(index)
        })
    }
}

impl Default for LocationPickerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for LocationPickerWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if !self.input.open || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        ctx.list.fill_rounded(self.rect, ctx.theme.palette.bg_surface, 8.0 * ctx.dpi);
        let header_baseline = self.rect.y
            + PICKER_HEADER_HEIGHT_LOGICAL * ctx.dpi * 0.5
            + PICKER_FONT_SIZE_LOGICAL * ctx.dpi * 0.35;
        let header = if self.input.current_relative_path.is_empty() {
            self.input.workspace_name.clone()
        } else {
            format!("{} · {}", self.input.workspace_name, self.input.current_relative_path)
        };
        ctx.text(
            self.rect.x + PICKER_HORIZONTAL_PADDING_LOGICAL * ctx.dpi,
            header_baseline,
            PICKER_FONT_SIZE_LOGICAL * ctx.dpi,
            ctx.theme.palette.text_main,
            &header,
        );
        for (index, row) in self.input.directories.iter().enumerate() {
            let row_rect = self.row_rect(index, ctx.dpi);
            let text_x = row_rect.x
                + PICKER_HORIZONTAL_PADDING_LOGICAL * ctx.dpi
                + row.depth as f32 * PICKER_HORIZONTAL_PADDING_LOGICAL * ctx.dpi;
            let baseline =
                row_rect.y + row_rect.h * 0.5 + PICKER_FONT_SIZE_LOGICAL * ctx.dpi * 0.35;
            let color = if row.enabled {
                ctx.theme.palette.text_main
            } else {
                ctx.theme.palette.text_muted
            };
            ctx.text(text_x, baseline, PICKER_FONT_SIZE_LOGICAL * ctx.dpi, color, &row.label);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.input.open && self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        self.event_action(event, ctx.dpi).map(|action| match action {
            LocationPickerAction::DirectorySelected { row_key } => {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: LOCATION_PICKER_SELECT_ID,
                    value: TextPayload::Plain(row_key),
                })
            }
            LocationPickerAction::DirectoryToggled { row_key } => {
                WidgetAction::Control(ControlAction::TextEdited {
                    id: LOCATION_PICKER_TOGGLE_ID,
                    value: TextPayload::Plain(row_key),
                })
            }
            LocationPickerAction::Cancelled => {
                WidgetAction::Control(ControlAction::Activated { id: LOCATION_PICKER_CANCEL_ID })
            }
            LocationPickerAction::Dismissed => {
                WidgetAction::Control(ControlAction::Activated { id: LOCATION_PICKER_DISMISS_ID })
            }
        })
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
    use crate::core::{Event, KeyCode, Modifiers};

    fn input() -> LocationPickerInput {
        LocationPickerInput {
            workspace_name: "Notora".to_owned(),
            current_relative_path: "notes".to_owned(),
            directories: vec![
                LocationDirectoryInput {
                    row_key: "root".to_owned(),
                    label: "工作区根目录".to_owned(),
                    depth: 0,
                    expanded: true,
                    has_children: true,
                    enabled: true,
                },
                LocationDirectoryInput {
                    row_key: "notes".to_owned(),
                    label: "notes".to_owned(),
                    depth: 1,
                    expanded: false,
                    has_children: false,
                    enabled: true,
                },
            ],
            open: true,
        }
    }

    #[test]
    fn escape_cancels_and_clicking_outside_dismisses_without_domain_values() {
        let mut picker = LocationPickerWidget::new();
        picker.set_input(input());

        assert_eq!(
            picker.event_action(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), 1.0),
            Some(LocationPickerAction::Cancelled)
        );
        assert_eq!(
            picker.event_action(
                &Event::MouseDown { px: 900.0, py: 900.0, button: crate::core::MouseButton::Left },
                1.0,
            ),
            Some(LocationPickerAction::Dismissed)
        );
    }

    #[test]
    fn directory_rows_expose_stable_keys_for_toggle_and_selection() {
        let mut picker = LocationPickerWidget::new();
        picker.set_input(input());
        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        picker.set_rect(Rect::new(0.0, 0.0, 360.0, 180.0), &mut layout_context);

        assert_eq!(
            picker.event_action(
                &Event::MouseDown {
                    px: 4.0,
                    py: PICKER_HEADER_HEIGHT_LOGICAL + PICKER_ROW_HEIGHT_LOGICAL * 0.5,
                    button: crate::core::MouseButton::Left,
                },
                1.0,
            ),
            Some(LocationPickerAction::DirectoryToggled { row_key: "root".to_owned() })
        );
        assert_eq!(
            picker.event_action(
                &Event::MouseDown {
                    px: 80.0,
                    py: PICKER_HEADER_HEIGHT_LOGICAL + PICKER_ROW_HEIGHT_LOGICAL * 1.5,
                    button: crate::core::MouseButton::Left,
                },
                1.0,
            ),
            Some(LocationPickerAction::DirectorySelected { row_key: "notes".to_owned() })
        );
    }
}
