//! 与产品领域无关的分层树列表。

mod layout;

use std::any::Any;

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, DrawCmd, Event, EventCtx, KeyCode, LayoutCtx,
    MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::widgets::icon::draw_icon;
use crate::widgets::text_box::{TextBox, TextBoxChrome};
use crate::widgets::tooltip::TooltipHint;

use self::layout::{TreeListLayout, build_tree_layout};

const TREE_LIST_EDITOR_WIDGET_ID_SALT: u64 = 0x7472_6565_6564_6974;

/// 仅在单帧 UI 输入中有效的树行键。
///
/// 产品层负责将此键映射到自己的领域动作；键不承载路径或领域 ID 语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeRowKey(pub u64);

/// 仅在单帧 UI 输入中有效的树行动作键。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeRowActionKey(pub u64);

/// 与产品领域无关的树行尾动作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRowActionInput {
    pub key: TreeRowActionKey,
    pub icon: String,
    pub tooltip: String,
    pub accessibility_label: String,
    pub enabled: bool,
}

impl TreeRowActionInput {
    pub fn enabled(key: u64, icon: impl Into<String>, label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            key: TreeRowActionKey(key),
            icon: icon.into(),
            tooltip: label.clone(),
            accessibility_label: label,
            enabled: true,
        }
    }
}

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
    pub tooltip: Option<String>,
    pub trailing_actions: Vec<TreeRowActionInput>,
}

/// 插在父节点第一行子节点位置的单个行内编辑器。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRowEditorInput {
    pub key: TreeRowKey,
    pub parent_key: TreeRowKey,
    pub depth: usize,
    pub value: String,
    pub placeholder: String,
}

/// 树列表的每帧输入。滚动和选择均由产品层独立持有。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeListInput {
    pub rows: Vec<TreeRowInput>,
    pub editor: Option<TreeRowEditorInput>,
    pub scroll_offset_px: f32,
}

/// 树列表向调用方返回的通用 UI 动作。
#[derive(Clone, Debug, PartialEq)]
pub enum TreeListAction {
    Selected(TreeRowKey),
    ExpansionToggled(TreeRowKey),
    ScrollOffsetChanged(f32),
    HoverChanged(Option<TreeRowKey>),
    TrailingActionActivated { row_key: TreeRowKey, action_key: TreeRowActionKey },
    EditorTextChanged { key: TreeRowKey, value: String },
    EditorCommitRequested { key: TreeRowKey, value: String },
    EditorCancelled { key: TreeRowKey },
}

/// 分层树列表组件。
pub struct TreeListWidget {
    id: Option<WidgetId>,
    rect: Rect,
    input: TreeListInput,
    layout: TreeListLayout,
    selected_key: Option<TreeRowKey>,
    hovered_key: Option<TreeRowKey>,
    hovered_action: Option<(TreeRowKey, TreeRowActionKey)>,
    pressed_action: Option<(TreeRowKey, TreeRowActionKey)>,
    inline_editor: TextBox,
    inline_editor_id: WidgetId,
    focused: bool,
    accessibility_label: Option<String>,
}

impl Default for TreeListWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeListWidget {
    pub fn new() -> Self {
        let inline_editor_id = WidgetId(TREE_LIST_EDITOR_WIDGET_ID_SALT);
        let mut inline_editor = TextBox::with_id(inline_editor_id);
        inline_editor.set_chrome(TextBoxChrome::Seamless);
        inline_editor.set_accessibility_label(Some("新目录名称".to_owned()));
        inline_editor.set_font_size_logical(layout::TREE_ROW_FONT_SIZE_LOGICAL);
        Self {
            id: None,
            rect: Rect::ZERO,
            input: TreeListInput::default(),
            layout: TreeListLayout::default(),
            selected_key: None,
            hovered_key: None,
            hovered_action: None,
            pressed_action: None,
            inline_editor,
            inline_editor_id,
            focused: false,
            accessibility_label: None,
        }
    }

