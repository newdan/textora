//! SidebarState — main state machine for sidebar.

use std::time::{Duration, Instant};

use super::layout::*;
use super::menu::build_new_document_menu;
use super::persistent::SidebarPersistent;
use super::types::*;
use crate::constants;
use crate::core::{PaintCtx, Rect};
use crate::view_mode::ViewMode;
use crate::widgets::icon::draw_icon;
use crate::widgets::popup_menu::{PopupMenu, PopupMenuAction as PMA, PopupMenuItem};
use crate::widgets::split_button::SPLIT_BUTTON_MENU_WIDTH_LOGICAL;

struct EdgeDragState {
    start_px: f32,
    start_width: f32,
}

#[derive(Default)]
pub struct SidebarState {
    visibility: Visibility,
    layout: Option<SidebarLayout>,
    open_menu: Option<PopupMenu>,
    menu_hovered_index: Option<usize>,
    hovered_index: Option<usize>,
    list_scroll_offset: f32,
    drag: Option<EdgeDragState>,
    hover_enter_at: Option<Instant>,
    hover_leave_at: Option<Instant>,
    hover_peek_start: Option<Instant>,
    hover_peek_leave_start: Option<Instant>,
    pub(crate) hovered_button: SidebarHoverButton,
    suppress_hover_enter: bool,
}

impl SidebarState {
    pub fn new(cfg: &SidebarConfig) -> Self {
        let visibility = if cfg.pinned { Visibility::Pinned } else { Visibility::Hidden };
        Self { visibility, ..Self::default() }
    }

    pub fn visibility(&self) -> Visibility {
        self.visibility
    }
    pub fn set_visibility(&mut self, v: Visibility) {
        self.visibility = v;
        // 离开 hover 态时清理动画状态
        if !matches!(v, Visibility::HoverPeek | Visibility::HoverPeekFadingOut) {
            self.hover_peek_start = None;
            self.hover_peek_leave_start = None;
        }
    }
    pub fn current_layout(&self) -> Option<&SidebarLayout> {
        self.layout.as_ref()
    }
    pub fn open_menu(&self) -> Option<&PopupMenu> {
        self.open_menu.as_ref()
    }
    pub fn set_open_menu(&mut self, menu: Option<PopupMenu>) {
        self.open_menu = menu;
        self.menu_hovered_index = None;
    }
    pub fn update_menu_hover(&mut self, px: f32, py: f32) {
        if let Some(ref menu) = self.open_menu {
            self.menu_hovered_index = menu
                .item_rects
                .iter()
                .enumerate()
                .find(|(i, r)| r.contains(px, py) && !menu.items[*i].is_separator)
                .map(|(i, _)| i);
        }
    }
    pub fn menu_hovered_index(&self) -> Option<usize> {
        self.menu_hovered_index
    }
    pub fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }
    pub fn hover_peek_start(&self) -> Option<Instant> {
        self.hover_peek_start
    }

    pub fn hover_peek_leave_start(&self) -> Option<Instant> {
        self.hover_peek_leave_start
    }

    pub fn on_scroll(
        &mut self,
        delta_px: f32,
        total_tabs: usize,
        _metrics: &crate::settings::UiMetrics,
    ) {
        let dpi = 1.0;
        let row_h = ROW_H * dpi;
        let visible_h =
            self.layout.as_ref().map(|l| l.list_clip.h).filter(|&h| h > 0.0).unwrap_or(400.0 * dpi);
        self.list_scroll_offset += delta_px;
        self.clamp_scroll(total_tabs, row_h, visible_h);
    }
    pub fn set_hovered_index(&mut self, idx: Option<usize>) {
        self.hovered_index = idx;
    }
    pub fn list_scroll_offset(&self) -> f32 {
        self.list_scroll_offset
    }
    pub fn set_list_scroll_offset(&mut self, off: f32) {
        self.list_scroll_offset = off;
    }

    pub fn to_persistent(&self) -> SidebarPersistent {
        SidebarPersistent {
            visibility: self.visibility,
            hovered_index: self.hovered_index,
            list_scroll_offset: self.list_scroll_offset,
            open_menu: self.open_menu.clone(),
            hover_enter_at: self.hover_enter_at,
            hover_leave_at: self.hover_leave_at,
            hover_peek_start: self.hover_peek_start,
            hover_peek_leave_start: self.hover_peek_leave_start,
            hovered_button: self.hovered_button,
            suppress_hover_enter: self.suppress_hover_enter,
            settings_btn_rect: self
                .layout
                .as_ref()
                .map(|l| l.settings_btn_rect)
                .unwrap_or(crate::core::Rect::ZERO),
        }
    }

    pub fn restore_from_persistent(&mut self, p: &SidebarPersistent) {
        // Restore visibility from persistent (may differ from cfg.pinned during hover transitions)
        self.visibility = p.visibility;
        self.hovered_index = p.hovered_index;
        self.list_scroll_offset = p.list_scroll_offset;
        self.open_menu = p.open_menu.clone();
        self.hover_enter_at = p.hover_enter_at;
        self.hover_leave_at = p.hover_leave_at;
        self.hover_peek_start = p.hover_peek_start;
        self.hover_peek_leave_start = p.hover_peek_leave_start;
        self.suppress_hover_enter = p.suppress_hover_enter;
        // hovered_button 是帧内 transient 状态，由 on_mouse_move 驱动；
        // 不从 persistent 恢复，否则每帧 inject 后会覆盖掉事件分发刚设好的值。
    }

    /// Clamp scroll offset to valid range.
    pub fn clamp_scroll(&mut self, total_items: usize, row_h: f32, visible_h: f32) {
        let max_scroll = ((total_items as f32) * row_h - visible_h).max(0.0);
        self.list_scroll_offset = self.list_scroll_offset.clamp(0.0, max_scroll);
    }

    pub fn current_width(&self, cfg: &SidebarConfig) -> f32 {
        match self.visibility {
            Visibility::Hidden => 0.0,
            Visibility::HoverPeek | Visibility::HoverPeekFadingOut | Visibility::Pinned => {
                cfg.width
            }
        }
    }

    pub fn editor_left_offset(
        &self,
        cfg: &SidebarConfig,
        _metrics: &crate::settings::UiMetrics,
    ) -> f32 {
        match self.visibility {
            Visibility::Pinned => cfg.width,
            _ => 0.0,
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self.visibility, Visibility::Hidden)
    }
}

// ── Layout constants ──

const HEADER_H: f32 = constants::TITLE_BAR_HEIGHT;
const ROW_H: f32 = constants::ROW_HEIGHT;
const NEW_BTN_H: f32 = constants::ROW_HEIGHT;
const SETTINGS_BTN_H: f32 = constants::ROW_HEIGHT;
const PADDING: f32 = 6.0;
const EDGE_RESIZE_W: f32 = 4.0;
const MINIMUM_EDITOR_WIDTH_LOGICAL: f32 = 100.0;

impl SidebarState {
    pub fn on_drag_start(
        &mut self,
        px: f32,
        _py: f32,
        cfg: &SidebarConfig,
        screen_w: f32,
        metrics: &crate::settings::UiMetrics,
    ) -> bool {
        if !self.is_visible() {
            return false;
        }
        let edge = cfg.width;
        let band = 4.0 * metrics.dpi;
        if (px - edge).abs() <= band && px < screen_w {
            self.drag = Some(EdgeDragState { start_px: px, start_width: cfg.width });
            true
        } else {
            false
        }
    }

    pub fn on_drag(
        &mut self,
        px: f32,
        _py: f32,
        cfg: &mut SidebarConfig,
        _metrics: &crate::settings::UiMetrics,
    ) -> Option<SidebarAction> {
        let drag = self.drag.as_ref()?;
        let dpi = 1.0;
        let mut new_w = drag.start_width + (px - drag.start_px);
        let lo = 160.0 * dpi;
        let hi = 400.0 * dpi;
        new_w = new_w.clamp(lo, hi);
        cfg.width = new_w;
        Some(SidebarAction::SetWidth(new_w))
    }

