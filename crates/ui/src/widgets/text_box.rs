//! TextBox — single-line text input component.
//! Manages text state, cursor, selection, IME preedit, and clipboard shortcuts.

use crate::core::widget::{ControlAction, SensitiveText, TextPayload, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, Event, EventCtx, KeyCode, LayoutCtx, Modifiers,
    MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use unicode_segmentation::UnicodeSegmentation;

const MASKED_ECHO_GLYPH: char = '•';
const DEFAULT_FONT_SIZE_LOGICAL: f32 = 14.0;
const MINIMUM_FONT_SIZE_LOGICAL: f32 = 1.0;
const FRAMED_CORNER_RADIUS_LOGICAL: f32 = 3.0;
const SEAMLESS_FOCUS_CORNER_RADIUS_LOGICAL: f32 = 6.0;

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

impl std::fmt::Debug for TextBoxIme {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preedit { cursor, .. } => formatter
                .debug_struct("Preedit")
                .field("text", &"<redacted>")
                .field("cursor", cursor)
                .finish(),
            Self::Commit(_) => formatter.write_str("Commit(<redacted>)"),
            Self::Enabled => formatter.write_str("Enabled"),
            Self::Disabled => formatter.write_str("Disabled"),
        }
    }
}

impl zeroize::Zeroize for TextBoxIme {
    fn zeroize(&mut self) {
        match self {
            Self::Preedit { text, .. } | Self::Commit(text) => {
                zeroize::Zeroize::zeroize(text);
            }
            Self::Enabled | Self::Disabled => {}
        }
    }
}

impl zeroize::ZeroizeOnDrop for TextBoxIme {}

impl Drop for TextBoxIme {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
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
    focused: bool,
    chrome: TextBoxChrome,
    font_size_logical: f32,

    // Mouse drag
    dragging: bool,

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
            blink_on: false,
            focused: false,
            chrome: TextBoxChrome::Framed,
            font_size_logical: DEFAULT_FONT_SIZE_LOGICAL,
            dragging: false,
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

    pub fn set_blink(&mut self, on: bool) {
        self.blink_on = on;
    }

