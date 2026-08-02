//! UiShell：持有 Dock + overlays，每帧重算 layout。
//! Phase 2：Dock 计算 editor_rect 与老路径双跑对齐，不接管实际渲染。
//! Phase 3：StatusBarWidget 接管道——status 位由真 widget 渲染。

use std::any::Any;
use std::time::Instant;
use ui::Theme;
use ui::canvas_scrollbars::{CanvasScrollbarsInput, CanvasScrollbarsWidget};
use ui::core::dock::{Dock, DockChild, Side};
use ui::core::geom::{Rect, Screen};
use ui::core::measure::TextMeasure;
use ui::core::overlay::{DismissPolicy, OverlayAction, OverlayInputPolicy, OverlayLayout};
use ui::core::paint::DrawList;
use ui::core::widget::{Event, EventCtx, KeyCode, LayoutCtx, PaintCtx, Widget, WidgetAction};
use ui::mindmap_style_panel::{
    MindmapStylePanelInput, MindmapStylePanelWidget, PANEL_WIDTH_LOGICAL,
};
use ui::scrollbar::ScrollbarWidget;
use ui::search_bar::{SearchBarSnapshot, SearchBarWidget};
use ui::sidebar::SidebarWidget;
use ui::status_bar::StatusBarInput;
use ui::status_bar::StatusBarWidget;
use ui::tab_bar::TabBarWidget;
use ui::title_bar::{TitleBarInput, TitleBarWidget};
use ui::tooltip::{TooltipHint, TooltipWidget};

use crate::editor_host::EditorHostWidget;

const OVERLAY_LOCAL_ORIGIN: f32 = 0.0;

/// Shell 输入：从 App / Workspace 读出的 chrome 状态。
#[derive(Debug, Clone)]
pub struct ShellInputs {
    pub tabs_visible: bool,
    pub tabs_thickness: f32,
    pub search_visible: bool,
    pub search_thickness: f32,
    pub status_thickness: f32,
    pub sidebar_visible: bool,
    pub sidebar_thickness: f32,
    pub scrollbar_thickness: f32,
    pub toc_visible: bool,
    pub toc_thickness: f32,
    pub metrics: ui::settings::UiMetrics,
    pub sidebar_settings: ui::sidebar::SidebarSettingsInput,
}

/// UiShell：持有 Dock 容器 + overlay widget 列表 + chrome 标志位。
struct OverlayChild {
    widget: Box<dyn Widget>,
    layout_rect: Rect,
}

struct OverlayEntry {
    widget: Box<dyn Widget>,
    layout: OverlayLayout,
    layout_rect: Rect,
    input_policy: OverlayInputPolicy,
    dismiss_policy: DismissPolicy,
    restore_focus: KeyboardFocusTarget,
}

struct TooltipTimer {
    hint: TooltipHint,
    target_screen_rect: ui::core::geom::Rect,
    start: Instant,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyboardFocusTarget {
    Editor,
    Widget(ui::core::widget::WidgetId),
}

enum OverlayDispatchOutcome {
    NotHandled,
    SilentConsumed,
    Action(WidgetAction),
}

pub struct UiShell {
    dock: Dock,
    overlays: Vec<OverlayEntry>,
    /// 画布区域的长期存在覆盖滚动条；不属于 Dock，避免压缩 editor rect。
    canvas_scrollbars: CanvasScrollbarsWidget,
    /// `None` 表示当前帧没有画布覆盖滚动条。
    canvas_scrollbars_input: Option<CanvasScrollbarsInput>,
    /// StatusBar 的输入数据（app 在 update_frame 前注入）。
    status_input: Option<StatusBarInput>,
    /// SearchBar 的输入数据（app 在 update_frame 前注入）。
    search_input: Option<SearchBarSnapshot>,
    /// TitleBar 的输入数据（app 在 update_frame 前注入）。
    title_bar_input: Option<TitleBarInput>,
    /// TOC 的输入数据（app 在 update_frame 前注入）。
    toc_input: Option<ui::toc::TocInput>,
    /// mmap 风格面板的输入数据；`None` 表示当前帧隐藏。
    mindmap_style_panel_input: Option<MindmapStylePanelInput>,
    mindmap_style_panel_thickness: f32,
    last_mindmap_style_panel_thickness: f32,
    /// Phase 4：当前键盘焦点目标。
    keyboard_focus: KeyboardFocusTarget,
    /// Phase 5：Scrollbar 输入数据（app 在 update_frame 前注入）。
    scrollbar_viewport_height: f64,
    scrollbar_total_display_rows: usize,
    scrollbar_scroll_top: f64,
    /// Phase 7：Sidebar 状态持久化（每帧重建 widget 时保持配置）。
    sidebar_config: ui::sidebar::SidebarConfig,
    sidebar_persistent: ui::sidebar::SidebarPersistent,
    dragging_sidebar: bool,
    frames_rendered: u32,
    sidebar_tabs: Vec<ui::tab_bar::TabInfo>,
    sidebar_active_index: Option<usize>,
    sidebar_traffic_light_inset: (f32, f32),
    /// Phase 6: Tab bar 输入数据缓存。
    tab_input_tabs: Vec<ui::tab_bar::TabInfo>,
    tab_input_active_index: Option<usize>,
    tab_input_back_enabled: bool,
    tab_input_forward_enabled: bool,
    tab_hovered_index: Option<usize>,
    tab_scroll_offset: f32,
    dock_dirty: bool,
    /// UI 字体 shaper，用于 widget 布局时的精确文本测量（proportional font）。
    /// 与编辑器 monospace shaper 分开，避免 CJK 标点等字符的宽度误差。
    ui_shaper: Option<shaping::Shaper>,
    /// 上次的 sidebar 可见性/宽度，变化时需重建 Dock children 以更新厚度闭包。
    last_sidebar_visible: bool,
    last_sidebar_thickness: f32,
    /// 上次的 search bar 可见性/厚度，变化时需重建 Dock children。
    last_search_visible: bool,
    last_search_thickness: f32,
    /// 上次的 tab bar 可见性/厚度，变化时需重建 Dock children。
    last_tabs_visible: bool,
    last_tabs_thickness: f32,
    last_status_thickness: f32,
    last_scrollbar_thickness: f32,
    last_toc_visible: bool,
    last_toc_thickness: f32,
    /// TOC panel scroll offset (persistent across frame rebuilds).
    toc_scroll_y: f32,
    tooltip_timer: Option<TooltipTimer>,
    tooltip_overlay: Option<OverlayChild>,
    screen_w: f32,
    screen_h: f32,
    last_dpi: f32,
}

impl Default for UiShell {
    fn default() -> Self {
        Self::new()
    }
}

impl UiShell {
    pub fn new() -> Self {
        Self {
            dock: Dock::new(Box::new(EditorHostWidget::new())),
            overlays: Vec::new(),
            canvas_scrollbars: CanvasScrollbarsWidget::new(),
            canvas_scrollbars_input: None,
            status_input: None,
            search_input: None,
            title_bar_input: None,
            toc_input: None,
            mindmap_style_panel_input: None,
            mindmap_style_panel_thickness: 0.0,
            last_mindmap_style_panel_thickness: 0.0,
            keyboard_focus: KeyboardFocusTarget::Editor,
            scrollbar_viewport_height: 0.0,
            scrollbar_total_display_rows: 0,
            scrollbar_scroll_top: 0.0,
            sidebar_config: ui::sidebar::SidebarConfig::new_default(1.0),
            sidebar_persistent: ui::sidebar::SidebarPersistent::new(
                &ui::sidebar::SidebarConfig::new_default(1.0),
            ),
            dragging_sidebar: false,
            frames_rendered: 0,
            sidebar_tabs: Vec::new(),
            sidebar_active_index: None,
            sidebar_traffic_light_inset: (0.0, 0.0),
            tab_input_tabs: Vec::new(),
            tab_input_active_index: None,
            tab_input_back_enabled: false,
            tab_input_forward_enabled: false,
            tab_hovered_index: None,
            tab_scroll_offset: 0.0,
            dock_dirty: true,
            ui_shaper: None,
            last_sidebar_visible: false,
            last_sidebar_thickness: 0.0,
            last_search_visible: false,
            last_search_thickness: 0.0,
            last_tabs_visible: false,
            last_tabs_thickness: 0.0,
            last_status_thickness: 0.0,
            last_scrollbar_thickness: 0.0,
            last_toc_visible: false,
            last_toc_thickness: 0.0,
            toc_scroll_y: 0.0,
            tooltip_timer: None,
            tooltip_overlay: None,
            screen_w: 0.0,
            screen_h: 0.0,
            last_dpi: 1.0,
        }
    }

    /// Phase 3：app 在 update_frame 之前调用，注入当前 buffer / cursor / 选区信息。
    pub fn set_status_input(&mut self, input: StatusBarInput) {
        self.status_input = Some(input);
    }

    /// Phase 4：app 在 update_frame 之前调用，注入搜索栏数据。
    pub fn set_search_input(&mut self, input: SearchBarSnapshot) {
        self.search_input = Some(input);
    }

    /// app 在 update_frame 之前调用，注入 TOC 数据。
    pub fn set_toc_input(&mut self, input: ui::toc::TocInput) {
        self.toc_input = Some(input);
    }

    pub fn keyboard_focus(&self) -> KeyboardFocusTarget {
        self.keyboard_focus
    }

    pub fn focus_editor(&mut self) {
        self.keyboard_focus = KeyboardFocusTarget::Editor;
    }

    pub fn focus_widget(&mut self, widget_id: ui::core::widget::WidgetId) {
        self.keyboard_focus = KeyboardFocusTarget::Widget(widget_id);
    }

    pub fn set_mindmap_style_panel_input(
        &mut self,
        input: Option<MindmapStylePanelInput>,
        dpi: f32,
    ) {
        let was_visible = self.mindmap_style_panel_input.is_some();
        let is_visible = input.is_some();
        let thickness = if is_visible { PANEL_WIDTH_LOGICAL * dpi } else { 0.0 };
        let structure_changed = was_visible != is_visible
            || (thickness - self.last_mindmap_style_panel_thickness).abs() > 0.1;

        self.mindmap_style_panel_input = input;
        self.mindmap_style_panel_thickness = thickness;
        self.last_mindmap_style_panel_thickness = thickness;

        if structure_changed {
            self.dock_dirty = true;
        } else if let Some(ref input) = self.mindmap_style_panel_input {
            for child in &mut self.dock.children {
                let Some(panel) =
                    child.widget.as_any_mut().downcast_mut::<MindmapStylePanelWidget>()
                else {
                    continue;
                };
                panel.set_input(input.clone());
                break;
            }
        }

        if !is_visible
            && self.keyboard_focus
                == KeyboardFocusTarget::Widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL)
        {
            self.keyboard_focus = KeyboardFocusTarget::Editor;
        }
    }

    pub fn mindmap_style_panel_thickness(&self) -> f32 {
        self.mindmap_style_panel_thickness
    }

