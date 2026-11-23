use unicode_segmentation::UnicodeSegmentation;

use super::{EchoMode, MASKED_ECHO_GLYPH, TextBox, TextBoxChrome, TextBoxIme};
use crate::core::widget::ControlAction;
use crate::core::{LayoutCtx, PaintCtx, Rect};

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

impl TextBox {
    pub fn ime_allowed(&self) -> bool {
        self.echo_mode == EchoMode::Plain
    }

    /// Receive an IME event from the parent widget.
    pub fn on_ime(&mut self, ev: &TextBoxIme) {
        if !self.focused {
            return;
        }
        self.handle_ime_event(ev);
    }

    pub(super) fn handle_ime_event(&mut self, ev: &TextBoxIme) -> Option<ControlAction> {
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

    pub(super) fn handle_ime_preedit(
        &mut self,
        text: &str,
        cursor: Option<(usize, usize)>,
    ) -> Option<ControlAction> {
        self.preedit.replace(text);
        self.preedit_cursor = cursor;
        None
    }

    pub(super) fn handle_ime_commit(&mut self, text: &str) -> Option<ControlAction> {
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
            let placeholder_color = ctx.theme.application_theme().text_secondary;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    text_x,
                    baseline,
                    font_size,
                    placeholder_color,
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

    pub(super) fn paint_chrome(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        let corner_radius = ctx.theme.control_metrics().corner_radius_logical * dpi;
        if self.chrome == TextBoxChrome::Seamless {
            if self.focused {
                ctx.list.fill_rounded(self.rect, ctx.theme.palette.bg_hover, corner_radius);
            }
            return;
        }

        let mut background = ctx.theme.palette.input_bg;
        if self.focused {
            background[0] = (background[0] + 0.04).min(1.0);
            background[1] = (background[1] + 0.04).min(1.0);
            background[2] = (background[2] + 0.04).min(1.0);
        }
        ctx.list.fill_rounded(self.rect, background, corner_radius);

        let border_color = if self.focused {
            ctx.theme.application_theme().focus_ring
        } else {
            ctx.theme.palette.input_border
        };
        let line_width = if self.focused { 1.5 * dpi } else { 1.0 * dpi };
        ctx.list.stroke_rounded(self.rect, border_color, corner_radius, line_width);
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

    pub(super) fn display_text(&self) -> String {
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
}
