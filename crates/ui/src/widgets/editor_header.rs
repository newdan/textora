//! 编辑区文档头部：标题草稿、时间/保存状态、星标、加密状态和删除入口。

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, Modifiers, MouseButton, PaintCtx, Rect, Widget,
    WidgetAction,
};
use crate::widgets::icon::draw_icon;
use crate::widgets::text_box::TextBox;
use std::any::Any;

const HEADER_HORIZONTAL_PADDING_LOGICAL: f32 = 16.0;
const HEADER_VERTICAL_PADDING_LOGICAL: f32 = 8.0;
const HEADER_ACTION_SIZE_LOGICAL: f32 = 28.0;
const HEADER_ACTION_GAP_LOGICAL: f32 = 6.0;
const HEADER_META_GAP_LOGICAL: f32 = 12.0;
const HEADER_META_FONT_SIZE_LOGICAL: f32 = 11.0;
const HEADER_MINIMUM_TITLE_WIDTH_LOGICAL: f32 = 120.0;
const HEADER_TITLE_HEIGHT_LOGICAL: f32 = 40.0;
const HEADER_COMPACT_HEIGHT_THRESHOLD_LOGICAL: f32 = 64.0;
const HEADER_COMPACT_WIDTH_THRESHOLD_LOGICAL: f32 = 420.0;

