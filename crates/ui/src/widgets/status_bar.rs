//! StatusBarWidget — 基于 ui::status_bar 的纯文本/缓存逻辑，包装为 Widget。

use crate::core::{Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use std::any::Any;

// ── StatusBarInput / Cache / build_text（从旧 status_bar.rs 合并）──

/// Input data for building status bar text.
#[derive(Clone)]
pub struct StatusBarInput {
    /// Total buffer length in bytes (0 = empty/untitled).
    pub buffer_len: usize,
    /// Selection range as (start, end) byte offsets, if any.
    pub selection_range: Option<(usize, usize)>,
    /// Character count within the selection (None if not computed).
    pub selection_char_count: Option<usize>,
    /// 0-based cursor line.
    pub cursor_line: usize,
    /// 0-based cursor column.
    pub cursor_col: usize,
    /// Optional external-change/conflict label supplied by the app layer.
    pub conflict_label: Option<String>,
}

/// Cache for status bar text to avoid recomputing when selection hasn't changed.
pub struct StatusBarCache {
    selection_anchor: Option<usize>,
    selection_cursor: usize,
    char_count: usize,
    byte_count: usize,
}

impl Default for StatusBarCache {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBarCache {
    pub fn new() -> Self {
        Self { selection_anchor: None, selection_cursor: 0, char_count: 0, byte_count: 0 }
    }

    pub fn invalidate(&mut self) {
        self.selection_anchor = None;
    }
}

/// Build status bar text showing selection info or cursor position.
pub fn build_text(input: &StatusBarInput, cache: &mut StatusBarCache) -> String {
    let position_text = if input.buffer_len == 0 {
        String::new()
    } else if let Some((start, end)) = input.selection_range {
        let byte_count = end - start;
        if byte_count > 0 {
            if cache.selection_anchor.is_some()
                && cache.selection_cursor == end
                && cache.byte_count == byte_count
            {
                format!("{}c,{}b", cache.char_count, cache.byte_count)
            } else {
                let char_count = input.selection_char_count.unwrap_or(byte_count);
                cache.selection_anchor = Some(start);
                cache.selection_cursor = end;
                cache.byte_count = byte_count;
                cache.char_count = char_count;
                format!("{}c,{}b", char_count, byte_count)
            }
        } else {
            cache.invalidate();
            format!("{},{}", input.cursor_line + 1, input.cursor_col + 1)
        }
    } else {
        cache.invalidate();
        format!("{},{}", input.cursor_line + 1, input.cursor_col + 1)
    };

    match input.conflict_label.as_deref().filter(|label| !label.is_empty()) {
        Some(label) if position_text.is_empty() => label.to_owned(),
        Some(label) => format!("{position_text} · {label}"),
        None => position_text,
    }
}

pub struct StatusBarWidget {
    rect: Rect,
    cache: StatusBarCache,
    last_text: String,
    input: Option<StatusBarInput>,
}

impl Default for StatusBarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBarWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            cache: StatusBarCache::new(),
            last_text: String::new(),
            input: None,
        }
    }

    pub fn set_input(&mut self, input: StatusBarInput) {
        self.input = Some(input);
    }
}

impl Widget for StatusBarWidget {
    fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        if let Some(ref input) = self.input {
            self.last_text = build_text(input, &mut self.cache);
        } else {
            self.last_text.clear();
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        ctx.list.fill(Rect::new(0.0, 0.0, self.rect.w, self.rect.h), ctx.theme.palette.bg_surface);
        if !self.last_text.is_empty() {
            let font_size = crate::constants::CAPTION_FONT_SIZE * ctx.dpi;
            let y_baseline = self.rect.h * 0.5 + font_size * 0.35;
            let x = 32.0 * ctx.dpi;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    x,
                    y_baseline,
                    font_size,
                    ctx.theme.palette.text_muted,
                    &self.last_text,
                    shaper,
                );
            }
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
        None
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
    use crate::core::paint::DrawList;
    use crate::theme::Theme;

    fn test_theme() -> Theme {
        let mut t = crate::theme::test_theme();
        t.palette.bg_surface = [0.1, 0.1, 0.1, 1.0];
        t.palette.text_muted = [0.8, 0.8, 0.8, 1.0];
        t
    }
    fn empty_input() -> StatusBarInput {
        StatusBarInput {
            buffer_len: 0,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 0,
            cursor_col: 0,
            conflict_label: None,
        }
    }

