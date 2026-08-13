//! 编辑器通用菜单栏；文档类型命令集合由产品层注入。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget,
    WidgetAction,
};
use crate::widgets::icon::draw_icon;
use std::any::Any;

const TOOLBAR_COMMAND_SIZE_LOGICAL: f32 = 32.0;
const TOOLBAR_COMMAND_GAP_LOGICAL: f32 = 4.0;
const TOOLBAR_HORIZONTAL_PADDING_LOGICAL: f32 = 16.0;
const TOOLBAR_ICON_SIZE_LOGICAL: f32 = 16.0;
const TOOLBAR_FONT_SIZE_LOGICAL: f32 = 12.0;
const TOOLBAR_CORNER_RADIUS_LOGICAL: f32 = 5.0;
const EDITOR_TOOLBAR_ACCESSIBILITY_ID: AccessibilityId = AccessibilityId(0x6564_6974_746f_6f6c);

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
    hovered_command_key: Option<String>,
    overflow_hovered: bool,
    dpi: f32,
}

impl EditorToolbarWidget {
    pub fn new() -> Self {
        Self {
            input: EditorToolbarInput::default(),
            rect: Rect::ZERO,
            hovered_command_key: None,
            overflow_hovered: false,
            dpi: 1.0,
        }
    }

    pub fn set_input(&mut self, input: EditorToolbarInput) {
        self.input = input;
    }

    pub fn visible_command_keys(&self, available_width: f32) -> (Vec<String>, Vec<String>) {
        let commands = self.commands();
        let content_width = (available_width - TOOLBAR_HORIZONTAL_PADDING_LOGICAL * 2.0).max(0.0);
        let all_commands_width =
            command_row_width(&commands, |command| self.command_width(command));
        if all_commands_width <= content_width {
            return (commands.into_iter().map(|command| command.command_key).collect(), Vec::new());
        }

        let mut remaining_width =
            (content_width - TOOLBAR_COMMAND_SIZE_LOGICAL - TOOLBAR_COMMAND_GAP_LOGICAL).max(0.0);
        let mut visible_indices = Vec::new();
        let mut overflow_indices = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            let command_width = self.command_width(command) + TOOLBAR_COMMAND_GAP_LOGICAL;
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
        if toolbar_icon(&command.command_key).is_some() {
            return TOOLBAR_COMMAND_SIZE_LOGICAL;
        }
        let label_width = command.label.chars().count() as f32 * TOOLBAR_FONT_SIZE_LOGICAL * 0.55;
        (label_width + TOOLBAR_COMMAND_SIZE_LOGICAL * 0.5).max(TOOLBAR_COMMAND_SIZE_LOGICAL)
    }

    fn command_key_at(&self, px: f32, py: f32, dpi: f32) -> Option<String> {
        let (visible, _) = self.visible_command_keys(self.rect.w / dpi);
        let commands = self.commands();
        let mut left = self.rect.x + TOOLBAR_HORIZONTAL_PADDING_LOGICAL * dpi;
        for key in visible {
            let command = commands.iter().find(|command| command.command_key == key)?;
            let width = self.command_width(command) * dpi;
            if Rect::new(left, self.rect.y, width, self.rect.h).contains(px, py) {
                return Some(key);
            }
            left += width + TOOLBAR_COMMAND_GAP_LOGICAL * dpi;
        }
        None
    }

    fn overflow_rect(&self, dpi: f32) -> Rect {
        let (visible, overflow) = self.visible_command_keys(self.rect.w / dpi);
        if overflow.is_empty() {
            return Rect::ZERO;
        }
        let commands = self.commands();
        let visible_width = visible
            .iter()
            .filter_map(|key| commands.iter().find(|command| command.command_key == *key))
            .map(|command| self.command_width(command) + TOOLBAR_COMMAND_GAP_LOGICAL)
            .sum::<f32>()
            * dpi;
        Rect::new(
            self.rect.x + TOOLBAR_HORIZONTAL_PADDING_LOGICAL * dpi + visible_width,
            self.rect.y,
            TOOLBAR_COMMAND_SIZE_LOGICAL * dpi,
            self.rect.h,
        )
    }
}

