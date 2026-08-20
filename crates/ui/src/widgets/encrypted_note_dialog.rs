//! Pure encrypted-note password dialog with sensitive text actions.

use crate::WidgetAction;
use crate::button::{Button, ButtonStyle};
use crate::core::widget::{ControlAction, SensitiveText, TextPayload, WidgetId};
use crate::core::{Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget};
use crate::text_box::TextBox;

const PASSWORD_INPUT_ID: WidgetId = WidgetId(9_200);
const CONFIRMATION_INPUT_ID: WidgetId = WidgetId(9_201);
const SUBMIT_BUTTON_ID: WidgetId = WidgetId(9_202);
const CANCEL_BUTTON_ID: WidgetId = WidgetId(9_203);
const PANEL_WIDTH_LOGICAL: f32 = 480.0;
const CREATE_PANEL_HEIGHT_LOGICAL: f32 = 330.0;
const UNLOCK_PANEL_HEIGHT_LOGICAL: f32 = 270.0;
const PANEL_MARGIN_LOGICAL: f32 = 24.0;
const FIELD_HEIGHT_LOGICAL: f32 = 34.0;
const BUTTON_WIDTH_LOGICAL: f32 = 92.0;
const BUTTON_GAP_LOGICAL: f32 = 8.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncryptedNoteDialogMode {
    Create,
    ConflictCopy { file_name: String },
    Unlock { title: String },
}

impl EncryptedNoteDialogMode {
    fn requires_confirmation(&self) -> bool {
        matches!(self, Self::Create | Self::ConflictCopy { .. })
    }
}

#[derive(Clone, PartialEq)]
pub struct EncryptedNoteDialogInput {
    pub mode: EncryptedNoteDialogMode,
    pub password: SensitiveText,
    pub confirmation: Option<SensitiveText>,
    pub submitting: bool,
    pub error_message: Option<String>,
    pub failure_generation: u64,
}

impl EncryptedNoteDialogInput {
    pub fn create() -> Self {
        Self {
            mode: EncryptedNoteDialogMode::Create,
            password: SensitiveText::new(String::new()),
            confirmation: Some(SensitiveText::new(String::new())),
            submitting: false,
            error_message: None,
            failure_generation: 0,
        }
    }

    pub fn unlock(title: String) -> Self {
        Self {
            mode: EncryptedNoteDialogMode::Unlock { title },
            password: SensitiveText::new(String::new()),
            confirmation: None,
            submitting: false,
            error_message: None,
            failure_generation: 0,
        }
    }

    pub fn conflict_copy(file_name: String) -> Self {
        Self {
            mode: EncryptedNoteDialogMode::ConflictCopy { file_name },
            password: SensitiveText::new(String::new()),
            confirmation: Some(SensitiveText::new(String::new())),
            submitting: false,
            error_message: None,
            failure_generation: 0,
        }
    }

    fn can_submit(&self) -> bool {
        if self.submitting || self.password.expose().is_empty() {
            return false;
        }
        self.confirmation.as_ref().is_none_or(|value| !value.expose().is_empty())
    }
}

impl std::fmt::Debug for EncryptedNoteDialogInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EncryptedNoteDialogInput")
            .field("mode", &self.mode)
            .field("password", &"<redacted>")
            .field("confirmation", &self.confirmation.as_ref().map(|_| "<redacted>"))
            .field("submitting", &self.submitting)
            .field("error_message", &self.error_message)
            .field("failure_generation", &self.failure_generation)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EncryptedNoteDialogAction {
    PasswordChanged(SensitiveText),
    ConfirmationChanged(SensitiveText),
    Submit,
    Cancel,
}

pub struct EncryptedNoteDialog {
    input: EncryptedNoteDialogInput,
    password_input: TextBox,
    confirmation_input: TextBox,
    submit_button: Button,
    cancel_button: Button,
    panel_rect: Rect,
    open: bool,
    observed_failure_generation: u64,
}

