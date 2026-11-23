use std::any::Any;

use winit::window::CursorIcon;

use ui::core::geom::Rect;
use ui::core::widget::{
    Event, EventCtx, KeyCode, LayoutCtx, Modifiers, MouseButton, PaintCtx, Widget, WidgetAction,
    WidgetId, ids,
};
use ui::theme::{
    DEFAULT_MINDMAP_COLOR_SCHEME_ID, MindmapThemeSelection, built_in_mindmap_color_schemes,
};
use ui::tooltip::TooltipHint;

pub use ui::core::widget::MindmapStylePanelAction;

pub const PANEL_WIDTH_LOGICAL: f32 = 280.0;
const PANEL_PADDING_LOGICAL: f32 = 12.0;
const COLUMN_GAP_LOGICAL: f32 = 8.0;
const CARD_COLUMN_COUNT: usize = 2;
const CARD_ASPECT_RATIO: f32 = 120.0 / 76.0;
const HEADER_HEIGHT_LOGICAL: f32 = 48.0;
const SECTION_TOP_GAP_LOGICAL: f32 = 16.0;
const SECTION_LABEL_HEIGHT_LOGICAL: f32 = 20.0;
const SUMMARY_TOP_GAP_LOGICAL: f32 = 8.0;
const SUMMARY_HEIGHT_LOGICAL: f32 = 48.0;
const GRID_TOP_GAP_LOGICAL: f32 = 12.0;
const ROW_GAP_LOGICAL: f32 = 8.0;
const NOTICE_TOP_GAP_LOGICAL: f32 = 8.0;
const NOTICE_HEIGHT_LOGICAL: f32 = 20.0;
const CARD_RADIUS_LOGICAL: f32 = 8.0;
const FOCUS_RING_OUTSET_LOGICAL: f32 = 2.0;
const SUMMARY_RADIUS_LOGICAL: f32 = 8.0;
const FONT_SIZE_TITLE_LOGICAL: f32 = 14.0;
const FONT_SIZE_BODY_LOGICAL: f32 = 12.0;
const FONT_SIZE_NOTICE_LOGICAL: f32 = 11.0;
const CLOSE_SIZE_LOGICAL: f32 = 28.0;
const CLOSE_RIGHT_INSET_LOGICAL: f32 = 8.0;

#[derive(Clone, Copy)]
struct CardFocusGeometry {
    outset: f32,
    line_width: f32,
    corner_radius: f32,
}

impl CardFocusGeometry {
    fn for_dpi(dpi: f32) -> Self {
        Self {
            outset: FOCUS_RING_OUTSET_LOGICAL * dpi,
            line_width: dpi.max(1.0),
            corner_radius: (CARD_RADIUS_LOGICAL + FOCUS_RING_OUTSET_LOGICAL) * dpi,
        }
    }

    fn outline_rect(self, card_rect: Rect) -> Rect {
        expand_rect(card_rect, self.outset)
    }

    fn paint_outset(self) -> f32 {
        // One stroke width also reserves the renderer's antialias fringe.
        self.outset + self.line_width
    }

    fn paint_bounds(self, card_rect: Rect) -> Rect {
        expand_rect(card_rect, self.paint_outset())
    }
}

fn expand_rect(rect: Rect, outset: f32) -> Rect {
    Rect::new(rect.x - outset, rect.y - outset, rect.w + 2.0 * outset, rect.h + 2.0 * outset)
}

#[derive(Clone, Debug)]
pub struct MindmapStyleOption {
    pub id: String,
    pub display_name: String,
    pub canvas_background: [f32; 4],
    pub root_fill: [f32; 4],
    pub branch_colors: Vec<[f32; 4]>,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct MindmapStylePanelInput {
    pub selection: MindmapThemeSelection,
    pub options: Vec<MindmapStyleOption>,
    pub presets_expanded: bool,
}

impl MindmapStylePanelInput {
    pub fn from_selection(selection: MindmapThemeSelection, presets_expanded: bool) -> Self {
        let selected_id = match &selection {
            MindmapThemeSelection::Default => Some(DEFAULT_MINDMAP_COLOR_SCHEME_ID),
            MindmapThemeSelection::Selected(id) => Some(id.as_str()),
            MindmapThemeSelection::Unknown(_) | MindmapThemeSelection::InvalidMetadata => None,
        };
        let options = built_in_mindmap_color_schemes()
            .iter()
            .map(|scheme| MindmapStyleOption {
                id: scheme.id.to_owned(),
                display_name: scheme.display_name.to_owned(),
                canvas_background: scheme.canvas.background,
                root_fill: scheme.node.root.fill,
                branch_colors: scheme.canvas.branch_palette.clone(),
                selected: selected_id == Some(scheme.id),
            })
            .collect();

        Self { selection, options, presets_expanded }
    }
}

pub struct MindmapStylePanelWidget {
    input: MindmapStylePanelInput,
    rect: Rect,
    close_rect: Rect,
    summary_rect: Rect,
    notice_rect: Option<Rect>,
    card_rects: Vec<Rect>,
    hovered_option_index: Option<usize>,
    focused_option_index: usize,
    has_keyboard_focus: bool,
    input_initialized: bool,
    scroll_offset: f32,
    scroll_limit: f32,
    grid_clip_top: f32,
    focus_geometry: CardFocusGeometry,
}

impl Default for MindmapStylePanelWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl MindmapStylePanelWidget {
    pub fn new() -> Self {
        let input = MindmapStylePanelInput::from_selection(MindmapThemeSelection::Default, true);
        Self {
            input,
            rect: Rect::ZERO,
            close_rect: Rect::ZERO,
            summary_rect: Rect::ZERO,
            notice_rect: None,
            card_rects: Vec::new(),
            hovered_option_index: None,
            focused_option_index: 0,
            has_keyboard_focus: false,
            input_initialized: false,
            scroll_offset: 0.0,
            scroll_limit: 0.0,
            grid_clip_top: 0.0,
            focus_geometry: CardFocusGeometry::for_dpi(1.0),
        }
    }

