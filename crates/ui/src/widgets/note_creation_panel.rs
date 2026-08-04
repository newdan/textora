//! 新建笔记面板；产品层注入通用选项，不携带领域类型。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use std::any::Any;

const CREATION_PANEL_ROW_HEIGHT_LOGICAL: f32 = 28.0;
const CREATION_PANEL_SECTION_GAP_LOGICAL: f32 = 8.0;
const CREATION_PANEL_HORIZONTAL_PADDING_LOGICAL: f32 = 12.0;
const CREATION_PANEL_FONT_SIZE_LOGICAL: f32 = 13.0;

pub const NOTE_CREATION_TYPE_ID: WidgetId = WidgetId(10_401);
pub const NOTE_CREATION_DIRECTORY_ID: WidgetId = WidgetId(10_402);
pub const NOTE_CREATION_STORAGE_ID: WidgetId = WidgetId(10_403);
pub const NOTE_CREATION_SUBMIT_ID: WidgetId = WidgetId(10_404);
pub const NOTE_CREATION_CANCEL_ID: WidgetId = WidgetId(10_405);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteCreationOptionInput {
    pub option_key: String,
    pub label: String,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoteCreationSubmissionState {
    #[default]
    Idle,
    Submitting,
    Failed,
    Succeeded,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteCreationPanelInput {
    pub document_types: Vec<NoteCreationOptionInput>,
    pub directories: Vec<NoteCreationOptionInput>,
    pub storage_modes: Vec<NoteCreationOptionInput>,
    pub selected_document_type: Option<String>,
    pub selected_directory: Option<String>,
    pub selected_storage_mode: Option<String>,
    pub submission: NoteCreationSubmissionState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteCreationPanelAction {
    DocumentTypeSelected(String),
    DirectorySelected(String),
    StorageModeSelected(String),
    Confirmed,
    Cancelled,
}

pub struct NoteCreationPanelWidget {
    input: NoteCreationPanelInput,
    rect: Rect,
    focused_section: usize,
}

impl NoteCreationPanelWidget {
    pub fn new() -> Self {
        Self { input: NoteCreationPanelInput::default(), rect: Rect::ZERO, focused_section: 0 }
    }

    pub fn set_input(&mut self, input: NoteCreationPanelInput) {
        self.input = input;
    }

    pub fn event_action(&mut self, event: &Event, dpi: f32) -> Option<NoteCreationPanelAction> {
        if self.input.submission == NoteCreationSubmissionState::Submitting {
            return None;
        }
        match event {
            Event::KeyDown(crate::core::KeyCode::Escape, _) => {
                Some(NoteCreationPanelAction::Cancelled)
            }
            Event::KeyDown(crate::core::KeyCode::Tab, modifiers) => {
                let section_count = self.section_count().max(1);
                if modifiers.shift {
                    self.focused_section =
                        (self.focused_section + section_count - 1) % section_count;
                } else {
                    self.focused_section = (self.focused_section + 1) % section_count;
                }
                None
            }
            Event::KeyDown(crate::core::KeyCode::Enter, _) => {
                self.creation_ready().then_some(NoteCreationPanelAction::Confirmed)
            }
            Event::MouseDown { px, py, button: crate::core::MouseButton::Left } => {
                if !self.rect.contains(*px, *py) {
                    return Some(NoteCreationPanelAction::Cancelled);
                }
                let (section, option_index) = self.option_at(*py, dpi)?;
                let option = self.option(section, option_index)?;
                if !option.enabled {
                    return None;
                }
                match section {
                    0 => Some(NoteCreationPanelAction::DocumentTypeSelected(
                        option.option_key.clone(),
                    )),
                    1 => {
                        Some(NoteCreationPanelAction::DirectorySelected(option.option_key.clone()))
                    }
                    _ => Some(NoteCreationPanelAction::StorageModeSelected(
                        option.option_key.clone(),
                    )),
                }
            }
            _ => None,
        }
    }

    fn section_count(&self) -> usize {
        [&self.input.document_types, &self.input.directories, &self.input.storage_modes]
            .into_iter()
            .filter(|options| !options.is_empty())
            .count()
    }

    fn creation_ready(&self) -> bool {
        self.input.selected_document_type.is_some()
            && self.input.selected_directory.is_some()
            && self.input.selected_storage_mode.is_some()
            && matches!(
                self.input.submission,
                NoteCreationSubmissionState::Idle | NoteCreationSubmissionState::Failed
            )
    }

    fn option(&self, section: usize, index: usize) -> Option<&NoteCreationOptionInput> {
        match section {
            0 => self.input.document_types.get(index),
            1 => self.input.directories.get(index),
            _ => self.input.storage_modes.get(index),
        }
    }

    fn option_at(&self, py: f32, dpi: f32) -> Option<(usize, usize)> {
        let mut top = self.rect.y;
        for (section, options) in
            [&self.input.document_types, &self.input.directories, &self.input.storage_modes]
                .into_iter()
                .enumerate()
        {
            if options.is_empty() {
                continue;
            }
            top += CREATION_PANEL_SECTION_GAP_LOGICAL * dpi;
            for (index, _) in options.iter().enumerate() {
                let row = Rect::new(
                    self.rect.x,
                    top,
                    self.rect.w,
                    CREATION_PANEL_ROW_HEIGHT_LOGICAL * dpi,
                );
                if row.contains(self.rect.x + 1.0, py) {
                    return Some((section, index));
                }
                top += CREATION_PANEL_ROW_HEIGHT_LOGICAL * dpi;
            }
        }
        None
    }
}

impl Default for NoteCreationPanelWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for NoteCreationPanelWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        ctx.list.fill_rounded(self.rect, ctx.theme.palette.bg_surface, 8.0 * ctx.dpi);
        let mut top = self.rect.y;
        for options in
            [&self.input.document_types, &self.input.directories, &self.input.storage_modes]
        {
            if options.is_empty() {
                continue;
            }
            top += CREATION_PANEL_SECTION_GAP_LOGICAL * ctx.dpi;
            for option in options {
                let baseline = top
                    + CREATION_PANEL_ROW_HEIGHT_LOGICAL * ctx.dpi * 0.5
                    + CREATION_PANEL_FONT_SIZE_LOGICAL * ctx.dpi * 0.35;
                let color = if option.enabled {
                    ctx.theme.palette.text_main
                } else {
                    ctx.theme.palette.text_muted
                };
                ctx.text(
                    self.rect.x + CREATION_PANEL_HORIZONTAL_PADDING_LOGICAL * ctx.dpi,
                    baseline,
                    CREATION_PANEL_FONT_SIZE_LOGICAL * ctx.dpi,
                    color,
                    &option.label,
                );
                top += CREATION_PANEL_ROW_HEIGHT_LOGICAL * ctx.dpi;
            }
        }
        if self.input.submission == NoteCreationSubmissionState::Failed {
            ctx.text(
                self.rect.x + CREATION_PANEL_HORIZONTAL_PADDING_LOGICAL * ctx.dpi,
                self.rect.bottom() - CREATION_PANEL_FONT_SIZE_LOGICAL * ctx.dpi,
                CREATION_PANEL_FONT_SIZE_LOGICAL * ctx.dpi,
                ctx.theme.palette.danger,
                "创建失败，请重试",
            );
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        self.event_action(event, ctx.dpi).map(|action| match action {
            NoteCreationPanelAction::DocumentTypeSelected(key) => {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: NOTE_CREATION_TYPE_ID,
                    value: TextPayload::Plain(key),
                })
            }
            NoteCreationPanelAction::DirectorySelected(key) => {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: NOTE_CREATION_DIRECTORY_ID,
                    value: TextPayload::Plain(key),
                })
            }
            NoteCreationPanelAction::StorageModeSelected(key) => {
                WidgetAction::Control(ControlAction::TextCommitted {
                    id: NOTE_CREATION_STORAGE_ID,
                    value: TextPayload::Plain(key),
                })
            }
            NoteCreationPanelAction::Confirmed => {
                WidgetAction::Control(ControlAction::Activated { id: NOTE_CREATION_SUBMIT_ID })
            }
            NoteCreationPanelAction::Cancelled => {
                WidgetAction::Control(ControlAction::Activated { id: NOTE_CREATION_CANCEL_ID })
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

    fn panel_input() -> NoteCreationPanelInput {
        NoteCreationPanelInput {
            document_types: vec![NoteCreationOptionInput {
                option_key: "markdown".to_owned(),
                label: "Markdown".to_owned(),
                enabled: true,
            }],
            directories: vec![NoteCreationOptionInput {
                option_key: "root".to_owned(),
                label: "工作区根目录".to_owned(),
                enabled: true,
            }],
            storage_modes: vec![NoteCreationOptionInput {
                option_key: "plain".to_owned(),
                label: "普通存储".to_owned(),
                enabled: true,
            }],
            selected_document_type: Some("markdown".to_owned()),
            selected_directory: Some("root".to_owned()),
            selected_storage_mode: Some("plain".to_owned()),
            submission: NoteCreationSubmissionState::Idle,
        }
    }

    #[test]
    fn creation_panel_escape_cancels_and_enter_confirms_only_when_ready() {
        let mut panel = NoteCreationPanelWidget::new();
        panel.set_input(panel_input());

        assert_eq!(
            panel.event_action(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), 1.0),
            Some(NoteCreationPanelAction::Cancelled)
        );
        assert_eq!(
            panel.event_action(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), 1.0),
            Some(NoteCreationPanelAction::Confirmed)
        );
    }
}
