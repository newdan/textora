//! SearchBarWidget — 搜索面板的绘制 + 键盘事件转译。
//! 显隐由 app 通过 set_visible 注入（visibility 信息源是 doc.search_state.panel_visible）。
//! query 与 match_count 由 app 通过 set_input 注入。
mod layout;

/// Height of the search bar in logical pixels (before DPI scaling).
pub use crate::constants::BAR_HEIGHT as SEARCH_BAR_HEIGHT;

use self::layout::{SearchBarLayout, SearchBarLayoutInput, calculate_search_bar_layout};
use crate::core::widget::MouseButton;
use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction, WidgetId,
};
use crate::widgets::button::{ButtonStyle, ButtonVisualState};
use crate::widgets::icon::draw_icon;
use crate::widgets::tooltip::TooltipHint;
use std::any::Any;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum HoveredButton {
    None,
    CloseBar,
    Prev,
    Next,
    ToggleReplace,
    Regex,
    Replace,
    ReplaceAll,
}

/// app 端注入的纯数据（widget 内部不知道 doc / search_state 概念）。
#[derive(Clone, Default)]
pub struct SearchBarSnapshot {
    pub query: String,
    pub preedit_text: String,
    pub match_count: usize,
    pub current_match: usize,
    pub visible: bool,
    pub blink_on: bool,
    pub replace_query: String,
    pub replace_mode: bool,
    pub focus_replace: bool,
    pub options_use_regex: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchBarAction {
    Next,
    Prev,
    Close,
    DismissOrClear,
    ToggleReplace,
    ToggleRegex,
    Replace,
    ReplaceAll,
    FocusFind,
    FocusReplace,
    QueryChanged(String),
    ReplaceQueryChanged(String),
    HoverChanged,
}

use crate::widgets::text_box::{TextBox, TextBoxIme};

const FIND_BOX_ID: WidgetId = WidgetId(2);
const REPLACE_BOX_ID: WidgetId = WidgetId(3);
const SEARCH_FONT_SIZE_LOGICAL: f32 = 14.0;
const BUTTON_ASCII_GLYPH_WIDTH_LOGICAL: f32 = 8.0;
const BUTTON_WIDE_GLYPH_WIDTH_LOGICAL: f32 = 14.0;

pub struct SearchBarWidget {
    rect: Rect,
    layout: SearchBarLayout,
    snap: SearchBarSnapshot,
    find_box: TextBox,
    replace_box: TextBox,
    hovered_btn: HoveredButton,
}

impl Default for SearchBarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchBarWidget {
    pub fn new() -> Self {
        let mut find_box = TextBox::with_id(FIND_BOX_ID);
        find_box.set_placeholder("查找…");
        find_box.set_max_len_bytes(2048);

        let mut replace_box = TextBox::with_id(REPLACE_BOX_ID);
        replace_box.set_placeholder("替换为…");
        replace_box.set_max_len_bytes(2048);

        Self {
            rect: Rect::ZERO,
            layout: SearchBarLayout::default(),
            snap: SearchBarSnapshot::default(),
            find_box,
            replace_box,
            hovered_btn: HoveredButton::None,
        }
    }

    pub fn with_snap(rect: Rect, snap: SearchBarSnapshot) -> Self {
        let mut w = Self::new();
        w.rect = rect;
        w.set_input(snap);
        w
    }

    /// Get the close button rect (for testing / tooltip integration).
    pub fn close_btn_rect(&self) -> Rect {
        self.layout.close_btn_rect
    }

    pub fn set_input(&mut self, snap: SearchBarSnapshot) {
        if !snap.visible {
            self.hovered_btn = HoveredButton::None;
            self.find_box.cancel_transient_interaction();
            self.replace_box.cancel_transient_interaction();
        }
        self.find_box.sync_text(&snap.query);
        self.replace_box.sync_text(&snap.replace_query);
        self.find_box.set_blink(snap.blink_on);
        self.replace_box.set_blink(snap.blink_on);
        self.find_box.set_focus(snap.visible && (!snap.replace_mode || !snap.focus_replace));
        self.replace_box.set_focus(snap.visible && snap.replace_mode && snap.focus_replace);
        self.snap = snap;
    }

