//! SidebarWidget — merged from old ui/src/sidebar.rs + ui/src/widgets/sidebar.rs.

pub(crate) mod layout;
pub(crate) mod menu;
pub(crate) mod persistent;
pub(crate) mod state;
pub mod types;

// Re-export all public types for backward compatibility
pub use menu::{build_new_document_menu, build_settings_menu};
pub use persistent::SidebarPersistent;
pub(crate) use state::SidebarState;
#[allow(unused_imports)]
pub(crate) use types::SidebarInput;
pub use types::{
    NewDocumentKind, SidebarAction, SidebarConfig, SidebarHoverButton, SidebarKey,
    SidebarSettingsInput, SidebarWidgetInput, Visibility,
};

// SidebarWidget — Phase 7.5：内嵌 ListWidget 管理 items 渲染/命中。
// 框架背景与普通按钮由 SidebarState::paint 负责；新建按钮和列表分别委托给子 widget。

use std::any::Any;

use crate::core::widget::{ControlAction, WidgetAction, WidgetId};
use crate::core::{Event, EventCtx, KeyCode, LayoutCtx, MouseButton, PaintCtx, Rect, Widget};
use crate::widgets::list::{
    ListAction, ListItem, ListItemIndicator, ListItemKind, ListStyle, ListWidget, Orientation,
};
use crate::widgets::split_button::{SplitButtonInput, SplitButtonWidget};

const NEW_DOCUMENT_BUTTON_ID: WidgetId = WidgetId(8_100);
const NEW_DOCUMENT_MENU_BUTTON_ID: WidgetId = WidgetId(8_101);

pub struct SidebarWidget {
    pub(crate) rect: Rect,
    pub(crate) state: SidebarState,
    pub(crate) cfg: SidebarConfig,

    // ── 子 widget ──
    pub(crate) list: ListWidget,
    new_document_button: SplitButtonWidget,

    // ── 外部注入数据 ──
    pub(crate) tabs: Vec<crate::tab_bar::TabInfo>,
    pub(crate) active_index: Option<usize>,
    /// sorted_index → original_workspace_index mapping (for SwitchTab after pin sort)
    tab_index_map: Vec<usize>,
    pub(crate) traffic_light_inset: (f32, f32),
    pub(crate) screen_w: f32,
    pub(crate) screen_h: f32,
    pub metrics: crate::settings::UiMetrics,

    // ── resize 拖拽状态（入口已移除，保留实现） ──
    #[allow(dead_code)]
    dragging: bool,
    #[allow(dead_code)]
    drag_start_px: f32,
    #[allow(dead_code)]
    drag_start_width: f32,
    // ── 脏检查缓存 ──
    pub(crate) list_items: Vec<ListItem>,
    pub(crate) list_items_dirty: bool,
    pub(crate) settings_input: types::SidebarSettingsInput,
}

fn make_style_from_theme(theme: &crate::theme::Theme) -> ListStyle {
    let application = theme.application_theme();
    ListStyle {
        row_h_logical: crate::constants::ROW_HEIGHT,
        item_w_logical: 0.0,
        pad_x_logical: theme.control_metrics().horizontal_padding_logical,
        pad_y_logical: 0.0,
        font_size_logical: 13.0,
        bg: [0.0, 0.0, 0.0, 0.0], // 透明：sidebar 主背景已铺好
        item_active_bg: application.navigation_selected_surface,
        item_hover_bg: application.navigation_hover_surface,
        item_fg: application.text_secondary,
        item_active_fg: application.accent,
        item_hover_fg: application.text_primary,
        item_accent: application.accent,
        separator: application.strong_border,
        indicator_color: theme.editor.foreground,
    }
}