    pub fn with_id(mut self, id: WidgetId) -> Self {
        self.id = Some(id);
        self.inline_editor_id = WidgetId(id.0 ^ TREE_LIST_EDITOR_WIDGET_ID_SALT);
        self.inline_editor = TextBox::with_id(self.inline_editor_id);
        self.inline_editor.set_chrome(TextBoxChrome::Seamless);
        self.inline_editor.set_accessibility_label(Some("新目录名称".to_owned()));
        self.inline_editor.set_font_size_logical(layout::TREE_ROW_FONT_SIZE_LOGICAL);
        self
    }

    pub fn set_accessibility_label(&mut self, label: Option<String>) {
        self.accessibility_label = label;
    }

    /// 覆盖当前帧展示输入，并丢弃已不存在行的悬停状态。
    pub fn set_input(&mut self, mut input: TreeListInput) {
        debug_assert!(has_unique_keys(&input.rows), "tree row keys must be unique per frame");
        debug_assert!(
            input.editor.as_ref().is_none_or(|editor| {
                input.rows.iter().all(|row| row.key != editor.key)
                    && input.rows.iter().any(|row| row.key == editor.parent_key)
            }),
            "tree editor key must be unique and its parent must exist"
        );
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
        self.hovered_action = self.hovered_action.filter(|(row_key, action_key)| {
            input.rows.iter().any(|row| {
                row.key == *row_key
                    && row.trailing_actions.iter().any(|action| action.key == *action_key)
            })
        });
        self.pressed_action = self.pressed_action.filter(|(row_key, action_key)| {
            input.rows.iter().any(|row| {
                row.key == *row_key
                    && row
                        .trailing_actions
                        .iter()
                        .any(|action| action.key == *action_key && action.enabled)
            })
        });
        if let Some(editor) = &input.editor {
            if self.inline_editor.text() != editor.value {
                self.inline_editor.set_text(&editor.value);
            }
            self.inline_editor.set_placeholder(&editor.placeholder);
            self.inline_editor.set_focus(self.focused);
        } else {
            self.inline_editor.set_focus(false);
        }
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

    pub fn hovered_action(&self) -> Option<(TreeRowKey, TreeRowActionKey)> {
        self.hovered_action
    }

    pub fn ime_cursor_rect(&self) -> Option<Rect> {
        self.input.editor.as_ref().map(|_| self.inline_editor.ime_cursor_rect())
    }

    pub fn set_editor_blink(&mut self, visible: bool) {
        self.inline_editor.set_blink(visible);
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

    fn action_at(&self, px: f32, py: f32) -> Option<(TreeRowKey, TreeRowActionKey, bool)> {
        let row_index = self.row_at(px, py)?;
        let row = self.input.rows.get(row_index)?;
        let row_layout = self.layout.rows.get(row_index)?;
        row.trailing_actions
            .iter()
            .zip(&row_layout.action_rects)
            .find(|(_, action_rect)| action_rect.contains(px, py))
            .map(|(action, _)| (row.key, action.key, action.enabled))
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

    fn translate_editor_action(&mut self, action: WidgetAction) -> Option<WidgetAction> {
        let editor_key = self.input.editor.as_ref()?.key;
        match action {
            WidgetAction::Control(ControlAction::TextEdited {
                id,
                value: TextPayload::Plain(value),
            }) if id == self.inline_editor_id => {
                Some(WidgetAction::TreeList(TreeListAction::EditorTextChanged {
                    key: editor_key,
                    value,
                }))
            }
            WidgetAction::Control(ControlAction::TextCommitted {
                id,
                value: TextPayload::Plain(value),
            }) if id == self.inline_editor_id => {
                Some(WidgetAction::TreeList(TreeListAction::EditorCommitRequested {
                    key: editor_key,
                    value,
                }))
            }
            WidgetAction::Control(ControlAction::FocusRequested { id })
                if id == self.inline_editor_id =>
            {
                self.inline_editor.set_focus(true);
                Some(WidgetAction::Consumed)
            }
            WidgetAction::Consumed => Some(WidgetAction::Consumed),
            _ => None,
        }
    }

    fn commit_or_cancel_editor(&self) -> Option<TreeListAction> {
        let editor = self.input.editor.as_ref()?;
        let value = self.inline_editor.text().to_owned();
        if value.trim().is_empty() {
            return Some(TreeListAction::EditorCancelled { key: editor.key });
        }
        Some(TreeListAction::EditorCommitRequested { key: editor.key, value })
    }
}

impl Widget for TreeListWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.layout = build_tree_layout(
            &self.input.rows,
            self.input.editor.as_ref(),
            rect,
            self.input.scroll_offset_px,
            ctx.dpi,
        );
        if let Some(editor_layout) = &self.layout.editor {
            self.inline_editor.set_rect(editor_layout.text_box_rect, ctx);
        }
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
            if row_layout.label_rect.w > 0.0 {
                let label_clip = Rect::new(
                    row_layout.label_rect.x + ctx.list.offset.0,
                    row_layout.label_rect.y + ctx.list.offset.1,
                    row_layout.label_rect.w,
                    row_layout.label_rect.h,
                );
                ctx.list.cmds.push(DrawCmd::PushClip(label_clip));
                ctx.text(row_layout.label_rect.x, baseline, font_size, text_color, &row.label);
                ctx.list.cmds.push(DrawCmd::PopClip);
            }

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

            if is_hovered {
                for (action, action_rect) in
                    row.trailing_actions.iter().zip(&row_layout.action_rects)
                {
                    let action_identity = (row.key, action.key);
                    if self.hovered_action == Some(action_identity)
                        || self.pressed_action == Some(action_identity)
                    {
                        ctx.list.fill_rounded(
                            *action_rect,
                            ctx.theme.palette.sidebar_active_bg,
                            4.0 * ctx.dpi,
                        );
                    }
                    let mut color = ctx.theme.palette.text_muted;
                    if !action.enabled {
                        color[3] *= 0.4;
                    }
                    let icon_size =
                        (layout::TREE_ACTION_ICON_SIZE_LOGICAL * ctx.dpi).min(action_rect.w);
                    draw_icon(
                        ctx.list,
                        &action.icon,
                        action_rect.x + (action_rect.w - icon_size) * 0.5,
                        action_rect.y + (action_rect.h - icon_size) * 0.5,
                        icon_size,
                        color,
                    );
                }
            }
        }

        if let Some(editor_layout) = &self.layout.editor
            && editor_layout.row_rect.bottom() > self.rect.top()
            && editor_layout.row_rect.top() < self.rect.bottom()
        {
            self.inline_editor.paint(ctx);
        }

        ctx.list.cmds.push(DrawCmd::PopClip);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        self.id
    }

