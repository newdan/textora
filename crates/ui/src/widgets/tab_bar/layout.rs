//! tab_bar/layout.rs — Tab 布局计算。

use super::text::{compute_text_width, truncate_title_by_width};
use super::types::{TabBarCtx, TabInfo};
use crate::core::geom::Rect;
use shaping::Shaper;
use std::collections::HashMap;
use std::path::Path;

/// Tab status indicator — inspired by Zed's `render_item_indicator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabIndicator {
    None,
    Dirty,
    Conflict,
}

impl TabIndicator {
    pub fn for_doc(dirty: bool, _has_conflict: bool) -> Self {
        // NOTE: conflict not yet exposed by TabInfo;
        // when it is, Conflict takes priority over Dirty.
        if dirty { Self::Dirty } else { Self::None }
    }
}

/// Check if a non-pinned tab at position `x` is within the visible clip region.
/// Used by both hit testing and rendering to avoid duplicate logic.
pub fn is_tab_in_clip(x: f32, layout: &TabBarLayout) -> bool {
    x >= layout.clip_left_px && x <= layout.clip_right_px
}

/// Per-tab layout entry.
#[derive(Debug, Clone)]
pub struct TabEntry {
    pub index: usize,
    pub title: String,
    pub indicator: TabIndicator,
    /// Parent directory shown when filename collides with another tab (e.g. "src/README.md").
    pub disambiguation: Option<String>,
    /// Whether this tab is pinned (stays open, can't be closed with Cmd+W).
    pub pinned: bool,
    /// Whether this is a preview tab (italic/dimmed rendering, auto-closes on switch).
    pub preview: bool,
    /// Tab background rectangle in pixel coordinates
    pub rect_px: Rect,
    /// Close button rectangle in pixel coordinates
    pub close_rect_px: Rect,
}

/// Layout result for the tab bar.
#[derive(Debug, Clone)]
pub struct TabBarLayout {
    pub tabs: Vec<TabEntry>,
    pub overflow: bool,
    pub scroll_offset: f32,
    /// Maximum horizontal scroll (pixels). 0 when all tabs fit.
    pub max_scroll: f32,
    /// Navigation history buttons
    pub nav_buttons: NavButtonLayout,
    /// "+" button rect for new tab (px)
    pub new_tab_rect_px: Rect,
    /// Overflow scroll-left arrow rect (px) — gray when !overflow
    pub overflow_left_rect_px: Rect,
    /// Overflow scroll-right arrow rect (px) — gray when !overflow
    pub overflow_right_rect_px: Rect,
    /// Left clip edge (NDC) — tabs left of this are hidden
    pub clip_left_px: f32,
    /// Right clip edge (NDC) — tabs right of this are hidden
    pub clip_right_px: f32,
    /// Left faded-edge gradient rect (px) — masks tabs on left side
    pub fade_left_rect_px: Rect,
    /// Right faded-edge gradient rect (px) — masks tabs on right side
    pub fade_right_rect_px: Rect,
    /// Dropdown "all tabs" button rect (px) — placed left of the "+" button
    pub dropdown_rect_px: Rect,
    /// Whether left arrow should render disabled (at min scroll)
    pub left_arrow_disabled: bool,
    /// Whether right arrow should render disabled (at max scroll)
    pub right_arrow_disabled: bool,
    /// Total width of pinned tabs (px) — pinned tabs are fixed at left, don't scroll
    pub pinned_total_width: f32,
}