    pub fn is_visible(&self) -> bool {
        self.snap.visible
    }
}

impl Widget for SearchBarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        self.layout = calculate_search_bar_layout(SearchBarLayoutInput {
            width: self.rect.w,
            height: self.rect.h,
            dpi: ctx.dpi,
            replace_mode: self.snap.replace_mode,
            has_query: !self.snap.query.is_empty(),
            match_count: self.snap.match_count,
            current_match: self.snap.current_match,
        });
        self.find_box.layout(self.layout.find_input_rect, ctx);
        if self.snap.replace_mode {
            self.replace_box.layout(self.layout.replace_input_rect, ctx);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 || !self.snap.visible {
            return;
        }
        let dpi = ctx.dpi;
        let font_size = SEARCH_FONT_SIZE_LOGICAL * dpi;
        let baseline = self.rect.h * 0.5 + font_size * 0.35;

        if self.snap.replace_mode {
            self.paint_find_replace(ctx, dpi, baseline);
        } else {
            self.paint_find_only(ctx, dpi, baseline);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.layout.pill_rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.snap.visible {
            return None;
        }

        match ev {
            Event::PointerLeave => {
                let hover_changed = self.hovered_btn != HoveredButton::None;
                self.hovered_btn = HoveredButton::None;
                let _ = self.find_box.on_event(ev, _ctx);
                let _ = self.replace_box.on_event(ev, _ctx);
                hover_changed.then_some(WidgetAction::SearchBar(SearchBarAction::HoverChanged))
            }
            Event::InteractionCancel => {
                let hover_changed = self.hovered_btn != HoveredButton::None;
                self.hovered_btn = HoveredButton::None;
                let find_changed = self.find_box.on_event(ev, _ctx).is_some();
                let replace_changed = self.replace_box.on_event(ev, _ctx).is_some();
                (hover_changed || find_changed || replace_changed).then_some(WidgetAction::Consumed)
            }
            Event::KeyDown(kc, _modifiers) => {
                // Handle tab locally
                if *kc == KeyCode::Tab {
                    if self.snap.replace_mode {
                        if self.snap.focus_replace {
                            return Some(WidgetAction::SearchBar(SearchBarAction::FocusFind));
                        } else {
                            return Some(WidgetAction::SearchBar(SearchBarAction::FocusReplace));
                        }
                    } else {
                        return Some(WidgetAction::SearchBar(SearchBarAction::ToggleReplace));
                    }
                }

                if *kc == KeyCode::Escape {
                    if self.snap.replace_mode && self.snap.focus_replace {
                        let action = self.replace_box.on_event(ev, _ctx);
                        return self.map_text_box_widget_action(action);
                    }
                    return Some(WidgetAction::SearchBar(SearchBarAction::DismissOrClear));
                }

                let action = if self.snap.replace_mode && self.snap.focus_replace {
                    self.replace_box.on_event(ev, _ctx)
                } else {
                    self.find_box.on_event(ev, _ctx)
                };
                self.map_text_box_widget_action(action)
            }
            Event::ImePreedit { text, cursor } => {
                let action = if self.snap.replace_mode && self.snap.focus_replace {
                    self.replace_box
                        .on_event(&Event::ImePreedit { text: text.clone(), cursor: *cursor }, _ctx)
                } else {
                    self.find_box
                        .on_event(&Event::ImePreedit { text: text.clone(), cursor: *cursor }, _ctx)
                };
                self.map_text_box_widget_action(action)
            }
            Event::ImeCommit(text) => {
                let action = if self.snap.replace_mode && self.snap.focus_replace {
                    self.replace_box.on_event(&Event::ImeCommit(text.clone()), _ctx)
                } else {
                    self.find_box.on_event(&Event::ImeCommit(text.clone()), _ctx)
                };
                self.map_text_box_widget_action(action)
            }
            Event::ImeEnable => {
                self.find_box.on_ime(&TextBoxIme::Enabled);
                self.replace_box.on_ime(&TextBoxIme::Enabled);
                Some(WidgetAction::Consumed)
            }
            Event::ImeDisable => {
                self.find_box.on_ime(&TextBoxIme::Disabled);
                self.replace_box.on_ime(&TextBoxIme::Disabled);
                Some(WidgetAction::Consumed)
            }
            Event::MouseMove { px, py } => {
                let text_box_action = if self.find_box.is_capturing() {
                    self.find_box.on_event(ev, _ctx)
                } else if self.replace_box.is_capturing() {
                    self.replace_box.on_event(ev, _ctx)
                } else {
                    None
                };
                if text_box_action.is_some() {
                    return self.map_text_box_widget_action(text_box_action);
                }

                let old = self.hovered_btn;
                self.hovered_btn = HoveredButton::None;
                self.update_hover(*px, *py);
                if self.hovered_btn != HoveredButton::None {
                    _ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                if old != self.hovered_btn {
                    Some(WidgetAction::SearchBar(SearchBarAction::HoverChanged))
                } else {
                    None
                }
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if let Some(action) = self.handle_mouse_down(*px, *py) {
                    return Some(action);
                }
                let text_box_action = if self.snap.replace_mode && self.replace_box.hit(*px, *py) {
                    self.replace_box.on_event(ev, _ctx)
                } else if self.find_box.hit(*px, *py) {
                    self.find_box.on_event(ev, _ctx)
                } else {
                    None
                };
                self.map_text_box_widget_action(text_box_action)
            }
            Event::MouseUp { .. } => {
                let was_dragging = self.find_box.is_capturing() || self.replace_box.is_capturing();
                self.find_box.on_mouse_up();
                self.replace_box.on_mouse_up();
                was_dragging.then_some(WidgetAction::Consumed)
            }
            _ => None,
        }
    }

    fn id(&self) -> Option<WidgetId> {
        Some(crate::core::widget::ids::SEARCH_BAR)
    }

    fn is_capturing(&self) -> bool {
        self.find_box.is_capturing() || self.replace_box.is_capturing()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        let hit = |r: &Rect| r.w > 0.0 && r.contains(px, py);

        let r = self.layout.close_btn_rect;
        if hit(&r) {
            return Some(TooltipHint { label: "关闭查找".into(), target_rect: r });
        }

        let r = self.layout.toggle_replace_btn_rect;
        if hit(&r) {
            let label = if self.snap.replace_mode { "隐藏替换" } else { "显示替换" };
            return Some(TooltipHint { label: label.into(), target_rect: r });
        }

        let r = self.layout.regex_btn_rect;
        if hit(&r) {
            return Some(TooltipHint { label: "正则表达式".into(), target_rect: r });
        }

        let r = self.layout.prev_btn_rect;
        if hit(&r) {
            return Some(TooltipHint { label: "上一个匹配".into(), target_rect: r });
        }

        let r = self.layout.next_btn_rect;
        if hit(&r) {
            return Some(TooltipHint { label: "下一个匹配".into(), target_rect: r });
        }

        let r = self.layout.replace_btn_rect;
        if hit(&r) {
            return Some(TooltipHint { label: "替换".into(), target_rect: r });
        }

        let r = self.layout.replace_all_btn_rect;
        if hit(&r) {
            return Some(TooltipHint { label: "全部替换".into(), target_rect: r });
        }

        None
    }
}

impl SearchBarWidget {
    fn map_text_box_widget_action(&self, action: Option<WidgetAction>) -> Option<WidgetAction> {
        match action {
            Some(WidgetAction::Control(control_action)) => self.map_control_action(control_action),
            other => other,
        }
    }

    fn map_control_action(
        &self,
        control_action: crate::core::widget::ControlAction,
    ) -> Option<WidgetAction> {
        match control_action {
            crate::core::widget::ControlAction::TextEdited {
                id: FIND_BOX_ID,
                value: crate::core::widget::TextPayload::Plain(text),
            } => Some(WidgetAction::SearchBar(SearchBarAction::QueryChanged(text))),
            crate::core::widget::ControlAction::TextEdited {
                id: REPLACE_BOX_ID,
                value: crate::core::widget::TextPayload::Plain(text),
            } => Some(WidgetAction::SearchBar(SearchBarAction::ReplaceQueryChanged(text))),
            crate::core::widget::ControlAction::TextCommitted { id: FIND_BOX_ID, .. } => {
                Some(WidgetAction::SearchBar(SearchBarAction::Next))
            }
            crate::core::widget::ControlAction::TextCommitted { id: REPLACE_BOX_ID, .. } => {
                Some(WidgetAction::SearchBar(SearchBarAction::Replace))
            }
            crate::core::widget::ControlAction::FocusRequested { id: FIND_BOX_ID } => {
                Some(WidgetAction::SearchBar(SearchBarAction::FocusFind))
            }
            crate::core::widget::ControlAction::FocusRequested { id: REPLACE_BOX_ID } => {
                Some(WidgetAction::SearchBar(SearchBarAction::FocusReplace))
            }
            _ => Some(WidgetAction::Consumed),
        }
    }

    fn update_hover(&mut self, px: f32, py: f32) {
        let check = |r: &Rect| r.w > 0.0 && r.contains(px, py);
        if check(&self.layout.close_btn_rect) {
            self.hovered_btn = HoveredButton::CloseBar;
            return;
        }
        if check(&self.layout.toggle_replace_btn_rect) {
            self.hovered_btn = HoveredButton::ToggleReplace;
            return;
        }
        if check(&self.layout.regex_btn_rect) {
            self.hovered_btn = HoveredButton::Regex;
            return;
        }
        if check(&self.layout.prev_btn_rect) {
            self.hovered_btn = HoveredButton::Prev;
            return;
        }
        if check(&self.layout.next_btn_rect) {
            self.hovered_btn = HoveredButton::Next;
            return;
        }
        if check(&self.layout.replace_btn_rect) {
            self.hovered_btn = HoveredButton::Replace;
            return;
        }
        if check(&self.layout.replace_all_btn_rect) {
            self.hovered_btn = HoveredButton::ReplaceAll;
        }
    }

    fn handle_mouse_down(&mut self, px: f32, py: f32) -> Option<WidgetAction> {
        let check = |r: &Rect| r.w > 0.0 && r.contains(px, py);
        if check(&self.layout.close_btn_rect) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Close));
        }
        if check(&self.layout.toggle_replace_btn_rect) {
            return Some(WidgetAction::SearchBar(SearchBarAction::ToggleReplace));
        }
        if check(&self.layout.regex_btn_rect) {
            return Some(WidgetAction::SearchBar(SearchBarAction::ToggleRegex));
        }
        if check(&self.layout.prev_btn_rect) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Prev));
        }
        if check(&self.layout.next_btn_rect) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Next));
        }
        if self.snap.match_count > 0 {
            if check(&self.layout.replace_btn_rect) {
                return Some(WidgetAction::SearchBar(SearchBarAction::Replace));
            }
            if check(&self.layout.replace_all_btn_rect) {
                return Some(WidgetAction::SearchBar(SearchBarAction::ReplaceAll));
            }
        }
        None
    }

    /// Paint the find-only bar.
    fn paint_find_only(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        self.paint_shell(ctx, dpi);
        self.paint_search_icon(ctx);
        self.paint_text_box_clipped(&self.find_box, self.layout.find_input_rect, ctx);
        self.paint_controls(ctx, dpi, baseline);
    }

    /// Paint the find+replace bar (inline single-row layout).
    fn paint_find_replace(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        self.paint_shell(ctx, dpi);
        self.paint_search_icon(ctx);
        self.paint_separator(ctx, dpi, baseline);
        self.paint_text_box_clipped(&self.find_box, self.layout.find_input_rect, ctx);
        self.paint_text_box_clipped(&self.replace_box, self.layout.replace_input_rect, ctx);
        self.paint_controls(ctx, dpi, baseline);
    }

    fn paint_shell(&self, ctx: &mut PaintCtx, dpi: f32) {
        ctx.list.fill(self.layout.pill_rect, ctx.theme.palette.input_bg);
        ctx.list.stroke(self.layout.pill_rect, ctx.theme.palette.input_border, dpi);
    }

    fn paint_search_icon(&self, ctx: &mut PaintCtx) {
        let icon_rect = self.layout.search_icon_rect;
        if icon_rect.w <= 0.0 {
            return;
        }
        let mut icon_color = ctx.theme.palette.input_fg;
        icon_color[3] *= 0.6;
        draw_icon(ctx.list, "search", icon_rect.x, icon_rect.y, icon_rect.w, icon_color);
    }

    fn paint_separator(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        let separator_rect = self.layout.separator_rect;
        if separator_rect.w <= 0.0 {
            return;
        }
        let mut color = ctx.theme.palette.input_fg;
        color[3] *= 0.6;
        if let Some(ref mut shaper) = ctx.shaper {
            ctx.list.text_shaped(
                separator_rect.x + (separator_rect.w - BUTTON_WIDE_GLYPH_WIDTH_LOGICAL * dpi) * 0.5,
                baseline,
                SEARCH_FONT_SIZE_LOGICAL * dpi,
                color,
                "\u{2192}",
                shaper,
            );
        }
    }

    fn paint_text_box_clipped(&self, text_box: &TextBox, clip_rect: Rect, ctx: &mut PaintCtx) {
        if clip_rect.w <= 0.0 || clip_rect.h <= 0.0 {
            return;
        }
        let (offset_x, offset_y) = ctx.list.offset;
        let absolute_clip =
            Rect::new(clip_rect.x + offset_x, clip_rect.y + offset_y, clip_rect.w, clip_rect.h);
        ctx.list.cmds.push(crate::core::paint::DrawCmd::PushClip(absolute_clip));
        text_box.paint(ctx);
        ctx.list.cmds.push(crate::core::paint::DrawCmd::PopClip);
    }

    fn paint_controls(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        self.paint_text_button(
            ctx,
            self.layout.close_btn_rect,
            "\u{2715}",
            HoveredButton::CloseBar,
            dpi,
            baseline,
        );
        let toggle_label = if self.snap.replace_mode { "\u{25b2}" } else { "\u{25bc}" };
        self.paint_text_button(
            ctx,
            self.layout.toggle_replace_btn_rect,
            toggle_label,
            HoveredButton::ToggleReplace,
            dpi,
            baseline,
        );
        self.paint_regex_button(ctx, dpi);
        self.paint_navigation(ctx, dpi, baseline);
        self.paint_auxiliary_text(ctx, dpi, baseline);
        self.paint_replace_actions(ctx, dpi, baseline);
    }

    fn paint_regex_button(&self, ctx: &mut PaintCtx, dpi: f32) {
        let rect = self.layout.regex_btn_rect;
        if rect.w <= 0.0 {
            return;
        }
        self.paint_hover_background(ctx, rect, HoveredButton::Regex, dpi);
        let color = if self.snap.options_use_regex {
            ctx.theme.palette.accent
        } else {
            self.button_color(ctx, HoveredButton::Regex)
        };
        let icon_size = (14.0 * dpi).min(rect.w);
        draw_icon(
            ctx.list,
            "regex",
            rect.x + (rect.w - icon_size) * 0.5,
            rect.y + (rect.h - icon_size) * 0.5,
            icon_size,
            color,
        );
    }

    fn paint_navigation(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        self.paint_text_button(
            ctx,
            self.layout.prev_btn_rect,
            "\u{25c0}",
            HoveredButton::Prev,
            dpi,
            baseline,
        );
        self.paint_text_button(
            ctx,
            self.layout.next_btn_rect,
            "\u{25b6}",
            HoveredButton::Next,
            dpi,
            baseline,
        );
    }

    fn paint_auxiliary_text(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        let rect = self.layout.auxiliary_text_rect;
        if rect.w <= 0.0 || self.snap.query.is_empty() {
            return;
        }
        let (label, color) = if self.snap.match_count > 0 {
            let current = self.snap.current_match.saturating_add(1).min(self.snap.match_count);
            (format!("{current}/{}", self.snap.match_count), ctx.theme.palette.input_fg)
        } else {
            ("无匹配结果".to_owned(), ctx.theme.palette.danger)
        };
        if let Some(ref mut shaper) = ctx.shaper {
            ctx.list.text_shaped(
                rect.x,
                baseline,
                SEARCH_FONT_SIZE_LOGICAL * dpi,
                color,
                &label,
                shaper,
            );
        }
    }

    fn paint_replace_actions(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        if !self.snap.replace_mode {
            return;
        }
        let (replace_label, replace_all_label) = if self.layout.full_replace_labels {
            ("\u{66ff}\u{6362}", "\u{5168}\u{90e8}")
        } else {
            ("\u{6362}", "\u{5168}")
        };
        self.paint_text_button(
            ctx,
            self.layout.replace_btn_rect,
            replace_label,
            HoveredButton::Replace,
            dpi,
            baseline,
        );
        self.paint_text_button(
            ctx,
            self.layout.replace_all_btn_rect,
            replace_all_label,
            HoveredButton::ReplaceAll,
            dpi,
            baseline,
        );
    }

    fn paint_text_button(
        &self,
        ctx: &mut PaintCtx,
        rect: Rect,
        label: &str,
        hovered_button: HoveredButton,
        dpi: f32,
        baseline: f32,
    ) {
        if rect.w <= 0.0 {
            return;
        }
        self.paint_hover_background(ctx, rect, hovered_button, dpi);
        let text_width = label
            .chars()
            .map(|character| {
                if character.is_ascii() {
                    BUTTON_ASCII_GLYPH_WIDTH_LOGICAL
                } else {
                    BUTTON_WIDE_GLYPH_WIDTH_LOGICAL
                }
            })
            .sum::<f32>()
            * dpi;
        let color = self.button_color(ctx, hovered_button);
        if let Some(ref mut shaper) = ctx.shaper {
            ctx.list.text_shaped(
                rect.x + (rect.w - text_width) * 0.5,
                baseline,
                SEARCH_FONT_SIZE_LOGICAL * dpi,
                color,
                label,
                shaper,
            );
        }
    }

    fn paint_hover_background(
        &self,
        ctx: &mut PaintCtx,
        rect: Rect,
        hovered_button: HoveredButton,
        dpi: f32,
    ) {
        if self.hovered_btn == hovered_button {
            let hover_color = ButtonStyle::from_theme(ctx.theme)
                .background_color(ButtonVisualState::Hovered, ctx.global_alpha);
            ctx.list.fill_rounded(rect, hover_color, 4.0 * dpi);
        }
    }

    fn button_color(&self, ctx: &PaintCtx, hovered_button: HoveredButton) -> [f32; 4] {
        let mut color = ctx.theme.palette.input_fg;
        color[3] *= if self.hovered_btn == hovered_button { 0.9 } else { 0.6 };
        color
    }

    /// Returns the IME cursor rect of the currently focused TextBox.
    /// Used by app layer to position the OS IME candidate window.
    pub fn focused_textbox_ime_cursor_rect(&self) -> Option<Rect> {
        if !self.snap.visible {
            return None;
        }
        if self.snap.replace_mode
            && self.snap.focus_replace
            && (self.replace_box.has_preedit() || self.replace_box.is_focused())
        {
            return Some(self.replace_box.ime_cursor_rect());
        }
        if self.find_box.has_preedit() || self.find_box.is_focused() {
            return Some(self.find_box.ime_cursor_rect());
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Clipboard;
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::DrawList;
    use crate::theme::test_theme;

    #[derive(Default)]
    struct TestClipboard {
        text: Option<String>,
    }

    impl Clipboard for TestClipboard {
        fn read_text(&mut self) -> Option<String> {
            self.text.clone()
        }

        fn write_text(&mut self, text: &str) -> bool {
            self.text = Some(text.to_owned());
            true
        }
    }

    fn setup_search_bar(query: &str) -> SearchBarWidget {
        let mut w = SearchBarWidget::new();
        w.set_input(SearchBarSnapshot {
            query: query.into(),
            preedit_text: String::new(),
            match_count: 0,
            current_match: 0,
            visible: true,

            blink_on: false,
            replace_query: String::new(),
            replace_mode: false,
            focus_replace: false,
            options_use_regex: false,
        });
        // Do a layout + paint to initialize button rects
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut ctx = LayoutCtx { ui_measure: None, measure: &mut m, theme: &theme, dpi: 1.0 };
        w.set_rect(crate::core::geom::Rect::new(0.0, 0.0, 800.0, 28.0), &mut ctx);
        let mut dl = DrawList::new();
        let mut pctx = PaintCtx {
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: None,
        };
        w.paint(&mut pctx);
        w
    }

    fn layout_search_bar(width: f32, snapshot: SearchBarSnapshot) -> SearchBarWidget {
        let mut search_bar = SearchBarWidget::new();
        search_bar.set_input(snapshot);
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        search_bar.set_rect(Rect::new(0.0, 0.0, width, 28.0), &mut context);
        search_bar
    }

    fn paint_search_bar(search_bar: &SearchBarWidget) -> DrawList {
        let theme = test_theme();
        let mut draw_list = DrawList::new();
        let mut context = PaintCtx::new(&mut draw_list, &theme, 1.0);
        search_bar.paint(&mut context);
        draw_list
    }

    #[test]
    fn find_only_layout_does_not_reserve_replace_action_width() {
        let search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot { visible: true, ..SearchBarSnapshot::default() },
        );
        let _ = paint_search_bar(&search_bar);

        assert!(
            search_bar.find_box.rect().right() + 12.0 >= search_bar.layout.regex_btn_rect.x,
            "find-only input should extend close to the first trailing button"
        );
    }

    #[test]
    fn button_hit_regions_exist_immediately_after_layout() {
        let mut search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot { visible: true, ..SearchBarSnapshot::default() },
        );
        let close_rect = search_bar.close_btn_rect();

        assert!(close_rect.w > 0.0, "layout should establish the close button hit region");
        assert_eq!(
            search_bar.handle_mouse_down(
                close_rect.x + close_rect.w * 0.5,
                close_rect.y + close_rect.h * 0.5,
            ),
            Some(WidgetAction::SearchBar(SearchBarAction::Close))
        );
    }

    #[test]
    fn narrow_find_layout_keeps_input_before_visible_buttons() {
        let search_bar = layout_search_bar(
            160.0,
            SearchBarSnapshot {
                query: "needle".into(),
                match_count: usize::MAX,
                current_match: usize::MAX - 1,
                visible: true,
                ..SearchBarSnapshot::default()
            },
        );
        let _ = paint_search_bar(&search_bar);
        let first_button_x = [
            search_bar.layout.close_btn_rect,
            search_bar.layout.toggle_replace_btn_rect,
            search_bar.layout.regex_btn_rect,
            search_bar.layout.prev_btn_rect,
            search_bar.layout.next_btn_rect,
        ]
        .into_iter()
        .filter(|rect| rect.w > 0.0)
        .map(|rect| rect.x)
        .fold(f32::INFINITY, f32::min);

        assert!(search_bar.find_box.rect().w > 0.0, "find input should remain usable");
        assert!(
            search_bar.find_box.rect().right() <= first_button_x,
            "find input must not overlap visible buttons"
        );
    }

    #[test]
    fn narrow_replace_layout_keeps_actions_clickable_and_inside_bounds() {
        let mut search_bar = layout_search_bar(
            160.0,
            SearchBarSnapshot {
                query: "needle".into(),
                replace_query: "replacement".into(),
                match_count: 1,
                visible: true,
                replace_mode: true,
                ..SearchBarSnapshot::default()
            },
        );

        assert!(search_bar.find_box.rect().w > 0.0);
        assert!(search_bar.replace_box.rect().w > 0.0);
        assert!(search_bar.layout.replace_btn_rect.w > 0.0);
        assert!(search_bar.layout.replace_all_btn_rect.w > 0.0);
        for rect in [
            search_bar.layout.close_btn_rect,
            search_bar.layout.toggle_replace_btn_rect,
            search_bar.layout.replace_btn_rect,
            search_bar.layout.replace_all_btn_rect,
        ] {
            assert!(rect.x >= 0.0 && rect.right() <= search_bar.layout.pill_rect.right());
        }

        let replace_rect = search_bar.layout.replace_btn_rect;
        assert_eq!(
            search_bar.handle_mouse_down(
                replace_rect.x + replace_rect.w * 0.5,
                replace_rect.y + replace_rect.h * 0.5,
            ),
            Some(WidgetAction::SearchBar(SearchBarAction::Replace))
        );
    }

    #[test]
    fn compact_replace_labels_stay_inside_their_buttons() {
        use crate::core::paint::DrawCmd;

        let search_bar = layout_search_bar(
            160.0,
            SearchBarSnapshot {
                query: "needle".into(),
                replace_query: "replacement".into(),
                match_count: 1,
                visible: true,
                replace_mode: true,
                ..SearchBarSnapshot::default()
            },
        );
        let theme = test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("测试环境必须能创建文字塑形器");
        let mut context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };
        search_bar.paint(&mut context);

        for (label, button_rect) in [
            ("换", search_bar.layout.replace_btn_rect),
            ("全", search_bar.layout.replace_all_btn_rect),
        ] {
            let (text_x, text_width) = draw_list
                .cmds
                .iter()
                .find_map(|command| match command {
                    DrawCmd::TextLayout { layout, x, .. } if layout.text == label => {
                        Some((*x, layout.shaped.width))
                    }
                    _ => None,
                })
                .expect("compact replace label should be painted");
            assert!(text_x >= button_rect.x);
            assert!(text_x + text_width <= button_rect.right());
        }
    }

    #[test]
    fn reasonable_replace_width_shows_all_controls() {
        let search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot {
                query: "needle".into(),
                replace_query: "replacement".into(),
                match_count: 120,
                current_match: 11,
                visible: true,
                replace_mode: true,
                ..SearchBarSnapshot::default()
            },
        );

        assert!(search_bar.layout.full_replace_labels);
        for rect in [
            search_bar.layout.close_btn_rect,
            search_bar.layout.toggle_replace_btn_rect,
            search_bar.layout.regex_btn_rect,
            search_bar.layout.prev_btn_rect,
            search_bar.layout.next_btn_rect,
            search_bar.layout.replace_btn_rect,
            search_bar.layout.replace_all_btn_rect,
            search_bar.layout.auxiliary_text_rect,
        ] {
            assert!(rect.w > 0.0, "all controls should fit at 480 logical pixels");
            assert!(rect.x >= 0.0 && rect.right() <= search_bar.layout.pill_rect.right());
        }
    }

    #[test]
    fn no_match_message_reserves_width_for_chinese_text() {
        use crate::core::paint::DrawCmd;

        let search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot {
                query: "needle".into(),
                visible: true,
                ..SearchBarSnapshot::default()
            },
        );

        assert!(
            search_bar.layout.auxiliary_text_rect.w >= 70.0,
            "无匹配结果 should reserve five UI glyphs at the 14px font size"
        );

        let theme = test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("测试环境必须能创建文字塑形器");
        let mut context = PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        };
        search_bar.paint(&mut context);
        let no_match_layout = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TextLayout { layout, .. } if layout.text == "无匹配结果" => {
                    Some(layout)
                }
                _ => None,
            })
            .expect("无匹配结果 should be emitted as visible text");
        assert!(no_match_layout.shaped.width <= search_bar.layout.auxiliary_text_rect.w);
    }

    #[test]
    fn search_shell_uses_input_fill_border_stroke_and_clips_query() {
        use crate::core::paint::DrawCmd;

        let search_bar = layout_search_bar(
            240.0,
            SearchBarSnapshot {
                query: "a very long query that must stay inside its input".into(),
                visible: true,
                ..SearchBarSnapshot::default()
            },
        );
        let theme = test_theme();
        let draw_list = paint_search_bar(&search_bar);

        assert!(matches!(
            draw_list.cmds.first(),
            Some(DrawCmd::FillRect { color, .. }) if *color == theme.palette.input_bg
        ));
        assert!(matches!(
            draw_list.cmds.get(1),
            Some(DrawCmd::StrokeRect { color, .. }) if *color == theme.palette.input_border
        ));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::PushClip(rect) if *rect == search_bar.find_box.rect()
        )));
    }

    #[test]
    fn basic_creation() {
        let w = SearchBarWidget::new();
        assert_eq!(w.find_box.text(), "");
    }

    #[test]
    fn tooltip_at_close_button() {
        let w = setup_search_bar("test");
        let r = w.layout.close_btn_rect;
        assert!(r.w > 0.0, "close button should be laid out");
        let hint = w.tooltip_at(r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert!(hint.is_some(), "hovering close button should return tooltip");
        assert_eq!(hint.unwrap().label, "关闭查找");
    }

    #[test]
    fn tooltip_at_empty_area_returns_none() {
        let w = setup_search_bar("test");
        let hint = w.tooltip_at(400.0, 14.0); // center of search bar, not on any button
        assert!(hint.is_none(), "hovering empty area should return None");
    }

    #[test]
    fn tooltip_at_toggle_replace_button() {
        let w = setup_search_bar("test");
        let r = w.layout.toggle_replace_btn_rect;
        if r.w > 0.0 {
            let hint = w.tooltip_at(r.x + r.w / 2.0, r.y + r.h / 2.0);
            assert!(hint.is_some(), "hovering toggle replace should return tooltip");
            assert_eq!(hint.unwrap().label, "显示替换");
        }
    }

    #[test]
    fn action_tooltips_use_chinese_labels() {
        let search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot {
                query: "needle".into(),
                replace_query: "replacement".into(),
                match_count: 2,
                visible: true,
                replace_mode: true,
                ..SearchBarSnapshot::default()
            },
        );
        for (rect, expected) in [
            (search_bar.layout.toggle_replace_btn_rect, "隐藏替换"),
            (search_bar.layout.regex_btn_rect, "正则表达式"),
            (search_bar.layout.prev_btn_rect, "上一个匹配"),
            (search_bar.layout.next_btn_rect, "下一个匹配"),
            (search_bar.layout.replace_btn_rect, "替换"),
            (search_bar.layout.replace_all_btn_rect, "全部替换"),
        ] {
            let hint = search_bar
                .tooltip_at(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
                .expect("visible action should expose a tooltip");
            assert_eq!(hint.label, expected);
        }
    }

    #[test]
    fn cmd_a_selects_find_box_text() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;

        let mut w = setup_search_bar("hello");
        let theme = test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

        let action = w.on_event(&Event::KeyDown(KeyCode::Char('a'), cmd), &mut ctx);
        assert_eq!(action, Some(WidgetAction::Consumed));
        assert_eq!(w.find_box.selection_text(), Some("hello"));
    }

    #[test]
    fn dragging_find_text_keeps_mouse_capture_and_forwards_pointer_moves() {
        use crate::core::EventCtx;

        let mut search_bar = setup_search_bar("hello");
        let theme = test_theme();
        let mut event_context = EventCtx::new(&theme, 1.0);
        let find_rect = search_bar.find_box.rect();

        assert!(
            search_bar
                .on_event(
                    &Event::MouseDown {
                        px: find_rect.x + 1.0,
                        py: find_rect.y + find_rect.h * 0.5,
                        button: MouseButton::Left,
                    },
                    &mut event_context,
                )
                .is_some()
        );
        assert!(search_bar.is_capturing());
        assert_eq!(
            search_bar.on_event(
                &Event::MouseMove {
                    px: find_rect.right() - 1.0,
                    py: find_rect.y + find_rect.h * 0.5,
                },
                &mut event_context,
            ),
            Some(WidgetAction::Consumed)
        );
    }

    #[test]
    fn search_bar_leave_clears_hover_and_cancel_ends_child_interaction() {
        let mut search_bar = setup_search_bar("hello");
        let theme = test_theme();
        let mut event_context = EventCtx::new(&theme, 1.0);
        let close_rect = search_bar.close_btn_rect();
        let find_rect = search_bar.find_box.rect();

        assert_eq!(
            search_bar.on_event(
                &Event::MouseMove {
                    px: close_rect.x + close_rect.w * 0.5,
                    py: close_rect.y + close_rect.h * 0.5,
                },
                &mut event_context,
            ),
            Some(WidgetAction::SearchBar(SearchBarAction::HoverChanged))
        );
        assert_eq!(search_bar.hovered_btn, HoveredButton::CloseBar);
        let _ = search_bar.on_event(
            &Event::MouseDown {
                px: find_rect.x + 1.0,
                py: find_rect.y + find_rect.h * 0.5,
                button: MouseButton::Left,
            },
            &mut event_context,
        );
        let _ = search_bar.on_event(
            &Event::ImePreedit { text: "未完成".to_owned(), cursor: Some((0, 6)) },
            &mut event_context,
        );

        assert_eq!(
            search_bar.on_event(&Event::PointerLeave, &mut event_context),
            Some(WidgetAction::SearchBar(SearchBarAction::HoverChanged))
        );
        assert_eq!(search_bar.hovered_btn, HoveredButton::None);
        assert!(search_bar.is_capturing());
        assert!(search_bar.find_box.has_preedit());

        assert_eq!(
            search_bar.on_event(&Event::InteractionCancel, &mut event_context),
            Some(WidgetAction::Consumed)
        );
        assert!(!search_bar.is_capturing());
        assert!(!search_bar.find_box.has_preedit());
        assert_eq!(search_bar.on_event(&Event::InteractionCancel, &mut event_context), None);
    }

    #[test]
    fn hiding_search_bar_releases_child_capture_and_preedit() {
        let mut search_bar = setup_search_bar("hello");
        let theme = test_theme();
        let mut event_context = EventCtx::new(&theme, 1.0);
        let find_rect = search_bar.find_box.rect();
        let _ = search_bar.on_event(
            &Event::MouseDown {
                px: find_rect.x + 1.0,
                py: find_rect.y + find_rect.h * 0.5,
                button: MouseButton::Left,
            },
            &mut event_context,
        );
        let _ = search_bar.on_event(
            &Event::ImePreedit { text: "未完成".to_owned(), cursor: Some((0, 6)) },
            &mut event_context,
        );
        assert!(search_bar.is_capturing());
        assert!(search_bar.find_box.has_preedit());

        search_bar.set_input(SearchBarSnapshot {
            query: "hello".to_owned(),
            visible: false,
            ..SearchBarSnapshot::default()
        });

        assert!(!search_bar.is_capturing());
        assert!(!search_bar.find_box.has_preedit());
    }

    #[test]
    fn cmd_c_copies_find_box_selection() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;
        let mut w = setup_search_bar("hello");
        w.find_box.select_all();

        let theme = test_theme();
        let mut clipboard = TestClipboard::default();
        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

        {
            let mut ctx = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
            let action = w.on_event(&Event::KeyDown(KeyCode::Char('c'), cmd), &mut ctx);
            assert_eq!(action, Some(WidgetAction::Consumed));
        }
        assert_eq!(clipboard.text.as_deref(), Some("hello"));
    }

    #[test]
    fn cmd_v_pastes_into_focused_find_box() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;
        let mut w = setup_search_bar("");

        let theme = test_theme();
        let mut clipboard = TestClipboard { text: Some("pasted".to_owned()) };
        let mut ctx = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

        w.on_event(&Event::KeyDown(KeyCode::Char('v'), cmd), &mut ctx);
        assert_eq!(w.find_box.text(), "pasted");
    }

    #[test]
    fn delete_after_select_all_keeps_find_focus() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;

        let mut w = setup_search_bar("hello");
        w.find_box.select_all();

        let theme = test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);

        w.on_event(&Event::KeyDown(KeyCode::Delete, Modifiers::NONE), &mut ctx);
        assert_eq!(w.find_box.text(), "");
        assert!(w.find_box.is_focused());
    }

    #[test]
    fn replace_box_receives_clipboard_shortcuts_when_focused() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;
        let mut w = setup_search_bar("find");
        w.set_input(SearchBarSnapshot {
            query: "find".into(),
            replace_query: "repl".into(),
            visible: true,
            replace_mode: true,
            focus_replace: true,
            ..Default::default()
        });
        w.replace_box.select_all();

        let theme = test_theme();
        let mut clipboard = TestClipboard::default();
        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

        {
            let mut ctx = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
            w.on_event(&Event::KeyDown(KeyCode::Char('c'), cmd), &mut ctx);
        }
        assert_eq!(clipboard.text.as_deref(), Some("repl"));
    }

    #[test]
    fn escape_keeps_replace_focus_behavior_without_dismissing() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;

        let mut w = setup_search_bar("find");
        w.set_input(SearchBarSnapshot {
            query: "find".into(),
            replace_query: "repl".into(),
            visible: true,
            replace_mode: true,
            focus_replace: true,
            ..Default::default()
        });

        let theme = test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(
            w.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut ctx),
            Some(WidgetAction::Consumed)
        );
    }

    #[test]
    fn escape_dismisses_when_find_box_is_focused() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;

        let mut w = setup_search_bar("find");
        let theme = test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(
            w.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut ctx),
            Some(WidgetAction::SearchBar(SearchBarAction::DismissOrClear))
        );
    }

    #[test]
    fn find_box_widget_actions_map_to_existing_search_bar_actions() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;

        let mut w = setup_search_bar("");
        let theme = test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(
            w.on_event(&Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE), &mut ctx),
            Some(WidgetAction::SearchBar(SearchBarAction::QueryChanged("x".into())))
        );
        assert_eq!(
            w.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut ctx),
            Some(WidgetAction::SearchBar(SearchBarAction::Next))
        );
    }
}
