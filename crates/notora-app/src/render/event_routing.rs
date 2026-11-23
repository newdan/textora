//! 外壳事件派发与输入消费规则。

use std::time::Instant;

use ui::canvas_scrollbars::CanvasScrollbarsAction;
use ui::core::widget::{ControlAction, TextPayload};
use ui::tooltip::TooltipHint;
use ui::{Event, EventCtx, Rect, Widget, WidgetAction};

use crate::action::NotoraAction;
use crate::{FocusTarget, OverlayState, Pane};

use super::NotoraShell;
use super::widget_actions::{
    encrypted_note_dialog_action_to_notora_action, encrypted_note_unlock_action_to_notora_action,
    new_document_menu_action, new_workspace_dialog_action_to_notora_action,
    settings_overlay_action_to_notora_action, splitter_action_to_notora_action,
};

/// 产品事件的路由结果；控件可以只消费事件而不立即产生产品动作。
pub struct NotoraEventRoute {
    pub actions: Vec<NotoraAction>,
    pub consumed: bool,
    pub canvas_scrollbar_action: Option<CanvasScrollbarsAction>,
    pub canvas_view_command: Option<CanvasViewCommand>,
    pub cursor_hint: Option<winit::window::CursorIcon>,
}

impl NotoraEventRoute {
    fn ignored() -> Self {
        Self {
            actions: Vec::new(),
            consumed: false,
            canvas_scrollbar_action: None,
            canvas_view_command: None,
            cursor_hint: None,
        }
    }

    fn consumed(action: Option<NotoraAction>) -> Self {
        Self {
            actions: action.into_iter().collect(),
            consumed: true,
            canvas_scrollbar_action: None,
            canvas_view_command: None,
            cursor_hint: None,
        }
    }

    fn passthrough(action: NotoraAction) -> Self {
        Self {
            actions: vec![action],
            consumed: false,
            canvas_scrollbar_action: None,
            canvas_view_command: None,
            cursor_hint: None,
        }
    }

    fn canvas_scrollbar(action: Option<CanvasScrollbarsAction>) -> Self {
        Self {
            actions: Vec::new(),
            consumed: true,
            canvas_scrollbar_action: action,
            canvas_view_command: None,
            cursor_hint: None,
        }
    }
}

/// 画布操作只改变视口，不产生笔记内容修改。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasViewCommand {
    ZoomOut,
    ZoomIn,
    Fit,
}

impl CanvasViewCommand {
    pub(crate) fn viewport_action(
        self,
        viewport: Rect,
    ) -> appkit_shell::canvas_viewport::CanvasViewportAction {
        use appkit_shell::canvas_viewport::CanvasViewportAction;
        const ZOOM_STEP: f32 = 1.2;
        let factor = match self {
            Self::Fit => return CanvasViewportAction::ResetView,
            Self::ZoomIn => ZOOM_STEP,
            Self::ZoomOut => ZOOM_STEP.recip(),
        };
        CanvasViewportAction::ZoomBy {
            factor,
            screen_anchor: ui::canvas::CanvasPoint::new(
                viewport.x + viewport.w * 0.5,
                viewport.y + viewport.h * 0.5,
            ),
        }
    }
}

impl NotoraShell {
    pub(super) fn route_editor_widget_action(
        &self,
        widget_action: &WidgetAction,
    ) -> NotoraEventRoute {
        let mut route = NotoraEventRoute::consumed(self.translate_widget_action(widget_action));
        if let WidgetAction::Control(ControlAction::TextCommitted {
            id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
            value: TextPayload::Plain(command_key),
        }) = widget_action
        {
            route.canvas_view_command = match command_key.as_str() {
                "canvas_zoom_out" => Some(CanvasViewCommand::ZoomOut),
                "canvas_zoom_in" => Some(CanvasViewCommand::ZoomIn),
                "canvas_fit" => Some(CanvasViewCommand::Fit),
                _ => None,
            };
        }
        route
    }

