//! TocWidget — table of contents panel for markdown preview.
//!
//! Renders a hierarchical list of headings with click-to-jump support.

use crate::core::geom::Rect;
use crate::core::measure::TextMeasure;
use crate::core::text_util::truncate_title_precise;
use crate::core::widget::{Event, EventCtx, LayoutCtx, PaintCtx, Widget, WidgetAction};
use crate::widgets::scrollbar::{SCROLLBAR_RESERVE_PX, ScrollbarAction, ScrollbarWidget};
use winit::window::CursorIcon;

/// A heading entry for the TOC panel.
#[derive(Clone, Debug)]
pub struct TocHeadingEntry {
    /// Heading text (plain, no markdown formatting).
    pub text: String,
    /// Heading level (1-6, corresponding to h1-h6).
    pub level: u8,
}

/// Input data for the TOC widget, rebuilt each frame.
#[derive(Clone, Debug)]
pub struct TocInput {
    /// Headings extracted from the markdown preview.
    pub headings: Vec<TocHeadingEntry>,
    /// Index of the currently visible/active heading (for highlighting).
    pub active_index: Option<usize>,
}

/// Actions emitted by the TOC widget.
#[derive(Clone, Debug, PartialEq)]
pub enum TocAction {
    /// Jump to the heading at the given index.
    JumpToHeading(usize),
}

/// Table of contents widget.
pub struct TocWidget {
    /// Current input data.
    input: TocInput,
    /// Scroll offset within the TOC panel (for long lists).
    scroll_y: f32,
    /// Index of the hovered heading entry.
    hovered_index: Option<usize>,
    /// Widget rectangle.
    rect: Rect,
    /// Pre-computed truncated text for each heading (computed in set_rect).
    truncated_texts: Vec<String>,
    /// Embedded scrollbar widget for overflow scrolling.
    scrollbar: ScrollbarWidget,
}

impl Default for TocWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TocWidget {
    pub fn new() -> Self {
        Self {
            input: TocInput { headings: Vec::new(), active_index: None },
            scroll_y: 0.0,
            hovered_index: None,
            rect: Rect::ZERO,
            truncated_texts: Vec::new(),
            scrollbar: ScrollbarWidget::new(),
        }
    }

    /// Update the TOC input data.
    pub fn set_input(&mut self, input: TocInput) {
        self.input = input;
    }

    /// Get the current input data.
    pub fn input(&self) -> &TocInput {
        &self.input
    }

    /// Scroll the TOC panel by delta pixels.
    pub fn scroll(&mut self, delta: f32, viewport_h: f32, dpi_scale: f32) {
        let max_scroll = (self.content_height(dpi_scale) - viewport_h).max(0.0);
        self.scroll_y = (self.scroll_y + delta).clamp(0.0, max_scroll);
    }

    /// Set the scroll offset directly (for persistent scroll state).
    pub fn set_scroll_y(&mut self, scroll_y: f32, _dpi_scale: f32) {
        self.scroll_y = scroll_y;
    }

    /// Get the current scroll offset.
    pub fn scroll_y(&self) -> f32 {
        self.scroll_y
    }

    /// Calculate total content height of all headings.
    pub fn content_height(&self, dpi_scale: f32) -> f32 {
        self.input.headings.len() as f32 * Self::ENTRY_HEIGHT * dpi_scale
    }

    /// Fixed line height for TOC entries (independent of editor font size).
    pub const ENTRY_HEIGHT: f32 = 22.0;

    /// Calculate the height of a single heading entry.
    fn entry_height(&self, dpi_scale: f32) -> f32 {
        Self::ENTRY_HEIGHT * dpi_scale
    }

    /// Calculate indentation for a heading level.
    fn indent_for_level(&self, level: u8, dpi_scale: f32) -> f32 {
        let base_indent = 8.0;
        let level_indent = 12.0;
        base_indent + (level.saturating_sub(1) as f32) * level_indent * dpi_scale
    }

    /// Hit-test a mouse position to find which heading was clicked.
    pub fn hit_test(&self, x: f32, y: f32, dpi_scale: f32) -> Option<usize> {
        if !self.rect.contains(x, y) {
            return None;
        }

        let rel_y = y - self.rect.y + self.scroll_y;
        let entry_h = self.entry_height(dpi_scale);
        let idx = (rel_y / entry_h).floor() as usize;

        if idx < self.input.headings.len() { Some(idx) } else { None }
    }

