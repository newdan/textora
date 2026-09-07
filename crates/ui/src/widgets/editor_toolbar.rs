//! 编辑器通用菜单栏；文档类型命令集合由产品层注入。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget,
    WidgetAction,
};
use crate::widgets::icon::draw_icon;
use crate::widgets::tooltip::TooltipHint;
use std::any::Any;

const TOOLBAR_COMMAND_SIZE_LOGICAL: f32 = 28.0;
const TOOLBAR_COMMAND_GAP_LOGICAL: f32 = 2.0;
const TOOLBAR_GROUP_GAP_LOGICAL: f32 = 12.0;
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

struct ToolbarCommandLayout {
    command_key: String,
    rect: Rect,
}

struct ToolbarLayout {
    commands: Vec<ToolbarCommandLayout>,
    overflow_command_keys: Vec<String>,
    overflow_rect: Option<Rect>,
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
        let layout = self.toolbar_layout(available_width, 1.0);
        (
            layout.commands.into_iter().map(|command| command.command_key).collect(),
            layout.overflow_command_keys,
        )
    }

    fn commands(&self) -> Vec<EditorToolbarCommandInput> {
        self.input.groups.iter().flat_map(|group| group.commands.iter().cloned()).collect()
    }

    fn grouped_commands(&self) -> Vec<(usize, &EditorToolbarCommandInput)> {
        self.input
            .groups
            .iter()
            .enumerate()
            .flat_map(|(group_index, group)| {
                group.commands.iter().map(move |command| (group_index, command))
            })
            .collect()
    }

    fn command_width(&self, command: &EditorToolbarCommandInput) -> f32 {
        if toolbar_icon(&command.command_key).is_some() {
            return TOOLBAR_COMMAND_SIZE_LOGICAL;
        }
        let label_width = command.label.chars().count() as f32 * TOOLBAR_FONT_SIZE_LOGICAL * 0.55;
        (label_width + TOOLBAR_COMMAND_SIZE_LOGICAL * 0.5).max(TOOLBAR_COMMAND_SIZE_LOGICAL)
    }

    fn command_key_at(&self, px: f32, py: f32, dpi: f32) -> Option<String> {
        self.toolbar_layout(self.rect.w, dpi)
            .commands
            .into_iter()
            .find(|command| command.rect.contains(px, py))
            .map(|command| command.command_key)
    }

    fn command_rect(&self, requested_key: &str, dpi: f32) -> Option<Rect> {
        self.toolbar_layout(self.rect.w, dpi)
            .commands
            .into_iter()
            .find(|command| command.command_key == requested_key)
            .map(|command| command.rect)
    }

    fn overflow_rect(&self, dpi: f32) -> Rect {
        self.toolbar_layout(self.rect.w, dpi).overflow_rect.unwrap_or(Rect::ZERO)
    }

    fn toolbar_layout(&self, available_width_px: f32, dpi: f32) -> ToolbarLayout {
        let horizontal_inset = crate::layout::reading_content_inset(available_width_px, dpi);
        let content_width = (available_width_px - horizontal_inset * 2.0).max(0.0);
        let grouped_commands = self.grouped_commands();
        let all_indices = (0..grouped_commands.len()).collect::<Vec<_>>();
        let all_commands_width = self.command_row_width(&grouped_commands, &all_indices, dpi);
        if all_commands_width <= content_width {
            return ToolbarLayout {
                commands: self.position_commands(
                    &grouped_commands,
                    &all_indices,
                    horizontal_inset,
                    dpi,
                ),
                overflow_command_keys: Vec::new(),
                overflow_rect: None,
            };
        }

        let overflow_width = (TOOLBAR_COMMAND_SIZE_LOGICAL * dpi).min(content_width);
        let visible_budget = (content_width - overflow_width).max(0.0);
        let mut candidates = all_indices.clone();
        candidates.sort_by(|left, right| {
            let left_command = grouped_commands[*left].1;
            let right_command = grouped_commands[*right].1;
            left_command
                .overflow_priority
                .cmp(&right_command.overflow_priority)
                .then_with(|| {
                    self.command_width(left_command).total_cmp(&self.command_width(right_command))
                })
                .then_with(|| left.cmp(right))
        });
        let mut visible_indices = Vec::new();
        for candidate in candidates {
            let mut trial_indices = visible_indices.clone();
            trial_indices.push(candidate);
            trial_indices.sort_unstable();
            let commands_width = self.command_row_width(&grouped_commands, &trial_indices, dpi);
            let overflow_gap =
                if trial_indices.is_empty() { 0.0 } else { TOOLBAR_COMMAND_GAP_LOGICAL * dpi };
            if commands_width + overflow_gap <= visible_budget {
                visible_indices = trial_indices;
            }
        }

        let commands =
            self.position_commands(&grouped_commands, &visible_indices, horizontal_inset, dpi);
        let commands_width = self.command_row_width(&grouped_commands, &visible_indices, dpi);
        let overflow_left = horizontal_inset
            + commands_width
            + if commands.is_empty() { 0.0 } else { TOOLBAR_COMMAND_GAP_LOGICAL * dpi };
        let overflow_command_keys = all_indices
            .into_iter()
            .filter(|index| !visible_indices.contains(index))
            .map(|index| grouped_commands[index].1.command_key.clone())
            .collect();

        ToolbarLayout {
            commands,
            overflow_command_keys,
            overflow_rect: Some(Rect::new(overflow_left, self.rect.y, overflow_width, self.rect.h)),
        }
    }

    fn position_commands(
        &self,
        grouped_commands: &[(usize, &EditorToolbarCommandInput)],
        visible_indices: &[usize],
        horizontal_inset: f32,
        dpi: f32,
    ) -> Vec<ToolbarCommandLayout> {
        let mut left = horizontal_inset;
        let mut previous_group = None;
        visible_indices
            .iter()
            .map(|index| {
                let (group_index, command) = grouped_commands[*index];
                if let Some(previous_group) = previous_group {
                    let gap = if previous_group == group_index {
                        TOOLBAR_COMMAND_GAP_LOGICAL
                    } else {
                        TOOLBAR_GROUP_GAP_LOGICAL
                    };
                    left += gap * dpi;
                }
                let width = self.command_width(command) * dpi;
                let layout = ToolbarCommandLayout {
                    command_key: command.command_key.clone(),
                    rect: Rect::new(left, self.rect.y, width, self.rect.h),
                };
                left += width;
                previous_group = Some(group_index);
                layout
            })
            .collect()
    }

    fn command_row_width(
        &self,
        grouped_commands: &[(usize, &EditorToolbarCommandInput)],
        visible_indices: &[usize],
        dpi: f32,
    ) -> f32 {
        let commands_width = visible_indices
            .iter()
            .map(|index| self.command_width(grouped_commands[*index].1) * dpi)
            .sum::<f32>();
        let gaps_width = visible_indices
            .windows(2)
            .map(|indices| {
                if grouped_commands[indices[0]].0 == grouped_commands[indices[1]].0 {
                    TOOLBAR_COMMAND_GAP_LOGICAL
                } else {
                    TOOLBAR_GROUP_GAP_LOGICAL
                }
            })
            .sum::<f32>()
            * dpi;
        commands_width + gaps_width
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
        let layout = self.toolbar_layout(self.rect.w, ctx.dpi);
        let commands = self.commands();
        for command_layout in &layout.commands {
            let Some(command) =
                commands.iter().find(|command| command.command_key == command_layout.command_key)
            else {
                continue;
            };
            let button_rect = command_layout.rect;
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
        }
        if let Some(overflow_rect) = layout.overflow_rect {
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
        let divider_thickness = ctx.dpi.max(1.0);
        ctx.list.fill(
            Rect::new(
                self.rect.x,
                self.rect.bottom() - divider_thickness,
                self.rect.w,
                divider_thickness,
            ),
            ctx.theme.palette.border_subtle,
        );
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let layout = self.toolbar_layout(self.rect.w, self.dpi);
        let commands = self.commands();
        let mut root = AccessibilityNode::new(
            EDITOR_TOOLBAR_ACCESSIBILITY_ID,
            AccessibilityRole::Toolbar,
            ctx.screen_bounds(self.rect),
        )
        .with_name("编辑器工具栏");
        for command_layout in &layout.commands {
            let Some(command) =
                commands.iter().find(|command| command.command_key == command_layout.command_key)
            else {
                continue;
            };
            let mut child = AccessibilityNode::new(
                EDITOR_TOOLBAR_ACCESSIBILITY_ID.named_child(&command.command_key),
                AccessibilityRole::Button,
                ctx.screen_bounds(command_layout.rect),
            )
            .with_name(command.label.clone())
            .with_disabled(!command.enabled);
            if command.enabled {
                child = child.with_action(AccessibilityAction::Activate);
            }
            root.children.push(child);
        }
        if let Some(overflow_rect) = layout.overflow_rect {
            root.children.push(
                AccessibilityNode::new(
                    EDITOR_TOOLBAR_ACCESSIBILITY_ID.named_child("overflow"),
                    AccessibilityRole::Button,
                    ctx.screen_bounds(overflow_rect),
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
        let layout = self.toolbar_layout(self.rect.w, self.dpi);
        if layout.overflow_rect.is_some()
            && request.target == EDITOR_TOOLBAR_ACCESSIBILITY_ID.named_child("overflow")
        {
            return Some(WidgetAction::Control(ControlAction::Activated {
                id: EDITOR_TOOLBAR_OVERFLOW_ID,
            }));
        }
        let command = self.commands().into_iter().find(|command| {
            command.enabled
                && layout.commands.iter().any(|layout| layout.command_key == command.command_key)
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

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        if let Some(command_key) = self.command_key_at(px, py, self.dpi) {
            let command = self.commands().into_iter().find(|command| {
                command.command_key == command_key && toolbar_icon(&command.command_key).is_some()
            })?;
            let target_rect = self.command_rect(&command_key, self.dpi)?;
            return Some(TooltipHint { label: command.label, target_rect });
        }
        let overflow_rect = self.overflow_rect(self.dpi);
        overflow_rect
            .contains(px, py)
            .then_some(TooltipHint { label: "更多命令".to_owned(), target_rect: overflow_rect })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
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
        "canvas_zoom_out" => Some("minus"),
        "canvas_zoom_in" => Some("plus"),
        "canvas_fit" => Some("maximize"),
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
                    EditorToolbarCommandInput {
                        command_key: "custom_action".to_owned(),
                        label: "自定义动作".to_owned(),
                        enabled: true,
                        overflow_priority: 20,
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
        toolbar.set_rect(Rect::new(0.0, 0.0, 124.0, 40.0), &mut layout);
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
            (Vec::new(), vec!["undo".to_owned(), "link".to_owned(), "custom_action".to_owned()])
        );
    }

    #[test]
    fn narrow_toolbar_keeps_overflow_button_inside_content_bounds_when_priorities_are_zero() {
        let mut toolbar = EditorToolbarWidget::new();
        toolbar.set_input(EditorToolbarInput {
            groups: vec![
                EditorToolbarGroupInput {
                    label: "编辑".to_owned(),
                    commands: ["undo", "redo"]
                        .into_iter()
                        .map(|command_key| EditorToolbarCommandInput {
                            command_key: command_key.to_owned(),
                            label: command_key.to_owned(),
                            enabled: true,
                            overflow_priority: 0,
                        })
                        .collect(),
                },
                EditorToolbarGroupInput {
                    label: "格式".to_owned(),
                    commands: ["bold", "italic"]
                        .into_iter()
                        .map(|command_key| EditorToolbarCommandInput {
                            command_key: command_key.to_owned(),
                            label: command_key.to_owned(),
                            enabled: true,
                            overflow_priority: 0,
                        })
                        .collect(),
                },
            ],
            overflow_open: false,
        });
        toolbar.rect = Rect::new(0.0, 0.0, 132.0, 36.0);

        let (_, overflow) = toolbar.visible_command_keys(toolbar.rect.w);
        let overflow_rect = toolbar.overflow_rect(1.0);

        assert!(!overflow.is_empty(), "窄栏必须提供更多菜单以访问被隐藏的命令");
        assert!(
            overflow_rect.right() <= toolbar.rect.right() - 32.0,
            "更多按钮不能越过工具栏右侧内容边界"
        );
    }

    #[test]
    fn icon_commands_and_overflow_expose_tooltips() {
        let mut toolbar = toolbar();
        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        toolbar.set_rect(Rect::new(0.0, 0.0, 124.0, 40.0), &mut layout);

        assert_eq!(toolbar.tooltip_at(40.0, 20.0).map(|hint| hint.label), Some("撤销".to_owned()));
        let overflow_rect = toolbar.overflow_rect(1.0);
        assert_eq!(
            toolbar
                .tooltip_at(
                    overflow_rect.x + overflow_rect.w * 0.5,
                    overflow_rect.y + overflow_rect.h * 0.5,
                )
                .map(|hint| hint.label),
            Some("更多命令".to_owned())
        );
    }

    #[test]
    fn commands_in_different_groups_receive_distinct_group_spacing() {
        let mut toolbar = EditorToolbarWidget::new();
        toolbar.set_input(EditorToolbarInput {
            groups: ["undo", "bold"]
                .into_iter()
                .map(|command_key| EditorToolbarGroupInput {
                    label: command_key.to_owned(),
                    commands: vec![EditorToolbarCommandInput {
                        command_key: command_key.to_owned(),
                        label: command_key.to_owned(),
                        enabled: true,
                        overflow_priority: 0,
                    }],
                })
                .collect(),
            overflow_open: false,
        });
        toolbar.rect = Rect::new(0.0, 0.0, 200.0, 36.0);

        let undo_rect = toolbar.command_rect("undo", 1.0).expect("undo should remain visible");
        let bold_rect = toolbar.command_rect("bold", 1.0).expect("bold should remain visible");

        assert_eq!(bold_rect.x - undo_rect.right(), 12.0);
    }

    #[test]
    fn view_and_mindmap_style_commands_use_icons() {
        assert_eq!(toolbar_icon("toggle_source"), Some("eye"));
        assert_eq!(toolbar_icon("mindmap_style"), Some("palette"));
        assert_eq!(toolbar_icon("canvas_zoom_out"), Some("minus"));
        assert_eq!(toolbar_icon("canvas_zoom_in"), Some("plus"));
        assert_eq!(toolbar_icon("canvas_fit"), Some("maximize"));
    }

    #[test]
    fn hover_state_changes_emit_an_immediate_redraw_signal() {
        let mut toolbar = toolbar();
        toolbar.rect = Rect::new(0.0, 0.0, 320.0, 40.0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);

        assert_eq!(
            toolbar.on_event(&Event::MouseMove { px: 40.0, py: 20.0 }, &mut context),
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
        let mut context = EventCtx::new(&theme, 1.0);

        assert_eq!(
            toolbar.on_event(&Event::MouseMove { px: 40.0, py: 20.0 }, &mut context),
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
