//! 新建工作区的产品级模态框；只接收纯展示输入并输出类型化交互。

use std::path::PathBuf;

use ui::WidgetAction;
use ui::button::{Button, ButtonStyle};
use ui::core::widget::{ControlAction, TextPayload, WidgetId};
use ui::core::{Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget};
use ui::label::{Label, LabelForeground, LabelStyle};
use ui::text_box::TextBox;

const NAME_INPUT_ID: WidgetId = WidgetId(9_100);
const LOCATION_BUTTON_ID: WidgetId = WidgetId(9_101);
const CREATE_BUTTON_ID: WidgetId = WidgetId(9_102);
const CANCEL_BUTTON_ID: WidgetId = WidgetId(9_103);
const PANEL_WIDTH_LOGICAL: f32 = 520.0;
const PANEL_HEIGHT_LOGICAL: f32 = 310.0;
const PANEL_MARGIN_LOGICAL: f32 = 24.0;
const FIELD_HEIGHT_LOGICAL: f32 = 34.0;
const BUTTON_WIDTH_LOGICAL: f32 = ui::button::ButtonMetrics::text_width(2);
const LOCATION_BUTTON_WIDTH_LOGICAL: f32 = ui::button::ButtonMetrics::text_width(4);
const BUTTON_GAP_LOGICAL: f32 = ui::button::ButtonMetrics::ACTION_GAP;
const CAPTION_FONT_SIZE_LOGICAL: f32 = 12.0;
const CAPTION_HEIGHT_LOGICAL: f32 = 18.0;
const LOCATION_LABEL_GAP_LOGICAL: f32 = 10.0;
const PREVIEW_LABEL_TOP_LOGICAL: f32 = 189.0;
const ERROR_LABEL_TOP_LOGICAL: f32 = 213.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewWorkspaceDialogInput {
    pub name: String,
    pub parent_directory: Option<PathBuf>,
    pub error_message: Option<String>,
}

impl NewWorkspaceDialogInput {
    pub fn target_path(&self) -> Option<PathBuf> {
        let name = notora_core::validate_workspace_directory_name(&self.name).ok()?;
        self.parent_directory.as_ref().map(|parent| parent.join(name))
    }

    fn can_create(&self) -> bool {
        self.target_path().is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewWorkspaceDialogAction {
    NameChanged(String),
    ChooseLocation,
    Create,
    Cancel,
}

pub struct NewWorkspaceDialog {
    input: NewWorkspaceDialogInput,
    name_input: TextBox,
    location_button: Button,
    create_button: Button,
    cancel_button: Button,
    location_label: Label,
    preview_label: Label,
    error_label: Label,
    panel_rect: Rect,
    open: bool,
}

impl NewWorkspaceDialog {
    pub fn new(theme: &ui::Theme) -> Self {
        let mut name_input = TextBox::with_id(NAME_INPUT_ID);
        name_input.set_placeholder("例如：我的笔记");
        name_input.set_accessibility_label(Some("工作区名称".to_owned()));
        name_input.set_max_len_bytes(255);
        let style = ButtonStyle::from_theme(theme);
        Self {
            input: NewWorkspaceDialogInput::default(),
            name_input,
            location_button: labeled_button(LOCATION_BUTTON_ID, "选择位置", style.clone()),
            create_button: labeled_button(CREATE_BUTTON_ID, "创建", style.clone()),
            cancel_button: labeled_button(CANCEL_BUTTON_ID, "取消", style),
            location_label: clipped_label("尚未选择"),
            preview_label: clipped_label("选择名称和保存位置后显示最终路径"),
            error_label: clipped_label(""),
            panel_rect: Rect::ZERO,
            open: false,
        }
    }

    pub fn set_input(&mut self, input: NewWorkspaceDialogInput, open: bool) {
        self.input = input;
        self.name_input.sync_text(&self.input.name);
        self.create_button.set_enabled(self.input.can_create());
        let location = self
            .input
            .parent_directory
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "尚未选择".to_owned());
        self.location_label.set_text(location);
        let preview = self
            .input
            .target_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "选择名称和保存位置后显示最终路径".to_owned());
        self.preview_label.set_text(format!("最终路径：{preview}"));
        self.error_label.set_text(self.input.error_message.clone().unwrap_or_default());
        if open && !self.open {
            self.name_input.set_focus(true);
            self.name_input.select_all();
        }
        if !open {
            self.name_input.set_focus(false);
        }
        self.open = open;
    }

