use crate::constants::{BAR_HEIGHT, BODY_FONT_SIZE, CLOSE_BTN_SIZE, H_PADDING, SMALL_GAP};
use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::theme::SettingsTheme;
use crate::widgets::button::{Button, ButtonStyle};
use crate::widgets::label::{Label, LabelForeground, LabelStyle};
use std::any::Any;
use std::borrow::Cow;

const DEFAULT_MODAL_BORDER_WIDTH_LOGICAL: f32 = 1.0;
const DEFAULT_MODAL_CORNER_RADIUS_LOGICAL: f32 = 8.0;
const DEFAULT_MODAL_CONTENT_PADDING_LOGICAL: f32 = H_PADDING;
const DEFAULT_MODAL_HEADER_GAP_LOGICAL: f32 = SMALL_GAP;
const DEFAULT_MODAL_HEADER_HEIGHT_LOGICAL: f32 = BAR_HEIGHT;
const DEFAULT_MODAL_TITLE_FONT_SIZE_LOGICAL: f32 = BODY_FONT_SIZE;
const MODAL_FRAME_CLOSE_BUTTON_ID: WidgetId = WidgetId(0x6d6f_6461_6c5f_636c);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerTarget {
    CloseButton,
    Content,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModalFrameStyle {
    pub header_height_logical: f32,
    pub content_padding_logical: f32,
    pub header_gap_logical: f32,
    pub border_width_logical: f32,
    pub corner_radius_logical: f32,
    pub close_button_size_logical: f32,
    pub close_icon_size_logical: f32,
    pub title_font_size_logical: f32,
}

impl Default for ModalFrameStyle {
    fn default() -> Self {
        Self {
            header_height_logical: DEFAULT_MODAL_HEADER_HEIGHT_LOGICAL,
            content_padding_logical: DEFAULT_MODAL_CONTENT_PADDING_LOGICAL,
            header_gap_logical: DEFAULT_MODAL_HEADER_GAP_LOGICAL,
            border_width_logical: DEFAULT_MODAL_BORDER_WIDTH_LOGICAL,
            corner_radius_logical: DEFAULT_MODAL_CORNER_RADIUS_LOGICAL,
            close_button_size_logical: BAR_HEIGHT,
            close_icon_size_logical: CLOSE_BTN_SIZE,
            title_font_size_logical: DEFAULT_MODAL_TITLE_FONT_SIZE_LOGICAL,
        }
    }
}

pub struct ModalFrame {
    rect: Rect,
    header_rect: Rect,
    title_rect: Rect,
    close_button_rect: Rect,
    content_rect: Rect,
    title: Label,
    close_button: Button,
    content: Box<dyn Widget>,
    style: ModalFrameStyle,
    focused_id: Option<WidgetId>,
    pointer_target: Option<PointerTarget>,
    hover_target: Option<PointerTarget>,
}

impl ModalFrame {
    pub fn new(title: impl Into<String>, content: Box<dyn Widget>) -> Self {
        Self::with_style(title, content, ModalFrameStyle::default())
    }

    pub fn with_style(
        title: impl Into<String>,
        content: Box<dyn Widget>,
        style: ModalFrameStyle,
    ) -> Self {
        let title = Label::new(
            title,
            LabelStyle {
                font_size_logical: style.title_font_size_logical,
                foreground: LabelForeground::ThemeMain,
                ..LabelStyle::default()
            },
        );
        let close_button = Self::build_close_button(&style, None);

        Self {
            rect: Rect::ZERO,
            header_rect: Rect::ZERO,
            title_rect: Rect::ZERO,
            close_button_rect: Rect::ZERO,
            content_rect: Rect::ZERO,
            title,
            close_button,
            content,
            style,
            focused_id: None,
            pointer_target: None,
            hover_target: None,
        }
    }

    pub fn content_as_any_mut(&mut self) -> &mut dyn Any {
        self.content.as_any_mut()
    }

    pub fn content_as_any(&self) -> &dyn Any {
        self.content.as_any()
    }

    pub fn content_rect(&self) -> Rect {
        self.content_rect
    }

    fn build_close_button(style: &ModalFrameStyle, tokens: Option<SettingsTheme>) -> Button {
        let mut close_button = Button::new(
            MODAL_FRAME_CLOSE_BUTTON_ID,
            Self::close_button_style(style, tokens.unwrap_or_else(Self::fallback_settings_theme)),
        );
        close_button.set_icon(Some("x".into()));
        close_button.set_icon_size(style.close_icon_size_logical);
        close_button
    }

    fn fallback_settings_theme() -> SettingsTheme {
        crate::theme::test_theme().settings_theme()
    }

    fn close_button_style(style: &ModalFrameStyle, settings: SettingsTheme) -> ButtonStyle {
        ButtonStyle {
            font_size_logical: style.title_font_size_logical,
            pad_x_logical: 0.0,
            foreground: settings.text_secondary,
            selected_foreground: settings.text_secondary,
            background: [0.0, 0.0, 0.0, 0.0],
            border: [0.0, 0.0, 0.0, 0.0],
            hover_background: settings.control_surface,
            pressed_background: settings.control_border,
            selected_background: settings.control_surface,
            disabled_foreground: settings.text_secondary,
            disabled_background: [0.0, 0.0, 0.0, 0.0],
            corner_radius_logical: style.corner_radius_logical * 0.5,
        }
    }

    fn logical_to_px(logical_value: f32, dpi: f32) -> f32 {
        logical_value.max(0.0) * dpi
    }

    fn local_event<'a>(event: &'a Event, child_rect: Rect) -> Cow<'a, Event> {
        crate::core::dock::Dock::to_local(event, child_rect.x, child_rect.y)
    }

    fn dispatch_to_close_button(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let local_event = Self::local_event(event, self.close_button_rect);
        let action = self.close_button.on_event(&local_event, ctx)?;

        match action {
            WidgetAction::Control(ControlAction::Activated { id })
                if id == MODAL_FRAME_CLOSE_BUTTON_ID =>
            {
                Some(WidgetAction::Overlay(crate::OverlayAction::DismissRequested))
            }
            _ => Some(action),
        }
    }

    fn dispatch_to_content(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let local_event = Self::local_event(event, self.content_rect);
        self.content.on_event(&local_event, ctx)
    }

    fn pointer_target_at(&self, px: f32, py: f32) -> Option<PointerTarget> {
        if self.close_button_rect.contains(px, py) {
            return Some(PointerTarget::CloseButton);
        }
        if self.content_rect.contains(px, py) {
            return Some(PointerTarget::Content);
        }
        None
    }

    fn content_is_capturing(&self) -> bool {
        self.content.is_capturing()
    }

    fn dispatch_to_target(
        &mut self,
        target: PointerTarget,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        match target {
            PointerTarget::CloseButton => self.dispatch_to_close_button(event, ctx),
            PointerTarget::Content => self.dispatch_to_content(event, ctx),
        }
    }

    fn dispatch_to_target_preserving_cursor_hint(
        &mut self,
        target: PointerTarget,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let saved_cursor_hint = ctx.cursor_hint;
        let action = self.dispatch_to_target(target, event, ctx);
        ctx.cursor_hint = saved_cursor_hint;
        action
    }

    fn update_hover_target(
        &mut self,
        next_target: Option<PointerTarget>,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let previous_target = self.hover_target;
        let previous_hover_action = if previous_target != next_target {
            previous_target.and_then(|target| {
                self.dispatch_to_target_preserving_cursor_hint(target, event, ctx)
            })
        } else {
            None
        };
        self.hover_target = next_target;

        if let Some(target) = next_target {
            return self.dispatch_to_target(target, event, ctx).or(previous_hover_action);
        }

        previous_hover_action
    }
}

