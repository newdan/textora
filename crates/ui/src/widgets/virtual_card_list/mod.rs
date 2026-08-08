//! 与产品领域无关的虚拟化卡片列表。

mod layout;

use std::any::Any;

use crate::core::widget::{ControlAction, WidgetId};
use crate::core::{
    AccessibilityAction, AccessibilityActionRequest, AccessibilityContext, AccessibilityId,
    AccessibilityNode, AccessibilityRole, DrawCmd, Event, EventCtx, KeyCode, LayoutCtx,
    MouseButton, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::widgets::icon::draw_icon;

pub use layout::{CardGeometry, VirtualCardListLayout};

use self::layout::build_virtual_card_layout;

/// 仅在单帧 UI 输入中有效的卡片键。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CardKey(pub u64);

/// 卡片的选择状态，由调用方基于稳定键持有。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardSelection {
    #[default]
    Unselected,
    Selected,
}

/// 一张卡片的纯展示输入。字符串均由调用方预先准备。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardInput {
    pub key: CardKey,
    pub title: String,
    pub excerpt: String,
    pub timestamp: String,
    pub icon: Option<String>,
    pub tag_summary: String,
    pub selection: CardSelection,
}

/// 虚拟列表每帧输入。选择和滚动状态分别由调用方维护。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VirtualCardListInput {
    pub cards: Vec<CardInput>,
    pub scroll_offset_px: f32,
}

/// 虚拟卡片列表向调用方返回的通用 UI 动作。
#[derive(Clone, Debug, PartialEq)]
pub enum VirtualCardListAction {
    Selected(CardKey),
    Activated(CardKey),
    ScrollOffsetChanged(f32),
    HoverChanged(Option<CardKey>),
}

/// 仅布局可见卡片范围的纵向列表组件。
pub struct VirtualCardListWidget {
    id: Option<WidgetId>,
    rect: Rect,
    input: VirtualCardListInput,
    layout: VirtualCardListLayout,
    selected_key: Option<CardKey>,
    hovered_key: Option<CardKey>,
    focused: bool,
    accessibility_label: Option<String>,
}

impl Default for VirtualCardListWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualCardListWidget {
    pub fn new() -> Self {
        Self {
            id: None,
            rect: Rect::ZERO,
            input: VirtualCardListInput::default(),
            layout: VirtualCardListLayout::default(),
            selected_key: None,
            hovered_key: None,
            focused: false,
            accessibility_label: None,
        }
    }

