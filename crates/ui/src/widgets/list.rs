//! ListWidget — 通用列表 primitive（支持 Vertical/Horizontal 方向）。
//!
//! 不知道"tab/menu"概念。调用方喂入 ListItem，命中后返回 ListAction::Selected(index)。
//! 调用方负责把 index 翻译成自己的强类型 action。

use crate::core::measure::TextMeasure;
use crate::core::text_util::truncate_title_precise;
use crate::core::widget::ControlAction;
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, DrawCmd, Event, EventCtx, KeyCode, LayoutCtx, Modifiers,
    MouseButton, PaintCtx, Rect, Widget, WidgetAction, WidgetId,
};

/// Layout direction for the list.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// 固定标签左侧竖线宽度（逻辑像素）
const PIN_BAR_WIDTH_LOGICAL: f32 = 2.0;
/// 固定标签左侧竖线右边距（逻辑像素）
const PIN_BAR_MARGIN_LOGICAL: f32 = 6.0;
/// 关闭按钮尺寸（逻辑像素）
const CLOSE_BTN_SIZE_LOGICAL: f32 = 12.0;

/// 关闭按钮与 label 文字之间的间距（逻辑像素）
const CLOSE_BTN_LABEL_GAP_LOGICAL: f32 = 2.0;
/// extra_label 预留宽度（逻辑像素）
const EXTRA_LABEL_WIDTH_LOGICAL: f32 = 40.0;

/// 行类型修饰：影响视觉与命中。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ListItemKind {
    #[default]
    Normal,
    /// 不可点击的分割行（仅画分隔线）。
    Separator,
    /// 不可点击的标题/分组（与 normal 同样画文字，不响应 click）。
    Header,
}

/// 行尾右侧小指示符：dirty 圆点 / conflict 等。语义化、不知道领域。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ListItemIndicator {
    #[default]
    None,
    /// 行尾画一个"未保存"圆点。
    Dot,
}

#[derive(Clone, Debug, Default)]
pub struct ListItem {
    pub label: String,
    pub kind: ListItemKind,
    pub icon: Option<String>,
    pub indicator: ListItemIndicator,
    pub pinned: bool,
    pub extra_label: Option<String>,
    pub is_active: bool,
    pub closeable: bool,
}

#[derive(Copy, Clone, Debug)]
pub struct ListStyle {
    pub row_h_logical: f32,     // 例如 24
    pub item_w_logical: f32,    // Horizontal 模式列宽（Vertical 忽略）
    pub pad_x_logical: f32,     // 行内左右内边距
    pub pad_y_logical: f32,     // 列表上下内边距
    pub font_size_logical: f32, // 字号（未乘 dpi）
    pub bg: [f32; 4],           // 列表背景；调用方决定（0 alpha 即透明）
    pub item_active_bg: [f32; 4],
    pub item_hover_bg: [f32; 4],
    pub item_fg: [f32; 4],
    pub item_active_fg: [f32; 4],
    pub item_hover_fg: [f32; 4],
    pub item_accent: [f32; 4],
    pub separator: [f32; 4],
    pub indicator_color: [f32; 4],
}