    pub fn set_input(&mut self, input: MindmapStylePanelInput) {
        let previous_focused_id =
            self.input.options.get(self.focused_option_index).map(|option| option.id.as_str());
        let selected_index = input.options.iter().position(|option| option.selected);
        let preserved_index = previous_focused_id
            .and_then(|id| input.options.iter().position(|option| option.id == id));
        self.focused_option_index = if self.input_initialized {
            preserved_index.or(selected_index).unwrap_or(0)
        } else {
            selected_index.unwrap_or(0)
        };
        self.input = input;
        self.input_initialized = true;
    }

    fn selection_enabled(&self) -> bool {
        !matches!(self.input.selection, MindmapThemeSelection::InvalidMetadata)
    }

    fn current_option(&self) -> Option<&MindmapStyleOption> {
        self.input.options.iter().find(|option| option.selected).or_else(|| {
            self.input.options.iter().find(|option| option.id == DEFAULT_MINDMAP_COLOR_SCHEME_ID)
        })
    }

    fn notice_text(&self) -> Option<String> {
        match &self.input.selection {
            MindmapThemeSelection::Unknown(id) => Some(format!("找不到主题：{id}，已使用默认主题")),
            MindmapThemeSelection::InvalidMetadata => Some("请先修复文件元数据".to_owned()),
            MindmapThemeSelection::Default | MindmapThemeSelection::Selected(_) => None,
        }
    }

    fn option_at(&self, px: f32, py: f32) -> Option<usize> {
        if py < self.grid_clip_top {
            return None;
        }
        (0..self.card_rects.len()).find(|&index| self.visible_card_rect(index).contains(px, py))
    }

    fn move_focus(&mut self, key_code: KeyCode) {
        if self.input.options.is_empty() {
            return;
        }
        let last_index = self.input.options.len() - 1;
        self.focused_option_index = match key_code {
            KeyCode::Left => self.focused_option_index.saturating_sub(1),
            KeyCode::Right => (self.focused_option_index + 1).min(last_index),
            KeyCode::Up if self.focused_option_index >= CARD_COLUMN_COUNT => {
                self.focused_option_index - CARD_COLUMN_COUNT
            }
            KeyCode::Down
                if self.focused_option_index + CARD_COLUMN_COUNT < self.input.options.len() =>
            {
                self.focused_option_index + CARD_COLUMN_COUNT
            }
            KeyCode::Home => 0,
            KeyCode::End => last_index,
            _ => self.focused_option_index,
        };
        self.ensure_focused_card_visible();
    }

    fn select_focused_option(&self) -> WidgetAction {
        if !self.selection_enabled() {
            return WidgetAction::Consumed;
        }
        let Some(option) = self.input.options.get(self.focused_option_index) else {
            return WidgetAction::Consumed;
        };
        WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(option.id.clone()))
    }

    fn paint_header(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        ctx.text(
            self.rect.x + PANEL_PADDING_LOGICAL * dpi,
            self.rect.y + 30.0 * dpi,
            FONT_SIZE_TITLE_LOGICAL * dpi,
            ctx.theme.palette.text_main,
            "风格",
        );
        ctx.text(
            self.close_rect.x + 8.0 * dpi,
            self.close_rect.y + 19.0 * dpi,
            FONT_SIZE_TITLE_LOGICAL * dpi,
            ctx.theme.palette.text_muted,
            "×",
        );
        let separator_y = self.rect.y + HEADER_HEIGHT_LOGICAL * dpi;
        ctx.list.fill(
            Rect::new(self.rect.x, separator_y, self.rect.w, dpi.max(1.0)),
            ctx.theme.palette.border_subtle,
        );
    }

    fn paint_summary(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        ctx.list.fill_rounded(
            self.summary_rect,
            ctx.theme.palette.bg_elevated,
            SUMMARY_RADIUS_LOGICAL * dpi,
        );
        ctx.list.stroke_rounded(
            self.summary_rect,
            ctx.theme.palette.border_subtle,
            SUMMARY_RADIUS_LOGICAL * dpi,
            dpi.max(1.0),
        );
        if let Some(option) = self.current_option() {
            self.paint_summary_swatch(ctx, option);
            ctx.text(
                self.summary_rect.x + 90.0 * dpi,
                self.summary_rect.y + 30.0 * dpi,
                FONT_SIZE_BODY_LOGICAL * dpi,
                ctx.theme.palette.text_main,
                &option.display_name,
            );
        }
        self.paint_summary_chevron(ctx);
    }

    fn paint_summary_swatch(&self, ctx: &mut PaintCtx, option: &MindmapStyleOption) {
        let dpi = ctx.dpi;
        let colors = option.branch_colors.iter().take(6).copied().collect::<Vec<_>>();
        if colors.is_empty() {
            return;
        }
        let swatch_rect = Rect::new(
            self.summary_rect.x + 10.0 * dpi,
            self.summary_rect.y + 14.0 * dpi,
            70.0 * dpi,
            20.0 * dpi,
        );
        let segment_width = swatch_rect.w / colors.len() as f32;
        for (index, color) in colors.into_iter().enumerate() {
            ctx.list.fill(
                Rect::new(
                    swatch_rect.x + index as f32 * segment_width,
                    swatch_rect.y,
                    segment_width + 0.5,
                    swatch_rect.h,
                ),
                color,
            );
        }
    }

