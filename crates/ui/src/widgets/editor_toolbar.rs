//! 编辑器通用菜单栏；文档类型命令集合由产品层注入。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use std::any::Any;

const TOOLBAR_COMMAND_WIDTH_LOGICAL: f32 = 44.0;
const TOOLBAR_COMMAND_GAP_LOGICAL: f32 = 4.0;
const TOOLBAR_OVERFLOW_WIDTH_LOGICAL: f32 = 32.0;
const TOOLBAR_FONT_SIZE_LOGICAL: f32 = 12.0;

pub const EDITOR_TOOLBAR_COMMAND_ID: WidgetId = WidgetId(10_301);
pub const EDITOR_TOOLBAR_OVERFLOW_ID: WidgetId = WidgetId(10_302);
pub const EDITOR_TOOLBAR_DISMISS_ID: WidgetId = WidgetId(10_303);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorToolbarCommandInput {
    pub command_key: String,
    pub label: String,
    pub enabled: bool,
    pub overflow_priority: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorToolbarGroupInput {
    pub label: String,
    pub commands: Vec<EditorToolbarCommandInput>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorToolbarInput {
    pub groups: Vec<EditorToolbarGroupInput>,
    pub overflow_open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorToolbarAction {
    CommandInvoked(String),
    OverflowOpened,
    Dismissed,
}

pub struct EditorToolbarWidget {
    input: EditorToolbarInput,
    rect: Rect,
}

impl EditorToolbarWidget {
    pub fn new() -> Self {
        Self { input: EditorToolbarInput::default(), rect: Rect::ZERO }
    }

    pub fn set_input(&mut self, input: EditorToolbarInput) {
        self.input = input;
    }

    pub fn visible_command_keys(&self, available_width: f32) -> (Vec<String>, Vec<String>) {
        let commands = self.commands();
        let mut remaining_width = (available_width - TOOLBAR_OVERFLOW_WIDTH_LOGICAL).max(0.0);
        let mut visible_indices = Vec::new();
        let mut overflow_indices = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            let command_width = self.command_width(command);
            let must_remain_visible = command.overflow_priority == 0;
            if must_remain_visible || command_width <= remaining_width {
                visible_indices.push(index);
                remaining_width = (remaining_width - command_width).max(0.0);
            } else {
                overflow_indices.push(index);
            }
        }
        let visible = visible_indices
            .into_iter()
            .filter_map(|index| commands.get(index).map(|command| command.command_key.clone()))
            .collect();
        let overflow = overflow_indices
            .into_iter()
            .filter_map(|index| commands.get(index).map(|command| command.command_key.clone()))
            .collect();
        (visible, overflow)
    }

    fn commands(&self) -> Vec<EditorToolbarCommandInput> {
        self.input.groups.iter().flat_map(|group| group.commands.iter().cloned()).collect()
    }

    fn command_width(&self, command: &EditorToolbarCommandInput) -> f32 {
        let label_width = command.label.chars().count() as f32 * TOOLBAR_FONT_SIZE_LOGICAL * 0.55;
        (label_width + TOOLBAR_COMMAND_WIDTH_LOGICAL * 0.5).max(TOOLBAR_COMMAND_WIDTH_LOGICAL)
    }

    fn command_index_at(&self, px: f32, dpi: f32) -> Option<usize> {
        let (visible, _) = self.visible_command_keys(self.rect.w / dpi);
        let mut left = self.rect.x;
        for (index, key) in visible.iter().enumerate() {
            let command =
                self.commands().into_iter().find(|command| command.command_key == *key)?;
            let width = self.command_width(&command) * dpi;
            if Rect::new(left, self.rect.y, width, self.rect.h).contains(px, self.rect.y + 1.0) {
                return Some(index);
            }
            left += width + TOOLBAR_COMMAND_GAP_LOGICAL * dpi;
        }
        None
    }
}

impl Default for EditorToolbarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for EditorToolbarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (visible, overflow) = self.visible_command_keys(self.rect.w / ctx.dpi);
        let commands = self.commands();
        let mut left = self.rect.x;
        for key in visible {
            let Some(command) = commands.iter().find(|command| command.command_key == key) else {
                continue;
            };
            let width = self.command_width(command) * ctx.dpi;
            let color = ctx.theme.palette.text_muted;
            ctx.text(
                left + 8.0 * ctx.dpi,
                self.rect.y + self.rect.h * 0.5 + TOOLBAR_FONT_SIZE_LOGICAL * ctx.dpi * 0.35,
                TOOLBAR_FONT_SIZE_LOGICAL * ctx.dpi,
                color,
                &command.label,
            );
            left += width + TOOLBAR_COMMAND_GAP_LOGICAL * ctx.dpi;
        }
        if !overflow.is_empty() {
            ctx.text(
                left + 8.0 * ctx.dpi,
                self.rect.y + self.rect.h * 0.5 + TOOLBAR_FONT_SIZE_LOGICAL * ctx.dpi * 0.35,
                TOOLBAR_FONT_SIZE_LOGICAL * ctx.dpi,
                ctx.theme.palette.text_muted,
                "更多",
            );
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match event {
            Event::KeyDown(crate::core::KeyCode::Escape, _) if self.input.overflow_open => {
                Some(WidgetAction::Control(ControlAction::Activated {
                    id: EDITOR_TOOLBAR_DISMISS_ID,
                }))
            }
            Event::MouseDown { px, py, button: crate::core::MouseButton::Left } => {
                if !self.rect.contains(*px, *py) {
                    return Some(WidgetAction::Control(ControlAction::Activated {
                        id: EDITOR_TOOLBAR_DISMISS_ID,
                    }));
                }
                if let Some(index) = self.command_index_at(*px, ctx.dpi) {
                    let (visible, _) = self.visible_command_keys(self.rect.w / ctx.dpi);
                    let key = visible.get(index)?.clone();
                    let command =
                        self.commands().into_iter().find(|command| command.command_key == key)?;
                    if !command.enabled {
                        return None;
                    }
                    return Some(WidgetAction::Control(ControlAction::TextCommitted {
                        id: EDITOR_TOOLBAR_COMMAND_ID,
                        value: TextPayload::Plain(key),
                    }));
                }
                Some(WidgetAction::Control(ControlAction::Activated {
                    id: EDITOR_TOOLBAR_OVERFLOW_ID,
                }))
            }
            _ => None,
        }
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

    fn toolbar() -> EditorToolbarWidget {
        let mut toolbar = EditorToolbarWidget::new();
        toolbar.set_input(EditorToolbarInput {
            groups: vec![EditorToolbarGroupInput {
                label: "格式".to_owned(),
                commands: vec![
                    EditorToolbarCommandInput {
                        command_key: "undo".to_owned(),
                        label: "撤销".to_owned(),
                        enabled: true,
                        overflow_priority: 0,
                    },
                    EditorToolbarCommandInput {
                        command_key: "link".to_owned(),
                        label: "链接".to_owned(),
                        enabled: true,
                        overflow_priority: 10,
                    },
                ],
            }],
            overflow_open: false,
        });
        toolbar
    }

    #[test]
    fn narrow_toolbar_moves_low_priority_commands_to_overflow_without_changing_keys() {
        let toolbar = toolbar();
        assert_eq!(
            toolbar.visible_command_keys(48.0),
            (vec!["undo".to_owned()], vec!["link".to_owned()])
        );
    }
}