impl EncryptedNoteDialog {
    pub fn new(theme: &crate::Theme) -> Self {
        let mut password_input = sensitive_text_box(PASSWORD_INPUT_ID, "密码");
        password_input.set_placeholder("至少 8 个字符");
        let mut confirmation_input = sensitive_text_box(CONFIRMATION_INPUT_ID, "确认密码");
        confirmation_input.set_placeholder("再次输入密码");
        let style = ButtonStyle::from_theme(theme);
        Self {
            input: EncryptedNoteDialogInput::create(),
            password_input,
            confirmation_input,
            submit_button: labeled_button(SUBMIT_BUTTON_ID, "创建", style.clone()),
            cancel_button: labeled_button(CANCEL_BUTTON_ID, "取消", style),
            panel_rect: Rect::ZERO,
            open: false,
            observed_failure_generation: 0,
        }
    }

    pub fn set_input(&mut self, input: EncryptedNoteDialogInput, open: bool) {
        let submit_label = match input.mode {
            EncryptedNoteDialogMode::Create => "创建",
            EncryptedNoteDialogMode::ConflictCopy { .. } => "保存副本",
            EncryptedNoteDialogMode::Unlock { .. } => "打开",
        };
        self.password_input.sync_text(input.password.expose());
        self.confirmation_input
            .sync_text(input.confirmation.as_ref().map_or("", SensitiveText::expose));
        self.submit_button.set_text(Some(submit_label.to_owned()));
        self.submit_button.set_enabled(input.can_submit());

        let should_focus_password =
            (open && !self.open) || input.failure_generation != self.observed_failure_generation;
        if should_focus_password {
            self.password_input.set_focus(true);
            self.password_input.select_all();
            self.confirmation_input.set_focus(false);
        } else if !open {
            self.password_input.set_focus(false);
            self.confirmation_input.set_focus(false);
        }
        self.observed_failure_generation = input.failure_generation;
        self.input = input;
        self.open = open;
    }