    /// 产品事件先路由给本产品 widget，并独立返回消费状态与产品动作。
    pub fn route_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        theme: &ui::Theme,
        dpi: f32,
    ) -> NotoraEventRoute {
        let mut clipboard = appkit_shell::SystemClipboard;
        let mut event_context = EventCtx::with_clipboard(theme, dpi, &mut clipboard);
        self.synchronize_focus(focus_target, Instant::now());
        let mut route =
            self.route_event_with_context(event, focus_target, None, &mut event_context);
        let tooltip_changed = self.synchronize_tooltip(event, None);
        route.consumed |= tooltip_changed;
        route.cursor_hint = event_context.cursor_hint;
        route
    }

    pub(crate) fn route_event_with_overlay(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        overlay: OverlayState,
        theme: &ui::Theme,
        dpi: f32,
    ) -> NotoraEventRoute {
        let mut clipboard = appkit_shell::SystemClipboard;
        let mut event_context = EventCtx::with_clipboard(theme, dpi, &mut clipboard);
        self.synchronize_focus(focus_target, Instant::now());
        let mut route =
            self.route_event_with_context(event, focus_target, Some(overlay), &mut event_context);
        let tooltip_changed = self.synchronize_tooltip(event, Some(overlay));
        route.consumed |= tooltip_changed;
        route.cursor_hint = event_context.cursor_hint;
        route
    }

    pub(super) fn route_event_with_context(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        product_overlay: Option<OverlayState>,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        let (appearance_changed, chrome_action) =
            self.dispatch_chrome_button_event(event, product_overlay, event_context);
        if let Some(action) = chrome_action {
            return NotoraEventRoute::consumed(Some(action));
        }
        let pointer_button_event = matches!(event, Event::MouseDown { .. } | Event::MouseUp { .. });
        let captured_move =
            matches!(event, Event::MouseMove { .. }) && self.chrome_button_is_capturing();
        if (appearance_changed && pointer_button_event) || captured_move {
            return NotoraEventRoute::consumed(None);
        }
        let mut route =
            self.route_content_event(event, focus_target, product_overlay, event_context);
        route.consumed |= appearance_changed;
        route
    }

    fn route_content_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        product_overlay: Option<OverlayState>,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        if let Some(route) = self.route_product_overlay_event(event, product_overlay, event_context)
        {
            return route;
        }
        if let Some(route) = self.route_inspector_panel_event(event, event_context) {
            return route;
        }
        if self.editor_pane.has_open_popup() {
            return self.route_editor_popup_event(event, event_context);
        }
        if event_is_keyboard(event) {
            return self.route_keyboard_or_ime_event(event, focus_target, event_context);
        }
        self.route_pointer_event(event, focus_target, event_context)
    }

    fn synchronize_tooltip(
        &mut self,
        event: &Event,
        product_overlay: Option<OverlayState>,
    ) -> bool {
        let next_tooltip = if product_overlay.is_some_and(|overlay| overlay != OverlayState::None) {
            None
        } else {
            match event {
                Event::MouseMove { px, py } => self.tooltip_at(*px, *py),
                Event::MouseDown { .. }
                | Event::PointerLeave
                | Event::InteractionCancel
                | Event::Wheel { .. } => None,
                Event::MouseUp { .. }
                | Event::KeyDown(..)
                | Event::ImePreedit { .. }
                | Event::ImeCommit(_)
                | Event::ImeEnable
                | Event::ImeDisable => return false,
            }
        };
        if self.active_tooltip == next_tooltip {
            return false;
        }
        self.active_tooltip = next_tooltip;
        true
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        if self.editor_pane.has_open_popup() {
            return None;
        }
        if self.inspector_panel_stamp.is_some() && self.inspector_panel_rect.contains(px, py) {
            let hint = self
                .inspector_panel
                .tooltip_at(px - self.inspector_panel_rect.x, py - self.inspector_panel_rect.y)?;
            return Some(offset_tooltip_hint(hint, self.inspector_panel_rect));
        }
        self.new_note_button
            .tooltip_at(px, py)
            .or_else(|| {
                self.navigation_collapse_rect.contains(px, py).then(|| TooltipHint {
                    label: "收起笔记库".to_owned(),
                    target_rect: self.navigation_collapse_rect,
                })
            })
            .or_else(|| {
                self.navigation_expand_rect.contains(px, py).then(|| TooltipHint {
                    label: "展开笔记库".to_owned(),
                    target_rect: self.navigation_expand_rect,
                })
            })
            .or_else(|| self.navigation_tree.tooltip_at(px, py))
            .or_else(|| self.card_list.tooltip_at(px, py))
            .or_else(|| self.editor_pane.tooltip_at(px, py))
    }

    fn route_product_overlay_event(
        &mut self,
        event: &Event,
        product_overlay: Option<OverlayState>,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        if product_overlay.is_some_and(|overlay| {
            overlay != OverlayState::None && !self.modal_input_is_ready(overlay)
        }) {
            return Some(NotoraEventRoute::consumed(escape_dismiss_action(event)));
        }
        if self.settings_overlay_open()
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::Settings)
        {
            let action = self
                .settings_overlay
                .route_event(event, event_context)
                .map(settings_overlay_action_to_notora_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.new_workspace_dialog_open
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::NewWorkspace)
        {
            let action = self
                .new_workspace_dialog
                .as_mut()
                .and_then(|dialog| dialog.route_event(event, event_context))
                .map(new_workspace_dialog_action_to_notora_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.encrypted_note_dialog_open
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::EncryptedNoteDialog)
        {
            let action = self
                .encrypted_note_dialog
                .as_mut()
                .and_then(|dialog| dialog.route_event(event, event_context))
                .map(encrypted_note_dialog_action_to_notora_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.save_conflict_actions.is_some()
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::SaveConflict)
        {
            return Some(NotoraEventRoute::consumed(escape_dismiss_action(event)));
        }
        if self.confirmation_action.is_some() && product_overlay.is_none_or(is_confirmation_overlay)
        {
            let backdrop_click = matches!(event,
                Event::MouseDown { px, py, button: ui::MouseButton::Left }
                    if !self.confirmation_panel_rect.contains(*px, *py)
            );
            let action = backdrop_click
                .then_some(NotoraAction::OverlayDismissed)
                .or_else(|| escape_dismiss_action(event));
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.new_document_menu_open
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::NewDocumentMenu)
        {
            return Some(self.route_new_document_menu_event(event, event_context));
        }
        None
    }

    pub(super) fn route_new_document_menu_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        let local_event =
            translate_event(event, self.new_document_menu_rect.x, self.new_document_menu_rect.y);
        let action = self
            .new_document_menu
            .as_mut()
            .and_then(|menu| menu.on_event(&local_event, event_context))
            .as_ref()
            .and_then(new_document_menu_action);
        NotoraEventRoute::consumed(action)
    }

    fn route_editor_popup_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        let widget_action = self.editor_pane.route_event(event, event_context);
        widget_action
            .as_ref()
            .map(|action| self.route_editor_widget_action(action))
            .unwrap_or_else(|| NotoraEventRoute::consumed(None))
    }

    fn route_inspector_panel_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        self.inspector_panel_stamp.as_ref()?;
        let local_event =
            translate_event(event, self.inspector_panel_rect.x, self.inspector_panel_rect.y);
        let widget_action = self.inspector_panel.on_event(&local_event, event_context);
        let action = widget_action.as_ref().and_then(|action| self.translate_widget_action(action));
        let pointer_inside = event_pointer_position(event)
            .is_some_and(|(px, py)| self.inspector_panel_rect.contains(px, py));
        if widget_action.is_some() || event_is_keyboard(event) || pointer_inside {
            return Some(NotoraEventRoute::consumed(action));
        }
        None
    }

    fn route_keyboard_or_ime_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        if focus_target == FocusTarget::Editor && self.encrypted_note_unlock_open {
            let action = self
                .encrypted_note_unlock
                .as_mut()
                .and_then(|unlock| unlock.route_event(event, event_context))
                .map(encrypted_note_unlock_action_to_notora_action);
            return NotoraEventRoute::consumed(action);
        }
        if matches!(focus_target, FocusTarget::EditorTitle | FocusTarget::EditorTag)
            && let Some(widget_action) = self.editor_pane.route_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return NotoraEventRoute::consumed(action);
        }
        let widget_action = self.route_focused_widget_event(event, focus_target, event_context);
        let action = widget_action
            .as_ref()
            .and_then(|widget_action| self.translate_widget_action(widget_action));
        if action.is_some() || widget_action.is_some() {
            return NotoraEventRoute::consumed(action);
        }
        if matches!(focus_target, FocusTarget::EditorTag | FocusTarget::Overlay) {
            return NotoraEventRoute::consumed(None);
        }
        NotoraEventRoute::ignored()
    }

    fn route_pointer_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        let pointer_focus = pointer_target(event, self);
        let card_hover_cleared = matches!(event, Event::MouseMove { .. })
            && pointer_focus != Some(FocusTarget::CardList)
            && self.card_list.on_event(event, event_context).is_some();
        if let Some(mut route) = self.route_pointer_chrome_event(event, event_context) {
            route.consumed |= card_hover_cleared;
            return route;
        }
        let widget_focus = pointer_focus.unwrap_or(focus_target);
        let widget_action = self.route_focused_widget_event(event, widget_focus, event_context);
        let action = widget_action
            .as_ref()
            .and_then(|widget_action| self.translate_widget_action(widget_action));
        if action.is_some() {
            return NotoraEventRoute::consumed(action);
        }
        if is_left_mouse_down(event)
            && let Some(focus_target) = pointer_focus
        {
            let focus_action = NotoraAction::FocusRequested(focus_target);
            if focus_target == FocusTarget::Editor {
                return NotoraEventRoute::passthrough(focus_action);
            }
            return NotoraEventRoute::consumed(Some(focus_action));
        }
        if widget_action.is_some() || card_hover_cleared {
            return NotoraEventRoute::consumed(None);
        }
        NotoraEventRoute::ignored()
    }

    fn route_pointer_chrome_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        if self.encrypted_note_unlock_open
            && let Some(unlock) = self.encrypted_note_unlock.as_mut()
            && event_pointer_position(event).is_some_and(|(px, py)| unlock.contains(px, py))
        {
            let action = unlock
                .route_event(event, event_context)
                .map(encrypted_note_unlock_action_to_notora_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if let Some(route) = self.route_canvas_scrollbars_event(event, event_context) {
            return Some(route);
        }
        if let Some(widget_action) = self.editor_pane.route_event(event, event_context) {
            return Some(self.route_editor_widget_action(&widget_action));
        }
        if is_splitter_pointer_event(event)
            && let Some(widget_action) = self.new_note_button.on_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if let Some(action) = self.route_splitter_event(event, event_context) {
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.card_empty_state_visible
            && let Some(widget_action) = self.card_empty_state.on_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        None
    }

    fn route_focused_widget_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        event_context: &mut EventCtx,
    ) -> Option<WidgetAction> {
        match focus_target {
            FocusTarget::NavigationSearch => self.search_box.on_event(event, event_context),
            FocusTarget::NavigationTree => self.navigation_tree.on_event(event, event_context),
            FocusTarget::CardList => self.card_list.on_event(event, event_context),
            FocusTarget::Editor
            | FocusTarget::DocumentSearch
            | FocusTarget::EditorTitle
            | FocusTarget::EditorTag
            | FocusTarget::Overlay => None,
        }
    }

    pub(super) fn modal_input_is_ready(&self, overlay: OverlayState) -> bool {
        match overlay {
            OverlayState::None => true,
            OverlayState::Settings => self.settings_overlay_open,
            OverlayState::NewDocumentMenu => self.new_document_menu_open,
            OverlayState::NewWorkspace => {
                self.new_workspace_dialog_open && self.new_workspace_dialog.is_some()
            }
            OverlayState::EncryptedNoteDialog => {
                self.encrypted_note_dialog_open && self.encrypted_note_dialog.is_some()
            }
            OverlayState::TrashPermanentDeletionConfirmation { .. }
            | OverlayState::TrashRestoreConflictConfirmation { .. } => {
                self.confirmation_action.is_some()
            }
            OverlayState::SaveConflict => self.save_conflict_actions.is_some(),
        }
    }

    fn route_canvas_scrollbars_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        if !matches!(
            event,
            Event::MouseMove { .. }
                | Event::MouseDown { .. }
                | Event::MouseUp { .. }
                | Event::Wheel { .. }
        ) {
            return None;
        }
        let was_capturing = self.canvas_scrollbars.is_capturing();
        if self.canvas_scrollbars_input.is_none() && !was_capturing {
            return None;
        }
        let local_event = translate_event(event, self.canvas_rect.x, self.canvas_rect.y);
        let pointer_hits_scrollbar = match &local_event {
            Event::MouseMove { px, py }
            | Event::MouseDown { px, py, .. }
            | Event::MouseUp { px, py, .. }
            | Event::Wheel { px, py, .. } => self.canvas_scrollbars.hit(*px, *py),
            _ => false,
        };
        let should_dispatch =
            was_capturing || pointer_hits_scrollbar || matches!(event, Event::MouseMove { .. });
        if !should_dispatch {
            return None;
        }
        let widget_action = self.canvas_scrollbars.on_event(&local_event, event_context);
        match widget_action {
            Some(WidgetAction::CanvasScrollbars(action)) => {
                Some(NotoraEventRoute::canvas_scrollbar(Some(action)))
            }
            Some(_) => Some(NotoraEventRoute::canvas_scrollbar(None)),
            None if matches!(event, Event::Wheel { .. }) => None,
            None if was_capturing || pointer_hits_scrollbar => {
                Some(NotoraEventRoute::canvas_scrollbar(None))
            }
            None => None,
        }
    }

    fn route_splitter_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<Option<NotoraAction>> {
        if !is_splitter_pointer_event(event) {
            return None;
        }
        let navigation_action = self.navigation_splitter.on_event(event, event_context);
        if navigation_action.is_some() {
            return Some(
                navigation_action
                    .as_ref()
                    .and_then(|action| splitter_action_to_notora_action(action, Pane::Navigation)),
            );
        }
        let card_list_action = self.card_list_splitter.on_event(event, event_context);
        card_list_action.map(|action| splitter_action_to_notora_action(&action, Pane::CardList))
    }

    pub(super) fn settings_overlay_open(&self) -> bool {
        self.settings_overlay_open
    }
}