/// Detect filename collisions among open documents and return the
/// parent-directory prefix (if any) for each doc that needs disambiguation.
///
/// When two or more documents share the same filename (e.g. `a/README.md`
/// and `b/README.md`), this function returns `Some("parent_dir")` so the
/// tab can display `a/README.md` / `b/README.md` instead of just `README.md`.
pub fn compute_disambiguation(file_paths: &[Option<&Path>]) -> Vec<Option<String>> {
    // For each path, collect ancestor components bottom-up [filename, parent, grandparent, …]
    let ancestors: Vec<Vec<String>> = file_paths
        .iter()
        .map(|path| {
            let mut comps = Vec::new();
            if let Some(p) = path {
                if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
                    comps.push(fname.to_string());
                }
                let mut cur = p.parent();
                while let Some(parent) = cur {
                    if let Some(name) = parent.file_name().and_then(|n| n.to_str()) {
                        comps.push(name.to_string());
                    }
                    cur = parent.parent();
                }
            }
            comps
        })
        .collect();

    // Compute the deepest full-path display for each path (used as fallback)
    let deepest: Vec<String> = ancestors
        .iter()
        .map(|comps| comps.iter().rev().cloned().collect::<Vec<_>>().join("/"))
        .collect();

    let max_depth = ancestors.iter().map(|a| a.len()).max().unwrap_or(0);
    let mut result: Vec<Option<String>> = vec![None; file_paths.len()];

    for detail in 0..max_depth {
        // Build display strings at this detail level
        let display: Vec<Option<String>> = ancestors
            .iter()
            .map(|comps| {
                if detail >= comps.len() {
                    None
                } else {
                    Some(comps[..=detail].iter().rev().cloned().collect::<Vec<_>>().join("/"))
                }
            })
            .collect();

        // Count occurrences among unresolved entries
        let mut display_counts: HashMap<String, usize> = HashMap::new();
        for (i, d) in display.iter().enumerate() {
            if result[i].is_none() {
                let key = d.clone().unwrap_or_else(|| deepest[i].clone());
                *display_counts.entry(key).or_default() += 1;
            }
        }

        // Resolve entries that are now unique at this detail level
        for (i, d) in display.iter().enumerate() {
            if result[i].is_some() {
                continue;
            }
            let key = d.clone().unwrap_or_else(|| deepest[i].clone());
            if display_counts.get(&key).copied().unwrap_or(1) == 1 {
                if detail == 0 {
                    result[i] = Some(String::new());
                } else {
                    let prefix = ancestors[i][1..=detail]
                        .iter()
                        .rev()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("/");
                    result[i] = Some(prefix);
                }
            }
        }

        if result.iter().all(|r| r.is_some()) {
            break;
        }
    }

    // Convert empty strings back to None
    result.into_iter().map(|r| r.filter(|s| !s.is_empty())).collect()
}

/// Layout for navigation history buttons (back/forward arrows).
#[derive(Debug, Clone, Copy)]
pub struct NavButtonLayout {
    pub back_rect_px: Rect,
    pub forward_rect_px: Rect,
    pub back_enabled: bool,
    pub forward_enabled: bool,
}

/// Maximum horizontal scroll distance for the tab bar (in pixels).
/// Returns 0 if all tabs fit within the available width.
pub fn max_tab_scroll(doc_count: usize, ctx: &TabBarCtx, _tab_height: f32) -> f32 {
    if doc_count == 0 {
        return 0.0;
    }
    let gap = 2.0 * ctx.dpi;
    let max_tab_w = 310.0 * ctx.dpi;
    let nav_area_w = (32.0 + 24.0) * ctx.dpi; // nav buttons 48px + plus btn 24px
    let available = ctx.screen_w - gap - nav_area_w;
    let tab_width = max_tab_w.min((available - gap) / 1.0);
    let total_width = doc_count as f32 * (tab_width + gap) + gap;
    (total_width - available).max(0.0)
}

/// Mark a specific tab as preview (or clear all preview marks).
pub fn set_preview_tab(layout: &mut TabBarLayout, index: Option<usize>) {
    for entry in &mut layout.tabs {
        entry.preview = Some(entry.index) == index;
    }
}

/// Clamp a scroll offset to valid range.
pub fn clamp_tab_scroll(offset: f32, max_scroll: f32) -> f32 {
    offset.clamp(0.0, max_scroll)
}

