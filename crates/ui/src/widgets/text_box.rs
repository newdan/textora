//! TextBox — single-line text input component.
//! Manages text state, cursor, selection, IME preedit, and clipboard shortcuts.

use std::time::{Duration, Instant};

use crate::core::widget::{
    ControlAction, PointerClickTracker, SensitiveText, TextPayload, WidgetId,
};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, LayoutCtx, MouseButton, PaintCtx, Rect,
    Widget, WidgetAction,
};
use unicode_segmentation::UnicodeSegmentation;

mod ime_and_render;
mod input;

const MASKED_ECHO_GLYPH: char = '•';
const DEFAULT_FONT_SIZE_LOGICAL: f32 = 14.0;
const MINIMUM_FONT_SIZE_LOGICAL: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextBoxChrome {
    #[default]
    Framed,
    Seamless,
}

/// IME event type — received by TextBox from the parent widget.
#[derive(Clone)]
pub enum TextBoxIme {
    Preedit { text: String, cursor: Option<(usize, usize)> },
    Commit(String),
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MouseDragSelection {
    Grapheme { anchor: usize },
    Word { start: usize, end: usize },
    EntireLine,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EchoMode {
    #[default]
    Plain,
    Masked,
}

#[derive(Clone, PartialEq)]
enum TextStorage {
    Plain(String),
    Sensitive(SensitiveText),
}

impl TextStorage {
    fn as_str(&self) -> &str {
        match self {
            Self::Plain(value) => value,
            Self::Sensitive(value) => value.expose(),
        }
    }

    fn set_echo_mode(&mut self, echo_mode: EchoMode) {
        let already_matches = matches!(
            (&*self, echo_mode),
            (Self::Plain(_), EchoMode::Plain) | (Self::Sensitive(_), EchoMode::Masked)
        );
        if already_matches {
            return;
        }

        let previous = std::mem::replace(self, Self::Plain(String::new()));
        *self = match previous {
            Self::Plain(value) => Self::Sensitive(SensitiveText::new(value)),
            Self::Sensitive(value) => Self::Plain(value.expose().to_owned()),
        };
    }

    fn replace(&mut self, value: &str) {
        match self {
            Self::Plain(current) => *current = value.to_owned(),
            Self::Sensitive(current) => *current = SensitiveText::new(value.to_owned()),
        }
    }

    fn clear(&mut self) {
        match self {
            Self::Plain(value) => value.clear(),
            Self::Sensitive(value) => value.clear(),
        }
    }

    fn replace_range(&mut self, range: std::ops::Range<usize>, replacement: &str) {
        match self {
            Self::Plain(value) => value.replace_range(range, replacement),
            Self::Sensitive(value) => value.replace_range(range, replacement),
        }
    }

    fn insert_str(&mut self, byte_index: usize, value: &str) {
        self.replace_range(byte_index..byte_index, value);
    }

    fn insert(&mut self, byte_index: usize, value: char) {
        let mut encoded = [0; 4];
        self.insert_str(byte_index, value.encode_utf8(&mut encoded));
    }

    fn payload(&self) -> TextPayload {
        match self {
            Self::Plain(value) => TextPayload::Plain(value.clone()),
            Self::Sensitive(value) => TextPayload::Sensitive(value.clone()),
        }
    }

    fn into_single_line(self) -> Self {
        if !self.as_str().contains(['\r', '\n']) {
            return self;
        }

        match self {
            Self::Plain(value) => {
                let mut normalized = String::with_capacity(value.len());
                TextBox::push_single_line_text(&value, &mut normalized);
                Self::Plain(normalized)
            }
            Self::Sensitive(value) => Self::Sensitive(value.rewrite(|source, normalized| {
                TextBox::push_single_line_text(source, normalized);
            })),
        }
    }
}

impl std::fmt::Debug for TextStorage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plain(value) => formatter.debug_tuple("Plain").field(value).finish(),
            Self::Sensitive(_) => formatter.write_str("Sensitive(<redacted>)"),
        }
    }
}

impl std::ops::Deref for TextStorage {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl PartialEq<&str> for TextStorage {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

pub struct TextBox {
    id: Option<WidgetId>,
    rect: Rect,

    // Text state
    text: TextStorage,
    cursor_byte: usize,

    // Selection: (anchor_byte, cursor_byte), no ordering guarantee. None = no selection.
    selection: Option<(usize, usize)>,

