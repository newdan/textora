//! tab_bar/state.rs — TabBar 状态与交互。

use super::hit::TabHit;
use super::layout::is_tab_in_clip;
use super::layout::{TabBarLayout, clamp_tab_scroll, layout_tabs, set_preview_tab};
use crate::core::geom::Rect;
use crate::core::paint::DrawList;
use crate::theme::Theme;

use super::types::{TabBarCtx, TabInfo, tab_bar_height};
use crate::core::widget::MouseButton;
use crate::widgets::popup_menu::ContextMenuAction;
pub struct TabBarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: Option<usize>,
    pub back_enabled: bool,
    pub forward_enabled: bool,
    pub screen_w: f32,
    pub screen_h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabBarAction {
    SwitchTab(usize),
    CloseTab(usize),
    NewEmptyTab,
    NavigateBack,
    NavigateForward,
    OpenContextMenuPx { tab_index: usize, anchor_px: (f32, f32) },
    OpenOverflowMenu,
    Context { action: ContextMenuAction, tab_index: usize },
    ScrollLeft,
    ScrollRight,
    HoverTab(Option<usize>),
}

#[derive(Default)]
pub struct TabBarState {
    layout: Option<TabBarLayout>,
    scroll_offset: f32,
    scroll_target: f32,
    hovered_index: Option<usize>,
    preview_index: Option<usize>,
    open_menu: Option<crate::widgets::popup_menu::PopupMenu>,
}