    pub fn set_focus(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            if !focused {
                self.selection = None;
                self.dragging = false;
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

    fn prev_grapheme_boundary(s: &str, pos: usize) -> usize {
        let current = Self::clamp_to_grapheme_boundary(s, pos);
        if current < pos {
            return current;
        }
        s[..current].grapheme_indices(true).next_back().map(|(byte, _)| byte).unwrap_or(0)
    }

    fn next_grapheme_boundary(s: &str, pos: usize) -> usize {
        if pos >= s.len() {
            return s.len();
        }
        s.grapheme_indices(true).map(|(byte, _)| byte).find(|&byte| byte > pos).unwrap_or(s.len())
    }

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

    /// Delete selected range and return true if something was deleted.
    fn prev_word_boundary(s: &str, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let mut i = pos;
        while i > 0 {
            let prev = Self::prev_grapheme_boundary(s, i);
            if s[prev..i].chars().next().unwrap().is_alphanumeric() {
                break;
            }
            i = prev;
        }
        while i > 0 {
            let prev = Self::prev_grapheme_boundary(s, i);
            if !s[prev..i].chars().next().unwrap().is_alphanumeric() {
                break;
            }
            i = prev;
        }
        i
    }

    fn next_word_boundary(s: &str, pos: usize) -> usize {
        let len = s.len();
        if pos >= len {
            return len;
        }
        let mut i = pos;
        while i < len {
            let next = Self::next_grapheme_boundary(s, i);
            if !s[i..next].chars().next().unwrap().is_alphanumeric() {
                break;
            }
            i = next;
        }
        while i < len {
            let next = Self::next_grapheme_boundary(s, i);
            if s[i..next].chars().next().unwrap().is_alphanumeric() {
                break;
            }
            i = next;
        }
        i
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

    fn committed_action(&self) -> Option<ControlAction> {
        let id = self.id?;
        Some(ControlAction::TextCommitted { id, value: self.build_committed_payload() })
    }

    fn focus_requested_action(&self) -> Option<ControlAction> {
        self.id.map(|id| ControlAction::FocusRequested { id })
    }

    fn display_value(&self, text: &str) -> String {
        match self.echo_mode {
            EchoMode::Plain => text.to_string(),
            EchoMode::Masked => Self::mask_text(text),
        }
    }

    fn display_prefix_value(&self, text: &str, byte_end: usize) -> String {
        self.display_value(&text[..byte_end])
    }

    fn display_text(&self) -> String {
        self.display_value(self.text.as_str())
    }

    fn display_prefix_text(&self, byte_end: usize) -> String {
        self.display_prefix_value(self.text.as_str(), byte_end)
    }

    fn display_suffix_text(&self, byte_start: usize) -> String {
        self.display_value(&self.text.as_str()[byte_start..])
    }

    fn display_preedit_text(&self) -> String {
        self.display_value(self.preedit.as_str())
    }

    fn display_preedit_prefix_text(&self, byte_end: usize) -> String {
        self.display_prefix_value(self.preedit.as_str(), byte_end)
    }

    fn measure_display_text(
        measure: &mut dyn crate::core::measure::TextMeasure,
        text: &str,
        font_size: f32,
    ) -> f32 {
        measure.measure_with_font(
            text,
            font_size,
            None,
            shaping::Weight::NORMAL,
            shaping::Style::Normal,
        )
    }

    fn mask_text(text: &str) -> String {
        std::iter::repeat_n(MASKED_ECHO_GLYPH, text.graphemes(true).count()).collect()
    }

    fn build_committed_payload(&self) -> TextPayload {
        self.text.payload()
    }

    #[cfg(test)]
    fn on_key(&mut self, key_code: KeyCode, modifiers: Modifiers) -> bool {
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::new(&theme, 1.0);
        self.handle_key_down(key_code, modifiers, &mut event_context).0
    }

    fn handle_key_down(
        &mut self,
        kc: KeyCode,
        modifiers: Modifiers,
        event_context: &mut EventCtx<'_>,
    ) -> (bool, Option<ControlAction>) {
        match kc {
            KeyCode::Char(c) => {
                if modifiers.cmd {
                    match c {
                        'a' | 'A' => {
                            self.select_all();
                            return (true, None);
                        }
                        'c' | 'C' => {
                            if let Some(selected_text) = self.selection_text().map(str::to_owned) {
                                event_context.write_clipboard_text(&selected_text);
                            }
                            return (true, None);
                        }
                        'x' | 'X' => {
                            let previous_text = self.text.clone();
                            if self.echo_mode == EchoMode::Masked {
                                self.delete_selection();
                                return (true, self.edited_action_if_text_changed(&previous_text));
                            }
                            let clipboard_write_succeeded = self
                                .selection_text()
                                .map(str::to_owned)
                                .is_some_and(|selected_text| {
                                    event_context.write_clipboard_text(&selected_text)
                                });
                            if clipboard_write_succeeded {
                                self.delete_selection();
                            }
                            return (true, self.edited_action_if_text_changed(&previous_text));
                        }
                        'v' | 'V' => {
                            let previous_text = self.text.clone();
                            let pasted_text = match self.echo_mode {
                                EchoMode::Plain => {
                                    event_context.read_clipboard_text().map(TextStorage::Plain)
                                }
                                EchoMode::Masked => event_context
                                    .read_sensitive_clipboard_text()
                                    .map(TextStorage::Sensitive),
                            };
                            if let Some(pasted_text) = pasted_text {
                                self.insert_clipboard_text(pasted_text);
                            }
                            return (true, self.edited_action_if_text_changed(&previous_text));
                        }
                        _ => return (false, None),
                    }
                }
                // Regular char insertion
                let previous_text = self.text.clone();
                self.delete_selection();
                if self.text.len() + c.len_utf8() <= self.max_len_bytes {
                    let pos = self.cursor_byte;
                    self.text.insert(pos, c);
                    self.cursor_byte = Self::next_grapheme_boundary(&self.text, pos);
                }
                (true, self.edited_action_if_text_changed(&previous_text))
            }
            KeyCode::Backspace => {
                let previous_text = self.text.clone();
                if !self.delete_selection() && self.cursor_byte > 0 {
                    let prev = Self::prev_grapheme_boundary(&self.text, self.cursor_byte);
                    self.text.replace_range(prev..self.cursor_byte, "");
                    self.cursor_byte = prev;
                }
                (true, self.edited_action_if_text_changed(&previous_text))
            }
            KeyCode::Delete => {
                let previous_text = self.text.clone();
                if !self.delete_selection() && self.cursor_byte < self.text.len() {
                    let next = Self::next_grapheme_boundary(&self.text, self.cursor_byte);
                    self.text.replace_range(self.cursor_byte..next, "");
                }
                (true, self.edited_action_if_text_changed(&previous_text))
            }
            KeyCode::Left => {
                let new_cursor = if modifiers.cmd {
                    0
                } else if modifiers.alt {
                    Self::prev_word_boundary(&self.text, self.cursor_byte)
                } else {
                    Self::prev_grapheme_boundary(&self.text, self.cursor_byte)
                };
                if modifiers.shift {
                    // Extend/start selection
                    if self.selection.is_none() {
                        self.selection = Some((self.cursor_byte, self.cursor_byte));
                    }
                    self.cursor_byte = new_cursor;
                    // Update selection cursor end
                    if let Some((anchor, _)) = self.selection {
                        self.selection = Some((anchor, new_cursor));
                    }
                } else if self.selection.is_some() && !(modifiers.cmd || modifiers.alt) {
                    // Collapse to the left edge of selection
                    let (a, b) = self.selection.take().expect("selection was checked above");
                    self.cursor_byte = a.min(b);
                } else {
                    self.selection = None;
                    self.cursor_byte = new_cursor;
                }
                (true, None)
            }
            KeyCode::Right => {
                let new_cursor = if modifiers.cmd {
                    self.text.len()
                } else if modifiers.alt {
                    Self::next_word_boundary(&self.text, self.cursor_byte)
                } else {
                    Self::next_grapheme_boundary(&self.text, self.cursor_byte)
                };
                if modifiers.shift {
                    if self.selection.is_none() {
                        self.selection = Some((self.cursor_byte, self.cursor_byte));
                    }
                    self.cursor_byte = new_cursor;
                    if let Some((anchor, _)) = self.selection {
                        self.selection = Some((anchor, new_cursor));
                    }
                } else if self.selection.is_some() && !(modifiers.cmd || modifiers.alt) {
                    let (a, b) = self.selection.take().expect("selection was checked above");
                    self.cursor_byte = a.max(b);
                } else {
                    self.selection = None;
                    self.cursor_byte = new_cursor;
                }
                (true, None)
            }
            KeyCode::Home => {
                self.cursor_byte = if modifiers.shift {
                    if self.selection.is_none() {
                        self.selection = Some((self.cursor_byte, 0));
                    } else if let Some((anchor, _)) = self.selection {
                        self.selection = Some((anchor, 0));
                    }
                    0
                } else {
                    self.selection = None;
                    0
                };
                (true, None)
            }
            KeyCode::End => {
                let end = self.text.len();
                self.cursor_byte = if modifiers.shift {
                    if self.selection.is_none() {
                        self.selection = Some((self.cursor_byte, end));
                    } else if let Some((anchor, _)) = self.selection {
                        self.selection = Some((anchor, end));
                    }
                    end
                } else {
                    self.selection = None;
                    end
                };
                (true, None)
            }
            KeyCode::Enter => {
                self.committed_payload = Some(self.build_committed_payload());
                (true, self.committed_action())
            }
            KeyCode::Escape => (true, None),
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => (true, None),
            _ => (false, None),
        }
    }

    fn insert_clipboard_text(&mut self, clipboard_text: TextStorage) -> bool {
        let normalized_text = clipboard_text.into_single_line();
        let selection_byte_count =
            self.selection.map(|(anchor, cursor)| anchor.abs_diff(cursor)).unwrap_or_default();
        let retained_byte_count = self.text.len().saturating_sub(selection_byte_count);
        let available_byte_count = self.max_len_bytes.saturating_sub(retained_byte_count);
        let accepted_byte_count = if normalized_text.len() <= available_byte_count {
            normalized_text.len()
        } else {
            Self::clamp_to_grapheme_boundary(&normalized_text, available_byte_count)
        };
        if accepted_byte_count == 0 {
            return false;
        }

        let accepted_text = &normalized_text.as_str()[..accepted_byte_count];
        self.delete_selection();
        let insertion_byte = self.cursor_byte;
        self.text.insert_str(insertion_byte, accepted_text);
        self.cursor_byte += accepted_text.len();
        true
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

    /// Mouse down: position cursor, clear selection, begin drag.
    pub fn on_mouse_down(&mut self, px: f32, py: f32) -> bool {
        if !self.rect.contains(px, py) {
            return false;
        }
        self.selection = None;
        self.dragging = true;
        self.cursor_byte = self.nearest_grapheme_byte_at_x(px);
        true
    }

    fn nearest_grapheme_byte_at_x(&self, px: f32) -> usize {
        let rel_x = (px - self.rect.x - self.text_pad).max(0.0);
        let mut nearest_byte = 0;
        let mut nearest_distance = f32::MAX;
        for &(byte, offset_x) in &self.grapheme_offsets {
            let distance = (offset_x - rel_x).abs();
            if distance < nearest_distance {
                nearest_distance = distance;
                nearest_byte = byte;
            }
        }
        nearest_byte
    }

    fn handle_mouse_down(&mut self, px: f32, py: f32) -> (bool, Option<ControlAction>) {
        if !self.on_mouse_down(px, py) {
            return (false, None);
        }
        (true, self.focus_requested_action())
    }

    /// Mouse drag: extend selection.
    pub fn on_mouse_drag(&mut self, px: f32, _py: f32) {
        if !self.dragging {
            return;
        }
        let anchor = self.selection.map_or(self.cursor_byte, |(anchor, _)| anchor);
        self.cursor_byte = self.nearest_grapheme_byte_at_x(px);
        self.selection = Some((anchor, self.cursor_byte));
    }

    /// Mouse up: end drag.
    pub fn on_mouse_up(&mut self) {
        self.dragging = false;
        if matches!(self.selection, Some((anchor, cursor)) if anchor == cursor) {
            self.selection = None;
        }
    }

    pub(super) fn cancel_transient_interaction(&mut self) -> bool {
        let interaction_changed = std::mem::take(&mut self.dragging)
            | !self.preedit.is_empty()
            | self.preedit_cursor.take().is_some();
        self.preedit.clear();
        interaction_changed
    }

    /// Receive an IME event from the parent widget.
    pub fn on_ime(&mut self, ev: &TextBoxIme) {
        if !self.focused {
            return;
        }
        self.handle_ime_event(ev);
    }

    fn handle_ime_event(&mut self, ev: &TextBoxIme) -> Option<ControlAction> {
        match ev {
            TextBoxIme::Preedit { text, cursor } => self.handle_ime_preedit(text, *cursor),
            TextBoxIme::Commit(text) => self.handle_ime_commit(text),
            TextBoxIme::Enabled | TextBoxIme::Disabled => {
                self.preedit.clear();
                self.preedit_cursor = None;
                None
            }
        }
    }

    fn handle_ime_preedit(
        &mut self,
        text: &str,
        cursor: Option<(usize, usize)>,
    ) -> Option<ControlAction> {
        self.preedit.replace(text);
        self.preedit_cursor = cursor;
        None
    }

    fn handle_ime_commit(&mut self, text: &str) -> Option<ControlAction> {
        let previous_text = self.text.clone();
        self.preedit.clear();
        self.preedit_cursor = None;
        if text.is_empty() {
            return self.edited_action_if_text_changed(&previous_text);
        }

        self.delete_selection();
        let mut to_insert = text;
        if self.text.len() + to_insert.len() > self.max_len_bytes {
            let allowed = self.max_len_bytes.saturating_sub(self.text.len());
            if allowed == 0 {
                to_insert = "";
            } else {
                let cutoff = Self::clamp_to_grapheme_boundary(to_insert, allowed);
                to_insert = &to_insert[..cutoff];
            }
        }
        if !to_insert.is_empty() {
            let insert_at = self.cursor_byte;
            self.text.insert_str(insert_at, to_insert);
            self.cursor_byte = insert_at + to_insert.len();
        }
        self.edited_action_if_text_changed(&previous_text)
    }

    /// Pixel rect where the OS IME candidate window should appear.
    pub fn ime_cursor_rect(&self) -> Rect {
        let cursor_h = self.rect.h * 0.6;
        let cursor_y = self.rect.y + (self.rect.h - cursor_h) * 0.5;
        let effective_x = self.cursor_x + self.preedit_cursor_x;
        Rect::new(self.rect.x + self.text_pad + effective_x, cursor_y, 2.0, cursor_h)
    }

    pub fn has_preedit(&self) -> bool {
        !self.preedit.is_empty()
    }

    /// Compute layout: measure text widths for cursor positioning.
    /// Called by parent during set_rect / layout phase.
    pub fn layout(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let font_size = self.font_size_logical * ctx.dpi;

        // Measure text up to cursor for cursor_x
        let measure: &mut dyn crate::core::measure::TextMeasure = match ctx.ui_measure {
            Some(ref mut m) => &mut **m,
            None => ctx.measure,
        };
        self.cursor_x = Self::measure_display_text(
            measure,
            &self.display_prefix_text(self.cursor_byte),
            font_size,
        );

        // Measure preedit text width
        if !self.preedit.is_empty() {
            self.preedit_width =
                Self::measure_display_text(measure, &self.display_preedit_text(), font_size);

            let cur = self.preedit_cursor.map(|(_, c)| c).unwrap_or(self.preedit.len());
            let valid_cur = Self::clamp_to_grapheme_boundary(self.preedit.as_str(), cur);
            self.preedit_cursor_x = Self::measure_display_text(
                measure,
                &self.display_preedit_prefix_text(valid_cur),
                font_size,
            );
        } else {
            self.preedit_width = 0.0;
            self.preedit_cursor_x = 0.0;
        }

        self.text_pad = self.leading_content_inset_logical * ctx.dpi;

        // Build byte→pixel offset table for mouse click positioning
        self.grapheme_offsets.clear();
        self.grapheme_offsets.push((0, 0.0));
        for (byte_start, grapheme) in self.text.as_str().grapheme_indices(true) {
            let byte_pos = byte_start + grapheme.len();
            let px =
                Self::measure_display_text(measure, &self.display_prefix_text(byte_pos), font_size);
            self.grapheme_offsets.push((byte_pos, px));
        }
    }

    /// Paint the text input: background, border, text/placeholder, selection, cursor, preedit.
    pub fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let dpi = ctx.dpi;
        let font_size = self.font_size_logical * dpi;
        let baseline = self.rect.y + self.rect.h * 0.5 + font_size * 0.35;
        self.paint_chrome(ctx);

        let text_x = self.rect.x + self.text_pad;

        // 3. Selection highlight
        if let Some((anchor, cur)) = self.selection {
            let sel_start = anchor.min(cur);
            let sel_end = anchor.max(cur);
            // 从 grapheme_offsets 查找像素位置
            let mut sx = 0.0f32;
            let mut ex = 0.0f32;
            for &(byte, px) in &self.grapheme_offsets {
                if byte == sel_start {
                    sx = px;
                }
                if byte == sel_end {
                    ex = px;
                }
            }
            if ex > sx {
                let sel_rect = Rect::new(
                    text_x + sx,
                    self.rect.y + 2.0 * dpi,
                    ex - sx,
                    self.rect.h - 4.0 * dpi,
                );
                ctx.list.fill_rounded(sel_rect, ctx.theme.editor.selection, 2.0 * dpi);
            }
        }

        // 4. Text or placeholder
        if !self.text.is_empty() {
            let display_text = self.display_text();
            if let Some(ref mut shaper) = ctx.shaper {
                if self.preedit.is_empty() {
                    ctx.list.text_shaped(
                        text_x,
                        baseline,
                        font_size,
                        ctx.theme.palette.input_fg,
                        &display_text,
                        shaper,
                    );
                } else {
                    let head = self.display_prefix_text(self.cursor_byte);
                    let tail = self.display_suffix_text(self.cursor_byte);
                    if !head.is_empty() {
                        ctx.list.text_shaped(
                            text_x,
                            baseline,
                            font_size,
                            ctx.theme.palette.input_fg,
                            &head,
                            shaper,
                        );
                    }
                    if !tail.is_empty() {
                        ctx.list.text_shaped(
                            text_x + self.cursor_x + self.preedit_width,
                            baseline,
                            font_size,
                            ctx.theme.palette.input_fg,
                            &tail,
                            shaper,
                        );
                    }
                }
            }
        } else if self.preedit.is_empty() && !self.placeholder.is_empty() {
            let ph_color = {
                let mut c = ctx.theme.palette.input_fg;
                c[3] *= 0.4;
                c
            };
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    text_x,
                    baseline,
                    font_size,
                    ph_color,
                    &self.placeholder,
                    shaper,
                );
            }
        }

        // 5. IME preedit text + underline
        if !self.preedit.is_empty() {
            let preedit_x = text_x + self.cursor_x;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    preedit_x,
                    baseline,
                    font_size,
                    ctx.theme.palette.input_fg,
                    &self.display_preedit_text(),
                    shaper,
                );
            }
            // Underline
            let ul_y = baseline + 2.0 * dpi;
            let ul_h = 1.5 * dpi;
            let ul_w = self.preedit_width;
            ctx.list.fill(Rect::new(preedit_x, ul_y, ul_w, ul_h), ctx.theme.palette.input_fg);
        }

