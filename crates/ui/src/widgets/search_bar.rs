//! SearchBarWidget — 搜索面板的绘制 + 键盘事件转译。
//! 宿主通过快照注入显隐、查询词和匹配计数，并按产品转译查找动作。
mod layout;

/// Height of the search bar in logical pixels (before DPI scaling).
pub use crate::constants::BAR_HEIGHT as SEARCH_BAR_HEIGHT;

use self::layout::{
    SearchBarLayout, SearchBarLayoutInput, calculate_full_width_find_layout,
    calculate_search_bar_layout,
};
use crate::core::widget::MouseButton;
use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction, WidgetId,
};
use crate::widgets::button::{Button, ButtonCollection, ButtonStyle};
use crate::widgets::icon::draw_icon;
use crate::widgets::tooltip::TooltipHint;
use std::any::Any;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SearchControl {
    Close,
    ToggleReplace,
    Regex,
    Prev,
    Next,
    Replace,
    ReplaceAll,
}

impl SearchControl {
    const ALL: [Self; 7] = [
        Self::Close,
        Self::ToggleReplace,
        Self::Regex,
        Self::Prev,
        Self::Next,
        Self::Replace,
        Self::ReplaceAll,
    ];

    fn id(self) -> WidgetId {
        match self {
            Self::Close => WidgetId(0x7365_6172_6368_636c),
            Self::ToggleReplace => WidgetId(0x7365_6172_6368_7467),
            Self::Regex => WidgetId(0x7365_6172_6368_7267),
            Self::Prev => WidgetId(0x7365_6172_6368_7072),
            Self::Next => WidgetId(0x7365_6172_6368_6e78),
            Self::Replace => WidgetId(0x7365_6172_6368_7270),
            Self::ReplaceAll => WidgetId(0x7365_6172_6368_7261),
        }
    }

    fn rect(self, layout: &SearchBarLayout) -> Rect {
        match self {
            Self::Close => layout.close_btn_rect,
            Self::ToggleReplace => layout.toggle_replace_btn_rect,
            Self::Regex => layout.regex_btn_rect,
            Self::Prev => layout.prev_btn_rect,
            Self::Next => layout.next_btn_rect,
            Self::Replace => layout.replace_btn_rect,
            Self::ReplaceAll => layout.replace_all_btn_rect,
        }
    }

    fn accessibility_label(self) -> &'static str {
        match self {
            Self::Close => "关闭查找",
            Self::ToggleReplace => "显示或隐藏替换",
            Self::Regex => "正则表达式",
            Self::Prev => "上一个匹配",
            Self::Next => "下一个匹配",
            Self::Replace => "替换",
            Self::ReplaceAll => "全部替换",
        }
    }
}