impl TabBarState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_layout(&self) -> Option<&TabBarLayout> {
        self.layout.as_ref()
    }
    pub fn open_menu(&self) -> Option<&crate::widgets::popup_menu::PopupMenu> {
        self.open_menu.as_ref()
    }
    pub fn set_open_menu(&mut self, menu: Option<crate::widgets::popup_menu::PopupMenu>) {
        self.open_menu = menu;
    }
    pub fn scroll_offset(&self) -> f32 {
        self.scroll_offset
    }
    pub fn set_scroll_offset(&mut self, off: f32) {
        self.scroll_offset = off;
    }
    /// 用户滚动输入。clamp 到 [0, max_scroll]。
    pub fn scroll_by(&mut self, delta: f32) {
        let max = self.layout.as_ref().map(|l| l.max_scroll).unwrap_or(0.0);
        self.scroll_target = (self.scroll_target - delta).clamp(0.0, max);
    }

    /// 当前滚动目标（供外部动画驱动读取）。
    pub fn scroll_target(&self) -> f32 {
        self.scroll_target
    }

    /// 直接设置滚动目标（autoscroll 用）。
    pub fn set_scroll_target(&mut self, target: f32) {
        self.scroll_target = target;
    }

    pub fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }
    pub fn set_hovered_index(&mut self, idx: Option<usize>) {
        self.hovered_index = idx;
    }
    pub fn preview_index(&self) -> Option<usize> {
        self.preview_index
    }
    pub fn set_preview_index(&mut self, idx: Option<usize>) {
        self.preview_index = idx;
    }
    /// Set the cached layout directly (used by workspace layout methods).
    pub fn set_layout_raw(&mut self, layout: TabBarLayout) {
        self.layout = Some(layout);
    }

    /// Recompute tab bar layout from input. Stores result in self.layout.
    pub fn update_layout(
        &mut self,
        input: &TabBarInput<'_>,
        shaper: Option<&mut shaping::Shaper>,
        dpi: f32,
    ) {
        let ctx = TabBarCtx { screen_w: input.screen_w, screen_h: input.screen_h, dpi };
        let mut layout = layout_tabs(
            input.tabs,
            input.active_index.unwrap_or(0),
            &ctx,
            tab_bar_height(dpi),
            input.back_enabled,
            input.forward_enabled,
            self.scroll_offset,
            shaper,
        );
        set_preview_tab(&mut layout, self.preview_index);
        self.layout = Some(layout);
    }

    pub fn clamp_scroll(&mut self, off: f32, max: f32) {
        self.scroll_offset = clamp_tab_scroll(off, max);
    }

    // ── Phase 6: px 新 API ──

    /// Pixel-space hit test using _px fields.
    pub fn hit_test_px(&self, px: f32, py: f32) -> Option<TabHit> {
        let layout = self.layout.as_ref()?;
        if !layout.left_arrow_disabled && layout.overflow_left_rect_px.contains(px, py) {
            return Some(TabHit::ScrollLeft);
        }
        if !layout.right_arrow_disabled && layout.overflow_right_rect_px.contains(px, py) {
            return Some(TabHit::ScrollRight);
        }
        if layout.dropdown_rect_px.contains(px, py) {
            return Some(TabHit::Dropdown);
        }
        if layout.new_tab_rect_px.contains(px, py) {
            return Some(TabHit::NewTab);
        }
        for entry in &layout.tabs {
            if entry.rect_px.contains(px, py) {
                // Non-pinned tabs must be within clip bounds to be clickable
                if !entry.pinned {
                    if !is_tab_in_clip(px, layout) {
                        continue;
                    }
                    if entry.close_rect_px.contains(px, py) {
                        return Some(TabHit::Close(entry.index));
                    }
                }
                return Some(TabHit::Tab(entry.index));
            }
        }
        None
    }

    /// Pixel-space click handler.
    pub fn on_click_px(&mut self, px: f32, py: f32, button: MouseButton) -> Option<TabBarAction> {
        let hit = self.hit_test_px(px, py)?;
        match hit {
            TabHit::Tab(idx) if button == MouseButton::Right => {
                Some(TabBarAction::OpenContextMenuPx { tab_index: idx, anchor_px: (px, py) })
            }
            TabHit::Tab(idx) => Some(TabBarAction::SwitchTab(idx)),
            TabHit::Close(idx) => Some(TabBarAction::CloseTab(idx)),
            TabHit::NewTab => Some(TabBarAction::NewEmptyTab),
            TabHit::ScrollLeft => Some(TabBarAction::ScrollLeft),
            TabHit::ScrollRight => Some(TabBarAction::ScrollRight),
            TabHit::Dropdown => Some(TabBarAction::OpenOverflowMenu),
        }
    }

    /// Pixel-space mouse move: update hovered tab index.
    pub fn on_mouse_move_px(&mut self, px: f32, py: f32) {
        self.hovered_index = self.layout.as_ref().and_then(|layout| {
            layout.tabs.iter().find(|e| e.rect_px.contains(px, py)).map(|e| e.index)
        });
    }

    /// Generate DrawList from layout (px path).
    pub fn to_drawlist(
        &self,
        active_index: usize,
        theme: &Theme,
        dpi: f32,
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
    ) {
        let Some(layout) = &self.layout else { return };
        if layout.tabs.is_empty() {
            return;
        }

        let tab_h = tab_bar_height(dpi);
        let bar_bg = darken_color(theme.palette.bg_surface, 0.85);
        let hovered = self.hovered_index;
        let font_size = 15.0 * dpi;
        let baseline = tab_h * 0.5 + font_size * 0.25;
        let active_fg = theme.editor.foreground;
        let inactive_fg =
            [active_fg[0] * 0.48, active_fg[1] * 0.48, active_fg[2] * 0.48, active_fg[3]];

        // Full-width bar background
        if let Some(first) = layout.tabs.first() {
            let w = layout.new_tab_rect_px.right().max(first.rect_px.right());
            dl.fill(Rect::new(0.0, first.rect_px.bottom() - tab_h, w, tab_h), bar_bg);
        }

        // Right button area background (darker to distinguish from tab area)
        let btn_area_x = layout.dropdown_rect_px.x - 4.0 * dpi;
        let btn_area_w = layout.new_tab_rect_px.right() + 6.0 * dpi - btn_area_x;
        dl.fill(
            Rect::new(btn_area_x, 0.0, btn_area_w, tab_h),
            darken_color(theme.palette.bg_surface, 0.75),
        );

        // ── Fixed areas: arrows, dropdown, new tab button (outside clip) ──

        // New tab "+"
        {
            let r = layout.new_tab_rect_px;
            let cx = r.x + r.w * 0.5;
            let cy = r.y + r.h * 0.5;
            let half = r.w.min(r.h) * 0.25;
            let lw = 2.0 * dpi;
            dl.fill(Rect::new(cx - lw * 0.5, cy - half, lw, half * 2.0), theme.editor.foreground);
            dl.fill(Rect::new(cx - half, cy - lw * 0.5, half * 2.0, lw), theme.editor.foreground);
        }

        // Dropdown chevron (filled triangle pointing down)
        if layout.dropdown_rect_px.w > 0.0 && !layout.dropdown_rect_px.w.is_nan() {
            let r = layout.dropdown_rect_px;
            let cx = r.x + r.w * 0.5;
            let cy = r.y + r.h * 0.5;
            let s = r.w.min(r.h) * 0.28;
            dl.fill_triangle(
                [cx - s, cy - s * 0.5],
                [cx + s, cy - s * 0.5],
                [cx, cy + s * 0.5],
                theme.editor.foreground,
            );
        }

        // Overflow scroll arrows
        let arrow_color =
            if layout.overflow { theme.editor.foreground } else { [0.3, 0.3, 0.33, 0.5] };
        // Left arrow (filled triangle pointing left)
        let la_color = if layout.left_arrow_disabled { [0.3, 0.3, 0.33, 0.3] } else { arrow_color };
        {
            let r = layout.overflow_left_rect_px;
            if r.w > 0.0 {
                let cx = r.x + r.w * 0.5;
                let cy = r.y + r.h * 0.5;
                let s = r.w.min(r.h) * 0.3;
                dl.fill_triangle([cx - s, cy], [cx + s, cy - s], [cx + s, cy + s], la_color);
            }
        }
        // Right arrow (filled triangle pointing right)
        let ra_color =
            if layout.right_arrow_disabled { [0.3, 0.3, 0.33, 0.3] } else { arrow_color };
        {
            let r = layout.overflow_right_rect_px;
            if r.w > 0.0 {
                let cx = r.x + r.w * 0.5;
                let cy = r.y + r.h * 0.5;
                let s = r.w.min(r.h) * 0.3;
                dl.fill_triangle([cx + s, cy], [cx - s, cy - s], [cx - s, cy + s], ra_color);
            }
        }

        // ── Tab rendering (split into pinned / non-pinned clips) ──
        let last_pinned_pos = layout.tabs.iter().rposition(|t| t.pinned);

        // Pinned area clip: content always visible (no fade gradient)
        if let Some(lp) = last_pinned_pos {
            let pinned_right = layout.tabs[lp].rect_px.right() + 1.0 * dpi;
            let pinned_clip =
                Rect::new(layout.clip_left_px, 0.0, pinned_right - layout.clip_left_px, tab_h);
            dl.clip(pinned_clip, |dl| {
                let pinned_tabs: Vec<_> = layout.tabs.iter().filter(|t| t.pinned).collect();
                for (i, entry) in pinned_tabs.iter().enumerate() {
                    let is_active = entry.index == active_index;
                    draw_tab_bg(dl, entry, is_active, theme, dpi);
                    draw_tab_content(
                        dl,
                        entry,
                        is_active,
                        hovered,
                        theme,
                        dpi,
                        font_size,
                        baseline,
                        active_fg,
                        inactive_fg,
                        shaper.as_deref_mut(),
                    );
                    // Separator between adjacent pinned tabs
                    if i + 1 < pinned_tabs.len() {
                        let next = pinned_tabs[i + 1];
                        if !is_active && next.index != active_index {
                            let sep_x = entry.rect_px.right();
                            let sep_w = 1.0 * dpi;
                            let sep_h = entry.rect_px.h * 0.4;
                            let sep_y = entry.rect_px.y + (entry.rect_px.h - sep_h) * 0.5;
                            dl.fill(
                                Rect::new(sep_x, sep_y, sep_w, sep_h),
                                darken_color(theme.editor.gutter_bg, 0.7),
                            );
                        }
                    }
                    // Pin group separator (after last pinned tab)
                    if i + 1 == pinned_tabs.len() && entry.index + 1 < layout.tabs.len() {
                        let sep_x = entry.rect_px.right() + 1.0 * dpi;
                        let sep_w = 2.0 * dpi;
                        let sep_y = entry.rect_px.y + 4.0 * dpi;
                        let sep_h = entry.rect_px.h - 8.0 * dpi;
                        dl.fill(Rect::new(sep_x, sep_y, sep_w, sep_h), [0.4, 0.4, 0.45, 0.6]);
                    }
                }
            });
        }

        // Non-pinned area clip
        {
            let pinned_right = last_pinned_pos
                .map(|lp| layout.tabs[lp].rect_px.right() + 1.0 * dpi)
                .unwrap_or(layout.clip_left_px);
            let np_clip = Rect::new(pinned_right, 0.0, layout.clip_right_px - pinned_right, tab_h);
            dl.clip(np_clip, |dl| {
                let non_pinned: Vec<_> = layout.tabs.iter().filter(|t| !t.pinned).collect();
                for (i, entry) in non_pinned.iter().enumerate() {
                    let is_active = entry.index == active_index;
                    draw_tab_bg(dl, entry, is_active, theme, dpi);
                    draw_tab_content(
                        dl,
                        entry,
                        is_active,
                        hovered,
                        theme,
                        dpi,
                        font_size,
                        baseline,
                        active_fg,
                        inactive_fg,
                        shaper.as_deref_mut(),
                    );
                    // Separator between adjacent non-pinned tabs
                    if i + 1 < non_pinned.len() {
                        let next = non_pinned[i + 1];
                        if !is_active && next.index != active_index {
                            let sep_x = entry.rect_px.right();
                            let sep_w = 1.0 * dpi;
                            let sep_h = entry.rect_px.h * 0.4;
                            let sep_y = entry.rect_px.y + (entry.rect_px.h - sep_h) * 0.5;
                            dl.fill(
                                Rect::new(sep_x, sep_y, sep_w, sep_h),
                                darken_color(theme.editor.gutter_bg, 0.7),
                            );
                        }
                    }
                }
            });
        }
    }
}