    fn paint_summary_chevron(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        let center_x = self.summary_rect.right() - 18.0 * dpi;
        let center_y = self.summary_rect.y + self.summary_rect.h * 0.5;
        let color = ctx.theme.palette.text_muted;
        if self.input.presets_expanded {
            ctx.list.fill_triangle(
                [center_x - 4.0 * dpi, center_y + 2.0 * dpi],
                [center_x + 4.0 * dpi, center_y + 2.0 * dpi],
                [center_x, center_y - 3.0 * dpi],
                color,
            );
        } else {
            ctx.list.fill_triangle(
                [center_x - 4.0 * dpi, center_y - 2.0 * dpi],
                [center_x + 4.0 * dpi, center_y - 2.0 * dpi],
                [center_x, center_y + 3.0 * dpi],
                color,
            );
        }
    }

    fn paint_card(&self, ctx: &mut PaintCtx, index: usize, option: &MindmapStyleOption) {
        let dpi = ctx.dpi;
        let card_rect = self.visible_card_rect(index);
        ctx.list.fill_rounded(card_rect, option.canvas_background, CARD_RADIUS_LOGICAL * dpi);
        let border_color = if option.selected {
            ctx.theme.palette.accent_text
        } else if self.hovered_option_index == Some(index) {
            ctx.theme.palette.border_strong
        } else {
            ctx.theme.palette.border_subtle
        };
        let line_width = if option.selected { 2.0 * dpi } else { dpi.max(1.0) };
        ctx.list.stroke_rounded(card_rect, border_color, CARD_RADIUS_LOGICAL * dpi, line_width);
        self.paint_miniature_tree(ctx, card_rect, option);
        if option.selected {
            self.paint_checkmark(ctx, card_rect);
        }
        if self.has_keyboard_focus && self.focused_option_index == index {
            ctx.list.stroke_rounded(
                self.focus_geometry.outline_rect(card_rect),
                ctx.theme.palette.focus_ring,
                self.focus_geometry.corner_radius,
                self.focus_geometry.line_width,
            );
        }
    }

    fn paint_miniature_tree(
        &self,
        ctx: &mut PaintCtx,
        card_rect: Rect,
        option: &MindmapStyleOption,
    ) {
        let dpi = ctx.dpi;
        let root = Rect::new(
            card_rect.x + 9.0 * dpi,
            card_rect.y + card_rect.h * 0.5 - 7.0 * dpi,
            35.0 * dpi,
            14.0 * dpi,
        );
        let upper_child =
            Rect::new(root.right() + 20.0 * dpi, root.y - 17.0 * dpi, 27.0 * dpi, 11.0 * dpi);
        let lower_child =
            Rect::new(root.right() + 20.0 * dpi, root.bottom() + 6.0 * dpi, 27.0 * dpi, 11.0 * dpi);
        let upper_leaf = Rect::new(
            upper_child.right() + 9.0 * dpi,
            upper_child.y + 1.5 * dpi,
            16.0 * dpi,
            8.0 * dpi,
        );
        let lower_leaf = Rect::new(
            lower_child.right() + 9.0 * dpi,
            lower_child.y + 1.5 * dpi,
            16.0 * dpi,
            8.0 * dpi,
        );
        self.paint_tree_connectors(
            ctx,
            root,
            upper_child,
            lower_child,
            upper_leaf,
            lower_leaf,
            option,
        );
        ctx.list.fill_rounded(root, option.root_fill, 3.0 * dpi);
        self.paint_branch_node(ctx, upper_child, option, 0);
        self.paint_branch_node(ctx, lower_child, option, 1);
        self.paint_branch_node(ctx, upper_leaf, option, 2);
        self.paint_branch_node(ctx, lower_leaf, option, 3);
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "connector preview receives five precomputed node rectangles from layout"
    )]
    fn paint_tree_connectors(
        &self,
        ctx: &mut PaintCtx,
        root: Rect,
        upper_child: Rect,
        lower_child: Rect,
        upper_leaf: Rect,
        lower_leaf: Rect,
        option: &MindmapStyleOption,
    ) {
        let dpi = ctx.dpi;
        let connector = option.branch_colors.first().copied().unwrap_or(option.root_fill);
        let trunk_x = root.right() + 10.0 * dpi;
        let upper_y = upper_child.y + upper_child.h * 0.5;
        let lower_y = lower_child.y + lower_child.h * 0.5;
        ctx.list.fill(Rect::new(root.right(), root.y + root.h * 0.5, 10.0 * dpi, dpi), connector);
        ctx.list.fill(Rect::new(trunk_x, upper_y, dpi, lower_y - upper_y), connector);
        ctx.list.fill(Rect::new(trunk_x, upper_y, upper_child.x - trunk_x, dpi), connector);
        ctx.list.fill(Rect::new(trunk_x, lower_y, lower_child.x - trunk_x, dpi), connector);
        ctx.list.fill(
            Rect::new(upper_child.right(), upper_y, upper_leaf.x - upper_child.right(), dpi),
            connector,
        );
        ctx.list.fill(
            Rect::new(lower_child.right(), lower_y, lower_leaf.x - lower_child.right(), dpi),
            connector,
        );
    }

    fn paint_branch_node(
        &self,
        ctx: &mut PaintCtx,
        node_rect: Rect,
        option: &MindmapStyleOption,
        branch_index: usize,
    ) {
        let color = option.branch_colors.get(branch_index).copied().unwrap_or(option.root_fill);
        ctx.list.fill_rounded(node_rect, color, 2.0 * ctx.dpi);
    }

    fn paint_checkmark(&self, ctx: &mut PaintCtx, card_rect: Rect) {
        let dpi = ctx.dpi;
        let marker = Rect::new(
            card_rect.right() - 20.0 * dpi,
            card_rect.y + 6.0 * dpi,
            14.0 * dpi,
            14.0 * dpi,
        );
        ctx.list.fill_rounded(marker, ctx.theme.palette.accent, 7.0 * dpi);
        ctx.text(
            marker.x + 3.0 * dpi,
            marker.y + 11.0 * dpi,
            9.0 * dpi,
            ctx.theme.palette.on_accent,
            "✓",
        );
    }

    fn paint_content(&self, ctx: &mut PaintCtx) {
        ctx.list.fill(self.rect, ctx.theme.palette.bg_surface);
        self.paint_header(ctx);
        let dpi = ctx.dpi;
        ctx.text(
            self.rect.x + PANEL_PADDING_LOGICAL * dpi,
            self.rect.y + (HEADER_HEIGHT_LOGICAL + SECTION_TOP_GAP_LOGICAL + 14.0) * dpi,
            FONT_SIZE_BODY_LOGICAL * dpi,
            ctx.theme.palette.text_muted,
            "配色方案",
        );
        self.paint_summary(ctx);
        if let (Some(notice_rect), Some(notice)) = (self.notice_rect, self.notice_text()) {
            ctx.text(
                notice_rect.x,
                notice_rect.y + 14.0 * dpi,
                FONT_SIZE_NOTICE_LOGICAL * dpi,
                ctx.theme.palette.warning_text,
                &notice,
            );
        }
        if self.input.presets_expanded {
            let grid_clip = self.grid_paint_rect();
            ctx.list.clip(grid_clip, |clipped_list| {
                let mut clipped_ctx = PaintCtx {
                    list: clipped_list,
                    theme: ctx.theme,
                    dpi: ctx.dpi,
                    offset: ctx.offset,
                    global_alpha: ctx.global_alpha,
                    shaper: ctx.shaper.as_deref_mut(),
                };
                for (index, option) in self.input.options.iter().enumerate() {
                    if index < self.card_rects.len() {
                        self.paint_card(&mut clipped_ctx, index, option);
                    }
                }
            });
        }
    }

    #[cfg(test)]
    fn card_rects_for_test(&self) -> &[Rect] {
        &self.card_rects
    }

    #[cfg(test)]
    fn scroll_offset_for_test(&self) -> f32 {
        self.scroll_offset
    }

    #[cfg(test)]
    fn grid_clip_top_for_test(&self) -> f32 {
        self.grid_clip_top
    }

    #[cfg(test)]
    fn visible_card_rect_for_test(&self, index: usize) -> Rect {
        self.visible_card_rect(index)
    }

    #[cfg(test)]
    fn selected_option_for_test(&self) -> Option<&MindmapStyleOption> {
        self.input.options.iter().find(|option| option.selected)
    }

    #[cfg(test)]
    fn option_index_for_test(&self, id: &str) -> Option<usize> {
        self.input.options.iter().position(|option| option.id == id)
    }
}