/// Compute tab bar layout from document list.
pub fn layout_tabs(
    tab_infos: &[TabInfo],
    _active_index: usize,
    ctx: &TabBarCtx,
    _tab_height: f32,
    _back_enabled: bool,
    _forward_enabled: bool,
    scroll_offset: f32,
    mut shaper: Option<&mut Shaper>,
) -> TabBarLayout {
    if tab_infos.is_empty() {
        return TabBarLayout {
            tabs: Vec::new(),
            overflow: false,
            scroll_offset: 0.0,
            nav_buttons: NavButtonLayout {
                back_rect_px: Rect::ZERO,
                forward_rect_px: Rect::ZERO,
                back_enabled: false,
                forward_enabled: false,
            },
            new_tab_rect_px: Rect::ZERO,
            max_scroll: 0.0,
            clip_left_px: 0.0,
            clip_right_px: 0.0,
            fade_left_rect_px: Rect::ZERO,
            fade_right_rect_px: Rect::ZERO,
            dropdown_rect_px: Rect::ZERO,
            overflow_left_rect_px: Rect::ZERO,
            overflow_right_rect_px: Rect::ZERO,
            left_arrow_disabled: false,
            right_arrow_disabled: false,
            pinned_total_width: 0.0,
        };
    }

    let min_tab_w = 40.0 * ctx.dpi;
    let pinned_min_tab_w = 30.0 * ctx.dpi;
    let max_tab_w = 310.0 * ctx.dpi;
    let pinned_max_tab_w = 160.0 * ctx.dpi;
    let gap = 2.0 * ctx.dpi;

    // Reserve areas: overflow arrows (left) ... tabs ... dropdown | + button (right)
    let icon_btn_w = 20.0 * ctx.dpi; // unified button size for dropdown and "+" buttons
    let right_reserved = icon_btn_w + 2.0 * ctx.dpi + icon_btn_w; // dropdown + gap + plus
    let overflow_arrow_w = 10.0 * ctx.dpi;
    let overflow_pad = 2.0 * ctx.dpi; // gap around each arrow
    let left_margin = 4.0 * ctx.dpi; // small margin from left edge
    let arrows_area = left_margin + overflow_arrow_w * 2.0 + overflow_pad * 2.0; // margin + pad + arrow + pad + arrow
    // Available width for tabs
    let available = ctx.screen_w - arrows_area - gap - right_reserved;

    // Font size for tab text (consistent with app.rs rendering)
    let font_size = 15.0 * ctx.dpi;

    // Compute disambiguation for duplicate filenames
    let file_paths: Vec<Option<&Path>> =
        tab_infos.iter().map(|tab| tab.file_path.as_deref()).collect();
    let disambigs = compute_disambiguation(&file_paths);

    // Sort doc indices: pinned tabs first, then unpinned (stable relative order)
    let mut sorted_indices: Vec<usize> = (0..tab_infos.len()).collect();
    sorted_indices.sort_by(|a, b| {
        let a_pinned = tab_infos[*a].pinned;
        let b_pinned = tab_infos[*b].pinned;
        b_pinned.cmp(&a_pinned) // pinned first
    });

    // Phase 1: compute titles and per-tab widths
    struct TabWidth {
        index: usize,
        title: String,
        disambig: Option<String>,
        indicator: TabIndicator,
        pinned: bool,
        width_px: f32,
    }
    // title_pad in text_positions is (24*dpi).min(28), so base_pad = title_pad * 10/24
    let title_pad_cap = (24.0 * ctx.dpi).min(28.0);
    let pad_x = title_pad_cap * 10.0 / 24.0; // matches base_pad in text_positions
    let close_area = 20.0 * ctx.dpi; // close button + right margin
    let pinned_right_pad = 12.0 * ctx.dpi;
    let mut tab_widths: Vec<TabWidth> = Vec::with_capacity(tab_infos.len());

    for &i in &sorted_indices {
        let tab = &tab_infos[i];
        let base_title = tab.title.clone();
        let disambig = disambigs.get(i).and_then(|d| d.as_ref().map(|s| s.to_owned()));
        let title = if let Some(ref parent) = disambig {
            format!("{}/{}", parent, base_title)
        } else {
            base_title
        };
        let max_text_w = 20.0 * font_size; // 15 wide chars worth
        let title = truncate_title_by_width(&title, max_text_w, font_size);
        let text_w = compute_text_width(&title, font_size, shaper.as_deref_mut());
        let indicator = TabIndicator::for_doc(tab.is_dirty, false);
        let indicator_pad =
            if indicator != TabIndicator::None { title_pad_cap * 14.0 / 24.0 } else { 0.0 };
        let is_pinned = tab_infos[i].pinned;
        let (right_pad, eff_min, eff_max) = if is_pinned {
            (pinned_right_pad, pinned_min_tab_w, pinned_max_tab_w)
        } else {
            (close_area, min_tab_w, max_tab_w)
        };
        let width_px = (pad_x + indicator_pad + text_w + right_pad).max(eff_min).min(eff_max);

        tab_widths.push(TabWidth {
            index: i,
            title,
            disambig: disambig.map(|s| s.to_string()),
            indicator,
            pinned: is_pinned,
            width_px,
        });
    }

    // Phase 2: compute total width and overflow
    // Pinned tabs are fixed at left; only non-pinned tabs participate in scrolling
    let pinned_width: f32 =
        tab_widths.iter().filter(|tw| tw.pinned).map(|tw| tw.width_px + gap).sum::<f32>();
    let non_pinned_width: f32 =
        tab_widths.iter().filter(|tw| !tw.pinned).map(|tw| tw.width_px + gap).sum::<f32>();
    let pinned_total_width = if pinned_width > 0.0 { pinned_width - gap } else { 0.0 };
    let scrollable_area = (available - pinned_total_width).max(0.0);
    let overflow =
        non_pinned_width > scrollable_area || pinned_width + non_pinned_width > available;
    let max_scroll =
        if non_pinned_width > 0.0 { (non_pinned_width - scrollable_area).max(0.0) } else { 0.0 };
    let clamped_offset = scroll_offset.min(max_scroll);

    // Phase 3: layout each tab with individual widths
    // Pinned tabs are fixed at left (no scroll offset), non-pinned tabs scroll
    let tab_area_start = arrows_area + gap;
    let mut tabs = Vec::with_capacity(tab_infos.len());

    // Helper closure to create a tab entry at position x
    let make_entry = |x: f32, tw: &TabWidth| -> TabEntry {
        let close_btn_w = 12.0 * ctx.dpi;
        let close_btn_h = 12.0 * ctx.dpi;
        let close_margin = 4.0 * ctx.dpi;
        let close_right_px = x + tw.width_px - close_margin;
        let close_left_px = close_right_px - close_btn_w;
        let close_top_px = (_tab_height - close_btn_h) * 0.5;
        TabEntry {
            index: tw.index,
            title: tw.title.clone(),
            indicator: tw.indicator,
            disambiguation: tw.disambig.clone(),
            pinned: tw.pinned,
            preview: false,
            rect_px: Rect { x, y: 0.0, w: tw.width_px, h: _tab_height },
            close_rect_px: Rect {
                x: close_left_px,
                y: close_top_px,
                w: close_btn_w,
                h: close_btn_h,
            },
        }
    };

    // First pass: layout pinned tabs at fixed positions
    let mut x = tab_area_start;
    for tw in tab_widths.iter().filter(|tw| tw.pinned) {
        tabs.push(make_entry(x, tw));
        x += tw.width_px + gap;
    }

    // Second pass: layout non-pinned tabs with scroll offset
    let mut x = tab_area_start + pinned_total_width - clamped_offset;
    for tw in tab_widths.iter().filter(|tw| !tw.pinned) {
        tabs.push(make_entry(x, tw));
        x += tw.width_px + gap;
    }

    // "+" button (px) — same size as dropdown, near right edge
    let btn_left_px = ctx.screen_w - icon_btn_w;
    let btn_top_px = (_tab_height - icon_btn_w) * 0.5;
    let new_tab_rect_px = Rect { x: btn_left_px, y: btn_top_px, w: icon_btn_w, h: icon_btn_w };

    // Navigation buttons removed — use keyboard shortcuts instead
    let nav_buttons = NavButtonLayout {
        back_rect_px: Rect::ZERO,
        forward_rect_px: Rect::ZERO,
        back_enabled: false,
        forward_enabled: false,
    };

    // Overflow scroll arrows (px)
    let overflow_arrow_h = _tab_height + 8.0 * ctx.dpi;
    let overflow_top_px = -4.0 * ctx.dpi;
    let arrows_group_w = overflow_arrow_w + overflow_pad + overflow_arrow_w;
    let arrow_base_px = (arrows_area - arrows_group_w) * 0.5;
    let left_arrow_left_px = arrow_base_px;
    let left_arrow_right_px = arrow_base_px + overflow_arrow_w;
    let right_arrow_left_px = left_arrow_right_px + overflow_pad;
    let _right_arrow_right_px = right_arrow_left_px + overflow_arrow_w;

    let clip_left_px = arrows_area;
    let clip_right_px = ctx.screen_w - right_reserved;

    // clip_left_px / clip_right_px stored in px (was NDC before Phase 9)

    // Faded edge gradient masks (px)
    let fade_width_px = 24.0 * ctx.dpi;
    let fade_left_rect_px = Rect { x: clip_left_px, y: 0.0, w: fade_width_px, h: _tab_height };
    let fade_right_rect_px =
        Rect { x: clip_right_px - fade_width_px, y: 0.0, w: fade_width_px, h: _tab_height };

    // Dropdown button (px)
    let dropdown_gap = 2.0 * ctx.dpi;
    let dd_right_px = btn_left_px - dropdown_gap;
    let dd_left_px = dd_right_px - icon_btn_w;
    let dd_top_px = (_tab_height - icon_btn_w) * 0.5;
    let dropdown_rect_px = Rect { x: dd_left_px, y: dd_top_px, w: icon_btn_w, h: icon_btn_w };

    // Arrow disabled states
    let left_arrow_disabled = clamped_offset <= 0.0;
    let right_arrow_disabled = clamped_offset >= max_scroll;

    TabBarLayout {
        tabs,
        overflow,
        scroll_offset: clamped_offset,
        max_scroll,
        nav_buttons,
        new_tab_rect_px,
        overflow_left_rect_px: Rect {
            x: left_arrow_left_px,
            y: overflow_top_px,
            w: overflow_arrow_w,
            h: overflow_arrow_h,
        },
        overflow_right_rect_px: Rect {
            x: right_arrow_left_px,
            y: overflow_top_px,
            w: overflow_arrow_w,
            h: overflow_arrow_h,
        },
        clip_left_px,
        clip_right_px,
        fade_left_rect_px,
        fade_right_rect_px,
        dropdown_rect_px,
        left_arrow_disabled,
        right_arrow_disabled,
        pinned_total_width,
    }
}
