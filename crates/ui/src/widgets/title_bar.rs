//! TitleBarWidget — Sidebar 模式下显示在内容区顶部的标题栏。
//!
//! 显示：文件名（正常大小）+ 完整路径（更小、更淡的颜色）。
//! 高度与 sidebar header 一致（HEADER_H = 28px * dpi）。

use crate::core::widget::WidgetAction;
use crate::core::{Event, EventCtx, LayoutCtx, MouseButton, PaintCtx, Rect, Widget};
use crate::core::{TextMeasure, text_util::truncate_title_precise};
use crate::widgets::icon::draw_icon;
use crate::widgets::tooltip::TooltipHint;
use std::any::Any;
use std::path::PathBuf;
use winit::window::CursorIcon;

const ACTION_HORIZONTAL_INSET_LOGICAL: f32 = 8.0;
const ACTION_VERTICAL_INSET_LOGICAL: f32 = 4.0;
const ACTION_GAP_LOGICAL: f32 = 4.0;
const ACTION_ICON_SIZE_LOGICAL: f32 = 18.0;
const TEXT_HORIZONTAL_INSET_LOGICAL: f32 = 12.0;
const TEXT_ACTION_GAP_LOGICAL: f32 = 8.0;
const TITLE_PATH_GAP_LOGICAL: f32 = 8.0;
const TITLE_FONT_SIZE_LOGICAL: f32 = 13.0;
const PATH_FONT_SIZE_LOGICAL: f32 = 11.0;
const CENTERED_TEXT_BASELINE_EM: f32 = 0.35;
const ELLIPSIS: &str = "…";

/// mmap 风格按钮的标题栏输入。
#[derive(Clone, Copy, Debug)]
pub struct MindmapStyleButtonInput {
    /// 右侧风格面板当前是否可见。
    pub panel_visible: bool,
}

/// Height of the title bar given an explicit DPI scale (avoids RefCell borrow).
pub fn title_bar_height(dpi_scale: f32) -> f32 {
    crate::constants::TITLE_BAR_HEIGHT * dpi_scale
}

/// App 每帧注入的 titlebar 输入数据。
#[derive(Clone, Debug)]
pub struct TitleBarInput {
    /// 当前文件路径（None 表示未命名文档）。
    pub file_path: Option<PathBuf>,
    /// titlebar 左侧偏移（sidebar 宽度或 hamburger 按钮右边缘）。
    pub sidebar_left: f32,
    /// titlebar 在屏幕上的 x 坐标（＝ sidebar 厚度），用于将 sidebar_left
    /// 从屏幕坐标转换为 widget 局部坐标，避免背景覆盖汉堡按钮。
    pub titlebar_x: f32,
    /// 当前文件是否可切换视图（有 toggle_target）。
    pub can_toggle: bool,
    /// 当前是否处于切换后的视图（控制按钮高亮）。
    pub toggled: bool,
    /// 切换按钮 tooltip 文本（如 "基础编辑"、"小说模式"）。
    pub toggle_label: Option<String>,
    /// Whether the TOC panel is currently visible.
    pub toc_visible: bool,
    /// Whether the TOC toggle button should be shown (only in markdown preview mode).
    pub toc_enabled: bool,
    /// mmap 风格按钮；非 mmap 视图传入 `None`。
    pub mindmap_style: Option<MindmapStyleButtonInput>,
}

/// Actions emitted by the title bar widget.
#[derive(Debug, Clone, PartialEq)]
pub enum TitleBarAction {
    /// 切换当前文件的视图模式。
    ToggleView,
    /// Toggle the table of contents panel.
    ToggleToc,
    /// Toggle the mmap style panel.
    ToggleMindmapStylePanel,
}

pub struct TitleBarWidget {
    rect: Rect,
    input: Option<TitleBarInput>,
    /// 布局后实际显示的文件名。
    displayed_name: String,
    /// 布局后实际显示的父路径；空间不足时为空。
    displayed_path: String,
    /// 标题文字可绘制和命中的安全区域。
    text_rect: Rect,
    /// 实际显示文字占据的区域，用于避免空白标题栏弹出 tooltip。
    text_hit_rect: Rect,
    /// 缓存实际显示文件名的宽度。
    name_width: f32,
    /// Precomputed toggle button rect (for hit testing).
    toggle_rect: Rect,
    /// Precomputed mmap style button rect (for hit testing).
    mindmap_style_rect: Rect,
    /// Precomputed TOC toggle button rect (for hit testing).
    toc_rect: Rect,
    toggle_hovered: bool,
    mindmap_style_hovered: bool,
    toc_hovered: bool,
}

impl Default for TitleBarWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleBarWidget {
    pub fn new() -> Self {
        Self {
            rect: Rect::ZERO,
            input: None,
            displayed_name: String::new(),
            displayed_path: String::new(),
            text_rect: Rect::ZERO,
            text_hit_rect: Rect::ZERO,
            name_width: 0.0,
            toggle_rect: Rect::ZERO,
            mindmap_style_rect: Rect::ZERO,
            toc_rect: Rect::ZERO,
            toggle_hovered: false,
            mindmap_style_hovered: false,
            toc_hovered: false,
        }
    }

    pub fn set_input(&mut self, input: TitleBarInput) {
        self.input = Some(input);
    }