impl SidebarWidget {
    pub fn new(cfg: SidebarConfig, metrics: crate::settings::UiMetrics) -> Self {
        let mut cfg = cfg;
        cfg.clamp_width(metrics.dpi);
        let state = SidebarState::new(&cfg);
        let list = ListWidget::new(
            ListStyle {
                row_h_logical: crate::constants::ROW_HEIGHT,
                item_w_logical: 0.0,
                pad_x_logical: 12.0,
                pad_y_logical: 0.0,
                font_size_logical: 13.0,
                bg: [0.0; 4],
                item_active_bg: [0.0; 4],
                item_hover_bg: [0.0; 4],
                item_fg: [0.0; 4],
                item_active_fg: [0.0; 4],
                item_hover_fg: [0.0; 4],
                item_accent: [0.0; 4],
                separator: [0.0; 4],
                indicator_color: [0.0; 4],
            },
            Orientation::Vertical,
        );
        let mut new_document_button = SplitButtonWidget::new();
        new_document_button.set_action_ids(NEW_DOCUMENT_BUTTON_ID, NEW_DOCUMENT_MENU_BUTTON_ID);
        new_document_button.set_icon(Some("plus".to_owned()));
        new_document_button
            .set_input(SplitButtonInput { label: "新建".to_owned(), enabled: true });
        Self {
            rect: Rect::ZERO,
            state,
            cfg,
            list,
            new_document_button,
            tabs: Vec::new(),
            active_index: None,
            tab_index_map: Vec::new(),
            traffic_light_inset: (0.0, 0.0),
            screen_w: 800.0,
            screen_h: 600.0,
            dragging: false,
            drag_start_px: 0.0,
            drag_start_width: 0.0,
            list_items: Vec::new(),
            list_items_dirty: true,
            metrics,
            settings_input: types::SidebarSettingsInput::default(),
        }
    }

    /// 唯一输入入口：从 `SidebarWidgetInput` 更新所有 per-frame 数据。
    pub fn set_input(&mut self, input: types::SidebarWidgetInput) {
        let types::SidebarWidgetInput {
            tabs,
            active_index,
            traffic_light_inset_px,
            screen_size_px,
            metrics,
            settings,
        } = input;

        let mut indexed: Vec<(usize, crate::tab_bar::TabInfo)> =
            tabs.into_iter().enumerate().collect();
        indexed.sort_by_key(|(_, tab)| !tab.pinned);
        let new_active = active_index
            .and_then(|active| indexed.iter().position(|(original, _)| *original == active));
        let new_tab_index_map: Vec<usize> = indexed.iter().map(|(original, _)| *original).collect();
        let new_tabs: Vec<crate::tab_bar::TabInfo> =
            indexed.into_iter().map(|(_, tab)| tab).collect();

        let tabs_changed = new_tabs.len() != self.tabs.len()
            || new_active != self.active_index
            || new_tabs.iter().zip(self.tabs.iter()).any(|(a, b)| {
                a.title != b.title || a.is_dirty != b.is_dirty || a.pinned != b.pinned
            });
        if tabs_changed {
            self.list_items_dirty = true;
        }

        self.active_index = new_active;
        self.tab_index_map = new_tab_index_map;
        self.tabs = new_tabs;
        self.traffic_light_inset = traffic_light_inset_px;
        self.screen_w = screen_size_px.0;
        self.screen_h = screen_size_px.1;
        self.metrics = metrics;
        self.settings_input = settings;
    }

    /// 跨帧持久化：导出轻量状态
    pub fn steal_persistent(&mut self) -> SidebarPersistent {
        self.state.to_persistent()
    }

    /// 跨帧持久化：恢复轻量状态
    pub fn inject_persistent(&mut self, p: &SidebarPersistent) {
        self.state.restore_from_persistent(p);
    }

    pub fn config(&self) -> &SidebarConfig {
        &self.cfg
    }

    pub fn config_mut(&mut self) -> &mut SidebarConfig {
        &mut self.cfg
    }

    pub fn editor_left_offset(&self) -> f32 {
        self.state.editor_left_offset(&self.cfg, &self.metrics)
    }

    pub fn visibility(&self) -> Visibility {
        self.state.visibility()
    }

    pub fn set_visibility(&mut self, v: Visibility) {
        self.state.set_visibility(v);
    }

    pub fn tick(&mut self, now: std::time::Instant) -> (bool, bool) {
        self.state.tick(now, &self.cfg, &self.metrics)
    }

    pub fn on_mouse_move_for_hover(&mut self, px: f32, py: f32) {
        self.state.on_mouse_move(px, py, self.screen_w, self.screen_h, &self.cfg, &self.metrics);
    }

    pub fn on_key(&mut self, key: types::SidebarKey) -> Option<SidebarAction> {
        self.state.on_key(key, &mut self.cfg)
    }