    pub fn set_rect(&mut self, overlay_rect: Rect, context: &mut LayoutCtx<'_>) {
        let margin = PANEL_MARGIN_LOGICAL * context.dpi;
        let width =
            (PANEL_WIDTH_LOGICAL * context.dpi).min((overlay_rect.w - margin * 2.0).max(0.0));
        let height =
            (PANEL_HEIGHT_LOGICAL * context.dpi).min((overlay_rect.h - margin * 2.0).max(0.0));
        self.panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - width) * 0.5,
            overlay_rect.y + (overlay_rect.h - height) * 0.5,
            width,
            height,
        );
        let horizontal_padding = 24.0 * context.dpi;
        let field_height = FIELD_HEIGHT_LOGICAL * context.dpi;
        let content_width = (width - horizontal_padding * 2.0).max(0.0);
        self.name_input.set_rect(
            Rect::new(
                self.panel_rect.x + horizontal_padding,
                self.panel_rect.y + 66.0 * context.dpi,
                content_width,
                field_height,
            ),
            context,
        );
        self.location_button.set_rect(
            Rect::new(
                self.panel_rect.x + horizontal_padding,
                self.panel_rect.y + 142.0 * context.dpi,
                LOCATION_BUTTON_WIDTH_LOGICAL * context.dpi,
                field_height,
            ),
            context,
        );
        let button_width = BUTTON_WIDTH_LOGICAL * context.dpi;
        let button_gap = BUTTON_GAP_LOGICAL * context.dpi;
        let button_y = self.panel_rect.bottom() - horizontal_padding - field_height;
        self.create_button.set_rect(
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
                self.create_button.rect().x - button_gap - button_width,
                button_y,
                button_width,
                field_height,
            ),
            context,
        );
        let caption_style = dialog_caption_style(context.theme.application_theme().text_secondary);
        self.location_label.set_style(caption_style.clone());
        self.preview_label.set_style(caption_style);
        self.error_label.set_style(dialog_caption_style(context.theme.application_theme().danger));
        self.location_label.set_rect(
            Rect::new(
                self.location_button.rect().right() + LOCATION_LABEL_GAP_LOGICAL * context.dpi,
                self.location_button.rect().y,
                (self.panel_rect.right()
                    - horizontal_padding
                    - self.location_button.rect().right()
                    - LOCATION_LABEL_GAP_LOGICAL * context.dpi)
                    .max(0.0),
                field_height,
            ),
            context,
        );
        self.preview_label.set_rect(
            Rect::new(
                self.panel_rect.x + horizontal_padding,
                self.panel_rect.y + PREVIEW_LABEL_TOP_LOGICAL * context.dpi,
                content_width,
                CAPTION_HEIGHT_LOGICAL * context.dpi,
            ),
            context,
        );
        self.error_label.set_rect(
            Rect::new(
                self.panel_rect.x + horizontal_padding,
                self.panel_rect.y + ERROR_LABEL_TOP_LOGICAL * context.dpi,
                content_width,
                CAPTION_HEIGHT_LOGICAL * context.dpi,
            ),
            context,
        );
    }

    pub fn paint(&self, context: &mut PaintCtx<'_>) {
        let theme = context.theme.application_theme();
        context.list.fill_rounded(self.panel_rect, theme.overlay_surface, 10.0 * context.dpi);
        let left = self.panel_rect.x + 24.0 * context.dpi;
        context.text(
            left,
            self.panel_rect.y + 34.0 * context.dpi,
            18.0 * context.dpi,
            theme.text_primary,
            "新建工作区",
        );
        context.text(
            left,
            self.panel_rect.y + 58.0 * context.dpi,
            12.0 * context.dpi,
            theme.text_secondary,
            "工作区名称",
        );
        self.name_input.paint(context);
        context.text(
            left,
            self.panel_rect.y + 132.0 * context.dpi,
            12.0 * context.dpi,
            theme.text_secondary,
            "保存位置",
        );
        self.location_button.paint(context);
        self.location_label.paint(context);
        self.preview_label.paint(context);
        if self.input.error_message.is_some() {
            self.error_label.paint(context);
        }
        self.cancel_button.paint(context);
        self.create_button.paint(context);
    }

    pub fn route_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<NewWorkspaceDialogAction> {
        if matches!(event, Event::KeyDown(KeyCode::Escape, _)) {
            return Some(NewWorkspaceDialogAction::Cancel);
        }
        if let Event::MouseDown { px, py, button: MouseButton::Left } = event
            && !self.panel_rect.contains(*px, *py)
        {
            return Some(NewWorkspaceDialogAction::Cancel);
        }
        if let Some(action) = self.name_input.on_event(event, context) {
            return self.map_name_action(action);
        }
        for (button, action) in [
            (&mut self.location_button, NewWorkspaceDialogAction::ChooseLocation),
            (&mut self.create_button, NewWorkspaceDialogAction::Create),
            (&mut self.cancel_button, NewWorkspaceDialogAction::Cancel),
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
        self.name_input.is_focused().then(|| self.name_input.ime_cursor_rect())
    }

    pub fn next_cursor_blink_at(&self) -> Option<std::time::Instant> {
        self.name_input.next_cursor_blink_at()
    }

    pub fn advance_cursor_blink(&mut self, now: std::time::Instant) -> bool {
        self.name_input.advance_cursor_blink(now)
    }

    fn map_name_action(&mut self, action: WidgetAction) -> Option<NewWorkspaceDialogAction> {
        match action {
            WidgetAction::Control(ControlAction::TextEdited {
                id: NAME_INPUT_ID,
                value: TextPayload::Plain(value),
            }) => Some(NewWorkspaceDialogAction::NameChanged(value)),
            WidgetAction::Control(ControlAction::TextCommitted { id: NAME_INPUT_ID, .. })
                if self.input.can_create() =>
            {
                Some(NewWorkspaceDialogAction::Create)
            }
            WidgetAction::Control(ControlAction::FocusRequested { id: NAME_INPUT_ID }) => {
                self.name_input.set_focus(true);
                None
            }
            _ => None,
        }
    }
}