impl Default for EditorToolbarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for EditorToolbarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        self.dpi = ctx.dpi;
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (visible, overflow) = self.visible_command_keys(self.rect.w / ctx.dpi);
        let commands = self.commands();
        let mut left = self.rect.x + TOOLBAR_HORIZONTAL_PADDING_LOGICAL * ctx.dpi;
        for key in visible {
            let Some(command) = commands.iter().find(|command| command.command_key == key) else {
                continue;
            };
            let width = self.command_width(command) * ctx.dpi;
            let button_rect = Rect::new(left, self.rect.y, width, self.rect.h);
            if self.hovered_command_key.as_deref() == Some(command.command_key.as_str())
                && command.enabled
            {
                ctx.list.fill_rounded(
                    button_rect,
                    ctx.theme.palette.bg_hover,
                    TOOLBAR_CORNER_RADIUS_LOGICAL * ctx.dpi,
                );
            }
            let mut color = ctx.theme.palette.text_muted;
            if !command.enabled {
                color[3] *= 0.45;
            }
            if let Some(icon) = toolbar_icon(&command.command_key) {
                let icon_size = TOOLBAR_ICON_SIZE_LOGICAL * ctx.dpi;
                draw_icon(
                    ctx.list,
                    icon,
                    button_rect.x + (button_rect.w - icon_size) * 0.5,
                    button_rect.y + (button_rect.h - icon_size) * 0.5,
                    icon_size,
                    color,
                );
            } else {
                ctx.text(
                    button_rect.x + 8.0 * ctx.dpi,
                    button_rect.y
                        + button_rect.h * 0.5
                        + TOOLBAR_FONT_SIZE_LOGICAL * ctx.dpi * 0.35,
                    TOOLBAR_FONT_SIZE_LOGICAL * ctx.dpi,
                    color,
                    &command.label,
                );
            }
            left += width + TOOLBAR_COMMAND_GAP_LOGICAL * ctx.dpi;
        }
        if !overflow.is_empty() {
            let overflow_rect = self.overflow_rect(ctx.dpi);
            if self.overflow_hovered {
                ctx.list.fill_rounded(
                    overflow_rect,
                    ctx.theme.palette.bg_hover,
                    TOOLBAR_CORNER_RADIUS_LOGICAL * ctx.dpi,
                );
            }
            let icon_size = TOOLBAR_ICON_SIZE_LOGICAL * ctx.dpi;
            draw_icon(
                ctx.list,
                "ellipsis",
                overflow_rect.x + (overflow_rect.w - icon_size) * 0.5,
                overflow_rect.y + (overflow_rect.h - icon_size) * 0.5,
                icon_size,
                ctx.theme.palette.text_muted,
            );
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let (visible, overflow) = self.visible_command_keys(self.rect.w / self.dpi);
        let commands = self.commands();
        let mut left = self.rect.x + TOOLBAR_HORIZONTAL_PADDING_LOGICAL * self.dpi;
        let mut root = AccessibilityNode::new(
            EDITOR_TOOLBAR_ACCESSIBILITY_ID,
            AccessibilityRole::Toolbar,
            ctx.screen_bounds(self.rect),
        )
        .with_name("编辑器工具栏");
        for key in visible {
            let Some(command) = commands.iter().find(|command| command.command_key == key) else {
                continue;
            };
            let width = self.command_width(command) * self.dpi;
            let mut child = AccessibilityNode::new(
                EDITOR_TOOLBAR_ACCESSIBILITY_ID.named_child(&command.command_key),
                AccessibilityRole::Button,
                ctx.screen_bounds(Rect::new(left, self.rect.y, width, self.rect.h)),
            )
            .with_name(command.label.clone())
            .with_disabled(!command.enabled);
            if command.enabled {
                child = child.with_action(AccessibilityAction::Activate);
            }
            root.children.push(child);
            left += width + TOOLBAR_COMMAND_GAP_LOGICAL * self.dpi;
        }
        if !overflow.is_empty() {
            root.children.push(
                AccessibilityNode::new(
                    EDITOR_TOOLBAR_ACCESSIBILITY_ID.named_child("overflow"),
                    AccessibilityRole::Button,
                    ctx.screen_bounds(self.overflow_rect(self.dpi)),
                )
                .with_name("更多命令")
                .with_expanded(self.input.overflow_open)
                .with_action(AccessibilityAction::Activate),
            );
        }
        Some(root)
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if request.action != AccessibilityAction::Activate {
            return None;
        }
        let (visible, overflow) = self.visible_command_keys(self.rect.w / self.dpi);
        if !overflow.is_empty()
            && request.target == EDITOR_TOOLBAR_ACCESSIBILITY_ID.named_child("overflow")
        {
            return Some(WidgetAction::Control(ControlAction::Activated {
                id: EDITOR_TOOLBAR_OVERFLOW_ID,
            }));
        }
        let command = self.commands().into_iter().find(|command| {
            command.enabled
                && visible.contains(&command.command_key)
                && request.target
                    == EDITOR_TOOLBAR_ACCESSIBILITY_ID.named_child(&command.command_key)
        })?;
        Some(WidgetAction::Control(ControlAction::TextCommitted {
            id: EDITOR_TOOLBAR_COMMAND_ID,
            value: TextPayload::Plain(command.command_key),
        }))
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match event {
            Event::PointerLeave | Event::InteractionCancel => {
                let hover_changed = self.hovered_command_key.take().is_some()
                    | std::mem::take(&mut self.overflow_hovered);
                hover_changed.then_some(WidgetAction::Consumed)
            }
            Event::KeyDown(crate::core::KeyCode::Escape, _) if self.input.overflow_open => {
                Some(WidgetAction::Control(ControlAction::Activated {
                    id: EDITOR_TOOLBAR_DISMISS_ID,
                }))
            }
            Event::MouseMove { px, py } => {
                let hovered_command_key = self.command_key_at(*px, *py, ctx.dpi);
                let overflow_hovered = self.overflow_rect(ctx.dpi).contains(*px, *py);
                let hover_changed = self.hovered_command_key != hovered_command_key
                    || self.overflow_hovered != overflow_hovered;
                self.hovered_command_key = hovered_command_key;
                self.overflow_hovered = overflow_hovered;
                if self.hovered_command_key.is_some() || self.overflow_hovered {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                hover_changed.then_some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: crate::core::MouseButton::Left } => {
                if !self.rect.contains(*px, *py) {
                    return Some(WidgetAction::Control(ControlAction::Activated {
                        id: EDITOR_TOOLBAR_DISMISS_ID,
                    }));
                }
                if let Some(key) = self.command_key_at(*px, *py, ctx.dpi) {
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
                self.overflow_rect(ctx.dpi).contains(*px, *py).then_some(WidgetAction::Control(
                    ControlAction::Activated { id: EDITOR_TOOLBAR_OVERFLOW_ID },
                ))
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

fn command_row_width(
    commands: &[EditorToolbarCommandInput],
    command_width: impl Fn(&EditorToolbarCommandInput) -> f32,
) -> f32 {
    let commands_width = commands.iter().map(command_width).sum::<f32>();
    let gap_count = commands.len().saturating_sub(1) as f32;
    commands_width + gap_count * TOOLBAR_COMMAND_GAP_LOGICAL
}

fn toolbar_icon(command_key: &str) -> Option<&'static str> {
    match command_key {
        "undo" => Some("undo-2"),
        "redo" => Some("redo-2"),
        "heading" => Some("heading"),
        "bold" => Some("bold"),
        "italic" => Some("italic"),
        "strike" => Some("strikethrough"),
        "inline_code" => Some("code"),
        "unordered_list" => Some("list"),
        "ordered_list" => Some("list-ordered"),
        "task_list" => Some("list-checks"),
        "quote" => Some("quote"),
        "code_block" => Some("square-code"),
        "link" => Some("link"),
        "toggle_source" => Some("eye"),
        "mindmap_style" => Some("palette"),
        "promote" => Some("outdent"),
        "demote" => Some("indent"),
        "delete" => Some("trash-2"),
        _ => None,
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
    fn accessibility_exposes_visible_commands_disabled_state_and_overflow_action() {
        let mut toolbar = toolbar();
        toolbar.input.groups[0].commands[0].enabled = false;
        toolbar.input.overflow_open = true;
        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        toolbar.set_rect(Rect::new(0.0, 0.0, 80.0, 40.0), &mut layout);
        let node = toolbar
            .accessibility_node(&crate::core::AccessibilityContext::new(10.0, 20.0))
            .expect("toolbar should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::Toolbar);
        assert_eq!(node.children.len(), 2);
        assert_eq!(node.children[0].name.as_deref(), Some("撤销"));
        assert!(node.children[0].state.disabled);
        assert!(node.children[0].actions.is_empty());
        assert_eq!(node.children[1].name.as_deref(), Some("更多命令"));
        assert_eq!(node.children[1].state.expanded, Some(true));
        assert_eq!(
            toolbar.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.children[1].id,
                crate::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::Control(ControlAction::Activated {
                id: EDITOR_TOOLBAR_OVERFLOW_ID,
            }))
        );
    }

    #[test]
    fn narrow_toolbar_moves_low_priority_commands_to_overflow_without_changing_keys() {
        let toolbar = toolbar();
        assert_eq!(
            toolbar.visible_command_keys(48.0),
            (vec!["undo".to_owned()], vec!["link".to_owned()])
        );
    }

    #[test]
    fn icon_commands_use_compact_fixed_width_buttons() {
        let mut toolbar = EditorToolbarWidget::new();
        toolbar.set_input(EditorToolbarInput {
            groups: vec![EditorToolbarGroupInput {
                label: "编辑".to_owned(),
                commands: ["undo", "redo", "promote", "demote"]
                    .into_iter()
                    .map(|command_key| EditorToolbarCommandInput {
                        command_key: command_key.to_owned(),
                        label: command_key.to_owned(),
                        enabled: true,
                        overflow_priority: 0,
                    })
                    .collect(),
            }],
            overflow_open: false,
        });

        for command in toolbar.commands() {
            assert_eq!(toolbar.command_width(&command), 32.0, "{}", command.command_key);
        }
    }

    #[test]
    fn view_and_mindmap_style_commands_use_icons() {
        assert_eq!(toolbar_icon("toggle_source"), Some("eye"));
        assert_eq!(toolbar_icon("mindmap_style"), Some("palette"));
    }

    #[test]
    fn hover_state_changes_emit_an_immediate_redraw_signal() {
        let mut toolbar = toolbar();
        toolbar.rect = Rect::new(0.0, 0.0, 320.0, 40.0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        assert_eq!(
            toolbar.on_event(&Event::MouseMove { px: 20.0, py: 20.0 }, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(toolbar.hovered_command_key.as_deref(), Some("undo"));

        assert_eq!(
            toolbar.on_event(&Event::MouseMove { px: 400.0, py: 20.0 }, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(toolbar.hovered_command_key, None);
    }

    #[test]
    fn toolbar_lifecycle_clears_hover_state_idempotently() {
        let mut toolbar = toolbar();
        toolbar.rect = Rect::new(0.0, 0.0, 320.0, 40.0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        assert_eq!(
            toolbar.on_event(&Event::MouseMove { px: 20.0, py: 20.0 }, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            toolbar.on_event(&Event::PointerLeave, &mut context),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(toolbar.hovered_command_key, None);
        assert!(!toolbar.overflow_hovered);
        assert_eq!(toolbar.on_event(&Event::InteractionCancel, &mut context), None);
    }
}