impl Widget for ModalFrame {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));

        let settings = ctx.theme.settings_theme();
        self.close_button.set_style(Self::close_button_style(&self.style, settings));

        let header_height =
            Self::logical_to_px(self.style.header_height_logical, ctx.dpi).min(self.rect.h);
        let content_padding = Self::logical_to_px(self.style.content_padding_logical, ctx.dpi);
        let header_gap = Self::logical_to_px(self.style.header_gap_logical, ctx.dpi);
        let close_button_size =
            Self::logical_to_px(self.style.close_button_size_logical, ctx.dpi).min(header_height);

        self.header_rect = Rect::new(0.0, 0.0, self.rect.w, header_height);

        let close_x = (self.rect.w - content_padding - close_button_size).max(0.0);
        let close_y = (header_height - close_button_size) * 0.5;
        self.close_button_rect = Rect::new(close_x, close_y, close_button_size, close_button_size);
        self.close_button.set_rect(Rect::new(0.0, 0.0, close_button_size, close_button_size), ctx);

        let title_width = (close_x - content_padding - header_gap).max(0.0);
        self.title_rect = Rect::new(content_padding, 0.0, title_width, header_height);
        self.title.set_rect(Rect::new(0.0, 0.0, title_width, header_height), ctx);

        let content_y = header_height + content_padding;
        let content_width = (self.rect.w - content_padding * 2.0).max(0.0);
        let content_height = (self.rect.h - content_y - content_padding).max(0.0);
        self.content_rect = Rect::new(content_padding, content_y, content_width, content_height);
        self.content.set_rect(Rect::new(0.0, 0.0, content_width, content_height), ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let saved_offset = ctx.list.offset;
        let settings = ctx.theme.settings_theme();
        let corner_radius = Self::logical_to_px(self.style.corner_radius_logical, ctx.dpi);
        let border_width = Self::logical_to_px(self.style.border_width_logical, ctx.dpi);
        let header_corner_radius =
            corner_radius.min(self.header_rect.h).min(self.header_rect.w * 0.5);

        ctx.list.offset = saved_offset;
        ctx.list.fill_rounded(self.rect, settings.modal_surface, corner_radius);
        ctx.list.fill_rounded(self.header_rect, settings.sidebar_surface, header_corner_radius);
        if self.header_rect.h > header_corner_radius {
            ctx.list.fill(
                Rect::new(
                    self.header_rect.x,
                    self.header_rect.y + header_corner_radius,
                    self.header_rect.w,
                    self.header_rect.h - header_corner_radius,
                ),
                settings.sidebar_surface,
            );
        }
        if self.header_rect.h > 0.0 && self.header_rect.w > 0.0 {
            let separator_y = (self.header_rect.y + self.header_rect.h - border_width).max(0.0);
            ctx.list.fill(
                Rect::new(
                    self.header_rect.x,
                    separator_y,
                    self.header_rect.w,
                    border_width.max(1.0),
                ),
                settings.separator,
            );
        }
        ctx.list.stroke_rounded(
            self.rect,
            settings.section_border,
            corner_radius,
            border_width.max(1.0),
        );

        ctx.list.offset = (saved_offset.0 + self.title_rect.x, saved_offset.1 + self.title_rect.y);
        self.title.paint(ctx);

        ctx.list.offset =
            (saved_offset.0 + self.close_button_rect.x, saved_offset.1 + self.close_button_rect.y);
        self.close_button.paint(ctx);

        ctx.list.offset =
            (saved_offset.0 + self.content_rect.x, saved_offset.1 + self.content_rect.y);
        self.content.paint(ctx);
        ctx.list.offset = saved_offset;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn is_capturing(&self) -> bool {
        self.pointer_target.is_some() || self.content_is_capturing()
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        self.close_button.collect_focusable_ids(output);
        self.content.collect_focusable_ids(output);
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused_id = focused_id;
        self.close_button.set_keyboard_focus(focused_id);
        self.content.set_keyboard_focus(focused_id);
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if matches!(event, Event::KeyDown(KeyCode::Escape, _)) {
            return Some(WidgetAction::Overlay(crate::OverlayAction::DismissRequested));
        }

        if matches!(event, Event::KeyDown(..))
            && self.focused_id == Some(MODAL_FRAME_CLOSE_BUTTON_ID)
        {
            return self.dispatch_to_close_button(event, ctx);
        }

        match event {
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                let target = self.pointer_target_at(*px, *py)?;
                self.pointer_target = Some(target);
                self.hover_target = Some(target);
                self.dispatch_to_target(target, event, ctx)
            }
            Event::MouseMove { px, py } => {
                if let Some(target) = self.pointer_target {
                    return self.dispatch_to_target(target, event, ctx);
                }
                if self.content_is_capturing() {
                    return self.dispatch_to_content(event, ctx);
                }

                self.update_hover_target(self.pointer_target_at(*px, *py), event, ctx)
            }
            Event::MouseUp { px, py, .. } => {
                if self.pointer_target.is_none() && self.content_is_capturing() {
                    return self.dispatch_to_content(event, ctx);
                }
                let target = self.pointer_target.take()?;
                self.hover_target = self.pointer_target_at(*px, *py);
                self.dispatch_to_target(target, event, ctx)
            }
            Event::Wheel { px, py, .. } => {
                if self.content_is_capturing() {
                    return self.dispatch_to_content(event, ctx);
                }
                let target = self.pointer_target_at(*px, *py)?;
                self.dispatch_to_target(target, event, ctx)
            }
            Event::MouseDown { .. } => None,
            Event::KeyDown(..)
            | Event::ImePreedit { .. }
            | Event::ImeCommit(..)
            | Event::ImeEnable
            | Event::ImeDisable => self.dispatch_to_content(event, ctx),
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
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::{Event, EventCtx, KeyCode, LayoutCtx, Modifiers, MouseButton, PaintCtx};
    use winit::window::CursorIcon;

    #[derive(Default)]
    struct StubContent {
        rect: Rect,
        events: Vec<Event>,
        next_action: Option<WidgetAction>,
        paint_count: usize,
        capture_active: bool,
        consume_mouse_move: bool,
        cursor_on_mouse_move: Option<CursorIcon>,
    }

    impl StubContent {
        fn consuming_mouse_move(mut self) -> Self {
            self.consume_mouse_move = true;
            self
        }

        fn with_capture(mut self, capture_active: bool) -> Self {
            self.capture_active = capture_active;
            self
        }

        fn with_mouse_move_cursor(mut self, cursor: CursorIcon) -> Self {
            self.cursor_on_mouse_move = Some(cursor);
            self
        }
    }

    impl Widget for StubContent {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn paint(&self, _ctx: &mut PaintCtx) {}

        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }

        fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
            self.events.push(event.clone());
            if matches!(event, Event::MouseMove { .. }) {
                if let Some(cursor) = self.cursor_on_mouse_move {
                    ctx.cursor_hint = Some(cursor);
                }
                if self.consume_mouse_move {
                    return Some(WidgetAction::Consumed);
                }
            }
            self.next_action.clone()
        }

        fn is_capturing(&self) -> bool {
            self.capture_active
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn fixture_modal_with_content(content: Box<dyn Widget>) -> ModalFrame {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut modal = ModalFrame::new("Settings", content);
        modal.set_rect(Rect::new(0.0, 0.0, 320.0, 180.0), &mut layout_ctx);
        modal
    }

    fn fixture_modal() -> ModalFrame {
        fixture_modal_with_content(Box::new(StubContent::default()))
    }

    #[test]
    fn ime_content_routing_reuses_the_original_text_allocation() {
        let event = Event::ImeCommit("modal-sensitive-ime-route".to_owned());
        let original_allocation = match &event {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("test event is an IME commit"),
        };

        let local_event = ModalFrame::local_event(&event, Rect::new(8.0, 12.0, 100.0, 60.0));
        let local_allocation = match local_event.as_ref() {
            Event::ImeCommit(text) => text.as_ptr(),
            _ => unreachable!("local event must remain an IME commit"),
        };

        assert_eq!(local_allocation, original_allocation);
    }

    fn paint_for_test(modal: &ModalFrame) -> DrawList {
        let theme = crate::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        modal.paint(&mut paint_ctx);
        draw_list
    }

    fn click_close(modal: &mut ModalFrame) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        modal.on_event(
            &Event::MouseDown { px: 294.0, py: 14.0, button: MouseButton::Left },
            &mut ctx,
        );
        modal.on_event(&Event::MouseUp { px: 294.0, py: 14.0, button: MouseButton::Left }, &mut ctx)
    }

    fn key_escape(modal: &mut ModalFrame) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        modal.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut ctx)
    }

    fn stub_content(modal: &mut ModalFrame) -> &mut StubContent {
        modal
            .content_as_any_mut()
            .downcast_mut::<StubContent>()
            .expect("modal test content should be downcastable")
    }

    fn is_modal_surface(cmd: &DrawCmd) -> bool {
        matches!(cmd, DrawCmd::FillRect { rect, .. } if *rect == Rect::new(0.0, 0.0, 320.0, 180.0))
    }

    #[test]
    fn modal_frame_paints_surface_and_requests_close() {
        let mut modal = fixture_modal();

        assert!(paint_for_test(&modal).cmds.iter().any(is_modal_surface));
        assert_eq!(
            click_close(&mut modal),
            Some(WidgetAction::Overlay(crate::OverlayAction::DismissRequested))
        );
        assert_eq!(
            key_escape(&mut modal),
            Some(WidgetAction::Overlay(crate::OverlayAction::DismissRequested))
        );
    }

    #[test]
    fn modal_close_button_is_focusable_and_activates_from_keyboard() {
        let mut modal = fixture_modal();
        let mut focusable_ids = Vec::new();
        modal.collect_focusable_ids(&mut focusable_ids);
        assert_eq!(focusable_ids.first(), Some(&MODAL_FRAME_CLOSE_BUTTON_ID));

        modal.set_keyboard_focus(Some(MODAL_FRAME_CLOSE_BUTTON_ID));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        for key in [KeyCode::Enter, KeyCode::Char(' ')] {
            assert_eq!(
                modal.on_event(&Event::KeyDown(key, Modifiers::NONE), &mut ctx),
                Some(WidgetAction::Overlay(crate::OverlayAction::DismissRequested))
            );
        }
    }

    #[test]
    fn modal_frame_passes_child_actions_through_without_rewriting() {
        let mut modal = fixture_modal();
        let child = modal
            .content_as_any_mut()
            .downcast_mut::<StubContent>()
            .expect("modal test content should be downcastable");
        child.next_action = Some(WidgetAction::Consumed);

        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let action = modal.on_event(
            &Event::MouseDown { px: 20.0, py: 60.0, button: MouseButton::Left },
            &mut ctx,
        );

        assert_eq!(action, Some(WidgetAction::Consumed));
    }

    #[test]
    fn content_as_any_mut_exposes_inner_widget() {
        let mut modal = fixture_modal();
        let child = modal
            .content_as_any_mut()
            .downcast_mut::<StubContent>()
            .expect("modal test content should be downcastable");

        child.paint_count = 42;

        let updated = modal
            .content_as_any_mut()
            .downcast_mut::<StubContent>()
            .expect("modal test content should remain downcastable");
        assert_eq!(updated.paint_count, 42);
    }

    #[test]
    fn modal_frame_reports_capture_and_releases_pressed_content_after_mouse_up_outside_frame() {
        let mut modal = fixture_modal();
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let down_px = modal.content_rect.x + 18.0;
        let down_py = modal.content_rect.y + 10.0;
        let outside_px = modal.rect.w + 24.0;
        let outside_py = modal.rect.h + 16.0;

        assert!(!modal.is_capturing());
        assert_eq!(
            modal.on_event(
                &Event::MouseDown { px: down_px, py: down_py, button: MouseButton::Left },
                &mut ctx,
            ),
            None
        );
        assert!(modal.is_capturing());

        assert_eq!(
            modal.on_event(&Event::MouseMove { px: outside_px, py: outside_py }, &mut ctx),
            None
        );
        assert!(modal.is_capturing());

        assert_eq!(
            modal.on_event(
                &Event::MouseUp { px: outside_px, py: outside_py, button: MouseButton::Left },
                &mut ctx,
            ),
            None
        );
        assert!(!modal.is_capturing());

        let expected_events = vec![
            Event::MouseDown {
                px: down_px - modal.content_rect.x,
                py: down_py - modal.content_rect.y,
                button: MouseButton::Left,
            },
            Event::MouseMove {
                px: outside_px - modal.content_rect.x,
                py: outside_py - modal.content_rect.y,
            },
            Event::MouseUp {
                px: outside_px - modal.content_rect.x,
                py: outside_py - modal.content_rect.y,
                button: MouseButton::Left,
            },
        ];
        let child = stub_content(&mut modal);
        assert_eq!(child.events, expected_events);
    }

    #[test]
    fn modal_frame_reflects_content_capture_and_routes_mouse_move_directly_to_content() {
        let mut modal = fixture_modal_with_content(Box::new(
            StubContent::default()
                .with_capture(true)
                .consuming_mouse_move()
                .with_mouse_move_cursor(CursorIcon::Crosshair),
        ));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let move_px = modal.close_button_rect.x + modal.close_button_rect.w * 0.5;
        let move_py = modal.close_button_rect.y + modal.close_button_rect.h * 0.5;

        assert!(modal.is_capturing());
        assert_eq!(
            modal.on_event(&Event::MouseMove { px: move_px, py: move_py }, &mut ctx),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(ctx.cursor_hint, Some(CursorIcon::Crosshair));

        let expected_events = vec![Event::MouseMove {
            px: move_px - modal.content_rect.x,
            py: move_py - modal.content_rect.y,
        }];
        let child = stub_content(&mut modal);
        assert_eq!(child.events, expected_events);
    }

    #[test]
    fn modal_frame_sends_outside_move_before_switching_hover_to_close_button() {
        let mut modal = fixture_modal_with_content(Box::new(
            StubContent::default()
                .consuming_mouse_move()
                .with_mouse_move_cursor(CursorIcon::Crosshair),
        ));
        let theme = crate::theme::test_theme();
        let mut first_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let content_px = modal.content_rect.x + 12.0;
        let content_py = modal.content_rect.y + 8.0;
        let close_px = modal.close_button_rect.x + modal.close_button_rect.w * 0.5;
        let close_py = modal.close_button_rect.y + modal.close_button_rect.h * 0.5;

        assert_eq!(
            modal.on_event(&Event::MouseMove { px: content_px, py: content_py }, &mut first_ctx),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(first_ctx.cursor_hint, Some(CursorIcon::Crosshair));

        let mut second_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        assert_eq!(
            modal.on_event(&Event::MouseMove { px: close_px, py: close_py }, &mut second_ctx),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(second_ctx.cursor_hint, Some(CursorIcon::Pointer));

        let expected_events = vec![
            Event::MouseMove {
                px: content_px - modal.content_rect.x,
                py: content_py - modal.content_rect.y,
            },
            Event::MouseMove {
                px: close_px - modal.content_rect.x,
                py: close_py - modal.content_rect.y,
            },
        ];
        let child = stub_content(&mut modal);
        assert_eq!(child.events, expected_events);
    }

    #[test]
    fn modal_frame_clears_hover_when_pointer_moves_into_header_gap() {
        let mut modal = fixture_modal_with_content(Box::new(
            StubContent::default()
                .consuming_mouse_move()
                .with_mouse_move_cursor(CursorIcon::Crosshair),
        ));
        let theme = crate::theme::test_theme();
        let content_px = modal.content_rect.x + 10.0;
        let content_py = modal.content_rect.y + 10.0;
        let header_gap_px = modal.title_rect.x + 8.0;
        let header_gap_py = modal.header_rect.y + modal.header_rect.h * 0.5;

        let mut first_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        assert_eq!(
            modal.on_event(&Event::MouseMove { px: content_px, py: content_py }, &mut first_ctx),
            Some(WidgetAction::Consumed)
        );

        let mut second_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        assert_eq!(
            modal.on_event(
                &Event::MouseMove { px: header_gap_px, py: header_gap_py },
                &mut second_ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(second_ctx.cursor_hint, None);

        let mut third_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        assert_eq!(
            modal.on_event(
                &Event::MouseMove { px: header_gap_px, py: header_gap_py },
                &mut third_ctx,
            ),
            None
        );

        let expected_events = vec![
            Event::MouseMove {
                px: content_px - modal.content_rect.x,
                py: content_py - modal.content_rect.y,
            },
            Event::MouseMove {
                px: header_gap_px - modal.content_rect.x,
                py: header_gap_py - modal.content_rect.y,
            },
        ];
        let child = stub_content(&mut modal);
        assert_eq!(child.events, expected_events);
    }

    #[test]
    fn modal_frame_paints_header_with_rounded_top_fill_and_square_bottom_cover() {
        let modal = fixture_modal();
        let draw_list = paint_for_test(&modal);
        let corner_radius = ModalFrame::logical_to_px(modal.style.corner_radius_logical, 1.0);
        let square_cover_rect = Rect::new(
            modal.header_rect.x,
            modal.header_rect.y + corner_radius,
            modal.header_rect.w,
            modal.header_rect.h - corner_radius,
        );

        assert!(draw_list.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, radius, .. }
                    if *rect == modal.header_rect && *radius == corner_radius
            )
        }));
        assert!(draw_list.cmds.iter().any(|cmd| {
            matches!(
                cmd,
                DrawCmd::FillRect { rect, radius, .. }
                    if *rect == square_cover_rect && *radius == 0.0
            )
        }));
    }
}