pub const EDITOR_HEADER_TITLE_ID: WidgetId = WidgetId(10_001);
pub const EDITOR_HEADER_STAR_ID: WidgetId = WidgetId(10_002);
pub const EDITOR_HEADER_DELETE_ID: WidgetId = WidgetId(10_003);
pub const EDITOR_HEADER_CANCEL_TITLE_ID: WidgetId = WidgetId(10_004);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncryptionStatusInput {
    #[default]
    Unencrypted,
    Encrypted,
    Hidden,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorHeaderInput {
    pub title: String,
    pub title_editable: bool,
    pub created_at_text: String,
    pub modified_at_text: String,
    pub save_status_text: String,
    pub starred: bool,
    pub star_enabled: bool,
    pub encryption: EncryptionStatusInput,
    pub delete_visible: bool,
    pub delete_enabled: bool,
    pub compact: bool,
}

pub struct EditorHeaderWidget {
    input: EditorHeaderInput,
    rect: Rect,
    title_box: TextBox,
    star_rect: Rect,
    delete_rect: Rect,
    encryption_rect: Rect,
    metadata_rect: Rect,
    hovered_star: bool,
    hovered_delete: bool,
}

impl EditorHeaderWidget {
    pub fn new() -> Self {
        Self {
            input: EditorHeaderInput::default(),
            rect: Rect::ZERO,
            title_box: TextBox::with_id(EDITOR_HEADER_TITLE_ID),
            star_rect: Rect::ZERO,
            delete_rect: Rect::ZERO,
            encryption_rect: Rect::ZERO,
            metadata_rect: Rect::ZERO,
            hovered_star: false,
            hovered_delete: false,
        }
    }

    pub fn set_input(&mut self, input: EditorHeaderInput) {
        if !self.title_box.is_focused() {
            self.title_box.sync_text(&input.title);
        }
        self.input = input;
    }

    pub fn title_text(&self) -> &str {
        self.title_box.text()
    }

    pub fn title_is_focused(&self) -> bool {
        self.title_box.is_focused()
    }

    pub fn handle_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<ControlAction> {
        if let Event::KeyDown(KeyCode::Escape, Modifiers { .. }) = event
            && self.title_box.is_focused()
        {
            self.title_box.set_text(&self.input.title);
            self.title_box.set_focus(false);
            return Some(ControlAction::Activated { id: EDITOR_HEADER_CANCEL_TITLE_ID });
        }

        if let Event::MouseMove { px, py } = event {
            self.hovered_star = self.input.star_enabled && self.star_rect.contains(*px, *py);
            self.hovered_delete = self.input.delete_visible
                && self.input.delete_enabled
                && self.delete_rect.contains(*px, *py);
            if self.hovered_star || self.hovered_delete {
                context.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                return None;
            }
        }

        if let Event::MouseDown { px, py, button: MouseButton::Left } = event {
            if self.input.star_enabled && self.star_rect.contains(*px, *py) {
                return Some(ControlAction::Activated { id: EDITOR_HEADER_STAR_ID });
            }
            if self.input.delete_visible
                && self.input.delete_enabled
                && self.delete_rect.contains(*px, *py)
            {
                return Some(ControlAction::Activated { id: EDITOR_HEADER_DELETE_ID });
            }
            if self.encryption_rect.contains(*px, *py) {
                return None;
            }
        }

        if !self.input.title_editable && self.title_box.hit_event(event) {
            return None;
        }
        let action = self.title_box.on_event(event, context)?;
        let WidgetAction::Control(control) = action else {
            return None;
        };
        if let ControlAction::TextCommitted { id: EDITOR_HEADER_TITLE_ID, value } = &control
            && let Some(title) = text_payload_string(value)
        {
            self.input.title = title;
        }
        Some(control)
    }
}

impl Default for EditorHeaderWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for EditorHeaderWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        let dpi = ctx.dpi;
        let horizontal_padding = HEADER_HORIZONTAL_PADDING_LOGICAL * dpi;
        let vertical_padding = HEADER_VERTICAL_PADDING_LOGICAL * dpi;
        let action_size = HEADER_ACTION_SIZE_LOGICAL * dpi;
        let action_gap = HEADER_ACTION_GAP_LOGICAL * dpi;
        let encryption_width = match self.input.encryption {
            EncryptionStatusInput::Encrypted | EncryptionStatusInput::Unencrypted => action_size,
            EncryptionStatusInput::Hidden => 0.0,
        };
        let full_metadata_width =
            ctx.measure.measure(&self.full_metadata_text(), HEADER_META_FONT_SIZE_LOGICAL * dpi);
        let mut compact = self.input.compact
            || rect.h / dpi <= HEADER_COMPACT_HEIGHT_THRESHOLD_LOGICAL
            || rect.w / dpi <= HEADER_COMPACT_WIDTH_THRESHOLD_LOGICAL;
        if !compact {
            let trailing_width = trailing_action_width(
                self.input.delete_visible,
                self.input.star_enabled,
                encryption_width,
                action_size,
                action_gap,
            );
            let required_width = horizontal_padding * 2.0
                + HEADER_MINIMUM_TITLE_WIDTH_LOGICAL * dpi
                + HEADER_META_GAP_LOGICAL * dpi
                + full_metadata_width
                + trailing_width;
            compact = rect.w < required_width;
        }
        self.input.compact = compact;
        let mut right_edge = self.rect.right() - horizontal_padding;
        let action_y = (self.rect.h - action_size).max(0.0) * 0.5;

        self.delete_rect = if self.input.delete_visible && !self.input.compact {
            right_edge -= action_size;
            let action_rect = Rect::new(right_edge, action_y, action_size, action_size);
            right_edge -= action_gap;
            action_rect
        } else {
            Rect::ZERO
        };
        self.star_rect = if self.input.star_enabled {
            right_edge -= action_size;
            let action_rect = Rect::new(right_edge, action_y, action_size, action_size);
            right_edge -= action_gap;
            action_rect
        } else {
            Rect::ZERO
        };
        self.encryption_rect = match self.input.encryption {
            EncryptionStatusInput::Encrypted | EncryptionStatusInput::Unencrypted => {
                right_edge -= encryption_width;
                let badge_rect = Rect::new(right_edge, action_y, encryption_width, action_size);
                right_edge -= action_gap;
                badge_rect
            }
            EncryptionStatusInput::Hidden => Rect::ZERO,
        };

        let title_x = horizontal_padding;
        let metadata_text = self.metadata_text();
        self.metadata_rect = if metadata_text.is_empty() {
            Rect::ZERO
        } else {
            let metadata_width = ctx
                .measure
                .measure(&metadata_text, HEADER_META_FONT_SIZE_LOGICAL * dpi)
                .min((right_edge - title_x).max(0.0));
            right_edge -= metadata_width;
            let metadata_rect = Rect::new(right_edge, action_y, metadata_width, action_size);
            right_edge -= HEADER_META_GAP_LOGICAL * dpi;
            metadata_rect
        };
        let title_right = right_edge.max(title_x);
        let title_width = (title_right - title_x).min(self.rect.w - title_x * 2.0).max(0.0);
        let available_title_height = (self.rect.h - vertical_padding * 2.0).max(0.0);
        let title_height = available_title_height.min(HEADER_TITLE_HEIGHT_LOGICAL * dpi);
        let title_y = (self.rect.h - title_height).max(0.0) * 0.5;
        self.title_box.set_rect(Rect::new(title_x, title_y, title_width, title_height), ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        ctx.list.fill(self.rect, ctx.theme.editor.background);
        self.title_box.paint(ctx);

        let dpi = ctx.dpi;
        let metadata_text = self.metadata_text();
        if !metadata_text.is_empty() {
            let baseline =
                self.rect.y + self.rect.h * 0.5 + HEADER_META_FONT_SIZE_LOGICAL * dpi * 0.35;
            ctx.text(
                self.metadata_rect.x,
                baseline,
                HEADER_META_FONT_SIZE_LOGICAL * dpi,
                ctx.theme.palette.text_muted,
                &metadata_text,
            );
        }

        self.paint_action_button(
            ctx,
            self.star_rect,
            self.hovered_star,
            "star",
            self.input.starred,
        );
        self.paint_action_button(ctx, self.delete_rect, self.hovered_delete, "trash-2", false);
        if self.encryption_rect.w > 0.0 {
            self.paint_action_button(
                ctx,
                self.encryption_rect,
                false,
                self.encryption_icon(),
                self.input.encryption == EncryptionStatusInput::Encrypted,
            );
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        Some(EDITOR_HEADER_TITLE_ID)
    }

    fn is_focusable(&self) -> bool {
        self.input.title_editable
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.title_box.set_keyboard_focus(focused_id);
    }

    fn on_event(&mut self, event: &Event, context: &mut EventCtx) -> Option<WidgetAction> {
        self.handle_event(event, context).map(WidgetAction::Control)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl EditorHeaderWidget {
    fn encryption_icon(&self) -> &'static str {
        match self.input.encryption {
            EncryptionStatusInput::Encrypted => "lock",
            EncryptionStatusInput::Unencrypted => "lock-open",
            EncryptionStatusInput::Hidden => "",
        }
    }

    fn metadata_text(&self) -> String {
        if self.input.compact {
            return self.input.save_status_text.clone();
        }
        self.full_metadata_text()
    }

    fn full_metadata_text(&self) -> String {
        [
            self.input.created_at_text.as_str(),
            self.input.modified_at_text.as_str(),
            self.input.save_status_text.as_str(),
        ]
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
    }

    fn paint_action_button(
        &self,
        ctx: &mut PaintCtx,
        rect: Rect,
        hovered: bool,
        icon: &str,
        selected: bool,
    ) {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        if hovered || selected {
            ctx.list.fill_rounded(rect, ctx.theme.palette.bg_hover, 6.0 * ctx.dpi);
        }
        let icon_size = 16.0 * ctx.dpi;
        let icon_color =
            if selected { ctx.theme.palette.accent } else { ctx.theme.palette.text_muted };
        draw_icon(
            ctx.list,
            icon,
            rect.x + (rect.w - icon_size) * 0.5,
            rect.y + (rect.h - icon_size) * 0.5,
            icon_size,
            icon_color,
        );
    }
}

fn trailing_action_width(
    delete_visible: bool,
    star_visible: bool,
    encryption_width: f32,
    action_size: f32,
    action_gap: f32,
) -> f32 {
    let mut width = 0.0;
    if delete_visible {
        width += action_size + action_gap;
    }
    if star_visible {
        width += action_size + action_gap;
    }
    if encryption_width > 0.0 {
        width += encryption_width + action_gap;
    }
    width
}

fn text_payload_string(payload: &TextPayload) -> Option<String> {
    match payload {
        TextPayload::Plain(value) => Some(value.clone()),
        TextPayload::Sensitive(_) => None,
    }
}

trait TextBoxEventSupport {
    fn hit_event(&self, event: &Event) -> bool;
}

impl TextBoxEventSupport for TextBox {
    fn hit_event(&self, event: &Event) -> bool {
        match event {
            Event::MouseMove { px, py } | Event::MouseDown { px, py, .. } => self.hit(*px, *py),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::KeyCode;

    struct HeaderMeasure;

    impl crate::core::TextMeasure for HeaderMeasure {
        fn measure(&mut self, text: &str, font_size: f32) -> f32 {
            text.chars().count() as f32 * font_size * 0.6
        }
    }

    fn input() -> EditorHeaderInput {
        EditorHeaderInput {
            title: "原始标题".to_owned(),
            title_editable: true,
            created_at_text: "创建于 2026-08-03".to_owned(),
            modified_at_text: "修改于刚刚".to_owned(),
            save_status_text: "已保存".to_owned(),
            starred: false,
            star_enabled: true,
            encryption: EncryptionStatusInput::Unencrypted,
            delete_visible: true,
            delete_enabled: true,
            compact: false,
        }
    }

    #[test]
    fn escape_cancels_title_draft_without_emitting_a_document_commit() {
        let mut header = EditorHeaderWidget::new();
        header.set_input(input());
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        header.set_keyboard_focus(Some(EDITOR_HEADER_TITLE_ID));
        let _ = header.handle_event(
            &Event::KeyDown(KeyCode::Char('x'), crate::core::Modifiers::NONE),
            &mut event_context,
        );
        let action = header.handle_event(
            &Event::KeyDown(KeyCode::Escape, crate::core::Modifiers::NONE),
            &mut event_context,
        );

        assert_eq!(header.title_text(), "原始标题");
        assert_eq!(action, Some(ControlAction::Activated { id: EDITOR_HEADER_CANCEL_TITLE_ID }));
    }

    #[test]
    fn enter_commits_title_and_buttons_emit_only_their_stable_ids() {
        let mut header = EditorHeaderWidget::new();
        header.set_input(input());
        header.set_keyboard_focus(Some(EDITOR_HEADER_TITLE_ID));
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        let commit = header.handle_event(
            &Event::KeyDown(KeyCode::Enter, crate::core::Modifiers::NONE),
            &mut event_context,
        );
        assert!(matches!(
            commit,
            Some(ControlAction::TextCommitted { id: EDITOR_HEADER_TITLE_ID, .. })
        ));

        let mut measure = HeaderMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        header.set_rect(Rect::new(0.0, 0.0, 640.0, 128.0), &mut layout_context);

        assert_eq!(
            header.handle_event(
                &Event::MouseDown {
                    px: header.star_rect.x + 2.0,
                    py: header.star_rect.y + 2.0,
                    button: MouseButton::Left,
                },
                &mut event_context,
            ),
            Some(ControlAction::Activated { id: EDITOR_HEADER_STAR_ID })
        );
        assert_eq!(
            header.handle_event(
                &Event::MouseDown {
                    px: header.delete_rect.x + 2.0,
                    py: header.delete_rect.y + 2.0,
                    button: MouseButton::Left,
                },
                &mut event_context,
            ),
            Some(ControlAction::Activated { id: EDITOR_HEADER_DELETE_ID })
        );
    }

    #[test]
    fn compact_header_keeps_save_star_and_encryption_but_moves_delete_out() {
        let mut header = EditorHeaderWidget::new();
        header.set_input(input());
        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };

        header.set_rect(Rect::new(0.0, 0.0, 320.0, 56.0), &mut layout_context);

        assert!(header.input.compact);
        assert_eq!(header.delete_rect, Rect::ZERO);
        assert!(header.star_rect.w > 0.0);
        assert!(header.encryption_rect.w > 0.0);
        assert_eq!(header.metadata_text(), "已保存");
        assert!(header.title_box.rect().w >= 0.0);
    }

    #[test]
    fn wide_header_reserves_measured_metadata_space_before_trailing_actions() {
        let mut header = EditorHeaderWidget::new();
        header.set_input(input());
        let theme = crate::theme::test_theme();
        let mut measure = HeaderMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };

        header.set_rect(Rect::new(0.0, 0.0, 760.0, 72.0), &mut layout_context);

        assert!(!header.input.compact);
        assert_eq!(header.title_box.rect().h, HEADER_TITLE_HEIGHT_LOGICAL);
        assert_eq!(header.title_box.rect().y, 16.0);
        assert_eq!(header.star_rect.y, 22.0);
        assert_eq!(header.metadata_rect.y, header.star_rect.y);
        assert!(header.metadata_rect.w > 0.0);
        assert!(
            header.title_box.rect().right() + HEADER_META_GAP_LOGICAL <= header.metadata_rect.x
        );
        assert!(
            header.metadata_rect.right() + HEADER_ACTION_GAP_LOGICAL <= header.encryption_rect.x
        );
    }

    #[test]
    fn encrypted_status_is_read_only_and_does_not_receive_focus_or_actions() {
        let mut encrypted_input = input();
        encrypted_input.title_editable = false;
        encrypted_input.encryption = EncryptionStatusInput::Encrypted;
        let mut header = EditorHeaderWidget::new();
        header.set_input(encrypted_input);
        assert!(!header.is_focusable());

        let theme = crate::theme::test_theme();
        let mut measure = crate::core::NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        header.set_rect(Rect::new(0.0, 0.0, 640.0, 56.0), &mut layout_context);
        let mut event_context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        assert_eq!(
            header.handle_event(
                &Event::MouseDown {
                    px: header.encryption_rect.x + 2.0,
                    py: header.encryption_rect.y + 2.0,
                    button: MouseButton::Left,
                },
                &mut event_context,
            ),
            None
        );
        assert_eq!(event_context.cursor_hint, None);
    }
}