    #[cfg(test)]
    fn toggle_rect_for_test(&self) -> Rect {
        self.toggle_rect
    }

    #[cfg(test)]
    fn mindmap_style_rect_for_test(&self) -> Rect {
        self.mindmap_style_rect
    }

    fn layout_text(&mut self, input: &TitleBarInput, dpi: f32, measure: &mut dyn TextMeasure) {
        self.displayed_name.clear();
        self.displayed_path.clear();
        self.text_hit_rect = Rect::ZERO;
        self.name_width = 0.0;
        if self.text_rect.w <= 0.0 {
            return;
        }

        let (full_name, full_path) = title_parts(input);
        let name_font_size = TITLE_FONT_SIZE_LOGICAL * dpi;
        self.displayed_name =
            truncate_or_hide(&full_name, self.text_rect.w, name_font_size, measure);
        self.name_width = measure.measure(&self.displayed_name, name_font_size);
        self.text_hit_rect = Rect::new(
            self.text_rect.x,
            self.text_rect.y,
            self.name_width.min(self.text_rect.w),
            self.text_rect.h,
        );
        if full_path.is_empty() {
            return;
        }

        let title_path_gap = TITLE_PATH_GAP_LOGICAL * dpi;
        let path_width = (self.text_rect.w - self.name_width - title_path_gap).max(0.0);
        self.displayed_path =
            truncate_or_hide(&full_path, path_width, PATH_FONT_SIZE_LOGICAL * dpi, measure);
        if !self.displayed_path.is_empty() {
            let displayed_path_width =
                measure.measure(&self.displayed_path, PATH_FONT_SIZE_LOGICAL * dpi);
            self.text_hit_rect.w =
                (self.name_width + title_path_gap + displayed_path_width).min(self.text_rect.w);
        }
    }
}

fn background_start(input: &TitleBarInput, title_bar_width: f32) -> f32 {
    (input.sidebar_left - input.titlebar_x).clamp(0.0, title_bar_width.max(0.0))
}

fn title_parts(input: &TitleBarInput) -> (String, String) {
    let Some(file_path) = input.file_path.as_ref() else {
        return ("untitled".to_owned(), String::new());
    };
    let filename = file_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled".to_owned());
    let parent_path =
        file_path.parent().map(|parent| parent.to_string_lossy().into_owned()).unwrap_or_default();
    (filename, parent_path)
}

fn truncate_or_hide(
    text: &str,
    available_width: f32,
    font_size: f32,
    measure: &mut dyn TextMeasure,
) -> String {
    if text.is_empty() || available_width <= 0.0 {
        return String::new();
    }
    if measure.measure(text, font_size) <= available_width {
        return text.to_owned();
    }
    if measure.measure(ELLIPSIS, font_size) > available_width {
        return String::new();
    }

    let truncated = truncate_title_precise(text, available_width, font_size, measure);
    if measure.measure(&truncated, font_size) <= available_width {
        truncated
    } else {
        ELLIPSIS.to_owned()
    }
}

fn next_action_rect(
    next_right: &mut f32,
    minimum_x: f32,
    button_size: f32,
    button_y: f32,
    action_gap: f32,
) -> Rect {
    let button_x = *next_right - button_size;
    *next_right = button_x - action_gap;
    if button_size <= 0.0 || button_x < minimum_x {
        return Rect::ZERO;
    }
    Rect::new(button_x, button_y, button_size, button_size)
}

impl Widget for TitleBarWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        if let Some(input) = self.input.clone() {
            let dpi = ctx.dpi;
            let background_x = background_start(&input, rect.w);
            let btn_size = (rect.h - 2.0 * ACTION_VERTICAL_INSET_LOGICAL * dpi).max(0.0);
            let btn_y = ACTION_VERTICAL_INSET_LOGICAL * dpi;
            let action_gap = ACTION_GAP_LOGICAL * dpi;
            let mut next_action_right =
                (rect.w - ACTION_HORIZONTAL_INSET_LOGICAL * dpi).clamp(0.0, rect.w.max(0.0));

            if input.can_toggle {
                self.toggle_rect = next_action_rect(
                    &mut next_action_right,
                    background_x,
                    btn_size,
                    btn_y,
                    action_gap,
                );
            } else {
                self.toggle_rect = Rect::ZERO;
            }

            if input.mindmap_style.is_some() {
                self.mindmap_style_rect = next_action_rect(
                    &mut next_action_right,
                    background_x,
                    btn_size,
                    btn_y,
                    action_gap,
                );
            } else {
                self.mindmap_style_rect = Rect::ZERO;
            }

            if input.can_toggle && input.toc_enabled {
                self.toc_rect = next_action_rect(
                    &mut next_action_right,
                    background_x,
                    btn_size,
                    btn_y,
                    action_gap,
                );
            } else {
                self.toc_rect = Rect::ZERO;
            }