    pub fn set_rect(&mut self, overlay_rect: Rect, context: &mut LayoutCtx<'_>) {
        let margin = PANEL_MARGIN_LOGICAL * context.dpi;
        let panel_height = match self.input.mode {
            EncryptedNoteDialogMode::Create | EncryptedNoteDialogMode::ConflictCopy { .. } => {
                CREATE_PANEL_HEIGHT_LOGICAL
            }
            EncryptedNoteDialogMode::Unlock { .. } => UNLOCK_PANEL_HEIGHT_LOGICAL,
        };
        let width =
            (PANEL_WIDTH_LOGICAL * context.dpi).min((overlay_rect.w - margin * 2.0).max(0.0));
        let height = (panel_height * context.dpi).min((overlay_rect.h - margin * 2.0).max(0.0));
        self.panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - width) * 0.5,
            overlay_rect.y + (overlay_rect.h - height) * 0.5,
            width,
            height,
        );
        let horizontal_padding = 24.0 * context.dpi;
        let field_height = FIELD_HEIGHT_LOGICAL * context.dpi;
        let content_width = (width - horizontal_padding * 2.0).max(0.0);
        self.password_input.set_rect(
            Rect::new(
                self.panel_rect.x + horizontal_padding,
                self.panel_rect.y + 92.0 * context.dpi,
                content_width,
                field_height,
            ),
            context,
        );
        self.confirmation_input.set_rect(
            Rect::new(
                self.panel_rect.x + horizontal_padding,
                self.panel_rect.y + 158.0 * context.dpi,
                content_width,
                field_height,
            ),
            context,
        );
        let button_width = BUTTON_WIDTH_LOGICAL * context.dpi;
        let button_gap = BUTTON_GAP_LOGICAL * context.dpi;
        let button_y = self.panel_rect.bottom() - horizontal_padding - field_height;
        self.submit_button.set_rect(
            Rect::new(
                self.panel_rect.right() - horizontal_padding - button_width,
                button_y,
                button_width,
                field_height,
            ),
            context,
        );
        self.cancel_button.set_rect(
            Rect::new(
                self.submit_button.rect().x - button_gap - button_width,
                button_y,
                button_width,
                field_height,
            ),
            context,
        );
    }

    pub fn paint(&self, context: &mut PaintCtx<'_>) {
        let theme = context.theme.application_theme();
        context.list.fill_rounded(self.panel_rect, theme.overlay_surface, 10.0 * context.dpi);
        let left = self.panel_rect.x + 24.0 * context.dpi;
        let (heading, detail) = match &self.input.mode {
            EncryptedNoteDialogMode::Create => {
                ("新建加密笔记", "密码丢失后无法恢复正文；标题与文件名不会加密。")
            }
            EncryptedNoteDialogMode::ConflictCopy { file_name } => {
                ("保存加密冲突副本", file_name.as_str())
            }
            EncryptedNoteDialogMode::Unlock { title } => ("解锁加密笔记", title.as_str()),
        };
        context.text(
            left,
            self.panel_rect.y + 34.0 * context.dpi,
            18.0 * context.dpi,
            theme.text_primary,
            heading,
        );
        context.text(
            left,
            self.panel_rect.y + 60.0 * context.dpi,
            12.0 * context.dpi,
            theme.text_secondary,
            detail,
        );
        context.text(
            left,
            self.panel_rect.y + 84.0 * context.dpi,
            12.0 * context.dpi,
            theme.text_secondary,
            "密码",
        );
        self.password_input.paint(context);
        if self.input.mode.requires_confirmation() {
            context.text(
                left,
                self.panel_rect.y + 150.0 * context.dpi,
                12.0 * context.dpi,
                theme.text_secondary,
                "确认密码",
            );
            self.confirmation_input.paint(context);
        }
        if let Some(message) = &self.input.error_message {
            context.text(
                left,
                self.submit_button.rect().y - 16.0 * context.dpi,
                12.0 * context.dpi,
                theme.danger,
                message,
            );
        }
        self.cancel_button.paint(context);
        self.submit_button.paint(context);
    }

    pub fn route_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<EncryptedNoteDialogAction> {
        if matches!(event, Event::KeyDown(KeyCode::Escape, _)) {
            return Some(EncryptedNoteDialogAction::Cancel);
        }
        if let Event::MouseDown { px, py, button: MouseButton::Left } = event
            && !self.panel_rect.contains(*px, *py)
        {
            return Some(EncryptedNoteDialogAction::Cancel);
        }
        if let Some(action) = self.password_input.on_event(event, context)
            && let Some(mapped) = self.map_text_action(action)
        {
            return Some(mapped);
        }
        if self.input.mode.requires_confirmation()
            && let Some(action) = self.confirmation_input.on_event(event, context)
            && let Some(mapped) = self.map_text_action(action)
        {
            return Some(mapped);
        }
        for (button, action) in [
            (&mut self.submit_button, EncryptedNoteDialogAction::Submit),
            (&mut self.cancel_button, EncryptedNoteDialogAction::Cancel),
        ] {
            if matches!(
                button.on_event(event, context),
                Some(WidgetAction::Control(ControlAction::Activated { .. }))
            ) {
                return Some(action);
            }
        }
        None
    }

    pub fn ime_cursor_rect(&self) -> Option<Rect> {
        if self.password_input.is_focused() {
            return Some(self.password_input.ime_cursor_rect());
        }
        self.confirmation_input.is_focused().then(|| self.confirmation_input.ime_cursor_rect())
    }

    fn map_text_action(&mut self, action: WidgetAction) -> Option<EncryptedNoteDialogAction> {
        match action {
            WidgetAction::Control(ControlAction::TextEdited {
                id: PASSWORD_INPUT_ID,
                value: TextPayload::Sensitive(value),
            }) => Some(EncryptedNoteDialogAction::PasswordChanged(value)),
            WidgetAction::Control(ControlAction::TextEdited {
                id: CONFIRMATION_INPUT_ID,
                value: TextPayload::Sensitive(value),
            }) => Some(EncryptedNoteDialogAction::ConfirmationChanged(value)),
            WidgetAction::Control(ControlAction::TextCommitted {
                id: PASSWORD_INPUT_ID,
                value: TextPayload::Sensitive(_),
            }) if self.input.mode.requires_confirmation() => {
                self.password_input.set_focus(false);
                self.confirmation_input.set_focus(true);
                None
            }
            WidgetAction::Control(ControlAction::TextCommitted {
                id: PASSWORD_INPUT_ID | CONFIRMATION_INPUT_ID,
                value: TextPayload::Sensitive(_),
            }) if self.input.can_submit() => Some(EncryptedNoteDialogAction::Submit),
            WidgetAction::Control(ControlAction::FocusRequested { id: PASSWORD_INPUT_ID }) => {
                self.password_input.set_focus(true);
                self.confirmation_input.set_focus(false);
                None
            }
            WidgetAction::Control(ControlAction::FocusRequested { id: CONFIRMATION_INPUT_ID }) => {
                self.password_input.set_focus(false);
                self.confirmation_input.set_focus(true);
                None
            }
            _ => None,
        }
    }
}