    pub fn on_drag_end(&mut self) -> Option<SidebarAction> {
        self.drag.take()?;
        Some(SidebarAction::PersistConfig)
    }

    // ── Settings menu ──

    pub fn open_settings_menu(
        &mut self,
        screen_w: f32,
        screen_h: f32,
        _metrics: &crate::settings::UiMetrics,
        input: &SidebarSettingsInput,
    ) {
        let dpi = 1.0;
        let item_h = constants::ROW_HEIGHT * dpi;
        let menu_w = 200.0 * dpi;

        // Anchor (px): below the settings button, 水平居中于 sidebar
        let settings_btn = self.layout.as_ref().map(|l| l.settings_btn_rect);
        let (anchor_x, anchor_y) = if let Some(rect) = settings_btn {
            (rect.x + rect.w / 2.0 - menu_w / 2.0, rect.bottom() + 2.0 * dpi)
        } else {
            (screen_w * 0.025, screen_h * 0.65)
        };

        let show_line_numbers = input.show_line_numbers;
        let word_wrap = input.word_wrap;
        let show_status_bar = input.show_status_bar;
        let theme_mode = input.theme_mode;
        let current_mode = input.view_mode;
        let items = vec![
            PopupMenuItem {
                label: "显示行号".into(),
                is_active: show_line_numbers,
                is_separator: false,
                action: PMA::ToggleLineNumbers,
            },
            PopupMenuItem {
                label: "自动换行".into(),
                is_active: word_wrap,
                is_separator: false,
                action: PMA::ToggleWordWrap,
            },
            PopupMenuItem {
                label: "显示状态栏".into(),
                is_active: show_status_bar,
                is_separator: false,
                action: PMA::ToggleStatusBar,
            },
            PopupMenuItem {
                label: "".into(),
                is_active: false,
                is_separator: true,
                action: PMA::SetViewMode(ViewMode::Sidebar), // unused for separator
            },
            PopupMenuItem {
                label: "跟随系统".into(),
                is_active: theme_mode == crate::settings::ThemeMode::System,
                is_separator: false,
                action: PMA::SetThemeMode(crate::settings::ThemeMode::System),
            },
            PopupMenuItem {
                label: "深色模式".into(),
                is_active: theme_mode == crate::settings::ThemeMode::Dark,
                is_separator: false,
                action: PMA::SetThemeMode(crate::settings::ThemeMode::Dark),
            },
            PopupMenuItem {
                label: "浅色模式".into(),
                is_active: theme_mode == crate::settings::ThemeMode::Light,
                is_separator: false,
                action: PMA::SetThemeMode(crate::settings::ThemeMode::Light),
            },
            PopupMenuItem {
                label: "".into(),
                is_active: false,
                is_separator: true,
                action: PMA::SetViewMode(ViewMode::Sidebar), // unused for separator
            },
            PopupMenuItem {
                label: "Sidebar 模式".into(),
                is_active: current_mode == ViewMode::Sidebar,
                is_separator: false,
                action: PMA::SetViewMode(ViewMode::Sidebar),
            },
            PopupMenuItem {
                label: "Tabs 模式".into(),
                is_active: current_mode == ViewMode::Tabs,
                is_separator: false,
                action: PMA::SetViewMode(ViewMode::Tabs),
            },
            PopupMenuItem {
                label: "".into(),
                is_active: false,
                is_separator: true,
                action: PMA::SetViewMode(ViewMode::Sidebar), // unused for separator
            },
            PopupMenuItem {
                label: "打开Settings".into(),
                is_active: false,
                is_separator: false,
                action: PMA::OpenSettingsFile,
            },
        ];

        let menu_left = anchor_x.min(screen_w - menu_w).max(0.0);
        let menu_right = menu_left + menu_w;
        let mut top_px = anchor_y;
        let mut item_rects = Vec::with_capacity(items.len());
        for item in &items {
            let h = if item.is_separator { 8.0 * dpi } else { item_h };
            item_rects.push(Rect::new(menu_left, top_px, menu_right - menu_left, h));
            top_px += h;
        }
        let menu_h = top_px - anchor_y;

        // Overflow protection: flip menu upward if it extends below screen
        let final_top = if anchor_y + menu_h > screen_h - 4.0 * dpi {
            (anchor_y - menu_h - 4.0 * dpi).max(4.0 * dpi)
        } else {
            anchor_y
        };
        let offset = final_top - anchor_y;
        let adjusted_rects: Vec<Rect> =
            item_rects.iter().map(|r| Rect::new(r.x, r.y + offset, r.w, r.h)).collect();

        self.open_menu = Some(PopupMenu {
            items,
            item_rects: adjusted_rects,
            menu_rect: Rect::new(menu_left, final_top, menu_right - menu_left, menu_h),
            screen_size: (screen_w, screen_h),
            show_checkmarks: true,
        });
    }

    pub fn open_new_document_menu(
        &mut self,
        screen_w: f32,
        screen_h: f32,
        metrics: &crate::settings::UiMetrics,
    ) {
        let split_button_rect =
            self.layout.as_ref().map(|layout| layout.new_menu_btn_rect).unwrap_or(Rect::ZERO);
        self.open_menu =
            Some(build_new_document_menu(split_button_rect, (screen_w, screen_h), metrics));
        self.menu_hovered_index = None;
    }

    pub fn dispatch_menu_click(
        &mut self,
        px: f32,
        py: f32,
        _metrics: &crate::settings::UiMetrics,
    ) -> Option<SidebarAction> {
        let menu = self.open_menu.as_ref()?;
        let action = menu.hit_test_px(px, py)?;
        let result = match action {
            PMA::SetViewMode(mode) => Some(SidebarAction::SetViewMode(*mode)),
            PMA::OpenSettingsFile => Some(SidebarAction::OpenSettingsFile),
            PMA::ToggleLineNumbers => Some(SidebarAction::ToggleLineNumbers),
            PMA::ToggleWordWrap => Some(SidebarAction::ToggleWordWrap),
            PMA::ToggleStatusBar => Some(SidebarAction::ToggleStatusBar),
            PMA::SetThemeMode(mode) => Some(SidebarAction::SetThemeMode(*mode)),
            PMA::NewDocument(kind) => Some(SidebarAction::NewDocument(*kind)),
            _ => None,
        };
        self.open_menu = None;
        result
    }