impl ListStyle {
    pub fn from_theme(theme: &crate::theme::Theme) -> Self {
        let metrics = theme.control_metrics();
        let application = theme.application_theme();
        Self {
            row_h_logical: metrics.control_height_logical,
            item_w_logical: crate::constants::SIDEBAR_MIN_WIDTH,
            pad_x_logical: metrics.horizontal_padding_logical,
            pad_y_logical: 0.0,
            font_size_logical: metrics.font_size_logical,
            bg: application.navigation_surface,
            item_active_bg: application.navigation_selected_surface,
            item_hover_bg: application.navigation_hover_surface,
            item_fg: application.text_primary,
            item_active_fg: application.navigation_selected_text,
            item_hover_fg: application.text_primary,
            item_accent: application.accent,
            separator: application.divider,
            indicator_color: application.warning,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListAction {
    /// 用户单击了第 index 行（已过滤 Separator/Header）
    Selected(usize),
    /// 鼠标移入/移出某行（用于上层做悬停 tooltip 等；不强制处理）
    HoverChanged(Option<usize>),
    /// 用户点击了关闭按钮
    CloseRequested(usize),
    /// 用户请求了某行的上下文菜单。
    ContextRequested { index: usize, anchor_px: (f32, f32) },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ListPressTarget {
    Row(usize),
    CloseButton(usize),
}

pub struct ListWidget {
    id: Option<WidgetId>,
    rect: Rect,
    items: Vec<ListItem>,
    active_index: Option<usize>,
    hovered_index: Option<usize>,
    close_hovered: bool,
    press_target: Option<ListPressTarget>,
    focused: bool,
    focused_index: Option<usize>,
    style: ListStyle,
    scroll_offset: f32,
    orientation: Orientation,
    truncated_labels: Vec<String>,
    truncated_label_widths: Vec<f32>,
    dpi: f32,
    accessibility_label: Option<String>,
}

impl ListWidget {
    pub fn new(style: ListStyle, orientation: Orientation) -> Self {
        Self {
            id: None,
            rect: Rect::ZERO,
            items: Vec::new(),
            active_index: None,
            hovered_index: None,
            close_hovered: false,
            press_target: None,
            focused: false,
            focused_index: None,
            style,
            scroll_offset: 0.0,
            orientation,
            truncated_labels: Vec::new(),
            truncated_label_widths: Vec::new(),
            dpi: 1.0,
            accessibility_label: None,
        }
    }

    pub fn with_id(mut self, id: WidgetId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn set_scroll_offset(&mut self, off: f32) {
        self.scroll_offset = off;
    }

    pub fn set_items(&mut self, items: Vec<ListItem>) {
        self.items = items;
        if self.press_target.is_some_and(|target| match target {
            ListPressTarget::Row(index) | ListPressTarget::CloseButton(index) => {
                index >= self.items.len()
            }
        }) {
            self.press_target = None;
        }
        self.normalize_focused_index(false);
    }
    pub fn set_active(&mut self, idx: Option<usize>) {
        self.active_index = idx;
    }
    pub fn set_style(&mut self, s: ListStyle) {
        self.style = s;
    }
    pub fn set_accessibility_label(&mut self, label: Option<String>) {
        self.accessibility_label = label;
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }
    pub fn items(&self) -> &[ListItem] {
        &self.items
    }
    pub fn hovered_index(&self) -> Option<usize> {
        self.hovered_index
    }
    pub fn set_hovered_index(&mut self, idx: Option<usize>) {
        self.hovered_index = idx;
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.focused_index
    }

    fn is_selectable(&self, index: usize) -> bool {
        self.items.get(index).is_some_and(|item| item.kind == ListItemKind::Normal)
    }

    fn first_selectable_index(&self) -> Option<usize> {
        self.items.iter().position(|item| item.kind == ListItemKind::Normal)
    }

    fn last_selectable_index(&self) -> Option<usize> {
        self.items.iter().rposition(|item| item.kind == ListItemKind::Normal)
    }

    fn normalize_focused_index(&mut self, prefer_active: bool) {
        if prefer_active
            && let Some(active_index) = self.active_index.filter(|index| self.is_selectable(*index))
        {
            self.focused_index = Some(active_index);
            return;
        }
        if self.focused_index.is_some_and(|index| self.is_selectable(index)) {
            return;
        }
        self.focused_index = self.first_selectable_index();
    }

    fn move_focused_index(&mut self, forward: bool) {
        self.normalize_focused_index(false);
        let Some(current_index) = self.focused_index else {
            return;
        };
        self.focused_index = if forward {
            ((current_index + 1)..self.items.len())
                .find(|index| self.is_selectable(*index))
                .or(Some(current_index))
        } else {
            (0..current_index)
                .rev()
                .find(|index| self.is_selectable(*index))
                .or(Some(current_index))
        };
    }

    fn handle_keyboard_event(
        &mut self,
        key: KeyCode,
        modifiers: Modifiers,
    ) -> Option<WidgetAction> {
        if !self.focused || modifiers != Modifiers::NONE {
            return None;
        }
        let move_forward = match (self.orientation, key) {
            (Orientation::Vertical, KeyCode::Down) | (Orientation::Horizontal, KeyCode::Right) => {
                Some(true)
            }
            (Orientation::Vertical, KeyCode::Up) | (Orientation::Horizontal, KeyCode::Left) => {
                Some(false)
            }
            _ => None,
        };
        if let Some(forward) = move_forward {
            self.move_focused_index(forward);
            return Some(WidgetAction::Consumed);
        }
        match key {
            KeyCode::Home => {
                self.focused_index = self.first_selectable_index();
                Some(WidgetAction::Consumed)
            }
            KeyCode::End => {
                self.focused_index = self.last_selectable_index();
                Some(WidgetAction::Consumed)
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.focused_index.map(|index| WidgetAction::List(ListAction::Selected(index)))
            }
            _ => None,
        }
    }

    /// pinned item 左侧竖线占用的总宽度（0 表示非 pinned 或 Horizontal 模式）
    fn pinned_left_offset(&self, item: &ListItem, dpi: f32) -> f32 {
        if item.pinned && self.orientation == Orientation::Vertical {
            PIN_BAR_WIDTH_LOGICAL * dpi + PIN_BAR_MARGIN_LOGICAL * dpi
        } else {
            0.0
        }
    }

    /// 计算第 i 行的矩形（px，减去 scroll_offset 实现滚动。允许超出 bounds，交由可见行过滤处理）。
    pub(crate) fn item_rect(&self, i: usize, dpi: f32) -> Rect {
        match self.orientation {
            Orientation::Vertical => {
                let row_h = self.style.row_h_logical * dpi;
                let pad_y = self.style.pad_y_logical * dpi;
                let top = self.rect.y + pad_y + i as f32 * row_h - self.scroll_offset;
                Rect::new(self.rect.x, top, self.rect.w, row_h)
            }
            Orientation::Horizontal => {
                let col_w = self.style.item_w_logical * dpi;
                let pad_x = self.style.pad_x_logical * dpi;
                let left = self.rect.x + pad_x + i as f32 * col_w - self.scroll_offset;
                Rect::new(left, self.rect.y, col_w, self.rect.h)
            }
        }
    }

    /// 把屏幕 (px, py) 翻译为命中的行 index（仅 Normal）；不可点行返回 None。
    /// item_rect 已内含 scroll_offset，无需额外传入。
    pub(crate) fn hit_row(&self, px: f32, py: f32, dpi: f32) -> Option<usize> {
        if !self.rect.contains(px, py) {
            return None;
        }
        match self.orientation {
            Orientation::Vertical => {
                for (i, item) in self.items.iter().enumerate() {
                    let r = self.item_rect(i, dpi);
                    if r.contains(px, py) {
                        return matches!(item.kind, ListItemKind::Normal).then_some(i);
                    }
                }
            }
            Orientation::Horizontal => {
                for (i, item) in self.items.iter().enumerate() {
                    let r = self.item_rect(i, dpi);
                    if Rect::new(r.x, self.rect.y, r.w, self.rect.h).contains(px, py) {
                        return matches!(item.kind, ListItemKind::Normal).then_some(i);
                    }
                }
            }
        }
        None
    }

    /// 检查点击是否在 close button 区域内（仅 hovered 的 closeable item）。
    /// 返回 Some(sorted_index) 表示命中。item_rect 已内含 scroll_offset。
    pub(crate) fn hit_close_btn(&self, px: f32, py: f32, dpi: f32) -> Option<usize> {
        if !self.rect.contains(px, py) {
            return None;
        }
        let i = self.hovered_index?;
        let item = self.items.get(i)?;
        if !item.closeable || item.kind != ListItemKind::Normal {
            return None;
        }
        let pad_x = self.style.pad_x_logical * dpi;
        let btn_size = CLOSE_BTN_SIZE_LOGICAL * dpi;
        let row_rect = self.item_rect(i, dpi);
        // Close button 位于行右侧
        let btn_x = row_rect.x + row_rect.w - pad_x - btn_size;
        let btn_y = row_rect.y + (row_rect.h - btn_size) * 0.5;
        let minimum_hit_target = crate::theme::ControlMetrics::default().minimum_hit_target_logical;
        let hit_pad = ((minimum_hit_target - CLOSE_BTN_SIZE_LOGICAL) * 0.5).max(0.0) * dpi;
        let btn_rect = Rect::new(
            btn_x - hit_pad,
            btn_y - hit_pad,
            btn_size + hit_pad * 2.0,
            btn_size + hit_pad * 2.0,
        );
        if !row_rect.contains(px, py) {
            return None;
        }
        if btn_rect.contains(px, py) { Some(i) } else { None }
    }
}

impl Widget for ListWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        let dpi = ctx.dpi;
        self.dpi = dpi;
        let font_size = self.style.font_size_logical * dpi;
        let measure: &mut dyn TextMeasure = ctx.measure;
        let pad_x = self.style.pad_x_logical * dpi;
        let dot_r = (font_size * 0.18).max(2.0);
        self.truncated_labels = self
            .items
            .iter()
            .map(|item| {
                if !matches!(item.kind, ListItemKind::Normal | ListItemKind::Header) {
                    return String::new();
                }
                let left_offset = self.pinned_left_offset(item, dpi);
                let dot_extra = if matches!(item.indicator, ListItemIndicator::Dot) {
                    dot_r * 2.0 + 4.0 * dpi
                } else {
                    0.0
                };
                let icon_extra = if item.icon.is_some() {
                    (self.style.font_size_logical + 4.0) * dpi
                } else {
                    0.0
                };
                let close_extra = if item.closeable {
                    (CLOSE_BTN_SIZE_LOGICAL + CLOSE_BTN_LABEL_GAP_LOGICAL) * dpi
                } else {
                    0.0
                };
                let row_w = match self.orientation {
                    Orientation::Vertical => rect.w,
                    Orientation::Horizontal => self.style.item_w_logical * dpi,
                };
                let label_max_w =
                    (row_w - pad_x * 2.0 - left_offset - dot_extra - icon_extra - close_extra)
                        .max(0.0);
                truncate_title_precise(&item.label, label_max_w, font_size, measure)
            })
            .collect();
        self.truncated_label_widths = self
            .truncated_labels
            .iter()
            .map(|label| if label.is_empty() { 0.0 } else { measure.measure(label, font_size) })
            .collect();
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let alpha = ctx.global_alpha;

        if self.style.bg[3] > 0.0 {
            let mut bg = self.style.bg;
            bg[3] *= alpha;
            ctx.list.fill(Rect::new(self.rect.x, self.rect.y, self.rect.w, self.rect.h), bg);
        }
        // Clip to list rect so items don't overflow header/settings area
        let clip_rect = Rect::new(
            self.rect.x + ctx.list.offset.0,
            self.rect.y + ctx.list.offset.1,
            self.rect.w,
            self.rect.h,
        );
        ctx.list.cmds.push(DrawCmd::PushClip(clip_rect));

        let dpi = ctx.dpi;
        let metrics = ctx.theme.control_metrics();
        let pad_x = self.style.pad_x_logical * dpi;
        let font_size = self.style.font_size_logical * dpi;

        // 可见行过滤：只绘制在 self.rect 范围内的行（考虑 pad_y 偏移）
        let row_h = self.style.row_h_logical * dpi;
        let pad_y = self.style.pad_y_logical * dpi;
        // item i 严格可见：底部 > scroll_offset 且 顶部 < scroll_offset + rect.h
        // first_visible = ceil((scroll_offset - pad_y) / row_h)
        // last_visible (exclusive) = floor((scroll_offset + rect.h - pad_y) / row_h) + 1
        //   若 bottom 恰好对齐（item 刚刚完全滚出），first_visible 会跳过它
        //   若 top 恰好对齐（item 底边贴底），+1 不含它
        let first_visible = if row_h > 0.0 {
            (((self.scroll_offset - pad_y) / row_h).ceil().max(0.0)) as usize
        } else {
            0
        };
        let last_visible = if row_h > 0.0 {
            // item i 的顶部 < visible_bottom → i < (scroll_offset + rect.h - pad_y) / row_h
            // last_visible (exclusive) = floor(...)，不 +1，保证 item 不超出 rect 底部
            (((self.scroll_offset + self.rect.h - pad_y) / row_h).floor() as usize)
                .min(self.items.len())
        } else {
            self.items.len()
        };

        for i in first_visible..last_visible {
            let item = &self.items[i];
            let row_rect = self.item_rect(i, dpi);

            match item.kind {
                ListItemKind::Separator => {
                    let mut sep = self.style.separator;
                    sep[3] *= alpha;
                    match self.orientation {
                        Orientation::Vertical => {
                            let sep_h = (1.0 * dpi).max(1.0);
                            let y = row_rect.y + (row_rect.h - sep_h) * 0.5;
                            ctx.list.fill(
                                Rect::new(row_rect.x + pad_x, y, row_rect.w - pad_x * 2.0, sep_h),
                                sep,
                            );
                        }
                        Orientation::Horizontal => {
                            let sep_w = (1.0 * dpi).max(1.0);
                            let x = row_rect.x + (row_rect.w - sep_w) * 0.5;
                            ctx.list.fill(
                                Rect::new(x, row_rect.y + pad_x, sep_w, row_rect.h - pad_x * 2.0),
                                sep,
                            );
                        }
                    }
                    continue;
                }
                ListItemKind::Header | ListItemKind::Normal => {
                    let is_active = item.is_active || Some(i) == self.active_index;
                    let is_keyboard_focused = self.focused && Some(i) == self.focused_index;
                    let is_hovered = Some(i) == self.hovered_index || is_keyboard_focused;

                    if matches!(item.kind, ListItemKind::Normal) {
                        if is_active {
                            let mut color = self.style.item_active_bg;
                            color[3] *= alpha;
                            ctx.list.fill_menu_hover(row_rect, color, dpi);
                        } else if is_hovered {
                            let mut color = self.style.item_hover_bg;
                            color[3] *= alpha;
                            ctx.list.fill_menu_hover(row_rect, color, dpi);
                        }
                        if is_keyboard_focused {
                            ctx.list.stroke_rounded(
                                row_rect,
                                ctx.theme.settings_theme().focus_ring,
                                metrics.compact_corner_radius_logical * dpi,
                                metrics.focus_ring_width_logical * dpi,
                            );
                        }
                    }

                    // Pin bar
                    if item.pinned {
                        let bar_len = PIN_BAR_WIDTH_LOGICAL * dpi;
                        let bar_pad = metrics.content_spacing_logical * dpi;
                        let mut bar_color = self.style.item_accent;
                        bar_color[3] *= alpha;
                        match self.orientation {
                            Orientation::Vertical => {
                                let bar_x = row_rect.x + pad_x;
                                ctx.list.fill_rounded(
                                    Rect::new(
                                        bar_x,
                                        row_rect.y + bar_pad,
                                        bar_len,
                                        row_rect.h - bar_pad * 2.0,
                                    ),
                                    bar_color,
                                    bar_len * 0.5,
                                );
                            }
                            Orientation::Horizontal => {
                                let bar_y = row_rect.y + pad_x;
                                ctx.list.fill_rounded(
                                    Rect::new(
                                        row_rect.x + bar_pad,
                                        bar_y,
                                        row_rect.w - bar_pad * 2.0,
                                        bar_len,
                                    ),
                                    bar_color,
                                    bar_len * 0.5,
                                );
                            }
                        }
                    }

                    let baseline = row_rect.y + row_rect.h * 0.5 + font_size * 0.35;
                    let mut fg = if is_active {
                        self.style.item_active_fg
                    } else if is_hovered {
                        self.style.item_hover_fg
                    } else {
                        self.style.item_fg
                    };
                    fg[3] *= alpha;
                    let left_offset = self.pinned_left_offset(item, dpi);
                    let icon_extra = if item.icon.is_some() {
                        (self.style.font_size_logical + metrics.compact_spacing_logical) * dpi
                    } else {
                        0.0
                    };
                    let mut text_x = row_rect.x + pad_x + left_offset;

                    // Icon
                    if let Some(ref icon_name) = item.icon {
                        let icon_sz = font_size;
                        let icon_y = row_rect.y + (row_rect.h - icon_sz) * 0.5;
                        crate::widgets::icon::draw_icon(
                            ctx.list, icon_name, text_x, icon_y, icon_sz, fg,
                        );
                        text_x += icon_extra;
                    }

                    let label = self
                        .truncated_labels
                        .get(i)
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .unwrap_or_else(|| item.label.clone());

                    let mut actual_w = self.truncated_label_widths.get(i).copied().unwrap_or(0.0);

                    if let Some(ref mut shaper) = ctx.shaper {
                        if is_active {
                            shaper.set_font_weight(shaping::Weight::MEDIUM);
                        }
                        actual_w =
                            ctx.list.text_shaped(text_x, baseline, font_size, fg, &label, shaper);
                        if is_active {
                            shaper.set_font_weight(shaping::Weight::NORMAL);
                        }
                    }

                    // Indicator dot
                    if matches!(item.indicator, ListItemIndicator::Dot) {
                        let mut ind = self.style.indicator_color;
                        ind[3] *= alpha;
                        if let Some(ref mut shaper) = ctx.shaper {
                            // '*' is naturally drawn high in most fonts, so shift baseline down slightly to visually center it
                            let star_y = baseline + font_size * 0.22;
                            ctx.list.text_shaped(
                                text_x + actual_w + 2.0 * dpi,
                                star_y,
                                font_size,
                                ind,
                                "*",
                                shaper,
                            );
                        }
                    }

                    // Extra label (right side)
                    if let Some(ref extra) = item.extra_label {
                        let extra_x =
                            row_rect.x + row_rect.w - pad_x - EXTRA_LABEL_WIDTH_LOGICAL * dpi;
                        if let Some(ref mut shaper) = ctx.shaper {
                            ctx.list.text_shaped(extra_x, baseline, font_size, fg, extra, shaper);
                        }
                    }

                    // Close button on hovered closeable items
                    if item.closeable && is_hovered {
                        let btn_size = CLOSE_BTN_SIZE_LOGICAL * dpi;
                        let application = ctx.theme.application_theme();
                        let mut close_fg = if self.close_hovered {
                            application.text_primary
                        } else {
                            application.text_secondary
                        };
                        close_fg[3] *= alpha;
                        if let Some(ref mut shaper) = ctx.shaper {
                            ctx.list.text_shaped(
                                row_rect.x + row_rect.w - pad_x - btn_size * 0.5,
                                baseline,
                                font_size,
                                close_fg,
                                "x",
                                shaper,
                            );
                        }
                    }
                }
            }
        }
        ctx.list.cmds.push(DrawCmd::PopClip);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        self.id
    }

    fn is_focusable(&self) -> bool {
        self.id.is_some()
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused = self.id.is_some_and(|id| focused_id == Some(id));
        if self.focused {
            self.normalize_focused_index(true);
        }
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let id = self.id?;
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let root_id = AccessibilityId::from(id);
        let mut root =
            AccessibilityNode::new(root_id, AccessibilityRole::List, ctx.screen_bounds(self.rect))
                .with_name(self.accessibility_label.as_deref().unwrap_or("列表"))
                .with_focused(self.focused && self.focused_index.is_none())
                .with_action(AccessibilityAction::Focus);
        for (index, item) in self.items.iter().enumerate() {
            if item.kind != ListItemKind::Normal {
                continue;
            }
            let item_rect = self.item_rect(index, self.dpi);
            if item_rect.bottom() <= self.rect.top() || item_rect.top() >= self.rect.bottom() {
                continue;
            }
            let selected = item.is_active || self.active_index == Some(index);
            let child = AccessibilityNode::new(
                root_id.child(index as u64 + 1),
                AccessibilityRole::ListItem,
                ctx.screen_bounds(item_rect),
            )
            .with_name(item.label.clone())
            .with_selected(selected)
            .with_focused(self.focused && self.focused_index == Some(index))
            .with_action(AccessibilityAction::Focus)
            .with_action(AccessibilityAction::Activate);
            root.children.push(child);
        }
        Some(root)
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        let id = self.id?;
        let root_id = AccessibilityId::from(id);
        if request.target == root_id && request.action == AccessibilityAction::Focus {
            return Some(WidgetAction::Control(ControlAction::FocusRequested { id }));
        }
        let index = (0..self.items.len()).find(|index| {
            root_id.child(*index as u64 + 1) == request.target && self.is_selectable(*index)
        })?;
        match request.action {
            AccessibilityAction::Focus => {
                self.focused_index = Some(index);
                Some(WidgetAction::Control(ControlAction::FocusRequested { id }))
            }
            AccessibilityAction::Activate => Some(WidgetAction::List(ListAction::Selected(index))),
            _ => None,
        }
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.press_target = self
                    .hit_close_btn(*px, *py, ctx.dpi)
                    .map(ListPressTarget::CloseButton)
                    .or_else(|| self.hit_row(*px, *py, ctx.dpi).map(ListPressTarget::Row));
                if self.press_target.is_none() {
                    return None;
                }
                if let Some(id) = self.id
                    && !self.focused
                {
                    return Some(WidgetAction::Control(ControlAction::FocusRequested { id }));
                }
                Some(WidgetAction::Consumed)
            }
            Event::MouseDown { px, py, button: MouseButton::Right } => {
                self.hit_row(*px, *py, ctx.dpi).map(|index| {
                    WidgetAction::List(ListAction::ContextRequested {
                        index,
                        anchor_px: (*px, *py),
                    })
                })
            }
            Event::MouseUp { px, py, button: MouseButton::Left } => {
                let press_target = self.press_target.take()?;
                let action = match press_target {
                    ListPressTarget::Row(index)
                        if self.hit_row(*px, *py, ctx.dpi) == Some(index)
                            && self.hit_close_btn(*px, *py, ctx.dpi).is_none() =>
                    {
                        WidgetAction::List(ListAction::Selected(index))
                    }
                    ListPressTarget::CloseButton(index)
                        if self.hit_close_btn(*px, *py, ctx.dpi) == Some(index) =>
                    {
                        WidgetAction::List(ListAction::CloseRequested(index))
                    }
                    _ => WidgetAction::Consumed,
                };
                Some(action)
            }
            Event::MouseMove { px, py } => {
                let new = self.hit_row(*px, *py, ctx.dpi);
                if new.is_some() {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                let close_now = self.hit_close_btn(*px, *py, ctx.dpi).is_some();
                if new != self.hovered_index || close_now != self.close_hovered {
                    self.hovered_index = new;
                    self.close_hovered = close_now;
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            Event::PointerLeave => {
                let hover_changed =
                    self.hovered_index.take().is_some() | std::mem::take(&mut self.close_hovered);
                hover_changed.then_some(WidgetAction::Consumed)
            }
            Event::InteractionCancel => {
                let interaction_changed = self.hovered_index.take().is_some()
                    | std::mem::take(&mut self.close_hovered)
                    | self.press_target.take().is_some();
                interaction_changed.then_some(WidgetAction::Consumed)
            }
            Event::KeyDown(key, modifiers) => self.handle_keyboard_event(*key, *modifiers),
            _ => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.press_target.is_some()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use crate::core::{DrawCmd, DrawList, NoopMeasure};

    fn style() -> ListStyle {
        ListStyle {
            row_h_logical: 24.0,
            item_w_logical: 120.0,
            pad_x_logical: 8.0,
            pad_y_logical: 4.0,
            font_size_logical: 13.0,
            bg: [0.1, 0.1, 0.1, 1.0],
            item_active_bg: [0.2; 4],
            item_hover_bg: [0.15; 4],
            item_fg: [0.9; 4],
            item_active_fg: [1.0; 4],
            item_hover_fg: [0.95; 4],
            item_accent: [0.5, 0.5, 0.8, 1.0],
            separator: [0.3; 4],
            indicator_color: [1.0, 0.5, 0.0, 1.0],
        }
    }

    /// Mock measure that uses char_width for realistic text width estimation.
    /// Unlike NoopMeasure (returns 0.0), this actually measures text so truncation works.
    struct CharWidthMeasure;
    impl crate::core::TextMeasure for CharWidthMeasure {
        fn measure(&mut self, s: &str, font_size: f32) -> f32 {
            s.chars().map(|ch| crate::core::text_util::char_width(ch, font_size)).sum()
        }
    }

    fn layout_ctx<'a>(theme: &'a Theme, m: &'a mut dyn crate::core::TextMeasure) -> LayoutCtx<'a> {
        LayoutCtx { ui_measure: None, measure: m, theme, dpi: 1.0 }
    }

    fn make_list(items: Vec<ListItem>, rect: Rect) -> ListWidget {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(items);
        w.set_rect(rect, &mut layout);
        w
    }

    #[test]
    fn standard_list_style_uses_shared_metrics_and_semantic_colors() {
        for theme in [
            crate::theme::Theme::resolve_builtin(
                crate::settings::ThemeMode::Light,
                winit::window::Theme::Light,
            ),
            crate::theme::Theme::resolve_builtin(
                crate::settings::ThemeMode::Dark,
                winit::window::Theme::Dark,
            ),
        ] {
            let metrics = theme.control_metrics();
            let application = theme.application_theme();
            let style = ListStyle::from_theme(&theme);

            assert_eq!(style.row_h_logical, metrics.control_height_logical);
            assert_eq!(style.pad_x_logical, metrics.horizontal_padding_logical);
            assert_eq!(style.font_size_logical, metrics.font_size_logical);
            assert_eq!(style.item_fg, application.text_primary);
            assert_eq!(style.item_hover_bg, application.navigation_hover_surface);
        }
    }

    #[test]
    fn close_icon_uses_semantic_colors_at_two_times_dpi() {
        for theme in [
            crate::theme::Theme::resolve_builtin(
                crate::settings::ThemeMode::Light,
                winit::window::Theme::Light,
            ),
            crate::theme::Theme::resolve_builtin(
                crate::settings::ThemeMode::Dark,
                winit::window::Theme::Dark,
            ),
        ] {
            let application = theme.application_theme();
            assert_eq!(painted_close_icon_color(&theme, false), application.text_secondary);
            assert_eq!(painted_close_icon_color(&theme, true), application.text_primary);
        }
    }

    fn painted_close_icon_color(theme: &Theme, close_hovered: bool) -> [f32; 4] {
        let mut widget = ListWidget::new(ListStyle::from_theme(theme), Orientation::Vertical);
        widget.set_items(vec![item("Document")]);
        widget.hovered_index = Some(0);
        widget.close_hovered = close_hovered;

        let mut measure = NoopMeasure;
        let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi: 2.0 };
        widget.set_rect(Rect::new(0.0, 0.0, 440.0, 80.0), &mut layout);

        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        widget.paint(&mut PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme,
            dpi: 2.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        });

        draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TextLayout { layout, color, .. } if layout.text == "x" => Some(*color),
                _ => None,
            })
            .expect("hovered closeable row should paint a close icon")
    }

    fn item(label: &str) -> ListItem {
        ListItem {
            label: label.into(),
            kind: ListItemKind::Normal,
            icon: None,
            indicator: ListItemIndicator::None,
            pinned: false,
            extra_label: None,
            is_active: false,
            closeable: true,
        }
    }

    #[test]
    fn accessibility_exposes_list_items_and_selection_action() {
        let id = WidgetId(70);
        let mut widget = ListWidget::new(style(), Orientation::Vertical).with_id(id);
        widget.set_accessibility_label(Some("打开的文档".into()));
        widget.set_items(vec![item("a.rs"), item("b.rs")]);
        widget.set_active(Some(1));
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        widget.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout_ctx(&theme, &mut measure));
        widget.set_keyboard_focus(Some(id));
        let node = widget
            .accessibility_node(&crate::core::AccessibilityContext::new(10.0, 20.0))
            .expect("identified list should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::List);
        assert_eq!(node.name.as_deref(), Some("打开的文档"));
        assert_eq!(node.children.len(), 2);
        assert_eq!(node.children[0].name.as_deref(), Some("a.rs"));
        assert_eq!(node.children[1].state.selected, Some(true));
        assert!(node.children[1].state.focused);
        assert_eq!(node.children[0].bounds, Rect::new(10.0, 24.0, 220.0, 24.0));
        assert_eq!(
            widget.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.children[0].id,
                crate::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::List(ListAction::Selected(0)))
        );
    }

    fn pinned_item(label: &str) -> ListItem {
        ListItem {
            label: label.into(),
            kind: ListItemKind::Normal,
            icon: None,
            indicator: ListItemIndicator::None,
            pinned: true,
            extra_label: None,
            is_active: false,
            closeable: false,
        }
    }

    #[test]
    fn paint_emits_bg_plus_text_per_visible_row() {
        let theme = crate::theme::test_theme();
        let w = make_list(
            vec![item("a.rs"), item("b.rs"), item("c.rs")],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);
        // bg + PushClip + 3 text + PopClip = 6
        assert_eq!(list.cmds.len(), 6);
    }

    #[test]
    fn rows_overflowing_rect_are_truncated() {
        // rect 高 60px, pad_y=4, row=24 → 可见行: item0(y=4..28), item1(y=28..52) ✓; item2(y=52..76) ✗
        let theme = crate::theme::test_theme();
        let w = make_list(
            vec![item("a"), item("b"), item("c"), item("d")],
            Rect::new(0.0, 0.0, 220.0, 60.0),
        );
        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);
        // bg + PushClip + 2 text + PopClip = 5
        assert_eq!(list.cmds.len(), 5);
    }