            let text_x = background_x + TEXT_HORIZONTAL_INSET_LOGICAL * dpi;
            let first_action_x = [self.toc_rect, self.mindmap_style_rect, self.toggle_rect]
                .into_iter()
                .filter(|action_rect| action_rect.w > 0.0)
                .map(|action_rect| action_rect.x)
                .reduce(f32::min);
            let text_right = first_action_x
                .map(|action_x| action_x - TEXT_ACTION_GAP_LOGICAL * dpi)
                .unwrap_or(rect.w - ACTION_HORIZONTAL_INSET_LOGICAL * dpi)
                .clamp(text_x.min(rect.w.max(0.0)), rect.w.max(0.0));
            self.text_rect =
                Rect::new(text_x, 0.0, (text_right - text_x).max(0.0), rect.h.max(0.0));

            let measure: &mut dyn TextMeasure = match ctx.ui_measure.as_deref_mut() {
                Some(ui_measure) => ui_measure,
                None => ctx.measure,
            };
            self.layout_text(&input, dpi, measure);

            debug_assert!(
                self.mindmap_style_rect == Rect::ZERO
                    || self.toggle_rect == Rect::ZERO
                    || self.mindmap_style_rect.right() <= self.toggle_rect.x
            );
            debug_assert!(
                self.toc_rect == Rect::ZERO
                    || self.mindmap_style_rect == Rect::ZERO
                    || self.toc_rect.right() <= self.mindmap_style_rect.x
            );
        } else {
            self.displayed_name.clear();
            self.displayed_path.clear();
            self.text_rect = Rect::ZERO;
            self.text_hit_rect = Rect::ZERO;
            self.name_width = 0.0;
            self.toggle_rect = Rect::ZERO;
            self.mindmap_style_rect = Rect::ZERO;
            self.toc_rect = Rect::ZERO;
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let Some(ref input) = self.input else { return };

        let dpi = ctx.dpi;
        let h = self.rect.h;

        // 1) 背景（左侧留出汉堡按钮空间，避免覆盖）
        let bg_x = background_start(input, self.rect.w);
        let bg = Rect::new(bg_x, 0.0, (self.rect.w - bg_x).max(0.0), h);
        if bg.w > 0.0 {
            ctx.list.fill(bg, ctx.theme.editor.background);
        }

        // 2) 文件名与路径
        if self.text_rect.w > 0.0
            && !self.displayed_name.is_empty()
            && let Some(ref mut shaper) = ctx.shaper
        {
            let application = ctx.theme.application_theme();
            let name_font_size = TITLE_FONT_SIZE_LOGICAL * dpi;
            let path_font_size = PATH_FONT_SIZE_LOGICAL * dpi;
            let name_baseline = h * 0.5 + name_font_size * CENTERED_TEXT_BASELINE_EM;
            let path_baseline = h * 0.5 + path_font_size * CENTERED_TEXT_BASELINE_EM;
            let text_x = self.text_rect.x;
            let path_x = text_x + self.name_width + TITLE_PATH_GAP_LOGICAL * dpi;
            ctx.list.clip(self.text_rect, |draw_list| {
                draw_list.text_shaped(
                    text_x,
                    name_baseline,
                    name_font_size,
                    application.text_primary,
                    &self.displayed_name,
                    shaper,
                );
                if !self.displayed_path.is_empty() {
                    draw_list.text_shaped(
                        path_x,
                        path_baseline,
                        path_font_size,
                        application.text_secondary,
                        &self.displayed_path,
                        shaper,
                    );
                }
            });
        }

        // 4) Markdown 预览切换按钮
        if input.can_toggle && self.toggle_rect.w > 0.0 {
            let r = &self.toggle_rect;
            let btn_bg = if self.toggle_hovered {
                ctx.theme.application_theme().hover_surface
            } else {
                ctx.theme.editor.background
            };
            ctx.list.fill_rounded(
                Rect::new(r.x, r.y, r.w, r.h),
                btn_bg,
                ctx.theme.control_metrics().compact_corner_radius_logical * dpi,
            );

            let icon_color = if input.toggled {
                ctx.theme.palette.accent
            } else {
                let mut c = ctx.theme.palette.text_muted;
                c[3] *= 0.6;
                c
            };
            let icon_size = ACTION_ICON_SIZE_LOGICAL * dpi;
            let icon_x = r.x + (r.w - icon_size) * 0.5;
            let icon_y = r.y + (r.h - icon_size) * 0.5;
            draw_icon(ctx.list, "eye", icon_x, icon_y, icon_size, icon_color);
        }

        // 5) mmap style button
        if let Some(style_input) = input.mindmap_style
            && self.mindmap_style_rect.w > 0.0
        {
            let r = &self.mindmap_style_rect;
            let btn_bg = if self.mindmap_style_hovered {
                ctx.theme.application_theme().hover_surface
            } else {
                ctx.theme.editor.background
            };
            ctx.list.fill_rounded(
                Rect::new(r.x, r.y, r.w, r.h),
                btn_bg,
                ctx.theme.control_metrics().compact_corner_radius_logical * dpi,
            );

            let icon_color = if style_input.panel_visible {
                ctx.theme.palette.accent
            } else {
                let mut color = ctx.theme.palette.text_muted;
                color[3] *= 0.6;
                color
            };
            let icon_size = ACTION_ICON_SIZE_LOGICAL * dpi;
            let icon_x = r.x + (r.w - icon_size) * 0.5;
            let icon_y = r.y + (r.h - icon_size) * 0.5;
            draw_icon(ctx.list, "palette", icon_x, icon_y, icon_size, icon_color);
        }

        // 6) TOC toggle button (only in markdown preview mode)
        if input.toc_enabled && self.toc_rect.w > 0.0 {
            let r = &self.toc_rect;
            let btn_bg = if self.toc_hovered {
                ctx.theme.application_theme().hover_surface
            } else {
                ctx.theme.editor.background
            };
            ctx.list.fill_rounded(
                Rect::new(r.x, r.y, r.w, r.h),
                btn_bg,
                ctx.theme.control_metrics().compact_corner_radius_logical * dpi,
            );

            let icon_color = if input.toc_visible {
                ctx.theme.palette.accent
            } else {
                let mut c = ctx.theme.palette.text_muted;
                c[3] *= 0.6;
                c
            };
            let icon_size = ACTION_ICON_SIZE_LOGICAL * dpi;
            let icon_x = r.x + (r.w - icon_size) * 0.5;
            let icon_y = r.y + (r.h - icon_size) * 0.5;
            draw_icon(ctx.list, "list-tree", icon_x, icon_y, icon_size, icon_color);
        }

        // 分割线已移至 paint_chrome() 中绘制（内容区顶部）
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        // 排除汉堡按钮区域（与背景填充的 bg_x 偏移一致）
        let bg_x =
            self.input.as_ref().map(|input| background_start(input, self.rect.w)).unwrap_or(0.0);
        let effective = Rect::new(bg_x, 0.0, self.rect.w - bg_x, self.rect.h);
        effective.contains(px, py)
    }

    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match ev {
            // MouseMove: only consume when inside rect, so the editor
            // can get the event and show its I-beam cursor when outside.
            Event::MouseMove { px, py } => {
                self.toc_hovered = self.toc_rect.w > 0.0 && self.toc_rect.contains(*px, *py);
                self.mindmap_style_hovered =
                    self.mindmap_style_rect.w > 0.0 && self.mindmap_style_rect.contains(*px, *py);
                self.toggle_hovered =
                    self.toggle_rect.w > 0.0 && self.toggle_rect.contains(*px, *py);

                if self.toc_hovered || self.mindmap_style_hovered || self.toggle_hovered {
                    ctx.cursor_hint = Some(CursorIcon::Pointer);
                    return Some(WidgetAction::Consumed);
                }

                if self.rect.contains(*px, *py) {
                    ctx.cursor_hint = Some(CursorIcon::Default);
                    Some(WidgetAction::Consumed)
                } else {
                    self.toc_hovered = false;
                    self.mindmap_style_hovered = false;
                    self.toggle_hovered = false;
                    None
                }
            }
            // MouseDown: check toggle button first, then consume for title bar.
            Event::MouseDown { px, py, button } => {
                if *button == MouseButton::Left
                    && self.toggle_rect.w > 0.0
                    && self.toggle_rect.contains(*px, *py)
                {
                    return Some(WidgetAction::TitleBar(TitleBarAction::ToggleView));
                }
                if *button == MouseButton::Left
                    && self.mindmap_style_rect.w > 0.0
                    && self.mindmap_style_rect.contains(*px, *py)
                {
                    return Some(WidgetAction::TitleBar(TitleBarAction::ToggleMindmapStylePanel));
                }
                if *button == MouseButton::Left
                    && self.toc_rect.w > 0.0
                    && self.toc_rect.contains(*px, *py)
                {
                    return Some(WidgetAction::TitleBar(TitleBarAction::ToggleToc));
                }
                if self.rect.contains(*px, *py) {
                    ctx.cursor_hint = Some(CursorIcon::Default);
                    Some(WidgetAction::Consumed)
                } else {
                    None
                }
            }
            // MouseUp/Wheel: always consume (dock dispatches MouseUp
            // to ALL children without hit testing; title bar must
            // consume to prevent fallthrough to editor).
            Event::MouseUp { .. } | Event::Wheel { .. } => Some(WidgetAction::Consumed),
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        if self.mindmap_style_rect.w > 0.0 && self.mindmap_style_rect.contains(px, py) {
            return Some(TooltipHint {
                label: "思维导图风格".to_string(),
                target_rect: self.mindmap_style_rect,
            });
        }
        if self.toc_rect.w > 0.0 && self.toc_rect.contains(px, py) {
            let label = if self.input.as_ref().is_some_and(|input| input.toc_visible) {
                "隐藏目录 ⌘⇧T"
            } else {
                "显示目录 ⌘⇧T"
            };
            return Some(TooltipHint { label: label.to_owned(), target_rect: self.toc_rect });
        }
        if self.toggle_rect.w > 0.0
            && self.toggle_rect.contains(px, py)
            && let Some(input) = &self.input
        {
            return input
                .toggle_label
                .as_ref()
                .map(|label| TooltipHint { label: label.clone(), target_rect: self.toggle_rect });
        }
        if self.text_hit_rect.w > 0.0 && self.text_hit_rect.contains(px, py) {
            let label = self
                .input
                .as_ref()
                .and_then(|input| input.file_path.as_ref())
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| "untitled".to_owned());
            return Some(TooltipHint { label, target_rect: self.text_hit_rect });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::{NoopMeasure, TextMeasure};
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::widget::{KeyCode, Modifiers, MouseButton};
    use crate::theme::Theme;

    struct EstimatedMeasure;

    impl TextMeasure for EstimatedMeasure {
        fn measure(&mut self, text: &str, font_size: f32) -> f32 {
            text.chars()
                .map(|character| crate::core::text_util::char_width(character, font_size))
                .sum()
        }
    }

    fn test_theme() -> Theme {
        let mut t = crate::theme::test_theme();
        t.editor.background = [0.1, 0.1, 0.1, 1.0];
        t.palette.text_muted = [0.8, 0.8, 0.8, 1.0];
        t.palette.border_subtle = [0.3, 0.3, 0.3, 1.0];
        t
    }

    fn test_title_bar_input() -> TitleBarInput {
        TitleBarInput {
            file_path: Some(PathBuf::from("/test.mmap")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("源码".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        }
    }

    fn laid_out_title_bar(input: TitleBarInput) -> TitleBarWidget {
        let mut widget = TitleBarWidget::new();
        widget.set_input(input);
        let theme = test_theme();
        let mut measure = NoopMeasure;
        let mut layout_ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        widget.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut layout_ctx);
        widget
    }

    fn laid_out_title_bar_at(
        input: TitleBarInput,
        width: f32,
        height: f32,
        dpi: f32,
        theme: &Theme,
    ) -> TitleBarWidget {
        let mut widget = TitleBarWidget::new();
        widget.set_input(input);
        let mut measure = EstimatedMeasure;
        let mut layout_ctx = LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi };
        widget.set_rect(Rect::new(0.0, 0.0, width, height), &mut layout_ctx);
        widget
    }

    fn paint_title_bar(widget: &TitleBarWidget, theme: &Theme, dpi: f32) -> DrawList {
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("测试环境必须能创建文字塑形器");
        let mut paint_ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme,
            dpi,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        widget.paint(&mut paint_ctx);
        draw_list
    }

    #[test]
    fn narrow_title_bar_clips_long_chinese_name_before_actions_and_keeps_full_tooltip() {
        let full_path = PathBuf::from(
            "/用户/项目/包含很多层级的资料目录/这是一份非常长的中文文件名用于验证标题栏裁剪.mmap",
        );
        let theme = test_theme();
        let widget = laid_out_title_bar_at(
            TitleBarInput {
                file_path: Some(full_path.clone()),
                toc_enabled: true,
                mindmap_style: Some(MindmapStyleButtonInput { panel_visible: false }),
                ..test_title_bar_input()
            },
            220.0,
            36.0,
            1.0,
            &theme,
        );

        let draw_list = paint_title_bar(&widget, &theme, 1.0);
        let clip_rect = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::PushClip(rect) => Some(*rect),
                _ => None,
            })
            .expect("标题文字应限制在裁剪区域内");
        let displayed_name = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .expect("窄标题栏仍应显示可辨认的文件名");

        assert!(displayed_name.contains('…'), "长文件名应省略：{displayed_name}");
        assert!(clip_rect.right() <= widget.toc_rect.x);
        let tooltip = widget
            .tooltip_at(clip_rect.x + 1.0, clip_rect.y + clip_rect.h * 0.5)
            .expect("标题文字区域应提供完整路径 tooltip");
        assert_eq!(tooltip.label, full_path.to_string_lossy());
    }

    #[test]
    fn title_and_path_use_shared_text_tokens_sizes_and_vertical_centering() {
        let theme = test_theme();
        let widget = laid_out_title_bar_at(
            TitleBarInput {
                file_path: Some(PathBuf::from(
                    "/用户/项目/包含很多层级的资料目录/另一层很长的目录/归档资料/设计稿/历史版本/notes.md",
                )),
                ..test_title_bar_input()
            },
            420.0,
            36.0,
            1.0,
            &theme,
        );

        let draw_list = paint_title_bar(&widget, &theme, 1.0);
        let text_commands: Vec<_> = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TextLayout { layout, y_baseline, color, .. } => {
                    Some((layout.as_ref(), *y_baseline, *color))
                }
                _ => None,
            })
            .collect();
        assert_eq!(text_commands.len(), 2, "文件名和路径都应有足够空间显示");

        let application = theme.application_theme();
        let (name_layout, name_baseline, name_color) = text_commands[0];
        assert_eq!(name_layout.font_size, 13.0);
        assert_eq!(name_color, application.text_primary);
        assert!((name_baseline - (18.0 + 13.0 * 0.35)).abs() < 0.01);

        let (path_layout, path_baseline, path_color) = text_commands[1];
        assert_eq!(path_layout.font_size, 11.0);
        assert_eq!(path_color, application.text_secondary);
        assert_eq!(path_color[3], application.text_secondary[3]);
        assert!(path_layout.text.contains('…'), "长路径应保留省略提示");
        assert!((path_baseline - (18.0 + 11.0 * 0.35)).abs() < 0.01);
    }

    #[test]
    fn narrow_action_geometry_and_hover_tokens_are_safe_across_themes_and_dpi() {
        let themes = [
            Theme::from_definition(&crate::theme::ThemeDefinition::default_light()),
            Theme::from_definition(&crate::theme::ThemeDefinition::default_dark()),
        ];

        for theme in &themes {
            for dpi in [1.0, 2.0] {
                let width = 72.0 * dpi;
                let height = 36.0 * dpi;
                let mut widget = laid_out_title_bar_at(
                    TitleBarInput {
                        toc_enabled: true,
                        mindmap_style: Some(MindmapStyleButtonInput { panel_visible: false }),
                        ..test_title_bar_input()
                    },
                    width,
                    height,
                    dpi,
                    theme,
                );

                for rect in [widget.toggle_rect, widget.mindmap_style_rect, widget.toc_rect] {
                    assert!(rect.x >= 0.0 && rect.y >= 0.0 && rect.w >= 0.0 && rect.h >= 0.0);
                    assert!(rect.right() <= width && rect.bottom() <= height);
                }

                let toggle = widget.toggle_rect;
                let mut event_ctx = EventCtx::new(theme, dpi);
                widget.on_event(
                    &Event::MouseMove {
                        px: toggle.x + toggle.w * 0.5,
                        py: toggle.y + toggle.h * 0.5,
                    },
                    &mut event_ctx,
                );
                let draw_list = paint_title_bar(&widget, theme, dpi);
                let (hover_color, hover_radius) = draw_list
                    .cmds
                    .iter()
                    .find_map(|command| match command {
                        DrawCmd::FillRect { rect, color, radius } if *rect == toggle => {
                            Some((*color, *radius))
                        }
                        _ => None,
                    })
                    .expect("可见操作按钮应绘制 hover 背景");
                assert_eq!(hover_color, theme.application_theme().hover_surface);
                assert_eq!(
                    hover_radius,
                    theme.control_metrics().compact_corner_radius_logical * dpi
                );
            }
        }
    }

    #[test]
    fn action_buttons_do_not_enter_reserved_hamburger_area() {
        let theme = test_theme();
        let reserved_hamburger_width = 68.0;
        let widget = laid_out_title_bar_at(
            TitleBarInput {
                sidebar_left: reserved_hamburger_width,
                titlebar_x: 0.0,
                toc_enabled: true,
                mindmap_style: Some(MindmapStyleButtonInput { panel_visible: false }),
                ..test_title_bar_input()
            },
            120.0,
            36.0,
            1.0,
            &theme,
        );

        for rect in [widget.toggle_rect, widget.mindmap_style_rect, widget.toc_rect] {
            assert!(
                rect == Rect::ZERO || rect.x >= reserved_hamburger_width,
                "按钮不得进入汉堡保留区：{rect:?}"
            );
        }
    }

    #[test]
    fn mmap_style_button_is_left_of_view_toggle_and_does_not_overlap() {
        let widget = laid_out_title_bar(TitleBarInput {
            mindmap_style: Some(MindmapStyleButtonInput { panel_visible: false }),
            ..test_title_bar_input()
        });

        assert!(widget.mindmap_style_rect_for_test().right() <= widget.toggle_rect_for_test().x);
    }

    #[test]
    fn mmap_style_button_is_absent_when_input_is_none() {
        let widget =
            laid_out_title_bar(TitleBarInput { mindmap_style: None, ..test_title_bar_input() });

        assert_eq!(widget.mindmap_style_rect_for_test(), Rect::ZERO);
    }

    #[test]
    fn active_mmap_style_button_uses_accent_color() {
        let widget = laid_out_title_bar(TitleBarInput {
            mindmap_style: Some(MindmapStyleButtonInput { panel_visible: true }),
            ..test_title_bar_input()
        });
        let theme = test_theme();
        let mut draw_list = DrawList::new();
        let mut paint_ctx = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: None,
        };

        widget.paint(&mut paint_ctx);

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillTriangle { color, .. } if *color == theme.palette.accent
        )));
    }

    #[test]
    fn mmap_style_button_click_emits_toggle_action() {
        let mut widget = laid_out_title_bar(TitleBarInput {
            mindmap_style: Some(MindmapStyleButtonInput { panel_visible: false }),
            ..test_title_bar_input()
        });
        let rect = widget.mindmap_style_rect_for_test();
        let theme = test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(
            widget.on_event(
                &Event::MouseDown {
                    px: rect.x + rect.w * 0.5,
                    py: rect.y + rect.h * 0.5,
                    button: MouseButton::Left,
                },
                &mut event_ctx,
            ),
            Some(WidgetAction::TitleBar(TitleBarAction::ToggleMindmapStylePanel))
        );
    }

    #[test]
    fn paint_without_input_emits_nothing() {
        let w = TitleBarWidget::new();
        let t = test_theme();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        let mut w = w;
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);
        w.paint(&mut pc);
        assert!(dl.cmds.is_empty());
    }

    #[test]
    fn paint_with_file_path_emits_background_text_divider() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/home/user/project/src/main.rs")),
            sidebar_left: 220.0,
            titlebar_x: 220.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut pc);

        // FillRect (bg) + clip + Text (filename) + Text (path) + unclip
        assert_eq!(dl.cmds.len(), 5);
        assert!(matches!(dl.cmds[0], DrawCmd::FillRect { .. }));
        assert!(matches!(dl.cmds[1], DrawCmd::PushClip(..)));
        assert!(matches!(dl.cmds[2], DrawCmd::TextLayout { .. }));
        assert!(matches!(dl.cmds[3], DrawCmd::TextLayout { .. }));
        assert!(matches!(dl.cmds[4], DrawCmd::PopClip));
    }

    #[test]
    fn paint_untitled_has_no_path_text() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: None,
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut pc);

        // FillRect (bg) + clip + Text ("untitled") + unclip — no path
        assert_eq!(dl.cmds.len(), 4);
    }

    #[test]
    fn hit_returns_true_within_rect() {
        let mut w = TitleBarWidget::new();
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        assert!(w.hit(600.0, 18.0));
        assert!(!w.hit(600.0, 40.0));
    }

    #[test]
    fn paint_background_covers_full_width() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 220.0,
            titlebar_x: 220.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut pc);

        // 背景应该覆盖整个宽度（从 0 到 rect.w）
        if let DrawCmd::FillRect { rect, .. } = &dl.cmds[0] {
            assert_eq!(rect.x, 0.0);
            assert_eq!(rect.w, 1200.0);
        } else {
            panic!("第一个命令应该是 FillRect（背景）");
        }
    }

    #[test]
    fn zero_size_rect_paints_nothing() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 0.0, 36.0), &mut lc);

        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut pc);
        assert!(dl.cmds.is_empty());
    }

    // ── title_bar_height 纯函数测试（从旧 title_bar.rs 合并）──

    #[test]
    fn test_title_bar_height() {
        assert!(title_bar_height(1.0) > 0.0);
        assert!(title_bar_height(1.0) > 0.0);
    }

    #[test]
    fn title_bar_height_scales_with_dpi() {
        assert!((title_bar_height(2.0) - 2.0 * title_bar_height(1.0)).abs() < 0.01);
    }

    // ── on_event 测试 ──

    #[test]
    fn on_event_mouse_move_inside_sets_default_cursor() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w.on_event(&Event::MouseMove { px: 600.0, py: 18.0 }, &mut ctx);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), WidgetAction::Consumed));
        assert_eq!(ctx.cursor_hint, Some(CursorIcon::Default));
    }

    #[test]
    fn on_event_mouse_move_outside_returns_none() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w.on_event(&Event::MouseMove { px: 600.0, py: 50.0 }, &mut ctx);
        assert!(result.is_none());
        assert!(ctx.cursor_hint.is_none());
    }

    #[test]
    fn on_event_mouse_down_consumes() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w.on_event(
            &Event::MouseDown { px: 100.0, py: 18.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), WidgetAction::Consumed));
        assert_eq!(ctx.cursor_hint, Some(CursorIcon::Default));
    }

    #[test]
    fn on_event_mouse_up_consumes() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w
            .on_event(&Event::MouseUp { px: 100.0, py: 18.0, button: MouseButton::Left }, &mut ctx);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), WidgetAction::Consumed));
    }

    #[test]
    fn on_event_wheel_consumes() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let result =
            w.on_event(&Event::Wheel { px: 600.0, py: 18.0, dx: 0.0, dy: -120.0 }, &mut ctx);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), WidgetAction::Consumed));
    }

    #[test]
    fn on_event_key_down_returns_none() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w.on_event(&Event::KeyDown(KeyCode::Char('a'), Modifiers::NONE), &mut ctx);
        assert!(result.is_none());
    }

    // ── set_input 测试 ──

    #[test]
    fn set_input_stores_file_path() {
        let mut w = TitleBarWidget::new();
        assert!(w.input.is_none());
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/home/user/test.rs")),
            sidebar_left: 220.0,
            titlebar_x: 220.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        assert!(w.input.is_some());
        assert_eq!(
            w.input.as_ref().unwrap().file_path.as_ref().unwrap().to_str().unwrap(),
            "/home/user/test.rs"
        );
    }

    // ── Toggle button tests ──

    #[test]
    fn toggle_rect_zero_for_non_md() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);
        assert_eq!(w.toggle_rect, Rect::ZERO, "non-md file should have no toggle");
    }

    #[test]
    fn toggle_rect_nonzero_for_md() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);
        assert!(w.toggle_rect.w > 0.0, "md file should have toggle button");
        assert!(w.toggle_rect.x > 0.0, "toggle should be right-aligned");
    }

    #[test]
    fn toggle_rect_is_square() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);
        assert!(w.toggle_rect.w > 0.0);
        assert!(
            (w.toggle_rect.w - w.toggle_rect.h).abs() < 0.01,
            "toggle should be square, got {}x{}",
            w.toggle_rect.w,
            w.toggle_rect.h
        );
    }

    #[test]
    fn toggle_click_emits_action() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let cx = w.toggle_rect.x + w.toggle_rect.w * 0.5;
        let cy = w.toggle_rect.y + w.toggle_rect.h * 0.5;
        let result =
            w.on_event(&Event::MouseDown { px: cx, py: cy, button: MouseButton::Left }, &mut ctx);
        assert!(result.is_some(), "click on toggle should emit action");
        assert_eq!(result.unwrap(), WidgetAction::TitleBar(TitleBarAction::ToggleView));
    }

    #[test]
    fn toggle_right_click_does_not_emit_action() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let cx = w.toggle_rect.x + w.toggle_rect.w * 0.5;
        let cy = w.toggle_rect.y + w.toggle_rect.h * 0.5;
        let result =
            w.on_event(&Event::MouseDown { px: cx, py: cy, button: MouseButton::Right }, &mut ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), WidgetAction::Consumed);
    }

    #[test]
    fn toggle_click_outside_toggle_returns_consumed() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let result = w.on_event(
            &Event::MouseDown { px: 10.0, py: 18.0, button: MouseButton::Left },
            &mut ctx,
        );
        assert!(result.is_some(), "click inside title bar should consume");
        assert_eq!(result.unwrap(), WidgetAction::Consumed);
    }

    #[test]
    fn toggle_hover_sets_pointer_cursor() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut ctx = EventCtx::new(&t, 1.0);
        let cx = w.toggle_rect.x + w.toggle_rect.w * 0.5;
        let cy = w.toggle_rect.y + w.toggle_rect.h * 0.5;
        let result = w.on_event(&Event::MouseMove { px: cx, py: cy }, &mut ctx);
        assert!(result.is_some(), "hover on toggle should consume event");
        assert_eq!(ctx.cursor_hint, Some(CursorIcon::Pointer));
    }

    #[test]
    fn paint_md_preview_emits_eye_icon() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: true,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut pc);

        assert!(dl.cmds.len() > 5, "preview should have eye icon, got {}", dl.cmds.len());
    }

    #[test]
    fn paint_md_edit_emits_eye_icon() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        let mut pc = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        w.paint(&mut pc);

        assert!(dl.cmds.len() > 5, "edit mode should have eye icon, got {}", dl.cmds.len());
    }

    #[test]
    fn eye_icon_color_differs_between_preview_and_edit() {
        let mut w1 = TitleBarWidget::new();
        w1.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: true,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let mut w2 = TitleBarWidget::new();
        w2.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w1.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);
        w2.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let mut dl1 = DrawList::new();
        let mut shaper1 = shaping::Shaper::new().unwrap();
        let mut pc1 = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl1,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper1),
        };
        w1.paint(&mut pc1);

        let mut dl2 = DrawList::new();
        let mut shaper2 = shaping::Shaper::new().unwrap();
        let mut pc2 = PaintCtx {
            global_alpha: 1.0,
            list: &mut dl2,
            theme: &t,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper2),
        };
        w2.paint(&mut pc2);

        assert_eq!(dl1.cmds.len(), dl2.cmds.len());
        let triangles1: Vec<_> =
            dl1.cmds.iter().filter(|c| matches!(c, DrawCmd::FillTriangle { .. })).collect();
        let triangles2: Vec<_> =
            dl2.cmds.iter().filter(|c| matches!(c, DrawCmd::FillTriangle { .. })).collect();
        assert!(!triangles1.is_empty(), "preview should have eye icon triangles");
        assert_eq!(
            triangles1.len(),
            triangles2.len(),
            "both modes should have same triangle count"
        );
    }

    #[test]
    fn tooltip_returns_preview_label_for_edit_mode() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览 ⌘⇧M".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let cx = w.toggle_rect.x + w.toggle_rect.w * 0.5;
        let cy = w.toggle_rect.y + w.toggle_rect.h * 0.5;
        let hint = w.tooltip_at(cx, cy);
        assert!(hint.is_some());
        assert_eq!(hint.unwrap().label, "预览 ⌘⇧M");
    }

    #[test]
    fn tooltip_returns_edit_label_for_preview_mode() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: true,
            toggle_label: Some("编辑 ⌘⇧M".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let cx = w.toggle_rect.x + w.toggle_rect.w * 0.5;
        let cy = w.toggle_rect.y + w.toggle_rect.h * 0.5;
        let hint = w.tooltip_at(cx, cy);
        assert!(hint.is_some());
        assert_eq!(hint.unwrap().label, "编辑 ⌘⇧M");
    }

    #[test]
    fn tooltip_returns_none_outside_toggle() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.md")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: true,
            toggled: false,
            toggle_label: Some("预览".into()),
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let hint = w.tooltip_at(10.0, 18.0);
        assert!(hint.is_none());
    }

    #[test]
    fn tooltip_returns_none_for_non_md() {
        let mut w = TitleBarWidget::new();
        w.set_input(TitleBarInput {
            file_path: Some(PathBuf::from("/test.rs")),
            sidebar_left: 0.0,
            titlebar_x: 0.0,
            can_toggle: false,
            toggled: false,
            toggle_label: None,
            toc_visible: false,
            toc_enabled: false,
            mindmap_style: None,
        });
        let t = test_theme();
        let mut m = NoopMeasure;
        let mut lc = LayoutCtx { ui_measure: None, measure: &mut m, theme: &t, dpi: 1.0 };
        w.set_rect(Rect::new(0.0, 0.0, 1200.0, 36.0), &mut lc);

        let hint = w.tooltip_at(1180.0, 18.0);
        assert!(hint.is_none());
    }
    #[test]
    fn toc_button_exposes_its_own_tooltip() {
        let mut input = test_title_bar_input();
        input.toc_enabled = true;
        let widget = laid_out_title_bar(input);
        let rect = widget.toc_rect;
        let tooltip = widget
            .tooltip_at(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
            .expect("TOC button should describe its own action");
        assert_eq!(tooltip.label, "显示目录 ⌘⇧T");
        assert_eq!(tooltip.target_rect, rect);
    }
}
