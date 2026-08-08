//! PopupMenu types, construction, hit-test and paint logic.
//!
//! Merged from old `ui/src/popup_menu.rs` into `widgets::popup_menu::types`.

use crate::core::geom::Rect;
use crate::core::widget::PaintCtx;
use crate::tab_bar::truncate_title_by_width;
use crate::view_mode::ViewMode;

/// Context menu action for right-click on a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    Close,
    CloseOthers,
    CloseRight,
    CloseAll,
    CopyPath,
    TogglePin,
}

// ── Unified Popup Menu ──

/// Action produced by a popup menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupMenuAction {
    /// Switch to the tab at the given index (overflow menu).
    SwitchTab(usize),
    /// Context menu action on a specific tab.
    Context { action: ContextMenuAction, tab_index: usize },
    /// Switch view mode (sidebar vs tabs).
    SetViewMode(ViewMode),
    /// Open the settings.yaml file.
    OpenSettingsFile,
    /// Toggle line number display.
    ToggleLineNumbers,
    /// Toggle word wrap.
    ToggleWordWrap,
    /// Toggle status bar visibility.
    ToggleStatusBar,
    /// Set theme mode (System / Dark / Light).
    SetThemeMode(crate::settings::ThemeMode),
    /// Create a typed untitled document.
    NewDocument(crate::sidebar::types::NewDocumentKind),
}

/// A single item in a popup menu.
#[derive(Debug, Clone)]
pub struct PopupMenuItem {
    pub label: String,
    pub is_active: bool,
    pub is_separator: bool,
    pub enabled: bool,
    pub action: PopupMenuAction,
}

impl PopupMenuItem {
    pub fn action(label: impl Into<String>, action: PopupMenuAction) -> Self {
        Self { label: label.into(), is_active: false, is_separator: false, enabled: true, action }
    }

    pub fn separator(action: PopupMenuAction) -> Self {
        Self { label: String::new(), is_active: false, is_separator: true, enabled: false, action }
    }

    pub fn with_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn is_selectable(&self) -> bool {
        self.enabled && !self.is_separator
    }
}

/// Overflow 菜单入口数据（px，供 overflow_px 使用）。
#[derive(Debug, Clone)]
pub struct OverflowEntry {
    pub tab_index: usize,
    pub title: String,
}

/// Unified popup menu.  Overflow menu and right-click context menu are
/// two instances of the same type, just anchored differently.
#[derive(Debug, Clone)]
pub struct PopupMenu {
    pub items: Vec<PopupMenuItem>,
    /// px rects for each item, top to bottom.
    pub item_rects: Vec<Rect>,
    /// Full menu background rect (px) with padding included.
    pub menu_rect: Rect,
    /// Screen dimensions used for NDC→px conversion (backward compat for hit_test).
    pub screen_size: (f32, f32),
    /// Show checkmark for active items (settings menu) vs highlight only (tab-bar menu).
    pub show_checkmarks: bool,
}

impl PopupMenu {
    // ── 新：px 原生构造函数 ──

    /// Build an overflow (all-tabs) menu anchored below the dropdown button（px）。
    pub fn overflow_px(
        entries: &[OverflowEntry],
        dropdown_rect_px: Rect,
        screen_size: (f32, f32),
        active_index: usize,
        dpi: f32,
    ) -> Self {
        let (sw, sh) = screen_size;
        let item_h = 30.0 * dpi;
        let padding = 4.0 * dpi;
        let menu_w = 200.0 * dpi;
        let menu_font_size = crate::constants::BODY_FONT_SIZE * dpi;
        let menu_max_text_w = (200.0 - 8.0 - 8.0) * dpi;

        let dd_bottom = dropdown_rect_px.bottom();
        let available_h = sh - dd_bottom;
        let max_visible = ((available_h - padding) / item_h).floor() as usize;
        let max_items = max_visible.clamp(5, 40);

        let items: Vec<PopupMenuItem> = entries
            .iter()
            .take(max_items)
            .map(|e| {
                PopupMenuItem::action(
                    truncate_title_by_width(&e.title, menu_max_text_w, menu_font_size),
                    PopupMenuAction::SwitchTab(e.tab_index),
                )
                .with_active(e.tab_index == active_index)
            })
            .collect();

        let n = items.len() as f32;
        let menu_h = item_h * n + padding;
        let dd_right = dropdown_rect_px.right();
        let menu_left = (dd_right - menu_w).max(0.0);
        let menu_right = dd_right.min(sw);
        let menu_top = dd_bottom + 2.0 * dpi;
        let menu_top = if menu_top + menu_h > sh {
            (dd_bottom - menu_h - 2.0 * dpi).max(0.0)
        } else {
            menu_top
        };

        let pad_x = 2.0 * dpi;
        let pad_y = 2.0 * dpi;
        let item_w = (menu_right - menu_left) - pad_x * 2.0;

        let item_rects: Vec<Rect> = (0..items.len())
            .map(|i| {
                let top = menu_top + pad_y + i as f32 * item_h;
                Rect::new(menu_left + pad_x, top, item_w, item_h)
            })
            .collect();

        PopupMenu {
            items,
            item_rects,
            menu_rect: Rect::new(menu_left, menu_top, menu_right - menu_left, menu_h),
            screen_size,
            show_checkmarks: false,
        }
    }