        // 6. Cursor
        if self.blink_on && self.focused && self.selection.is_none() {
            let cursor_h = font_size * 1.2;
            let cursor_w = 2.0 * dpi;
            let cursor_y = self.rect.y + (self.rect.h - cursor_h) * 0.5;
            let effective_cursor_x = self.cursor_x + self.preedit_cursor_x;
            let cursor_rect = Rect::new(
                text_x + effective_cursor_x - cursor_w * 0.5,
                cursor_y,
                cursor_w,
                cursor_h,
            );
            ctx.list.fill(cursor_rect, ctx.theme.palette.input_fg);
        }
    }

    fn paint_chrome(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        if self.chrome == TextBoxChrome::Seamless {
            if self.focused {
                ctx.list.fill_rounded(
                    self.rect,
                    ctx.theme.palette.bg_hover,
                    SEAMLESS_FOCUS_CORNER_RADIUS_LOGICAL * dpi,
                );
            }
            return;
        }

        let mut background = ctx.theme.palette.input_bg;
        if self.focused {
            background[0] = (background[0] + 0.04).min(1.0);
            background[1] = (background[1] + 0.04).min(1.0);
            background[2] = (background[2] + 0.04).min(1.0);
        }
        let corner_radius = FRAMED_CORNER_RADIUS_LOGICAL * dpi;
        ctx.list.fill_rounded(self.rect, background, corner_radius);

        let border_color =
            if self.focused { ctx.theme.palette.accent } else { ctx.theme.palette.input_border };
        let line_width = if self.focused { 1.5 * dpi } else { 1.0 * dpi };
        ctx.list.stroke_rounded(self.rect, border_color, corner_radius, line_width);
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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Clipboard;
    use crate::core::widget::{ControlAction, Event, EventCtx, Widget, WidgetAction, WidgetId};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct TestClipboard {
        text: Rc<RefCell<Option<String>>>,
    }

    impl Clipboard for TestClipboard {
        fn read_text(&mut self) -> Option<String> {
            self.text.borrow().clone()
        }

        fn write_text(&mut self, text: &str) -> bool {
            self.text.replace(Some(text.to_owned()));
            true
        }
    }

    struct RejectingClipboard;

    impl Clipboard for RejectingClipboard {
        fn read_text(&mut self) -> Option<String> {
            None
        }

        fn write_text(&mut self, _text: &str) -> bool {
            false
        }
    }

    fn paint_laid_out(tb: &mut TextBox) -> crate::core::paint::DrawList {
        use crate::core::measure::NoopMeasure;

        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        tb.layout(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_ctx);

        let mut draw_list = crate::core::paint::DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint_ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        tb.paint(&mut paint_ctx);
        draw_list
    }

    fn laid_out_widget(mut text_box: TextBox) -> TextBox {
        use crate::core::measure::NoopMeasure;

        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        text_box.set_rect(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_ctx);
        text_box
    }

    fn key(text_box: &mut TextBox, key_code: KeyCode) -> Option<WidgetAction> {
        key_with_modifiers(text_box, key_code, Modifiers::NONE)
    }

    fn key_with_modifiers(
        text_box: &mut TextBox,
        key_code: KeyCode,
        modifiers: Modifiers,
    ) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);
        text_box.set_focus(true);
        text_box.on_event(&Event::KeyDown(key_code, modifiers), &mut event_ctx)
    }

    #[test]
    fn unfocused_text_box_rejects_keyboard_and_ime_input() {
        let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(29)));
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::new(&theme, 1.0);

        assert_eq!(
            text_box.on_event(
                &Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE),
                &mut event_context,
            ),
            None
        );
        assert_eq!(
            text_box.on_event(
                &Event::ImePreedit { text: "ni".into(), cursor: Some((2, 2)) },
                &mut event_context
            ),
            None
        );
        assert_eq!(text_box.on_event(&Event::ImeCommit("你好".into()), &mut event_context), None);
        text_box.on_ime(&TextBoxIme::Commit("仍不应写入".into()));
        assert_eq!(text_box.text(), "");
        assert!(!text_box.has_preedit());
    }

    #[test]
    fn accessibility_exposes_plain_value_and_set_value_reuses_text_edited_action() {
        let id = WidgetId(80);
        let mut text_box = laid_out_widget(TextBox::with_id(id));
        text_box.set_accessibility_label(Some("字体名称".into()));
        text_box.set_text("Menlo");
        text_box.set_keyboard_focus(Some(id));
        let context = crate::core::AccessibilityContext::new(10.0, 20.0);
        let node =
            text_box.accessibility_node(&context).expect("identified textbox exposes semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::TextField);
        assert_eq!(node.name.as_deref(), Some("字体名称"));
        assert_eq!(node.value.as_deref(), Some("Menlo"));
        assert_eq!(node.bounds, Rect::new(10.0, 20.0, 200.0, 28.0));
        assert!(node.state.focused);
        assert!(!node.state.sensitive);
        assert!(node.actions.contains(&crate::core::AccessibilityAction::SetValue));

        assert_eq!(
            text_box.on_accessibility_action(&crate::core::AccessibilityActionRequest::set_value(
                node.id,
                TextPayload::Plain("Monaco".into()),
            )),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                id,
                value: TextPayload::Plain("Monaco".into()),
            }))
        );
        assert_eq!(text_box.text(), "Monaco");
    }

    #[test]
    fn accessibility_never_exposes_masked_text_value() {
        let id = WidgetId(81);
        let mut text_box = laid_out_widget(TextBox::with_id(id));
        text_box.set_accessibility_label(Some("访问令牌".into()));
        text_box.set_password_mode(true);
        text_box.set_text("secret-token");

        let node = text_box
            .accessibility_node(&crate::core::AccessibilityContext::default())
            .expect("masked textbox remains discoverable");

        assert_eq!(node.value, None);
        assert!(node.state.sensitive);
        assert_eq!(crate::core::AccessibilityTree::new(node, None).validate(), Ok(()));
    }

    #[test]
    fn pointer_leave_preserves_text_drag_and_cancel_clears_drag_and_preedit() {
        let mut text_box = laid_out_widget(TextBox::new());
        text_box.set_text("hello");
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::new(&theme, 1.0);

        assert!(
            text_box
                .on_event(
                    &Event::MouseDown { px: 10.0, py: 14.0, button: MouseButton::Left },
                    &mut event_context,
                )
                .is_some()
        );
        text_box.set_focus(true);
        assert_eq!(
            text_box.on_event(
                &Event::ImePreedit { text: "未完成".to_owned(), cursor: Some((0, 6)) },
                &mut event_context,
            ),
            Some(WidgetAction::Consumed)
        );

        assert_eq!(text_box.on_event(&Event::PointerLeave, &mut event_context), None);
        assert!(text_box.is_capturing());
        assert!(text_box.has_preedit());

        assert_eq!(
            text_box.on_event(&Event::InteractionCancel, &mut event_context),
            Some(WidgetAction::Consumed)
        );
        assert!(!text_box.is_capturing());
        assert!(!text_box.has_preedit());
        assert_eq!(text_box.on_event(&Event::InteractionCancel, &mut event_context), None);
        assert_eq!(
            text_box.on_event(
                &Event::MouseUp { px: 10.0, py: 14.0, button: MouseButton::Left },
                &mut event_context,
            ),
            None
        );
    }

    #[test]
    fn tiny_pointer_move_during_click_keeps_caret_visible_after_mouse_up() {
        use crate::core::paint::DrawCmd;

        let mut text_box = laid_out_widget(TextBox::new());
        text_box.set_text("hello");
        text_box.set_focus(true);
        text_box.set_blink(true);

        assert!(text_box.on_mouse_down(20.0, 14.0));
        text_box.on_mouse_drag(20.25, 14.0);
        text_box.on_mouse_up();

        assert_eq!(text_box.selection, None);
        let draw_list = paint_laid_out(&mut text_box);
        assert!(draw_list.cmds.iter().any(|command| {
            matches!(
                command,
                DrawCmd::FillRect { rect, radius, .. }
                    if *radius == 0.0 && rect.w == 2.0
            )
        }));
    }

    #[test]
    fn fixed_size_text_box_is_left_aligned_and_vertically_centered() {
        let theme = crate::theme::test_theme();
        let mut measure = crate::core::measure::NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut text_box = TextBox::with_id(WidgetId(90));
        text_box.set_fixed_size_logical(200.0, 32.0);

        text_box.set_rect(Rect::new(0.0, 0.0, 240.0, 56.0), &mut layout);

        assert_eq!(text_box.rect(), Rect::new(0.0, 12.0, 200.0, 32.0));
    }

    #[test]
    fn leading_content_inset_reserves_room_for_an_input_icon() {
        let mut text_box = TextBox::with_id(WidgetId(91));
        text_box.set_placeholder("Search...");
        text_box.set_leading_content_inset_logical(28.0);

        let draw_list = paint_laid_out(&mut text_box);

        assert_eq!(text_box.text_pad, 28.0);
        assert!(draw_list.cmds.iter().any(|command| {
            matches!(command, crate::core::paint::DrawCmd::TextLayout { x, .. } if *x == 28.0)
        }));
    }

    #[test]
    fn textbox_widget_emits_plain_edit_and_commit_actions() {
        let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(30)));
        assert_eq!(
            key(&mut box_, KeyCode::Char('x')),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                id: WidgetId(30),
                value: TextPayload::Plain("x".into()),
            }))
        );
        assert!(matches!(
            key(&mut box_, KeyCode::Enter),
            Some(WidgetAction::Control(ControlAction::TextCommitted {
                id: WidgetId(30),
                value: TextPayload::Plain(text),
            })) if text == "x"
        ));
    }

    #[test]
    fn no_change_key_path_cmd_x_without_selection_consumes_without_edit_action() {
        let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(31)));
        box_.set_text("hello");

        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };
        assert_eq!(
            key_with_modifiers(&mut box_, KeyCode::Char('x'), cmd),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(box_.text(), "hello");
    }

    #[test]
    fn no_change_key_path_backspace_at_start_consumes_without_edit_action() {
        let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(32)));
        box_.set_text("a");
        box_.cursor_byte = 0;

        assert_eq!(key(&mut box_, KeyCode::Backspace), Some(WidgetAction::Consumed));
        assert_eq!(box_.text(), "a");
        assert_eq!(box_.cursor_byte(), 0);
    }

    #[test]
    fn no_change_key_path_delete_at_end_consumes_without_edit_action() {
        let mut box_ = laid_out_widget(TextBox::with_id(WidgetId(33)));
        box_.set_text("a");
        box_.cursor_byte = box_.text.len();

        assert_eq!(key(&mut box_, KeyCode::Delete), Some(WidgetAction::Consumed));
        assert_eq!(box_.text(), "a");
        assert_eq!(box_.cursor_byte(), 1);
    }

    #[test]
    fn new_textbox_is_empty() {
        let tb = TextBox::new();
        assert_eq!(tb.text(), "");
        assert_eq!(tb.cursor_byte(), 0);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn insert_char() {
        let mut tb = TextBox::new();
        tb.on_key(KeyCode::Char('a'), Modifiers::NONE);
        assert_eq!(tb.text(), "a");
        assert_eq!(tb.cursor_byte(), 1);
        tb.on_key(KeyCode::Char('b'), Modifiers::NONE);
        assert_eq!(tb.text(), "ab");
        assert_eq!(tb.cursor_byte(), 2);
    }

    #[test]
    fn backspace() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "hell");
        assert_eq!(tb.cursor_byte(), 4);
    }

    #[test]
    fn backspace_at_start_does_nothing() {
        let mut tb = TextBox::new();
        tb.set_text("a");
        tb.cursor_byte = 0;
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "a");
        assert_eq!(tb.cursor_byte(), 0);
    }

    #[test]
    fn backspace_utf8_char() {
        let mut tb = TextBox::new();
        tb.set_text("ab中d");
        // cursor is at end (byte 6 for "ab中d")
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "ab中");
        assert_eq!(tb.cursor_byte(), 5); // "中" is 3 bytes in UTF-8
    }

    #[test]
    fn cursor_and_backspace_treat_combining_sequence_as_one_grapheme() {
        let mut tb = TextBox::new();
        tb.set_text("e\u{301}");

        tb.on_key(KeyCode::Left, Modifiers::NONE);
        assert_eq!(tb.cursor_byte(), 0);

        tb.on_key(KeyCode::End, Modifiers::NONE);
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "");
        assert_eq!(tb.cursor_byte(), 0);
    }

    #[test]
    fn cursor_and_delete_treat_zwj_emoji_as_one_grapheme() {
        let family = "👨‍👩‍👧‍👦";
        let mut tb = TextBox::new();
        tb.set_text(family);

        tb.on_key(KeyCode::Left, Modifiers::NONE);
        assert_eq!(tb.cursor_byte(), 0);

        tb.on_key(KeyCode::Delete, Modifiers::NONE);
        assert_eq!(tb.text(), "");
    }

    #[test]
    fn layout_exposes_only_grapheme_boundaries_for_pointer_hit_testing() {
        let combining_sequence = "e\u{301}";
        let family = "👨‍👩‍👧‍👦";
        let text = format!("{combining_sequence}{family}");
        let mut tb = TextBox::new();
        tb.set_text(&text);

        paint_laid_out(&mut tb);

        let boundaries: Vec<_> =
            tb.grapheme_offsets.iter().map(|(byte_offset, _)| *byte_offset).collect();
        assert_eq!(boundaries, vec![0, combining_sequence.len(), text.len()]);
    }

    #[test]
    fn mouse_drag_selects_complete_graphemes() {
        struct GraphemeMeasure;

        impl crate::core::measure::TextMeasure for GraphemeMeasure {
            fn measure(&mut self, text: &str, _font_size: f32) -> f32 {
                text.graphemes(true).count() as f32 * 10.0
            }
        }

        let mut text_box = TextBox::new();
        text_box.set_text("ae\u{301}中b");
        let theme = crate::theme::test_theme();
        let mut measure = GraphemeMeasure;
        let mut layout_context =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        text_box.layout(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_context);
        let text_origin_x = text_box.rect.x + text_box.text_pad;

        assert!(text_box.on_mouse_down(text_origin_x + 10.0, 14.0));
        text_box.on_mouse_drag(text_origin_x + 30.0, 14.0);

        assert_eq!(text_box.selection_text(), Some("e\u{301}中"));
        assert_eq!(text_box.cursor_byte(), "ae\u{301}中".len());
    }

    #[test]
    fn set_text_resets_cursor_and_selection() {
        let mut tb = TextBox::new();
        tb.selection = Some((0, 3));
        tb.set_text("new");
        assert_eq!(tb.text(), "new");
        assert_eq!(tb.cursor_byte(), 3);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn cursor_rendering_is_centered() {
        use crate::core::paint::{DrawCmd, DrawList};
        use crate::core::{LayoutCtx, PaintCtx};

        let mut tb = TextBox::new();
        tb.set_text("a");

        struct DummyMeasure;
        impl crate::core::measure::TextMeasure for DummyMeasure {
            fn measure(&mut self, text: &str, _size: f32) -> f32 {
                text.len() as f32 * 10.0
            }
        }
        let mut measure = DummyMeasure;
        let theme = crate::theme::test_theme();

        let mut lctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        // text_pad will be 4.0 * dpi = 4.0
        // text_x will be 0.0 + 4.0 = 4.0
        tb.layout(Rect::new(0.0, 0.0, 100.0, 30.0), &mut lctx);

        tb.set_focus(true);
        tb.set_blink(true);

        let mut dl = DrawList::new();
        let mut pctx = PaintCtx::new(&mut dl, &theme, 1.0);
        tb.paint(&mut pctx);

        // Filter FillRect with 0 radius (cursor). Background has corner_radius 3.0, selection is not drawn.
        let cursor_cmds: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|cmd| match cmd {
                DrawCmd::FillRect { rect, radius, .. } if *radius == 0.0 => Some(rect),
                _ => None,
            })
            .collect();

        assert_eq!(cursor_cmds.len(), 1, "Should emit one FillRect for cursor");

        let cursor_rect = cursor_cmds[0];

        // Logical cursor_x = 10.0 (text length 1 * 10.0)
        // rect.x = 0.0, text_x = 4.0
        // logical drawing center = text_x + cursor_x = 14.0
        // cursor_w = 2.0 * dpi = 2.0
        // centered left bound = 14.0 - 1.0 = 13.0
        assert_eq!(cursor_rect.x, 13.0);
        assert_eq!(cursor_rect.w, 2.0);

        // IME candidate window follows the painted cursor, including the text inset.
        let ime_rect = tb.ime_cursor_rect();
        assert_eq!(ime_rect.x, 14.0);
    }

    #[test]
    fn selection_text_returns_correct_slice() {
        let mut tb = TextBox::new();
        tb.set_text("hello world");
        tb.selection = Some((0, 5));
        assert_eq!(tb.selection_text(), Some("hello"));
        // Order doesn't matter
        tb.selection = Some((11, 6));
        assert_eq!(tb.selection_text(), Some("world"));
    }

    #[test]
    fn delete_selection_replaces_on_insert() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.selection = Some((0, 5));
        tb.on_key(KeyCode::Char('x'), Modifiers::NONE);
        assert_eq!(tb.text(), "x");
        assert_eq!(tb.cursor_byte(), 1);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn backspace_deletes_selection() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.selection = Some((0, 3));
        tb.on_key(KeyCode::Backspace, Modifiers::NONE);
        assert_eq!(tb.text(), "lo");
        assert_eq!(tb.cursor_byte(), 0);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn cursor_left_right() {
        let mut tb = TextBox::new();
        tb.set_text("abc");
        tb.cursor_byte = 0;
        tb.on_key(KeyCode::Right, Modifiers::NONE);
        assert_eq!(tb.cursor_byte(), 1);
        tb.on_key(KeyCode::Right, Modifiers::NONE);
        assert_eq!(tb.cursor_byte(), 2);
        tb.on_key(KeyCode::Left, Modifiers::NONE);
        assert_eq!(tb.cursor_byte(), 1);
    }

    #[test]
    fn cursor_home_end() {
        let mut tb = TextBox::new();
        tb.set_text("abc");
        tb.cursor_byte = 1;
        tb.on_key(KeyCode::Home, Modifiers::NONE);
        assert_eq!(tb.cursor_byte(), 0);
        tb.on_key(KeyCode::End, Modifiers::NONE);
        assert_eq!(tb.cursor_byte(), 3);
    }

    #[test]
    fn shift_right_creates_selection() {
        let mut tb = TextBox::new();
        tb.set_text("abc");
        tb.cursor_byte = 0;
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        tb.on_key(KeyCode::Right, shift);
        assert!(tb.selection.is_some());
        let (anchor, cursor) = tb.selection.unwrap();
        assert_eq!(anchor, 0);
        assert_eq!(cursor, 1);
        assert_eq!(tb.cursor_byte(), 1);
    }

    #[test]
    fn shift_left_creates_selection() {
        let mut tb = TextBox::new();
        tb.set_text("abc");
        tb.cursor_byte = 3;
        let shift = Modifiers { shift: true, ..Modifiers::NONE };
        tb.on_key(KeyCode::Left, shift);
        assert!(tb.selection.is_some());
    }

    #[test]
    fn left_collapses_selection() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.selection = Some((0, 5));
        tb.on_key(KeyCode::Left, Modifiers::NONE);
        assert!(tb.selection.is_none());
        assert_eq!(tb.cursor_byte(), 0);
    }

    #[test]
    fn right_collapses_selection() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.selection = Some((0, 5));
        tb.on_key(KeyCode::Right, Modifiers::NONE);
        assert!(tb.selection.is_none());
        assert_eq!(tb.cursor_byte(), 5);
    }

    #[test]
    fn select_all_works() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.select_all();
        assert_eq!(tb.selection, Some((0, 5)));
        assert_eq!(tb.selection_text(), Some("hello"));
    }

    #[test]
    fn selection_text_utf8() {
        let mut tb = TextBox::new();
        tb.set_text("a中b");
        tb.selection = Some((1, 4)); // "中" in UTF-8: a=byte0, 中=bytes 1-3, b=byte4
        assert_eq!(tb.selection_text(), Some("中"));
    }

    #[test]
    fn set_focus_clears_selection() {
        let mut tb = TextBox::new();
        tb.set_focus(true);
        tb.set_text("hello");
        tb.selection = Some((0, 3));
        tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
        tb.set_focus(false);
        assert!(tb.selection.is_none());
        assert!(!tb.has_preedit());
    }

    #[test]
    fn ime_preedit_updates_state() {
        let mut tb = TextBox::new();
        tb.set_focus(true);
        tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
        assert!(tb.has_preedit());
        assert_eq!(tb.preedit, "ni");
    }

    #[test]
    fn ime_commit_inserts_text() {
        let mut tb = TextBox::new();
        tb.set_focus(true);
        tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((2, 2)) });
        tb.on_ime(&TextBoxIme::Commit("你好".into()));
        assert!(!tb.has_preedit());
        assert_eq!(tb.text(), "你好");
        assert_eq!(tb.cursor_byte(), 6);
    }

    #[test]
    fn text_box_ime_debug_never_exposes_text() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}

        const SECRET: &str = "text-box-ime-secret";
        let preedit = TextBoxIme::Preedit { text: SECRET.to_owned(), cursor: Some((0, 4)) };
        let commit = TextBoxIme::Commit(SECRET.to_owned());

        assert_zeroize_on_drop::<TextBoxIme>();
        assert!(!format!("{preedit:?}").contains(SECRET));
        assert!(!format!("{commit:?}").contains(SECRET));
    }

    #[test]
    fn layout_computes_cursor_x() {
        use crate::core::measure::NoopMeasure;

        let mut tb = TextBox::new();
        tb.set_text("abc");
        tb.cursor_byte = 2;

        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        let rect = Rect::new(10.0, 0.0, 200.0, 28.0);
        tb.layout(rect, &mut ctx);
        assert_eq!(tb.rect.x, 10.0);
        // cursor_x is 0.0 with NoopMeasure since it returns 0 for everything
        assert_eq!(tb.cursor_x, 0.0);
    }

    #[test]
    fn paint_empty_shows_placeholder() {
        use crate::core::paint::{DrawCmd, DrawList};

        let mut tb = TextBox::new();
        tb.set_placeholder("Search...");
        tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
        tb.blink_on = true;
        tb.focused = true;

        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        tb.paint(&mut pc);

        // Should have: bg, border, placeholder text, cursor
        assert!(dl.cmds.len() >= 4);
        let texts: Vec<_> =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).collect();
        assert_eq!(texts.len(), 1, "expected 1 text (placeholder)");
    }

    #[test]
    fn paint_shows_text_not_placeholder() {
        use crate::core::paint::{DrawCmd, DrawList};

        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.set_placeholder("Search...");
        tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);

        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        tb.paint(&mut pc);

        let texts: Vec<_> =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).collect();
        assert_eq!(texts.len(), 1);
        if let DrawCmd::TextLayout { layout, .. } = &texts[0] {
            assert_eq!(&layout.text, "hello");
        }
    }

    #[test]
    fn seamless_chrome_removes_the_idle_input_frame() {
        use crate::core::paint::DrawCmd;

        let mut text_box = TextBox::new();
        text_box.set_text("沉浸式标题");
        text_box.set_chrome(TextBoxChrome::Seamless);

        let draw_list = paint_laid_out(&mut text_box);

        assert!(!draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { .. } | DrawCmd::StrokeRect { .. }
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(command, DrawCmd::TextLayout { .. })));
    }

    #[test]
    fn masked_textbox_paints_bullets_and_commits_sensitive_payload() {
        let mut box_ = TextBox::new();
        box_.set_echo_mode(EchoMode::Masked);
        box_.set_text("secret-value");

        let draw_list = paint_laid_out(&mut box_);
        assert!(!format!("{:?}", draw_list.cmds).contains("secret-value"));

        assert!(box_.on_key(KeyCode::Enter, Modifiers::NONE));
        assert!(matches!(
            box_.take_committed_payload(),
            Some(TextPayload::Sensitive(secret)) if secret.expose() == "secret-value"
        ));
    }

    #[test]
    fn masked_text_uses_one_bullet_per_grapheme() {
        let mut text_box = TextBox::new();
        text_box.set_echo_mode(EchoMode::Masked);
        text_box.set_text("e\u{301}👨‍👩‍👧‍👦");

        assert_eq!(text_box.display_text(), "••");
    }

    #[test]
    fn byte_limit_does_not_insert_partial_grapheme_from_ime_or_clipboard() {
        let mut text_box = TextBox::new();
        text_box.set_max_len_bytes(1);
        text_box.set_focus(true);

        text_box.on_ime(&TextBoxIme::Commit("e\u{301}".into()));
        assert_eq!(text_box.text(), "");

        let clipboard_text = Rc::new(RefCell::new(Some("e\u{301}".to_owned())));
        let mut clipboard = TestClipboard { text: clipboard_text };
        let command = Modifiers { cmd: true, ..Modifiers::NONE };
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);
        assert_eq!(text_box.text(), "");
    }

    #[test]
    fn clipboard_paste_normalizes_line_breaks_for_single_line_input() {
        let clipboard_text =
            Rc::new(RefCell::new(Some("第一行\r\n第二行\n第三行\r第四行".to_owned())));
        let mut clipboard = TestClipboard { text: clipboard_text };
        let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(46)));
        text_box.set_focus(true);
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        let command = Modifiers { cmd: true, ..Modifiers::NONE };

        let action =
            text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);

        assert!(matches!(action, Some(WidgetAction::Control(ControlAction::TextEdited { .. }))));
        assert_eq!(text_box.text(), "第一行 第二行 第三行 第四行");
    }

    #[test]
    fn rejected_grapheme_paste_preserves_the_replaced_selection() {
        let clipboard_text = Rc::new(RefCell::new(Some("e\u{301}".to_owned())));
        let mut clipboard = TestClipboard { text: clipboard_text };
        let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(47)));
        text_box.set_max_len_bytes(1);
        text_box.set_text("a");
        text_box.set_focus(true);
        text_box.select_all();
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        let command = Modifiers { cmd: true, ..Modifiers::NONE };

        let action =
            text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);

        assert_eq!(action, Some(WidgetAction::Consumed));
        assert_eq!(text_box.text(), "a");
        assert_eq!(text_box.selection_text(), Some("a"));
    }

    #[test]
    fn widget_event_uses_context_clipboard_without_component_configuration() {
        let clipboard_text = Rc::new(RefCell::new(Some("粘贴内容".to_owned())));
        let mut clipboard = TestClipboard { text: Rc::clone(&clipboard_text) };
        let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(44)));
        text_box.set_text("原文");
        text_box.set_focus(true);
        text_box.select_all();
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        let command = Modifiers { cmd: true, ..Modifiers::NONE };

        let paste =
            text_box.on_event(&Event::KeyDown(KeyCode::Char('v'), command), &mut event_context);
        assert!(matches!(
            paste,
            Some(WidgetAction::Control(ControlAction::TextEdited {
                value: TextPayload::Plain(text),
                ..
            })) if text == "粘贴内容"
        ));

        text_box.select_all();
        let _ = text_box.on_event(&Event::KeyDown(KeyCode::Char('c'), command), &mut event_context);
        assert_eq!(clipboard_text.borrow().as_deref(), Some("粘贴内容"));

        text_box.select_all();
        let cut =
            text_box.on_event(&Event::KeyDown(KeyCode::Char('x'), command), &mut event_context);
        assert!(matches!(cut, Some(WidgetAction::Control(ControlAction::TextEdited { .. }))));
        assert_eq!(text_box.text(), "");
    }

    #[test]
    fn cut_preserves_selection_when_clipboard_write_fails() {
        let mut clipboard = RejectingClipboard;
        let mut text_box = laid_out_widget(TextBox::with_id(WidgetId(45)));
        text_box.set_text("不能丢失");
        text_box.set_focus(true);
        text_box.select_all();
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        let command = Modifiers { cmd: true, ..Modifiers::NONE };

        let action =
            text_box.on_event(&Event::KeyDown(KeyCode::Char('x'), command), &mut event_context);

        assert_eq!(action, Some(WidgetAction::Consumed));
        assert_eq!(text_box.text(), "不能丢失");
        assert_eq!(text_box.selection_text(), Some("不能丢失"));
    }

    #[test]
    fn masked_text_and_preedit_storage_never_debug_plaintext() {
        let mut text_box = TextBox::new();
        text_box.set_echo_mode(EchoMode::Masked);
        text_box.set_text("stored-api-key");
        text_box.set_focus(true);
        text_box
            .on_ime(&TextBoxIme::Preedit { text: "preedit-secret".into(), cursor: Some((0, 14)) });

        let stored_text_debug = format!("{:?}", text_box.text);
        let preedit_debug = format!("{:?}", text_box.preedit);
        assert!(!stored_text_debug.contains("stored-api-key"));
        assert!(!preedit_debug.contains("preedit-secret"));
    }

    #[test]
    fn masked_copy_and_cut_never_send_plaintext_to_clipboard() {
        let clipboard_text = Rc::new(RefCell::new(None));
        let mut clipboard = TestClipboard { text: Rc::clone(&clipboard_text) };
        let mut text_box = TextBox::new();
        text_box.set_echo_mode(EchoMode::Masked);
        text_box.set_text("clipboard-secret");
        text_box.set_focus(true);
        text_box.select_all();

        let command = Modifiers { cmd: true, ..Modifiers::NONE };
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        assert!(
            text_box
                .on_event(&Event::KeyDown(KeyCode::Char('c'), command), &mut event_context)
                .is_some()
        );
        assert!(clipboard_text.borrow().is_none());

        assert!(
            text_box
                .on_event(&Event::KeyDown(KeyCode::Char('x'), command), &mut event_context)
                .is_some()
        );
        assert!(clipboard_text.borrow().is_none());
        assert_eq!(text_box.text(), "");
    }

    #[test]
    fn masked_text_edit_emits_only_sensitive_payload() {
        let mut masked = laid_out_widget(TextBox::with_id(WidgetId(41)));
        masked.set_echo_mode(EchoMode::Masked);
        masked.sync_text("never-print-m");
        let action = key(&mut masked, KeyCode::Char('e'));
        assert!(matches!(
            action,
            Some(WidgetAction::Control(ControlAction::TextEdited {
                id: WidgetId(41),
                value: TextPayload::Sensitive(_),
            }))
        ));
        assert!(!format!("{action:?}").contains("never-print-me"));

        masked.selection = Some((0, masked.text.len()));
        assert!(matches!(
            key(&mut masked, KeyCode::Delete),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                value: TextPayload::Sensitive(_),
                ..
            }))
        ));

        masked.set_text("ab");
        masked.cursor_byte = masked.text.len();
        assert!(matches!(
            key(&mut masked, KeyCode::Backspace),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                value: TextPayload::Sensitive(_),
                ..
            }))
        ));

        masked.set_text("ab");
        masked.cursor_byte = 0;
        assert!(matches!(
            key(&mut masked, KeyCode::Delete),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                value: TextPayload::Sensitive(_),
                ..
            }))
        ));

        masked.set_text("");
        masked.cursor_byte = 0;
        let paste_modifiers = Modifiers { cmd: true, ..Modifiers::NONE };
        let clipboard_text = Rc::new(RefCell::new(Some(String::from("paste"))));
        let mut clipboard = TestClipboard { text: clipboard_text };
        let theme = crate::theme::test_theme();
        let mut event_ctx = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        assert!(matches!(
            masked.on_event(&Event::KeyDown(KeyCode::Char('v'), paste_modifiers), &mut event_ctx),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                value: TextPayload::Sensitive(_),
                ..
            }))
        ));

        masked.set_text("");
        masked.cursor_byte = 0;
        assert!(matches!(
            masked.on_event(&Event::ImeCommit(String::from("你好")), &mut event_ctx),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                value: TextPayload::Sensitive(_),
                ..
            }))
        ));

        let mut plain = laid_out_widget(TextBox::with_id(WidgetId(42)));
        assert_eq!(
            key(&mut plain, KeyCode::Char('p')),
            Some(WidgetAction::Control(ControlAction::TextEdited {
                id: WidgetId(42),
                value: TextPayload::Plain("p".into()),
            }))
        );
    }

    #[test]
    fn masked_preedit_uses_bullets_for_layout_and_paint() {
        use crate::core::measure::TextMeasure;
        use crate::core::paint::{DrawCmd, DrawList};

        struct DistinguishingMeasure;

        impl TextMeasure for DistinguishingMeasure {
            fn measure(&mut self, text: &str, _size: f32) -> f32 {
                text.chars().map(|ch| if ch == MASKED_ECHO_GLYPH { 7.0 } else { 13.0 }).sum()
            }
        }

        let mut tb = TextBox::new();
        tb.set_echo_mode(EchoMode::Masked);
        tb.set_text("secret");
        tb.set_focus(true);
        tb.cursor_byte = tb.text.len();
        tb.on_ime(&TextBoxIme::Preedit { text: "ni".into(), cursor: Some((0, 1)) });

        let theme = crate::theme::test_theme();
        let mut measure = DistinguishingMeasure;
        let mut layout_ctx =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        tb.layout(Rect::new(0.0, 0.0, 200.0, 28.0), &mut layout_ctx);

        assert_eq!(tb.preedit_width, 14.0);
        assert_eq!(tb.preedit_cursor_x, 7.0);

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
        tb.paint(&mut paint_ctx);

        let debug_output = format!("{:?}", draw_list.cmds);
        assert!(!debug_output.contains("ni"));

        let text_layouts: Vec<_> =
            draw_list.cmds.iter().filter(|cmd| matches!(cmd, DrawCmd::TextLayout { .. })).collect();
        assert_eq!(text_layouts.len(), 2, "expected masked text plus masked preedit");
        if let DrawCmd::TextLayout { layout, .. } = &text_layouts[1] {
            assert_eq!(&layout.text, "••");
        }

        let underline_count = draw_list
            .cmds
            .iter()
            .filter(|cmd| {
                matches!(
                    cmd,
                    DrawCmd::FillRect { rect, radius, .. }
                    if *radius == 0.0 && rect.w == 14.0 && rect.h == 1.5
                )
            })
            .count();
        assert_eq!(underline_count, 1, "expected masked preedit underline to remain visible");
    }

    #[test]
    fn paint_hides_cursor_when_blink_off() {
        use crate::core::paint::{DrawCmd, DrawList};

        let mut tb = TextBox::new();
        tb.set_text("x");
        tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
        tb.focused = true;
        tb.blink_on = false;

        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        tb.paint(&mut pc);

        let fills: Vec<_> =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).collect();
        // Border is now StrokeRect, so only bg fill remains (no cursor fill)
        assert_eq!(fills.len(), 1, "expected only bg fill, no cursor");
    }

    #[test]
    fn paint_preedit_draws_text_and_underline() {
        use crate::core::paint::{DrawCmd, DrawList};

        let mut tb = TextBox::new();
        tb.set_text("hello ");
        tb.set_focus(true);
        tb.cursor_byte = 6;
        tb.rect = Rect::new(0.0, 0.0, 200.0, 28.0);
        tb.on_ime(&TextBoxIme::Preedit { text: "世界".into(), cursor: Some((0, 6)) });
        tb.preedit_width = 28.0; // simulate layout

        let theme = crate::theme::test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        tb.paint(&mut pc);

        let texts: Vec<_> =
            dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).collect();
        assert_eq!(texts.len(), 2, "expected main text + preedit text");
        if let DrawCmd::TextLayout { layout, .. } = &texts[1] {
            assert_eq!(&layout.text, "世界");
        }
    }

    #[test]
    fn sync_text_overwrites_when_different() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.cursor_byte = 3;
        tb.sync_text("world");
        assert_eq!(tb.text(), "world");
        assert!(tb.selection.is_none());
    }

    #[test]
    fn sync_text_noop_when_same() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.cursor_byte = 3;
        tb.sync_text("hello");
        assert_eq!(tb.text(), "hello");
        assert_eq!(tb.cursor_byte(), 3); // preserved!
    }

    #[test]
    fn sync_text_clamps_cursor_when_shorter() {
        let mut tb = TextBox::new();
        tb.set_text("long text");
        tb.sync_text("short");
        assert_eq!(tb.cursor_byte(), 5); // clamped to "short".len()
    }

    #[test]
    fn sync_text_clamps_cursor_to_utf8_char_boundary() {
        let mut tb = TextBox::new();
        tb.set_text("123456789");

        tb.sync_text("H2AI 战略");

        assert_eq!(tb.cursor_byte(), 8);
        assert!(tb.text().is_char_boundary(tb.cursor_byte()));
        paint_laid_out(&mut tb);
    }

    #[test]
    fn sync_text_clamps_cursor_to_grapheme_boundary() {
        let mut tb = TextBox::new();
        tb.set_text("x");

        tb.sync_text("e\u{301}");

        assert_eq!(tb.cursor_byte(), 0);
        paint_laid_out(&mut tb);
    }

    #[test]
    fn ime_cursor_rect_includes_preedit_width() {
        let mut tb = TextBox::new();
        tb.rect = Rect::new(10.0, 0.0, 200.0, 28.0);
        tb.cursor_x = 50.0;
        tb.preedit_cursor_x = 30.0;
        let r = tb.ime_cursor_rect();
        assert_eq!(r.x, 10.0 + 50.0 + 30.0);
    }

    #[test]
    fn ime_cursor_rect_no_preedit() {
        let mut tb = TextBox::new();
        tb.rect = Rect::new(10.0, 0.0, 200.0, 28.0);
        tb.cursor_x = 50.0;
        tb.preedit_width = 0.0;
        let r = tb.ime_cursor_rect();
        assert_eq!(r.x, 10.0 + 50.0);
    }

    #[test]
    fn enter_key_records_committed_payload_without_widget_binding() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        let result = tb.on_key(KeyCode::Enter, Modifiers::NONE);
        assert!(result);
        assert_eq!(tb.take_committed_payload(), Some(TextPayload::Plain("hello".into())));
    }

    #[test]
    fn escape_key_is_consumed_without_action() {
        let mut tb = laid_out_widget(TextBox::with_id(WidgetId(43)));
        assert_eq!(key(&mut tb, KeyCode::Escape), Some(WidgetAction::Consumed));
    }

    #[test]
    fn delete_removes_char_after_cursor() {
        let mut tb = TextBox::new();
        tb.set_text("abc");
        tb.cursor_byte = 1;
        tb.on_key(KeyCode::Delete, Modifiers::NONE);
        assert_eq!(tb.text(), "ac");
        assert_eq!(tb.cursor_byte(), 1);
    }

    #[test]
    fn delete_removes_selection() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.selection = Some((1, 4));
        tb.on_key(KeyCode::Delete, Modifiers::NONE);
        assert_eq!(tb.text(), "ho");
        assert_eq!(tb.cursor_byte(), 1);
        assert!(tb.selection.is_none());
    }

    #[test]
    fn alt_left_right_moves_by_word() {
        let mut tb = TextBox::new();
        tb.set_text("hello world!");
        tb.cursor_byte = 0;
        let alt = Modifiers { alt: true, ..Modifiers::NONE };

        tb.on_key(KeyCode::Right, alt);
        assert_eq!(tb.cursor_byte(), 6);

        tb.on_key(KeyCode::Right, alt);
        assert_eq!(tb.cursor_byte(), 12);

        tb.on_key(KeyCode::Left, alt);
        assert_eq!(tb.cursor_byte(), 6);

        tb.on_key(KeyCode::Left, alt);
        assert_eq!(tb.cursor_byte(), 0);
    }

    #[test]
    fn cmd_left_right_moves_to_edges() {
        let mut tb = TextBox::new();
        tb.set_text("hello world!");
        tb.cursor_byte = 5;
        let cmd = Modifiers { cmd: true, ..Modifiers::NONE };

        tb.on_key(KeyCode::Left, cmd);
        assert_eq!(tb.cursor_byte(), 0);

        tb.cursor_byte = 5;
        tb.on_key(KeyCode::Right, cmd);
        assert_eq!(tb.cursor_byte(), 12);
    }

    #[test]
    fn up_down_are_consumed_without_mutating_text() {
        let mut tb = TextBox::new();
        tb.set_text("hello");
        tb.cursor_byte = 3;

        let res_up = tb.on_key(KeyCode::Up, Modifiers::NONE);
        assert!(res_up);
        assert_eq!(tb.text(), "hello");
        assert_eq!(tb.cursor_byte(), 3);

        let res_down = tb.on_key(KeyCode::Down, Modifiers::NONE);
        assert!(res_down);
        assert_eq!(tb.text(), "hello");
        assert_eq!(tb.cursor_byte(), 3);

        let res_pgup = tb.on_key(KeyCode::PageUp, Modifiers::NONE);
        assert!(res_pgup);

        let res_pgdn = tb.on_key(KeyCode::PageDown, Modifiers::NONE);
        assert!(res_pgdn);
    }
}