    pub fn with_id(mut self, id: WidgetId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn set_accessibility_label(&mut self, label: Option<String>) {
        self.accessibility_label = label;
    }

    /// 更新当前帧输入，同时按稳定键保留仍存在的选择和悬停状态。
    pub fn set_input(&mut self, mut input: VirtualCardListInput) {
        debug_assert!(has_unique_keys(&input.cards), "card keys must be unique per frame");
        let input_selected_key = input
            .cards
            .iter()
            .find(|card| card.selection == CardSelection::Selected)
            .map(|card| card.key);
        let preserved_selected_key =
            self.selected_key.filter(|key| input.cards.iter().any(|card| card.key == *key));
        self.selected_key = input_selected_key.or(preserved_selected_key);
        for card in &mut input.cards {
            card.selection = if Some(card.key) == self.selected_key {
                CardSelection::Selected
            } else {
                CardSelection::Unselected
            };
        }
        self.hovered_key =
            self.hovered_key.filter(|key| input.cards.iter().any(|card| card.key == *key));
        self.input = input;
    }

    pub fn input(&self) -> &VirtualCardListInput {
        &self.input
    }

    pub fn layout(&self) -> &VirtualCardListLayout {
        &self.layout
    }

    pub fn selected_key(&self) -> Option<CardKey> {
        self.selected_key
    }

    fn max_scroll_offset(&self) -> f32 {
        (self.layout.content_height_px - self.rect.h).max(0.0)
    }

    fn card_at(&self, px: f32, py: f32) -> Option<usize> {
        if !self.rect.contains(px, py) {
            return None;
        }
        self.layout
            .visible_range
            .clone()
            .find(|index| self.layout.card_geometry(*index).card_rect.contains(px, py))
    }

    fn update_hover(&mut self, px: f32, py: f32) -> Option<VirtualCardListAction> {
        let hovered_key =
            self.card_at(px, py).and_then(|index| self.input.cards.get(index)).map(|card| card.key);
        if hovered_key == self.hovered_key {
            return None;
        }
        self.hovered_key = hovered_key;
        Some(VirtualCardListAction::HoverChanged(hovered_key))
    }

    fn select_adjacent_card(&self, direction: i32) -> Option<VirtualCardListAction> {
        let selected_index = self
            .input
            .cards
            .iter()
            .position(|card| card.selection == CardSelection::Selected)
            .unwrap_or(0);
        let next_index = if direction.is_negative() {
            selected_index.saturating_sub(1)
        } else {
            (selected_index + 1).min(self.input.cards.len().saturating_sub(1))
        };
        self.input.cards.get(next_index).map(|card| VirtualCardListAction::Selected(card.key))
    }

    fn select_card(&mut self, key: CardKey) {
        self.selected_key = Some(key);
        for card in &mut self.input.cards {
            card.selection =
                if card.key == key { CardSelection::Selected } else { CardSelection::Unselected };
        }
    }
}

impl Widget for VirtualCardListWidget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = rect;
        self.layout = build_virtual_card_layout(
            self.input.cards.len(),
            rect,
            self.input.scroll_offset_px,
            ctx.dpi,
        );
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }

        let clip_rect = Rect::new(
            self.rect.x + ctx.list.offset.0,
            self.rect.y + ctx.list.offset.1,
            self.rect.w,
            self.rect.h,
        );
        ctx.list.cmds.push(DrawCmd::PushClip(clip_rect));
        for index in self.layout.visible_range.clone() {
            let Some(card) = self.input.cards.get(index) else {
                continue;
            };
            let geometry = self.layout.card_geometry(index);
            let is_selected = card.selection == CardSelection::Selected;
            let is_hovered = self.hovered_key == Some(card.key);
            let background = if is_selected {
                ctx.theme.palette.sidebar_active_bg
            } else if is_hovered {
                ctx.theme.palette.sidebar_hover_bg
            } else {
                ctx.theme.palette.bg_surface
            };
            ctx.list.fill_rounded(
                geometry.card_rect,
                background,
                layout::CARD_CORNER_RADIUS_LOGICAL * ctx.dpi,
            );

            if let Some(icon) = &card.icon {
                draw_icon(
                    ctx.list,
                    icon,
                    geometry.icon_rect.x,
                    geometry.icon_rect.y,
                    geometry.icon_rect.w,
                    ctx.theme.palette.text_muted,
                );
            }
            let title_color = if is_selected {
                ctx.theme.palette.sidebar_active_fg
            } else {
                ctx.theme.palette.text_main
            };
            ctx.text(
                geometry.title_rect.x,
                geometry.title_baseline,
                layout::CARD_TITLE_FONT_SIZE_LOGICAL * ctx.dpi,
                title_color,
                &card.title,
            );
            ctx.text(
                geometry.excerpt_rect.x,
                geometry.excerpt_baseline,
                layout::CARD_EXCERPT_FONT_SIZE_LOGICAL * ctx.dpi,
                ctx.theme.palette.text_muted,
                &card.excerpt,
            );
            ctx.text(
                geometry.metadata_rect.x,
                geometry.metadata_baseline,
                layout::CARD_METADATA_FONT_SIZE_LOGICAL * ctx.dpi,
                ctx.theme.palette.text_muted,
                &card.timestamp,
            );
            if !card.tag_summary.is_empty() {
                ctx.text(
                    geometry.tag_rect.x,
                    geometry.metadata_baseline,
                    layout::CARD_METADATA_FONT_SIZE_LOGICAL * ctx.dpi,
                    ctx.theme.palette.accent,
                    &card.tag_summary,
                );
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
    }

    fn accessibility_node(&self, ctx: &AccessibilityContext) -> Option<AccessibilityNode> {
        let id = self.id?;
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return None;
        }
        let root_id = AccessibilityId::from(id);
        let mut root =
            AccessibilityNode::new(root_id, AccessibilityRole::List, ctx.screen_bounds(self.rect))
                .with_name(self.accessibility_label.as_deref().unwrap_or("卡片列表"))
                .with_focused(self.focused)
                .with_action(AccessibilityAction::Focus);
        for index in self.layout.visible_range.clone() {
            let Some(card) = self.input.cards.get(index) else {
                continue;
            };
            let geometry = self.layout.card_geometry(index);
            if geometry.card_rect.bottom() <= self.rect.top()
                || geometry.card_rect.top() >= self.rect.bottom()
            {
                continue;
            }
            let description =
                [card.excerpt.as_str(), card.timestamp.as_str(), card.tag_summary.as_str()]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("，");
            let mut child = AccessibilityNode::new(
                root_id.child(card.key.0),
                AccessibilityRole::ListItem,
                ctx.screen_bounds(geometry.card_rect),
            )
            .with_name(card.title.clone())
            .with_selected(card.selection == CardSelection::Selected)
            .with_action(AccessibilityAction::Activate);
            if !description.is_empty() {
                child = child.with_description(description);
            }
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
        let card =
            self.input.cards.iter().find(|card| root_id.child(card.key.0) == request.target)?;
        match request.action {
            AccessibilityAction::Activate => {
                Some(WidgetAction::VirtualCardList(VirtualCardListAction::Activated(card.key)))
            }
            _ => None,
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if self.id.is_some() && !self.focused && matches!(event, Event::KeyDown(..)) {
            return None;
        }
        let action = match event {
            Event::MouseMove { px, py } => {
                if self.hit(*px, *py) {
                    ctx.cursor_hint = Some(winit::window::CursorIcon::Pointer);
                }
                self.update_hover(*px, *py)
            }
            Event::MouseDown { px, py, button: MouseButton::Left } => self
                .card_at(*px, *py)
                .and_then(|index| self.input.cards.get(index))
                .map(|card| VirtualCardListAction::Selected(card.key)),
            Event::Wheel { dy, px, py, .. } if self.hit(*px, *py) => {
                let next_offset =
                    (self.input.scroll_offset_px - *dy).clamp(0.0, self.max_scroll_offset());
                if (next_offset - self.input.scroll_offset_px).abs() <= f32::EPSILON {
                    None
                } else {
                    self.input.scroll_offset_px = next_offset;
                    Some(VirtualCardListAction::ScrollOffsetChanged(next_offset))
                }
            }
            Event::KeyDown(KeyCode::Up, _) => self.select_adjacent_card(-1),
            Event::KeyDown(KeyCode::Down, _) => self.select_adjacent_card(1),
            Event::KeyDown(KeyCode::Enter, _) => {
                self.selected_key.map(VirtualCardListAction::Activated)
            }
            _ => None,
        }?;
        if let VirtualCardListAction::Selected(key) = action {
            self.select_card(key);
        }
        Some(WidgetAction::VirtualCardList(action))
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn has_unique_keys(cards: &[CardInput]) -> bool {
    let mut keys = std::collections::HashSet::with_capacity(cards.len());
    cards.iter().all(|card| keys.insert(card.key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{EventCtx, LayoutCtx, Modifiers, NoopMeasure};

    fn card(key: u64) -> CardInput {
        CardInput {
            key: CardKey(key),
            title: format!("Card {key}"),
            excerpt: "Precomputed excerpt".to_owned(),
            timestamp: "Just now".to_owned(),
            icon: Some("file-text".to_owned()),
            tag_summary: "#work".to_owned(),
            selection: CardSelection::Unselected,
        }
    }

    fn layout(widget: &mut VirtualCardListWidget, rect: Rect, dpi: f32) {
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut context = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
        widget.set_rect(rect, &mut context);
    }

    #[test]
    fn accessibility_exposes_visible_cards_and_activation_action() {
        let id = crate::WidgetId(70);
        let mut widget = VirtualCardListWidget::new().with_id(id);
        widget.set_accessibility_label(Some("笔记卡片".into()));
        let mut selected = card(2);
        selected.selection = CardSelection::Selected;
        widget.set_input(VirtualCardListInput {
            cards: vec![card(1), selected],
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 360.0, 220.0), 1.0);
        let theme = crate::theme::test_theme();
        let mut event_context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Down, Modifiers::NONE), &mut event_context),
            None
        );
        widget.set_keyboard_focus(Some(id));
        let node = widget
            .accessibility_node(&crate::core::AccessibilityContext::new(10.0, 20.0))
            .expect("identified card list should expose semantics");

        assert_eq!(node.role, crate::core::AccessibilityRole::List);
        assert_eq!(node.name.as_deref(), Some("笔记卡片"));
        assert!(node.state.focused);
        assert_eq!(node.children.len(), 2);
        assert_eq!(node.children[0].name.as_deref(), Some("Card 1"));
        assert_eq!(node.children[1].state.selected, Some(true));
        assert_eq!(
            widget.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                node.children[0].id,
                crate::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::VirtualCardList(VirtualCardListAction::Activated(CardKey(1))))
        );
    }

    #[test]
    fn lays_out_no_cards_for_an_empty_input() {
        let mut widget = VirtualCardListWidget::new();
        layout(&mut widget, Rect::new(0.0, 0.0, 360.0, 500.0), 1.0);

        assert_eq!(widget.layout().visible_range, 0..0);
        assert_eq!(widget.layout().content_height_px, 0.0);
    }

    #[test]
    fn ten_thousand_cards_only_layout_the_viewport_and_overscan() {
        let mut widget = VirtualCardListWidget::new();
        widget.set_input(VirtualCardListInput {
            cards: (0..10_000).map(card).collect(),
            scroll_offset_px: 48_000.0,
        });
        layout(&mut widget, Rect::new(10.0, 20.0, 360.0, 500.0), 1.0);

        assert!(widget.layout().visible_range.start > 0);
        assert!(widget.layout().visible_range.len() < 20);
        assert_eq!(widget.layout().card_count, 10_000);
    }

    #[test]
    fn stable_selection_survives_card_replacement() {
        let mut widget = VirtualCardListWidget::new();
        let mut selected_card = card(7);
        selected_card.selection = CardSelection::Selected;
        widget.set_input(VirtualCardListInput {
            cards: vec![card(3), selected_card],
            scroll_offset_px: 0.0,
        });
        widget.set_input(VirtualCardListInput {
            cards: vec![card(9), card(7)],
            scroll_offset_px: 80.0,
        });

        assert_eq!(widget.selected_key(), Some(CardKey(7)));
        assert_eq!(widget.input().scroll_offset_px, 80.0);
        assert_eq!(
            widget
                .input()
                .cards
                .iter()
                .find(|card| card.key == CardKey(7))
                .map(|card| card.selection),
            Some(CardSelection::Selected)
        );
    }

    #[test]
    fn pointer_and_keyboard_emit_stable_card_keys() {
        let mut widget = VirtualCardListWidget::new();
        let mut first_card = card(1);
        first_card.selection = CardSelection::Selected;
        widget.set_input(VirtualCardListInput {
            cards: vec![first_card, card(2)],
            scroll_offset_px: 0.0,
        });
        layout(&mut widget, Rect::new(0.0, 0.0, 360.0, 500.0), 1.0);
        let card_rect = widget.layout().card_geometry(1).card_rect;
        let theme = crate::theme::test_theme();
        let mut context = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        assert_eq!(
            widget.on_event(
                &Event::MouseDown {
                    px: card_rect.x + 1.0,
                    py: card_rect.y + 1.0,
                    button: MouseButton::Left
                },
                &mut context,
            ),
            Some(WidgetAction::VirtualCardList(VirtualCardListAction::Selected(CardKey(2))))
        );
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Up, Modifiers::NONE), &mut context),
            Some(WidgetAction::VirtualCardList(VirtualCardListAction::Selected(CardKey(1))))
        );
        assert_eq!(
            widget.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut context),
            Some(WidgetAction::VirtualCardList(VirtualCardListAction::Activated(CardKey(1))))
        );
    }
}