fn pointer_target(event: &Event, shell: &NotoraShell) -> Option<FocusTarget> {
    let (px, py) = event_pointer_position(event)?;
    if shell.search_rect.contains(px, py) {
        return Some(FocusTarget::NavigationSearch);
    }
    if shell.navigation_tree_rect.contains(px, py) {
        return Some(FocusTarget::NavigationTree);
    }
    if shell.card_content_rect.contains(px, py) {
        return Some(FocusTarget::CardList);
    }
    if shell.editor_rect.contains(px, py) {
        return Some(FocusTarget::Editor);
    }
    None
}

pub(super) fn event_pointer_position(event: &Event) -> Option<(f32, f32)> {
    match event {
        Event::MouseMove { px, py }
        | Event::MouseDown { px, py, .. }
        | Event::MouseUp { px, py, .. }
        | Event::Wheel { px, py, .. } => Some((*px, *py)),
        Event::KeyDown(..)
        | Event::ImePreedit { .. }
        | Event::ImeCommit(_)
        | Event::ImeEnable
        | Event::ImeDisable
        | Event::PointerLeave
        | Event::InteractionCancel => None,
    }
}

fn event_is_keyboard(event: &Event) -> bool {
    matches!(
        event,
        Event::KeyDown(..)
            | Event::ImePreedit { .. }
            | Event::ImeCommit(_)
            | Event::ImeEnable
            | Event::ImeDisable
    )
}