    pub fn update_layout(
        &mut self,
        input: &SidebarInput<'_>,
        cfg: &SidebarConfig,
        metrics: &crate::settings::UiMetrics,
    ) {
        // Extreme narrow window: force hide sidebar even if pinned
        if matches!(self.visibility, Visibility::Pinned)
            && input.screen_w < cfg.width + MINIMUM_EDITOR_WIDTH_LOGICAL * metrics.dpi
        {
            self.visibility = Visibility::Hidden;
        }
        if matches!(self.visibility, Visibility::Hidden) {
            // Produce minimal layout with only hamburger button overlay
            let dpi = metrics.dpi;
            // Hamburger: 14dp (matches traffic light size), centered vertically with traffic lights
            let btn_size = 16.0 * dpi;
            let hx = input.traffic_light_inset.0 + 12.0 * dpi;
            let hy = 8.0 * dpi;
            let menu_btn = Rect::new(hx, hy, btn_size, btn_size);
            self.layout = Some(SidebarLayout {
                bg_rect: menu_btn,
                header_rect: menu_btn,
                menu_btn_rect: menu_btn,
                new_btn_rect: Rect::ZERO,
                new_menu_btn_rect: Rect::ZERO,
                open_btn_rect: Rect::ZERO,
                items: Vec::new(),
                files_header_rect: Rect::ZERO,
                list_clip: Rect::ZERO,
                settings_btn_rect: Rect::ZERO,
                edge_resize_rect: Rect::ZERO,
            });
            return;
        }
        let dpi = metrics.dpi;
        let header_h = HEADER_H * dpi;
        let row_h = ROW_H * dpi;
        let new_h = NEW_BTN_H * dpi;
        let settings_h = SETTINGS_BTN_H * dpi;
        let pad = PADDING * dpi;
        let edge_w = EDGE_RESIZE_W * dpi;
        let w = cfg.width;
        let top = input.content_top.max(0.0);
        let sh = input.screen_h.max(1.0);

        // Background
        let bg_rect = Rect::new(0.0, top, w, (sh - top).max(0.0));

        // Header
        let header_rect = Rect::new(0.0, top, w, header_h);

        // Hamburger menu button
        let menu_x = input.traffic_light_inset.0 + 12.0 * dpi;
        let menu_y = 8.0 * dpi;
        let menu_btn_rect = Rect::new(menu_x, menu_y, 16.0 * dpi, 16.0 * dpi);

        // New document button
        let new_y = top + header_h + pad;
        let new_row_rect = Rect::new(12.0 * dpi, new_y, w - 24.0 * dpi, new_h);
        let new_menu_width = SPLIT_BUTTON_MENU_WIDTH_LOGICAL * dpi;
        let new_btn_rect = Rect::new(
            new_row_rect.x,
            new_row_rect.y,
            (new_row_rect.w - new_menu_width).max(0.0),
            new_row_rect.h,
        );
        let new_menu_btn_rect = Rect::new(
            new_btn_rect.right(),
            new_row_rect.y,
            new_menu_width.min(new_row_rect.w),
            new_row_rect.h,
        );

        // Open file button (below new button)
        let open_y = new_y + new_h + pad * 0.5;
        let open_btn_rect = Rect::new(12.0 * dpi, open_y, w - 24.0 * dpi, new_h);

        // Files section header
        let files_header_y = open_y + new_h + pad;
        let files_header_h = 24.0 * dpi;
        let files_header_rect =
            Rect::new(12.0 * dpi, files_header_y, w - 24.0 * dpi, files_header_h);

        // List area: from below "文件" header to above settings button
        let list_top_px = files_header_y + files_header_h + pad;
        let settings_y = sh - settings_h - 10.0 * dpi;
        let list_bottom_px = settings_y - pad;
        let visible_h = (list_bottom_px - list_top_px).max(0.0);
        self.clamp_scroll(input.tabs.len(), row_h, visible_h);
        let list_clip = Rect::new(12.0 * dpi, list_top_px, (w - 24.0 * dpi).max(0.0), visible_h);

        // Settings button (settings_y defined above for list area calculation)
        let settings_btn_rect = Rect::new(12.0 * dpi, settings_y, w - 24.0 * dpi, settings_h);

        // Edge resize handle
        let edge_resize_rect = Rect::new(w - edge_w, top + header_h, edge_w * 2.0, sh - header_h);

        self.layout = Some(SidebarLayout {
            bg_rect,
            header_rect,
            menu_btn_rect,
            new_btn_rect,
            new_menu_btn_rect,
            open_btn_rect,
            items: Vec::new(),
            files_header_rect,
            list_clip,
            settings_btn_rect,
            edge_resize_rect,
        });
        let _ = input.traffic_light_inset; // 阶段 5 接入
        let _ = input.active_index; // 渲染时再用
    }
}

/// Pre-computed geometry for a sidebar action button (New / Open).
struct ActionBtnGeom {
    fg: [f32; 4],
    cx: f32,
    cy: f32,
    dpi: f32,
    icon_half: f32,
}

impl SidebarState {
    // ── Hover state machine (Phase 6) ──

    /// Notify the hover state machine of the current mouse position.
    /// px, py are physical pixels; screen_w, screen_h are the full window size.
    pub fn on_mouse_move(
        &mut self,
        px: f32,
        py: f32,
        screen_w: f32,
        _screen_h: f32,
        cfg: &SidebarConfig,
        _metrics: &crate::settings::UiMetrics,
    ) -> bool {
        let prev = self.hovered_button;
        let mut btn_hover = SidebarHoverButton::None;
        if let Some(layout) = &self.layout {
            if layout.menu_btn_rect.contains(px, py) {
                btn_hover = SidebarHoverButton::Hamburger;
            } else if layout.new_btn_rect.contains(px, py)
                || layout.new_menu_btn_rect.contains(px, py)
            {
                btn_hover = SidebarHoverButton::NewDoc;
            } else if layout.open_btn_rect.contains(px, py) {
                btn_hover = SidebarHoverButton::OpenFile;
            } else if layout.settings_btn_rect.contains(px, py) {
                btn_hover = SidebarHoverButton::Settings;
            }
        }
        self.hovered_button = btn_hover;

        let dpi = 1.0;
        let hot_band = HOT_BAND_LOGICAL * dpi;
        let in_left_hot = px >= 0.0 && px <= hot_band;
        // Hamburger button as trigger zone (via layout)
        let on_hamburger =
            self.layout.as_ref().map(|l| l.menu_btn_rect.contains(px, py)).unwrap_or(false);
        let in_hot_zone = in_left_hot || on_hamburger;

        match self.visibility {
            Visibility::Hidden => {
                if in_hot_zone {
                    if self.suppress_hover_enter {
                        // Suppress: visibility was just set to Hidden programmatically
                        // (e.g. TogglePin). Wait for mouse to leave hot zone first.
                    } else if self.hover_enter_at.is_none() {
                        self.hover_enter_at = Some(Instant::now());
                    }
                } else {
                    self.hover_enter_at.is_some();
                    self.hover_enter_at = None;
                    self.suppress_hover_enter = false;
                }
            }
            Visibility::HoverPeek => {
                let sidebar_w = cfg.width;
                let in_sidebar = px >= 0.0 && px <= sidebar_w && px < screen_w;
                if !in_sidebar {
                    self.hover_leave_at.is_none();
                    self.hovered_button = SidebarHoverButton::None;
                    if self.hover_leave_at.is_none() {
                        self.hover_leave_at = Some(Instant::now());
                    }
                } else {
                    self.hover_leave_at.is_some();
                    self.hover_leave_at = None;
                }
            }
            Visibility::HoverPeekFadingOut => {
                let sidebar_w = cfg.width;
                let in_sidebar = px >= 0.0 && px <= sidebar_w && px < screen_w;
                if in_sidebar {
                    // 鼠标回到 sidebar：取消 fade out，回到 HoverPeek
                    self.visibility = Visibility::HoverPeek;
                    self.hover_peek_leave_start = None;
                    self.hover_leave_at = None;
                } else {
                    self.hovered_button = SidebarHoverButton::None;
                }
            }
            Visibility::Pinned => {
                let sidebar_w = cfg.width;
                let in_sidebar = px >= 0.0 && px <= sidebar_w && px < screen_w;
                if !in_sidebar {
                    self.hovered_button = SidebarHoverButton::None;
                }
            }
        }
        self.hovered_button != prev
    }