    /// Update hover state based on mouse position.
    pub fn update_hover(&mut self, x: f32, y: f32, dpi_scale: f32) {
        self.hovered_index =
            if self.rect.contains(x, y) { self.hit_test(x, y, dpi_scale) } else { None };
    }

    /// Truncate text to fit within max_width, adding ellipsis if needed.
    #[cfg(test)]
    /// NOTE: Character width estimation uses 0.6em as a rough average for proportional
    /// Latin text. CJK characters (~1.0em) may slightly overflow; very narrow Latin
    /// glyphs (~0.4em) leave extra space. For precise truncation, use TextMeasure.
    fn truncate_text(&self, text: &str, max_width: f32, font_size: f32) -> String {
        // Use char count (not byte count) to avoid panicking on multi-byte UTF-8.
        let char_width = font_size * 0.6;
        let max_chars = (max_width / char_width) as usize;
        let char_count = text.chars().count();
        if char_count <= max_chars {
            text.to_string()
        } else {
            let truncate_at = max_chars.saturating_sub(1);
            // Find a safe byte boundary using char_indices.
            let byte_idx =
                text.char_indices().nth(truncate_at).map(|(i, _)| i).unwrap_or(text.len());
            format!("{}…", &text[..byte_idx])
        }
    }

    /// Transform a mouse event's coordinates by subtracting an offset.
    /// Used to convert from TocWidget space to scrollbar-local space.
    fn to_local(ev: &Event, dx: f32, dy: f32) -> Event {
        match *ev {
            Event::MouseMove { px, py } => Event::MouseMove { px: px - dx, py: py - dy },
            Event::MouseDown { px, py, button } => {
                Event::MouseDown { px: px - dx, py: py - dy, button }
            }
            Event::MouseUp { px, py, button } => {
                Event::MouseUp { px: px - dx, py: py - dy, button }
            }
            Event::Wheel { dx: wdx, dy: wdy, px, py } => {
                Event::Wheel { dx: wdx, dy: wdy, px: px - dx, py: py - dy }
            }
            ref other => other.clone(),
        }
    }

    /// Translate a ScrollbarAction into a TOC scroll update.
    fn handle_scrollbar_action(&mut self, action: &WidgetAction, dpi_scale: f32) {
        if let WidgetAction::Scrollbar(sa) = action {
            match sa {
                ScrollbarAction::DragTo(new_scroll_top) => {
                    let entry_h = Self::ENTRY_HEIGHT * dpi_scale;
                    let new_scroll_y = (*new_scroll_top as f32) * entry_h;
                    let max_scroll = (self.content_height(dpi_scale) - self.rect.h).max(0.0);
                    self.scroll_y = new_scroll_y.clamp(0.0, max_scroll);
                }
                ScrollbarAction::PageUp => {
                    self.scroll(-self.rect.h, self.rect.h, dpi_scale);
                }
                ScrollbarAction::PageDown => {
                    self.scroll(self.rect.h, self.rect.h, dpi_scale);
                }
                _ => {}
            }
        }
    }
}