fn sensitive_text_box(id: WidgetId, accessibility_label: &str) -> TextBox {
    let mut text_box = TextBox::with_id(id);
    text_box.set_password_mode(true);
    text_box.set_accessibility_label(Some(accessibility_label.to_owned()));
    text_box.set_max_len_bytes(4096);
    text_box.set_blink(true);
    text_box
}

fn labeled_button(id: WidgetId, label: &str, style: ButtonStyle) -> Button {
    let mut button = Button::new(id, style);
    button.set_text(Some(label.to_owned()));
    button.set_accessibility_label(Some(label.to_owned()));
    button
}

#[cfg(test)]
mod tests {
    use super::{
        CONFIRMATION_INPUT_ID, EncryptedNoteDialog, EncryptedNoteDialogAction,
        EncryptedNoteDialogInput, EncryptedNoteDialogMode, PASSWORD_INPUT_ID,
    };
    use crate::WidgetAction;
    use crate::core::widget::{ControlAction, SensitiveText, TextPayload};

    #[test]
    fn sensitive_input_debug_output_is_redacted() {
        let mut input = EncryptedNoteDialogInput::create();
        input.password = SensitiveText::new("never-print-password".to_owned());
        input.confirmation = Some(SensitiveText::new("never-print-confirmation".to_owned()));
        let rendered = format!("{input:?}");

        assert!(!rendered.contains("never-print-password"));
        assert!(!rendered.contains("never-print-confirmation"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn text_actions_preserve_sensitive_payloads() {
        let theme = crate::theme::test_theme();
        let mut dialog = EncryptedNoteDialog::new(&theme);
        let password_action =
            dialog.map_text_action(WidgetAction::Control(ControlAction::TextEdited {
                id: PASSWORD_INPUT_ID,
                value: TextPayload::Sensitive(SensitiveText::new("password-value".to_owned())),
            }));
        let confirmation_action =
            dialog.map_text_action(WidgetAction::Control(ControlAction::TextEdited {
                id: CONFIRMATION_INPUT_ID,
                value: TextPayload::Sensitive(SensitiveText::new("confirmation-value".to_owned())),
            }));

        assert!(matches!(
            password_action,
            Some(EncryptedNoteDialogAction::PasswordChanged(value))
                if value.expose() == "password-value"
        ));
        assert!(matches!(
            confirmation_action,
            Some(EncryptedNoteDialogAction::ConfirmationChanged(value))
                if value.expose() == "confirmation-value"
        ));
    }

    #[test]
    fn conflict_copy_requires_confirmation_without_exposing_the_target_path() {
        let mut input = EncryptedNoteDialogInput::conflict_copy("冲突副本.md".to_owned());
        input.password = SensitiveText::new("copy-password".to_owned());
        assert!(!input.can_submit());

        input.confirmation = Some(SensitiveText::new("copy-password".to_owned()));

        assert!(input.can_submit());
        assert!(matches!(
            input.mode,
            EncryptedNoteDialogMode::ConflictCopy { ref file_name }
                if file_name == "冲突副本.md"
        ));
        assert!(!format!("{input:?}").contains("copy-password"));
    }
}