    // IME
    preedit: TextStorage,
    preedit_cursor: Option<(usize, usize)>,

    // Visual
    placeholder: String,
    accessibility_label: Option<String>,
    echo_mode: EchoMode,
    blink_on: bool,
    next_cursor_blink: Option<Instant>,
    focused: bool,
    chrome: TextBoxChrome,
    font_size_logical: f32,

    // Mouse drag
    dragging: bool,
    mouse_drag_selection: Option<MouseDragSelection>,
    pointer_click_tracker: PointerClickTracker,

    // Layout cache
    cursor_x: f32,
    preedit_width: f32,
    preedit_cursor_x: f32,
    /// grapheme 边界的 byte_offset → pixel_x 映射（layout 时填充），用于鼠标点击定位光标。
    grapheme_offsets: Vec<(usize, f32)>,
    /// 文本区域左侧 padding（layout 时按 DPI 缓存）
    text_pad: f32,
    leading_content_inset_logical: f32,
    fixed_size_logical: Option<(f32, f32)>,

    pub max_len_bytes: usize,
    committed_payload: Option<TextPayload>,
}

const TEXT_CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);

impl Default for TextBox {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBox {
    pub fn new() -> Self {
        Self {
            id: None,
            rect: Rect::ZERO,
            text: TextStorage::Plain(String::new()),
            cursor_byte: 0,
            selection: None,
            preedit: TextStorage::Plain(String::new()),
            preedit_cursor: None,
            placeholder: String::new(),
            accessibility_label: None,
            echo_mode: EchoMode::Plain,
            blink_on: true,
            next_cursor_blink: Some(Instant::now() + TEXT_CURSOR_BLINK_INTERVAL),
            focused: false,
            chrome: TextBoxChrome::Framed,
            font_size_logical: DEFAULT_FONT_SIZE_LOGICAL,
            dragging: false,
            mouse_drag_selection: None,
            pointer_click_tracker: PointerClickTracker::default(),
            cursor_x: 0.0,
            preedit_width: 0.0,
            preedit_cursor_x: 0.0,
            grapheme_offsets: Vec::new(),
            text_pad: 0.0,
            leading_content_inset_logical: 4.0,
            fixed_size_logical: None,
            max_len_bytes: usize::MAX,
            committed_payload: None,
        }
    }

    pub fn with_id(id: WidgetId) -> Self {
        let mut text_box = Self::new();
        text_box.id = Some(id);
        text_box
    }

    // ── Accessors ──

    pub fn text(&self) -> &str {
        self.text.as_str()
    }
    pub fn cursor_byte(&self) -> usize {
        self.cursor_byte
    }
    pub fn is_focused(&self) -> bool {
        self.focused
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_text(&mut self, text: &str) {
        self.text.replace(text);
        self.cursor_byte = self.text.len();
        self.selection = None;
    }

    pub(crate) fn preserve_editing_state_from(&mut self, previous: &Self) {
        if self.id != previous.id
            || self.echo_mode != previous.echo_mode
            || self.text.as_str() != previous.text.as_str()
        {
            return;
        }

        self.cursor_byte = previous.cursor_byte;
        self.selection = previous.selection;
        self.preedit = previous.preedit.clone();
        self.preedit_cursor = previous.preedit_cursor;
        self.focused = previous.focused;
        self.blink_on = previous.blink_on;
        if previous.next_cursor_blink.is_some() {
            self.next_cursor_blink = previous.next_cursor_blink;
        }
    }

    pub fn set_placeholder(&mut self, ph: &str) {
        self.placeholder = ph.to_string();
    }

    pub fn set_accessibility_label(&mut self, label: Option<String>) {
        self.accessibility_label = label;
    }

    pub fn set_chrome(&mut self, chrome: TextBoxChrome) {
        self.chrome = chrome;
    }

    pub fn set_font_size_logical(&mut self, font_size_logical: f32) {
        self.font_size_logical = font_size_logical.max(MINIMUM_FONT_SIZE_LOGICAL);
    }

    pub fn set_fixed_size_logical(&mut self, width: f32, height: f32) {
        self.fixed_size_logical = Some((width.max(0.0), height.max(0.0)));
    }

    /// 设置输入内容相对控件左边缘的逻辑像素距离，可为前置图标预留空间。
    pub fn set_leading_content_inset_logical(&mut self, inset: f32) {
        self.leading_content_inset_logical = inset.max(0.0);
    }

    pub fn set_echo_mode(&mut self, echo_mode: EchoMode) {
        self.text.set_echo_mode(echo_mode);
        self.preedit.set_echo_mode(echo_mode);
        self.echo_mode = echo_mode;
    }

    pub fn take_committed_payload(&mut self) -> Option<TextPayload> {
        self.committed_payload.take()
    }

    /// Compatibility helper for password fields that do not need committed payloads.
    pub fn set_password_mode(&mut self, enabled: bool) {
        self.set_echo_mode(if enabled { EchoMode::Masked } else { EchoMode::Plain });
    }

    pub fn set_max_len_bytes(&mut self, max: usize) {
        self.max_len_bytes = max;
    }

    /// Legacy externally controlled visibility. New inputs use component-owned blinking.
    pub fn set_blink(&mut self, on: bool) {
        self.next_cursor_blink = None;
        self.blink_on = on;
    }

    pub fn next_cursor_blink_at(&self) -> Option<Instant> {
        (self.focused && self.selection.is_none()).then_some(self.next_cursor_blink).flatten()
    }

    pub fn advance_cursor_blink(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.next_cursor_blink_at() else {
            return false;
        };
        if now < deadline {
            return false;
        }
        self.blink_on = !self.blink_on;
        self.next_cursor_blink = Some(now + TEXT_CURSOR_BLINK_INTERVAL);
        true
    }

    fn reset_cursor_blink(&mut self) {
        if self.next_cursor_blink.is_some() {
            self.blink_on = true;
            self.next_cursor_blink = Some(Instant::now() + TEXT_CURSOR_BLINK_INTERVAL);
        }
    }

    pub fn set_focus(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            self.reset_cursor_blink();
            if !focused {
                self.selection = None;
                self.dragging = false;
                self.mouse_drag_selection = None;
                self.pointer_click_tracker.reset();
                self.preedit.clear();
                self.preedit_cursor = None;
            }
        }
    }