fn labeled_button(id: WidgetId, label: &str, style: ButtonStyle) -> Button {
    let mut button = Button::new(id, style);
    button.set_text(Some(label.to_owned()));
    button.set_accessibility_label(Some(label.to_owned()));
    button
}

fn clipped_label(text: &str) -> Label {
    let mut label = Label::new(text, LabelStyle::default());
    label.set_clip_to_bounds(true);
    label
}

fn dialog_caption_style(color: [f32; 4]) -> LabelStyle {
    LabelStyle {
        font_size_logical: CAPTION_FONT_SIZE_LOGICAL,
        foreground: LabelForeground::Explicit(color),
        ..LabelStyle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{NewWorkspaceDialog, NewWorkspaceDialogInput};
    use ui::core::measure::NoopMeasure;
    use ui::core::paint::{DrawCmd, DrawList};
    use ui::core::{LayoutCtx, PaintCtx, Rect};

    #[test]
    fn target_preview_joins_the_selected_parent_and_valid_name() {
        let input = NewWorkspaceDialogInput {
            name: "笔记库".to_owned(),
            parent_directory: Some("/tmp".into()),
            error_message: None,
        };

        assert_eq!(input.target_path(), Some("/tmp/笔记库".into()));
    }

    #[test]
    fn invalid_trailing_space_has_no_creation_preview() {
        let input = NewWorkspaceDialogInput {
            name: "笔记库 ".to_owned(),
            parent_directory: Some("/tmp".into()),
            error_message: None,
        };

        assert_eq!(input.target_path(), None);
    }

    #[test]
    fn long_location_and_preview_are_clipped_within_panel() {
        let theme = ui::theme::test_theme();
        let mut dialog = NewWorkspaceDialog::new(&theme);
        dialog.set_input(
            NewWorkspaceDialogInput {
                name: "笔记库".to_owned(),
                parent_directory: Some(format!("/tmp/{}", "long-directory/".repeat(30)).into()),
                error_message: None,
            },
            true,
        );
        let mut measure = NoopMeasure;
        dialog.set_rect(
            Rect::new(0.0, 0.0, 640.0, 480.0),
            &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 },
        );
        let mut draw_list = DrawList::new();
        dialog.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));

        let clip_rects: Vec<_> = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::PushClip(rect) => Some(*rect),
                _ => None,
            })
            .collect();
        assert!(clip_rects.len() >= 2);
        assert!(clip_rects.iter().all(|rect| rect.right() <= dialog.panel_rect.right()));
    }
}