    fn is_focusable(&self) -> bool {
        self.id.is_some()
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused = self.id.is_some_and(|id| focused_id == Some(id));
        self.inline_editor.set_keyboard_focus(
            (self.focused && self.input.editor.is_some()).then_some(self.inline_editor_id),
        );
        self.inline_editor.set_blink(self.focused && self.input.editor.is_some());
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let id = self.id?;
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let root_id = AccessibilityId::from(id);
        let mut root =
            AccessibilityNode::new(root_id, AccessibilityRole::Tree, ctx.screen_bounds(self.rect))
                .with_name(self.accessibility_label.as_deref().unwrap_or("树列表"))
                .with_focused(self.focused)
                .with_action(AccessibilityAction::Focus);
        for (row, row_layout) in self.input.rows.iter().zip(&self.layout.rows) {
            if row_layout.row_rect.bottom() <= self.rect.top()
                || row_layout.row_rect.top() >= self.rect.bottom()
            {
                continue;
            }
            let mut child = AccessibilityNode::new(
                root_id.child(row.key.0),
                AccessibilityRole::TreeItem,
                ctx.screen_bounds(row_layout.row_rect),
            )
            .with_name(row.label.clone())
            .with_selected(row.selection == TreeRowSelection::Selected)
            .with_action(AccessibilityAction::Activate);
            if row.expansion != TreeRowExpansion::Leaf {
                child = child
                    .with_expanded(row.expansion == TreeRowExpansion::Expanded)
                    .with_action(AccessibilityAction::Toggle);
            }
            for (action, action_rect) in row.trailing_actions.iter().zip(&row_layout.action_rects) {
                let action_node = AccessibilityNode::new(
                    root_id.child(row.key.0).child(action.key.0),
                    AccessibilityRole::Button,
                    ctx.screen_bounds(*action_rect),
                )
                .with_name(action.accessibility_label.clone())
                .with_disabled(!action.enabled)
                .with_action(AccessibilityAction::Activate);
                child.children.push(action_node);
            }
            root.children.push(child);
            if self.input.editor.as_ref().is_some_and(|editor| editor.parent_key == row.key)
                && let Some(editor_node) = self.inline_editor.accessibility_node(ctx)
            {
                root.children.push(editor_node);
            }
        }
        Some(root)
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        let id = self.id?;
        let root_id = AccessibilityId::from(id);
        if request.target == root_id && request.action == AccessibilityAction::Focus {
            return Some(WidgetAction::Control(ControlAction::FocusRequested { id }));
        }
        if self.input.editor.is_some()
            && let Some(action) = self.inline_editor.on_accessibility_action(request)
        {
            return self.translate_editor_action(action);
        }
        for row in &self.input.rows {
            for action in &row.trailing_actions {
                if request.target == root_id.child(row.key.0).child(action.key.0)
                    && request.action == AccessibilityAction::Activate
                    && action.enabled
                {
                    return Some(WidgetAction::TreeList(TreeListAction::TrailingActionActivated {
                        row_key: row.key,
                        action_key: action.key,
                    }));
                }
            }
        }
        let row = self.input.rows.iter().find(|row| root_id.child(row.key.0) == request.target)?;
        let key = row.key;
        match request.action {
            AccessibilityAction::Activate => {
                self.select_row(key);
                Some(WidgetAction::TreeList(TreeListAction::Selected(key)))
            }
            AccessibilityAction::Toggle if row.expansion != TreeRowExpansion::Leaf => {
                Some(WidgetAction::TreeList(TreeListAction::ExpansionToggled(key)))
            }
            _ => None,
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if self.id.is_some() && !self.focused && matches!(event, Event::KeyDown(..)) {
            return None;
        }
        if let Some(editor) = self.input.editor.as_ref() {
            if matches!(event, Event::KeyDown(KeyCode::Escape, _)) {
                return Some(WidgetAction::TreeList(TreeListAction::EditorCancelled {
                    key: editor.key,
                }));
            }
            let pointer_inside_editor = match (event, self.layout.editor.as_ref()) {
                (Event::MouseDown { px, py, .. }, Some(editor_layout))
                | (Event::MouseUp { px, py, .. }, Some(editor_layout))
                | (Event::MouseMove { px, py }, Some(editor_layout)) => {
                    editor_layout.row_rect.contains(*px, *py)
                }
                _ => false,
            };
            let keyboard_or_ime = matches!(
                event,
                Event::KeyDown(..)
                    | Event::ImePreedit { .. }
                    | Event::ImeCommit(_)
                    | Event::ImeEnable
                    | Event::ImeDisable
            );
            if pointer_inside_editor || keyboard_or_ime || self.inline_editor.is_capturing() {
                if let Some(action) = self.inline_editor.on_event(event, ctx) {
                    return self.translate_editor_action(action);
                }
                if pointer_inside_editor {
                    return Some(WidgetAction::Consumed);
                }
            }
            if matches!(event, Event::MouseDown { button: MouseButton::Left, .. }) {
                return self.commit_or_cancel_editor().map(WidgetAction::TreeList);
            }
        }
        let action = match event {
            Event::MouseMove { px, py } => {
                if self.hit(*px, *py) {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                let previous_action = self.hovered_action;
                self.hovered_action =
                    self.action_at(*px, *py).map(|(row_key, action_key, _)| (row_key, action_key));
                let row_action = self.update_hover(*px, *py);
                if row_action.is_none() && previous_action != self.hovered_action {
                    return Some(WidgetAction::Consumed);
                }
                row_action
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if let Some((row_key, action_key, enabled)) = self.action_at(*px, *py) {
                    self.pressed_action = enabled.then_some((row_key, action_key));
                    return Some(WidgetAction::Consumed);
                }
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
            Event::MouseUp { px, py, button: MouseButton::Left } => {
                let pressed_action = self.pressed_action.take()?;
                match self.action_at(*px, *py) {
                    Some((row_key, action_key, true))
                        if pressed_action == (row_key, action_key) =>
                    {
                        Some(TreeListAction::TrailingActionActivated { row_key, action_key })
                    }
                    _ => return Some(WidgetAction::Consumed),
                }
            }
            Event::PointerLeave | Event::InteractionCancel => {
                let previous_hovered_key = self.hovered_key.take();
                let transient_state_changed =
                    self.hovered_action.take().is_some() | self.pressed_action.take().is_some();
                if previous_hovered_key.is_some() {
                    Some(TreeListAction::HoverChanged(None))
                } else if transient_state_changed {
                    return Some(WidgetAction::Consumed);
                } else {
                    return None;
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

    fn is_capturing(&self) -> bool {
        self.pressed_action.is_some() || self.inline_editor.is_capturing()
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        let row_index = self.row_at(px, py)?;
        let row = self.input.rows.get(row_index)?;
        let row_layout = self.layout.rows.get(row_index)?;
        if let Some((action, action_rect)) = row
            .trailing_actions
            .iter()
            .zip(&row_layout.action_rects)
            .find(|(_, action_rect)| action_rect.contains(px, py))
        {
            return Some(TooltipHint { label: action.tooltip.clone(), target_rect: *action_rect });
        }
        row.tooltip.as_ref().filter(|_| row_layout.label_rect.contains(px, py)).map(|tooltip| {
            TooltipHint { label: tooltip.clone(), target_rect: row_layout.label_rect }
        })
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
            tooltip: None,
            trailing_actions: Vec::new(),
        }
    }

    fn action(key: u64, enabled: bool) -> TreeRowActionInput {
        TreeRowActionInput {
            key: TreeRowActionKey(key),
            icon: "plus".to_owned(),
            tooltip: format!("Action {key}"),
            accessibility_label: format!("Activate action {key}"),
            enabled,
        }
    }

    fn layout(widget: &mut TreeListWidget, rect: Rect, dpi: f32) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut context = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
        widget.set_rect(rect, &mut context);
    }

    fn event_context(theme: &crate::Theme) -> EventCtx<'_> {
        EventCtx::new(theme, 1.0)
    }

    #[test]
    fn accessibility_exposes_tree_rows_and_expansion_action() {
        let id = crate::WidgetId(70);
        let mut widget = TreeListWidget::new().with_id(id);
        widget.set_accessibility_label(Some("工作区文件".into()));
        let mut expanded = row(1, 0, TreeRowExpansion::Expanded);
        expanded.selection = TreeRowSelection::Selected;
        widget.set_input(TreeListInput {
            rows: vec![expanded, row(2, 1, TreeRowExpansion::Leaf)],
            editor: None,
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 240.0, 80.0), 1.0);
        let theme = crate::theme::test_theme();
        assert_eq!(
            widget.on_event(
                &Event::KeyDown(KeyCode::Down, Modifiers::NONE),
                &mut event_context(&theme),
            ),
            None
        );
        widget.set_keyboard_focus(Some(id));
        let node = widget
            .accessibility_node(&crate::core::AccessibilityContext::new(10.0, 20.0))
            .expect("identified tree should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::Tree);
        assert_eq!(node.name.as_deref(), Some("工作区文件"));
        assert!(node.state.focused);
        assert_eq!(node.children.len(), 2);
        assert_eq!(node.children[0].role, crate::core::AccessibilityRole::TreeItem);
        assert_eq!(node.children[0].state.selected, Some(true));
        assert_eq!(node.children[0].state.expanded, Some(true));
        assert_eq!(
            widget.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.children[0].id,
                crate::core::AccessibilityAction::Toggle,
            )),
            Some(WidgetAction::TreeList(TreeListAction::ExpansionToggled(TreeRowKey(1))))
        );
    }

    #[test]
    fn preserves_stable_selection_when_rows_are_replaced() {
        let mut widget = TreeListWidget::new();
        let mut selected = row(7, 0, TreeRowExpansion::Leaf);
        selected.selection = TreeRowSelection::Selected;
        widget.set_input(TreeListInput {
            rows: vec![row(3, 0, TreeRowExpansion::Leaf), selected],
            editor: None,
            scroll_offset_px: 0.0,
        });
        widget.set_input(TreeListInput {
            rows: vec![row(9, 0, TreeRowExpansion::Leaf), row(7, 0, TreeRowExpansion::Leaf)],
            editor: None,
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
        widget.set_input(TreeListInput {
            rows: vec![deep_row],
            editor: None,
            scroll_offset_px: 0.0,
        });
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
            editor: None,
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
            editor: None,
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

    #[test]
    fn trailing_action_click_takes_priority_over_row_selection() {
        let mut widget = TreeListWidget::new();
        let mut input_row = row(1, 0, TreeRowExpansion::Collapsed);
        input_row.trailing_actions = vec![action(9, true)];
        widget.set_input(TreeListInput {
            rows: vec![input_row],
            editor: None,
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 240.0, 80.0), 1.0);
        let action_rect = widget.layout().rows[0].action_rects[0];
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);
        let px = action_rect.x + action_rect.w * 0.5;
        let py = action_rect.y + action_rect.h * 0.5;

        assert_eq!(
            widget.on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context,),
            Some(WidgetAction::Consumed),
        );
        assert_eq!(widget.selected_key(), None);
        assert_eq!(
            widget.on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut context,),
            Some(WidgetAction::TreeList(TreeListAction::TrailingActionActivated {
                row_key: TreeRowKey(1),
                action_key: TreeRowActionKey(9),
            })),
        );
    }

    #[test]
    fn long_row_label_is_clipped_before_trailing_actions_are_painted() {
        let mut widget = TreeListWidget::new();
        let mut input_row = row(1, 0, TreeRowExpansion::Leaf);
        input_row.label = "这是一个远远超过窄侧栏可用宽度的目录名称".repeat(4);
        input_row.selection = TreeRowSelection::Selected;
        input_row.trailing_actions = vec![action(9, true), action(10, true)];
        widget.set_input(TreeListInput {
            rows: vec![input_row],
            editor: None,
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 150.0, 40.0), 1.0);
        let label_rect = widget.layout().rows[0].label_rect;
        let theme = crate::theme::test_theme();
        let mut draw_list = crate::core::DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };

        widget.paint(&mut paint_context);

        assert!(
            draw_list
                .cmds
                .iter()
                .any(|command| matches!(command, DrawCmd::PushClip(rect) if *rect == label_rect))
        );
        assert!(label_rect.right() <= widget.layout().rows[0].action_rects[0].left());
    }

    #[test]
    fn selected_row_trailing_action_is_painted_only_while_hovered() {
        let mut widget = TreeListWidget::new();
        let mut input_row = row(1, 0, TreeRowExpansion::Leaf);
        input_row.icon = None;
        input_row.selection = TreeRowSelection::Selected;
        input_row.trailing_actions = vec![action(9, true)];
        widget.set_input(TreeListInput {
            rows: vec![input_row],
            editor: None,
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 240.0, 80.0), 1.0);
        let theme = crate::theme::test_theme();

        let paint_triangle_count = |widget: &TreeListWidget| {
            let mut draw_list = crate::core::DrawList::new();
            let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
            let mut paint_context = PaintCtx {
                list: &mut draw_list,
                theme: &theme,
                dpi: 1.0,
                offset: (0.0, 0.0),
                global_alpha: 1.0,
                shaper: Some(&mut shaper),
            };
            widget.paint(&mut paint_context);
            draw_list
                .cmds
                .iter()
                .filter(|command| matches!(command, DrawCmd::FillTriangle { .. }))
                .count()
        };

        assert_eq!(paint_triangle_count(&widget), 0);

        let row_rect = widget.layout().rows[0].row_rect;
        widget.on_event(
            &Event::MouseMove { px: row_rect.x + 1.0, py: row_rect.y + 1.0 },
            &mut event_context(&theme),
        );

        assert!(paint_triangle_count(&widget) > 0);
    }

    #[test]
    fn disabled_action_is_consumed_without_activation() {
        let mut widget = TreeListWidget::new();
        let mut input_row = row(1, 0, TreeRowExpansion::Leaf);
        input_row.trailing_actions = vec![action(9, false)];
        widget.set_input(TreeListInput {
            rows: vec![input_row],
            editor: None,
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 240.0, 80.0), 1.0);
        let action_rect = widget.layout().rows[0].action_rects[0];
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);
        let px = action_rect.x + 1.0;
        let py = action_rect.y + 1.0;

        assert_eq!(
            widget.on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context,),
            Some(WidgetAction::Consumed),
        );
        assert_eq!(
            widget.on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut context,),
            None,
        );
        assert_eq!(widget.selected_key(), None);
    }

    #[test]
    fn pointer_leave_clears_hovered_and_pressed_action_state() {
        let mut widget = TreeListWidget::new();
        let mut input_row = row(1, 0, TreeRowExpansion::Leaf);
        input_row.trailing_actions = vec![action(9, true)];
        widget.set_input(TreeListInput {
            rows: vec![input_row],
            editor: None,
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 240.0, 80.0), 1.0);
        let action_rect = widget.layout().rows[0].action_rects[0];
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);
        let px = action_rect.x + 1.0;
        let py = action_rect.y + 1.0;

        widget.on_event(&Event::MouseMove { px, py }, &mut context);
        widget.on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context);
        assert!(widget.hovered_action().is_some());
        assert!(widget.is_capturing());

        assert_eq!(
            widget.on_event(&Event::PointerLeave, &mut context),
            Some(WidgetAction::TreeList(TreeListAction::HoverChanged(None))),
        );
        assert_eq!(widget.hovered_action(), None);
        assert!(!widget.is_capturing());
    }

    #[test]
    fn tooltip_and_accessibility_identify_each_trailing_action() {
        let id = crate::WidgetId(71);
        let mut widget = TreeListWidget::new().with_id(id);
        let mut input_row = row(5, 0, TreeRowExpansion::Leaf);
        input_row.tooltip = Some("/workspace/root".to_owned());
        input_row.trailing_actions = vec![action(2, true), action(3, false)];
        widget.set_input(TreeListInput {
            rows: vec![input_row],
            editor: None,
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 240.0, 80.0), 1.0);

        let first_action_rect = widget.layout().rows[0].action_rects[0];
        assert_eq!(
            widget
                .tooltip_at(first_action_rect.x + 1.0, first_action_rect.y + 1.0)
                .map(|hint| hint.label),
            Some("Action 2".to_owned()),
        );
        let label_rect = widget.layout().rows[0].label_rect;
        assert_eq!(
            widget.tooltip_at(label_rect.x + 1.0, label_rect.y + 1.0).map(|hint| hint.label),
            Some("/workspace/root".to_owned()),
        );

        let root = widget
            .accessibility_node(&crate::core::AccessibilityContext::default())
            .expect("identified tree should expose semantics");
        assert_eq!(root.children[0].children.len(), 2);
        assert_eq!(root.children[0].children[0].role, AccessibilityRole::Button);
        assert_eq!(root.children[0].children[0].name.as_deref(), Some("Activate action 2"));
        assert!(root.children[0].children[1].state.disabled);
        assert_eq!(
            widget.on_accessibility_action(&AccessibilityActionRequest::new(
                root.children[0].children[0].id,
                AccessibilityAction::Activate,
            )),
            Some(WidgetAction::TreeList(TreeListAction::TrailingActionActivated {
                row_key: TreeRowKey(5),
                action_key: TreeRowActionKey(2),
            })),
        );
    }

    fn inline_editor_widget(value: &str) -> TreeListWidget {
        let tree_id = WidgetId(90);
        let mut widget = TreeListWidget::new().with_id(tree_id);
        widget.set_input(TreeListInput {
            rows: vec![row(1, 0, TreeRowExpansion::Expanded), row(2, 0, TreeRowExpansion::Leaf)],
            editor: Some(TreeRowEditorInput {
                key: TreeRowKey(99),
                parent_key: TreeRowKey(1),
                depth: 1,
                value: value.to_owned(),
                placeholder: "新目录名称".to_owned(),
            }),
            scroll_offset_px: 0.0,
        });
        widget.set_keyboard_focus(Some(tree_id));
        layout(&mut widget, Rect::new(0.0, 0.0, 240.0, 120.0), 1.0);
        widget
    }

    #[test]
    fn inline_editor_forwards_ime_text_and_enter_commit() {
        let mut widget = inline_editor_widget("");
        let theme = crate::theme::test_theme();
        let mut context = event_context(&theme);

        assert_eq!(
            widget.on_event(&Event::ImeCommit("计划".to_owned()), &mut context),
            Some(WidgetAction::TreeList(TreeListAction::EditorTextChanged {
                key: TreeRowKey(99),
                value: "计划".to_owned(),
            }))
        );
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            Some(WidgetAction::TreeList(TreeListAction::EditorCommitRequested {
                key: TreeRowKey(99),
                value: "计划".to_owned(),
            }))
        );
    }

    #[test]
    fn inline_editor_escape_cancels_without_committing() {
        let mut widget = inline_editor_widget("draft");
        let theme = crate::theme::test_theme();

        assert_eq!(
            widget.on_event(
                &Event::KeyDown(KeyCode::Escape, Modifiers::NONE),
                &mut event_context(&theme),
            ),
            Some(WidgetAction::TreeList(TreeListAction::EditorCancelled { key: TreeRowKey(99) }))
        );
    }

    #[test]
    fn clicking_outside_inline_editor_commits_non_empty_and_cancels_empty_values() {
        let theme = crate::theme::test_theme();
        for (value, expected) in [
            (
                "draft",
                TreeListAction::EditorCommitRequested {
                    key: TreeRowKey(99),
                    value: "draft".to_owned(),
                },
            ),
            ("", TreeListAction::EditorCancelled { key: TreeRowKey(99) }),
        ] {
            let mut widget = inline_editor_widget(value);
            let sibling_rect = widget.layout().rows[1].row_rect;

            assert_eq!(
                widget.on_event(
                    &Event::MouseDown {
                        px: sibling_rect.x + 1.0,
                        py: sibling_rect.y + 1.0,
                        button: MouseButton::Left,
                    },
                    &mut event_context(&theme),
                ),
                Some(WidgetAction::TreeList(expected))
            );
        }
    }
}