    fn translate_list_action(&self, action: WidgetAction) -> Option<WidgetAction> {
        let WidgetAction::List(action) = action else {
            return Some(action);
        };
        let workspace_index = |sorted_index: usize| {
            self.tab_index_map.get(sorted_index).copied().unwrap_or(sorted_index)
        };
        match action {
            ListAction::Selected(index) => {
                Some(WidgetAction::Sidebar(SidebarAction::SwitchTab(workspace_index(index))))
            }
            ListAction::CloseRequested(index) => {
                Some(WidgetAction::Sidebar(SidebarAction::CloseTab(workspace_index(index))))
            }
            ListAction::ContextRequested { index, anchor_px } => {
                Some(WidgetAction::Sidebar(SidebarAction::ContextMenuPx {
                    tab_index: workspace_index(index),
                    anchor_px,
                    screen_size: (self.screen_w, self.screen_h),
                }))
            }
            ListAction::HoverChanged(_) => Some(WidgetAction::Sidebar(SidebarAction::Hovered)),
        }
    }

    /// 委托：打开菜单状态
    pub fn open_menu(&self) -> Option<&crate::widgets::popup_menu::PopupMenu> {
        self.state.open_menu()
    }

    /// 委托：设置打开菜单
    pub fn set_open_menu(&mut self, menu: Option<crate::widgets::popup_menu::PopupMenu>) {
        let new_document_menu_open = menu.as_ref().is_some_and(is_new_document_menu);
        self.state.set_open_menu(menu);
        self.new_document_button.set_menu_open(new_document_menu_open);
    }

    /// 委托：分发菜单点击
    pub fn dispatch_menu_click(&mut self, px: f32, py: f32) -> Option<SidebarAction> {
        let action = self.state.dispatch_menu_click(px, py, &self.metrics);
        self.new_document_button.set_menu_open(false);
        action
    }

    /// 委托：滚动
    pub fn on_scroll(&mut self, delta_px: f32, total_tabs: usize) {
        self.state.on_scroll(delta_px, total_tabs, &self.metrics);
    }

    /// 委托：是否可见
    pub fn is_visible(&self) -> bool {
        self.state.is_visible()
    }

    /// 委托：hover 索引
    pub fn hovered_index(&self) -> Option<usize> {
        self.state.hovered_index()
    }

    /// 委托：设置 hover 索引
    pub fn set_hovered_index(&mut self, idx: Option<usize>) {
        self.state.set_hovered_index(idx);
    }

    /// 委托：列表滚动偏移
    pub fn list_scroll_offset(&self) -> f32 {
        self.state.list_scroll_offset()
    }

    /// 委托：命中测试（公开）
    pub fn hit_test_px(&self, px: f32, py: f32) -> Option<SidebarAction> {
        self.state.hit_test_px(px, py, &self.metrics)
    }

    /// SidebarConfig.pinned 访问
    pub fn pinned(&self) -> bool {
        self.cfg.pinned
    }

    /// SidebarConfig.pinned 设置
    pub fn set_pinned(&mut self, v: bool) {
        self.cfg.pinned = v;
    }

    /// SidebarConfig.width 访问
    pub fn sidebar_width(&self) -> f32 {
        self.cfg.width
    }

    /// SidebarConfig.width 设置（含 clamp）
    pub fn set_sidebar_width(&mut self, w: f32) {
        self.cfg.width = w;
        let dpi = self.metrics.dpi;
        self.cfg.clamp_width(dpi);
    }

    /// 当前布局引用
    pub fn current_layout(&self) -> Option<&layout::SidebarLayout> {
        self.state.current_layout()
    }

    /// 打开设置菜单
    pub fn open_settings_menu(&mut self) {
        self.state.open_settings_menu(
            self.screen_w,
            self.screen_h,
            &self.metrics,
            &self.settings_input,
        );
        self.new_document_button.set_menu_open(false);
    }

    /// on_mouse_move（完整签名，兼容 events.rs）
    pub fn on_mouse_move_full(&mut self, px: f32, py: f32, screen_w: f32, screen_h: f32) {
        self.state.on_mouse_move(px, py, screen_w, screen_h, &self.cfg, &self.metrics);
    }
}