    #[test]
    fn set_rect_without_input_clears_text() {
        let mut w = StatusBarWidget::new();
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 24.0), &mut lc);
        assert!(w.last_text.is_empty());
    }

    #[test]
    fn set_rect_with_empty_buffer() {
        let mut w = StatusBarWidget::new();
        w.set_input(empty_input());
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 24.0), &mut lc);
        assert!(w.last_text.is_empty());
    }

    #[test]
    fn build_text_shows_cursor_position() {
        let input = StatusBarInput {
            buffer_len: 100,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 4,
            cursor_col: 11,
            conflict_label: None,
        };
        let mut cache = StatusBarCache::new();
        assert_eq!(build_text(&input, &mut cache), "5,12");
    }

    #[test]
    fn build_text_shows_selection_count() {
        let input = StatusBarInput {
            buffer_len: 100,
            selection_range: Some((10, 20)),
            selection_char_count: Some(10),
            cursor_line: 0,
            cursor_col: 20,
            conflict_label: None,
        };
        let mut cache = StatusBarCache::new();
        assert_eq!(build_text(&input, &mut cache), "10c,10b");
    }

    #[test]
    fn build_text_appends_external_conflict_label() {
        let input = StatusBarInput {
            buffer_len: 100,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 4,
            cursor_col: 11,
            conflict_label: Some("已保留冲突副本".to_owned()),
        };
        let mut cache = StatusBarCache::new();
        assert_eq!(build_text(&input, &mut cache), "5,12 · 已保留冲突副本");
    }

    #[test]
    fn build_text_empty_input() {
        let input = empty_input();
        let mut cache = StatusBarCache::new();
        assert_eq!(build_text(&input, &mut cache), "");
    }

    #[test]
    fn cache_invalidate_resets() {
        let mut cache = StatusBarCache::new();
        let input1 = StatusBarInput {
            buffer_len: 100,
            selection_range: Some((0, 5)),
            selection_char_count: Some(5),
            cursor_line: 0,
            cursor_col: 5,
            conflict_label: None,
        };
        let _ = build_text(&input1, &mut cache);
        cache.invalidate();
        assert!(cache.selection_anchor.is_none());
    }

    #[test]
    fn build_text_uses_cache_on_same_selection() {
        let mut cache = StatusBarCache::new();
        let input1 = StatusBarInput {
            buffer_len: 100,
            selection_range: Some((10, 20)),
            selection_char_count: Some(10),
            cursor_line: 0,
            cursor_col: 20,
            conflict_label: None,
        };
        let first = build_text(&input1, &mut cache);
        let second = build_text(&input1, &mut cache);
        assert_eq!(first, second);
    }

    #[test]
    fn build_text_different_selection_invalidates_cache() {
        let mut cache = StatusBarCache::new();
        let input1 = StatusBarInput {
            buffer_len: 100,
            selection_range: Some((10, 20)),
            selection_char_count: Some(10),
            cursor_line: 0,
            cursor_col: 20,
            conflict_label: None,
        };
        let input2 = StatusBarInput {
            buffer_len: 100,
            selection_range: Some((10, 25)),
            selection_char_count: Some(15),
            cursor_line: 0,
            cursor_col: 25,
            conflict_label: None,
        };
        let first = build_text(&input1, &mut cache);
        let second = build_text(&input2, &mut cache);
        assert_ne!(first, second);
    }

    #[test]
    fn build_text_zero_length_selection_shows_cursor() {
        let input = StatusBarInput {
            buffer_len: 100,
            selection_range: Some((10, 10)),
            selection_char_count: None,
            cursor_line: 3,
            cursor_col: 7,
            conflict_label: None,
        };
        let mut cache = StatusBarCache::new();
        assert_eq!(build_text(&input, &mut cache), "4,8");
    }

    #[test]
    fn set_rect_with_input_shows_position() {
        let mut w = StatusBarWidget::new();
        w.set_input(StatusBarInput {
            buffer_len: 100,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 0,
            cursor_col: 0,
            conflict_label: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 24.0), &mut lc);
        assert_eq!(w.last_text, "1,1");
    }

    #[test]
    fn paint_fills_background() {
        let mut w = StatusBarWidget::new();
        w.set_input(StatusBarInput {
            buffer_len: 10,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 0,
            cursor_col: 0,
            conflict_label: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 24.0), &mut lc);

        let mut dl = DrawList::new();
        let mut pc = PaintCtx::new(&mut dl, &t, 1.0);
        w.paint(&mut pc);
        assert!(!dl.cmds.is_empty());
    }

    #[test]
    fn paint_empty_input_no_panic() {
        let mut w = StatusBarWidget::new();
        w.set_input(empty_input());
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 24.0), &mut lc);

        let mut dl = DrawList::new();
        let mut pc = PaintCtx::new(&mut dl, &t, 1.0);
        w.paint(&mut pc);
    }

    #[test]
    fn paint_zero_size_no_panic() {
        let mut w = StatusBarWidget::new();
        let t = test_theme();
        let mut dl = DrawList::new();
        let mut pc = PaintCtx::new(&mut dl, &t, 1.0);
        w.paint(&mut pc);
    }

    #[test]
    fn set_input_stores_data() {
        let mut w = StatusBarWidget::new();
        assert!(w.input.is_none());
        w.set_input(StatusBarInput {
            buffer_len: 50,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 2,
            cursor_col: 5,
            conflict_label: None,
        });
        assert!(w.input.is_some());
        assert_eq!(w.input.as_ref().unwrap().buffer_len, 50);
    }

    #[test]
    fn build_text_zero_buffer_len_returns_empty() {
        let input = StatusBarInput {
            buffer_len: 0,
            selection_range: Some((0, 10)),
            selection_char_count: Some(10),
            cursor_line: 0,
            cursor_col: 10,
            conflict_label: None,
        };
        let mut cache = StatusBarCache::new();
        assert_eq!(build_text(&input, &mut cache), "");
    }

    #[test]
    fn build_text_selection_shows_char_and_byte_count() {
        let input = StatusBarInput {
            buffer_len: 200,
            selection_range: Some((0, 15)),
            selection_char_count: Some(10),
            cursor_line: 0,
            cursor_col: 15,
            conflict_label: None,
        };
        let mut cache = StatusBarCache::new();
        let text = build_text(&input, &mut cache);
        assert_eq!(text, "10c,15b");
    }

    #[test]
    fn on_event_returns_none() {
        let mut w = StatusBarWidget::new();
        let t = test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &t, dpi: 1.0 };
        let result = w.on_event(&Event::MouseMove { px: 100.0, py: 12.0 }, &mut ctx);
        assert!(result.is_none());
    }
}