impl Widget for MindmapStylePanelWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        let dpi = ctx.dpi;
        self.focus_geometry = CardFocusGeometry::for_dpi(dpi);
        self.rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        let close_size = CLOSE_SIZE_LOGICAL * dpi;
        self.close_rect = Rect::new(
            (self.rect.w - CLOSE_RIGHT_INSET_LOGICAL * dpi - close_size).max(0.0),
            (HEADER_HEIGHT_LOGICAL * dpi - close_size) * 0.5,
            close_size,
            close_size,
        );
        let content_width = (self.rect.w - 2.0 * PANEL_PADDING_LOGICAL * dpi).max(0.0);
        let label_y = (HEADER_HEIGHT_LOGICAL + SECTION_TOP_GAP_LOGICAL) * dpi;
        let summary_y = label_y + (SECTION_LABEL_HEIGHT_LOGICAL + SUMMARY_TOP_GAP_LOGICAL) * dpi;
        self.summary_rect = Rect::new(
            PANEL_PADDING_LOGICAL * dpi,
            summary_y,
            content_width,
            SUMMARY_HEIGHT_LOGICAL * dpi,
        );
        self.layout_notice_and_cards(dpi, content_width);
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let theme = ctx.theme;
        let dpi = ctx.dpi;
        let offset = ctx.offset;
        let global_alpha = ctx.global_alpha;
        let mut shaper = ctx.shaper.take();
        ctx.list.clip(self.rect, |clipped_list| {
            let mut clipped_ctx = PaintCtx {
                list: clipped_list,
                theme,
                dpi,
                offset,
                global_alpha,
                shaper: shaper.as_deref_mut(),
            };
            self.paint_content(&mut clipped_ctx);
        });
        ctx.shaper = shaper;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn id(&self) -> Option<WidgetId> {
        Some(ids::MINDMAP_STYLE_PANEL)
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.has_keyboard_focus = focused_id == self.id();
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match event {
            Event::MouseMove { px, py } => self.handle_mouse_move(*px, *py, ctx),
            Event::MouseDown { px, py, button: MouseButton::Left } => {
                self.handle_left_click(*px, *py)
            }
            Event::Wheel { dy, px, py, .. } if self.rect.contains(*px, *py) => {
                self.scroll_by(-dy);
                Some(WidgetAction::Consumed)
            }
            Event::KeyDown(key_code, modifiers) => self.handle_key_down(*key_code, *modifiers),
            _ => None,
        }
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        let option_index = self.option_at(px, py)?;
        let option = self.input.options.get(option_index)?;
        Some(TooltipHint {
            label: option.display_name.clone(),
            target_rect: self.visible_card_rect(option_index),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl MindmapStylePanelWidget {
    fn layout_notice_and_cards(&mut self, dpi: f32, content_width: f32) {
        let mut grid_y = self.summary_rect.bottom() + GRID_TOP_GAP_LOGICAL * dpi;
        self.notice_rect = self.notice_text().map(|_| {
            let rect = Rect::new(
                PANEL_PADDING_LOGICAL * dpi,
                self.summary_rect.bottom() + NOTICE_TOP_GAP_LOGICAL * dpi,
                content_width,
                NOTICE_HEIGHT_LOGICAL * dpi,
            );
            grid_y = rect.bottom() + GRID_TOP_GAP_LOGICAL * dpi;
            rect
        });
        self.grid_clip_top = grid_y;
        self.card_rects.clear();
        if !self.input.presets_expanded {
            self.scroll_offset = 0.0;
            self.scroll_limit = 0.0;
            return;
        }
        let column_gap = COLUMN_GAP_LOGICAL * dpi;
        let card_width = ((content_width - column_gap) / 2.0).max(0.0);
        let card_height = card_width / CARD_ASPECT_RATIO;
        for index in 0..self.input.options.len() {
            let column = index % CARD_COLUMN_COUNT;
            let row = index / CARD_COLUMN_COUNT;
            let x = PANEL_PADDING_LOGICAL * dpi + column as f32 * (card_width + column_gap);
            let y = grid_y + row as f32 * (card_height + ROW_GAP_LOGICAL * dpi);
            self.card_rects.push(Rect::new(x, y, card_width, card_height));
        }
        let content_bottom = self.card_rects.last().map(|rect| rect.bottom()).unwrap_or(grid_y);
        self.scroll_limit =
            (content_bottom + PANEL_PADDING_LOGICAL * dpi - self.rect.bottom()).max(0.0);
        self.scroll_offset = self.scroll_offset.clamp(0.0, self.scroll_limit);
    }

    /// 卡片在屏幕上滚动的可视位置；布局矩形始终是未滚动的基准位置。
    fn visible_card_rect(&self, index: usize) -> Rect {
        let rect = self.card_rects[index];
        Rect::new(rect.x, rect.y - self.scroll_offset, rect.w, rect.h)
    }

    fn scroll_by(&mut self, delta: f32) {
        if self.scroll_limit <= 0.0 {
            return;
        }
        self.scroll_offset = (self.scroll_offset + delta).clamp(0.0, self.scroll_limit);
    }

    fn grid_paint_rect(&self) -> Rect {
        let top = self.grid_clip_top - self.focus_geometry.paint_outset();
        Rect::new(self.rect.x, top, self.rect.w, (self.rect.bottom() - top).max(0.0))
    }

    /// 键盘移动焦点时让完整焦点绘制范围可见；空间不足时固定顶部对齐。
    fn ensure_focused_card_visible(&mut self) {
        let Some(card_rect) = self.card_rects.get(self.focused_option_index).copied() else {
            return;
        };
        let bounds = self.focus_geometry.paint_bounds(card_rect);
        let viewport = self.grid_paint_rect();
        let visible_top = bounds.y - self.scroll_offset;
        if bounds.h > viewport.h || visible_top < viewport.y {
            self.scroll_offset = bounds.y - viewport.y;
        } else if visible_top + bounds.h > viewport.bottom() {
            self.scroll_offset = bounds.bottom() - viewport.bottom();
        }
        self.scroll_offset = self.scroll_offset.clamp(0.0, self.scroll_limit);
    }

    fn handle_mouse_move(&mut self, px: f32, py: f32, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if !self.hit(px, py) {
            if self.hovered_option_index.take().is_some() {
                return Some(WidgetAction::Consumed);
            }
            return None;
        }
        self.hovered_option_index = self.option_at(px, py);
        let interactive = self.close_rect.contains(px, py)
            || self.summary_rect.contains(px, py)
            || (self.hovered_option_index.is_some() && self.selection_enabled());
        ctx.cursor_hint = Some(if interactive { CursorIcon::Pointer } else { CursorIcon::Default });
        Some(WidgetAction::Consumed)
    }

    fn handle_left_click(&self, px: f32, py: f32) -> Option<WidgetAction> {
        if self.close_rect.contains(px, py) {
            return Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::Close));
        }
        if self.summary_rect.contains(px, py) {
            return Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::TogglePresets));
        }
        if let Some(option_index) = self.option_at(px, py) {
            if !self.selection_enabled() {
                return Some(WidgetAction::Consumed);
            }
            let theme_id = self.input.options[option_index].id.clone();
            return Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(
                theme_id,
            )));
        }
        self.hit(px, py).then_some(WidgetAction::Consumed)
    }

    fn handle_key_down(
        &mut self,
        key_code: KeyCode,
        _modifiers: Modifiers,
    ) -> Option<WidgetAction> {
        match key_code {
            KeyCode::Escape => {
                Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::Close))
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End => {
                self.move_focus(key_code);
                Some(WidgetAction::Consumed)
            }
            KeyCode::Enter | KeyCode::Char(' ') => Some(self.select_focused_option()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::core::{
        DrawCmd, DrawList, Event, EventCtx, KeyCode, LayoutCtx, Modifiers, MouseButton,
        NoopMeasure, PaintCtx, Rect, Widget, WidgetAction,
    };
    use ui::theme::MindmapThemeSelection;

    fn laid_out_panel(selection: MindmapThemeSelection) -> MindmapStylePanelWidget {
        let mut widget = MindmapStylePanelWidget::new();
        widget.set_input(MindmapStylePanelInput::from_selection(selection, true));
        let theme = ui::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        widget.set_rect(Rect::new(0.0, 0.0, PANEL_WIDTH_LOGICAL, 600.0), &mut layout);
        widget
    }

    fn paint_with_text(widget: &MindmapStylePanelWidget, theme: &ui::Theme) -> DrawList {
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("测试环境必须能创建文字塑形器");
        let mut context = PaintCtx::new(&mut draw_list, theme, 1.0);
        context.shaper = Some(&mut shaper);
        widget.paint(&mut context);
        draw_list
    }

    #[test]
    fn sixteen_scheme_cards_layout_as_two_columns_and_eight_rows() {
        let widget = laid_out_panel(MindmapThemeSelection::Default);
        let cards = widget.card_rects_for_test();
        assert_eq!(cards.len(), 16);
        for row in 0..8 {
            assert_eq!(cards[row * 2].y, cards[row * 2 + 1].y);
        }
        for row in 0..7 {
            assert_eq!(cards[row * 2].x, cards[row * 2 + 2].x);
            assert_eq!(cards[row * 2 + 1].x, cards[row * 2 + 3].x);
        }
        assert!(cards.iter().all(|card| card.right() <= PANEL_WIDTH_LOGICAL));
    }

    #[test]
    fn selected_card_paints_selection_border_and_checkmark() {
        let widget = laid_out_panel(MindmapThemeSelection::Selected("tide".into()));
        assert_eq!(
            widget.selected_option_for_test().map(|option| option.id.as_str()),
            Some("tide")
        );
        let theme = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_light());
        let draw_list = paint_with_text(&widget, &theme);
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::StrokeRect { line_width, color, .. }
                if *line_width == 2.0 && *color == theme.palette.accent_text
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, color, .. }
                if layout.text == "✓" && *color == theme.palette.on_accent
        )));
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::FillRect { color, radius, .. }
                if *radius == 7.0 && *color == theme.palette.accent
        )));
    }

    fn painted_background_at(commands: &[DrawCmd], sample: (f32, f32)) -> [f32; 4] {
        commands
            .iter()
            .rev()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, color, .. } if rect.contains(sample.0, sample.1) => {
                    Some(*color)
                }
                _ => None,
            })
            .expect("focus outline must have a painted background")
    }

    fn runtime_linear_contrast(foreground: [f32; 4], background: [f32; 4]) -> f32 {
        let luminance = |color: [f32; 4]| 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
        let foreground_luminance = luminance(foreground);
        let background_luminance = luminance(background);
        (foreground_luminance.max(background_luminance) + 0.05)
            / (foreground_luminance.min(background_luminance) + 0.05)
    }

    fn assert_focus_on_controlled_surface(theme: &ui::Theme, scheme_id: &str, dpi: f32) {
        let mut widget = laid_out_panel(MindmapThemeSelection::Selected(scheme_id.to_owned()));
        let mut measure = NoopMeasure;
        widget.set_rect(
            Rect::new(0.0, 0.0, PANEL_WIDTH_LOGICAL * dpi, 600.0 * dpi),
            &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi },
        );
        widget.set_keyboard_focus(widget.id());
        let focused_card = widget.visible_card_rect(widget.focused_option_index);
        let mut draw_list = DrawList::new();
        widget.paint(&mut PaintCtx::new(&mut draw_list, theme, dpi));
        let (command_index, focus_rect, focus_color, radius, line_width) = draw_list
            .cmds
            .iter()
            .enumerate()
            .find_map(|(index, command)| match command {
                DrawCmd::StrokeRect { rect, color, radius, line_width }
                    if *color == theme.palette.focus_ring =>
                {
                    Some((index, *rect, *color, *radius, *line_width))
                }
                _ => None,
            })
            .expect("focused card must paint a focus outline");
        let sample = (focus_rect.x + focus_rect.w * 0.5, focus_rect.y + line_width * 0.5);
        let background = painted_background_at(&draw_list.cmds[..command_index], sample);
        let contrast = runtime_linear_contrast(focus_color, background);
        assert!(
            contrast >= 3.0,
            "runtime linear focus contrast for {scheme_id}, dark={}: {contrast}",
            theme.is_dark
        );
        assert_eq!(background, theme.palette.bg_surface);
        assert!(focus_rect.x + line_width < focused_card.x);
        assert!(focus_rect.y + line_width < focused_card.y);
        assert!(focus_rect.right() - line_width > focused_card.right());
        assert!(focus_rect.bottom() - line_width > focused_card.bottom());
        assert_eq!(line_width, dpi.max(1.0));
        assert_eq!(radius, (CARD_RADIUS_LOGICAL + FOCUS_RING_OUTSET_LOGICAL) * dpi);
    }

    #[test]
    fn keyboard_focus_stays_on_controlled_surface_across_appearances() {
        for definition in [
            ui::theme::ThemeDefinition::default_dark(),
            ui::theme::ThemeDefinition::default_light(),
        ] {
            let theme = ui::Theme::from_definition(&definition);
            for scheme_id in ["paper", "tide"] {
                for dpi in [1.0, 2.0] {
                    assert_focus_on_controlled_surface(&theme, scheme_id, dpi);
                }
            }
        }
    }

    fn focus_clips(draw_list: &DrawList, theme: &ui::Theme) -> Vec<Rect> {
        let mut clips = Vec::new();
        for command in &draw_list.cmds {
            match command {
                DrawCmd::PushClip(rect) => clips.push(*rect),
                DrawCmd::PopClip => {
                    clips.pop();
                }
                DrawCmd::StrokeRect { color, .. } if *color == theme.palette.focus_ring => {
                    return clips;
                }
                _ => {}
            }
        }
        panic!("focus outline must be painted");
    }

    fn focus_paint_bounds(draw_list: &DrawList, theme: &ui::Theme) -> Rect {
        draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::StrokeRect { rect, color, line_width, .. }
                    if *color == theme.palette.focus_ring =>
                {
                    Some(Rect::new(
                        rect.x - line_width,
                        rect.y - line_width,
                        rect.w + 2.0 * line_width,
                        rect.h + 2.0 * line_width,
                    ))
                }
                _ => None,
            })
            .expect("focused card must paint its outline")
    }

    fn laid_out_focus_panel(theme: &ui::Theme, dpi: f32, height: f32) -> MindmapStylePanelWidget {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let mut measure = NoopMeasure;
        widget.set_rect(
            Rect::new(0.0, 0.0, PANEL_WIDTH_LOGICAL * dpi, height * dpi),
            &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi },
        );
        widget.set_keyboard_focus(widget.id());
        widget
    }

    #[test]
    fn keyboard_home_end_keep_complete_focus_bounds_visible_when_card_fits() {
        let theme = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        for dpi in [1.0, 2.0] {
            let mut widget = laid_out_focus_panel(&theme, dpi, 300.0);
            for key in [KeyCode::End, KeyCode::Home] {
                widget.on_event(
                    &Event::KeyDown(key, Modifiers::NONE),
                    &mut EventCtx::new(&theme, dpi),
                );
                let mut draw_list = DrawList::new();
                widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, dpi));
                let bounds = focus_paint_bounds(&draw_list, &theme);
                for clip in focus_clips(&draw_list, &theme) {
                    assert!(bounds.x >= clip.x && bounds.right() <= clip.right());
                    assert!(
                        bounds.y >= clip.y && bounds.bottom() <= clip.bottom(),
                        "{key:?} at dpi={dpi}: focus {bounds:?} exceeds clip {clip:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn first_row_focus_outline_has_clip_space_without_entering_header() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        widget.set_keyboard_focus(widget.id());
        let theme = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let mut draw_list = DrawList::new();
        widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));
        let clips = focus_clips(&draw_list, &theme);
        let grid_clip = clips.last().expect("focus must be clipped to the card grid");
        let bounds = focus_paint_bounds(&draw_list, &theme);
        assert!(bounds.y >= grid_clip.y && bounds.bottom() <= grid_clip.bottom());
        assert!(grid_clip.y > widget.summary_rect.bottom());
        assert_eq!(clips.first(), Some(&widget.rect));
    }

    #[test]
    fn scrolled_focus_remains_clipped_to_short_panel_and_below_summary() {
        let theme = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        for dpi in [1.0, 2.0] {
            let mut widget = laid_out_focus_panel(&theme, dpi, 220.0);
            for key in [KeyCode::End, KeyCode::End, KeyCode::Home] {
                widget.on_event(
                    &Event::KeyDown(key, Modifiers::NONE),
                    &mut EventCtx::new(&theme, dpi),
                );
                let previous_offset = widget.scroll_offset;
                let mut draw_list = DrawList::new();
                widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, dpi));
                let clips = focus_clips(&draw_list, &theme);
                let grid_clip =
                    clips.last().expect("scrolled focus must be clipped to the card grid");
                let bounds = focus_paint_bounds(&draw_list, &theme);
                assert_eq!(bounds.y, grid_clip.y);
                assert!(
                    bounds.bottom() > grid_clip.bottom(),
                    "short grid cannot fit the complete focus bounds"
                );
                assert!(grid_clip.y > widget.summary_rect.bottom());
                assert_eq!(grid_clip.bottom(), widget.rect.bottom());
                assert_eq!(clips.first(), Some(&widget.rect));
                widget.on_event(
                    &Event::KeyDown(key, Modifiers::NONE),
                    &mut EventCtx::new(&theme, dpi),
                );
                assert_eq!(
                    widget.scroll_offset, previous_offset,
                    "repeating a key must not oscillate scrolling"
                );
            }
        }
    }

    #[test]
    fn metadata_warning_uses_warning_body_color() {
        let widget = laid_out_panel(MindmapThemeSelection::InvalidMetadata);
        let theme = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_light());
        let draw_list = paint_with_text(&widget, &theme);

        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, color, .. }
                if layout.text == "请先修复文件元数据" && *color == theme.palette.warning_text
        )));
    }

    #[test]
    fn invalid_metadata_disables_card_selection() {
        let mut widget = laid_out_panel(MindmapThemeSelection::InvalidMetadata);
        let card = widget.card_rects_for_test()[0];
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);
        let action = widget.on_event(
            &Event::MouseDown {
                px: card.x + card.w * 0.5,
                py: card.y + card.h * 0.5,
                button: MouseButton::Left,
            },
            &mut event_ctx,
        );
        assert_eq!(action, Some(WidgetAction::Consumed));
    }

    #[test]
    fn card_click_emits_stable_theme_id() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let tide_index = widget.option_index_for_test("tide").expect("tide card");
        let card = widget.card_rects_for_test()[tide_index];
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);
        assert_eq!(
            widget.on_event(
                &Event::MouseDown {
                    px: card.x + card.w * 0.5,
                    py: card.y + card.h * 0.5,
                    button: MouseButton::Left,
                },
                &mut event_ctx,
            ),
            Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(
                "tide".into()
            )))
        );
    }

    #[test]
    fn arrow_keys_move_focus_and_enter_selects() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);
        widget.on_event(&Event::KeyDown(KeyCode::Right, Modifiers::NONE), &mut event_ctx);
        widget.on_event(&Event::KeyDown(KeyCode::Down, Modifiers::NONE), &mut event_ctx);
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx),
            Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(
                "meadow".into()
            )))
        );
    }

    #[test]
    fn vertical_arrow_keys_stay_in_the_same_column_at_grid_edges() {
        let boundary_cases = [
            ("paper", KeyCode::Up),
            ("dawn", KeyCode::Up),
            ("violet-night", KeyCode::Down),
            ("basalt-night", KeyCode::Down),
        ];
        let theme = ui::theme::test_theme();

        for (theme_id, key_code) in boundary_cases {
            let mut widget = laid_out_panel(MindmapThemeSelection::Selected(theme_id.to_owned()));
            let mut event_ctx = EventCtx::new(&theme, 1.0);
            widget.on_event(&Event::KeyDown(key_code, Modifiers::NONE), &mut event_ctx);

            assert_eq!(
                widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx,),
                Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(
                    theme_id.to_owned()
                ))),
                "vertical navigation crossed a column at {theme_id}"
            );
        }
    }

    #[test]
    fn paint_clips_overflowing_cards_to_the_panel_rect() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let theme = ui::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let panel_rect = Rect::new(0.0, 0.0, PANEL_WIDTH_LOGICAL, 220.0);
        widget.set_rect(panel_rect, &mut layout);
        let mut draw_list = DrawList::new();

        widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));

        assert!(matches!(
            draw_list.cmds.first(),
            Some(DrawCmd::PushClip(rect)) if *rect == panel_rect
        ));
        assert!(matches!(draw_list.cmds.last(), Some(DrawCmd::PopClip)));
    }

    #[test]
    fn zero_sized_panel_emits_no_draw_commands() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let theme = ui::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        widget.set_rect(Rect::ZERO, &mut layout);
        let mut draw_list = DrawList::new();

        widget.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));

        assert!(draw_list.cmds.is_empty());
    }

    #[test]
    fn escape_emits_close_action() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut event_ctx),
            Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::Close))
        );
    }

    #[test]
    fn input_marks_default_or_selected_scheme_but_not_fallback_states() {
        let default_input =
            MindmapStylePanelInput::from_selection(MindmapThemeSelection::Default, true);
        let selected_input = MindmapStylePanelInput::from_selection(
            MindmapThemeSelection::Selected("iris".into()),
            true,
        );
        let unknown_input = MindmapStylePanelInput::from_selection(
            MindmapThemeSelection::Unknown("future-theme".into()),
            true,
        );
        let invalid_input =
            MindmapStylePanelInput::from_selection(MindmapThemeSelection::InvalidMetadata, true);

        assert_eq!(
            default_input
                .options
                .iter()
                .filter(|option| option.selected)
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            ["paper"]
        );
        assert_eq!(
            selected_input
                .options
                .iter()
                .filter(|option| option.selected)
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            ["iris"]
        );
        assert!(!unknown_input.options.iter().any(|option| option.selected));
        assert!(!invalid_input.options.iter().any(|option| option.selected));
    }

    #[test]
    fn first_input_places_keyboard_focus_on_selected_scheme() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Selected("tide".into()));
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut event_ctx),
            Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(
                "tide".into()
            )))
        );
    }

    #[test]
    fn summary_click_toggles_presets_and_cards_expose_tooltips() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let summary = widget.summary_rect;
        let first_card = widget.card_rects_for_test()[0];
        let hint = widget
            .tooltip_at(first_card.x + first_card.w * 0.5, first_card.y + first_card.h * 0.5)
            .expect("scheme card must expose its display name");
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(hint.label, "素纸");
        assert_eq!(
            widget.on_event(
                &Event::MouseDown {
                    px: summary.x + summary.w * 0.5,
                    py: summary.y + summary.h * 0.5,
                    button: MouseButton::Left,
                },
                &mut event_ctx,
            ),
            Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::TogglePresets))
        );
    }

    #[test]
    fn hover_consumes_without_selecting_and_sets_pointer_cursor() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let tide_index = widget.option_index_for_test("tide").expect("tide card");
        let tide_card = widget.card_rects_for_test()[tide_index];
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);

        assert_eq!(
            widget.on_event(
                &Event::MouseMove {
                    px: tide_card.x + tide_card.w * 0.5,
                    py: tide_card.y + tide_card.h * 0.5,
                },
                &mut event_ctx,
            ),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(event_ctx.cursor_hint, Some(CursorIcon::Pointer));
        assert_eq!(
            widget.selected_option_for_test().map(|option| option.id.as_str()),
            Some("paper")
        );
    }

    #[test]
    fn wheel_scrolls_the_card_grid_within_limits() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);

        let wheel_down = Event::Wheel { dx: 0.0, dy: -120.0, px: 10.0, py: 400.0 };
        assert_eq!(widget.on_event(&wheel_down, &mut event_ctx), Some(WidgetAction::Consumed));
        assert!(widget.scroll_offset_for_test() > 0.0);

        let wheel_up_beyond_top = Event::Wheel { dx: 0.0, dy: 10_000.0, px: 10.0, py: 400.0 };
        widget.on_event(&wheel_up_beyond_top, &mut event_ctx);
        assert_eq!(widget.scroll_offset_for_test(), 0.0);
    }

    #[test]
    fn end_key_scrolls_the_last_card_into_view() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);

        widget.on_event(&Event::KeyDown(KeyCode::End, Modifiers::NONE), &mut event_ctx);

        assert!(widget.scroll_offset_for_test() > 0.0);
        let last_visible = widget.visible_card_rect_for_test(15);
        assert!(last_visible.y >= widget.grid_clip_top_for_test());
        assert!(last_visible.bottom() <= 600.0 + f32::EPSILON);
    }

    #[test]
    fn scrolled_away_card_is_not_hit_at_its_layout_position() {
        let mut widget = laid_out_panel(MindmapThemeSelection::Default);
        let theme = ui::theme::test_theme();
        let mut event_ctx = EventCtx::new(&theme, 1.0);
        widget.on_event(&Event::KeyDown(KeyCode::End, Modifiers::NONE), &mut event_ctx);

        let first_card = widget.card_rects_for_test()[0];
        let action = widget.on_event(
            &Event::MouseDown {
                px: first_card.x + first_card.w * 0.5,
                py: widget.grid_clip_top_for_test() + 1.0,
                button: MouseButton::Left,
            },
            &mut event_ctx,
        );

        assert_ne!(
            action,
            Some(WidgetAction::MindmapStylePanel(MindmapStylePanelAction::SelectTheme(
                "paper".into()
            )))
        );
    }

    #[test]
    fn panel_has_stable_focusable_widget_id() {
        let widget = laid_out_panel(MindmapThemeSelection::Default);

        assert_eq!(widget.id(), Some(ids::MINDMAP_STYLE_PANEL));
        assert!(widget.is_focusable());
    }
}