    /// Build a right-click context menu anchored at the given px position.
    pub fn context_px(
        tab_index: usize,
        px_pos: (f32, f32),
        screen_size: (f32, f32),
        is_pinned: bool,
        dpi: f32,
    ) -> Self {
        let (sw, sh) = screen_size;
        let (px, py) = px_pos;
        let item_h = 30.0 * dpi;
        let _padding = 4.0 * dpi;
        let menu_w = 200.0 * dpi;

        let pin_label = if is_pinned { "取消固定" } else { "固定标签" };
        let items = vec![
            PopupMenuItem::action(
                "关闭",
                PopupMenuAction::Context { action: ContextMenuAction::Close, tab_index },
            ),
            PopupMenuItem::action(
                "关闭其他",
                PopupMenuAction::Context { action: ContextMenuAction::CloseOthers, tab_index },
            ),
            PopupMenuItem::action(
                "关闭右侧",
                PopupMenuAction::Context { action: ContextMenuAction::CloseRight, tab_index },
            ),
            PopupMenuItem::action(
                "全部关闭",
                PopupMenuAction::Context { action: ContextMenuAction::CloseAll, tab_index },
            ),
            PopupMenuItem::separator(PopupMenuAction::Context {
                action: ContextMenuAction::CloseAll,
                tab_index,
            }),
            PopupMenuItem::action(
                "复制路径",
                PopupMenuAction::Context { action: ContextMenuAction::CopyPath, tab_index },
            ),
            PopupMenuItem::action(
                pin_label,
                PopupMenuAction::Context { action: ContextMenuAction::TogglePin, tab_index },
            ),
        ];

        let menu_left = (px - menu_w * 0.5).max(0.0).min(sw - menu_w);
        let menu_right = (menu_left + menu_w).min(sw);

        let pad_x = 2.0 * dpi;
        let pad_y = 2.0 * dpi;
        let item_w = (menu_right - menu_left) - pad_x * 2.0;
        let sep_h = 8.0 * dpi;

        let mut top_px = pad_y;
        let mut item_rects = Vec::with_capacity(items.len());
        for item in &items {
            let h = if item.is_separator { sep_h } else { item_h };
            item_rects.push(Rect::new(menu_left + pad_x, top_px, item_w, h));
            top_px += h;
        }
        let menu_h = top_px + pad_y;
        let menu_top = if py + menu_h > sh { (py - menu_h).max(0.0) } else { py };
        let offset = menu_top;
        let item_rects: Vec<Rect> =
            item_rects.iter().map(|r| Rect::new(r.x, r.y + offset, r.w, r.h)).collect();

        PopupMenu {
            items,
            item_rects,
            menu_rect: Rect::new(menu_left, menu_top, menu_right - menu_left, menu_h),
            screen_size,
            show_checkmarks: false,
        }
    }

    /// Hit-test in px coordinates.
    pub fn hit_test_px(&self, px: f32, py: f32) -> Option<&PopupMenuAction> {
        for (i, rect) in self.item_rects.iter().enumerate() {
            if rect.contains(px, py) && self.items[i].is_selectable() {
                return Some(&self.items[i].action);
            }
        }
        None
    }