    pub fn select_all(&mut self) {
        if !self.text.is_empty() {
            self.selection = Some((0, self.text.len()));
            self.cursor_byte = self.text.len();
        }
    }

    pub fn selection_text(&self) -> Option<&str> {
        if self.echo_mode == EchoMode::Masked {
            return None;
        }
        self.selection.map(|(a, b)| {
            let start = a.min(b);
            let end = a.max(b);
            &self.text.as_str()[start..end]
        })
    }

    // ── Helpers ──

    fn clamp_to_grapheme_boundary(s: &str, pos: usize) -> usize {
        if pos >= s.len() {
            return s.len();
        }
        s.grapheme_indices(true)
            .map(|(byte, _)| byte)
            .take_while(|&byte| byte <= pos)
            .last()
            .unwrap_or(0)
    }

    fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection.take() {
            if a == b {
                return false;
            }
            let start = a.min(b);
            let end = a.max(b);
            self.text.replace_range(start..end, "");
            self.cursor_byte = start;
            true
        } else {
            false
        }
    }

    fn edited_action(&self) -> Option<ControlAction> {
        let id = self.id?;
        Some(ControlAction::TextEdited { id, value: self.text.payload() })
    }

    fn edited_action_if_text_changed(&self, previous_text: &TextStorage) -> Option<ControlAction> {
        if &self.text == previous_text { None } else { self.edited_action() }
    }

    fn push_single_line_text(text: &str, normalized_text: &mut String) {
        let mut characters = text.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '\r' => {
                    if characters.peek() == Some(&'\n') {
                        characters.next();
                    }
                    normalized_text.push(' ');
                }
                '\n' => normalized_text.push(' '),
                _ => normalized_text.push(character),
            }
        }
    }

    pub(super) fn cancel_transient_interaction(&mut self) -> bool {
        let interaction_changed = std::mem::take(&mut self.dragging)
            | !self.preedit.is_empty()
            | self.preedit_cursor.take().is_some();
        self.preedit.clear();
        self.mouse_drag_selection = None;
        self.pointer_click_tracker.reset();
        interaction_changed
    }

    /// Sync text from an external source (e.g., app-layer snapshot).
    /// Only overwrites when the external text differs from internal text.
    /// This prevents snapshot-based state injection from overwriting
    /// live user input that hasn't been flushed yet.
    pub fn sync_text(&mut self, ext_text: &str) {
        if self.text != ext_text {
            self.text.replace(ext_text);
            self.cursor_byte =
                Self::clamp_to_grapheme_boundary(self.text.as_str(), self.cursor_byte);
            self.selection = None;
        }
    }
}