impl Widget for TocWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        // Convert to local coordinates (dock handles absolute offset via ctx.list.offset)
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        // Pre-compute truncated text for each heading using precise TextMeasure.
        let dpi = ctx.dpi;
        let font_size = 12.0 * dpi; // TOC uses a fixed small font size
        let measure: &mut dyn TextMeasure = ctx.measure;
        let indent_base = 8.0 * dpi;
        let indent_level = 12.0 * dpi;
        let right_pad = 8.0 * dpi;

        self.truncated_texts = self
            .input
            .headings
            .iter()
            .map(|heading| {
                let indent = indent_base + (heading.level.saturating_sub(1) as f32) * indent_level;
                let max_w = (rect.w - indent - right_pad).max(0.0);
                truncate_title_precise(&heading.text, max_w, font_size, measure)
            })
            .collect();

        // Configure the embedded scrollbar widget
        let content_h = self.content_height(ctx.dpi);
        if content_h > self.rect.h {
            let entry_h = self.entry_height(ctx.dpi);
            let total_rows = self.input.headings.len();
            let viewport_rows = (self.rect.h / entry_h) as f64;
            let scroll_rows = (self.scroll_y / entry_h) as f64;
            self.scrollbar.set_input(crate::widgets::scrollbar::ScrollbarInput {
                viewport_height_px: viewport_rows,
                total_display_rows: total_rows,
                scroll_top_rows: scroll_rows,
            });
            let sb_w = SCROLLBAR_RESERVE_PX * dpi;
            let sb_rect = Rect::new(self.rect.w - sb_w, 0.0, sb_w, self.rect.h);
            self.scrollbar.set_rect(sb_rect, ctx);
        } else {
            self.scrollbar.set_rect(Rect::ZERO, ctx);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        let alpha = ctx.global_alpha;

        // Background
        let mut bg = ctx.theme.markdown.toc_background;
        bg[3] *= alpha;
        ctx.list.fill(self.rect, bg);

        // Border (right side)
        let border_rect = Rect::new(self.rect.x + self.rect.w - 1.0, self.rect.y, 1.0, self.rect.h);
        let mut border_color = ctx.theme.palette.border_strong;
        border_color[3] *= alpha;
        ctx.list.fill(border_rect, border_color);

        if self.input.headings.is_empty() {
            // Empty state
            if let Some(ref mut shaper) = ctx.shaper {
                let text = "No headings";
                let text_x = self.rect.x + 12.0 * dpi;
                let font_size = 12.0 * dpi;
                let baseline = self.rect.y + 12.0 * dpi + font_size * 0.35;
                let mut text_color = ctx.theme.palette.text_muted;
                text_color[3] *= alpha;
                ctx.list.text_shaped(text_x, baseline, font_size, text_color, text, shaper);
            }
            return;
        }

        let entry_h = self.entry_height(ctx.dpi);
        let font_size = 12.0 * dpi; // Must match set_rect truncation font size

        // Calculate visible range
        let start_idx = (self.scroll_y / entry_h).floor() as usize;
        let end_idx = ((self.scroll_y + self.rect.h) / entry_h).ceil() as usize;
        let end_idx = end_idx.min(self.input.headings.len());

        for i in start_idx..end_idx {
            let heading = &self.input.headings[i];
            let y = self.rect.y + (i as f32 * entry_h) - self.scroll_y;

            // Skip if outside bounds
            if y + entry_h < self.rect.y || y > self.rect.y + self.rect.h {
                continue;
            }

            let is_active = self.input.active_index == Some(i);
            let is_hovered = self.hovered_index == Some(i);

            // Entry background (active or hovered)
            if is_active {
                let mut active_bg = ctx.theme.markdown.toc_active_background;
                active_bg[3] *= alpha;
                let bg_rect = Rect::new(self.rect.x, y, self.rect.w, entry_h);
                ctx.list.fill(bg_rect, active_bg);
            } else if is_hovered {
                let mut hover_bg = ctx.theme.markdown.toc_hover_background;
                hover_bg[3] *= alpha;
                let bg_rect = Rect::new(self.rect.x, y, self.rect.w, entry_h);
                ctx.list.fill(bg_rect, hover_bg);
            }

            // Heading text
            let indent = self.indent_for_level(heading.level, ctx.dpi);
            let text_x = self.rect.x + indent;
            let baseline = y + entry_h * 0.5 + font_size * 0.35;

            let mut text_color = if is_active {
                ctx.theme.markdown.toc_text
            } else if is_hovered {
                ctx.theme.markdown.toc_hover_text
            } else {
                ctx.theme.markdown.toc_text
            };
            text_color[3] *= alpha;

            // Use pre-computed truncated text from set_rect
            let display_text =
                self.truncated_texts.get(i).map(|s| s.as_str()).unwrap_or(&heading.text);

            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(text_x, baseline, font_size, text_color, display_text, shaper);
            }

            // Level indicator dot (small filled rect for visual hierarchy)
            if heading.level > 1 {
                let dot_x = self.rect.x + indent - 8.0 * dpi;
                let dot_y = y + entry_h * 0.5 - 2.0 * dpi;
                let dot_size = 4.0 * dpi;
                let mut dot_color = ctx.theme.markdown.toc_level_indicator;
                dot_color[3] *= alpha;
                let dot_rect = Rect::new(dot_x, dot_y, dot_size, dot_size);
                ctx.list.fill(dot_rect, dot_color);
            }
        }

        // Scrollbar: delegate to embedded ScrollbarWidget
        // Offset paint to scrollbar's position within the TOC panel
        let sb_w = SCROLLBAR_RESERVE_PX * dpi;
        let saved_offset = ctx.list.offset;
        ctx.list.offset = (saved_offset.0 + self.rect.w - sb_w, saved_offset.1);
        self.scrollbar.paint(ctx);
        ctx.list.offset = saved_offset;
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        // Transform event coordinates to scrollbar's local space.
        // The scrollbar is positioned at (rect.w - sb_w, 0) in TocWidget space.
        let sb_w = SCROLLBAR_RESERVE_PX * ctx.dpi;
        let sb_offset_x = self.rect.w - sb_w;
        let sb_ev = Self::to_local(ev, sb_offset_x, 0.0);

        match ev {
            Event::MouseMove { px, py, .. } => {
                // Delegate to scrollbar first (for hover/drag state)
                if let Some(action) = self.scrollbar.on_event(&sb_ev, ctx) {
                    self.handle_scrollbar_action(&action, ctx.dpi);
                }
                // Always update hover state so TOC items get hover highlighting
                self.update_hover(*px, *py, ctx.dpi);
                if self.hovered_index.is_some() {
                    ctx.cursor_hint = Some(CursorIcon::Pointer);
                }
                if self.rect.contains(*px, *py) { Some(WidgetAction::Consumed) } else { None }
            }
            Event::MouseDown { px, py, .. } => {
                // Delegate to scrollbar first (for thumb drag / page up/down)
                if let Some(action) = self.scrollbar.on_event(&sb_ev, ctx) {
                    self.handle_scrollbar_action(&action, ctx.dpi);
                    return Some(WidgetAction::Consumed);
                }
                self.hit_test(*px, *py, ctx.dpi)
                    .map(|idx| WidgetAction::Toc(TocAction::JumpToHeading(idx)))
            }
            Event::MouseUp { .. } => {
                if let Some(action) = self.scrollbar.on_event(&sb_ev, ctx) {
                    self.handle_scrollbar_action(&action, ctx.dpi);
                    return Some(WidgetAction::Consumed);
                }
                None
            }
            Event::Wheel { dy, px, py, .. } => {
                if self.rect.contains(*px, *py) {
                    self.scroll(*dy, self.rect.h, ctx.dpi);
                }
                if self.rect.contains(*px, *py) { Some(WidgetAction::Consumed) } else { None }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_widget(headings: Vec<(&str, u8)>) -> TocWidget {
        let mut w = TocWidget::new();
        w.set_input(TocInput {
            headings: headings
                .into_iter()
                .map(|(text, level)| TocHeadingEntry { text: text.to_string(), level })
                .collect(),
            active_index: None,
        });
        w
    }

    // ── truncate_text ──

    #[test]
    fn truncate_ascii_short() {
        let w = TocWidget::new();
        // max_width allows many chars
        let result = w.truncate_text("hello", 1000.0, 14.0);
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_ascii_exact_boundary() {
        let w = TocWidget::new();
        // font_size=14, char_width=8.4, max_width=8.4*5+0.01 → 5 chars fit
        let result = w.truncate_text("abcde", 42.01, 14.0);
        assert_eq!(result, "abcde");
    }

    #[test]
    fn truncate_ascii_needs_ellipsis() {
        let w = TocWidget::new();
        // max_width fits 3 chars → truncate at 2 + ellipsis
        let result = w.truncate_text("abcdefgh", 25.2, 14.0);
        assert_eq!(result, "ab…");
    }

    #[test]
    fn truncate_cjk_no_panic() {
        let w = TocWidget::new();
        // CJK chars are 3 bytes each; must not panic on byte-boundary issues
        let result = w.truncate_text("中文标题测试", 50.0, 14.0);
        // Should contain ellipsis and not panic
        assert!(result.ends_with('…'));
        assert!(result.len() < "中文标题测试".len());
    }

    #[test]
    fn truncate_empty_string() {
        let w = TocWidget::new();
        let result = w.truncate_text("", 100.0, 14.0);
        assert_eq!(result, "");
    }

    #[test]
    fn truncate_zero_width() {
        let w = TocWidget::new();
        let result = w.truncate_text("hello", 0.0, 14.0);
        assert_eq!(result, "…");
    }

    // ── hit_test ──

    #[test]
    fn hit_test_outside_rect() {
        let mut w = make_widget(vec![("H1", 1), ("H2", 2)]);
        w.rect = Rect::new(10.0, 10.0, 200.0, 400.0);
        assert_eq!(w.hit_test(5.0, 5.0, 1.0), None); // outside left/top
        assert_eq!(w.hit_test(300.0, 300.0, 1.0), None); // outside right/bottom
    }

    #[test]
    fn hit_test_first_item() {
        let mut w = make_widget(vec![("H1", 1), ("H2", 2)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        // Click at top of widget → index 0
        assert_eq!(w.hit_test(100.0, 5.0, 1.0), Some(0));
    }

    #[test]
    fn hit_test_second_item() {
        let mut w = make_widget(vec![("H1", 1), ("H2", 2)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        let entry_h = w.entry_height(1.0);
        assert_eq!(w.hit_test(100.0, entry_h + 5.0, 1.0), Some(1));
    }

    #[test]
    fn hit_test_beyond_last_item() {
        let mut w = make_widget(vec![("H1", 1)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        assert_eq!(w.hit_test(100.0, 9999.0, 1.0), None);
    }

    #[test]
    fn hit_test_empty_headings() {
        let mut w = TocWidget::new();
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        assert_eq!(w.hit_test(100.0, 5.0, 1.0), None);
    }

    // ── update_hover ──

    #[test]
    fn update_hover_sets_index() {
        let mut w = make_widget(vec![("H1", 1), ("H2", 2)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        w.update_hover(100.0, 5.0, 1.0);
        assert_eq!(w.hovered_index, Some(0));
    }

    #[test]
    fn update_hover_clears_when_outside() {
        let mut w = make_widget(vec![("H1", 1)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        w.hovered_index = Some(0);
        w.update_hover(999.0, 999.0, 1.0);
        assert_eq!(w.hovered_index, None);
    }

    // ── scroll ──

    #[test]
    fn scroll_clamps_to_zero() {
        let mut w = make_widget(vec![("H1", 1), ("H2", 2)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        w.scroll_y = 10.0;
        w.scroll(-100.0, 400.0, 1.0);
        assert_eq!(w.scroll_y, 0.0);
    }

    #[test]
    fn scroll_clamps_to_max() {
        let mut w = make_widget(vec![("H1", 1)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 10.0); // small viewport
        let content_h = w.content_height(1.0);
        w.scroll(9999.0, 10.0, 1.0);
        let expected = (content_h - 10.0).max(0.0);
        assert!((w.scroll_y - expected).abs() < 0.01);
    }

    // ── on_event ──

    #[test]
    fn click_emits_jump_action() {
        let mut w = make_widget(vec![("H1", 1), ("H2", 2)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let action = w.on_event(
            &Event::MouseDown { px: 50.0, py: 5.0, button: crate::core::widget::MouseButton::Left },
            &mut ctx,
        );
        assert_eq!(action, Some(WidgetAction::Toc(TocAction::JumpToHeading(0))));
    }

    #[test]
    fn click_outside_emits_nothing() {
        let mut w = make_widget(vec![("H1", 1)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let action = w.on_event(
            &Event::MouseDown {
                px: 999.0,
                py: 999.0,
                button: crate::core::widget::MouseButton::Left,
            },
            &mut ctx,
        );
        assert_eq!(action, None);
    }

    #[test]
    fn mousemove_updates_hover() {
        let mut w = make_widget(vec![("H1", 1), ("H2", 2)]);
        w.rect = Rect::new(0.0, 0.0, 200.0, 400.0);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        w.on_event(&Event::MouseMove { px: 50.0, py: 5.0 }, &mut ctx);
        assert_eq!(w.hovered_index, Some(0));
    }
}
