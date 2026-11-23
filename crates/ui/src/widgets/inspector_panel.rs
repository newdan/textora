use std::any::Any;

use winit::window::CursorIcon;

use crate::core::geom::Rect;
use crate::core::widget::{
    Event, EventCtx, KeyCode, LayoutCtx, Modifiers, MouseButton, PaintCtx, Widget, WidgetAction,
    WidgetId, ids,
};
use crate::tooltip::TooltipHint;

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PanelId(pub String);

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct GroupId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(pub String);

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorPresetItem {
    pub id: ItemId,
    pub display_name: String,
    pub canvas_background: [f32; 4],
    pub root_fill: [f32; 4],
    pub branch_colors: Vec<[f32; 4]>,
    pub disabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorPresetGroup {
    pub id: GroupId,
    pub title: String,
    pub items: Vec<InspectorPresetItem>,
    pub selected_item: Option<ItemId>,
    pub fallback_summary_item: Option<ItemId>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorPanelDescriptor {
    pub id: PanelId,
    pub title: String,
    pub presets: InspectorPresetGroup,
    pub notice: Option<String>,
    pub selection_disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectorPanelPresentation {
    pub presets_expanded: bool,
    pub focused_item: Option<ItemId>,
}

impl Default for InspectorPanelPresentation {
    fn default() -> Self {
        Self { presets_expanded: true, focused_item: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectorItemTarget {
    pub group_id: GroupId,
    pub item_id: ItemId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectorPanelIntent {
    Close,
    ToggleGroup(GroupId),
    FocusItem(InspectorItemTarget),
    SelectItem(InspectorItemTarget),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectorPanelAction {
    pub panel_id: PanelId,
    pub intent: InspectorPanelIntent,
}

pub struct InspectorPanelWidget {
    descriptor: InspectorPanelDescriptor,
    presets_expanded: bool,
    rect: Rect,
    close_rect: Rect,
    summary_rect: Rect,
    notice_rect: Option<Rect>,
    card_rects: Vec<Rect>,
    hovered_option_index: Option<usize>,
    focused_option_index: usize,
    has_keyboard_focus: bool,
    scroll_offset: f32,
    scroll_limit: f32,
    grid_clip_top: f32,
    layout_dpi: f32,
    focus_geometry: CardFocusGeometry,
}

impl Default for InspectorPanelWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl InspectorPanelWidget {
    pub fn new() -> Self {
        Self {
            descriptor: InspectorPanelDescriptor::default(),
            presets_expanded: true,
            rect: Rect::ZERO,
            close_rect: Rect::ZERO,
            summary_rect: Rect::ZERO,
            notice_rect: None,
            card_rects: Vec::new(),
            hovered_option_index: None,
            focused_option_index: 0,
            has_keyboard_focus: false,
            scroll_offset: 0.0,
            scroll_limit: 0.0,
            grid_clip_top: 0.0,
            layout_dpi: 1.0,
            focus_geometry: CardFocusGeometry::for_dpi(1.0),
        }
    }

    pub fn set_input(
        &mut self,
        descriptor: InspectorPanelDescriptor,
        presentation: &InspectorPanelPresentation,
    ) {
        let group = &descriptor.presets;
        let find_item = |item_id: &ItemId| group.items.iter().position(|item| &item.id == item_id);
        let previous_focus = self.focused_option_index;
        self.focused_option_index = presentation
            .focused_item
            .as_ref()
            .and_then(find_item)
            .or_else(|| group.selected_item.as_ref().and_then(find_item))
            .unwrap_or(0);
        if self.descriptor.id != descriptor.id || self.descriptor.presets.id != group.id {
            self.scroll_offset = 0.0;
        }
        let layout_changed =
            self.descriptor != descriptor || self.presets_expanded != presentation.presets_expanded;
        self.descriptor = descriptor;
        self.presets_expanded = presentation.presets_expanded;
        if layout_changed {
            self.hovered_option_index = None;
            self.layout_content(self.layout_dpi);
        }
        if layout_changed || previous_focus != self.focused_option_index {
            self.ensure_focused_card_visible();
        }
    }

    fn current_option(&self) -> Option<&InspectorPresetItem> {
        let group = &self.descriptor.presets;
        let item_id = group.selected_item.as_ref().or(group.fallback_summary_item.as_ref())?;
        group.items.iter().find(|item| &item.id == item_id)
    }

    fn notice_text(&self) -> Option<&str> {
        self.descriptor.notice.as_deref()
    }

    fn action(&self, intent: InspectorPanelIntent) -> WidgetAction {
        WidgetAction::InspectorPanel(InspectorPanelAction {
            panel_id: self.descriptor.id.clone(),
            intent,
        })
    }

    fn item_target(&self, index: usize) -> Option<InspectorItemTarget> {
        let group = &self.descriptor.presets;
        let item = group.items.get(index)?;
        Some(InspectorItemTarget { group_id: group.id.clone(), item_id: item.id.clone() })
    }

    fn selection_enabled(&self, index: usize) -> bool {
        !self.descriptor.selection_disabled
            && self.descriptor.presets.items.get(index).is_some_and(|item| !item.disabled)
    }

    fn select_option(&self, index: usize) -> WidgetAction {
        if self.presets_expanded
            && self.selection_enabled(index)
            && let Some(target) = self.item_target(index)
        {
            return self.action(InspectorPanelIntent::SelectItem(target));
        }
        WidgetAction::Consumed
    }

    fn option_at(&self, px: f32, py: f32) -> Option<usize> {
        if !self.presets_expanded || !self.rect.contains(px, py) || py < self.grid_clip_top {
            return None;
        }
        (0..self.card_rects.len()).find(|&index| self.visible_card_rect(index).contains(px, py))
    }

    fn move_focus(&mut self, key_code: KeyCode) {
        if self.descriptor.presets.items.is_empty() {
            return;
        }
        let last_index = self.descriptor.presets.items.len() - 1;
        self.focused_option_index = match key_code {
            KeyCode::Left => self.focused_option_index.saturating_sub(1),
            KeyCode::Right => (self.focused_option_index + 1).min(last_index),
            KeyCode::Up if self.focused_option_index >= CARD_COLUMN_COUNT => {
                self.focused_option_index - CARD_COLUMN_COUNT
            }
            KeyCode::Down
                if self.focused_option_index + CARD_COLUMN_COUNT
                    < self.descriptor.presets.items.len() =>
            {
                self.focused_option_index + CARD_COLUMN_COUNT
            }
            KeyCode::Home => 0,
            KeyCode::End => last_index,
            _ => self.focused_option_index,
        };
        self.ensure_focused_card_visible();
    }

    fn paint_header(&self, ctx: &mut PaintCtx) {
        let dpi = ctx.dpi;
        ctx.text(
            self.rect.x + PANEL_PADDING_LOGICAL * dpi,
            self.rect.y + 30.0 * dpi,
            FONT_SIZE_TITLE_LOGICAL * dpi,
            ctx.theme.palette.text_main,
            &self.descriptor.title,
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

    fn paint_summary_swatch(&self, ctx: &mut PaintCtx, option: &InspectorPresetItem) {
        let dpi = ctx.dpi;
        const SUMMARY_COLOR_COUNT: usize = 6;
        let colors = &option.branch_colors[..option.branch_colors.len().min(SUMMARY_COLOR_COUNT)];
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
        for (index, &color) in colors.iter().enumerate() {
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
        if self.presets_expanded {
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

    fn paint_card(&self, ctx: &mut PaintCtx, index: usize, option: &InspectorPresetItem) {
        let dpi = ctx.dpi;
        let card_rect = self.visible_card_rect(index);
        ctx.list.fill_rounded(card_rect, option.canvas_background, CARD_RADIUS_LOGICAL * dpi);
        let selected = self.descriptor.presets.selected_item.as_ref() == Some(&option.id);
        let border_color = if selected {
            ctx.theme.palette.accent_text
        } else if self.hovered_option_index == Some(index) {
            ctx.theme.palette.border_strong
        } else {
            ctx.theme.palette.border_subtle
        };
        let line_width = if selected { 2.0 * dpi } else { dpi.max(1.0) };
        ctx.list.stroke_rounded(card_rect, border_color, CARD_RADIUS_LOGICAL * dpi, line_width);
        self.paint_miniature_tree(ctx, card_rect, option);
        if selected {
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
        option: &InspectorPresetItem,
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
        option: &InspectorPresetItem,
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
        option: &InspectorPresetItem,
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
            &self.descriptor.presets.title,
        );
        self.paint_summary(ctx);
        if let (Some(notice_rect), Some(notice)) = (self.notice_rect, self.notice_text()) {
            ctx.text(
                notice_rect.x,
                notice_rect.y + 14.0 * dpi,
                FONT_SIZE_NOTICE_LOGICAL * dpi,
                ctx.theme.palette.warning_text,
                notice,
            );
        }
        if self.presets_expanded {
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
                for (index, option) in self.descriptor.presets.items.iter().enumerate() {
                    if index < self.card_rects.len() {
                        self.paint_card(&mut clipped_ctx, index, option);
                    }
                }
            });
        }
    }
}

impl Widget for InspectorPanelWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        let dpi = ctx.dpi;
        let panel_rect = Rect::new(0.0, 0.0, rect.w, rect.h);
        let geometry_changed = self.rect != panel_rect || self.layout_dpi != dpi;
        self.focus_geometry = CardFocusGeometry::for_dpi(dpi);
        self.rect = panel_rect;
        let close_size = CLOSE_SIZE_LOGICAL * dpi;
        self.close_rect = Rect::new(
            (self.rect.w - CLOSE_RIGHT_INSET_LOGICAL * dpi - close_size).max(0.0),
            (HEADER_HEIGHT_LOGICAL * dpi - close_size) * 0.5,
            close_size,
            close_size,
        );
        self.layout_dpi = dpi;
        self.layout_content(dpi);
        if geometry_changed {
            self.ensure_focused_card_visible();
        }
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
        Some(ids::INSPECTOR_PANEL)
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
        let option = self.descriptor.presets.items.get(option_index)?;
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

impl InspectorPanelWidget {
    fn layout_content(&mut self, dpi: f32) {
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
        if !self.presets_expanded {
            self.scroll_offset = 0.0;
            self.scroll_limit = 0.0;
            return;
        }
        let column_gap = COLUMN_GAP_LOGICAL * dpi;
        let card_width = ((content_width - column_gap) / 2.0).max(0.0);
        let card_height = card_width / CARD_ASPECT_RATIO;
        for index in 0..self.descriptor.presets.items.len() {
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
            || self.hovered_option_index.is_some_and(|index| self.selection_enabled(index));
        ctx.cursor_hint = Some(if interactive { CursorIcon::Pointer } else { CursorIcon::Default });
        Some(WidgetAction::Consumed)
    }

    fn handle_left_click(&self, px: f32, py: f32) -> Option<WidgetAction> {
        if self.close_rect.contains(px, py) {
            return Some(self.action(InspectorPanelIntent::Close));
        }
        if self.summary_rect.contains(px, py) {
            return Some(
                self.action(InspectorPanelIntent::ToggleGroup(self.descriptor.presets.id.clone())),
            );
        }
        if let Some(index) = self.option_at(px, py) {
            return Some(self.select_option(index));
        }
        self.hit(px, py).then_some(WidgetAction::Consumed)
    }

    fn handle_key_down(
        &mut self,
        key_code: KeyCode,
        _modifiers: Modifiers,
    ) -> Option<WidgetAction> {
        match key_code {
            KeyCode::Escape => Some(self.action(InspectorPanelIntent::Close)),
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End => {
                if !self.presets_expanded {
                    return Some(WidgetAction::Consumed);
                }
                self.move_focus(key_code);
                Some(
                    self.item_target(self.focused_option_index)
                        .map_or(WidgetAction::Consumed, |target| {
                            self.action(InspectorPanelIntent::FocusItem(target))
                        }),
                )
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                Some(self.select_option(self.focused_option_index))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{DrawCmd, DrawList, NoopMeasure};

    fn preset_item(id: &str) -> InspectorPresetItem {
        InspectorPresetItem {
            id: ItemId(id.into()),
            display_name: id.into(),
            canvas_background: [0.14, 0.28, 0.42, 1.0],
            root_fill: [0.31, 0.37, 0.41, 1.0],
            branch_colors: vec![[0.23, 0.42, 0.71, 1.0]],
            disabled: false,
        }
    }

    fn descriptor() -> InspectorPanelDescriptor {
        InspectorPanelDescriptor {
            id: PanelId("appearance".into()),
            title: "Appearance".into(),
            presets: InspectorPresetGroup {
                id: GroupId("presets".into()),
                title: "Presets".into(),
                items: vec![preset_item("first"), preset_item("second")],
                selected_item: None,
                fallback_summary_item: Some(ItemId("first".into())),
            },
            notice: None,
            selection_disabled: false,
        }
    }

    fn layout(widget: &mut InspectorPanelWidget) {
        layout_at(widget, &crate::theme::test_theme(), 1.0, 600.0);
    }

    #[test]
    fn selecting_a_card_keeps_its_panel_group_and_item_ids() {
        let mut widget = InspectorPanelWidget::new();
        let descriptor = descriptor();
        let target = InspectorItemTarget {
            group_id: descriptor.presets.id.clone(),
            item_id: descriptor.presets.items[1].id.clone(),
        };
        widget.set_input(descriptor.clone(), &InspectorPanelPresentation::default());
        layout(&mut widget);
        let card = widget.visible_card_rect(1);
        assert_eq!(
            widget.handle_left_click(card.x + card.w / 2.0, card.y + card.h / 2.0),
            Some(WidgetAction::InspectorPanel(InspectorPanelAction {
                panel_id: descriptor.id,
                intent: InspectorPanelIntent::SelectItem(target),
            })),
        );
    }

    #[test]
    fn disabled_or_collapsed_items_consume_activation() {
        let mut widget = InspectorPanelWidget::new();
        let mut descriptor = descriptor();
        descriptor.presets.items[0].disabled = true;
        widget.set_input(descriptor.clone(), &InspectorPanelPresentation::default());
        layout(&mut widget);
        assert_eq!(widget.select_option(0), WidgetAction::Consumed);

        descriptor.selection_disabled = true;
        widget.set_input(descriptor.clone(), &InspectorPanelPresentation::default());
        assert_eq!(widget.select_option(1), WidgetAction::Consumed);

        descriptor.selection_disabled = false;
        widget.set_input(
            descriptor,
            &InspectorPanelPresentation { presets_expanded: false, focused_item: None },
        );
        assert_eq!(widget.select_option(1), WidgetAction::Consumed);
    }

    #[test]
    fn focus_restores_by_id_after_presets_reorder() {
        let mut widget = InspectorPanelWidget::new();
        let mut descriptor = descriptor();
        let focused = descriptor.presets.items[1].id.clone();
        descriptor.presets.items.swap(0, 1);
        widget.set_input(
            descriptor,
            &InspectorPanelPresentation { presets_expanded: true, focused_item: Some(focused) },
        );
        assert_eq!(widget.focused_option_index, 0);
    }

    #[test]
    fn fallback_summary_does_not_select_a_card() {
        let mut widget = InspectorPanelWidget::new();
        let descriptor = descriptor();
        widget.set_input(descriptor, &InspectorPanelPresentation::default());
        assert_eq!(widget.current_option().map(|item| item.id.0.as_str()), Some("first"));
        assert_eq!(widget.descriptor.presets.selected_item, None);
    }

    fn layout_at(widget: &mut InspectorPanelWidget, theme: &crate::Theme, dpi: f32, height: f32) {
        let mut measure = NoopMeasure;
        let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi };
        widget.set_rect(Rect::new(0.0, 0.0, PANEL_WIDTH_LOGICAL * dpi, height * dpi), &mut layout);
    }

    fn sixteen_presets() -> InspectorPanelDescriptor {
        let mut descriptor = descriptor();
        descriptor.presets.items =
            (1..=16).map(|index| preset_item(&format!("preset-{index}"))).collect();
        descriptor
    }

    fn left_press(rect: Rect) -> Event {
        Event::MouseDown {
            px: rect.x + rect.w * 0.5,
            py: rect.y + rect.h * 0.5,
            button: MouseButton::Left,
        }
    }

    #[test]
    fn preset_geometry_has_two_columns_and_eight_rows_at_each_scale() {
        for dpi in [1.0, 2.0] {
            let mut widget = InspectorPanelWidget::new();
            widget.set_input(sixteen_presets(), &InspectorPanelPresentation::default());
            layout_at(&mut widget, &crate::theme::test_theme(), dpi, 600.0);
            let rows = widget.card_rects.chunks_exact(CARD_COLUMN_COUNT);
            assert_eq!(rows.len(), 8);
            assert!(rows.remainder().is_empty());
            for (index, row) in rows.enumerate() {
                assert_eq!(row[0].x, PANEL_PADDING_LOGICAL * dpi);
                assert_eq!(row[1].x - row[0].right(), COLUMN_GAP_LOGICAL * dpi);
                assert_eq!(row[0].y, row[1].y);
                assert_eq!(row[0].w, row[1].w);
                assert_eq!(
                    row[0].y,
                    widget.grid_clip_top + index as f32 * (row[0].h + ROW_GAP_LOGICAL * dpi)
                );
            }
        }
    }

    #[test]
    fn descriptor_colors_and_selected_marker_are_drawn_until_group_collapses() {
        let mut descriptor = descriptor();
        let preset = &descriptor.presets.items[0];
        let preview = [preset.canvas_background, preset.root_fill, preset.branch_colors[0]];
        descriptor.presets.selected_item = Some(preset.id.clone());

        let theme = crate::theme::test_theme();
        let mut widget = InspectorPanelWidget::new();
        widget.set_input(descriptor.clone(), &InspectorPanelPresentation::default());
        layout_at(&mut widget, &theme, 1.0, 600.0);

        let mut expanded = DrawList::new();
        widget.paint(&mut PaintCtx::new(&mut expanded, &theme, 1.0));
        for expected in preview {
            assert!(expanded.cmds.iter().any(|command| matches!(
                command, DrawCmd::FillRect { color, .. } if *color == expected
            )));
        }
        assert!(expanded.cmds.iter().any(|command| matches!(
            command, DrawCmd::StrokeRect { color, line_width, .. }
                if *color == theme.palette.accent_text && *line_width == 2.0
        )));
        assert!(expanded.cmds.iter().any(|command| matches!(
            command, DrawCmd::FillRect { color, radius, .. }
                if *color == theme.palette.accent && *radius == 7.0
        )));

        let action =
            widget.on_event(&left_press(widget.summary_rect), &mut EventCtx::new(&theme, 1.0));
        assert_eq!(
            action,
            Some(WidgetAction::InspectorPanel(InspectorPanelAction {
                panel_id: descriptor.id.clone(),
                intent: InspectorPanelIntent::ToggleGroup(descriptor.presets.id.clone()),
            }))
        );
        widget.set_input(
            descriptor,
            &InspectorPanelPresentation { presets_expanded: false, focused_item: None },
        );
        assert!(widget.card_rects.is_empty());
        let mut collapsed = DrawList::new();
        widget.paint(&mut PaintCtx::new(&mut collapsed, &theme, 1.0));
        assert!(collapsed.cmds.iter().any(|command| matches!(
            command, DrawCmd::FillRect { color, .. } if *color == preview[2]
        )));
        assert!(!collapsed.cmds.iter().any(|command| matches!(
            command, DrawCmd::FillRect { color, radius, .. }
                if *color == preview[0] && *radius == CARD_RADIUS_LOGICAL
        )));
    }

    #[test]
    fn keyboard_navigation_only_moves_focus_and_click_selects_without_moving_it() {
        let theme = crate::theme::test_theme();
        let mut widget = InspectorPanelWidget::new();
        let descriptor = descriptor();
        let group_id = descriptor.presets.id.clone();
        let first = descriptor.presets.items[0].id.clone();
        let second = descriptor.presets.items[1].id.clone();
        let action = |intent| {
            Some(WidgetAction::InspectorPanel(InspectorPanelAction {
                panel_id: descriptor.id.clone(),
                intent,
            }))
        };
        let target = |item_id| InspectorItemTarget { group_id: group_id.clone(), item_id };
        widget.set_input(descriptor.clone(), &InspectorPanelPresentation::default());
        layout_at(&mut widget, &theme, 1.0, 600.0);

        let mut context = EventCtx::new(&theme, 1.0);
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Right, Modifiers::NONE), &mut context),
            action(InspectorPanelIntent::FocusItem(target(second.clone())))
        );
        let presentation = InspectorPanelPresentation {
            presets_expanded: true,
            focused_item: Some(second.clone()),
        };
        widget.set_input(descriptor.clone(), &presentation);

        let first_card = widget.visible_card_rect(0);
        assert_eq!(
            widget.on_event(&left_press(first_card), &mut context),
            action(InspectorPanelIntent::SelectItem(target(first.clone())))
        );
        let mut selected = descriptor.clone();
        selected.presets.selected_item = Some(first);
        widget.set_input(selected, &presentation);
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            action(InspectorPanelIntent::SelectItem(target(second)))
        );
    }

    #[test]
    fn disabled_panel_consumes_clicks_and_activation_keys_but_routes_ime_to_its_host() {
        let theme = crate::theme::test_theme();
        let mut widget = InspectorPanelWidget::new();
        let mut descriptor = descriptor();
        descriptor.notice = Some("Invalid preset metadata".into());
        descriptor.selection_disabled = true;
        widget.set_input(descriptor, &InspectorPanelPresentation::default());
        layout_at(&mut widget, &theme, 1.0, 600.0);
        widget.set_keyboard_focus(widget.id());
        let card = widget.visible_card_rect(0);
        let mut context = EventCtx::new(&theme, 1.0);
        for event in [
            left_press(card),
            Event::KeyDown(KeyCode::Enter, Modifiers::NONE),
            Event::KeyDown(KeyCode::Char(' '), Modifiers::NONE),
        ] {
            assert_eq!(widget.on_event(&event, &mut context), Some(WidgetAction::Consumed));
        }
        for event in [
            Event::ImeEnable,
            Event::ImePreedit { text: "预".into(), cursor: Some((0, "预".len())) },
            Event::ImeCommit("完成".into()),
            Event::ImeDisable,
        ] {
            assert_eq!(widget.on_event(&event, &mut context), None);
        }
        assert_eq!(widget.focused_option_index, 0);
        assert!(widget.descriptor.presets.selected_item.is_none());
    }

    fn focus_clips(draw_list: &DrawList, theme: &crate::Theme) -> Vec<Rect> {
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

    fn focus_paint_bounds(draw_list: &DrawList, theme: &crate::Theme) -> Rect {
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

    fn laid_out_focus_panel(theme: &crate::Theme, dpi: f32, height: f32) -> InspectorPanelWidget {
        let mut widget = InspectorPanelWidget::new();
        widget.set_input(sixteen_presets(), &InspectorPanelPresentation::default());
        layout_at(&mut widget, theme, dpi, height);
        widget.set_keyboard_focus(widget.id());
        widget
    }

    #[test]
    fn keyboard_home_end_keep_complete_focus_bounds_visible_when_card_fits() {
        let theme = crate::Theme::from_definition(&crate::theme::ThemeDefinition::default_dark());
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
        let mut widget = InspectorPanelWidget::new();
        widget.set_input(sixteen_presets(), &InspectorPanelPresentation::default());
        layout(&mut widget);
        widget.set_keyboard_focus(widget.id());
        let theme = crate::Theme::from_definition(&crate::theme::ThemeDefinition::default_dark());
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
        let theme = crate::Theme::from_definition(&crate::theme::ThemeDefinition::default_dark());
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
    fn descriptor_notice_uses_warning_text_color() {
        const NOTICE: &str = "Preset metadata requires repair";
        let theme = crate::theme::test_theme();
        let mut widget = InspectorPanelWidget::new();
        let mut descriptor = descriptor();
        descriptor.notice = Some(NOTICE.into());
        widget.set_input(descriptor, &InspectorPanelPresentation::default());
        layout_at(&mut widget, &theme, 1.0, 600.0);

        let mut shaper = shaping::Shaper::new().expect("测试环境必须能创建文字塑形器");
        let mut draw_list = DrawList::new();
        let mut context = PaintCtx::new(&mut draw_list, &theme, 1.0);
        context.shaper = Some(&mut shaper);
        widget.paint(&mut context);
        assert!(draw_list.cmds.iter().any(|command| matches!(
            command,
            DrawCmd::TextLayout { layout, color, .. }
                if layout.text == NOTICE && *color == theme.palette.warning_text
        )));
    }
}