    #[test]
    fn click_in_row_returns_selected_index() {
        let mut w =
            make_list(vec![item("a"), item("b"), item("c")], Rect::new(0.0, 0.0, 220.0, 100.0));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        // 第二行：top = 4 + 24 = 28；中线 40
        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: 100.0, py: 40.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert!(w.is_capturing());
        assert_eq!(
            w.on_event(
                &Event::MouseUp { px: 100.0, py: 40.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::List(ListAction::Selected(1)))
        );
        assert!(!w.is_capturing());
    }

    #[test]
    fn releasing_on_another_row_or_outside_cancels_selection() {
        let mut w =
            make_list(vec![item("a"), item("b"), item("c")], Rect::new(0.0, 0.0, 220.0, 100.0));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: 100.0, py: 16.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            w.on_event(
                &Event::MouseUp { px: 100.0, py: 40.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );

        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: 100.0, py: 16.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            w.on_event(
                &Event::MouseUp { px: 300.0, py: 160.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
    }

    #[test]
    fn list_leave_preserves_press_and_cancel_is_idempotent() {
        let mut widget = make_list(vec![item("a"), item("b")], Rect::new(0.0, 0.0, 220.0, 100.0));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        assert_eq!(
            widget.on_event(&Event::MouseMove { px: 100.0, py: 16.0 }, &mut ctx),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            widget.on_event(
                &Event::MouseDown { px: 100.0, py: 16.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(widget.on_event(&Event::PointerLeave, &mut ctx), Some(WidgetAction::Consumed));
        assert_eq!(widget.hovered_index(), None);
        assert!(!widget.close_hovered);
        assert!(widget.is_capturing());

        assert_eq!(
            widget.on_event(&Event::InteractionCancel, &mut ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!widget.is_capturing());
        assert_eq!(widget.on_event(&Event::InteractionCancel, &mut ctx), None);
        assert_eq!(
            widget.on_event(
                &Event::MouseUp { px: 100.0, py: 16.0, button: MouseButton::Left },
                &mut ctx,
            ),
            None
        );
    }

    #[test]
    fn close_press_release_emits_only_close_action() {
        let mut w = make_list(vec![item("a")], Rect::new(0.0, 0.0, 220.0, 40.0));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        let row_rect = w.item_rect(0, 1.0);
        let close_x = row_rect.right() - style().pad_x_logical - CLOSE_BTN_SIZE_LOGICAL * 0.5;
        let close_y = row_rect.y + row_rect.h * 0.5;
        let _ = w.on_event(&Event::MouseMove { px: close_x, py: close_y }, &mut ctx);

        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: close_x, py: close_y, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(
            w.on_event(
                &Event::MouseUp { px: close_x, py: close_y, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::List(ListAction::CloseRequested(0)))
        );
    }

    #[test]
    fn focusable_vertical_list_skips_non_selectable_rows_and_activates_focused_row() {
        let list_id = crate::WidgetId(41);
        let mut w = ListWidget::new(style(), Orientation::Vertical).with_id(list_id);
        w.set_items(vec![
            ListItem { label: "Header".into(), kind: ListItemKind::Header, ..ListItem::default() },
            item("a"),
            ListItem { kind: ListItemKind::Separator, ..ListItem::default() },
            item("b"),
        ]);
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 120.0), &mut layout_ctx(&theme, &mut measure));
        let mut focusable_ids = Vec::new();
        w.collect_focusable_ids(&mut focusable_ids);
        assert_eq!(focusable_ids, vec![list_id]);

        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        assert_eq!(
            w.on_event(
                &Event::KeyDown(crate::KeyCode::Down, crate::core::Modifiers::NONE),
                &mut ctx
            ),
            None
        );
        w.set_keyboard_focus(Some(list_id));
        assert_eq!(w.focused_index(), Some(1));
        assert_eq!(
            w.on_event(
                &Event::KeyDown(crate::KeyCode::Down, crate::core::Modifiers::NONE),
                &mut ctx
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(w.focused_index(), Some(3));
        assert_eq!(
            w.on_event(
                &Event::KeyDown(crate::KeyCode::Enter, crate::core::Modifiers::NONE),
                &mut ctx
            ),
            Some(WidgetAction::List(ListAction::Selected(3)))
        );
        assert_eq!(
            w.on_event(
                &Event::KeyDown(crate::KeyCode::Char(' '), crate::core::Modifiers::NONE),
                &mut ctx,
            ),
            Some(WidgetAction::List(ListAction::Selected(3)))
        );
    }

    #[test]
    fn horizontal_list_uses_left_right_and_home_end() {
        let list_id = crate::WidgetId(42);
        let mut w = ListWidget::new(style(), Orientation::Horizontal).with_id(list_id);
        w.set_items(vec![item("a"), item("b"), item("c")]);
        w.set_keyboard_focus(Some(list_id));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        assert_eq!(w.focused_index(), Some(0));
        assert_eq!(
            w.on_event(
                &Event::KeyDown(crate::KeyCode::Right, crate::core::Modifiers::NONE),
                &mut ctx
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(w.focused_index(), Some(1));
        let _ = w
            .on_event(&Event::KeyDown(crate::KeyCode::End, crate::core::Modifiers::NONE), &mut ctx);
        assert_eq!(w.focused_index(), Some(2));
        let _ = w.on_event(
            &Event::KeyDown(crate::KeyCode::Home, crate::core::Modifiers::NONE),
            &mut ctx,
        );
        assert_eq!(w.focused_index(), Some(0));
        let _ = w.on_event(
            &Event::KeyDown(crate::KeyCode::Left, crate::core::Modifiers::NONE),
            &mut ctx,
        );
        assert_eq!(w.focused_index(), Some(0));
    }

    #[test]
    fn focused_row_is_normalized_when_items_change() {
        let list_id = crate::WidgetId(43);
        let mut w = ListWidget::new(style(), Orientation::Vertical).with_id(list_id);
        w.set_items(vec![item("a"), item("b"), item("c")]);
        w.set_active(Some(1));
        w.set_keyboard_focus(Some(list_id));
        assert_eq!(w.focused_index(), Some(1));

        w.set_items(vec![
            item("replacement"),
            ListItem { kind: ListItemKind::Header, ..ListItem::default() },
        ]);

        assert_eq!(w.focused_index(), Some(0));
    }

    #[test]
    fn pointer_press_on_focusable_list_requests_focus_and_keeps_release_selection() {
        let list_id = crate::WidgetId(44);
        let mut w = ListWidget::new(style(), Orientation::Vertical).with_id(list_id);
        w.set_items(vec![item("a")]);
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout_ctx(&theme, &mut measure));
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

        assert_eq!(
            w.on_event(
                &Event::MouseDown { px: 100.0, py: 16.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::Control(crate::core::widget::ControlAction::FocusRequested {
                id: list_id,
            }))
        );
        w.set_keyboard_focus(Some(list_id));
        assert_eq!(
            w.on_event(
                &Event::MouseUp { px: 100.0, py: 16.0, button: MouseButton::Left },
                &mut ctx,
            ),
            Some(WidgetAction::List(ListAction::Selected(0)))
        );
    }

    #[test]
    fn click_on_separator_returns_none() {
        let mut w = make_list(
            vec![
                item("a.rs"),
                ListItem {
                    label: "".into(),
                    kind: ListItemKind::Separator,
                    icon: None,
                    indicator: ListItemIndicator::None,
                    pinned: false,
                    extra_label: None,
                    is_active: false,
                    closeable: false,
                },
                item("b.rs"),
            ],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        // Separator 行中线: pad_y(4) + row_h(24) + row_h/2 = 4+24+12 = 40
        let action = w.on_event(
            &Event::MouseDown { px: 100.0, py: 40.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(action.is_none(), "Clicking a separator should return None");
    }

    #[test]
    fn click_on_header_returns_none() {
        let mut w = make_list(
            vec![
                ListItem {
                    label: "Section".into(),
                    kind: ListItemKind::Header,
                    icon: None,
                    indicator: ListItemIndicator::None,
                    pinned: false,
                    extra_label: None,
                    is_active: false,
                    closeable: false,
                },
                item("a.rs"),
            ],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
        // Header 行中线: pad_y(4) + row_h/2 = 4+12 = 16
        let action = w.on_event(
            &Event::MouseDown { px: 100.0, py: 16.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(action.is_none(), "Clicking a header should return None");
    }

    #[test]
    fn active_item_gets_active_bg() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![item("a.rs"), item("b.rs"), item("c.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);
        w.set_active(Some(1));

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        // bg fill + fill_menu_hover for active row + 3 text = 5
        assert_eq!(
            list.cmds.len(),
            7,
            "Should have bg + active highlight + 3 labels + PushClip + PopClip"
        );
    }

    #[test]
    fn separator_item_does_not_emit_text() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![
            item("before"),
            ListItem {
                label: "".into(),
                kind: ListItemKind::Separator,
                icon: None,
                indicator: ListItemIndicator::None,
                pinned: false,
                extra_label: None,
                is_active: false,
                closeable: false,
            },
            item("after"),
        ]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        let text_count =
            list.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).count();
        assert_eq!(text_count, 2, "Only non-separator items should emit text");
    }

    #[test]
    fn separator_item_rect_spans_full_width() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![
            item("a"),
            ListItem {
                label: "".into(),
                kind: ListItemKind::Separator,
                icon: None,
                indicator: ListItemIndicator::None,
                pinned: false,
                extra_label: None,
                is_active: false,
                closeable: false,
            },
        ]);
        w.set_rect(Rect::new(0.0, 0.0, 300.0, 80.0), &mut layout);

        let sep_rect = w.item_rect(1, 1.0);
        assert_eq!(sep_rect.w, 300.0, "Separator should span full width");
    }

    #[test]
    fn hit_row_returns_none_for_separator() {
        let mut w = make_list(
            vec![
                item("a.rs"),
                ListItem {
                    label: "".into(),
                    kind: ListItemKind::Separator,
                    icon: None,
                    indicator: ListItemIndicator::None,
                    pinned: false,
                    extra_label: None,
                    is_active: false,
                    closeable: false,
                },
                item("b.rs"),
            ],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        // Separator 行中线: pad_y(4) + row_h(24) + row_h/2 = 4+24+12 = 40
        assert_eq!(w.hit_row(100.0, 40.0, 1.0), None, "Separator should not be hittable");
    }

    #[test]
    fn hit_row_returns_none_for_header() {
        let mut w = make_list(
            vec![
                ListItem {
                    label: "Section".into(),
                    kind: ListItemKind::Header,
                    icon: None,
                    indicator: ListItemIndicator::None,
                    pinned: false,
                    extra_label: None,
                    is_active: false,
                    closeable: false,
                },
                item("a.rs"),
            ],
            Rect::new(0.0, 0.0, 220.0, 100.0),
        );
        assert_eq!(w.hit_row(100.0, 16.0, 1.0), None, "Header should not be hittable");
    }

    #[test]
    fn indicator_dot_emits_asterisk_after_label() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![ListItem {
            label: "x".into(),
            kind: ListItemKind::Normal,
            icon: None,
            indicator: ListItemIndicator::Dot,
            pinned: false,
            extra_label: None,
            is_active: false,
            closeable: true,
        }]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        let texts: Vec<&str> = list
            .cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["x", "*"], "Should emit label then indicator dot");
    }

    #[test]
    fn indicator_dot_item_rect_spans_full_width() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![ListItem {
            label: "main.rs".into(),
            kind: ListItemKind::Normal,
            icon: None,
            indicator: ListItemIndicator::Dot,
            pinned: false,
            extra_label: None,
            is_active: false,
            closeable: true,
        }]);
        w.set_rect(Rect::new(0.0, 0.0, 300.0, 40.0), &mut layout);

        let r = w.item_rect(0, 1.0);
        assert_eq!(r.w, 300.0, "Item rect should span full container width");
    }

    #[test]
    fn indicator_dot_hit_center_returns_index() {
        let mut w = make_list(
            vec![ListItem {
                label: "x".into(),
                kind: ListItemKind::Normal,
                icon: None,
                indicator: ListItemIndicator::Dot,
                pinned: false,
                extra_label: None,
                is_active: false,
                closeable: true,
            }],
            Rect::new(0.0, 0.0, 220.0, 40.0),
        );
        let center_y = 4.0 + 12.0; // pad_y + row_h/2
        assert_eq!(w.hit_row(110.0, center_y, 1.0), Some(0));
    }

    #[test]
    fn pinned_item_has_accent_bar() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![pinned_item("main.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        let rounded_count = list
            .cmds
            .iter()
            .filter(|c| matches!(c, DrawCmd::FillRect { radius, .. } if *radius > 0.0))
            .count();
        assert!(rounded_count >= 1, "Pinned item should have a rounded accent bar");
    }

    #[test]
    fn pinned_item_rect_spans_full_width() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![pinned_item("main.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 300.0, 40.0), &mut layout);

        let r = w.item_rect(0, 1.0);
        assert_eq!(r.w, 300.0, "Pinned item rect should span full container width");
    }

    #[test]
    fn pinned_item_hit_center_returns_index() {
        let mut w = make_list(vec![pinned_item("main.rs")], Rect::new(0.0, 0.0, 220.0, 40.0));
        let center_y = 4.0 + 12.0;
        assert_eq!(w.hit_row(110.0, center_y, 1.0), Some(0));
    }

    #[test]
    fn pinned_bar_position_changes_with_pad_x() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);

        let s1 = ListStyle { pad_x_logical: 4.0, ..style() };
        let mut w1 = ListWidget::new(s1, Orientation::Vertical);
        w1.set_items(vec![pinned_item("x.rs")]);
        w1.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);
        let mut list1 = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint1 = PaintCtx {
            global_alpha: 1.0,
            list: &mut list1,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w1.paint(&mut paint1);
        let bar1 = list1
            .cmds
            .iter()
            .find(|c| matches!(c, DrawCmd::FillRect { radius, .. } if *radius > 0.0))
            .unwrap();

        let s2 = ListStyle { pad_x_logical: 20.0, ..style() };
        let mut w2 = ListWidget::new(s2, Orientation::Vertical);
        w2.set_items(vec![pinned_item("x.rs")]);
        w2.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);
        let mut list2 = DrawList::new();
        let mut shaper2 = shaping::Shaper::new().unwrap();
        let mut paint2 = PaintCtx {
            global_alpha: 1.0,
            list: &mut list2,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper2),
        };
        w2.paint(&mut paint2);
        let bar2 = list2
            .cmds
            .iter()
            .find(|c| matches!(c, DrawCmd::FillRect { radius, .. } if *radius > 0.0))
            .unwrap();

        // Both bars should be at their respective pad_x positions, so they differ
        match (bar1, bar2) {
            (DrawCmd::FillRect { rect: r1, .. }, DrawCmd::FillRect { rect: r2, .. }) => {
                assert!(
                    r2.x > r1.x,
                    "Bar with larger pad_x should be further right: pad4={} pad20={}",
                    r1.x,
                    r2.x
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn pinned_item_label_starts_after_bar() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![pinned_item("main.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        let bar = list
            .cmds
            .iter()
            .find(|c| matches!(c, DrawCmd::FillRect { radius, .. } if *radius > 0.0))
            .unwrap();
        let text = list.cmds.iter().find(|c| matches!(c, DrawCmd::TextLayout { .. })).unwrap();

        let bar_x = match bar {
            DrawCmd::FillRect { rect, .. } => rect.x,
            _ => unreachable!(),
        };
        let text_x = match text {
            DrawCmd::TextLayout { x, .. } => *x,
            _ => unreachable!(),
        };

        assert!(
            text_x > bar_x,
            "Label text ({}) should start after the pin bar ({})",
            text_x,
            bar_x
        );
    }

    #[test]
    fn long_label_is_truncated_with_ellipsis() {
        let theme = crate::theme::test_theme();
        let mut m = CharWidthMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![pinned_item("very_long_filename_that_exceeds_width.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 120.0, 40.0), &mut layout);

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        let label = list
            .cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::TextLayout { layout, .. } if layout.text != "*" => {
                    Some(layout.text.clone())
                }
                _ => None,
            })
            .unwrap();

        assert!(
            label.contains('\u{2026}')
                || label.len() < "very_long_filename_that_exceeds_width.rs".len(),
            "Long label should be truncated: '{}'",
            label
        );
    }

    #[test]
    fn pinned_item_close_button_not_shown() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![pinned_item("pinned.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);
        w.set_hovered_index(Some(0));

        let dpi = 1.0f32;
        let row_rect = w.item_rect(0, dpi);
        let pad_x = 8.0f32;
        let btn_size = 12.0f32;
        let btn_x = row_rect.x + row_rect.w - pad_x - btn_size + 6.0;
        let btn_y = row_rect.y + row_rect.h * 0.5;

        let hit = w.hit_close_btn(btn_x, btn_y, dpi);
        assert_eq!(hit, None, "Pinned items should not have close button");
    }

    #[test]
    fn nonpinned_item_close_button_shown_on_hover() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![item("file.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);
        w.set_hovered_index(Some(0));

        let dpi = 1.0f32;
        let row_rect = w.item_rect(0, dpi);
        let pad_x = 8.0f32;
        let btn_size = 12.0f32;
        let btn_x = row_rect.x + row_rect.w - pad_x - btn_size + 6.0;
        let btn_y = row_rect.y + row_rect.h * 0.5;

        let hit = w.hit_close_btn(btn_x, btn_y, dpi);
        assert_eq!(hit, Some(0), "Hovered non-pinned item should have close button");
    }

    #[test]
    fn nonpinned_item_close_button_not_shown_without_hover() {
        let mut w = make_list(vec![item("file.rs")], Rect::new(0.0, 0.0, 220.0, 40.0));

        let dpi = 1.0f32;
        let row_rect = w.item_rect(0, dpi);
        let pad_x = 8.0f32;
        let btn_size = 12.0f32;
        let btn_x = row_rect.x + row_rect.w - pad_x - btn_size + 6.0;
        let btn_y = row_rect.y + row_rect.h * 0.5;

        let hit = w.hit_close_btn(btn_x, btn_y, dpi);
        assert_eq!(hit, None, "Non-hovered item should not have close button hit");
    }

    #[test]
    fn truncation_considers_close_button_space() {
        let theme = crate::theme::test_theme();
        let mut m = CharWidthMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![item("abcdefghijklmnop.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 120.0, 40.0), &mut layout);

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        let text = list
            .cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::TextLayout { layout, .. } if layout.text != "x" && layout.text != "*" => {
                    Some(layout.text.clone())
                }
                _ => None,
            })
            .unwrap();

        assert!(
            text.contains('\u{2026}') || text.len() < "abcdefghijklmnop.rs".len(),
            "Label should be truncated to make room for close button: '{}'",
            text
        );
    }

    #[test]
    fn hover_truncation_same_as_nonhover_for_nonpinned() {
        let theme = crate::theme::test_theme();

        // Non-hovered
        let mut m1 = CharWidthMeasure;
        let mut layout1 = layout_ctx(&theme, &mut m1);
        let mut w1 = ListWidget::new(style(), Orientation::Vertical);
        w1.set_items(vec![item("abcdefghijklmnop.rs")]);
        w1.set_rect(Rect::new(0.0, 0.0, 120.0, 40.0), &mut layout1);
        let mut list1 = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint1 = PaintCtx {
            global_alpha: 1.0,
            list: &mut list1,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w1.paint(&mut paint1);
        let text_no_hover = list1
            .cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::TextLayout { layout, .. } if layout.text != "*" => {
                    Some(layout.text.clone())
                }
                _ => None,
            })
            .unwrap();

        // Hovered
        let mut m2 = CharWidthMeasure;
        let mut layout2 = layout_ctx(&theme, &mut m2);
        let mut w2 = ListWidget::new(style(), Orientation::Vertical);
        w2.set_items(vec![item("abcdefghijklmnop.rs")]);
        w2.set_rect(Rect::new(0.0, 0.0, 120.0, 40.0), &mut layout2);
        w2.set_hovered_index(Some(0));
        let mut list2 = DrawList::new();
        let mut shaper2 = shaping::Shaper::new().unwrap();
        let mut paint2 = PaintCtx {
            global_alpha: 1.0,
            list: &mut list2,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper2),
        };
        w2.paint(&mut paint2);
        let text_hover = list2
            .cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::TextLayout { layout, .. } if layout.text != "x" && layout.text != "*" => {
                    Some(layout.text.clone())
                }
                _ => None,
            })
            .unwrap();

        // closeable 项无论是否 hover 都预留关闭按钮空间，因此标签相同
        assert_eq!(
            text_hover, text_no_hover,
            "Non-pinned labels should be same regardless of hover: hover='{}' vs no_hover='{}'",
            text_hover, text_no_hover
        );
        assert!(text_no_hover.contains('…'), "Label should be truncated: '{text_no_hover}'");
    }

    #[test]
    fn all_pinned_items_no_close_button() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![pinned_item("a.rs"), pinned_item("b.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);
        w.set_hovered_index(Some(0));

        let dpi = 1.0f32;
        let row_rect = w.item_rect(0, dpi);
        let btn_x = row_rect.x + row_rect.w - 8.0 - 12.0 + 6.0;
        let btn_y = row_rect.y + row_rect.h * 0.5;

        let hit = w.hit_close_btn(btn_x, btn_y, dpi);
        assert_eq!(hit, None, "All pinned items should have no close button");
    }

    #[test]
    fn mixed_pinned_close_btn_only_on_unpinned() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        // pinned first after sort, then unpinned
        w.set_items(vec![pinned_item("pinned.rs"), item("normal.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);
        let dpi = 1.0f32;

        // Hover pinned item (index 0) — no close button
        w.set_hovered_index(Some(0));
        let row0 = w.item_rect(0, dpi);
        let btn_x = row0.x + row0.w - 8.0 - 12.0 + 6.0;
        let btn_y = row0.y + row0.h * 0.5;
        assert_eq!(
            w.hit_close_btn(btn_x, btn_y, dpi),
            None,
            "Pinned item should not have close button"
        );

        // Hover unpinned item (index 1) — has close button
        w.set_hovered_index(Some(1));
        let row1 = w.item_rect(1, dpi);
        let btn_x2 = row1.x + row1.w - 8.0 - 12.0 + 6.0;
        let btn_y2 = row1.y + row1.h * 0.5;
        assert_eq!(
            w.hit_close_btn(btn_x2, btn_y2, dpi),
            Some(1),
            "Unpinned item should have close button"
        );
    }

    #[test]
    fn pinned_dirty_dot_not_covered_by_filename() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![ListItem {
            label: "main.rs".into(),
            kind: ListItemKind::Normal,
            icon: None,
            indicator: ListItemIndicator::Dot,
            pinned: true,
            extra_label: None,
            is_active: false,
            closeable: false,
        }]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);

        let mut list = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut paint);

        // Find label text and dot text commands
        let text_cmds: Vec<(f32, &str)> = list
            .cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::TextLayout { x, layout, .. } => Some((*x, layout.text.as_str())),
                _ => None,
            })
            .collect();
        let label_cmd = text_cmds.iter().find(|(_, s)| *s == "main.rs").expect("label");
        let dot_cmd = text_cmds.iter().find(|(_, s)| *s == "*").expect("dot");

        // Dot must be to the right of the label start
        assert!(
            dot_cmd.0 > label_cmd.0,
            "Dot x ({}) should be > label x ({})",
            dot_cmd.0,
            label_cmd.0
        );

        // Dot must be outside the label text area: label_x + some_min_offset < dot_x
        // With left_offset = (2+6)*1 = 8, label starts at pad_x(8) + 8 = 16
        // Dot should be at 16 + label_w + 2, well past the label
        let pad_x = 8.0f32; // style pad_x_logical * dpi
        let left_offset = (2.0 + 6.0) * 1.0; // PIN_BAR_WIDTH + PIN_BAR_MARGIN
        let label_start = pad_x + left_offset;
        assert!(
            dot_cmd.0 >= label_start + 1.0,
            "Dot x ({}) should be past pinned label start ({})",
            dot_cmd.0,
            label_start
        );
    }

    #[test]
    fn horizontal_item_rects_layout_left_to_right() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Horizontal);
        w.set_items(vec![item("a"), item("b"), item("c")]);
        w.set_rect(Rect::new(0.0, 0.0, 400.0, 28.0), &mut layout);

        let r0 = w.item_rect(0, 1.0);
        let r1 = w.item_rect(1, 1.0);
        let r2 = w.item_rect(2, 1.0);

        assert_eq!(r0.x, 8.0); // pad_x
        assert_eq!(r1.x, 8.0 + 120.0); // pad_x + item_w
        assert_eq!(r2.x, 8.0 + 240.0);
        assert_eq!(r0.h, 28.0);
    }

    #[test]
    fn horizontal_hit_row() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Horizontal);
        w.set_items(vec![item("a"), item("b")]);
        w.set_rect(Rect::new(0.0, 0.0, 400.0, 28.0), &mut layout);

        // First item center
        assert_eq!(w.hit_row(8.0 + 60.0, 14.0, 1.0), Some(0));
        // Second item center
        assert_eq!(w.hit_row(8.0 + 120.0 + 60.0, 14.0, 1.0), Some(1));
    }

    #[test]
    fn horizontal_pinned_item_has_no_left_offset() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Horizontal);
        let pinned = pinned_item("pinned.rs");
        w.set_items(vec![pinned.clone()]);
        w.set_rect(Rect::new(0.0, 0.0, 400.0, 28.0), &mut layout);

        // In Horizontal mode, pinned_left_offset should be 0 (pin bar is at top, not left)
        let dpi = 1.0f32;
        let offset = w.pinned_left_offset(&pinned, dpi);
        assert_eq!(offset, 0.0, "Horizontal pinned item should have 0 left offset, got {}", offset);
    }

    #[test]
    fn hit_close_btn_clipped_by_row_rect() {
        let theme = crate::theme::test_theme();
        let mut m = NoopMeasure;
        let mut layout = layout_ctx(&theme, &mut m);
        let mut w = ListWidget::new(style(), Orientation::Vertical);
        w.set_items(vec![item("file.rs")]);
        w.set_rect(Rect::new(0.0, 0.0, 220.0, 40.0), &mut layout);
        w.set_hovered_index(Some(0));

        let dpi = 1.0f32;
        let row_rect = w.item_rect(0, dpi);
        let pad_x = 8.0f32;
        let btn_size = 12.0f32;
        let hit_pad = 6.0f32;

        // btn_rect extends beyond row_rect top/bottom by hit_pad
        // But row_rect.contains should clip it
        let btn_center_x = row_rect.x + row_rect.w - pad_x - btn_size + hit_pad;

        // Just above row_rect — in btn_rect hit_pad zone but outside row_rect
        let above_y = row_rect.y - hit_pad * 0.5;
        assert_eq!(
            w.hit_close_btn(btn_center_x, above_y, dpi),
            None,
            "Click above row_rect should be clipped even if in btn hit_pad zone"
        );

        // Just below row_rect
        let below_y = row_rect.y + row_rect.h + hit_pad * 0.5;
        assert_eq!(
            w.hit_close_btn(btn_center_x, below_y, dpi),
            None,
            "Click below row_rect should be clipped even if in btn hit_pad zone"
        );

        // Inside row_rect at btn center — should hit
        let inside_y = row_rect.y + row_rect.h * 0.5;
        assert_eq!(
            w.hit_close_btn(btn_center_x, inside_y, dpi),
            Some(0),
            "Click at btn center inside row_rect should hit"
        );
    }
}