    /// Paint the menu (called by PopupMenuWidget::paint).
    pub fn paint(&self, ctx: &mut PaintCtx, hovered: Option<usize>) {
        let dpi = ctx.dpi;
        let mr = self.menu_rect;
        let radius = 8.0 * dpi;

        // Border (drawn as a larger rounded rect behind the bg)
        let border = 1.0 * dpi;
        let outer =
            Rect::new(mr.x - border, mr.y - border, mr.w + border * 2.0, mr.h + border * 2.0);
        ctx.list.fill_rounded(outer, ctx.theme.palette.border_strong, radius + border);
        // Background
        ctx.list.fill_rounded(mr, ctx.theme.palette.bg_elevated, radius);

        // Items
        for (i, item) in self.items.iter().enumerate() {
            let r = self.item_rects[i];

            let font_size = crate::constants::BODY_FONT_SIZE * dpi;
            let pad_x = r.x + 8.0 * dpi;

            if item.is_separator {
                let sep_y = r.y + r.h * 0.5;
                ctx.list.fill(
                    Rect::new(pad_x, sep_y, r.w - (pad_x - r.x) * 2.0, 1.0),
                    ctx.theme.palette.border_strong,
                );
                continue;
            }

            // Hover highlight (rounded)
            if item.enabled && Some(i) == hovered {
                let hr = r.shrink(1.0 * dpi, 1.0 * dpi, 1.0 * dpi, 1.0 * dpi);
                if hr.w > 0.0 && hr.h > 0.0 {
                    ctx.list.fill_rounded(hr, ctx.theme.palette.sidebar_hover_bg, radius);
                }
            }

            if self.show_checkmarks {
                // Settings menu: checkmark for active items, no background highlight
                let check_x = r.x + 8.0 * dpi;
                let label_x = check_x + font_size * 0.9;
                let y_baseline = r.y + r.h * 0.5 + font_size * 0.35;
                if item.is_active
                    && let Some(ref mut shaper) = ctx.shaper
                {
                    ctx.list.text_shaped(
                        check_x,
                        y_baseline,
                        font_size,
                        if item.enabled {
                            ctx.theme.palette.text_main
                        } else {
                            ctx.theme.palette.text_muted
                        },
                        "\u{2713}",
                        shaper,
                    );
                };
                if let Some(ref mut shaper) = ctx.shaper {
                    ctx.list.text_shaped(
                        label_x,
                        y_baseline,
                        font_size,
                        if item.enabled {
                            ctx.theme.palette.text_main
                        } else {
                            ctx.theme.palette.text_muted
                        },
                        &item.label,
                        shaper,
                    );
                };
            } else {
                // Tab-bar/context menu: background highlight for active items, no checkmark
                if item.is_active && Some(i) != hovered {
                    let hr = r.shrink(1.0 * dpi, 1.0 * dpi, 1.0 * dpi, 1.0 * dpi);
                    if hr.w > 0.0 && hr.h > 0.0 {
                        ctx.list.fill_rounded(hr, ctx.theme.palette.sidebar_active_bg, radius);
                    }
                }
                let y_baseline = r.y + r.h * 0.5 + font_size * 0.35;
                if let Some(ref mut shaper) = ctx.shaper {
                    ctx.list.text_shaped(
                        pad_x,
                        y_baseline,
                        font_size,
                        if item.enabled {
                            ctx.theme.palette.text_main
                        } else {
                            ctx.theme.palette.text_muted
                        },
                        &item.label,
                        shaper,
                    );
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_items_and_separators_are_not_hit_test_targets() {
        let menu = PopupMenu {
            items: vec![
                PopupMenuItem::action("可用", PopupMenuAction::ToggleLineNumbers),
                PopupMenuItem::action("禁用", PopupMenuAction::ToggleWordWrap).with_enabled(false),
                PopupMenuItem::separator(PopupMenuAction::ToggleStatusBar),
            ],
            item_rects: vec![
                Rect::new(0.0, 0.0, 100.0, 20.0),
                Rect::new(0.0, 20.0, 100.0, 20.0),
                Rect::new(0.0, 40.0, 100.0, 8.0),
            ],
            menu_rect: Rect::new(0.0, 0.0, 100.0, 48.0),
            screen_size: (100.0, 100.0),
            show_checkmarks: false,
        };

        assert_eq!(menu.hit_test_px(10.0, 10.0), Some(&PopupMenuAction::ToggleLineNumbers));
        assert_eq!(menu.hit_test_px(10.0, 30.0), None);
        assert_eq!(menu.hit_test_px(10.0, 44.0), None);
    }
}