impl Widget for SidebarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        let dpi = ctx.dpi;
        self.cfg.clamp_width(dpi);
        let input = types::SidebarInput {
            tabs: &self.tabs,
            active_index: self.active_index,
            screen_w: self.screen_w,
            screen_h: self.screen_h,
            traffic_light_inset: self.traffic_light_inset,
            content_top: 0.0,
        };
        // E.3: 动画期间缩放 sidebar 宽度
        let original_width = self.cfg.width;
        let vis = self.state.visibility();
        if let types::Visibility::HoverPeek = vis {
            if let Some(start) = self.state.hover_peek_start() {
                let progress = (start.elapsed().as_secs_f32() / 0.15).clamp(0.0, 1.0);
                self.cfg.width = original_width * progress;
            }
        } else if let types::Visibility::HoverPeekFadingOut = vis
            && let Some(leave_start) = self.state.hover_peek_leave_start()
        {
            let progress = 1.0 - (leave_start.elapsed().as_secs_f32() / 0.15).clamp(0.0, 1.0);
            self.cfg.width = original_width * progress;
        }
        self.state.update_layout(&input, &self.cfg, &self.metrics);
        self.cfg.width = original_width;

        // list 子 widget 的矩形 = list_clip
        let list_rect = self.state.current_layout().map(|l| l.list_clip).unwrap_or(Rect::ZERO);
        let new_document_rect = self
            .state
            .current_layout()
            .map(|layout| {
                Rect::new(
                    layout.new_btn_rect.x,
                    layout.new_btn_rect.y,
                    (layout.new_menu_btn_rect.right() - layout.new_btn_rect.x).max(0.0),
                    layout.new_btn_rect.h,
                )
            })
            .unwrap_or(Rect::ZERO);

        // 把 theme 颜色灌进 list style
        self.list.set_style(make_style_from_theme(ctx.theme));

        // items：tab → ListItem（脏检查：仅在 tabs 变化时重建并推入 list widget）
        if self.list_items_dirty {
            self.list_items = self
                .tabs
                .iter()
                .map(|t| ListItem {
                    label: t.title.clone(),
                    kind: ListItemKind::Normal,
                    icon: None,
                    indicator: if t.is_dirty {
                        ListItemIndicator::Dot
                    } else {
                        ListItemIndicator::None
                    },
                    pinned: t.pinned,
                    extra_label: None,
                    is_active: false,
                    closeable: !t.pinned,
                })
                .collect();
            self.list_items_dirty = false;
            self.list.set_items(self.list_items.clone());
        }
        self.list.set_active(self.active_index);
        self.list.set_rect(list_rect, ctx);
        self.list.set_scroll_offset(self.list_scroll_offset());
        self.new_document_button.set_rect(new_document_rect, ctx);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.state.current_layout().is_none() {
            return;
        }

        let vis = self.state.visibility();
        let alpha = match vis {
            types::Visibility::HoverPeek => {
                if let Some(start) = self.state.hover_peek_start() {
                    (start.elapsed().as_secs_f32() / 0.15).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            }
            types::Visibility::HoverPeekFadingOut => {
                if let Some(leave_start) = self.state.hover_peek_leave_start() {
                    1.0 - (leave_start.elapsed().as_secs_f32() / 0.15).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            }
            _ => 1.0,
        };
        let saved_alpha = ctx.global_alpha;
        ctx.global_alpha *= alpha;

        // During slide animation, draw a static hamburger at the target position
        // so it's visible from the start (prevents flash when transitioning from Hidden).
        match vis {
            types::Visibility::HoverPeek => {
                if let Some(start) = self.state.hover_peek_start() {
                    let t = (start.elapsed().as_secs_f32() / 0.15).clamp(0.0, 1.0);
                    if t < 1.0 {
                        self.state.paint_hamburger(ctx, alpha * t, false);
                    }
                }
            }
            types::Visibility::HoverPeekFadingOut
                // 自动隐藏时 hamburger 始终可见：fade-out 期间保持全不透明，
                // 避免 sidebar 滑走后回到 Hidden 时 hamburger 从 0→1 跳变闪动。
                if self.state.hover_peek_leave_start().is_some() => {
                    let saved = ctx.global_alpha;
                    ctx.global_alpha = 1.0;
                    self.state.paint_hamburger(ctx, 1.0, true);
                    ctx.global_alpha = saved;
                }
            _ => {}
        }

        // Slide animation: translate sidebar horizontally
        let slide_x = match vis {
            types::Visibility::HoverPeek => {
                if let Some(start) = self.state.hover_peek_start() {
                    let t = (start.elapsed().as_secs_f32() / 0.15).clamp(0.0, 1.0);
                    -self.cfg.width * (1.0 - t).powi(3)
                } else {
                    0.0
                }
            }
            types::Visibility::HoverPeekFadingOut => {
                if let Some(leave_start) = self.state.hover_peek_leave_start() {
                    let t = (leave_start.elapsed().as_secs_f32() / 0.15).clamp(0.0, 1.0);
                    -self.cfg.width * (1.0 - (1.0 - t).powi(3))
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        let saved_offset = ctx.list.offset;
        ctx.list.offset.0 += slide_x;

        // 框架（bg/header/按钮/文字）
        self.state.paint(ctx, self.active_index);
        self.new_document_button.paint(ctx);
        // items 列表
        if let Some(_layout) = self.state.current_layout() {
            self.list.paint(ctx);
        }

        // Popup menu must be drawn AFTER list items so it's on top
        self.state.paint_menu(ctx);

        ctx.list.offset = saved_offset;
        ctx.global_alpha = saved_alpha;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        // 使用 layout 边界做命中检测。汉堡按钮在 title bar 区域（y < bg_rect.y），
        // 必须独立命中，否则 events.rs 的 title-bar guard 会先把点击吞掉。
        // Auto-hide 模式时，左侧热区也视为命中，确保 cursor_hint 能生效。
        if let Some(layout) = self.state.current_layout()
            && (layout.bg_rect.contains(px, py) || layout.menu_btn_rect.contains(px, py))
        {
            return true;
        }
        let vis = self.state.visibility();
        if matches!(vis, Visibility::Hidden | Visibility::HoverPeekFadingOut) {
            let hot_band = types::HOT_BAND_LOGICAL * self.metrics.dpi;
            px >= 0.0 && px <= hot_band
        } else {
            false
        }
    }

    fn is_capturing(&self) -> bool {
        self.dragging
            || self.state.open_menu().is_some()
            || self.new_document_button.is_capturing()
            || self.list.is_capturing()
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            // MouseMove: dragging, hover, and prevent editor fallthrough
            Event::MouseMove { px, py } => {
                // Track menu hover when popup menu is open
                if self.state.open_menu().is_some() {
                    self.state.update_menu_hover(*px, *py);
                    return Some(WidgetAction::Sidebar(SidebarAction::Hovered));
                }
                // Update widget-local hover state machine; only redraw if hover target changed
                let hover_changed = self.state.on_mouse_move(
                    *px,
                    *py,
                    self.screen_w,
                    self.screen_h,
                    &self.cfg,
                    &self.metrics,
                );
                let new_document_action = self.new_document_button.on_event(ev, ctx);
                // Delegate hover to list widget; capture whether list hover changed
                let list_hover_changed = if let Some(_layout) = self.state.current_layout() {
                    self.list.set_scroll_offset(self.list_scroll_offset());
                    self.list.on_event(ev, ctx).is_some()
                } else {
                    false
                };

                // Cursor hint: Pointer for interactive elements, Default elsewhere.
                // Guard: only set when mouse is inside this widget.
                let inside_sidebar = self
                    .state
                    .current_layout()
                    .map(|l| l.bg_rect.contains(*px, *py) || l.menu_btn_rect.contains(*px, *py))
                    .unwrap_or(false);
                if inside_sidebar {
                    if self.state.hit_test_px(*px, *py, &self.metrics).is_some()
                        || self.list.hit_close_btn(*px, *py, ctx.dpi).is_some()
                    {
                        ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                    } else {
                        ctx.cursor_hint = Some(winit::window::CursorIcon::Default);
                    }
                }
                // Hot zone cursor in auto-hide mode: full-height left edge strip.
                // Include HoverPeek so the left edge stays consistent during the
                // entire hover lifecycle (no flicker between Pointer and Default).
                let in_hot_zone = matches!(
                    self.state.visibility(),
                    Visibility::Hidden | Visibility::HoverPeek | Visibility::HoverPeekFadingOut
                ) && *px >= 0.0
                    && *px <= types::HOT_BAND_LOGICAL * ctx.dpi;
                if in_hot_zone {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Default);
                }

                // Trigger redraw when sidebar or list hover target changed
                if hover_changed || list_hover_changed || new_document_action.is_some() {
                    return Some(WidgetAction::Sidebar(SidebarAction::Hovered));
                }
                // Consume event when inside sidebar OR in auto-hide hot zone
                // (hot zone must block event from reaching editor, otherwise
                // editor sets Text cursor and causes flicker between Pointer/Text).
                if inside_sidebar || in_hot_zone { Some(WidgetAction::Consumed) } else { None }
            }
            Event::MouseDown { px, py, button } if *button == MouseButton::Right => {
                // Right-click: hit test for context menu
                if self.state.open_menu().is_some() {
                    self.state.set_open_menu(None);
                }

                if self.state.current_layout().is_some()
                    && let Some(action) = self.list.on_event(ev, ctx)
                {
                    return self.translate_list_action(action);
                }

                // Check framework buttons (hamburger toggle)
                if let Some(action) = self.state.hit_test_px(*px, *py, &self.metrics)
                    && action == SidebarAction::TogglePin
                {
                    return Some(WidgetAction::Sidebar(SidebarAction::TogglePin));
                }
                None
            }
            Event::MouseDown { px, py, button } if *button == MouseButton::Left => {
                let px = *px;
                let py = *py;

                // 0) If settings menu is open, dispatch clicks to it
                if self.state.open_menu().is_some() {
                    if let Some(action) = self.state.dispatch_menu_click(px, py, &self.metrics) {
                        self.new_document_button.set_menu_open(false);
                        return Some(WidgetAction::Sidebar(action));
                    }
                    // Click outside menu: dismiss
                    self.state.set_open_menu(None);
                    self.new_document_button.set_menu_open(false);
                    return None;
                }

                if let Some(action) = self.new_document_button.on_event(ev, ctx) {
                    return Some(action);
                }

                // 1) Hit test sidebar framework buttons (header/new/settings)
                if let Some(action) = self.state.hit_test_px(px, py, &self.metrics) {
                    return Some(WidgetAction::Sidebar(action));
                }

                if self.state.current_layout().is_some()
                    && let Some(action) = self.list.on_event(ev, ctx)
                {
                    return self.translate_list_action(action);
                }

                None
            }

            Event::KeyDown(KeyCode::Escape, _) if self.state.open_menu().is_some() => {
                self.state.set_open_menu(None);
                self.new_document_button.set_menu_open(false);
                Some(WidgetAction::Sidebar(SidebarAction::Hovered))
            }

            Event::MouseUp { .. } => {
                if self.list.is_capturing() {
                    let action = self.list.on_event(ev, ctx)?;
                    return self.translate_list_action(action);
                }
                let action = self.new_document_button.on_event(ev, ctx)?;
                match action {
                    WidgetAction::Control(ControlAction::Activated {
                        id: NEW_DOCUMENT_BUTTON_ID,
                    }) => Some(WidgetAction::Sidebar(SidebarAction::NewDocument(
                        NewDocumentKind::Markdown,
                    ))),
                    WidgetAction::Control(ControlAction::Activated {
                        id: NEW_DOCUMENT_MENU_BUTTON_ID,
                    }) => {
                        self.state.open_new_document_menu(
                            self.screen_w,
                            self.screen_h,
                            &self.metrics,
                        );
                        self.new_document_button.set_menu_open(true);
                        Some(WidgetAction::Sidebar(SidebarAction::Hovered))
                    }
                    WidgetAction::Consumed => Some(WidgetAction::Consumed),
                    _ => None,
                }
            }

            _ => None,
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn is_new_document_menu(menu: &crate::widgets::popup_menu::PopupMenu) -> bool {
    menu.items.iter().any(|item| {
        matches!(item.action, crate::widgets::popup_menu::PopupMenuAction::NewDocument(_))
    })
}

#[cfg(test)]
mod widget_tests;