    /// Tick the hover state machine. Call every frame.
    /// Returns (visibility_changed, animating).
    pub fn tick(
        &mut self,
        now: Instant,
        cfg: &SidebarConfig,
        _metrics: &crate::settings::UiMetrics,
    ) -> (bool, bool) {
        match self.visibility {
            Visibility::Hidden => {
                if self.hover_enter_at.is_some() {
                    self.visibility = Visibility::HoverPeek;
                    self.hover_enter_at = None;
                    self.hover_peek_start = Some(now);
                    let _ = cfg;
                    return (true, true);
                }
            }
            Visibility::HoverPeek => {
                let animating = if let Some(start) = self.hover_peek_start {
                    let elapsed = now.duration_since(start);
                    elapsed < Duration::from_millis(150)
                } else {
                    false
                };
                if self.hover_leave_at.is_some() {
                    self.visibility = Visibility::HoverPeekFadingOut;
                    self.hover_leave_at = None;
                    self.hover_peek_leave_start = Some(now);
                    return (true, true);
                }
                if animating {
                    return (false, true);
                }
            }
            Visibility::HoverPeekFadingOut => {
                if let Some(leave_start) = self.hover_peek_leave_start {
                    let elapsed = now.duration_since(leave_start);
                    if elapsed >= Duration::from_millis(150) {
                        self.visibility = Visibility::Hidden;
                        self.hover_peek_leave_start = None;
                        self.hover_peek_start = None;
                        return (true, false);
                    }
                }
                return (false, true);
            }
            Visibility::Pinned => {}
        }
        (false, false)
    }
    pub fn on_key(&mut self, key: SidebarKey, cfg: &mut SidebarConfig) -> Option<SidebarAction> {
        match key {
            SidebarKey::TogglePin => {
                if self.visibility == Visibility::Pinned {
                    self.visibility = Visibility::Hidden;
                    cfg.pinned = false;
                    self.suppress_hover_enter = true;
                } else {
                    self.visibility = Visibility::Pinned;
                    cfg.pinned = true;
                    self.suppress_hover_enter = false;
                }
                Some(SidebarAction::TogglePin)
            }
            SidebarKey::Escape => {
                if self.visibility == Visibility::HoverPeek
                    || self.visibility == Visibility::HoverPeekFadingOut
                {
                    self.set_visibility(Visibility::Hidden);
                    self.hover_leave_at = None;
                    self.hover_peek_start = None;
                    self.hover_peek_leave_start = None;
                    Some(SidebarAction::PersistConfig)
                } else {
                    None
                }
            }
        }
    }

    // ── Paint helpers ──

    /// Draw hover background (if hovered) and return geometry for icon + text drawing.
    fn action_btn_geom(
        &self,
        ctx: &mut PaintCtx,
        rect: Rect,
        hover: SidebarHoverButton,
        alpha: f32,
    ) -> ActionBtnGeom {
        if self.hovered_button == hover {
            let mut h_bg = ctx.theme.palette.sidebar_hover_bg;
            h_bg[3] *= alpha;
            ctx.list.fill_rounded(rect, h_bg, 8.0 * ctx.dpi);
        }
        let mut fg = ctx.theme.palette.text_muted;
        fg[3] *= alpha;
        let icon_half = 5.0 * ctx.dpi;
        let cx = rect.x + 12.0 * ctx.dpi + icon_half;
        let cy = rect.y + rect.h * 0.5;
        ActionBtnGeom { fg, cx, cy, dpi: ctx.dpi, icon_half }
    }

    /// Draw the hamburger icon at its layout position with the given alpha.
    pub fn paint_hamburger(&self, ctx: &mut PaintCtx, override_alpha: f32, skip_hover_bg: bool) {
        let Some(layout) = &self.layout else {
            return;
        };
        let alpha = override_alpha;
        if !skip_hover_bg && self.hovered_button == SidebarHoverButton::Hamburger {
            let mut h_bg = ctx.theme.palette.sidebar_hover_bg;
            h_bg[3] *= alpha;
            ctx.list.fill_menu_hover(layout.menu_btn_rect, h_bg, ctx.dpi);
        }
        let icon_color = ctx.theme.palette.text_muted;
        let line_w = 1.5 * ctx.dpi;
        let line_len = 12.0 * ctx.dpi;
        let cx = layout.menu_btn_rect.x + layout.menu_btn_rect.w * 0.5;
        let cy = layout.menu_btn_rect.y + layout.menu_btn_rect.h * 0.5;
        let gap = 4.0 * ctx.dpi;
        let mut fg = icon_color;
        fg[3] *= alpha;
        for i in [-1.0, 0.0, 1.0] {
            let y = cy + i * gap - line_w * 0.5;
            ctx.list.fill_rounded(
                Rect::new(cx - line_len * 0.5, y, line_len, line_w),
                fg,
                line_w * 0.5,
            );
        }
    }

    // ── Paint ──

