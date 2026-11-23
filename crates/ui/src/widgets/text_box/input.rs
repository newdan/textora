use unicode_segmentation::UnicodeSegmentation;

use super::{EchoMode, MouseDragSelection, TextBox, TextStorage};
use crate::core::widget::{ControlAction, PointerClickKind, TextPayload};
use crate::core::{EventCtx, KeyCode, Modifiers};

impl TextBox {
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

    fn committed_action(&self) -> Option<ControlAction> {
        let id = self.id?;
        Some(ControlAction::TextCommitted { id, value: self.build_committed_payload() })
    }

    fn focus_requested_action(&self) -> Option<ControlAction> {
        self.id.map(|id| ControlAction::FocusRequested { id })
    }

    fn build_committed_payload(&self) -> TextPayload {
        self.text.payload()
    }

    #[cfg(test)]
    pub(super) fn on_key(&mut self, key_code: KeyCode, modifiers: Modifiers) -> bool {
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx::new(&theme, 1.0);
        self.handle_key_down(key_code, modifiers, &mut event_context).0
    }

    pub(super) fn handle_key_down(
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

    /// Mouse down: position cursor, clear selection, begin drag.
    pub fn on_mouse_down(&mut self, px: f32, py: f32) -> bool {
        if !self.rect.contains(px, py) {
            return false;
        }
        let click_kind = self.pointer_click_tracker.record_press((px, py));
        self.dragging = true;
        self.cursor_byte = self.nearest_grapheme_byte_at_x(px);
        match click_kind {
            PointerClickKind::Single => {
                self.selection = None;
                self.mouse_drag_selection =
                    Some(MouseDragSelection::Grapheme { anchor: self.cursor_byte });
            }
            PointerClickKind::Double => {
                let (start, end) = self.pointer_word_range(self.cursor_byte);
                self.selection = Some((start, end));
                self.cursor_byte = end;
                self.mouse_drag_selection = Some(MouseDragSelection::Word { start, end });
            }
            PointerClickKind::Triple => {
                self.select_all();
                self.mouse_drag_selection = Some(MouseDragSelection::EntireLine);
            }
        }
        true
    }

    fn pointer_word_range(&self, byte_offset: usize) -> (usize, usize) {
        if self.text.is_empty() {
            return (0, 0);
        }
        let clamped_offset = Self::clamp_to_grapheme_boundary(&self.text, byte_offset);
        let probe_start = if clamped_offset == self.text.len() {
            Self::prev_grapheme_boundary(&self.text, clamped_offset)
        } else {
            clamped_offset
        };
        let probe_end = Self::next_grapheme_boundary(&self.text, probe_start);
        let selects_word = Self::is_word_grapheme(&self.text[probe_start..probe_end]);

        let mut start = probe_start;
        while start > 0 {
            let previous = Self::prev_grapheme_boundary(&self.text, start);
            if Self::is_word_grapheme(&self.text[previous..start]) != selects_word {
                break;
            }
            start = previous;
        }

        let mut end = probe_end;
        while end < self.text.len() {
            let next = Self::next_grapheme_boundary(&self.text, end);
            if Self::is_word_grapheme(&self.text[end..next]) != selects_word {
                break;
            }
            end = next;
        }
        (start, end)
    }

    fn is_word_grapheme(grapheme: &str) -> bool {
        grapheme.chars().any(|character| character.is_alphanumeric() || character == '_')
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

    pub(super) fn handle_mouse_down(&mut self, px: f32, py: f32) -> (bool, Option<ControlAction>) {
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
        let hit_byte = self.nearest_grapheme_byte_at_x(px);
        match self.mouse_drag_selection {
            Some(MouseDragSelection::Grapheme { anchor }) => {
                self.cursor_byte = hit_byte;
                self.selection = Some((anchor, self.cursor_byte));
            }
            Some(MouseDragSelection::Word { start, end }) => {
                let (hit_start, hit_end) = self.pointer_word_range(hit_byte);
                if hit_byte >= start {
                    self.selection = Some((start, hit_end));
                    self.cursor_byte = hit_end;
                } else {
                    self.selection = Some((end, hit_start));
                    self.cursor_byte = hit_start;
                }
            }
            Some(MouseDragSelection::EntireLine) => self.select_all(),
            None => {}
        }
    }

    /// Mouse up: end drag.
    pub fn on_mouse_up(&mut self) {
        self.dragging = false;
        self.mouse_drag_selection = None;
        if matches!(self.selection, Some((anchor, cursor)) if anchor == cursor) {
            self.selection = None;
        }
    }
}
