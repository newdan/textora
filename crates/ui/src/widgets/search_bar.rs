//! SearchBarWidget — 搜索面板的绘制 + 键盘事件转译。
//! 显隐由 app 通过 set_visible 注入（visibility 信息源是 doc.search_state.panel_visible）。
//! query 与 match_count 由 app 通过 set_input 注入。
/// Height of the search bar in logical pixels (before DPI scaling).
pub use crate::constants::BAR_HEIGHT as SEARCH_BAR_HEIGHT;

use crate::core::widget::MouseButton;
use crate::core::{
    Event, EventCtx, KeyCode, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction, WidgetId,
};
use crate::widgets::icon::draw_icon;
use crate::widgets::tooltip::TooltipHint;
use std::any::Any;
use std::cell::Cell;

#[derive(Copy, Clone, PartialEq, Eq)]
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

pub struct SearchBarWidget {
    rect: Rect,
    pill_rect: Cell<Rect>,
    snap: SearchBarSnapshot,
    find_box: TextBox,
    replace_box: TextBox,
    close_btn_rect: Cell<Rect>,
    prev_btn_rect: Cell<Rect>,
    next_btn_rect: Cell<Rect>,
    replace_btn_rect: Cell<Rect>,
    replace_all_btn_rect: Cell<Rect>,
    toggle_replace_btn_rect: Cell<Rect>,
    regex_btn_rect: Cell<Rect>,
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
        find_box.set_placeholder("Find...");
        find_box.set_max_len_bytes(2048);

        let mut replace_box = TextBox::with_id(REPLACE_BOX_ID);
        replace_box.set_placeholder("Replace...");
        replace_box.set_max_len_bytes(2048);

        Self {
            rect: Rect::ZERO,
            pill_rect: Cell::new(Rect::ZERO),
            snap: SearchBarSnapshot::default(),
            find_box,
            replace_box,
            close_btn_rect: Cell::new(Rect::ZERO),
            prev_btn_rect: Cell::new(Rect::ZERO),
            next_btn_rect: Cell::new(Rect::ZERO),
            replace_btn_rect: Cell::new(Rect::ZERO),
            replace_all_btn_rect: Cell::new(Rect::ZERO),
            toggle_replace_btn_rect: Cell::new(Rect::ZERO),
            regex_btn_rect: Cell::new(Rect::ZERO),
            hovered_btn: HoveredButton::None,
        }
    }

    pub fn with_snap(rect: Rect, snap: SearchBarSnapshot) -> Self {
        let mut w = Self::new();
        w.rect = rect;
        w.set_input(snap);
        w
    }

    pub fn set_clipboard_callbacks(
        &mut self,
        on_copy: std::rc::Rc<dyn Fn(String)>,
        on_cut: std::rc::Rc<dyn Fn(String)>,
        on_paste: std::rc::Rc<dyn Fn() -> String>,
    ) {
        let cp1 = on_copy.clone();
        let cp2 = on_copy;
        let ct1 = on_cut.clone();
        let ct2 = on_cut;
        let pt1 = on_paste.clone();
        let pt2 = on_paste;

        self.find_box.on_copy = Some(Box::new(move |s| cp1(s)));
        self.find_box.on_cut = Some(Box::new(move |s| ct1(s)));
        self.find_box.on_paste = Some(Box::new(move || pt1()));

        self.replace_box.on_copy = Some(Box::new(move |s| cp2(s)));
        self.replace_box.on_cut = Some(Box::new(move |s| ct2(s)));
        self.replace_box.on_paste = Some(Box::new(move || pt2()));
    }

    /// Get the close button rect (for testing / tooltip integration).
    pub fn close_btn_rect(&self) -> Rect {
        self.close_btn_rect.get()
    }

    pub fn set_input(&mut self, snap: SearchBarSnapshot) {
        self.find_box.sync_text(&snap.query);
        self.replace_box.sync_text(&snap.replace_query);
        self.find_box.set_blink(snap.blink_on);
        self.replace_box.set_blink(snap.blink_on);
        self.find_box.set_focus(!snap.replace_mode || !snap.focus_replace);
        self.replace_box.set_focus(snap.replace_mode && snap.focus_replace);
        self.snap = snap;
    }

    pub fn is_visible(&self) -> bool {
        self.snap.visible
    }
}