/// Draw tab background (called inside clip block).
fn draw_tab_bg(
    dl: &mut DrawList,
    entry: &super::layout::TabEntry,
    is_active: bool,
    theme: &Theme,
    dpi: f32,
) {
    if is_active {
        // Active tab: floating rounded pill/card look
        let bg = theme.editor.background;
        let mut r = entry.rect_px;
        // Make it float slightly if Claude theme (or just generally)
        let pad_v = 4.0 * dpi;
        let pad_h = 2.0 * dpi;
        r.y += pad_v;
        r.h -= pad_v * 1.5;
        r.x += pad_h;
        r.w -= pad_h * 2.0;

        // Shadow/border
        let shadow_rect = Rect::new(r.x, r.y + 1.0, r.w, r.h);
        dl.fill_rounded(shadow_rect, [0.0, 0.0, 0.0, 0.04], 6.0 * dpi);
        dl.fill_rounded(r, bg, 6.0 * dpi);

        // No accent rect needed for rounded card style, but we can leave a subtle one if desired.
    } else {
        let bg = if entry.preview {
            darken_color(theme.editor.gutter_bg, 0.87)
        } else {
            darken_color(theme.editor.gutter_bg, 0.9)
        };
        dl.fill(entry.rect_px, bg);
    }
}

/// Draw tab content: pin indicator, close button, separators, dirty mark, text label.
#[allow(
    clippy::too_many_arguments,
    reason = "tab paint helper receives already-derived geometry and semantic colors"
)]
fn draw_tab_content(
    dl: &mut DrawList,
    entry: &super::layout::TabEntry,
    is_active: bool,
    hovered: Option<usize>,
    theme: &Theme,
    dpi: f32,
    font_size: f32,
    baseline: f32,
    active_fg: [f32; 4],
    inactive_fg: [f32; 4],
    mut shaper: Option<&mut shaping::Shaper>,
) {
    use super::layout::TabIndicator;

    // Pin indicator "|"
    if entry.pinned {
        let bar_w = 2.0 * dpi;
        let bar_h = entry.rect_px.h * 0.45;
        let bar_x = entry.rect_px.x + 6.0 * dpi;
        let bar_y = entry.rect_px.y + (entry.rect_px.h - bar_h) * 0.5;
        dl.fill(Rect::new(bar_x, bar_y, bar_w, bar_h), [0.4, 0.55, 0.8, 0.8]);
    }

    // Close button "x" on hover (non-pinned only)
    if !entry.pinned && hovered == Some(entry.index) {
        let cb = entry.close_rect_px;
        let cx = cb.x + cb.w * 0.5;
        let cy = cb.y + cb.h * 0.5;
        let x_font_size = 10.0 * dpi;
        let x_baseline = cy + x_font_size * 0.3;
        if let Some(ref mut shaper) = shaper {
            dl.text_shaped(
                cx - x_font_size * 0.3,
                x_baseline,
                x_font_size,
                [0.25, 0.25, 0.28, 0.95],
                "x",
                shaper,
            );
        };
    }

    // Dirty indicator
    if entry.indicator == TabIndicator::Dirty || entry.indicator == TabIndicator::Conflict {
        let pin_offset = if entry.pinned { 6.0 * dpi } else { 0.0 };
        let ind_c = if entry.indicator == TabIndicator::Dirty {
            theme.editor.cursor
        } else {
            [1.0, 0.75, 0.0, 1.0]
        };
        let ind_ch = if entry.indicator == TabIndicator::Dirty { "*" } else { "!" };
        let ind_x = entry.rect_px.x + 6.0 * dpi + pin_offset;
        if let Some(ref mut shaper) = shaper {
            dl.text_shaped(ind_x, entry.rect_px.y + baseline, font_size, ind_c, ind_ch, shaper);
        };
    }

    // Text label
    let fg = if is_active { active_fg } else { inactive_fg };
    let indicator_pad = if entry.indicator != TabIndicator::None { 14.0 * dpi } else { 0.0 };
    let base_pad = 10.0 * dpi;
    let x = entry.rect_px.x + base_pad + indicator_pad;
    if let Some(ref mut shaper) = shaper {
        dl.text_shaped(x, entry.rect_px.y + baseline, font_size, fg, &entry.title, shaper);
    };
}

/// Darken a color by a factor (0..1).
fn darken_color(c: [f32; 4], factor: f32) -> [f32; 4] {
    [c[0] * factor, c[1] * factor, c[2] * factor, c[3]]
}
