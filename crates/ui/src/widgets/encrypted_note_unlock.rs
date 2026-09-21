//! 内容区内联的加密笔记解锁组件。

use crate::WidgetAction;
use crate::button::{Button, ButtonStyle};
use crate::core::widget::{ControlAction, SensitiveText, TextPayload, WidgetId};
use crate::core::{
    ChildEventRouter, Event, EventCtx, FocusDirection, KeyCode, LayoutCtx, PaintCtx, Rect, Widget,
    dispatch_child_event_route,
};
use crate::text_box::TextBox;

const PASSWORD_INPUT_ID: WidgetId = WidgetId(9_204);
const SUBMIT_BUTTON_ID: WidgetId = WidgetId(9_205);
const CONTENT_WIDTH_LOGICAL: f32 = 400.0;
const FIELD_HEIGHT_LOGICAL: f32 = 34.0;
const BUTTON_WIDTH_LOGICAL: f32 = crate::button::ButtonMetrics::text_width(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnlockControl {
    Password,
    Submit,
}

impl UnlockControl {
    const ALL: [Self; 2] = [Self::Password, Self::Submit];

    fn widget_id(self) -> WidgetId {
        match self {
            Self::Password => PASSWORD_INPUT_ID,
            Self::Submit => SUBMIT_BUTTON_ID,
        }
    }

    fn from_widget_id(id: WidgetId) -> Option<Self> {
        match id {
            PASSWORD_INPUT_ID => Some(Self::Password),
            SUBMIT_BUTTON_ID => Some(Self::Submit),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct EncryptedNoteUnlockInput {
    pub title: String,
    pub password: SensitiveText,
    pub submitting: bool,
    pub error_message: Option<String>,
    pub failure_generation: u64,
}

impl EncryptedNoteUnlockInput {
    pub fn new(title: String) -> Self {
        Self {
            title,
            password: SensitiveText::new(String::new()),
            submitting: false,
            error_message: None,
            failure_generation: 0,
        }
    }

    fn can_submit(&self) -> bool {
        !self.submitting && !self.password.expose().is_empty()
    }
}

impl std::fmt::Debug for EncryptedNoteUnlockInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EncryptedNoteUnlockInput")
            .field("title", &self.title)
            .field("password", &"<redacted>")
            .field("submitting", &self.submitting)
            .field("error_message", &self.error_message)
            .field("failure_generation", &self.failure_generation)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EncryptedNoteUnlockAction {
    PasswordChanged(SensitiveText),
    Submit,
}

pub struct EncryptedNoteUnlock {
    input: EncryptedNoteUnlockInput,
    password_input: TextBox,
    submit_button: Button,
    content_rect: Rect,
    visible: bool,
    observed_failure_generation: u64,
    event_router: ChildEventRouter<UnlockControl>,
}

impl EncryptedNoteUnlock {
    pub fn new(theme: &crate::Theme) -> Self {
        let mut password_input = TextBox::with_id(PASSWORD_INPUT_ID);
        password_input.set_password_mode(true);
        password_input.set_placeholder("输入密码");
        password_input.set_accessibility_label(Some("加密笔记密码".to_owned()));
        password_input.set_max_len_bytes(4_096);
        let mut submit_button = Button::new(SUBMIT_BUTTON_ID, ButtonStyle::from_theme(theme));
        submit_button.set_text(Some("解密".to_owned()));
        submit_button.set_accessibility_label(Some("解密笔记".to_owned()));
        Self {
            input: EncryptedNoteUnlockInput::new(String::new()),
            password_input,
            submit_button,
            content_rect: Rect::ZERO,
            visible: false,
            observed_failure_generation: 0,
            event_router: ChildEventRouter::default(),
        }
    }

    pub fn set_input(&mut self, input: EncryptedNoteUnlockInput, visible: bool) {
        self.password_input.sync_text(input.password.expose());
        self.submit_button
            .set_text(Some(if input.submitting { "解密中…" } else { "解密" }.to_owned()));
        self.submit_button.set_enabled(input.can_submit());
        let should_focus_password = (visible && !self.visible)
            || input.failure_generation != self.observed_failure_generation;
        self.input = input;
        if should_focus_password {
            self.set_focused_control(Some(UnlockControl::Password));
            self.password_input.select_all();
        } else if !visible {
            self.set_focused_control(None);
            self.event_router.clear_interactions();
        }
        self.observed_failure_generation = self.input.failure_generation;
        self.visible = visible;
    }

    pub fn set_keyboard_focus(&mut self, focused: bool) {
        if focused && self.visible && self.event_router.focused_target().is_none() {
            self.set_focused_control(Some(UnlockControl::Password));
        } else if !focused {
            self.set_focused_control(None);
        }
    }

    pub fn set_rect(&mut self, content_rect: Rect, context: &mut LayoutCtx<'_>) {
        self.content_rect = content_rect;
        let content_width = (CONTENT_WIDTH_LOGICAL * context.dpi).min(content_rect.w.max(0.0));
        let left = content_rect.x + (content_rect.w - content_width) * 0.5;
        let center_y = content_rect.y + content_rect.h * 0.46;
        let field_height = FIELD_HEIGHT_LOGICAL * context.dpi;
        self.password_input
            .set_rect(Rect::new(left, center_y, content_width, field_height), context);
        let button_width = BUTTON_WIDTH_LOGICAL * context.dpi;
        self.submit_button.set_rect(
            Rect::new(
                left + content_width - button_width,
                center_y + field_height + 16.0 * context.dpi,
                button_width,
                field_height,
            ),
            context,
        );
    }

    pub fn paint(&self, context: &mut PaintCtx<'_>) {
        let theme = context.theme.application_theme();
        let left = self.password_input.rect().x;
        let heading_y = self.password_input.rect().y - 72.0 * context.dpi;
        context.text(left, heading_y, 18.0 * context.dpi, theme.text_primary, "解锁加密笔记");
        context.text(
            left,
            heading_y + 26.0 * context.dpi,
            12.0 * context.dpi,
            theme.text_secondary,
            &self.input.title,
        );
        self.password_input.paint(context);
        if let Some(message) = &self.input.error_message {
            context.text(
                left,
                self.submit_button.rect().y + 22.0 * context.dpi,
                12.0 * context.dpi,
                theme.danger,
                message,
            );
        }
        self.submit_button.paint(context);
    }

    pub fn route_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<EncryptedNoteUnlockAction> {
        if !self.visible {
            return None;
        }
        if let Event::KeyDown(KeyCode::Tab, modifiers) = event {
            self.cycle_focus(modifiers.shift);
            return None;
        }
        let hit_target = match event {
            Event::MouseDown { px, py, .. }
            | Event::MouseMove { px, py }
            | Event::MouseUp { px, py, .. } => self.control_at(*px, *py),
            _ => None,
        };
        let route = self.event_router.route_event(event, hit_target);
        let dispatch = dispatch_child_event_route(
            route,
            event,
            UnlockControl::ALL,
            context,
            |control, event, context| self.dispatch_to_control(control, event, context),
        );
        dispatch.action
    }

    pub fn focused_text_input(&self) -> Option<&TextBox> {
        self.password_input.is_focused().then_some(&self.password_input)
    }

    pub fn advance_cursor_blink(&mut self, now: std::time::Instant) -> bool {
        self.password_input.advance_cursor_blink(now)
    }

    pub fn ime_cursor_rect(&self) -> Option<Rect> {
        self.password_input.is_focused().then(|| self.password_input.ime_cursor_rect())
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        self.content_rect.contains(px, py)
    }

    fn dispatch_to_control(
        &mut self,
        control: UnlockControl,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<EncryptedNoteUnlockAction> {
        let action = match control {
            UnlockControl::Password => self.password_input.on_event(event, context),
            UnlockControl::Submit => self.submit_button.on_event(event, context),
        }?;
        match action {
            WidgetAction::Control(ControlAction::TextEdited {
                id: PASSWORD_INPUT_ID,
                value: TextPayload::Sensitive(password),
            }) => Some(EncryptedNoteUnlockAction::PasswordChanged(password)),
            WidgetAction::Control(ControlAction::TextCommitted {
                id: PASSWORD_INPUT_ID,
                value: TextPayload::Sensitive(_),
            }) if self.input.can_submit() => Some(EncryptedNoteUnlockAction::Submit),
            WidgetAction::Control(ControlAction::FocusRequested { id }) => {
                self.set_focused_control(UnlockControl::from_widget_id(id));
                None
            }
            WidgetAction::Control(ControlAction::Activated { id: SUBMIT_BUTTON_ID }) => {
                Some(EncryptedNoteUnlockAction::Submit)
            }
            _ => None,
        }
    }

    fn set_focused_control(&mut self, focused_control: Option<UnlockControl>) {
        let focused_id = focused_control.map(UnlockControl::widget_id);
        self.password_input.set_keyboard_focus(focused_id);
        self.submit_button.set_keyboard_focus(focused_id);
        self.event_router.set_focused_target(focused_control);
    }

    fn cycle_focus(&mut self, backwards: bool) {
        let controls = if self.input.can_submit() {
            &[UnlockControl::Password, UnlockControl::Submit][..]
        } else {
            &[UnlockControl::Password][..]
        };
        let direction = if backwards { FocusDirection::Backward } else { FocusDirection::Forward };
        let focused_control = self.event_router.cycle_focus(controls, direction);
        self.set_focused_control(focused_control);
    }

    fn control_at(&self, px: f32, py: f32) -> Option<UnlockControl> {
        if self.password_input.hit(px, py) {
            return Some(UnlockControl::Password);
        }
        self.submit_button.hit(px, py).then_some(UnlockControl::Submit)
    }
}

#[cfg(test)]
mod tests {
    use super::{EncryptedNoteUnlock, EncryptedNoteUnlockAction, EncryptedNoteUnlockInput};
    use crate::core::Clipboard;
    use crate::core::widget::SensitiveText;
    use crate::core::{Event, EventCtx, KeyCode, LayoutCtx, Modifiers, NoopMeasure, Rect};

    struct EmptyClipboard;

    impl Clipboard for EmptyClipboard {
        fn read_text(&mut self) -> Option<String> {
            None
        }

        fn write_text(&mut self, _text: &str) -> bool {
            true
        }
    }

    #[test]
    fn enter_submits_the_inline_password_and_failure_refocuses_it() {
        let theme = crate::theme::test_theme();
        let mut unlock = EncryptedNoteUnlock::new(&theme);
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, theme: &theme, dpi: 1.0, measure: &mut measure };
        unlock.set_rect(Rect::new(0.0, 0.0, 800.0, 600.0), &mut layout);
        let mut input = EncryptedNoteUnlockInput::new("私密笔记".to_owned());
        input.password = SensitiveText::new("correct-password".to_owned());
        unlock.set_input(input, true);
        let mut clipboard = EmptyClipboard;
        let mut context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);

        assert_eq!(
            unlock.route_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            Some(EncryptedNoteUnlockAction::Submit)
        );

        let mut failed = EncryptedNoteUnlockInput::new("私密笔记".to_owned());
        failed.error_message = Some("密码错误".to_owned());
        failed.failure_generation = 1;
        unlock.set_input(failed, true);
        assert!(unlock.ime_cursor_rect().is_some());
    }

    #[test]
    fn debug_output_redacts_the_password() {
        let mut input = EncryptedNoteUnlockInput::new("私密笔记".to_owned());
        input.password = SensitiveText::new("never-print-password".to_owned());

        assert!(!format!("{input:?}").contains("never-print-password"));
    }
}