    pub fn paint(&self, ctx: &mut PaintCtx, _active_index: Option<usize>) {
        let Some(layout) = &self.layout else {
            return;
        };

        let alpha = ctx.global_alpha;

        // Hidden (auto-hide) mode: only draw hamburger icon, no panel background
        if matches!(self.visibility, Visibility::Hidden) {
            self.paint_hamburger(ctx, alpha, true);
            return;
        }

        // 1) Background
        let mut bg = ctx.theme.palette.bg_surface;
        bg[3] *= alpha;
        ctx.list.fill(layout.bg_rect, bg);

        // Fill top-right corner gap outside the arc with titlebar color
        let radius = 8.0 * ctx.dpi;
        let mut titlebar_bg = ctx.theme.editor.background;
        titlebar_bg[3] *= alpha;
        let br = layout.bg_rect;
        // Fill entire corner square with titlebar color
        ctx.list.fill(Rect::new(br.x + br.w - radius, br.y, radius, radius), titlebar_bg);
        // Redraw sidebar bg as rounded rect → covers interior, matches arc
        ctx.list.fill_rounded(br, bg, radius);
        // Cover left side with plain rect to undo left-side rounding
        ctx.list.fill(Rect::new(br.x, br.y, br.w - radius, br.h), bg);

        // Right border: stroke rounded rect, bg overwrite on left → arc+line+arc
        let r = layout.bg_rect;
        let mut border = ctx.theme.palette.border_subtle;
        border[3] *= alpha;
        ctx.list.stroke_rounded(r, border, radius, 1.0);
        ctx.list.fill(Rect::new(r.x, r.y, r.w - radius, r.h), bg);

        // 3) Hamburger (on top of background, always visible)
        {
            if self.hovered_button == SidebarHoverButton::Hamburger {
                let mut h_bg = ctx.theme.palette.sidebar_hover_bg;
                h_bg[3] *= alpha;
                ctx.list.fill_menu_hover(layout.menu_btn_rect, h_bg, ctx.dpi);
            }
            let icon_color = ctx.theme.palette.text_muted;
            let line_w = 1.5 * ctx.dpi;
            let line_len = 12.0 * ctx.dpi;
            let cx = layout.menu_btn_rect.x + layout.menu_btn_rect.w * 0.5;
            let cy = layout.menu_btn_rect.y + layout.menu_btn_rect.h * 0.5;
            let gap = 4.0 * ctx.dpi;
            let mut fg = icon_color;
            fg[3] *= alpha;
            for i in [-1.0, 0.0, 1.0] {
                let y = cy + i * gap - line_w * 0.5;
                ctx.list.fill_rounded(
                    Rect::new(cx - line_len * 0.5, y, line_len, line_w),
                    fg,
                    line_w * 0.5,
                );
            }
        }

        // 4.5) Open file button
        {
            let g = self.action_btn_geom(
                ctx,
                layout.open_btn_rect,
                SidebarHoverButton::OpenFile,
                alpha,
            );
            let icon_sz = 14.0 * g.dpi;
            draw_icon(
                ctx.list,
                "folder-open",
                g.cx - icon_sz * 0.5,
                g.cy - icon_sz * 0.5,
                icon_sz,
                g.fg,
            );
            let font_size = 15.0 * g.dpi;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    g.cx + g.icon_half + 6.0 * g.dpi,
                    g.cy + font_size * 0.35,
                    font_size,
                    g.fg,
                    "\u{6253}\u{5f00}",
                    shaper,
                );
            };
        }

        // 4.5) Files section header
        {
            let font_size = 13.0 * ctx.dpi;
            let baseline =
                layout.files_header_rect.y + layout.files_header_rect.h * 0.5 + font_size * 0.35;
            let mut fg = ctx.theme.palette.text_muted;
            fg[3] *= 0.5 * alpha;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    layout.files_header_rect.x,
                    baseline,
                    font_size,
                    fg,
                    "\u{6587}\u{4ef6}",
                    shaper,
                );
            };
        }

        // 5) Settings button
        {
            if self.hovered_button == SidebarHoverButton::Settings {
                let mut h_bg = ctx.theme.palette.sidebar_hover_bg;
                h_bg[3] *= alpha;
                ctx.list.fill_rounded(layout.settings_btn_rect, h_bg, 8.0 * ctx.dpi);
            }

            let pad_left = 12.0 * ctx.dpi;
            let icon_size = 16.0 * ctx.dpi;
            let icon_x = layout.settings_btn_rect.x + pad_left;
            let icon_y =
                layout.settings_btn_rect.y + (layout.settings_btn_rect.h - icon_size) * 0.5;
            let mut fg = ctx.theme.palette.text_muted;
            fg[3] *= alpha;
            draw_icon(ctx.list, "settings", icon_x, icon_y, icon_size, fg);
            let font_size = 15.0 * ctx.dpi;
            let text_baseline =
                layout.settings_btn_rect.y + layout.settings_btn_rect.h * 0.5 + font_size * 0.35;
            if let Some(ref mut shaper) = ctx.shaper {
                ctx.list.text_shaped(
                    layout.settings_btn_rect.x + pad_left + icon_size + 2.0 * ctx.dpi,
                    text_baseline,
                    font_size,
                    fg,
                    "\u{8bbe}\u{7f6e}",
                    shaper,
                );
            };
        }
    }

    /// Paint the settings popup menu overlay.
    /// Must be called AFTER list.paint() so the menu draws on top of list items.
    pub fn paint_menu(&self, ctx: &mut PaintCtx) {
        if let Some(ref menu) = self.open_menu {
            menu.paint(ctx, self.menu_hovered_index);
        }
    }

    // ── Hit testing ──

    pub fn hit_test_px(
        &self,
        px: f32,
        py: f32,
        _metrics: &crate::settings::UiMetrics,
    ) -> Option<SidebarAction> {
        let layout = self.layout.as_ref()?;

        if layout.menu_btn_rect.contains(px, py) {
            return Some(SidebarAction::TogglePin);
        }
        // When hidden, only hamburger button is active
        if matches!(self.visibility, Visibility::Hidden) {
            return None;
        }
        if layout.new_btn_rect.contains(px, py) {
            return Some(SidebarAction::NewDocument(NewDocumentKind::Markdown));
        }
        if layout.new_menu_btn_rect.contains(px, py) {
            return Some(SidebarAction::OpenNewDocumentMenu);
        }
        if layout.open_btn_rect.contains(px, py) {
            return Some(SidebarAction::OpenDocument);
        }
        if layout.settings_btn_rect.contains(px, py) {
            return Some(SidebarAction::OpenSettingsMenu);
        }
        None
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::sidebar::persistent::SidebarPersistent;

    fn laid_out_sidebar_state(dpi: f32) -> (SidebarState, crate::settings::UiMetrics) {
        let mut cfg = SidebarConfig::new_default(dpi);
        cfg.pinned = true;
        let metrics =
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), dpi);
        let mut state = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0 * dpi,
            screen_h: 800.0 * dpi,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        state.update_layout(&input, &cfg, &metrics);
        (state, metrics)
    }

    #[test]
    fn sidebar_new_document_row_is_split_without_overlap() {
        let (state, _) = laid_out_sidebar_state(1.0);
        let layout = state.current_layout().expect("sidebar layout must exist");

        assert!(layout.new_btn_rect.w > layout.new_menu_btn_rect.w);
        assert_eq!(layout.new_btn_rect.right(), layout.new_menu_btn_rect.x);
        assert_eq!(layout.new_btn_rect.y, layout.new_menu_btn_rect.y);
        assert_eq!(layout.new_btn_rect.h, layout.new_menu_btn_rect.h);
    }

    #[test]
    fn sidebar_split_new_document_regions_emit_distinct_actions() {
        let (state, metrics) = laid_out_sidebar_state(1.0);
        let layout = state.current_layout().expect("sidebar layout must exist");
        let primary = layout.new_btn_rect;
        let dropdown = layout.new_menu_btn_rect;

        assert_eq!(
            state.hit_test_px(primary.x + 1.0, primary.y + 1.0, &metrics),
            Some(SidebarAction::NewDocument(NewDocumentKind::Markdown))
        );
        assert_eq!(
            state.hit_test_px(dropdown.x + 1.0, dropdown.y + 1.0, &metrics),
            Some(SidebarAction::OpenNewDocumentMenu)
        );
    }

    #[test]
    fn sidebar_split_new_document_regions_share_hover_state() {
        let (mut state, metrics) = laid_out_sidebar_state(1.0);
        let layout = state.current_layout().expect("sidebar layout must exist");
        let primary = layout.new_btn_rect;
        let dropdown = layout.new_menu_btn_rect;
        let cfg = SidebarConfig::new_default(1.0);

        state.on_mouse_move(
            primary.x + primary.w * 0.5,
            primary.y + primary.h * 0.5,
            1200.0,
            800.0,
            &cfg,
            &metrics,
        );
        assert_eq!(state.hovered_button, SidebarHoverButton::NewDoc);

        state.on_mouse_move(
            dropdown.x + dropdown.w * 0.5,
            dropdown.y + dropdown.h * 0.5,
            1200.0,
            800.0,
            &cfg,
            &metrics,
        );
        assert_eq!(state.hovered_button, SidebarHoverButton::NewDoc);
    }

    #[test]
    fn clamp_width_below_min() {
        let mut c = SidebarConfig { pinned: false, width: 50.0 };
        c.clamp_width(1.0);
        assert_eq!(c.width, 160.0);
    }

    #[test]
    fn clamp_width_above_max() {
        let mut c = SidebarConfig { pinned: false, width: 9999.0 };
        c.clamp_width(2.0);
        assert_eq!(c.width, 800.0);
    }

    #[test]
    fn clamp_width_within_range_unchanged() {
        let mut c = SidebarConfig { pinned: false, width: 300.0 };
        c.clamp_width(1.0);
        assert_eq!(c.width, 300.0);
    }

    #[test]
    fn sidebar_state_respects_cfg_pinned() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let s = SidebarState::new(&cfg);
        assert_eq!(s.visibility(), Visibility::Pinned);
        assert_eq!(s.current_width(&cfg), 220.0);
        assert_eq!(
            s.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            220.0
        );
    }

    #[test]
    fn sidebar_state_respects_cfg_not_pinned() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let s = SidebarState::new(&cfg);
        assert_eq!(s.visibility(), Visibility::Hidden);
        assert_eq!(s.current_width(&cfg), 0.0);
        assert_eq!(
            s.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            0.0
        );
    }

    #[test]
    fn sidebar_hidden_offsets_zero() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Hidden);
        assert_eq!(
            s.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            0.0
        );
        assert!(!s.is_visible());
    }

    #[test]
    fn sidebar_hover_peek_does_not_offset_editor() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        assert_eq!(
            s.editor_left_offset(
                &cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            ),
            0.0
        );
        assert_eq!(s.current_width(&cfg), 220.0);
        assert!(s.is_visible());
    }

    #[test]
    fn sidebar_layout_zero_items_when_no_tabs() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let layout = s.current_layout().expect("layout populated");
        assert!(layout.items.is_empty());
    }

    #[test]
    fn high_dpi_narrow_window_hides_pinned_sidebar_before_editor_becomes_too_narrow() {
        let dpi = 2.0;
        let mut cfg = SidebarConfig::new_default(dpi);
        cfg.pinned = true;
        let metrics =
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), dpi);
        let mut state = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: cfg.width + 180.0,
            screen_h: 800.0 * dpi,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };

        state.update_layout(&input, &cfg, &metrics);

        assert_eq!(state.visibility(), Visibility::Hidden);
    }

    #[test]
    fn sidebar_hidden_has_hamburger_layout() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Hidden);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // Hidden now has hamburger overlay layout
        assert!(s.current_layout().is_some());
    }

    #[test]
    fn sidebar_click_new_btn_emits_new_doc() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let new_rect = s.current_layout().unwrap().new_btn_rect;
        let px = new_rect.x + new_rect.w * 0.5;
        let py = new_rect.y + new_rect.h * 0.5;
        let action = s.hit_test_px(
            px,
            py,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::NewDocument(NewDocumentKind::Markdown))));
    }

    #[test]
    fn sidebar_click_open_btn_emits_open_doc() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let open_rect = s.current_layout().unwrap().open_btn_rect;
        assert!(open_rect.w > 0.0, "open_btn_rect should have non-zero width");
        let px = open_rect.x + open_rect.w * 0.5;
        let py = open_rect.y + open_rect.h * 0.5;
        let action = s.hit_test_px(
            px,
            py,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::OpenDocument)));
    }

    #[test]
    fn sidebar_open_btn_hover_sets_open_file() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let open_rect = s.current_layout().unwrap().open_btn_rect;
        let px = open_rect.x + open_rect.w * 0.5;
        let py = open_rect.y + open_rect.h * 0.5;
        let changed = s.on_mouse_move(
            px,
            py,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(s.hovered_button, SidebarHoverButton::OpenFile);
        assert!(changed);
    }

    #[test]
    fn sidebar_click_outside_returns_none() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(
            s.hit_test_px(
                1000.0,
                400.0,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            )
            .is_none()
        );
    }

    #[test]
    fn sidebar_hover_clears_when_no_hit() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.pinned = true;
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.set_hovered_index(Some(0));
        s.set_hovered_index(None);
        assert_eq!(s.hovered_index(), None);
    }

    #[test]
    fn sidebar_scroll_clamps_to_zero() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_list_scroll_offset(-50.0);
        let dpi = 1.0;
        s.clamp_scroll(5, 24.0 * dpi, 200.0 * dpi);
        assert_eq!(s.list_scroll_offset(), 0.0);
    }

    #[test]
    fn sidebar_scroll_clamps_to_max() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        let dpi = 1.0;
        let row_h = 24.0 * dpi;
        // 10 items at 24px row_h in 200px visible area: max = 10*24 - 200 = 40
        s.set_list_scroll_offset(999.0);
        s.clamp_scroll(10, row_h, 200.0 * dpi);
        let expected_max = (10.0 * row_h - 200.0 * dpi).max(0.0);
        assert_eq!(s.list_scroll_offset(), expected_max);
    }

    #[test]
    fn sidebar_width_drag_clamp_to_min() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        assert!(s.on_drag_start(
            220.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
        // Drag to 50px → clamp to min 160
        let action = s.on_drag(
            50.0,
            100.0,
            &mut cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::SetWidth(w)) if (w - 160.0).abs() < 0.01));
    }

    #[test]
    fn sidebar_width_drag_clamp_to_max() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        assert!(s.on_drag_start(
            220.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
        // Drag to 9999 → clamp to max 400
        let action = s.on_drag(
            9999.0,
            100.0,
            &mut cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::SetWidth(w)) if (w - 400.0).abs() < 0.01));
    }

    #[test]
    fn sidebar_width_drag_clamp_dpi_scaled() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        // width=220; global dpi=1 → clamp [160, 400]
        assert!(s.on_drag_start(
            220.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
        // Drag to min (drag px below 160)
        let action = s.on_drag(
            160.0 - 100.0,
            100.0,
            &mut cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::SetWidth(w)) if (w - 160.0).abs() < 0.01));
        // Drag to max
        s.on_drag(
            9999.0,
            100.0,
            &mut cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!((cfg.width - 400.0).abs() < 0.01);
        // Verify cfg.width was mutated
        assert_eq!(cfg.width, 400.0);
    }

    #[test]
    fn sidebar_drag_end_persists() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        assert!(s.on_drag_start(
            220.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
        let mid = s.on_drag(
            300.0,
            100.0,
            &mut cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(mid, Some(SidebarAction::SetWidth(_))));
        let end = s.on_drag_end();
        assert!(matches!(end, Some(SidebarAction::PersistConfig)));
        // After drag_end, a second call returns None (drag state consumed)
        assert!(s.on_drag_end().is_none());
    }

    #[test]
    fn sidebar_drag_start_outside_band_returns_false() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        // width=220, band=4; px=50 is far from 220
        assert!(!s.on_drag_start(
            50.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
        // px=1000 is far from 220
        assert!(!s.on_drag_start(
            1000.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
    }

    #[test]
    fn sidebar_drag_start_in_band_returns_true() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        // width=220, band=4; px=218 is within band
        assert!(s.on_drag_start(
            218.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
    }

    #[test]
    fn sidebar_drag_hidden_returns_false() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        // Hidden by default
        assert!(!s.on_drag_start(
            220.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
    }

    #[test]
    fn sidebar_drag_without_start_returns_none() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        // No drag_start called → on_drag returns None
        assert!(
            s.on_drag(
                300.0,
                100.0,
                &mut cfg,
                &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
            )
            .is_none()
        );
    }

    #[test]
    fn sidebar_drag_respects_initial_width() {
        let mut cfg = SidebarConfig::new_default(1.0);
        cfg.width = 300.0;
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        // Start drag at the right edge (300)
        assert!(s.on_drag_start(
            300.0,
            100.0,
            &cfg,
            1200.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0)
        ));
        // Move 20px right
        s.on_drag(
            320.0,
            100.0,
            &mut cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!((cfg.width - 320.0).abs() < 0.01);
    }

    // ── Hover state machine tests ──

    #[test]
    fn sidebar_hover_enter_instant() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        assert_eq!(s.visibility(), Visibility::Hidden);
        let t0 = Instant::now();
        // py=10.0 within header area (HEADER_H=28dp) to trigger hot zone
        s.on_mouse_move(
            2.0,
            10.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // Instant enter, no delay
        let (changed, _) = s.tick(
            t0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed);
        assert_eq!(s.visibility(), Visibility::HoverPeek);
    }

    #[test]
    fn sidebar_hover_enter_outside_hot_zone_does_nothing() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        let t0 = Instant::now();
        // Mouse at x=100 (way outside hot zone)
        s.on_mouse_move(
            100.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (changed, _) = s.tick(
            t0 + Duration::from_millis(500),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert_eq!(s.visibility(), Visibility::Hidden);
    }

    #[test]
    fn sidebar_hover_exit_instant() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        let t0 = Instant::now();
        // Mouse outside sidebar area
        s.on_mouse_move(
            500.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // 0ms leave delay: immediate transition to FadingOut
        let (changed, _) = s.tick(
            t0 + Duration::from_millis(1),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed);
        assert_eq!(s.visibility(), Visibility::HoverPeekFadingOut);
        // After fade-out animation (150ms), transition to Hidden
        let (changed, _) = s.tick(
            t0 + Duration::from_millis(200),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed);
        assert_eq!(s.visibility(), Visibility::Hidden);
    }

    #[test]
    fn sidebar_hover_exit_cancelled_by_reentry() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        let t0 = Instant::now();
        // Mouse leaves sidebar
        s.on_mouse_move(
            500.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // But re-enters before 300ms
        s.on_mouse_move(
            2.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (changed, _) = s.tick(
            t0 + Duration::from_millis(500),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert_eq!(s.visibility(), Visibility::HoverPeek);
    }

    #[test]
    fn sidebar_pinned_immune_to_hover_leave() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let t0 = Instant::now();
        s.on_mouse_move(
            900.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (changed, _) = s.tick(
            t0 + Duration::from_secs(5),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert_eq!(s.visibility(), Visibility::Pinned);
    }

    #[test]
    fn sidebar_hover_fading_out_animating_flag() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        let t0 = Instant::now();
        // 鼠标离开 sidebar
        s.on_mouse_move(
            500.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // 300ms 后进入 HoverPeekFadingOut，animating=true
        let (changed, animating) = s.tick(
            t0 + Duration::from_millis(310),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed);
        assert!(animating);
        assert_eq!(s.visibility(), Visibility::HoverPeekFadingOut);
        // 100ms 后仍在 fade out，无变化但 animating=true
        let (changed, animating) = s.tick(
            t0 + Duration::from_millis(410),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert!(animating);
        assert_eq!(s.visibility(), Visibility::HoverPeekFadingOut);
        // 150ms 后 fade out 完成
        let (changed, animating) = s.tick(
            t0 + Duration::from_millis(460),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed);
        assert!(!animating);
        assert_eq!(s.visibility(), Visibility::Hidden);
    }

    #[test]
    fn sidebar_hover_fading_out_cancel_by_reenter() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        let t0 = Instant::now();
        // 鼠标离开，300ms 后进入 fading out
        s.on_mouse_move(
            500.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let (_changed, _) = s.tick(
            t0 + Duration::from_millis(310),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(s.visibility(), Visibility::HoverPeekFadingOut);
        // 鼠标回到 sidebar → 取消 fade out，回到 HoverPeek
        s.on_mouse_move(
            2.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(s.visibility(), Visibility::HoverPeek);
        assert!(s.hover_peek_leave_start().is_none());
    }

    #[test]
    fn sidebar_esc_during_fading_out_collapses() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        let t0 = Instant::now();
        s.on_mouse_move(
            500.0,
            100.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        let _ = s.tick(
            t0 + Duration::from_millis(310),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(s.visibility(), Visibility::HoverPeekFadingOut);
        // ESC 应直接收起
        let action = s.on_key(SidebarKey::Escape, &mut cfg);
        assert!(matches!(action, Some(SidebarAction::PersistConfig)));
        assert_eq!(s.visibility(), Visibility::Hidden);
        assert!(s.hover_peek_start().is_none());
        assert!(s.hover_peek_leave_start().is_none());
    }

    #[test]
    fn sidebar_fade_in_animating_flag() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        let t0 = Instant::now();
        // py=10.0 within header area to trigger hot zone
        s.on_mouse_move(
            2.0,
            10.0,
            1200.0,
            800.0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // 即时进入 HoverPeek，fade-in 刚开始 → animating=true
        let (changed, animating) = s.tick(
            t0,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(changed);
        assert!(animating);
        assert_eq!(s.visibility(), Visibility::HoverPeek);
        // 100ms 后仍在 fade-in
        let (changed, animating) = s.tick(
            t0 + Duration::from_millis(100),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert!(animating);
        // 200ms 后 fade-in 完成 → animating=false
        let (changed, animating) = s.tick(
            t0 + Duration::from_millis(200),
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(!changed);
        assert!(!animating);
        assert_eq!(s.visibility(), Visibility::HoverPeek);
    }

    #[test]
    fn sidebar_cmdb_toggles_pin() {
        let mut cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        // Explicit pinned=true → starts Pinned
        assert_eq!(s.visibility(), Visibility::Pinned);
        let action = s.on_key(SidebarKey::TogglePin, &mut cfg);
        assert!(matches!(action, Some(SidebarAction::TogglePin)));
        assert_eq!(s.visibility(), Visibility::Hidden);
        assert!(!cfg.pinned);
        // Toggle again
        s.on_key(SidebarKey::TogglePin, &mut cfg);
        assert_eq!(s.visibility(), Visibility::Pinned);
        assert!(cfg.pinned);
    }

    #[test]
    fn sidebar_esc_collapses_hover_only() {
        let mut cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        let action = s.on_key(SidebarKey::Escape, &mut cfg);
        assert!(matches!(action, Some(SidebarAction::PersistConfig)));
        assert_eq!(s.visibility(), Visibility::Hidden);
        // Pinned: Esc does nothing
        s.set_visibility(Visibility::Pinned);
        let action2 = s.on_key(SidebarKey::Escape, &mut cfg);
        assert!(action2.is_none());
        assert_eq!(s.visibility(), Visibility::Pinned);
    }

    // ── Narrow window tests ──

    #[test]
    fn sidebar_extreme_narrow_window_disables_pin() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 250.0,
            screen_h: 600.0, // 250 < 220+100
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(s.visibility(), Visibility::Hidden);
    }

    #[test]
    fn sidebar_narrow_but_enough_window_keeps_pin() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 500.0,
            screen_h: 600.0, // 500 >= 220+100
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert_eq!(s.visibility(), Visibility::Pinned);
    }

    #[test]
    fn sidebar_narrow_window_hover_peek_still_works() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::HoverPeek);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 250.0,
            screen_h: 600.0, // narrow window
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        // HoverPeek should not be forced to Hidden
        assert_eq!(s.visibility(), Visibility::HoverPeek);
    }

    // ── Settings menu tests ──

    #[test]
    fn sidebar_settings_menu_open_close() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );
        assert!(s.open_menu().is_some());

        // Click "显示行号" item center → should toggle line numbers and close menu
        let menu = s.open_menu().unwrap().clone();
        let r = menu.item_rects[0];
        let cx = r.x + r.w * 0.5;
        let cy = r.y + r.h * 0.5;
        let action = s.dispatch_menu_click(
            cx,
            cy,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::ToggleLineNumbers)));
        assert!(s.open_menu().is_none());
    }

    #[test]
    fn sidebar_settings_menu_switch_to_tabs() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        // Click "Tabs 模式" item center → item index 9
        let menu = s.open_menu().unwrap().clone();
        let r = menu.item_rects[9];
        let cx = r.x + r.w * 0.5;
        let cy = r.y + r.h * 0.5;
        let action = s.dispatch_menu_click(
            cx,
            cy,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::SetViewMode(ViewMode::Tabs))));
        assert!(s.open_menu().is_none());
    }

    #[test]
    fn sidebar_settings_menu_open_settings_file() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        // Click "打开 settings.yaml" item → item index 11
        let menu = s.open_menu().unwrap().clone();
        let r = menu.item_rects[11];
        let cx = r.x + r.w * 0.5;
        let cy = r.y + r.h * 0.5;
        let action = s.dispatch_menu_click(
            cx,
            cy,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(matches!(action, Some(SidebarAction::OpenSettingsFile)));
        assert!(s.open_menu().is_none());
    }

    #[test]
    fn sidebar_settings_menu_click_outside_closes() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        // Click outside menu area → returns None
        let action = s.dispatch_menu_click(
            0.5,
            0.5,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(action.is_none());
    }

    #[test]
    fn sidebar_settings_menu_has_four_items() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );
        let menu = s.open_menu().unwrap();
        assert_eq!(menu.items.len(), 12);
        // Verify labels
        assert_eq!(menu.items[0].label, "显示行号");
        assert_eq!(menu.items[1].label, "自动换行");
        assert_eq!(menu.items[2].label, "显示状态栏");
        assert!(menu.items[3].is_separator);
        assert_eq!(menu.items[4].label, "跟随系统");
        assert_eq!(menu.items[5].label, "深色模式");
        assert_eq!(menu.items[6].label, "浅色模式");
        assert!(menu.items[7].is_separator);
        assert_eq!(menu.items[8].label, "Sidebar 模式");
        assert_eq!(menu.items[9].label, "Tabs 模式");
        assert!(menu.items[10].is_separator);
        assert_eq!(menu.items[11].label, "打开Settings");
    }

    // ── SidebarPersistent 测试 ──

    #[test]
    fn persistent_new_pinned() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let p = SidebarPersistent::new(&cfg);
        assert_eq!(p.visibility, Visibility::Pinned);
        assert!(p.hovered_index.is_none());
        assert_eq!(p.list_scroll_offset, 0.0);
        assert!(p.open_menu.is_none());
        assert!(!p.suppress_hover_enter);
    }

    #[test]
    fn persistent_new_not_pinned() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let p = SidebarPersistent::new(&cfg);
        assert_eq!(p.visibility, Visibility::Hidden);
    }

    #[test]
    fn persistent_current_width_hidden() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let p = SidebarPersistent::new(&cfg);
        assert_eq!(p.current_width(&cfg), 0.0);
    }

    #[test]
    fn persistent_current_width_pinned() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let p = SidebarPersistent::new(&cfg);
        assert_eq!(p.current_width(&cfg), 220.0);
    }

    #[test]
    fn persistent_current_width_hover_peek() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut p = SidebarPersistent::new(&cfg);
        p.visibility = Visibility::HoverPeek;
        assert_eq!(p.current_width(&cfg), 220.0);
    }

    #[test]
    fn persistent_editor_left_offset_pinned() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let p = SidebarPersistent::new(&cfg);
        assert_eq!(p.editor_left_offset(&cfg), 220.0);
    }

    #[test]
    fn persistent_editor_left_offset_hidden() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let p = SidebarPersistent::new(&cfg);
        assert_eq!(p.editor_left_offset(&cfg), 0.0);
    }

    #[test]
    fn persistent_editor_left_offset_hover_peek() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut p = SidebarPersistent::new(&cfg);
        p.visibility = Visibility::HoverPeek;
        assert_eq!(p.editor_left_offset(&cfg), 0.0);
    }

    #[test]
    fn state_to_restore_from_persistent_roundtrip() {
        let cfg = SidebarConfig { pinned: true, width: 220.0 };
        let _t0 = Instant::now();
        let mut s = SidebarState::new(&cfg);
        s.set_list_scroll_offset(42.0);
        s.hovered_index = Some(3);

        let p = s.to_persistent();
        assert_eq!(p.visibility, Visibility::Pinned);
        assert_eq!(p.hovered_index, Some(3));
        assert_eq!(p.list_scroll_offset, 42.0);

        // Restore into a fresh state
        let mut s2 = SidebarState::new(&cfg);
        s2.restore_from_persistent(&p);
        assert_eq!(s2.visibility, Visibility::Pinned);
        assert_eq!(s2.hovered_index, Some(3));
        assert_eq!(s2.list_scroll_offset, 42.0);
    }

    #[test]
    fn state_set_visibility() {
        let cfg = SidebarConfig { pinned: false, width: 220.0 };
        let mut s = SidebarState::new(&cfg);
        assert_eq!(s.visibility(), Visibility::Hidden);
        s.set_visibility(Visibility::Pinned);
        assert_eq!(s.visibility(), Visibility::Pinned);
    }

    #[test]
    fn update_menu_hover_tracks_item() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        // Initially no hover
        assert_eq!(s.menu_hovered_index(), None);

        // Hover over first item center
        let menu = s.open_menu().unwrap().clone();
        let r = menu.item_rects[0];
        s.update_menu_hover(r.x + r.w * 0.5, r.y + r.h * 0.5);
        assert_eq!(s.menu_hovered_index(), Some(0));
    }

    #[test]
    fn update_menu_hover_ignores_separator() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        // Find separator index (index 3)
        let menu = s.open_menu().unwrap().clone();
        assert!(menu.items[3].is_separator);
        let r = menu.item_rects[3];
        s.update_menu_hover(r.x + r.w * 0.5, r.y + r.h * 0.5);
        assert_eq!(s.menu_hovered_index(), None);
    }

    #[test]
    fn update_menu_hover_resets_on_close() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        let menu = s.open_menu().unwrap().clone();
        let r = menu.item_rects[0];
        s.update_menu_hover(r.x + r.w * 0.5, r.y + r.h * 0.5);
        assert_eq!(s.menu_hovered_index(), Some(0));

        // Close menu resets hover
        s.set_open_menu(None);
        assert_eq!(s.menu_hovered_index(), None);
    }

    #[test]
    fn settings_menu_overflow_protection() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 400.0, // increased screen height to fit the menu
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            400.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        let menu = s.open_menu().unwrap();
        // Menu should not extend below screen
        assert!(
            menu.menu_rect.bottom() <= 400.0,
            "Menu bottom {} exceeds screen height 400",
            menu.menu_rect.bottom()
        );
    }

    #[test]
    fn menu_stays_open_after_pointer_leaves_until_explicitly_dismissed() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );
        assert!(s.open_menu().is_some(), "menu should be open");

        // Simulate mouse leaving the menu area
        let menu = s.open_menu().unwrap().clone();
        let outside_x = menu.menu_rect.x - 50.0;
        let outside_y = menu.menu_rect.y - 50.0;
        s.update_menu_hover(outside_x, outside_y);
        assert!(s.open_menu().is_some(), "menu should still be open immediately after leave");

        // Menus are only dismissed by an explicit click or Escape, so a slow
        // pointer movement from the trigger into the menu must not close it.
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        let _ = s.tick(
            later,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(s.open_menu().is_some(), "menu should stay open after pointer leaves");
    }

    #[test]
    fn settings_menu_uses_latest_behavior_input() {
        let cfg = SidebarConfig::new_default(2.0);
        let mut state = SidebarState::new(&cfg);
        let metrics =
            crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 2.0);
        let input = SidebarSettingsInput {
            show_line_numbers: false,
            word_wrap: false,
            show_status_bar: true,
            theme_mode: crate::settings::ThemeMode::Dark,
            view_mode: ViewMode::Tabs,
        };

        state.open_settings_menu(800.0, 600.0, &metrics, &input);
        let menu = state.open_menu.as_ref().unwrap();
        assert!(!menu.items[0].is_active);
        assert!(!menu.items[1].is_active);
        assert!(menu.items[2].is_active);
        assert!(menu.items[5].is_active);
        assert!(menu.items[9].is_active);
    }

    #[test]
    fn menu_does_not_close_when_mouse_inside() {
        let cfg = SidebarConfig::new_default(1.0);
        let mut s = SidebarState::new(&cfg);
        s.set_visibility(Visibility::Pinned);
        let input = SidebarInput {
            tabs: &[],
            active_index: None,
            screen_w: 1200.0,
            screen_h: 800.0,
            traffic_light_inset: (0.0, 0.0),
            content_top: 0.0,
        };
        s.update_layout(
            &input,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        s.open_settings_menu(
            1200.0,
            800.0,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
            &SidebarSettingsInput::default(),
        );

        // Hover inside menu
        let menu = s.open_menu().unwrap().clone();
        let inside_x = menu.menu_rect.x + 10.0;
        let inside_y = menu.menu_rect.y + 10.0;
        s.update_menu_hover(inside_x, inside_y);

        let now = Instant::now();
        let later = now + Duration::from_millis(500);
        let _ = s.tick(
            later,
            &cfg,
            &crate::settings::UiMetrics::from_settings(&crate::settings::Settings::new(), 1.0),
        );
        assert!(s.open_menu().is_some(), "menu should stay when mouse is inside");
    }
}
