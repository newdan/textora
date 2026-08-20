use std::ops::Range;

use crate::core::Rect;
use crate::core::text_layout::wrap_text_to_lines;
use crate::core::text_util::estimate_text_width_px;

use super::CardInput;

pub const CARD_VERTICAL_GAP_LOGICAL: f32 = 8.0;
pub const CARD_HORIZONTAL_PADDING_LOGICAL: f32 = 14.0;
pub const CARD_VERTICAL_PADDING_LOGICAL: f32 = 12.0;
pub const CARD_ICON_SLOT_SIZE_LOGICAL: f32 = 24.0;
pub const CARD_ICON_GLYPH_SIZE_LOGICAL: f32 = 14.0;
pub const CARD_ICON_GAP_LOGICAL: f32 = 8.0;
pub const CARD_TITLE_FONT_SIZE_LOGICAL: f32 = 15.0;
pub const CARD_EXCERPT_FONT_SIZE_LOGICAL: f32 = 13.0;
pub const CARD_METADATA_FONT_SIZE_LOGICAL: f32 = 12.0;
pub const CARD_TITLE_LINE_HEIGHT_LOGICAL: f32 = 20.0;
pub const CARD_EXCERPT_LINE_HEIGHT_LOGICAL: f32 = 18.0;
pub const CARD_TITLE_MAX_LINES: usize = 2;
pub const CARD_EXCERPT_MAX_LINES: usize = 2;
pub const CARD_CORNER_RADIUS_LOGICAL: f32 = 8.0;
pub const CARD_METADATA_GAP_LOGICAL: f32 = 12.0;
pub const VIRTUAL_CARD_OVERSCAN_COUNT: usize = 2;
pub const CARD_CLOSE_BUTTON_SIZE_LOGICAL: f32 = 24.0;
pub const CARD_CLOSE_ICON_SIZE_LOGICAL: f32 = 16.0;