impl Widget for TextBox {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        let layout_rect =
            self.fixed_size_logical.map_or(rect, |(width_logical, height_logical)| {
                let width = (width_logical * ctx.dpi).min(rect.w);
                let height = (height_logical * ctx.dpi).min(rect.h);
                Rect::new(rect.x, rect.y + (rect.h - height) * 0.5, width, height)
            });
        self.layout(layout_rect, ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        TextBox::paint(self, ctx);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        self.id
    }

    fn is_focusable(&self) -> bool {
        self.id.is_some()
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        if let Some(id) = self.id {
            self.set_focus(focused_id == Some(id));
        }
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let id = self.id?;
        let sensitive = self.echo_mode == EchoMode::Masked;
        let mut node = AccessibilityNode::new(
            AccessibilityId::from(id),
            AccessibilityRole::TextField,
            ctx.screen_bounds(self.rect),
        )
        .with_focused(self.focused)
        .with_sensitive(sensitive)
        .with_action(AccessibilityAction::Focus)
        .with_action(AccessibilityAction::SetValue);
        if let Some(name) = self
            .accessibility_label
            .as_ref()
            .or_else(|| (!self.placeholder.is_empty()).then_some(&self.placeholder))
        {
            node = node.with_name(name.clone());
        }
        if !sensitive {
            node = node.with_value(self.text.as_str());
        }
        Some(node)
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        let id = self.id?;
        if request.target != AccessibilityId::from(id) {
            return None;
        }
        match request.action {
            AccessibilityAction::Focus => {
                Some(WidgetAction::Control(ControlAction::FocusRequested { id }))
            }
            AccessibilityAction::SetValue => {
                let requested_value = match request.value.as_ref()? {
                    TextPayload::Plain(value) => value.as_str(),
                    TextPayload::Sensitive(value) => value.expose(),
                };
                let accepted_end = if requested_value.len() <= self.max_len_bytes {
                    requested_value.len()
                } else {
                    Self::clamp_to_grapheme_boundary(requested_value, self.max_len_bytes)
                };
                let accepted_value = &requested_value[..accepted_end];
                if self.text.as_str() == accepted_value {
                    return Some(WidgetAction::Consumed);
                }
                self.set_text(accepted_value);
                Some(WidgetAction::Control(ControlAction::TextEdited {
                    id,
                    value: self.text.payload(),
                }))
            }
            _ => None,
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.focused
            && matches!(
                ev,
                Event::KeyDown(..)
                    | Event::ImePreedit { .. }
                    | Event::ImeCommit(..)
                    | Event::ImeEnable
                    | Event::ImeDisable
            )
        {
            return None;
        }
        if matches!(ev, Event::KeyDown(..) | Event::MouseDown { .. } | Event::ImeCommit(..)) {
            self.reset_cursor_blink();
        }
        match ev {
            Event::KeyDown(key_code, modifiers) => {
                let (consumed, action) = self.handle_key_down(*key_code, *modifiers, ctx);
                action
                    .map(WidgetAction::Control)
                    .or_else(|| consumed.then_some(WidgetAction::Consumed))
            }
            Event::ImePreedit { text, cursor } => {
                self.handle_ime_preedit(text, *cursor);
                Some(WidgetAction::Consumed)
            }
            Event::ImeCommit(text) => self
                .handle_ime_commit(text)
                .map(WidgetAction::Control)
                .or(Some(WidgetAction::Consumed)),
            Event::ImeEnable => {
                self.handle_ime_event(&TextBoxIme::Enabled);
                Some(WidgetAction::Consumed)
            }
            Event::ImeDisable => {
                self.handle_ime_event(&TextBoxIme::Disabled);
                Some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                let (consumed, action) = self.handle_mouse_down(*px, *py);
                action
                    .map(WidgetAction::Control)
                    .or_else(|| consumed.then_some(WidgetAction::Consumed))
            }
            Event::MouseUp { button: MouseButton::Left, .. } => {
                if self.dragging {
                    self.on_mouse_up();
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            Event::MouseMove { px, py } => {
                if self.dragging {
                    self.on_mouse_drag(*px, *py);
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            Event::InteractionCancel => {
                self.cancel_transient_interaction().then_some(WidgetAction::Consumed)
            }
            _ => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.dragging
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests;