/// app 端注入的纯数据（widget 内部不知道 doc / search_state 概念）。
#[derive(Clone, Default)]
pub struct SearchBarSnapshot {
    pub query: String,
    pub match_count: usize,
    pub current_match: usize,
    pub visible: bool,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchBarEvent {
    pub consumed: bool,
    pub action: Option<SearchBarAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchBarMode {
    FindOnly,
    FindReplace,
}

use crate::widgets::text_box::{TextBox, TextBoxIme};

const FIND_BOX_ID: WidgetId = WidgetId(0x7365_6172_6368_6669);
const REPLACE_BOX_ID: WidgetId = WidgetId(0x7365_6172_6368_7265);
const SEARCH_FONT_SIZE_LOGICAL: f32 = 14.0;
const BUTTON_WIDE_GLYPH_WIDTH_LOGICAL: f32 = 14.0;
const SEARCH_CONTROL_ICON_SIZE_LOGICAL: f32 = 14.0;

fn search_control_style(theme: &crate::theme::Theme) -> ButtonStyle {
    let mut style = ButtonStyle::ghost(theme.application_theme());
    let mut foreground = theme.palette.input_fg;
    foreground[3] *= 0.6;
    style.foreground = foreground;
    style.selected_foreground = theme.application_theme().accent_text;
    style.selected_background = [0.0; 4];
    style.font_size_logical = SEARCH_FONT_SIZE_LOGICAL;
    style.pad_x_logical = 0.0;
    style
}

pub struct SearchBarWidget {
    mode: SearchBarMode,
    input_focused: bool,
    rect: Rect,
    layout: SearchBarLayout,
    snap: SearchBarSnapshot,
    find_box: TextBox,
    replace_box: TextBox,
    controls: ButtonCollection,
}

impl Default for SearchBarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchBarWidget {
    fn map_button_widget_action(&mut self, action: WidgetAction) -> Option<WidgetAction> {
        let id = match action {
            WidgetAction::Control(crate::core::widget::ControlAction::Activated { id }) => id,
            WidgetAction::Control(crate::core::widget::ControlAction::FocusRequested { id }) => {
                if self.mode == SearchBarMode::FindOnly {
                    return Some(WidgetAction::SearchBar(SearchBarAction::FocusFind));
                }
                self.controls.set_keyboard_focus(Some(id));
                return Some(WidgetAction::SearchBar(SearchBarAction::HoverChanged));
            }
            _ => return Some(WidgetAction::SearchBar(SearchBarAction::HoverChanged)),
        };
        let control = SearchControl::ALL.into_iter().find(|control| control.id() == id)?;
        let action = match control {
            SearchControl::Close => SearchBarAction::Close,
            SearchControl::ToggleReplace => SearchBarAction::ToggleReplace,
            SearchControl::Regex => SearchBarAction::ToggleRegex,
            SearchControl::Prev => SearchBarAction::Prev,
            SearchControl::Next => SearchBarAction::Next,
            SearchControl::Replace => SearchBarAction::Replace,
            SearchControl::ReplaceAll => SearchBarAction::ReplaceAll,
        };
        Some(WidgetAction::SearchBar(action))
    }

    pub fn new() -> Self {
        Self::with_mode(SearchBarMode::FindReplace)
    }

    /// Standalone find row with screen-coordinate layout and no editing controls.
    pub fn find_only() -> Self {
        Self::with_mode(SearchBarMode::FindOnly)
    }

    fn with_mode(mode: SearchBarMode) -> Self {
        let mut find_box = TextBox::with_id(FIND_BOX_ID);
        if mode == SearchBarMode::FindOnly {
            find_box.set_placeholder("在文档中查找…");
            find_box.set_accessibility_label(Some("文档查找".to_owned()));
        } else {
            find_box.set_placeholder("查找…");
            find_box.set_max_len_bytes(2048);
        }

        let mut replace_box = TextBox::with_id(REPLACE_BOX_ID);
        replace_box.set_placeholder("替换为…");
        replace_box.set_max_len_bytes(2048);

        let controls = ButtonCollection::new(
            SearchControl::ALL
                .iter()
                .map(|control| {
                    let mut button = Button::new(
                        control.id(),
                        search_control_style(&crate::theme::test_theme()),
                    );
                    button.set_accessibility_label(Some(control.accessibility_label().to_owned()));
                    button.set_icon_size(SEARCH_CONTROL_ICON_SIZE_LOGICAL);
                    if *control == SearchControl::Regex {
                        button.set_icon(Some("regex".to_owned()));
                    }
                    button
                })
                .collect(),
        );

        Self {
            mode,
            input_focused: false,
            rect: Rect::ZERO,
            layout: SearchBarLayout::default(),
            snap: SearchBarSnapshot::default(),
            find_box,
            replace_box,
            controls,
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
        self.set_input_with_focus(snap, true);
    }

    pub fn set_input_with_focus(&mut self, mut snap: SearchBarSnapshot, focused: bool) {
        self.input_focused = snap.visible && focused;
        if self.mode == SearchBarMode::FindOnly {
            snap.replace_mode = false;
            snap.focus_replace = false;
            snap.options_use_regex = false;
        }
        if !snap.visible {
            let theme = crate::theme::test_theme();
            let mut context = EventCtx::new(&theme, 1.0);
            let _ = self.controls.on_event(&Event::InteractionCancel, &mut context);
            self.controls.set_keyboard_focus(None);
            self.find_box.cancel_transient_interaction();
            self.replace_box.cancel_transient_interaction();
        }
        self.find_box.sync_text(&snap.query);
        self.replace_box.sync_text(&snap.replace_query);
        self.find_box.set_focus(self.input_focused && (!snap.replace_mode || !snap.focus_replace));
        self.replace_box.set_focus(self.input_focused && snap.replace_mode && snap.focus_replace);
        for (control, button) in SearchControl::ALL.into_iter().zip(self.controls.buttons_mut()) {
            button.set_selected(control == SearchControl::Regex && snap.options_use_regex);
            let requires_matches = if self.mode == SearchBarMode::FindOnly {
                matches!(control, SearchControl::Prev | SearchControl::Next)
            } else {
                matches!(control, SearchControl::Replace | SearchControl::ReplaceAll)
            };
            let available = self.mode != SearchBarMode::FindOnly
                || matches!(
                    control,
                    SearchControl::Prev | SearchControl::Next | SearchControl::Close
                );
            button.set_enabled(available && (!requires_matches || snap.match_count > 0));
        }
        self.snap = snap;
    }

    pub fn select_query(&mut self) {
        self.find_box.select_all();
    }

    pub fn input_has_focus(&self) -> bool {
        self.input_focused
    }

    pub fn ime_allowed(&self) -> bool {
        self.input_focused
            && if self.snap.replace_mode && self.snap.focus_replace {
                self.replace_box.ime_allowed()
            } else {
                self.find_box.ime_allowed()
            }
    }

    pub fn ime_cursor_rect(&self) -> Option<Rect> {
        if !self.ime_allowed() {
            return None;
        }
        self.focused_textbox_ime_cursor_rect()
    }

    /// Route an event for a standalone search row whose parent owns editor fallthrough.
    pub fn route_event(&mut self, event: &Event, context: &mut EventCtx) -> SearchBarEvent {
        if !self.snap.visible {
            return SearchBarEvent::default();
        }
        let keyboard = matches!(
            event,
            Event::KeyDown(..)
                | Event::ImePreedit { .. }
                | Event::ImeCommit(..)
                | Event::ImeEnable
                | Event::ImeDisable
        );
        if keyboard && !self.input_focused {
            return SearchBarEvent::default();
        }
        let inside = match event {
            Event::MouseDown { px, py, .. }
            | Event::MouseUp { px, py, .. }
            | Event::Wheel { px, py, .. } => self.hit(*px, *py),
            _ => false,
        };
        let captured =
            self.is_capturing() && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. });
        let widget_action = self.on_event(event, context);
        SearchBarEvent {
            consumed: keyboard || inside || captured || widget_action.is_some(),
            action: match widget_action {
                Some(WidgetAction::SearchBar(action)) => Some(action),
                _ => None,
            },
        }
    }

    pub fn is_visible(&self) -> bool {
        self.snap.visible
    }

    pub fn next_cursor_blink_at(&self) -> Option<std::time::Instant> {
        if !self.snap.visible {
            return None;
        }
        if self.snap.replace_mode && self.snap.focus_replace {
            self.replace_box.next_cursor_blink_at()
        } else {
            self.find_box.next_cursor_blink_at()
        }
    }

    pub fn advance_cursor_blink(&mut self, now: std::time::Instant) -> bool {
        if !self.snap.visible {
            return false;
        }
        if self.snap.replace_mode && self.snap.focus_replace {
            self.replace_box.advance_cursor_blink(now)
        } else {
            self.find_box.advance_cursor_blink(now)
        }
    }
}

impl Widget for SearchBarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        if self.mode == SearchBarMode::FindOnly {
            self.rect = rect;
            self.layout = calculate_full_width_find_layout(rect, ctx.dpi);
        } else {
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
        }
        self.find_box.layout(self.layout.find_input_rect, ctx);
        if self.snap.replace_mode {
            self.replace_box.layout(self.layout.replace_input_rect, ctx);
        }
        for (control, button) in SearchControl::ALL.iter().zip(self.controls.buttons_mut()) {
            button.set_style(if self.mode == SearchBarMode::FindOnly {
                ButtonStyle::ghost(ctx.theme.application_theme())
            } else {
                search_control_style(ctx.theme)
            });
            button.set_text(match control {
                SearchControl::Close => Some("✕".to_owned()),
                SearchControl::ToggleReplace => {
                    Some(if self.snap.replace_mode { "▲" } else { "▼" }.to_owned())
                }
                SearchControl::Regex => None,
                SearchControl::Prev => Some("◀".to_owned()),
                SearchControl::Next => Some("▶".to_owned()),
                SearchControl::Replace => {
                    Some(if self.layout.full_replace_labels { "更换" } else { "换" }.to_owned())
                }
                SearchControl::ReplaceAll => {
                    Some(if self.layout.full_replace_labels { "全部" } else { "全" }.to_owned())
                }
            });
        }
        let button_rects =
            SearchControl::ALL.iter().map(|control| control.rect(&self.layout)).collect();
        self.controls.set_rects(button_rects, ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 || !self.snap.visible {
            return;
        }
        let dpi = ctx.dpi;
        let font_size = SEARCH_FONT_SIZE_LOGICAL * dpi;
        let baseline = self.rect.h * 0.5 + font_size * 0.35;

        if self.mode == SearchBarMode::FindOnly {
            self.paint_document_find_row(ctx, dpi);
        } else if self.snap.replace_mode {
            self.paint_find_replace(ctx, dpi, baseline);
        } else {
            self.paint_find_only(ctx, dpi, baseline);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.snap.visible && self.layout.pill_rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.snap.visible {
            return None;
        }

        match ev {
            Event::PointerLeave => {
                let button_changed = self.controls.on_event(ev, _ctx).is_some();
                let _ = self.find_box.on_event(ev, _ctx);
                let _ = self.replace_box.on_event(ev, _ctx);
                button_changed.then_some(WidgetAction::SearchBar(SearchBarAction::HoverChanged))
            }
            Event::InteractionCancel => {
                let button_changed = self.controls.on_event(ev, _ctx).is_some();
                let find_changed = self.find_box.on_event(ev, _ctx).is_some();
                let replace_changed = self.replace_box.on_event(ev, _ctx).is_some();
                (button_changed || find_changed || replace_changed)
                    .then_some(WidgetAction::Consumed)
            }
            Event::KeyDown(kc, modifiers) => {
                if self.mode == SearchBarMode::FindOnly
                    && *kc == KeyCode::Enter
                    && self.find_box.has_preedit()
                {
                    return Some(WidgetAction::Consumed);
                }
                if let Some(action) = self.controls.on_event(ev, _ctx) {
                    return self.map_button_widget_action(action);
                }
                if self.mode == SearchBarMode::FindOnly {
                    let action = match kc {
                        KeyCode::Escape => Some(SearchBarAction::Close),
                        KeyCode::Enter => Some(if modifiers.shift {
                            SearchBarAction::Prev
                        } else {
                            SearchBarAction::Next
                        }),
                        _ => None,
                    };
                    if let Some(action) = action {
                        return Some(WidgetAction::SearchBar(action));
                    }
                    let action = self.find_box.on_event(ev, _ctx);
                    return self.map_text_box_widget_action(action);
                }
                // Handle tab locally
                if *kc == KeyCode::Tab {
                    self.controls.set_keyboard_focus(None);
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
            Event::MouseMove { .. } => {
                let text_box_action = if self.find_box.is_capturing() {
                    self.find_box.on_event(ev, _ctx)
                } else if self.replace_box.is_capturing() {
                    self.replace_box.on_event(ev, _ctx)
                } else {
                    None
                };
                if text_box_action.is_some()
                    || (self.mode == SearchBarMode::FindOnly && self.find_box.is_capturing())
                {
                    return self.map_text_box_widget_action(text_box_action);
                }
                self.controls
                    .on_event(ev, _ctx)
                    .and_then(|action| self.map_button_widget_action(action))
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                if self.controls.index_at(*px, *py).is_some() {
                    return self
                        .controls
                        .on_event(ev, _ctx)
                        .and_then(|action| self.map_button_widget_action(action))
                        .or(Some(WidgetAction::Consumed));
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
                if self.controls.is_capturing() {
                    let action = self.controls.on_event(ev, _ctx)?;
                    return self.map_button_widget_action(action);
                }
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
        self.controls.is_capturing()
            || self.find_box.is_capturing()
            || self.replace_box.is_capturing()
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        if !self.snap.visible {
            return;
        }
        self.find_box.collect_focusable_ids(output);
        if self.snap.replace_mode {
            self.replace_box.collect_focusable_ids(output);
        }
        self.controls.collect_focusable_ids(output);
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        if focused_id == Some(crate::core::widget::ids::SEARCH_BAR) {
            return;
        }
        self.controls.set_keyboard_focus(if self.snap.visible { focused_id } else { None });
    }

    fn collect_accessibility_nodes(
        &self,
        context: &crate::core::AccessibilityContext,
        output: &mut Vec<crate::core::AccessibilityNode>,
    ) {
        if !self.snap.visible {
            return;
        }
        self.find_box.collect_accessibility_nodes(context, output);
        if self.snap.replace_mode {
            self.replace_box.collect_accessibility_nodes(context, output);
        }
        self.controls.collect_accessibility_nodes(context, output);
    }

    fn on_accessibility_action(
        &mut self,
        request: &crate::core::AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if !self.snap.visible {
            return None;
        }
        if let Some(action) = self.controls.on_accessibility_action(request) {
            if let WidgetAction::Control(crate::core::widget::ControlAction::FocusRequested {
                id,
            }) = action
            {
                self.controls.set_keyboard_focus(Some(id));
                return Some(WidgetAction::Control(
                    crate::core::widget::ControlAction::FocusRequested {
                        id: crate::core::widget::ids::SEARCH_BAR,
                    },
                ));
            }
            return self.map_button_widget_action(action);
        }
        let text_box_action = self.find_box.on_accessibility_action(request).or_else(|| {
            self.snap
                .replace_mode
                .then(|| self.replace_box.on_accessibility_action(request))
                .flatten()
        });
        self.map_text_box_widget_action(text_box_action)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        if !self.snap.visible {
            return None;
        }
        for control in SearchControl::ALL {
            let rect = control.rect(&self.layout);
            if rect.w <= 0.0 || !rect.contains(px, py) {
                continue;
            }
            let label = match (self.mode, control) {
                (SearchBarMode::FindOnly, SearchControl::Prev) => "上一个匹配（Shift+Enter）",
                (SearchBarMode::FindOnly, SearchControl::Next) => "下一个匹配（Enter）",
                (SearchBarMode::FindOnly, SearchControl::Close) => "关闭查找（Esc）",
                (_, SearchControl::ToggleReplace) if self.snap.replace_mode => "隐藏替换",
                (_, SearchControl::ToggleReplace) => "显示替换",
                _ => control.accessibility_label(),
            };
            return Some(TooltipHint { label: label.to_owned(), target_rect: rect });
        }
        None
    }
}

impl SearchBarWidget {
    fn map_text_box_widget_action(&mut self, action: Option<WidgetAction>) -> Option<WidgetAction> {
        match action {
            Some(WidgetAction::Control(control_action)) => self.map_control_action(control_action),
            other => other,
        }
    }

    fn map_control_action(
        &mut self,
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
                self.controls.set_keyboard_focus(None);
                Some(WidgetAction::SearchBar(SearchBarAction::FocusFind))
            }
            crate::core::widget::ControlAction::FocusRequested { id: REPLACE_BOX_ID } => {
                self.controls.set_keyboard_focus(None);
                Some(WidgetAction::SearchBar(SearchBarAction::FocusReplace))
            }
            _ => Some(WidgetAction::Consumed),
        }
    }

    fn auxiliary_result_text(&self) -> Option<String> {
        if self.snap.query.is_empty() {
            return None;
        }
        if self.mode == SearchBarMode::FindReplace && self.snap.match_count == 0 {
            return Some("无匹配结果".to_owned());
        }
        Some(layout::format_match_count(self.snap.match_count, self.snap.current_match))
    }

    fn paint_document_find_row(&self, ctx: &mut PaintCtx, dpi: f32) {
        const COUNT_BASELINE_CENTER_SHIFT: f32 = 0.35;

        ctx.list.fill(self.rect, ctx.theme.palette.input_bg);
        self.find_box.paint(ctx);
        self.controls.paint(ctx);
        let Some(label) = self.auxiliary_result_text() else {
            return;
        };
        let font_size = crate::constants::CAPTION_FONT_SIZE * dpi;
        let count = self.layout.auxiliary_text_rect;
        ctx.text(
            count.x + crate::constants::TINY_GAP * dpi,
            count.y + count.h * 0.5 + font_size * COUNT_BASELINE_CENTER_SHIFT,
            font_size,
            ctx.theme.palette.input_fg,
            &label,
        );
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
        self.controls.paint(ctx);
        self.paint_auxiliary_text(ctx, dpi, baseline);
    }

    fn paint_auxiliary_text(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        let rect = self.layout.auxiliary_text_rect;
        if rect.w <= 0.0 {
            return;
        }
        let Some(label) = self.auxiliary_result_text() else {
            return;
        };
        let color = if self.snap.match_count > 0 {
            ctx.theme.palette.input_fg
        } else {
            ctx.theme.application_theme().danger_text
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
    use crate::Modifiers;
    use crate::core::Clipboard;
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::DrawList;
    use crate::theme::test_theme;

    fn visible_find_only_bar(query: &str, focused: bool, count: usize) -> SearchBarWidget {
        let mut widget = SearchBarWidget::find_only();
        widget.set_input_with_focus(
            SearchBarSnapshot {
                query: query.to_owned(),
                visible: true,
                match_count: count,
                ..Default::default()
            },
            focused,
        );
        let theme = crate::theme::test_theme();
        let mut measure = crate::NoopMeasure;
        let mut context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        widget.set_rect(Rect::new(100.0, 50.0, 300.0, SEARCH_BAR_HEIGHT), &mut context);
        widget
    }

    #[test]
    fn full_width_find_ime_preedit_is_consumed_without_changing_query_and_commit_edits_query() {
        let mut widget = visible_find_only_bar("", true, 0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let preedit = widget.route_event(
            &Event::ImePreedit { text: "中文".to_owned(), cursor: Some((0, 6)) },
            &mut context,
        );
        assert!(preedit.consumed);
        assert_eq!(widget.find_box.text(), "");
        assert!(widget.find_box.has_preedit());
        assert_eq!(
            widget.route_event(&Event::ImeCommit("中文".to_owned()), &mut context).action,
            Some(SearchBarAction::QueryChanged("中文".to_owned()))
        );
        assert!(widget.ime_cursor_rect().expect("focused query has IME rectangle").x >= 100.0);
    }

    #[test]
    fn full_width_find_snapshot_keeps_selection_and_preedit_until_focus_is_lost() {
        let mut widget = visible_find_only_bar("中文", true, 1);
        widget.select_query();
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        widget.route_event(
            &Event::ImePreedit { text: "zhong".to_owned(), cursor: None },
            &mut context,
        );
        widget.set_input_with_focus(widget.snap.clone(), true);
        assert_eq!(widget.find_box.selection_text(), Some("中文"));
        assert!(widget.find_box.has_preedit());
        widget.set_input_with_focus(widget.snap.clone(), false);
        assert!(!widget.find_box.has_preedit());
        assert!(!widget.ime_allowed());
        assert!(widget.ime_cursor_rect().is_none());
    }

    #[test]
    fn full_width_find_unfocused_query_does_not_consume_keyboard_or_ime() {
        let mut widget = visible_find_only_bar("", false, 0);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        for event in [
            Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE),
            Event::KeyDown(KeyCode::Enter, Modifiers::NONE),
            Event::ImeCommit("中文".to_owned()),
            Event::ImeEnable,
            Event::ImeDisable,
        ] {
            assert!(!widget.route_event(&event, &mut context).consumed);
        }
        assert_eq!(widget.find_box.text(), "");
    }

    #[test]
    fn full_width_find_enter_during_ime_composition_does_not_navigate_results() {
        let mut widget = visible_find_only_bar("中文", true, 2);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        widget.route_event(
            &Event::ImePreedit { text: "拼".to_owned(), cursor: Some((0, 3)) },
            &mut context,
        );
        let routed =
            widget.route_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context);
        assert!(routed.consumed);
        assert_ne!(routed.action, Some(SearchBarAction::Next));
    }

    #[test]
    fn full_width_find_narrow_layout_uses_global_coordinates_and_buttons_navigate() {
        let mut widget = visible_find_only_bar("中文", true, 2);
        assert!(widget.find_box.rect().w > 100.0);
        assert!(widget.find_box.rect().x >= widget.rect.x);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        for (rect, expected) in [
            (widget.layout.prev_btn_rect, SearchBarAction::Prev),
            (widget.layout.next_btn_rect, SearchBarAction::Next),
            (widget.layout.close_btn_rect, SearchBarAction::Close),
        ] {
            assert!(rect.x >= widget.rect.x && rect.x + rect.w <= widget.rect.x + widget.rect.w);
            let px = rect.x + rect.w * 0.5;
            let py = rect.y + rect.h * 0.5;
            widget.route_event(
                &Event::MouseDown { px, py, button: crate::MouseButton::Left },
                &mut context,
            );
            assert_eq!(
                widget
                    .route_event(
                        &Event::MouseUp { px, py, button: crate::MouseButton::Left },
                        &mut context
                    )
                    .action,
                Some(expected)
            );
        }
    }

    #[test]
    fn full_width_find_no_results_shows_zero_and_disables_navigation() {
        let mut widget = visible_find_only_bar("不存在", true, 0);
        assert_eq!(widget.auxiliary_result_text().as_deref(), Some("0/0"));
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let rect = widget.layout.prev_btn_rect;
        let px = rect.x + rect.w * 0.5;
        let py = rect.y + rect.h * 0.5;
        widget.route_event(
            &Event::MouseDown { px, py, button: crate::MouseButton::Left },
            &mut context,
        );
        assert_eq!(
            widget
                .route_event(
                    &Event::MouseUp { px, py, button: crate::MouseButton::Left },
                    &mut context
                )
                .action,
            None
        );
        widget
            .set_input_with_focus(SearchBarSnapshot { visible: true, ..Default::default() }, false);
        assert_eq!(widget.auxiliary_result_text(), None);
    }

    #[test]
    fn full_width_find_dragging_query_over_button_keeps_text_selection_capture() {
        let mut widget = visible_find_only_bar("abcdef", true, 1);
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let rect = widget.find_box.rect();
        let py = rect.y + rect.h * 0.5;
        widget.route_event(
            &Event::MouseDown { px: rect.x + 1.0, py, button: crate::MouseButton::Left },
            &mut context,
        );
        let button_rect = widget.layout.prev_btn_rect;
        widget.route_event(&Event::MouseMove { px: button_rect.x + 1.0, py }, &mut context);
        assert!(widget.find_box.selection_text().is_some());
        widget.route_event(
            &Event::MouseUp { px: button_rect.x + 1.0, py, button: crate::MouseButton::Left },
            &mut context,
        );
        assert!(!widget.is_capturing());
    }

    #[test]
    fn full_width_find_focused_enter_navigates_and_escape_closes() {
        let mut widget = SearchBarWidget::find_only();
        widget.set_input(SearchBarSnapshot { visible: true, ..Default::default() });
        let theme = crate::theme::test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            widget
                .route_event(&Event::KeyDown(KeyCode::Enter, Modifiers::default()), &mut context)
                .action,
            Some(SearchBarAction::Next)
        );
        assert_eq!(
            widget
                .route_event(
                    &Event::KeyDown(
                        KeyCode::Enter,
                        Modifiers { shift: true, ..Default::default() }
                    ),
                    &mut context
                )
                .action,
            Some(SearchBarAction::Prev)
        );
        assert_eq!(
            widget
                .route_event(&Event::KeyDown(KeyCode::Escape, Modifiers::default()), &mut context)
                .action,
            Some(SearchBarAction::Close)
        );
    }
    #[test]
    fn full_width_find_hiding_releases_capture_and_clears_ime() {
        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let hit_rects: [fn(&SearchBarWidget) -> Rect; 2] =
            [|bar| bar.find_box.rect(), |bar| bar.layout.next_btn_rect];
        for press_rect in hit_rects {
            let mut bar = visible_find_only_bar("needle", true, 2);
            let rect = press_rect(&bar);
            let press = Event::MouseDown {
                px: rect.x + rect.w * 0.5,
                py: rect.y + rect.h * 0.5,
                button: MouseButton::Left,
            };
            assert!(bar.route_event(&press, &mut context).consumed);
            assert!(bar.is_capturing());
            bar.route_event(
                &Event::ImePreedit { text: "拼".to_owned(), cursor: None },
                &mut context,
            );
            bar.set_input_with_focus(SearchBarSnapshot::default(), false);

            assert!(!bar.is_capturing());
            assert!(!bar.find_box.has_preedit());
            let Event::MouseDown { px, py, button } = press else {
                unreachable!("press fixture must be a mouse-down event");
            };
            assert_eq!(
                bar.route_event(&Event::MouseUp { px, py, button }, &mut context),
                SearchBarEvent::default()
            );
        }
    }

    #[test]
    fn full_width_find_exposes_only_enabled_query_and_navigation_focus_targets() {
        let mut bar = visible_find_only_bar("missing", true, 0);
        let mut ids = Vec::new();
        bar.collect_focusable_ids(&mut ids);
        assert_eq!(ids, [FIND_BOX_ID, SearchControl::Close.id()]);

        let mut snapshot = bar.snap.clone();
        snapshot.match_count = 2;
        bar.set_input(snapshot);
        ids.clear();
        bar.collect_focusable_ids(&mut ids);
        assert_eq!(
            ids,
            [
                FIND_BOX_ID,
                SearchControl::Close.id(),
                SearchControl::Prev.id(),
                SearchControl::Next.id()
            ]
        );
    }

    #[test]
    fn full_width_find_routes_accessibility_focus_to_the_visible_previous_control() {
        use crate::core::widget::{ControlAction, ids};
        use crate::core::{AccessibilityAction, AccessibilityActionRequest, AccessibilityContext};

        let mut bar = visible_find_only_bar("needle", true, 2);
        let mut nodes = Vec::new();
        bar.collect_accessibility_nodes(&AccessibilityContext::new(0.0, 0.0), &mut nodes);
        let previous = nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("上一个匹配"))
            .expect("previous result exposes an accessibility button");
        assert_eq!(previous.bounds, bar.layout.prev_btn_rect);
        let focus_action = bar.on_accessibility_action(&AccessibilityActionRequest::new(
            previous.id,
            AccessibilityAction::Focus,
        ));
        assert_eq!(
            focus_action,
            Some(WidgetAction::Control(ControlAction::FocusRequested { id: ids::SEARCH_BAR }))
        );

        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            bar.route_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            SearchBarEvent { consumed: true, action: Some(SearchBarAction::Prev) }
        );
    }

    #[test]
    fn no_match_message_remains_readable_on_light_input_surface() {
        use crate::core::paint::DrawCmd;

        const MINIMUM_TEXT_CONTRAST: f32 = 4.5;
        let search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot { query: "needle".into(), visible: true, ..Default::default() },
        );
        let theme =
            crate::theme::Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("search contrast test needs text shaping");
        let mut context = PaintCtx::new(&mut draw_list, &theme, 1.0);
        context.shaper = Some(&mut shaper);
        search_bar.paint(&mut context);
        let foreground = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TextLayout { layout, color, .. } if layout.text == "无匹配结果" => {
                    Some(*color)
                }
                _ => None,
            })
            .expect("search with no matches paints its message");
        let luminance = |color: [f32; 4]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        let contrast = (luminance(theme.palette.input_bg) + 0.05) / (luminance(foreground) + 0.05);
        assert!(contrast >= MINIMUM_TEXT_CONTRAST, "no-match text contrast: {contrast}");
    }

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
            match_count: 0,
            current_match: 0,
            visible: true,

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
        let mut pctx =
            PaintCtx { list: &mut dl, theme: &theme, dpi: 1.0, global_alpha: 1.0, shaper: None };
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
        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let px = close_rect.x + close_rect.w * 0.5;
        let py = close_rect.y + close_rect.h * 0.5;
        assert_eq!(
            search_bar
                .on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context),
            Some(WidgetAction::SearchBar(SearchBarAction::HoverChanged))
        );
        assert_eq!(
            search_bar
                .on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut context),
            Some(WidgetAction::SearchBar(SearchBarAction::Close))
        );
    }

    #[test]
    fn search_control_release_outside_does_not_activate() {
        let mut search_bar = setup_search_bar("needle");
        let close_rect = search_bar.close_btn_rect();
        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let px = close_rect.x + close_rect.w * 0.5;
        let py = close_rect.y + close_rect.h * 0.5;

        let _ = search_bar
            .on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context);
        assert!(search_bar.is_capturing());
        assert_eq!(
            search_bar.on_event(
                &Event::MouseUp { px: 0.0, py: 0.0, button: MouseButton::Left },
                &mut context
            ),
            Some(WidgetAction::SearchBar(SearchBarAction::HoverChanged))
        );
        assert!(!search_bar.is_capturing());
    }

    #[test]
    fn search_control_supports_accessibility_focus_and_keyboard_activation() {
        let mut search_bar = setup_search_bar("needle");
        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let target = crate::core::AccessibilityId::from(SearchControl::Prev.id());

        assert_eq!(
            search_bar.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                target,
                crate::core::AccessibilityAction::Focus,
            )),
            Some(WidgetAction::Control(crate::core::widget::ControlAction::FocusRequested {
                id: crate::core::widget::ids::SEARCH_BAR,
            },)),
        );
        assert_eq!(
            search_bar.on_event(
                &Event::KeyDown(KeyCode::Enter, crate::core::Modifiers::NONE),
                &mut context
            ),
            Some(WidgetAction::SearchBar(SearchBarAction::Prev)),
        );
    }

    #[test]
    fn clicking_search_input_clears_accessibility_button_focus() {
        let mut search_bar = layout_search_bar(
            800.0,
            SearchBarSnapshot {
                query: "needle".to_owned(),
                match_count: 1,
                visible: true,
                ..SearchBarSnapshot::default()
            },
        );
        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let target = crate::core::AccessibilityId::from(SearchControl::Prev.id());
        let _ = search_bar.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
            target,
            crate::core::AccessibilityAction::Focus,
        ));
        assert_eq!(
            search_bar.on_event(
                &Event::KeyDown(KeyCode::Enter, crate::core::Modifiers::NONE),
                &mut context,
            ),
            Some(WidgetAction::SearchBar(SearchBarAction::Prev)),
        );
        let input = search_bar.layout.find_input_rect;
        let px = input.x + input.w * 0.5;
        let py = input.y + input.h * 0.5;
        let _ = search_bar
            .on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context);
        let _ = search_bar
            .on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut context);

        assert_eq!(
            search_bar.on_event(
                &Event::KeyDown(KeyCode::Enter, crate::core::Modifiers::NONE),
                &mut context,
            ),
            Some(WidgetAction::SearchBar(SearchBarAction::Next)),
        );
    }

    #[test]
    fn disabled_replace_control_consumes_pointer_press() {
        let mut search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot { visible: true, replace_mode: true, ..SearchBarSnapshot::default() },
        );
        let rect = search_bar.layout.replace_btn_rect;
        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);

        assert_eq!(
            search_bar.on_event(
                &Event::MouseDown {
                    px: rect.x + rect.w * 0.5,
                    py: rect.y + rect.h * 0.5,
                    button: MouseButton::Left,
                },
                &mut context,
            ),
            Some(WidgetAction::Consumed),
        );
        assert!(!search_bar.is_capturing());
    }

    #[test]
    fn focused_search_input_uses_text_box_blink_deadline() {
        let mut search_bar = layout_search_bar(
            480.0,
            SearchBarSnapshot { visible: true, ..SearchBarSnapshot::default() },
        );

        let deadline =
            search_bar.next_cursor_blink_at().expect("focused search input should blink");
        assert!(search_bar.advance_cursor_blink(deadline));
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
        let theme = test_theme();
        let mut context = EventCtx::new(&theme, 1.0);
        let px = replace_rect.x + replace_rect.w * 0.5;
        let py = replace_rect.y + replace_rect.h * 0.5;
        let _ = search_bar
            .on_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context);
        assert_eq!(
            search_bar
                .on_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut context),
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
        assert_eq!(
            search_bar
                .controls
                .index_at(close_rect.x + close_rect.w * 0.5, close_rect.y + close_rect.h * 0.5),
            Some(0)
        );
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
        assert_eq!(search_bar.on_event(&Event::PointerLeave, &mut event_context), None);
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