impl Widget for SearchBarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);

        let dpi = ctx.dpi;
        let font_size = 14.0 * dpi;
        let pad_left = 36.0 * dpi;
        let pad_right = 8.0 * dpi;
        let btn_size = 20.0 * dpi;
        let btn_gap = 4.0 * dpi;
        let pill_w = self.rect.w;
        let pill_h = self.rect.h;

        let replace_btn_w = 4.0 * 8.0 * dpi;
        let nav_width = if self.snap.match_count > 0 {
            let count_text = format!(
                "{}/{}",
                self.snap.current_match.saturating_add(1).min(self.snap.match_count),
                self.snap.match_count
            );
            btn_size
                + btn_gap
                + btn_size
                + btn_gap
                + count_text.len() as f32 * 8.0 * dpi
                + btn_gap
                + btn_size
                + btn_gap
        } else if !self.snap.query.is_empty() {
            let no_w = "No results".len() as f32 * 8.0 * dpi;
            no_w + btn_gap + btn_size + btn_gap
        } else {
            btn_size + btn_gap
        };
        let right_total = pad_right
            + btn_size
            + btn_gap
            + btn_size
            + btn_gap
            + nav_width
            + replace_btn_w
            + btn_gap
            + replace_btn_w
            + btn_gap;

        let input_area_left = pad_left;
        let input_area_right = pill_w - right_total;
        let input_area_w = (input_area_right - input_area_left).max(80.0 * dpi);
        let sep_w = 20.0 * dpi;
        let input_h = font_size + 4.0 * dpi;
        let input_y = (pill_h - input_h) * 0.5;

        if self.snap.replace_mode {
            let find_w = (input_area_w - sep_w) * 0.5;
            let replace_w = (input_area_w - sep_w) * 0.5;
            let find_rect =
                Rect::new(input_area_left - 4.0 * dpi, input_y, find_w + 4.0 * dpi, input_h);
            let replace_left = input_area_left + find_w + sep_w;
            let replace_rect =
                Rect::new(replace_left - 4.0 * dpi, input_y, replace_w + 4.0 * dpi, input_h);
            self.find_box.layout(find_rect, ctx);
            self.replace_box.layout(replace_rect, ctx);
        } else {
            let find_rect = Rect::new(input_area_left - 4.0 * dpi, input_y, input_area_w, input_h);
            self.find_box.layout(find_rect, ctx);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 || !self.snap.visible {
            return;
        }
        let dpi = ctx.dpi;
        let font_size = 14.0 * dpi;
        let baseline = self.rect.h * 0.5 + font_size * 0.35;

        if self.snap.replace_mode {
            self.paint_find_replace(ctx, dpi, baseline);
        } else {
            self.paint_find_only(ctx, dpi, baseline);
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.pill_rect.get().contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.snap.visible {
            return None;
        }

        match ev {
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

        let r = self.close_btn_rect.get();
        if hit(&r) {
            return Some(TooltipHint { label: "Close".into(), target_rect: r });
        }

        let r = self.toggle_replace_btn_rect.get();
        if hit(&r) {
            let label = if self.snap.replace_mode { "Hide Replace" } else { "Show Replace" };
            return Some(TooltipHint { label: label.into(), target_rect: r });
        }

        let r = self.regex_btn_rect.get();
        if hit(&r) {
            return Some(TooltipHint { label: "Regex".into(), target_rect: r });
        }

        let r = self.prev_btn_rect.get();
        if hit(&r) {
            return Some(TooltipHint { label: "Previous Match".into(), target_rect: r });
        }

        let r = self.next_btn_rect.get();
        if hit(&r) {
            return Some(TooltipHint { label: "Next Match".into(), target_rect: r });
        }

        let r = self.replace_btn_rect.get();
        if hit(&r) {
            return Some(TooltipHint { label: "Replace".into(), target_rect: r });
        }

        let r = self.replace_all_btn_rect.get();
        if hit(&r) {
            return Some(TooltipHint { label: "Replace All".into(), target_rect: r });
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
        if check(&self.close_btn_rect.get()) {
            self.hovered_btn = HoveredButton::CloseBar;
            return;
        }
        if check(&self.toggle_replace_btn_rect.get()) {
            self.hovered_btn = HoveredButton::ToggleReplace;
            return;
        }
        if check(&self.regex_btn_rect.get()) {
            self.hovered_btn = HoveredButton::Regex;
            return;
        }
        if check(&self.prev_btn_rect.get()) {
            self.hovered_btn = HoveredButton::Prev;
            return;
        }
        if check(&self.next_btn_rect.get()) {
            self.hovered_btn = HoveredButton::Next;
            return;
        }
        if check(&self.replace_btn_rect.get()) {
            self.hovered_btn = HoveredButton::Replace;
            return;
        }
        if check(&self.replace_all_btn_rect.get()) {
            self.hovered_btn = HoveredButton::ReplaceAll;
        }
    }

    fn handle_mouse_down(&mut self, px: f32, py: f32) -> Option<WidgetAction> {
        let check = |r: &Rect| r.w > 0.0 && r.contains(px, py);
        if check(&self.close_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Close));
        }
        if check(&self.toggle_replace_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::ToggleReplace));
        }
        if check(&self.regex_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::ToggleRegex));
        }
        if check(&self.prev_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Prev));
        }
        if check(&self.next_btn_rect.get()) {
            return Some(WidgetAction::SearchBar(SearchBarAction::Next));
        }
        if self.snap.match_count > 0 {
            if check(&self.replace_btn_rect.get()) {
                return Some(WidgetAction::SearchBar(SearchBarAction::Replace));
            }
            if check(&self.replace_all_btn_rect.get()) {
                return Some(WidgetAction::SearchBar(SearchBarAction::ReplaceAll));
            }
        }
        None
    }

    /// Paint a tooltip below a button rect.

    /// Paint the find-only bar.
    fn paint_find_only(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        let icon_x = 12.0 * dpi;
        let pill_w = self.rect.w;
        let pill_h = self.rect.h;
        let pill_x = 0.0;
        let pill_rect = Rect::new(pill_x, 0.0, pill_w, pill_h);
        self.pill_rect.set(pill_rect);

        ctx.list.fill(pill_rect, ctx.theme.palette.input_bg);
        ctx.list.fill_rounded(pill_rect, ctx.theme.palette.input_border, 0.0);

        {
            let icon_sz = 14.0 * dpi;
            let icon_color = {
                let mut c = ctx.theme.palette.input_fg;
                c[3] *= 0.6;
                c
            };
            draw_icon(
                ctx.list,
                "search",
                pill_x + icon_x - icon_sz * 0.5,
                (pill_h - icon_sz) * 0.5,
                icon_sz,
                icon_color,
            );
        }

        self.find_box.paint(ctx);

        self.paint_right_buttons(ctx, dpi, baseline, pill_x, pill_w);
    }

    /// Paint the find+replace bar (inline single-row layout).
    /// Layout: [/] [find] [→] [replace] [替换] [全部] [count] [◀][▶] [▲] [✕]
    fn paint_find_replace(&self, ctx: &mut PaintCtx, dpi: f32, baseline: f32) {
        let font_size = 14.0 * dpi;
        let pad_left = 36.0 * dpi;
        let icon_x = 12.0 * dpi;
        let pad_right = 8.0 * dpi;
        let btn_size = 20.0 * dpi;
        let btn_gap = 4.0 * dpi;
        let pill_w = self.rect.w;
        let pill_h = self.rect.h;
        let pill_x = 0.0;
        let pill_rect = Rect::new(pill_x, 0.0, pill_w, pill_h);
        self.pill_rect.set(pill_rect);

        ctx.list.fill(pill_rect, ctx.theme.palette.input_bg);
        ctx.list.fill_rounded(pill_rect, ctx.theme.palette.input_border, 0.0);

        // Compute right-side buttons width to determine input area
        let replace_btn_w = 4.0 * 8.0 * dpi; // "替换" / "全部" width
        let nav_width = if self.snap.match_count > 0 {
            let count_text = format!(
                "{}/{}",
                self.snap.current_match.saturating_add(1).min(self.snap.match_count),
                self.snap.match_count
            );
            btn_size
                + btn_gap
                + btn_size
                + btn_gap
                + count_text.len() as f32 * 8.0 * dpi
                + btn_gap
                + btn_size
                + btn_gap
        } else if !self.snap.query.is_empty() {
            let no_w = "No results".len() as f32 * 8.0 * dpi;
            no_w + btn_gap + btn_size + btn_gap
        } else {
            btn_size + btn_gap
        };
        let right_total = pad_right
            + btn_size + btn_gap          // close
            + btn_size + btn_gap          // toggle
            + nav_width                   // nav + count
            + replace_btn_w + btn_gap     // 替换
            + replace_btn_w + btn_gap; // 全部

        let input_area_left = pill_x + pad_left;
        let input_area_right = pill_x + pill_w - right_total;
        let input_area_w = (input_area_right - input_area_left).max(80.0 * dpi);
        let sep_w = 20.0 * dpi; // "→" + gap
        let find_w = (input_area_w - sep_w) * 0.5;

        // Search icon
        {
            let icon_sz = 14.0 * dpi;
            let icon_color = {
                let mut c = ctx.theme.palette.input_fg;
                c[3] *= 0.6;
                c
            };
            draw_icon(
                ctx.list,
                "search",
                pill_x + icon_x - icon_sz * 0.5,
                (pill_h - icon_sz) * 0.5,
                icon_sz,
                icon_color,
            );
        }

        // "→" separator
        let sep_x = input_area_left + find_w + 4.0 * dpi;
        if let Some(ref mut shaper) = ctx.shaper {
            ctx.list.text_shaped(
                sep_x,
                baseline,
                font_size,
                {
                    let mut c = ctx.theme.palette.input_fg;
                    c[3] *= 0.6;
                    c
                },
                "\u{2192}",
                shaper,
            );
        };

        self.find_box.paint(ctx);
        self.replace_box.paint(ctx);

        // Right-side buttons: replace actions + nav + common
        self.paint_right_buttons_inline(ctx, dpi, baseline, pill_x, pill_w);
    }

    /// Paint right-side buttons for inline replace mode.
    /// Order from right: close, toggle, nav+count, replaceAll, replace
    fn paint_right_buttons_inline(
        &self,
        ctx: &mut PaintCtx,
        dpi: f32,
        baseline: f32,
        pill_x: f32,
        pill_w: f32,
    ) {
        let font_size = 14.0 * dpi;
        let btn_size = 20.0 * dpi;
        let pad_right = 8.0 * dpi;
        let btn_gap = 4.0 * dpi;
        let btn_color = {
            let mut c = ctx.theme.palette.input_fg;
            c[3] *= 0.6;
            c
        };
        let btn_color_hovered = {
            let mut c = ctx.theme.palette.input_fg;
            c[3] *= 0.9;
            c
        };
        let btn_clr = |hovered: bool| if hovered { btn_color_hovered } else { btn_color };

        let mut right_x = pill_x + pill_w - pad_right;

        // Close
        {
            let cx = right_x - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.close_btn_rect.set(cr);
            let h = self.hovered_btn == HoveredButton::CloseBar;
            if h {
                ctx.list.fill_rounded(cr, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    cx - 4.0 * dpi,
                    baseline,
                    font_size,
                    btn_clr(h),
                    "\u{2715}",
                    shaper,
                );
            };
            right_x -= btn_size + btn_gap;
        }

        // Toggle replace (▲ in replace mode)
        {
            let cx = right_x - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.toggle_replace_btn_rect.set(cr);
            let h = self.hovered_btn == HoveredButton::ToggleReplace;
            if h {
                ctx.list.fill_rounded(cr, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    cx - 4.0 * dpi,
                    baseline,
                    font_size,
                    btn_clr(h),
                    "\u{25b2}",
                    shaper,
                );
            };
            right_x -= btn_size + btn_gap;
        }

        // Regex toggle (.*)
        {
            let cx = right_x - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.regex_btn_rect.set(cr);
            let h = self.hovered_btn == HoveredButton::Regex;
            if h {
                ctx.list.fill_rounded(cr, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            let clr =
                if self.snap.options_use_regex { ctx.theme.palette.accent } else { btn_clr(h) };
            {
                let icon_sz = 14.0 * dpi;
                draw_icon(ctx.list, "regex", cx - icon_sz * 0.5, cy - icon_sz * 0.5, icon_sz, clr);
            }
            right_x -= btn_size + btn_gap;
        }

        // Nav + count
        if !self.snap.query.is_empty() && self.snap.match_count > 0 {
            let current = self.snap.current_match.saturating_add(1).min(self.snap.match_count);
            let count_text = format!("{}/{}", current, self.snap.match_count);
            let count_w = count_text.len() as f32 * 8.0 * dpi;

            // ▶
            {
                let cx = right_x - btn_size * 0.5;
                let cy = self.rect.h * 0.5;
                self.next_btn_rect.set(Rect::new(
                    cx - btn_size * 0.5,
                    cy - btn_size * 0.5,
                    btn_size,
                    btn_size,
                ));
                let h = self.hovered_btn == HoveredButton::Next;
                if h {
                    ctx.list.fill_rounded(
                        self.next_btn_rect.get(),
                        ctx.theme.palette.bg_hover,
                        4.0 * dpi,
                    );
                }
                if let Some(ref mut shaper) = ctx.shaper {
                    ctx.list.text_shaped(
                        cx - 4.0 * dpi,
                        baseline,
                        font_size,
                        btn_clr(h),
                        "\u{25b6}",
                        shaper,
                    );
                };
                right_x -= btn_size + btn_gap;
            }

            // ◀
            {
                let cx = right_x - btn_size * 0.5;
                let cy = self.rect.h * 0.5;
                self.prev_btn_rect.set(Rect::new(
                    cx - btn_size * 0.5,
                    cy - btn_size * 0.5,
                    btn_size,
                    btn_size,
                ));
                let h = self.hovered_btn == HoveredButton::Prev;
                if h {
                    ctx.list.fill_rounded(
                        self.prev_btn_rect.get(),
                        ctx.theme.palette.bg_hover,
                        4.0 * dpi,
                    );
                }
                if let Some(ref mut shaper) = ctx.shaper {
                    ctx.list.text_shaped(
                        cx - 4.0 * dpi,
                        baseline,
                        font_size,
                        btn_clr(h),
                        "\u{25c0}",
                        shaper,
                    );
                };
                right_x -= btn_size + btn_gap;
            }

            // Count text (left of nav per spec: [count] [◀][▶])
            right_x -= count_w + btn_gap;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    right_x,
                    baseline,
                    font_size,
                    ctx.theme.palette.input_fg,
                    &count_text,
                    shaper,
                );
            };
        } else if !self.snap.query.is_empty() {
            let no_res = "No results";
            let no_w = no_res.len() as f32 * 8.0 * dpi;
            right_x -= no_w + btn_gap;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    right_x,
                    baseline,
                    font_size,
                    ctx.theme.palette.danger,
                    no_res,
                    shaper,
                );
            };
            self.prev_btn_rect.set(Rect::ZERO);
            self.next_btn_rect.set(Rect::ZERO);
        } else {
            self.prev_btn_rect.set(Rect::ZERO);
            self.next_btn_rect.set(Rect::ZERO);
        }

        // "全部" (ReplaceAll)
        {
            let all_text = "\u{5168}\u{90e8}";
            let all_w = 4.0 * 8.0 * dpi;
            right_x -= all_w + btn_gap;
            let btn_rect =
                Rect::new(right_x, baseline - font_size * 0.75, all_w, font_size + 4.0 * dpi);
            self.replace_all_btn_rect.set(btn_rect);
            let h = self.hovered_btn == HoveredButton::ReplaceAll;
            if h {
                ctx.list.fill_rounded(btn_rect, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    right_x + 4.0 * dpi,
                    baseline,
                    font_size,
                    btn_clr(h),
                    all_text,
                    shaper,
                );
            };
        }

        // "替换" (Replace)
        {
            let repl_text = "\u{66ff}\u{6362}";
            let repl_w = 4.0 * 8.0 * dpi;
            right_x -= repl_w + btn_gap;
            let btn_rect =
                Rect::new(right_x, baseline - font_size * 0.75, repl_w, font_size + 4.0 * dpi);
            self.replace_btn_rect.set(btn_rect);
            let h = self.hovered_btn == HoveredButton::Replace;
            if h {
                ctx.list.fill_rounded(btn_rect, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    right_x + 4.0 * dpi,
                    baseline,
                    font_size,
                    btn_clr(h),
                    repl_text,
                    shaper,
                );
            };
        }
    }

    fn paint_right_buttons(
        &self,
        ctx: &mut PaintCtx,
        dpi: f32,
        baseline: f32,
        pill_x: f32,
        pill_w: f32,
    ) {
        let font_size = 14.0 * dpi;
        let btn_size = 20.0 * dpi;
        let pad_right = 8.0 * dpi;
        let btn_gap = 4.0 * dpi;
        let btn_color = {
            let mut c = ctx.theme.palette.input_fg;
            c[3] *= 0.6;
            c
        };
        let btn_color_hovered = {
            let mut c = ctx.theme.palette.input_fg;
            c[3] *= 0.9;
            c
        };
        let btn_clr = |hovered: bool| if hovered { btn_color_hovered } else { btn_color };

        let mut right_x = pill_x + pill_w - pad_right;

        // Close
        {
            let cx = right_x - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.close_btn_rect.set(cr);
            let h = self.hovered_btn == HoveredButton::CloseBar;
            if h {
                ctx.list.fill_rounded(cr, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    cx - 4.0 * dpi,
                    baseline,
                    font_size,
                    btn_clr(h),
                    "\u{2715}",
                    shaper,
                );
            };
            right_x -= btn_size + btn_gap;
        }

        // Toggle replace
        {
            let cx = right_x - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.toggle_replace_btn_rect.set(cr);
            let h = self.hovered_btn == HoveredButton::ToggleReplace;
            if h {
                ctx.list.fill_rounded(cr, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            let arrow = if self.snap.replace_mode { "\u{25b2}" } else { "\u{25bc}" };
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    cx - 4.0 * dpi,
                    baseline,
                    font_size,
                    btn_clr(h),
                    arrow,
                    shaper,
                );
            };
            right_x -= btn_size + btn_gap;
        }

        // Regex
        {
            let cx = right_x - btn_size * 0.5;
            let cy = self.rect.h * 0.5;
            let cr = Rect::new(cx - btn_size * 0.5, cy - btn_size * 0.5, btn_size, btn_size);
            self.regex_btn_rect.set(cr);
            let h = self.hovered_btn == HoveredButton::Regex;
            if h {
                ctx.list.fill_rounded(cr, ctx.theme.palette.bg_hover, 4.0 * dpi);
            }
            let clr =
                if self.snap.options_use_regex { ctx.theme.palette.accent } else { btn_clr(h) };
            {
                let icon_sz = 14.0 * dpi;
                draw_icon(ctx.list, "regex", cx - icon_sz * 0.5, cy - icon_sz * 0.5, icon_sz, clr);
            }
            right_x -= btn_size + btn_gap;
        }

        // Nav
        if !self.snap.query.is_empty() {
            if self.snap.match_count > 0 {
                let current = self.snap.current_match.saturating_add(1).min(self.snap.match_count);
                let count_text = format!("{}/{}", current, self.snap.match_count);
                let count_w = count_text.len() as f32 * 8.0 * dpi;

                {
                    let cx = right_x - btn_size * 0.5;
                    let cy = self.rect.h * 0.5;
                    self.next_btn_rect.set(Rect::new(
                        cx - btn_size * 0.5,
                        cy - btn_size * 0.5,
                        btn_size,
                        btn_size,
                    ));
                    let h = self.hovered_btn == HoveredButton::Next;
                    if h {
                        ctx.list.fill_rounded(
                            self.next_btn_rect.get(),
                            ctx.theme.palette.bg_hover,
                            4.0 * dpi,
                        );
                    }
                    if let Some(ref mut shaper) = ctx.shaper {
                        ctx.list.text_shaped(
                            cx - 4.0 * dpi,
                            baseline,
                            font_size,
                            btn_clr(h),
                            "\u{25b6}",
                            shaper,
                        );
                    };
                    right_x -= btn_size + btn_gap;
                }

                {
                    let cx = right_x - btn_size * 0.5;
                    let cy = self.rect.h * 0.5;
                    self.prev_btn_rect.set(Rect::new(
                        cx - btn_size * 0.5,
                        cy - btn_size * 0.5,
                        btn_size,
                        btn_size,
                    ));
                    let h = self.hovered_btn == HoveredButton::Prev;
                    if h {
                        ctx.list.fill_rounded(
                            self.prev_btn_rect.get(),
                            ctx.theme.palette.bg_hover,
                            4.0 * dpi,
                        );
                    }
                    if let Some(ref mut shaper) = ctx.shaper {
                        ctx.list.text_shaped(
                            cx - 4.0 * dpi,
                            baseline,
                            font_size,
                            btn_clr(h),
                            "\u{25c0}",
                            shaper,
                        );
                    };
                    right_x -= btn_size + btn_gap;
                }

                right_x -= count_w + btn_gap;
                if let Some(ref mut shaper) = ctx.shaper {
                    ctx.list.text_shaped(
                        right_x,
                        baseline,
                        font_size,
                        ctx.theme.palette.input_fg,
                        &count_text,
                        shaper,
                    );
                };
            } else {
                let no_res = "No results";
                let no_w = no_res.len() as f32 * 8.0 * dpi;
                right_x -= no_w + btn_gap;
                if let Some(ref mut shaper) = ctx.shaper {
                    ctx.list.text_shaped(
                        right_x,
                        baseline,
                        font_size,
                        ctx.theme.palette.danger,
                        no_res,
                        shaper,
                    );
                };
                self.prev_btn_rect.set(Rect::ZERO);
                self.next_btn_rect.set(Rect::ZERO);
            }
        } else {
            self.prev_btn_rect.set(Rect::ZERO);
            self.next_btn_rect.set(Rect::ZERO);
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
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::DrawList;
    use crate::theme::test_theme;

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

    #[test]
    fn basic_creation() {
        let w = SearchBarWidget::new();
        assert_eq!(w.find_box.text(), "");
    }

    #[test]
    fn tooltip_at_close_button() {
        let w = setup_search_bar("test");
        let r = w.close_btn_rect.get();
        assert!(r.w > 0.0, "close button should be laid out");
        let hint = w.tooltip_at(r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert!(hint.is_some(), "hovering close button should return tooltip");
        assert_eq!(hint.unwrap().label, "Close");
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
        let r = w.toggle_replace_btn_rect.get();
        if r.w > 0.0 {
            let hint = w.tooltip_at(r.x + r.w / 2.0, r.y + r.h / 2.0);
            assert!(hint.is_some(), "hovering toggle replace should return tooltip");
            assert_eq!(hint.unwrap().label, "Show Replace");
        }
    }

    #[test]
    fn cmd_a_selects_find_box_text() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;

        let mut w = setup_search_bar("hello");
        let theme = test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
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
        let mut event_context = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
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
    fn cmd_c_copies_find_box_selection() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;
        use std::cell::Cell;
        use std::rc::Rc;

        let mut w = setup_search_bar("hello");
        w.find_box.select_all();

        let copied = Rc::new(Cell::new(String::new()));
        let c = copied.clone();
        w.set_clipboard_callbacks(
            Rc::new(move |s| c.set(s)),
            Rc::new(|_| {}),
            Rc::new(String::new),
        );

        let theme = test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

        let action = w.on_event(&Event::KeyDown(KeyCode::Char('c'), cmd), &mut ctx);
        assert_eq!(action, Some(WidgetAction::Consumed));
        assert_eq!(copied.take(), "hello");
    }

    #[test]
    fn cmd_v_pastes_into_focused_find_box() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;
        use std::rc::Rc;

        let mut w = setup_search_bar("");
        w.set_clipboard_callbacks(
            Rc::new(|_| {}),
            Rc::new(|_| {}),
            Rc::new(|| "pasted".to_string()),
        );

        let theme = test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
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
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        w.on_event(&Event::KeyDown(KeyCode::Delete, Modifiers::NONE), &mut ctx);
        assert_eq!(w.find_box.text(), "");
        assert!(w.find_box.is_focused());
    }

    #[test]
    fn replace_box_receives_clipboard_shortcuts_when_focused() {
        use crate::core::EventCtx;
        use crate::core::widget::{Event, KeyCode, Modifiers};
        use crate::theme::test_theme;
        use std::cell::Cell;
        use std::rc::Rc;

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

        let copied = Rc::new(Cell::new(String::new()));
        let c = copied.clone();
        w.set_clipboard_callbacks(
            Rc::new(move |s| c.set(s)),
            Rc::new(|_| {}),
            Rc::new(String::new),
        );

        let theme = test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

        w.on_event(&Event::KeyDown(KeyCode::Char('c'), cmd), &mut ctx);
        assert_eq!(copied.take(), "repl");
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
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

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
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

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
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

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