fn escape_dismiss_action(event: &Event) -> Option<NotoraAction> {
    matches!(event, Event::KeyDown(ui::KeyCode::Escape, _))
        .then_some(NotoraAction::OverlayDismissed)
}

pub(super) fn is_confirmation_overlay(overlay: OverlayState) -> bool {
    matches!(
        overlay,
        OverlayState::TrashPermanentDeletionConfirmation { .. }
            | OverlayState::TrashRestoreConflictConfirmation { .. }
    )
}

fn is_splitter_pointer_event(event: &Event) -> bool {
    matches!(event, Event::MouseMove { .. } | Event::MouseDown { .. } | Event::MouseUp { .. })
}

fn is_left_mouse_down(event: &Event) -> bool {
    matches!(event, Event::MouseDown { button: ui::core::widget::MouseButton::Left, .. })
}

fn offset_tooltip_hint(mut hint: TooltipHint, offset: Rect) -> TooltipHint {
    hint.target_rect.x += offset.x;
    hint.target_rect.y += offset.y;
    hint
}

pub(super) fn translate_event(event: &Event, offset_x: f32, offset_y: f32) -> Event {
    match event {
        Event::MouseMove { px, py } => Event::MouseMove { px: *px - offset_x, py: *py - offset_y },
        Event::PointerLeave => Event::PointerLeave,
        Event::MouseDown { px, py, button } => {
            Event::MouseDown { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
        Event::MouseUp { px, py, button } => {
            Event::MouseUp { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
        Event::InteractionCancel => Event::InteractionCancel,
        Event::Wheel { dx, dy, px, py } => {
            Event::Wheel { dx: *dx, dy: *dy, px: *px - offset_x, py: *py - offset_y }
        }
        Event::KeyDown(key, modifiers) => Event::KeyDown(*key, *modifiers),
        Event::ImePreedit { text, cursor } => {
            Event::ImePreedit { text: text.clone(), cursor: *cursor }
        }
        Event::ImeCommit(text) => Event::ImeCommit(text.clone()),
        Event::ImeEnable => Event::ImeEnable,
        Event::ImeDisable => Event::ImeDisable,
    }
}