const CARD_TEXT_SECTION_GAP_LOGICAL: f32 = 6.0;
const CARD_CONTENT_METADATA_GAP_LOGICAL: f32 = 12.0;
const CARD_TITLE_CLOSE_GAP_LOGICAL: f32 = 6.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CardPlacement {
    top_px: f32,
    height_px: f32,
    title_line_count: usize,
    excerpt_line_count: usize,
    closable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardGeometry {
    pub card_rect: Rect,
    /// 为图标预留的固定位置；没有图标的卡片仍保留该位置以保证标题对齐。
    pub icon_rect: Rect,
    pub title_rect: Rect,
    pub title_baseline: f32,
    pub close_rect: Rect,
    pub excerpt_rect: Rect,
    pub excerpt_baseline: f32,
    pub metadata_rect: Rect,
    pub metadata_baseline: f32,
    pub tag_rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VirtualCardListLayout {
    pub viewport_rect: Rect,
    pub card_count: usize,
    pub visible_range: Range<usize>,
    pub content_height_px: f32,
    card_placements: Vec<CardPlacement>,
    scroll_offset_px: f32,
    dpi: f32,
}

impl VirtualCardListLayout {
    pub fn card_geometry(&self, index: usize) -> CardGeometry {
        debug_assert!(index < self.card_count, "card geometry requires an in-range card index");
        let placement = self.card_placements[index];
        let card_rect = Rect::new(
            self.viewport_rect.x,
            self.viewport_rect.y + placement.top_px - self.scroll_offset_px,
            self.viewport_rect.w,
            placement.height_px,
        );
        let horizontal_padding = CARD_HORIZONTAL_PADDING_LOGICAL * self.dpi;
        let vertical_padding = CARD_VERTICAL_PADDING_LOGICAL * self.dpi;
        let icon_size = CARD_ICON_SLOT_SIZE_LOGICAL * self.dpi;
        let title_line_height = CARD_TITLE_LINE_HEIGHT_LOGICAL * self.dpi;
        let icon_rect = Rect::new(
            card_rect.x + horizontal_padding,
            card_rect.y + vertical_padding - (icon_size - title_line_height) * 0.5,
            icon_size,
            icon_size,
        );
        let title_x = icon_rect.right() + CARD_ICON_GAP_LOGICAL * self.dpi;
        let close_button_size = CARD_CLOSE_BUTTON_SIZE_LOGICAL * self.dpi;
        let close_rect = if placement.closable {
            Rect::new(
                card_rect.right() - horizontal_padding - close_button_size,
                card_rect.y + vertical_padding - (close_button_size - icon_size) * 0.5,
                close_button_size,
                close_button_size,
            )
        } else {
            Rect::ZERO
        };
        let title_font_size = CARD_TITLE_FONT_SIZE_LOGICAL * self.dpi;
        let excerpt_font_size = CARD_EXCERPT_FONT_SIZE_LOGICAL * self.dpi;
        let metadata_font_size = CARD_METADATA_FONT_SIZE_LOGICAL * self.dpi;
        let title_y = card_rect.y + vertical_padding;
        let excerpt_line_height = CARD_EXCERPT_LINE_HEIGHT_LOGICAL * self.dpi;
        let title_height = title_line_height * placement.title_line_count as f32;
        let excerpt_height = excerpt_line_height * placement.excerpt_line_count as f32;
        let excerpt_y = title_y
            + title_height
            + usize::from(placement.excerpt_line_count > 0) as f32
                * CARD_TEXT_SECTION_GAP_LOGICAL
                * self.dpi;
        let metadata_y = card_rect.bottom() - vertical_padding - metadata_font_size;
        let text_right = if placement.closable {
            close_rect.x - CARD_TITLE_CLOSE_GAP_LOGICAL * self.dpi
        } else {
            card_rect.right() - horizontal_padding
        };
        let secondary_text_right = card_rect.right() - horizontal_padding;
        let secondary_text_x = card_rect.x + horizontal_padding;
        let metadata_width = (secondary_text_right - secondary_text_x) * 0.42;
        let metadata_rect =
            Rect::new(secondary_text_x, metadata_y, metadata_width.max(0.0), metadata_font_size);

        CardGeometry {
            card_rect,
            icon_rect,
            title_rect: Rect::new(title_x, title_y, (text_right - title_x).max(0.0), title_height),
            title_baseline: title_y + title_font_size * 0.8,
            close_rect,
            excerpt_rect: Rect::new(
                secondary_text_x,
                excerpt_y,
                (secondary_text_right - secondary_text_x).max(0.0),
                excerpt_height,
            ),
            excerpt_baseline: excerpt_y + excerpt_font_size * 0.8,
            metadata_rect,
            metadata_baseline: metadata_y + metadata_font_size * 0.8,
            tag_rect: Rect::new(
                metadata_rect.right() + CARD_METADATA_GAP_LOGICAL * self.dpi,
                metadata_y,
                (secondary_text_right
                    - metadata_rect.right()
                    - CARD_METADATA_GAP_LOGICAL * self.dpi)
                    .max(0.0),
                metadata_font_size,
            ),
        }
    }
}

pub(super) fn build_virtual_card_layout(
    cards: &[CardInput],
    viewport_rect: Rect,
    scroll_offset_px: f32,
    dpi: f32,
) -> VirtualCardListLayout {
    let card_placements = build_card_placements(cards, viewport_rect.w, dpi);
    let content_height_px = card_placements
        .last()
        .map(|placement| placement.top_px + placement.height_px)
        .unwrap_or(0.0);
    let first_visible = card_placements
        .partition_point(|placement| placement.top_px + placement.height_px <= scroll_offset_px);
    let viewport_end = scroll_offset_px + viewport_rect.h;
    let visible_end = card_placements.partition_point(|placement| placement.top_px < viewport_end);
    let range_start = first_visible.saturating_sub(VIRTUAL_CARD_OVERSCAN_COUNT);
    let range_end =
        visible_end.saturating_add(VIRTUAL_CARD_OVERSCAN_COUNT).min(cards.len()).max(range_start);
    let visible_range = range_start..range_end;

    VirtualCardListLayout {
        viewport_rect,
        card_count: cards.len(),
        visible_range,
        content_height_px,
        card_placements,
        scroll_offset_px,
        dpi,
    }
}

fn build_card_placements(cards: &[CardInput], card_width_px: f32, dpi: f32) -> Vec<CardPlacement> {
    let horizontal_padding = CARD_HORIZONTAL_PADDING_LOGICAL * dpi;
    let title_width_px = (card_width_px
        - horizontal_padding * 2.0
        - CARD_ICON_SLOT_SIZE_LOGICAL * dpi
        - CARD_ICON_GAP_LOGICAL * dpi)
        .max(0.0);
    let excerpt_width_px = (card_width_px - horizontal_padding * 2.0).max(0.0);
    let card_gap_px = CARD_VERTICAL_GAP_LOGICAL * dpi;
    let mut next_card_top_px = 0.0;

    cards
        .iter()
        .map(|card| {
            let title_line_count = card_text_lines(
                &card.title,
                title_width_px,
                CARD_TITLE_FONT_SIZE_LOGICAL * dpi,
                CARD_TITLE_MAX_LINES,
            )
            .len()
            .max(1);
            let excerpt_line_count = card_text_lines(
                &card.excerpt,
                excerpt_width_px,
                CARD_EXCERPT_FONT_SIZE_LOGICAL * dpi,
                CARD_EXCERPT_MAX_LINES,
            )
            .len();
            let height_px = card_height_px(title_line_count, excerpt_line_count, dpi);
            let placement = CardPlacement {
                top_px: next_card_top_px,
                height_px,
                title_line_count,
                excerpt_line_count,
                closable: card.closable,
            };
            next_card_top_px += height_px + card_gap_px;
            placement
        })
        .collect()
}

fn card_height_px(title_line_count: usize, excerpt_line_count: usize, dpi: f32) -> f32 {
    let title_height_px = title_line_count as f32 * CARD_TITLE_LINE_HEIGHT_LOGICAL * dpi;
    let excerpt_height_px = excerpt_line_count as f32 * CARD_EXCERPT_LINE_HEIGHT_LOGICAL * dpi;
    let text_section_gap_px =
        usize::from(excerpt_line_count > 0) as f32 * CARD_TEXT_SECTION_GAP_LOGICAL * dpi;

    CARD_VERTICAL_PADDING_LOGICAL * 2.0 * dpi
        + title_height_px
        + text_section_gap_px
        + excerpt_height_px
        + CARD_CONTENT_METADATA_GAP_LOGICAL * dpi
        + CARD_METADATA_FONT_SIZE_LOGICAL * dpi
}

pub(super) fn card_text_lines(
    text: &str,
    max_width_px: f32,
    font_size_px: f32,
    max_lines: usize,
) -> Vec<String> {
    wrap_text_to_lines(text, max_width_px, max_lines, |candidate| {
        estimate_text_width_px(candidate, font_size_px)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_secondary_text_uses_one_left_alignment() {
        let cards = vec![card("")];
        let layout =
            build_virtual_card_layout(&cards, Rect::new(20.0, 30.0, 300.0, 500.0), 0.0, 1.0);
        let geometry = layout.card_geometry(0);
        let expected_text_x = geometry.card_rect.x + CARD_HORIZONTAL_PADDING_LOGICAL;

        assert_eq!(geometry.excerpt_rect.x, expected_text_x);
        assert_eq!(geometry.metadata_rect.x, expected_text_x);
    }

    #[test]
    fn card_height_matches_its_measured_text_rows() {
        let cards = vec![card("一行摘要")];
        let layout = build_virtual_card_layout(&cards, Rect::new(0.0, 0.0, 300.0, 500.0), 0.0, 1.0);
        let geometry = layout.card_geometry(0);

        assert_eq!(geometry.card_rect.h, card_height_px(1, 1, 1.0));
        assert_eq!(geometry.title_rect.h, CARD_TITLE_LINE_HEIGHT_LOGICAL);
        assert_eq!(geometry.excerpt_rect.h, CARD_EXCERPT_LINE_HEIGHT_LOGICAL);
        assert!(geometry.excerpt_rect.bottom() < geometry.metadata_rect.top());
    }

    #[test]
    fn card_title_reserves_a_compact_icon_slot() {
        let cards = vec![card("")];
        let layout =
            build_virtual_card_layout(&cards, Rect::new(20.0, 30.0, 300.0, 500.0), 0.0, 1.0);
        let geometry = layout.card_geometry(0);

        assert_eq!(geometry.icon_rect.w, 24.0);
        assert_eq!(geometry.icon_rect.h, 24.0);
        assert_eq!(geometry.title_rect.x, geometry.icon_rect.right() + 8.0);
    }

    fn card(excerpt: &str) -> CardInput {
        CardInput {
            key: super::super::CardKey(1),
            title: "标题".to_owned(),
            excerpt: excerpt.to_owned(),
            timestamp: "刚刚".to_owned(),
            icon: None,
            tag_summary: String::new(),
            selection: super::super::CardSelection::Unselected,
            closable: false,
        }
    }
}