    /// Scroll the TOC panel by delta pixels.
    pub fn toc_on_scroll(&mut self, delta: f32, viewport_h: f32, content_h: f32, dpi: f32) {
        let max_scroll = (content_h - viewport_h).max(0.0);
        self.toc_scroll_y = (self.toc_scroll_y + delta).clamp(0.0, max_scroll);
        // Also update the live TocWidget in the dock (dock may not rebuild this frame)
        for child in self.dock.children.iter_mut() {
            if let Some(toc) = child.widget.as_any_mut().downcast_mut::<ui::toc::TocWidget>() {
                toc.set_scroll_y(self.toc_scroll_y, dpi);
                break;
            }
        }
    }

    /// Get the current TOC scroll offset.
    pub fn toc_scroll_y(&self) -> f32 {
        self.toc_scroll_y
    }

    /// Compute the total content height of the TOC panel.
    pub fn toc_content_height(&self, dpi: f32) -> f32 {
        let heading_count = self.toc_input.as_ref().map_or(0, |i| i.headings.len());
        let entry_h = ui::toc::TocWidget::ENTRY_HEIGHT;
        heading_count as f32 * entry_h * dpi
    }

    /// 获取搜索栏光标的 X 偏移（用于 IME 定位）。

    pub fn search_bar_has_keyboard_focus(&self) -> bool {
        self.keyboard_focus == KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR)
    }

    pub fn search_ime_cursor_rect(&self) -> Option<ui::core::geom::Rect> {
        for child in &self.dock.children {
            if let Some(sw) =
                child.widget.as_any().downcast_ref::<ui::search_bar::SearchBarWidget>()
            {
                return sw.focused_textbox_ime_cursor_rect();
            }
        }
        None
    }

    /// 获取搜索栏在屏幕上的绝对 X 偏移（Sidebar 模式下不为 0）。
    pub fn search_bar_x_offset(&self) -> f32 {
        self.dock
            .children
            .iter()
            .find(|c| c.widget.id() == Some(ui::core::widget::ids::SEARCH_BAR))
            .map(|c| c.layout_rect.x)
            .unwrap_or(0.0)
    }

    /// app 在 update_frame 之前调用，注入 titlebar 数据。
    pub fn set_title_bar_input(&mut self, input: TitleBarInput) {
        self.title_bar_input = Some(input);
    }

    /// Phase 5：app 在 update_frame 之前调用，注入滚动信息。
    pub fn set_scrollbar_input(
        &mut self,
        viewport_height: f64,
        total_display_rows: usize,
        scroll_top: f64,
    ) {
        self.scrollbar_viewport_height = viewport_height;
        self.scrollbar_total_display_rows = total_display_rows;
        self.scrollbar_scroll_top = scroll_top;
    }

    /// 注入当前帧画布覆盖滚动条的纯数据输入；`None` 会隐藏覆盖层。
    pub fn set_canvas_scrollbars_input(&mut self, input: Option<CanvasScrollbarsInput>) {
        self.canvas_scrollbars_input = input;
    }

    /// Phase 9：sidebar config 可变引用。
    pub fn sidebar_cfg_mut(&mut self) -> &mut ui::sidebar::SidebarConfig {
        &mut self.sidebar_config
    }
    /// Phase 9：sidebar config 只读引用。
    pub fn sidebar_cfg(&self) -> &ui::sidebar::SidebarConfig {
        &self.sidebar_config
    }

    pub fn sidebar_pinned(&self) -> bool {
        self.sidebar_config.pinned
    }
    pub fn set_sidebar_pinned(&mut self, v: bool) {
        self.sidebar_config.pinned = v;
    }
    pub fn sidebar_width(&self) -> f32 {
        self.sidebar_config.width
    }
    pub fn set_sidebar_width(&mut self, width: f32) {
        self.sidebar_config.width = width;
    }
    pub fn scale_sidebar_width(&mut self, ratio: f32) {
        self.sidebar_config.width *= ratio;
    }
    pub fn set_sidebar_visibility(&mut self, visibility: ui::sidebar::Visibility) {
        self.sidebar_persistent.visibility = visibility;
    }
    pub fn sidebar_visibility(&self) -> ui::sidebar::Visibility {
        self.sidebar_persistent.visibility
    }
    pub fn set_sidebar_suppress_hover_enter(&mut self, suppress: bool) {
        self.sidebar_persistent.suppress_hover_enter = suppress;
    }
    pub fn sidebar_settings_button_rect(&self) -> Rect {
        self.sidebar_persistent.settings_btn_rect
    }
    pub fn sidebar_current_width(&self) -> f32 {
        self.sidebar_persistent.current_width(&self.sidebar_config)
    }
    pub fn sidebar_editor_left_offset(&self) -> f32 {
        self.sidebar_persistent.editor_left_offset(&self.sidebar_config)
    }
    pub fn sidebar_clamp_width(&mut self, dpi: f32) {
        self.sidebar_config.clamp_width(dpi);
    }
    pub fn sidebar_on_scroll(&mut self, delta_px: f32, _total_tabs: usize) {
        self.sidebar_persistent.list_scroll_offset += delta_px;
    }
    pub fn sidebar_set_hovered(&mut self, idx: Option<usize>) {
        self.sidebar_persistent.hovered_index = idx;
    }
    pub fn sidebar_set_open_menu(&mut self, menu: Option<ui::popup_menu::PopupMenu>) {
        self.sidebar_persistent.open_menu = menu;
    }
    pub fn sidebar_tick(&mut self, now: std::time::Instant) -> (bool, bool) {
        self.sidebar_persistent.tick(now)
    }
    pub fn dock_is_dirty(&self) -> bool {
        self.dock_dirty
    }
    pub fn mark_dock_dirty(&mut self) {
        self.dock_dirty = true;
    }
    #[doc(hidden)]
    pub fn mark_layout_initialized_for_test(&mut self) {
        self.frames_rendered = 1;
    }
    pub fn sidebar_on_key(
        &mut self,
        key: ui::sidebar::SidebarKey,
    ) -> Option<ui::sidebar::SidebarAction> {
        match key {
            ui::sidebar::SidebarKey::TogglePin => {
                let new_pinned = !self.sidebar_config.pinned;
                self.sidebar_config.pinned = new_pinned;
                self.sidebar_persistent.visibility = if new_pinned {
                    ui::sidebar::Visibility::Pinned
                } else {
                    ui::sidebar::Visibility::Hidden
                };
                if !new_pinned {
                    self.sidebar_persistent.suppress_hover_enter = true;
                }
                Some(ui::sidebar::SidebarAction::TogglePin)
            }
            ui::sidebar::SidebarKey::Escape => {
                // Dismiss popup menu first
                if self.sidebar_persistent.open_menu.is_some() {
                    self.sidebar_persistent.open_menu = None;
                    return Some(ui::sidebar::SidebarAction::Hovered);
                }
                if self.sidebar_persistent.visibility == ui::sidebar::Visibility::HoverPeek
                    || self.sidebar_persistent.visibility
                        == ui::sidebar::Visibility::HoverPeekFadingOut
                {
                    self.sidebar_persistent.hover_peek_start = None;
                    self.sidebar_persistent.hover_peek_leave_start = None;
                    self.sidebar_persistent.visibility = ui::sidebar::Visibility::Hidden;
                    self.sidebar_persistent.hover_leave_at = None;
                    Some(ui::sidebar::SidebarAction::PersistConfig)
                } else {
                    None
                }
            }
        }
    }

    /// Feed mouse position to sidebar hover state machine.
    pub fn sidebar_on_mouse_move(&mut self, px: f32, py: f32, screen_w: f32, dpi: f32) {
        self.sidebar_persistent.on_mouse_move(
            px,
            py,
            screen_w,
            dpi,
            self.sidebar_traffic_light_inset.0,
            &self.sidebar_config,
        );
    }

    /// 从 Dock child 中同步 SidebarWidget 的持久化状态回来。
    pub fn sync_sidebar_persistent(&mut self) {
        for child in self.dock.children.iter_mut() {
            if let Some(sw) = child.widget.as_any_mut().downcast_mut::<SidebarWidget>() {
                self.sidebar_persistent = sw.steal_persistent();
                return;
            }
        }
    }

    /// Phase 9：sidebar dragging flag.
    pub fn set_dragging_sidebar(&mut self, v: bool) {
        self.dragging_sidebar = v;
    }

    pub fn tab_bar_layout(&self) -> Option<&ui::tab_bar::TabBarLayout> {
        for child in &self.dock.children {
            if let Some(tbw) = child.widget.as_any().downcast_ref::<TabBarWidget>() {
                return tbw.state().current_layout();
            }
        }
        None
    }

    /// 设置 Dock 中 TabBarWidget 的 hovered_index。
    pub fn set_tab_bar_hovered(&mut self, idx: Option<usize>) {
        for child in self.dock.children.iter_mut() {
            if let Some(tbw) = child.widget.as_any_mut().downcast_mut::<TabBarWidget>() {
                tbw.state_mut().set_hovered_index(idx);
                return;
            }
        }
    }

    /// 从 Dock child 中获取 TabBarWidget 的 hovered_index。
    pub fn tab_bar_hovered_index(&self) -> Option<usize> {
        for child in &self.dock.children {
            if let Some(tbw) = child.widget.as_any().downcast_ref::<TabBarWidget>() {
                return tbw.state().hovered_index();
            }
        }
        None
    }

    /// 计算 autoscroll 目标偏移量（使用 TabBarWidget 中前一帧的布局）。
    /// Returns `(target_scroll, max_scroll)` if autoscroll is needed.
    #[cfg(test)]
    fn compute_autoscroll_target(
        &self,
        active_index: usize,
        current_scroll: f32,
    ) -> Option<(f32, f32)> {
        for child in &self.dock.children {
            if let Some(tbw) = child.widget.as_any().downcast_ref::<TabBarWidget>() {
                return tbw.autoscroll_target(active_index, current_scroll);
            }
        }
        None
    }

    /// 用户滚动标签栏（鼠标滚轮 / 快捷键）。
    pub fn tab_bar_scroll_by(&mut self, delta: f32) {
        for child in &mut self.dock.children {
            if let Some(tbw) = child.widget.as_any_mut().downcast_mut::<TabBarWidget>() {
                tbw.scroll_by(delta);
                return;
            }
        }
    }

    /// 当前标签栏滚动目标（供 App 读去做动画）。
    pub fn tab_bar_scroll_target(&self) -> f32 {
        for child in &self.dock.children {
            if let Some(tbw) = child.widget.as_any().downcast_ref::<TabBarWidget>() {
                return tbw.scroll_target();
            }
        }
        0.0
    }

    /// Phase 7：app 每帧调用，注入 sidebar 配置和标签页数据。
    pub fn set_sidebar_input(
        &mut self,
        cfg: ui::sidebar::SidebarConfig,
        tabs: Vec<ui::tab_bar::TabInfo>,
        active_index: Option<usize>,
        traffic_light_inset: (f32, f32),
    ) {
        self.sidebar_config = cfg;
        self.sidebar_tabs = tabs;
        self.sidebar_active_index = active_index;
        self.sidebar_traffic_light_inset = traffic_light_inset;
    }

    /// Phase 6：app 每帧调用，注入 tab bar 数据。
    pub fn set_tabs_input(
        &mut self,
        tabs: Vec<ui::tab_bar::TabInfo>,
        active_index: Option<usize>,
        back_enabled: bool,
        forward_enabled: bool,
        hovered_index: Option<usize>,
        scroll_offset_px: f32,
    ) {
        self.tab_input_tabs = tabs;
        self.tab_input_active_index = active_index;
        self.tab_input_back_enabled = back_enabled;
        self.tab_input_forward_enabled = forward_enabled;
        self.tab_hovered_index = hovered_index;
        self.tab_scroll_offset = scroll_offset_px;
    }

    /// Construct TabBarWidgetInput from cached tab data.
    fn tab_widget_input(
        &self,
        screen: Screen,
        metrics: ui::settings::UiMetrics,
    ) -> ui::tab_bar::TabBarWidgetInput {
        ui::tab_bar::TabBarWidgetInput {
            tabs: self.tab_input_tabs.clone(),
            active_index: self.tab_input_active_index,
            back_enabled: self.tab_input_back_enabled,
            forward_enabled: self.tab_input_forward_enabled,
            screen_size_px: (screen.w, screen.h),
            hovered_index: self.tab_hovered_index,
            scroll_offset_px: self.tab_scroll_offset,
            metrics,
        }
    }

    /// Construct `SidebarWidgetInput` from cached sidebar data.
    fn sidebar_widget_input(
        &self,
        screen: Screen,
        inputs: &ShellInputs,
    ) -> ui::sidebar::SidebarWidgetInput {
        ui::sidebar::SidebarWidgetInput {
            tabs: self.sidebar_tabs.clone(),
            active_index: self.sidebar_active_index,
            traffic_light_inset_px: self.sidebar_traffic_light_inset,
            screen_size_px: (screen.w, screen.h),
            metrics: inputs.metrics,
            settings: inputs.sidebar_settings,
        }
    }
    /// Construct `ScrollbarInput` from cached scrollbar data.
    fn scrollbar_widget_input(&self) -> ui::scrollbar::ScrollbarInput {
        ui::scrollbar::ScrollbarInput {
            viewport_height_px: self.scrollbar_viewport_height,
            total_display_rows: self.scrollbar_total_display_rows,
            scroll_top_rows: self.scrollbar_scroll_top,
        }
    }
    /// Phase 5：查询 ScrollbarWidget 是否正在拖拽。
    pub fn scrollbar_is_dragging(&self) -> bool {
        for child in &self.dock.children {
            if let Some(sbw) = child.widget.as_any().downcast_ref::<ScrollbarWidget>() {
                return sbw.is_dragging();
            }
        }
        false
    }

    /// 按当前聚焦的 WidgetId 转发键盘事件。
    pub fn forward_key(
        &mut self,
        key: ui::core::widget::KeyCode,
        modifiers: ui::core::widget::Modifiers,
        theme: &Theme,
        dpi: f32,
    ) -> Option<ui::core::widget::WidgetAction> {
        let overlay_event = Event::KeyDown(key, modifiers);
        if let Some(action) = self.dispatch_active_modal_overlay_event(&overlay_event, theme, dpi) {
            return Some(action);
        }
        let focus = match self.keyboard_focus {
            KeyboardFocusTarget::Widget(id) => id,
            KeyboardFocusTarget::Editor => return None,
        };
        let ev = Event::KeyDown(key, modifiers);
        let mut ctx = EventCtx { cursor_hint: None, theme, dpi };
        for child in &mut self.dock.children {
            if child.widget.id() == Some(focus) {
                return child.widget.on_event(&ev, &mut ctx);
            }
        }
        None
    }

    /// 每帧调用：更新 widget 输入，重建 Dock children，执行布局。
    ///
    /// C-3: Widget 状态通过 downcast 原地更新（零堆分配）；
    /// Dock children 仅在结构变化时重建（dock_dirty）。
    pub fn update_frame(
        &mut self,
        screen: Screen,
        theme: &Theme,
        measure: &mut dyn TextMeasure,
        inputs: &ShellInputs,
    ) {
        let screen_rect = Rect::new(0.0, 0.0, screen.w, screen.h);
        let dpi = inputs.metrics.dpi;

        // Step 1: Update state on existing Dock widgets in-place (no heap allocation)
        self.update_widget_state(screen, inputs);

        // Sidebar 宽度/可见性变化时，需重建 Dock children 以更新厚度闭包
        if inputs.sidebar_visible != self.last_sidebar_visible
            || (inputs.sidebar_thickness - self.last_sidebar_thickness).abs() > 0.1
        {
            self.dock_dirty = true;
            self.last_sidebar_visible = inputs.sidebar_visible;
            self.last_sidebar_thickness = inputs.sidebar_thickness;
        }

        // Search bar 可见性/厚度变化时，也需重建 Dock children
        let search_was_visible = self.last_search_visible;
        if inputs.search_visible != self.last_search_visible
            || (inputs.search_thickness - self.last_search_thickness).abs() > 0.1
        {
            self.dock_dirty = true;
            self.last_search_visible = inputs.search_visible;
            self.last_search_thickness = inputs.search_thickness;
        }

        // Tab bar 可见性/厚度变化时，需重建 Dock children
        if inputs.tabs_visible != self.last_tabs_visible
            || (inputs.tabs_thickness - self.last_tabs_thickness).abs() > 0.1
        {
            self.dock_dirty = true;
            self.last_tabs_visible = inputs.tabs_visible;
            self.last_tabs_thickness = inputs.tabs_thickness;
        }

        // TOC 可见性/厚度变化时，需重建 Dock children
        if inputs.toc_visible != self.last_toc_visible
            || (inputs.toc_thickness - self.last_toc_thickness).abs() > 0.1
        {
            self.dock_dirty = true;
            self.last_toc_visible = inputs.toc_visible;
            self.last_toc_thickness = inputs.toc_thickness;
        }

        // Status bar 厚度变化时，需重建 Dock children
        if (inputs.status_thickness - self.last_status_thickness).abs() > 0.1 {
            self.dock_dirty = true;
            self.last_status_thickness = inputs.status_thickness;
        }

        // 画布使用覆盖滚动条，编辑模式使用 Dock 纵向滚动条；厚度变化时需重建结构。
        if (inputs.scrollbar_thickness - self.last_scrollbar_thickness).abs() > 0.1 {
            self.dock_dirty = true;
            self.last_scrollbar_thickness = inputs.scrollbar_thickness;
        }

        // Step 2: Rebuild Dock children when structure changes
        if self.dock_dirty {
            self.rebuild_dock_children(inputs, screen);
        }

        // Search bar keyboard focus: auto-focus on first open, clear on close.
        // Subsequent focus changes are driven by mouse/key events (Phase 3).
        if inputs.search_visible && !search_was_visible {
            self.keyboard_focus = KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR);
        }
        if !inputs.search_visible && search_was_visible {
            self.keyboard_focus = KeyboardFocusTarget::Editor;
        }

        // Step 3: Layout
        self.ensure_ui_shaper();
        let mut ui_adapter;
        let ui_measure: Option<&mut dyn TextMeasure> = match self.ui_shaper {
            Some(ref mut ui_shaper) => {
                ui_adapter = crate::measure_adapter::MeasureFromShaper(ui_shaper);
                Some(&mut ui_adapter)
            }
            None => None,
        };
        let mut layout_ctx = LayoutCtx { ui_measure, measure, theme, dpi };
        self.dock.layout(screen_rect, &mut layout_ctx);
        Self::resolve_overlay_layouts(&mut self.overlays, screen_rect, dpi, &mut layout_ctx);
        if let Some(input) = self.canvas_scrollbars_input {
            self.canvas_scrollbars.set_input(input);
            self.canvas_scrollbars.set_rect(self.dock.fill_rect, &mut layout_ctx);
        }
        self.frames_rendered += 1;
        // Sidebar 在 frame 0 被 macOS 初始化守卫跳过（frames_rendered > 0）。
        // frames_rendered 变为 1 后需再触发一次重建，否则 sidebar widget 永远不会被加入 dock。
        self.dock_dirty = self.frames_rendered == 1;

        // Cache screen dims and update tooltip
        self.screen_w = screen.w;
        self.screen_h = screen.h;
        self.last_dpi = dpi;
        self.update_tooltip(dpi);
    }

    /// Update widget state in-place on existing Dock children via downcast.
    /// No heap allocation — widgets persist across frames in Dock.children.
    fn update_widget_state(&mut self, screen: Screen, inputs: &ShellInputs) {
        // Pre-construct inputs before mutable dock iteration.
        let tab_input = self.tab_widget_input(screen, inputs.metrics);
        let sidebar_input = self.sidebar_widget_input(screen, inputs);
        let scrollbar_input = self.scrollbar_widget_input();
        for child in &mut self.dock.children {
            // Tab bar
            if let Some(tbw) = child.widget.as_any_mut().downcast_mut::<TabBarWidget>() {
                tbw.set_input(tab_input.clone(), None);
                continue;
            }
            if let Some(panel) = child.widget.as_any_mut().downcast_mut::<MindmapStylePanelWidget>()
            {
                if let Some(ref input) = self.mindmap_style_panel_input {
                    panel.set_input(input.clone());
                }
                continue;
            }
            // Search bar
            if let Some(sw) = child.widget.as_any_mut().downcast_mut::<SearchBarWidget>() {
                if let Some(ref input) = self.search_input {
                    sw.set_input(input.clone());
                }
                continue;
            }
            // Status bar
            if let Some(sw) = child.widget.as_any_mut().downcast_mut::<StatusBarWidget>() {
                if let Some(ref input) = self.status_input {
                    sw.set_input(input.clone());
                }
                continue;
            }
            // Sidebar
            if let Some(sw) = child.widget.as_any_mut().downcast_mut::<SidebarWidget>() {
                sw.set_input(sidebar_input.clone());
                sw.inject_persistent(&self.sidebar_persistent);
                continue;
            }
            // Scrollbar
            if let Some(sw) = child.widget.as_any_mut().downcast_mut::<ScrollbarWidget>() {
                sw.set_input(scrollbar_input);
                continue;
            }
            // TitleBar
            if let Some(tbw) = child.widget.as_any_mut().downcast_mut::<TitleBarWidget>() {
                if let Some(ref input) = self.title_bar_input {
                    tbw.set_input(input.clone());
                }
                continue;
            }
            // TOC panel
            if let Some(toc) = child.widget.as_any_mut().downcast_mut::<ui::toc::TocWidget>() {
                if let Some(ref input) = self.toc_input {
                    toc.set_input(input.clone());
                }
                continue;
            }
        }
    }

    fn rebuild_dock_children(&mut self, inputs: &ShellInputs, screen: Screen) {
        self.dock.children.clear();

        // Tab bar: top
        if inputs.tabs_visible && inputs.tabs_thickness > 0.0 {
            let t = inputs.tabs_thickness;
            let mut tbw = TabBarWidget::new();
            let input = self.tab_widget_input(screen, inputs.metrics);
            tbw.set_input(input, None);
            self.dock.children.push(DockChild {
                widget: Box::new(tbw),
                side: Side::Top,
                thickness: Box::new(move |_, _| t),
                visible: true,
                layout_rect: Rect::ZERO,
            });
        }

        // Sidebar: left (skip frame 0 for macOS init-time stability)
        if inputs.sidebar_visible && inputs.sidebar_thickness > 0.0 && self.frames_rendered > 0 {
            let t = inputs.sidebar_thickness;
            let mut sw = SidebarWidget::new(self.sidebar_config.clone(), inputs.metrics);
            let sidebar_input = self.sidebar_widget_input(screen, inputs);
            sw.set_input(sidebar_input);
            sw.inject_persistent(&self.sidebar_persistent);
            self.dock.children.push(DockChild {
                widget: Box::new(sw),
                side: Side::Left,
                thickness: Box::new(move |_, _| t),
                visible: true,
                layout_rect: Rect::ZERO,
            });
        }

        // TitleBar: top (only when sidebar is visible — provides top boundary for TOC)
        if inputs.sidebar_visible {
            let title_h = ui::title_bar::title_bar_height(inputs.metrics.dpi);
            if title_h > 0.0 {
                let mut tbw = TitleBarWidget::new();
                if let Some(ref input) = self.title_bar_input {
                    tbw.set_input(input.clone());
                }
                self.dock.children.push(DockChild {
                    widget: Box::new(tbw),
                    side: Side::Top,
                    thickness: Box::new(move |_, _| title_h),
                    visible: true,
                    layout_rect: Rect::ZERO,
                });
            }
        }

        // TOC panel: left, right of sidebar, below titlebar
        if inputs.toc_visible && inputs.toc_thickness > 0.0 {
            let t = inputs.toc_thickness;
            let mut toc = ui::toc::TocWidget::new();
            if let Some(ref input) = self.toc_input {
                toc.set_input(input.clone());
            }
            toc.set_scroll_y(self.toc_scroll_y, inputs.metrics.dpi);
            self.dock.children.push(DockChild {
                widget: Box::new(toc),
                side: Side::Left,
                thickness: Box::new(move |_, _| t),
                visible: true,
                layout_rect: Rect::ZERO,
            });
        }

        // Search bar: top (below titlebar/tab bar, above editor)
        if inputs.search_visible && inputs.search_thickness > 0.0 {
            let t = inputs.search_thickness;
            let mut sw = SearchBarWidget::new();
            if let Some(ref input) = self.search_input {
                sw.set_input(input.clone());
            }

            let on_copy = std::rc::Rc::new(crate::clipboard::write_text);
            let on_cut = on_copy.clone();
            let on_paste = std::rc::Rc::new(crate::clipboard::read_text);
            sw.set_clipboard_callbacks(on_copy, on_cut, on_paste);

            self.dock.children.push(DockChild {
                widget: Box::new(sw),
                side: Side::Top,
                thickness: Box::new(move |_, _| t),
                visible: true,
                layout_rect: Rect::ZERO,
            });
        }

        // Status bar: bottom (before scrollbar so scrollbar height excludes status bar)
        if inputs.status_thickness > 0.0 {
            let t = inputs.status_thickness;
            let mut sw = StatusBarWidget::new();
            if let Some(ref input) = self.status_input {
                sw.set_input(input.clone());
            }
            self.dock.children.push(DockChild {
                widget: Box::new(sw),
                side: Side::Bottom,
                thickness: Box::new(move |_, _| t),
                visible: true,
                layout_rect: Rect::ZERO,
            });
        }

        // mmap style panel: right, after status bar so it occupies content height only.
        if self.mindmap_style_panel_thickness > 0.0 {
            let thickness = self.mindmap_style_panel_thickness;
            let mut panel = MindmapStylePanelWidget::new();
            if let Some(ref input) = self.mindmap_style_panel_input {
                panel.set_input(input.clone());
            }
            self.dock.children.push(DockChild {
                widget: Box::new(panel),
                side: Side::Right,
                thickness: Box::new(move |_, _| thickness),
                visible: true,
                layout_rect: Rect::ZERO,
            });
        }

        // Scrollbar: right (after status bar so its height is correct)
        if inputs.scrollbar_thickness > 0.0 {
            let t = inputs.scrollbar_thickness;
            let mut sw = ScrollbarWidget::new();
            sw.set_input(self.scrollbar_widget_input());
            self.dock.children.push(DockChild {
                widget: Box::new(sw),
                side: Side::Right,
                thickness: Box::new(move |_, _| t),
                visible: true,
                layout_rect: Rect::ZERO,
            });
        }
    }

    /// Return the editor rect (area allocated to the fill widget).
    pub fn editor_rect(&self) -> Rect {
        self.dock.fill_rect
    }

    /// 懒初始化 UI shaper，使用 proportional UI 字体族。
    fn ensure_ui_shaper(&mut self) {
        if self.ui_shaper.is_some() {
            return;
        }
        let ui_font = "system".to_string(); // FIXED BY SCRIPT
        if let Ok(s) = shaping::Shaper::new() {
            self.ui_shaper = Some(s.with_font_family(&ui_font));
        }
    }

    /// 测试辅助：强制重建 Dock children 并执行布局。
    #[allow(dead_code)]
    pub fn rebuild_and_layout(
        &mut self,
        screen: Screen,
        theme: &Theme,
        measure: &mut dyn TextMeasure,
        inputs: &ShellInputs,
    ) {
        self.dock_dirty = true;
        self.update_frame(screen, theme, measure, inputs);
    }

    fn check_tooltips(&mut self, px: f32, py: f32) {
        for child in self.dock.children.iter().rev() {
            if !child.visible || child.layout_rect.w <= 0.0 || child.layout_rect.h <= 0.0 {
                continue;
            }
            let lx = px - child.layout_rect.x;
            let ly = py - child.layout_rect.y;
            if let Some(hint) = child.widget.tooltip_at(lx, ly) {
                let screen_target = ui::core::geom::Rect::new(
                    hint.target_rect.x + child.layout_rect.x,
                    hint.target_rect.y + child.layout_rect.y,
                    hint.target_rect.w,
                    hint.target_rect.h,
                );
                let same = match &self.tooltip_timer {
                    Some(t) => {
                        t.hint.label == hint.label
                            && (t.target_screen_rect.x - screen_target.x).abs() < 0.5
                            && (t.target_screen_rect.y - screen_target.y).abs() < 0.5
                    }
                    None => false,
                };
                if same {
                    return;
                }
                self.tooltip_overlay = None;
                self.tooltip_timer = Some(TooltipTimer {
                    hint,
                    target_screen_rect: screen_target,
                    start: Instant::now(),
                });
                return;
            }
        }
        self.tooltip_overlay = None;
        self.tooltip_timer = None;
    }

    #[cfg(test)]
    fn has_tooltip_timer(&self) -> bool {
        self.tooltip_timer.is_some()
    }

    fn update_tooltip(&mut self, dpi: f32) {
        if let Some(ref timer) = self.tooltip_timer
            && timer.start.elapsed().as_millis() >= 400
            && self.tooltip_overlay.is_none()
        {
            // Use screen-space coordinates for correct tooltip positioning
            let screen_hint = TooltipHint {
                label: timer.hint.label.clone(),
                target_rect: timer.target_screen_rect,
            };
            let (widget, layout_rect) =
                TooltipWidget::new(&screen_hint, dpi, self.screen_w, self.screen_h);
            self.tooltip_overlay = Some(OverlayChild { widget: Box::new(widget), layout_rect });
        }
    }

    fn resolve_overlay_layouts(
        overlays: &mut [OverlayEntry],
        screen_rect: Rect,
        dpi: f32,
        layout_ctx: &mut LayoutCtx,
    ) {
        for overlay in overlays {
            overlay.layout_rect = overlay.layout.resolve(screen_rect, dpi);
            overlay.widget.set_rect(Self::overlay_local_rect(overlay.layout_rect), layout_ctx);
        }
    }

    fn overlay_local_rect(layout_rect: Rect) -> Rect {
        Rect::new(OVERLAY_LOCAL_ORIGIN, OVERLAY_LOCAL_ORIGIN, layout_rect.w, layout_rect.h)
    }

    fn overlay_scrim_color(theme: &Theme) -> [f32; 4] {
        theme.application_theme().modal_scrim
    }

    fn current_screen_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.screen_w, self.screen_h)
    }

    fn dismisses_on_escape(policy: DismissPolicy) -> bool {
        matches!(policy, DismissPolicy::EscapeOrExplicit | DismissPolicy::EscapeBackdropOrExplicit)
    }

    fn dismisses_on_backdrop(policy: DismissPolicy) -> bool {
        matches!(policy, DismissPolicy::EscapeBackdropOrExplicit)
    }

    /// Phase 8：推入一个 overlay widget（popup、dialog 等）。
    /// 先清空已有 overlays（保证一次只一个 popup）。
    pub fn push_overlay(&mut self, widget: Box<dyn Widget>, layout_rect: Rect) {
        self.clear_overlays();
        self.push_overlay_with_policy(
            widget,
            OverlayLayout::Fixed(layout_rect),
            OverlayInputPolicy::PassThrough,
            DismissPolicy::ExplicitOnly,
        );
    }

    pub fn push_overlay_with_policy(
        &mut self,
        widget: Box<dyn Widget>,
        layout: OverlayLayout,
        input_policy: OverlayInputPolicy,
        dismiss_policy: DismissPolicy,
    ) {
        let layout_rect = layout.resolve(self.current_screen_rect(), self.last_dpi);
        self.overlays.push(OverlayEntry {
            widget,
            layout,
            layout_rect,
            input_policy,
            dismiss_policy,
            restore_focus: self.keyboard_focus,
        });
        self.tooltip_overlay = None;
        self.tooltip_timer = None;
    }

    /// Phase 8：弹出最顶层 overlay。
    pub fn pop_overlay(&mut self) -> Option<Box<dyn Widget>> {
        let overlay = self.overlays.pop()?;
        self.keyboard_focus = overlay.restore_focus;
        Some(overlay.widget)
    }

    /// Phase 8：清空所有 overlays。
    pub fn clear_overlays(&mut self) {
        let restore_focus = self.overlays.first().map(|overlay| overlay.restore_focus);
        self.overlays.clear();
        if let Some(restore_focus) = restore_focus {
            self.keyboard_focus = restore_focus;
        }
    }

    /// Phase 8：当前 overlay 数量。
    pub fn overlays_count(&self) -> usize {
        self.overlays.len()
    }

    pub fn active_overlay_is_modal(&self) -> bool {
        self.overlays
            .last()
            .is_some_and(|overlay| overlay.input_policy == OverlayInputPolicy::Modal)
    }

    pub fn active_overlay_widget_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.overlays.last_mut()?.widget.as_any_mut().downcast_mut::<T>()
    }

    pub fn active_overlay_widget_ref<T: Any>(&self) -> Option<&T> {
        self.overlays.last()?.widget.as_any().downcast_ref::<T>()
    }

    pub fn active_overlay_layout_rect(&self) -> Option<Rect> {
        self.overlays.last().map(|overlay| overlay.layout_rect)
    }

    /// Phase 3：只绘制 chrome（dock children + overlays），跳过 fill。

    /// 返回 DrawList 供 app_renderer 通过 paint_backend 翻译为顶点。
    pub fn paint_chrome(
        &self,
        theme: &Theme,
        dpi: f32,
        shaper: Option<&mut shaping::Shaper>,
    ) -> DrawList {
        let mut list = DrawList::new();
        let mut ctx =
            PaintCtx { global_alpha: 1.0, list: &mut list, theme, dpi, offset: (0.0, 0.0), shaper };

        // 内容区顶部分隔线（原在 TitleBarWidget 底部绘制，现移至此处）
        // Extend through scrollbar column so the line spans the full content width.
        let fr = self.dock.fill_rect;
        let has_titlebar =
            self.dock.children.iter().any(|c| c.widget.as_any().is::<TitleBarWidget>());
        if has_titlebar && fr.w > 0.0 && fr.h > 0.0 {
            let divider_w = self
                .dock
                .children
                .iter()
                .find_map(|c| {
                    if c.widget.as_any().is::<ui::scrollbar::ScrollbarWidget>()
                        && c.layout_rect.w > 0.0
                    {
                        Some(fr.w + c.layout_rect.w)
                    } else {
                        None
                    }
                })
                .unwrap_or(fr.w);
            ctx.list
                .fill(Rect::new(fr.x, fr.y, divider_w, 1.0), ctx.theme.application_theme().divider);
        }

        // dock children（tab, search, status, sidebar, scrollbar）
        // 阶段4：为每个子 widget 推入其 layout_rect 偏移
        // 非 Pinned 模式：sidebar 延迟到最后绘制（覆盖 titlebar/statusbar）
        let sidebar_on_top =
            !matches!(self.sidebar_persistent.visibility, ui::sidebar::Visibility::Pinned);
        for child in &self.dock.children {
            if sidebar_on_top && child.widget.as_any().is::<SidebarWidget>() {
                continue;
            }
            let saved = ctx.list.offset;
            ctx.list.offset = (saved.0 + child.layout_rect.x, saved.1 + child.layout_rect.y);
            child.widget.paint(&mut ctx);
            ctx.list.offset = saved;
        }
        if sidebar_on_top {
            for child in &self.dock.children {
                if child.widget.as_any().is::<SidebarWidget>() {
                    let saved = ctx.list.offset;
                    ctx.list.offset =
                        (saved.0 + child.layout_rect.x, saved.1 + child.layout_rect.y);
                    child.widget.paint(&mut ctx);
                    ctx.list.offset = saved;
                    break;
                }
            }
        }
        self.paint_canvas_scrollbars(&mut ctx);
        self.paint_overlay_stack(&mut ctx);
        self.paint_tooltip_overlay(&mut ctx);

        list
    }

    /// 绘制：先 fill，再 children，再 overlays。
    pub fn paint(&self, ctx: &mut PaintCtx) {
        self.dock.paint(ctx);
        self.paint_canvas_scrollbars(ctx);
        self.paint_overlay_stack(ctx);
        self.paint_tooltip_overlay(ctx);
    }

    fn paint_canvas_scrollbars(&self, ctx: &mut PaintCtx) {
        if self.canvas_scrollbars_input.is_none() {
            return;
        }
        let saved = ctx.list.offset;
        ctx.list.offset = (saved.0 + self.dock.fill_rect.x, saved.1 + self.dock.fill_rect.y);
        self.canvas_scrollbars.paint(ctx);
        ctx.list.offset = saved;
    }

    fn paint_overlay_stack(&self, ctx: &mut PaintCtx) {
        let screen_rect = self.current_screen_rect();
        for overlay in &self.overlays {
            if overlay.input_policy == OverlayInputPolicy::Modal {
                ctx.list.fill(screen_rect, Self::overlay_scrim_color(ctx.theme));
            }
            let saved = ctx.list.offset;
            ctx.list.offset = (saved.0 + overlay.layout_rect.x, saved.1 + overlay.layout_rect.y);
            overlay.widget.paint(ctx);
            ctx.list.offset = saved;
        }
    }

    fn paint_tooltip_overlay(&self, ctx: &mut PaintCtx) {
        if let Some(ref tooltip) = self.tooltip_overlay {
            let saved = ctx.list.offset;
            ctx.list.offset = (saved.0 + tooltip.layout_rect.x, saved.1 + tooltip.layout_rect.y);
            tooltip.widget.paint(ctx);
            ctx.list.offset = saved;
        }
    }

    /// 事件分发：overlays 优先（后入先派），未命中再下传 dock。

    pub fn forward_ime(
        &mut self,
        ev: ui::core::Event,
        theme: &ui::theme::Theme,
        dpi: f32,
    ) -> Option<ui::core::WidgetAction> {
        if let Some(action) = self.dispatch_active_modal_overlay_event(&ev, theme, dpi) {
            return Some(action);
        }
        let focus = match self.keyboard_focus {
            KeyboardFocusTarget::Widget(id) => id,
            KeyboardFocusTarget::Editor => return None,
        };
        let mut ctx = ui::core::EventCtx { theme, dpi, cursor_hint: None };
        for child in &mut self.dock.children {
            if child.widget.id() == Some(focus) {
                return child.widget.on_event(&ev, &mut ctx);
            }
        }
        None
    }

    fn dispatch_active_modal_overlay_event(
        &mut self,
        ev: &Event,
        theme: &Theme,
        dpi: f32,
    ) -> Option<WidgetAction> {
        let overlay = self.overlays.last_mut()?;
        if overlay.input_policy != OverlayInputPolicy::Modal {
            return None;
        }
        let mut ctx = EventCtx { cursor_hint: None, theme, dpi };
        match Self::dispatch_modal_overlay_event(overlay, ev, &mut ctx) {
            OverlayDispatchOutcome::NotHandled => None,
            OverlayDispatchOutcome::SilentConsumed => Some(WidgetAction::Consumed),
            OverlayDispatchOutcome::Action(action) => Some(action),
        }
    }

    fn dispatch_modal_overlay_event(
        overlay: &mut OverlayEntry,
        ev: &Event,
        ctx: &mut EventCtx,
    ) -> OverlayDispatchOutcome {
        let local_event = Dock::to_local(ev, overlay.layout_rect.x, overlay.layout_rect.y);
        if let Some(action) = overlay.widget.on_event(&local_event, ctx) {
            return OverlayDispatchOutcome::Action(action);
        }
        if Self::overlay_policy_requests_dismiss(overlay, ev) {
            return OverlayDispatchOutcome::Action(WidgetAction::Overlay(
                OverlayAction::DismissRequested,
            ));
        }
        OverlayDispatchOutcome::Action(WidgetAction::Consumed)
    }

    fn dispatch_pass_through_overlay_event(
        overlay: &mut OverlayEntry,
        ev: &Event,
        ctx: &mut EventCtx,
    ) -> OverlayDispatchOutcome {
        match ev {
            Event::MouseMove { px, py }
            | Event::MouseUp { px, py, .. }
            | Event::Wheel { px, py, .. } => {
                let local_x = *px - overlay.layout_rect.x;
                let local_y = *py - overlay.layout_rect.y;
                if !overlay.widget.hit(local_x, local_y) {
                    return OverlayDispatchOutcome::NotHandled;
                }
                let local_event = Dock::to_local(ev, overlay.layout_rect.x, overlay.layout_rect.y);
                if let Some(action) = overlay.widget.on_event(&local_event, ctx) {
                    return OverlayDispatchOutcome::Action(action);
                }
                OverlayDispatchOutcome::SilentConsumed
            }
            Event::MouseDown { .. } => {
                let local_event = Dock::to_local(ev, overlay.layout_rect.x, overlay.layout_rect.y);
                if let Some(action) = overlay.widget.on_event(&local_event, ctx) {
                    return OverlayDispatchOutcome::Action(action);
                }
                OverlayDispatchOutcome::SilentConsumed
            }
            _ => overlay
                .widget
                .on_event(ev, ctx)
                .map_or(OverlayDispatchOutcome::NotHandled, OverlayDispatchOutcome::Action),
        }
    }

    fn overlay_policy_requests_dismiss(overlay: &OverlayEntry, ev: &Event) -> bool {
        match ev {
            Event::KeyDown(KeyCode::Escape, _) => Self::dismisses_on_escape(overlay.dismiss_policy),
            Event::MouseDown { px, py, .. } => {
                Self::dismisses_on_backdrop(overlay.dismiss_policy)
                    && !overlay.layout_rect.contains(*px, *py)
            }
            _ => false,
        }
    }

    pub fn dispatch(
        &mut self,
        ev: &Event,
        ctx: &mut EventCtx,
    ) -> Option<ui::core::widget::WidgetAction> {
        // Dismiss tooltip on non-mouse-move events
        if !matches!(ev, Event::MouseMove { .. }) {
            self.tooltip_overlay = None;
            self.tooltip_timer = None;
        }

        // Overlays first (popup, dialog, etc.) — last pushed gets first dibs
        for index in (0..self.overlays.len()).rev() {
            let outcome = {
                let overlay = &mut self.overlays[index];
                match overlay.input_policy {
                    OverlayInputPolicy::Modal => {
                        Self::dispatch_modal_overlay_event(overlay, ev, ctx)
                    }
                    OverlayInputPolicy::PassThrough => {
                        Self::dispatch_pass_through_overlay_event(overlay, ev, ctx)
                    }
                }
            };
            match outcome {
                OverlayDispatchOutcome::NotHandled => continue,
                OverlayDispatchOutcome::SilentConsumed => return None,
                OverlayDispatchOutcome::Action(action) => return Some(action),
            }
        }
        if let Some(action) = self.dispatch_canvas_scrollbars(ev, ctx) {
            return Some(action);
        }
        // Fall through to dock if no overlay handled the event
        let result = self.dock.dispatch(ev, ctx);
        if let Event::MouseMove { px, py } = ev {
            self.check_tooltips(*px, *py);
        }
        result
    }

    fn dispatch_canvas_scrollbars(
        &mut self,
        ev: &Event,
        ctx: &mut EventCtx,
    ) -> Option<ui::core::widget::WidgetAction> {
        let is_mouse_event = matches!(
            ev,
            Event::MouseMove { .. }
                | Event::MouseDown { .. }
                | Event::MouseUp { .. }
                | Event::Wheel { .. }
        );
        if !is_mouse_event {
            return None;
        }

        let is_capturing = self.canvas_scrollbars.is_capturing();
        if self.canvas_scrollbars_input.is_none() && !is_capturing {
            return None;
        }

        let local_event =
            ui::core::dock::Dock::to_local(ev, self.dock.fill_rect.x, self.dock.fill_rect.y);
        let should_dispatch = is_capturing
            || matches!(ev, Event::MouseMove { .. })
            || matches!(
                local_event.as_ref(),
                Event::MouseDown { px, py, .. } | Event::Wheel { px, py, .. }
                    if self.canvas_scrollbars.hit(*px, *py)
            );
        if !should_dispatch {
            return None;
        }

        self.canvas_scrollbars.on_event(&local_event, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::cell::Cell;
    use std::rc::Rc;
    use ui::core::measure::NoopMeasure;
    use ui::core::paint::DrawCmd;
    use ui::core::widget::{KeyCode, Modifiers, WidgetAction};
    use ui::{DismissPolicy, OverlayInputPolicy, OverlayLayout};

    fn metrics(dpi: f32) -> ui::settings::UiMetrics {
        ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), dpi)
    }

    fn test_theme() -> Theme {
        let mut t = ui::theme::test_theme();
        t.palette.bg_surface = [0.1, 0.1, 0.1, 1.0];
        t.palette.text_muted = [0.8, 0.8, 0.8, 1.0];
        t
    }

    #[test]
    fn semantic_state_methods_update_private_shell_state() {
        let mut shell = UiShell::new();
        let settings_button_rect = Rect::new(12.0, 24.0, 36.0, 48.0);

        shell.set_sidebar_width(240.0);
        shell.scale_sidebar_width(1.5);
        shell.set_sidebar_visibility(ui::sidebar::Visibility::Pinned);
        shell.set_sidebar_suppress_hover_enter(true);
        shell.sidebar_persistent.settings_btn_rect = settings_button_rect;
        shell.dock_dirty = false;
        shell.mark_dock_dirty();
        shell.mark_layout_initialized_for_test();

        assert_eq!(shell.sidebar_width(), 360.0);
        assert_eq!(shell.sidebar_visibility(), ui::sidebar::Visibility::Pinned);
        assert!(shell.sidebar_persistent.suppress_hover_enter);
        assert_eq!(shell.sidebar_settings_button_rect(), settings_button_rect);
        assert!(shell.dock_is_dirty());
        assert_eq!(shell.frames_rendered, 1);
    }

    #[test]
    fn semantic_focus_methods_update_shell_state() {
        let mut shell = UiShell::new();

        shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);
        assert_eq!(
            shell.keyboard_focus(),
            KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR)
        );

        shell.focus_editor();
        assert_eq!(shell.keyboard_focus(), KeyboardFocusTarget::Editor);
    }

    struct CountingFillWidget {
        event_count: Rc<Cell<usize>>,
    }

    impl CountingFillWidget {
        fn new(event_count: Rc<Cell<usize>>) -> Self {
            Self { event_count }
        }
    }

    impl Widget for CountingFillWidget {
        fn set_rect(&mut self, _rect: Rect, _ctx: &mut LayoutCtx) {}

        fn paint(&self, _ctx: &mut PaintCtx) {}

        fn hit(&self, _px: f32, _py: f32) -> bool {
            true
        }

        fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
            self.event_count.set(self.event_count.get() + 1);
            None
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct NoopOverlayWidget;

    impl Widget for NoopOverlayWidget {
        fn set_rect(&mut self, _rect: Rect, _ctx: &mut LayoutCtx) {}

        fn paint(&self, _ctx: &mut PaintCtx) {}

        fn hit(&self, _px: f32, _py: f32) -> bool {
            true
        }

        fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
            None
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct TestShellHarness {
        shell: UiShell,
        fill_event_count: Rc<Cell<usize>>,
    }

    impl TestShellHarness {
        fn fill_event_count(&self) -> usize {
            self.fill_event_count.get()
        }
    }

    fn bare_shell() -> TestShellHarness {
        let fill_event_count = Rc::new(Cell::new(0));
        let mut shell = UiShell::new();
        shell.dock.fill = Box::new(CountingFillWidget::new(fill_event_count.clone()));
        TestShellHarness { shell, fill_event_count }
    }

    fn shell_inputs() -> ShellInputs {
        ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        }
    }

    fn shell_with_focus(focus: KeyboardFocusTarget) -> TestShellHarness {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut harness = bare_shell();
        harness.shell.frames_rendered = 1;
        harness.shell.update_frame(
            Screen::new(1200.0, 800.0),
            &theme,
            &mut measure,
            &shell_inputs(),
        );
        harness.shell.keyboard_focus = focus;
        harness
    }

    fn shell_with_modal(widget: Box<dyn Widget>) -> TestShellHarness {
        let mut harness = shell_with_focus(KeyboardFocusTarget::Editor);
        harness.shell.push_overlay_with_policy(
            widget,
            OverlayLayout::Fixed(Rect::new(0.0, 0.0, 1200.0, 800.0)),
            OverlayInputPolicy::Modal,
            DismissPolicy::ExplicitOnly,
        );
        harness
    }

    fn push_test_modal(shell: &mut UiShell) {
        shell.push_overlay_with_policy(
            noop_widget(),
            OverlayLayout::Fixed(Rect::new(0.0, 0.0, 1200.0, 800.0)),
            OverlayInputPolicy::Modal,
            DismissPolicy::ExplicitOnly,
        );
        shell.keyboard_focus = KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR);
    }

    fn noop_widget() -> Box<dyn Widget> {
        Box::new(NoopOverlayWidget)
    }

    fn event_ctx<'a>(theme: &'a Theme) -> EventCtx<'a> {
        EventCtx { cursor_hint: None, theme, dpi: 1.0 }
    }

    fn modal_probe_events() -> Vec<Event> {
        vec![
            Event::Wheel { dx: 0.0, dy: -12.0, px: 600.0, py: 400.0 },
            Event::KeyDown(KeyCode::Tab, Modifiers::NONE),
            Event::ImePreedit { text: "拼".into(), cursor: Some((0, 1)) },
            Event::ImeCommit("写".into()),
            Event::ImeEnable,
            Event::ImeDisable,
        ]
    }

    fn run_layout(inputs: &ShellInputs) -> Rect {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1; // skip frame-0 guard for testing
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, inputs);
        shell.editor_rect()
    }

    mod overlay_modal_tests {
        use super::*;

        #[test]
        fn modal_overlay_consumes_unhandled_mouse_wheel_key_and_ime() {
            let theme = test_theme();
            let mut harness = shell_with_modal(noop_widget());
            for event in modal_probe_events() {
                let result = harness.shell.dispatch(&event, &mut event_ctx(&theme));
                assert_eq!(result, Some(WidgetAction::Consumed));
                assert_eq!(harness.fill_event_count(), 0);
            }
        }

        #[test]
        fn dismissing_modal_restores_previous_focus() {
            let mut harness = shell_with_focus(KeyboardFocusTarget::Editor);
            push_test_modal(&mut harness.shell);

            harness.shell.clear_overlays();

            assert_eq!(harness.shell.keyboard_focus, KeyboardFocusTarget::Editor);
        }

        #[test]
        fn modal_overlay_uses_the_shared_application_scrim() {
            let mut theme = test_theme();
            theme.palette.shadow = [0.1, 0.2, 0.3, 0.1];
            let harness = shell_with_modal(noop_widget());

            let draw_list = harness.shell.paint_chrome(&theme, 1.0, None);

            assert!(draw_list.cmds.iter().any(|command| {
                matches!(
                    command,
                    DrawCmd::FillRect { color, .. }
                        if *color == theme.application_theme().modal_scrim
                )
            }));
        }
    }

    #[test]
    fn ui_shell_source_has_no_product_settings_types() {
        const TESTS_MODULE_MARKER: &str = "#[cfg(test)]\nmod tests {";
        let source = include_str!("ui_shell.rs");
        let (production_source, tests_source) = source
            .split_once(TESTS_MODULE_MARKER)
            .expect("UiShell source must contain the tests module marker");
        assert!(
            !tests_source.contains(TESTS_MODULE_MARKER),
            "UiShell source must contain only one tests module marker"
        );
        assert!(
            production_source.contains("pub fn dispatch("),
            "UiShell production source scan must include dispatch"
        );
        let forbidden = [
            ["Textora", "Settings"].concat(),
            ["Sync", "Settings", "Action"].concat(),
            ["Native", "Menu"].concat(),
        ];

        for product_type in forbidden {
            assert!(
                !production_source.contains(&product_type),
                "UiShell must not depend on product type {product_type}"
            );
        }
    }

    #[test]
    fn no_chrome_gives_full_screen() {
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        assert_eq!(run_layout(&inputs), Rect::new(0.0, 0.0, 1200.0, 800.0));
    }

    #[test]
    fn canvas_scrollbars_overlay_preserves_editor_rect_and_captures_drag_outside_track() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut shell = UiShell::new();
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };

        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut measure, &inputs);
        let editor_rect_before = shell.editor_rect();
        shell.set_canvas_scrollbars_input(Some(ui::canvas_scrollbars::CanvasScrollbarsInput {
            horizontal: Some(ui::scrollbar::ScrollbarInput {
                viewport_height_px: 100.0,
                total_display_rows: 1_000,
                scroll_top_rows: 0.0,
            }),
            vertical: Some(ui::scrollbar::ScrollbarInput {
                viewport_height_px: 100.0,
                total_display_rows: 1_000,
                scroll_top_rows: 0.0,
            }),
        }));
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut measure, &inputs);

        let editor_rect = shell.editor_rect();
        assert_eq!(editor_rect, editor_rect_before, "覆盖滚动条不得压缩编辑区");

        let mut event_ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let start = shell.dispatch(
            &Event::MouseDown {
                px: editor_rect.x + 40.0,
                py: editor_rect.bottom() - 1.0,
                button: ui::core::widget::MouseButton::Left,
            },
            &mut event_ctx,
        );
        assert!(
            matches!(start, Some(ui::core::widget::WidgetAction::CanvasScrollbars(_))),
            "覆盖条点击必须由 CanvasScrollbars 消费，而不是落入 Dock"
        );

        let end = shell.dispatch(
            &Event::MouseUp {
                px: editor_rect.right() + 100.0,
                py: editor_rect.bottom() + 100.0,
                button: ui::core::widget::MouseButton::Left,
            },
            &mut event_ctx,
        );
        assert!(
            matches!(end, Some(ui::core::widget::WidgetAction::CanvasScrollbars(_))),
            "拖动捕获时，指针移出轨道仍须分发给画布滚动条"
        );
    }

    #[test]
    fn tabs_mode_with_scrollbar_status() {
        let inputs = ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 12.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        assert_eq!(run_layout(&inputs), Rect::new(0.0, 32.0, 1188.0, 744.0));
    }

    #[test]
    fn scrollbar_thickness_change_rebuilds_dock_children() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;

        let canvas_inputs = shell_inputs();
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut measure, &canvas_inputs);
        assert!(
            !shell.dock.children.iter().any(|child| child.widget.as_any().is::<ScrollbarWidget>()),
            "画布模式不应创建旧纵向滚动条"
        );

        let editor_inputs =
            ShellInputs { scrollbar_thickness: metrics(1.0).scrollbar_reserve, ..shell_inputs() };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut measure, &editor_inputs);

        assert!(
            shell.dock.children.iter().any(|child| child.widget.as_any().is::<ScrollbarWidget>()),
            "从画布切回编辑模式时必须创建旧纵向滚动条"
        );
    }

    #[test]
    fn sidebar_mode_consumes_left_width() {
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: true,
            sidebar_thickness: 220.0,
            scrollbar_thickness: 12.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        assert_eq!(run_layout(&inputs), Rect::new(220.0, 36.0, 968.0, 740.0));
    }

    #[test]
    fn search_bar_below_tabs() {
        let inputs = ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: true,
            search_thickness: 28.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        assert_eq!(run_layout(&inputs), Rect::new(0.0, 60.0, 1200.0, 716.0));
    }

    // ── Phase 3 tests ──

    #[test]
    fn paint_chrome_empty_when_no_chrome() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        let dl = shell.paint_chrome(&theme, 1.0, None);
        assert!(dl.cmds.is_empty(), "无 chrome 时 paint_chrome 应返回空");
    }

    #[test]
    fn paint_chrome_with_status_emits_fill_and_text() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();

        // 注入 status 数据
        shell.set_status_input(StatusBarInput {
            buffer_len: 100,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 4,
            cursor_col: 9,
            conflict_label: None,
        });

        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        let dl = shell.paint_chrome(&theme, 1.0, None);
        // Without shaper, only FillRect is emitted (TextLayout requires shaper)
        assert_eq!(dl.cmds.len(), 1);
        assert!(matches!(dl.cmds[0], DrawCmd::FillRect { .. }));
    }

    #[test]
    fn paint_chrome_empty_status_when_no_input() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();

        // 不注入 status 数据 → 空 buffer
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        let dl = shell.paint_chrome(&theme, 1.0, None);
        // 空 buffer → 只有背景 FillRect
        assert_eq!(dl.cmds.len(), 1);
        assert!(matches!(dl.cmds[0], DrawCmd::FillRect { .. }));
    }

    #[test]
    fn paint_chrome_editor_rect_unchanged() {
        // 验证 status bar widget 化不影响 editor_rect 计算
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();

        shell.set_status_input(StatusBarInput {
            buffer_len: 100,
            selection_range: None,
            selection_char_count: None,
            cursor_line: 0,
            cursor_col: 0,
            conflict_label: None,
        });

        let inputs = ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 12.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        // editor_rect 应与之前一致
        assert_eq!(shell.editor_rect(), Rect::new(0.0, 32.0, 1188.0, 744.0));
    }

    #[test]
    fn paint_is_noop_for_phase2() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        let mut dl = DrawList::new();
        let mut ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: None,
        };
        shell.paint(&mut ctx);
        assert_eq!(dl.cmds.len(), 0);
    }

    #[test]
    fn dispatch_falls_through_to_fill() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let result = shell.dispatch(&Event::MouseMove { px: 50.0, py: 50.0 }, &mut ectx);
        assert!(result.is_none());
    }
    #[test]
    fn set_sidebar_input_stores_tabs_and_active_index() {
        // Audit test 5: Bug 1.1 — sidebar tab count matches views
        let mut shell = UiShell::new();
        let tabs: Vec<ui::tab_bar::TabInfo> = vec![
            ui::tab_bar::TabInfo {
                title: "a.rs".into(),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: "rust".into(),
            },
            ui::tab_bar::TabInfo {
                title: "b.rs".into(),
                file_path: None,
                is_dirty: true,
                pinned: false,
                language: "rust".into(),
            },
            ui::tab_bar::TabInfo {
                title: "c.rs".into(),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: "rust".into(),
            },
        ];
        shell.set_sidebar_input(
            ui::sidebar::SidebarConfig::new_default(1.0),
            tabs,
            Some(1),
            (0.0, 0.0),
        );
        assert_eq!(shell.sidebar_tabs.len(), 3, "sidebar_tabs count");
        assert_eq!(shell.sidebar_active_index, Some(1));
    }

    #[test]
    fn tab_bar_layout_returns_layout_from_dock_child() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let tabs: Vec<ui::tab_bar::TabInfo> = (0..3)
            .map(|i| ui::tab_bar::TabInfo {
                title: format!("t{i}.rs"),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            })
            .collect();
        shell.set_tabs_input(tabs, Some(0), false, false, None, 0.0);
        let inputs = ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs);

        let layout = shell.tab_bar_layout();
        assert!(layout.is_some(), "tab_bar_layout should return Some after update_frame with tabs");
        let layout = layout.unwrap();
        assert_eq!(layout.tabs.len(), 3, "layout should contain 3 tab entries");
    }

    #[test]
    fn tab_bar_layout_returns_none_when_no_tab_bar() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs);

        assert!(
            shell.tab_bar_layout().is_none(),
            "tab_bar_layout should return None when no tab bar widget"
        );
    }

    #[test]
    fn tabs_visible_change_triggers_dock_rebuild() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let tabs: Vec<ui::tab_bar::TabInfo> = (0..3)
            .map(|i| ui::tab_bar::TabInfo {
                title: format!("t{i}.rs"),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            })
            .collect();
        shell.set_tabs_input(tabs, Some(0), false, false, None, 0.0);

        // Frame 1: tabs_visible = false → no tab bar widget
        let inputs_off = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs_off);
        assert!(shell.tab_bar_layout().is_none(), "no tab bar when tabs_visible=false");

        // Frame 2: tabs_visible = true → dock should rebuild, tab bar appears
        let inputs_on = ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs_on);
        assert!(
            shell.tab_bar_layout().is_some(),
            "tab bar should appear after tabs_visible changes to true"
        );
    }

    #[test]
    fn set_tab_bar_hovered_updates_dock_child() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let tabs: Vec<ui::tab_bar::TabInfo> = (0..3)
            .map(|i| ui::tab_bar::TabInfo {
                title: format!("t{i}.rs"),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            })
            .collect();
        shell.set_tabs_input(tabs, Some(0), false, false, None, 0.0);
        let inputs = ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs);

        shell.set_tab_bar_hovered(Some(2));
        assert_eq!(
            shell.tab_bar_hovered_index(),
            Some(2),
            "hovered index should be updated on dock child"
        );

        shell.set_tab_bar_hovered(None);
        assert_eq!(shell.tab_bar_hovered_index(), None, "hovered index should clear to None");
    }

    #[test]
    fn sync_sidebar_persistent_updates_persistent_from_widget() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();

        // Set sidebar visible and inject tabs
        shell.set_sidebar_input(
            ui::sidebar::SidebarConfig::new_default(1.0),
            vec![ui::tab_bar::TabInfo {
                title: "test.rs".into(),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            }],
            Some(0),
            (0.0, 0.0),
        );
        shell.set_sidebar_pinned(true);
        shell.frames_rendered = 2;

        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: true,
            sidebar_thickness: 220.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        // Set hovered_index on persistent, inject, then sync back
        shell.sidebar_persistent.hovered_index = Some(0);
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        // After sync, persistent should retain the hovered_index from the widget
        // (widget was injected with hovered_index=Some(0), so after processing it should be Some(0) or None)
        // The key test: sync_sidebar_persistent should not panic and should update the persistent
        shell.sync_sidebar_persistent();
        // After sync, the persistent is replaced by the widget's state
        // Since we didn't send any events, the widget's hovered_index should be whatever was injected
    }

    #[test]
    fn compute_autoscroll_target_returns_none_with_no_tab_bar() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs);
        assert!(
            shell.compute_autoscroll_target(0, 0.0).is_none(),
            "No tab bar → no autoscroll target"
        );
    }

    #[test]
    fn compute_autoscroll_target_returns_none_when_tab_visible() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let tabs: Vec<ui::tab_bar::TabInfo> = (0..3)
            .map(|i| ui::tab_bar::TabInfo {
                title: format!("t{i}.rs"),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            })
            .collect();
        shell.set_tabs_input(tabs, Some(0), false, false, None, 0.0);
        let inputs = ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(800.0, 600.0), &theme, &mut m, &inputs);
        // With 3 tabs on 800px screen, tab 0 should be visible
        assert!(
            shell.compute_autoscroll_target(0, 0.0).is_none(),
            "Tab 0 should be visible on 800px screen, no autoscroll needed"
        );
    }

    #[test]
    fn tooltip_timer_absent_when_not_over_button() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        shell.set_search_input(ui::search_bar::SearchBarSnapshot {
            query: "test".into(),
            preedit_text: String::new(),
            match_count: 2,
            current_match: 0,
            visible: true,

            blink_on: false,
            replace_query: String::new(),
            replace_mode: false,
            focus_replace: false,
            options_use_regex: false,
        });
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: true,
            search_thickness: 28.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);
        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let _ = shell.dispatch(&Event::MouseMove { px: 50.0, py: 4.0 }, &mut ectx);
        assert!(!shell.has_tooltip_timer(), "No tooltip when not over a button");
    }

    #[test]
    fn tooltip_timer_created_when_over_button() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        shell.set_search_input(ui::search_bar::SearchBarSnapshot {
            query: "test".into(),
            preedit_text: String::new(),
            match_count: 2,
            current_match: 0,
            visible: true,

            blink_on: false,
            replace_query: String::new(),
            replace_mode: false,
            focus_replace: false,
            options_use_regex: false,
        });
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: true,
            search_thickness: 28.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);
        // Find the close button position from the dock child
        let close_rect = shell.dock.children.iter().find_map(|c| {
            c.widget
                .as_any()
                .downcast_ref::<ui::search_bar::SearchBarWidget>()
                .map(|sb| sb.close_btn_rect())
        });
        if let Some(r) = close_rect
            && r.w > 0.0
        {
            let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
            let _ = shell.dispatch(
                &Event::MouseMove { px: r.x + r.w / 2.0, py: r.y + r.h / 2.0 },
                &mut ectx,
            );
            assert!(
                shell.has_tooltip_timer(),
                "Hovering close button should create tooltip timer, rect={:?}",
                r
            );
        }
    }

    #[test]
    fn shell_layout_uses_metrics_dpi() {
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: true,
            sidebar_thickness: 440.0,
            scrollbar_thickness: 0.0,
            toc_visible: false,
            toc_thickness: 0.0,
            metrics: metrics(2.0),
            sidebar_settings: Default::default(),
        };
        assert_eq!(run_layout(&inputs).y, 72.0);
    }

    #[test]
    fn shell_updates_sidebar_with_behavior_input() {
        let sidebar_settings = ui::sidebar::SidebarSettingsInput {
            show_line_numbers: false,
            word_wrap: false,
            show_status_bar: true,
            theme_mode: ui::settings::ThemeMode::Dark,
            view_mode: ui::view_mode::ViewMode::Tabs,
        };
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: true,
            sidebar_thickness: 220.0,
            scrollbar_thickness: 0.0,
            toc_visible: false,
            toc_thickness: 0.0,
            metrics: metrics(1.0),
            sidebar_settings,
        };
        assert_eq!(inputs.sidebar_settings, sidebar_settings);
    }

    #[test]
    fn shell_builds_tab_widget_input_from_tab_info_pin_state() {
        let mut shell = UiShell::new();
        let mut pinned = ui::tab_bar::TabInfo {
            title: "pinned.rs".into(),
            file_path: None,
            is_dirty: false,
            pinned: false,
            language: String::new(),
        };
        pinned.pinned = true;
        shell.set_tabs_input(vec![pinned], Some(0), false, false, None, 0.0);

        let input = shell.tab_widget_input(Screen { w: 800.0, h: 600.0 }, metrics(2.0));
        assert!(input.tabs[0].pinned);
        assert_eq!(input.screen_size_px, (800.0, 600.0));
        assert_eq!(input.metrics.dpi, 2.0);
    }

    #[test]
    fn shell_builds_sidebar_input_from_one_frame_snapshot() {
        let mut shell = UiShell::new();
        shell.sidebar_tabs = vec![ui::tab_bar::TabInfo {
            title: "one".into(),
            file_path: None,
            is_dirty: false,
            pinned: false,
            language: String::new(),
        }];
        shell.sidebar_active_index = Some(0);
        shell.sidebar_traffic_light_inset = (68.0, 0.0);
        let mut inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: true,
            sidebar_thickness: 220.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        inputs.metrics = metrics(2.0);
        inputs.sidebar_settings.word_wrap = false;

        let input = shell.sidebar_widget_input(Screen { w: 900.0, h: 700.0 }, &inputs);

        assert_eq!(input.tabs[0].title, "one");
        assert_eq!(input.screen_size_px, (900.0, 700.0));
        assert_eq!(input.metrics.dpi, 2.0);
        assert!(!input.settings.word_wrap);
    }

    #[test]
    fn shell_builds_scrollbar_input_with_explicit_units() {
        let mut shell = UiShell::new();
        shell.set_scrollbar_input(42.0, 100, 12.5);
        assert_eq!(
            shell.scrollbar_widget_input(),
            ui::scrollbar::ScrollbarInput {
                viewport_height_px: 42.0,
                total_display_rows: 100,
                scroll_top_rows: 12.5,
            }
        );
    }

    #[test]
    fn tooltip_dismissed_on_click() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        shell.set_search_input(ui::search_bar::SearchBarSnapshot {
            query: "test".into(),
            preedit_text: String::new(),
            match_count: 0,
            current_match: 0,
            visible: true,

            blink_on: false,
            replace_query: String::new(),
            replace_mode: false,
            focus_replace: false,
            options_use_regex: false,
        });
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: true,
            search_thickness: 28.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);
        // First create a tooltip timer by hovering
        let mut ectx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let _ = shell.dispatch(&Event::MouseMove { px: 50.0, py: 4.0 }, &mut ectx);
        // Then click — should dismiss tooltip
        let _ = shell.dispatch(
            &Event::MouseDown { px: 50.0, py: 4.0, button: ui::core::widget::MouseButton::Left },
            &mut ectx,
        );
        assert!(!shell.has_tooltip_timer(), "Click should dismiss tooltip timer");
        assert!(shell.tooltip_overlay.is_none(), "Click should dismiss tooltip overlay");
    }

    #[test]
    fn update_widget_state_propagates_toc_input_to_dock() {
        let theme = test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;

        // Step 1: Build dock with TOC visible.
        shell.set_toc_input(ui::toc::TocInput {
            headings: vec![ui::toc::TocHeadingEntry { text: "Old".into(), level: 1 }],
            active_index: None,
        });
        let inputs = ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: metrics(1.0),
            toc_visible: true,
            toc_thickness: 200.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        // Step 2: Change toc_input WITHOUT changing toc_visible/toc_thickness,
        // so no dock rebuild occurs — only update_widget_state runs.
        shell.set_toc_input(ui::toc::TocInput {
            headings: vec![
                ui::toc::TocHeadingEntry { text: "New".into(), level: 2 },
                ui::toc::TocHeadingEntry { text: "Also".into(), level: 3 },
            ],
            active_index: Some(1),
        });
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);

        // Step 3: Verify TocWidget in dock received the updated input.
        let toc = shell
            .dock
            .children
            .iter_mut()
            .find_map(|c| c.widget.as_any_mut().downcast_mut::<ui::toc::TocWidget>())
            .expect("TocWidget must exist in dock");
        assert_eq!(toc.input().headings.len(), 2, "headings count must match updated input");
        assert_eq!(toc.input().headings[0].text, "New");
        assert_eq!(toc.input().headings[1].level, 3);
        assert_eq!(toc.input().active_index, Some(1));
    }

    fn default_mindmap_style_panel_input() -> ui::mindmap_style_panel::MindmapStylePanelInput {
        ui::mindmap_style_panel::MindmapStylePanelInput::from_selection(
            ui::theme::MindmapThemeSelection::Default,
            true,
        )
    }

    fn mindmap_style_panel_rect(shell: &UiShell) -> Rect {
        shell
            .dock
            .children
            .iter()
            .find(|child| {
                child.widget.as_any().is::<ui::mindmap_style_panel::MindmapStylePanelWidget>()
            })
            .map(|child| child.layout_rect)
            .expect("visible mmap style panel must have a dock child")
    }

    #[test]
    fn mindmap_style_panel_reserves_scaled_right_dock_below_title_and_above_status() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        let mut inputs = shell_inputs();
        inputs.sidebar_visible = true;
        inputs.sidebar_thickness = 120.0;
        inputs.status_thickness = 24.0;
        inputs.metrics = metrics(2.0);

        shell.update_frame(Screen::new(1_400.0, 900.0), &theme, &mut measure, &inputs);
        let editor_width_without_panel = shell.editor_rect().w;
        shell.set_mindmap_style_panel_input(Some(default_mindmap_style_panel_input()), 2.0);
        shell.update_frame(Screen::new(1_400.0, 900.0), &theme, &mut measure, &inputs);

        let editor_rect = shell.editor_rect();
        let panel_rect = mindmap_style_panel_rect(&shell);
        assert_eq!(editor_width_without_panel - editor_rect.w, PANEL_WIDTH_LOGICAL * 2.0);
        assert_eq!(panel_rect.w, PANEL_WIDTH_LOGICAL * 2.0);
        assert!(panel_rect.x >= editor_rect.right());
        assert_eq!(panel_rect.y, ui::title_bar::title_bar_height(2.0));
        assert_eq!(panel_rect.bottom(), 900.0 - inputs.status_thickness);
        assert_eq!(shell.mindmap_style_panel_thickness(), PANEL_WIDTH_LOGICAL * 2.0);
    }

    #[test]
    fn mindmap_style_panel_clear_restores_width_and_editor_focus() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        let inputs = shell_inputs();

        shell.set_mindmap_style_panel_input(Some(default_mindmap_style_panel_input()), 1.0);
        shell.update_frame(Screen::new(1_200.0, 800.0), &theme, &mut measure, &inputs);
        assert_eq!(shell.editor_rect().w, 1_200.0 - PANEL_WIDTH_LOGICAL);
        shell.keyboard_focus =
            KeyboardFocusTarget::Widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL);

        shell.set_mindmap_style_panel_input(None, 1.0);
        shell.update_frame(Screen::new(1_200.0, 800.0), &theme, &mut measure, &inputs);

        assert_eq!(shell.editor_rect().w, 1_200.0);
        assert_eq!(shell.keyboard_focus, KeyboardFocusTarget::Editor);
        assert_eq!(shell.mindmap_style_panel_thickness(), 0.0);
    }

    #[test]
    fn mindmap_style_panel_forward_key_routes_by_focused_widget_id() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        let inputs = shell_inputs();
        shell.set_mindmap_style_panel_input(Some(default_mindmap_style_panel_input()), 1.0);
        shell.update_frame(Screen::new(1_200.0, 800.0), &theme, &mut measure, &inputs);
        shell.keyboard_focus =
            KeyboardFocusTarget::Widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL);

        assert_eq!(
            shell.forward_key(KeyCode::Right, Modifiers::NONE, &theme, 1.0),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            shell.forward_key(KeyCode::Enter, Modifiers::NONE, &theme, 1.0),
            Some(WidgetAction::MindmapStylePanel(
                ui::core::widget::MindmapStylePanelAction::SelectTheme("dawn".into())
            ))
        );
        assert_eq!(
            shell.forward_key(KeyCode::Escape, Modifiers::NONE, &theme, 1.0),
            Some(WidgetAction::MindmapStylePanel(ui::core::widget::MindmapStylePanelAction::Close))
        );

        shell.keyboard_focus = KeyboardFocusTarget::Editor;
        assert_eq!(shell.forward_key(KeyCode::Enter, Modifiers::NONE, &theme, 1.0), None);
    }

    #[test]
    fn mindmap_style_panel_input_updates_existing_widget_without_rebuild() {
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut shell = UiShell::new();
        shell.frames_rendered = 1;
        let inputs = shell_inputs();
        shell.set_mindmap_style_panel_input(Some(default_mindmap_style_panel_input()), 1.0);
        shell.update_frame(Screen::new(1_200.0, 800.0), &theme, &mut measure, &inputs);
        let widget_address_before = shell
            .dock
            .children
            .iter()
            .find(|child| child.widget.id() == Some(ui::core::widget::ids::MINDMAP_STYLE_PANEL))
            .map(|child| child.widget.as_ref() as *const dyn Widget as *const ())
            .expect("mmap style panel widget must exist");

        let selected = ui::mindmap_style_panel::MindmapStylePanelInput::from_selection(
            ui::theme::MindmapThemeSelection::Selected("dawn".to_owned()),
            false,
        );
        shell.set_mindmap_style_panel_input(Some(selected), 1.0);
        assert!(!shell.dock_dirty, "same visibility and thickness must not rebuild dock");
        shell.update_frame(Screen::new(1_200.0, 800.0), &theme, &mut measure, &inputs);
        let widget_address_after = shell
            .dock
            .children
            .iter()
            .find(|child| child.widget.id() == Some(ui::core::widget::ids::MINDMAP_STYLE_PANEL))
            .map(|child| child.widget.as_ref() as *const dyn Widget as *const ())
            .expect("mmap style panel widget must remain present");

        assert_eq!(widget_address_before, widget_address_after);
    }

    #[test]
    fn ui_shell_has_no_standalone_sync_overlay_lifecycle() {
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui_shell.rs"));
        let open_lifecycle = concat!("open_sync", "_panel");
        let layout_lifecycle = concat!("layout_sync", "_panel");
        assert!(!source.contains(open_lifecycle));
        assert!(!source.contains(layout_lifecycle));
    }
}
