use std::ops::Range;

use crate::core::Rect;

pub const CARD_HEIGHT_LOGICAL: f32 = 108.0;
pub const CARD_VERTICAL_GAP_LOGICAL: f32 = 8.0;
pub const CARD_HORIZONTAL_PADDING_LOGICAL: f32 = 14.0;
pub const CARD_VERTICAL_PADDING_LOGICAL: f32 = 12.0;
pub const CARD_ICON_SIZE_LOGICAL: f32 = 16.0;
pub const CARD_ICON_GAP_LOGICAL: f32 = 6.0;
pub const CARD_TITLE_FONT_SIZE_LOGICAL: f32 = 15.0;
pub const CARD_EXCERPT_FONT_SIZE_LOGICAL: f32 = 13.0;
pub const CARD_METADATA_FONT_SIZE_LOGICAL: f32 = 12.0;
pub const CARD_CORNER_RADIUS_LOGICAL: f32 = 8.0;
pub const CARD_METADATA_GAP_LOGICAL: f32 = 12.0;
pub const VIRTUAL_CARD_OVERSCAN_COUNT: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardGeometry {
    pub card_rect: Rect,
    /// 为图标预留的固定位置；没有图标的卡片仍保留该位置以保证标题对齐。
    pub icon_rect: Rect,
    pub title_rect: Rect,
    pub title_baseline: f32,
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
    card_height_px: f32,
    card_stride_px: f32,
    scroll_offset_px: f32,
    dpi: f32,
}

impl VirtualCardListLayout {
    pub fn card_geometry(&self, index: usize) -> CardGeometry {
        debug_assert!(index < self.card_count, "card geometry requires an in-range card index");
        let card_rect = Rect::new(
            self.viewport_rect.x,
            self.viewport_rect.y + index as f32 * self.card_stride_px - self.scroll_offset_px,
            self.viewport_rect.w,
            self.card_height_px,
        );
        let horizontal_padding = CARD_HORIZONTAL_PADDING_LOGICAL * self.dpi;
        let vertical_padding = CARD_VERTICAL_PADDING_LOGICAL * self.dpi;
        let icon_size = CARD_ICON_SIZE_LOGICAL * self.dpi;
        let icon_rect = Rect::new(
            card_rect.x + horizontal_padding,
            card_rect.y + vertical_padding,
            icon_size,
            icon_size,
        );
        let title_x = icon_rect.right() + CARD_ICON_GAP_LOGICAL * self.dpi;
        let title_font_size = CARD_TITLE_FONT_SIZE_LOGICAL * self.dpi;
        let excerpt_font_size = CARD_EXCERPT_FONT_SIZE_LOGICAL * self.dpi;
        let metadata_font_size = CARD_METADATA_FONT_SIZE_LOGICAL * self.dpi;
        let title_y = card_rect.y + vertical_padding;
        let excerpt_y = title_y + title_font_size + 8.0 * self.dpi;
        let metadata_y = card_rect.bottom() - vertical_padding - metadata_font_size;
        let text_right = card_rect.right() - horizontal_padding;
        let metadata_width = (text_right - title_x) * 0.42;
        let metadata_rect =
            Rect::new(title_x, metadata_y, metadata_width.max(0.0), metadata_font_size);

        CardGeometry {
            card_rect,
            icon_rect,
            title_rect: Rect::new(
                title_x,
                title_y,
                (text_right - title_x).max(0.0),
                title_font_size,
            ),
            title_baseline: title_y + title_font_size * 0.8,
            excerpt_rect: Rect::new(
                card_rect.x + horizontal_padding,
                excerpt_y,
                (text_right - card_rect.x - horizontal_padding).max(0.0),
                excerpt_font_size,
            ),
            excerpt_baseline: excerpt_y + excerpt_font_size * 0.8,
            metadata_rect,
            metadata_baseline: metadata_y + metadata_font_size * 0.8,
            tag_rect: Rect::new(
                metadata_rect.right() + CARD_METADATA_GAP_LOGICAL * self.dpi,
                metadata_y,
                (text_right - metadata_rect.right() - CARD_METADATA_GAP_LOGICAL * self.dpi)
                    .max(0.0),
                metadata_font_size,
            ),
        }
    }
}

pub(super) fn build_virtual_card_layout(
    card_count: usize,
    viewport_rect: Rect,
    scroll_offset_px: f32,
    dpi: f32,
) -> VirtualCardListLayout {
    let card_height_px = CARD_HEIGHT_LOGICAL * dpi;
    let card_stride_px = card_height_px + CARD_VERTICAL_GAP_LOGICAL * dpi;
    let content_height_px = card_count as f32 * card_stride_px
        - card_count.checked_sub(1).map(|_| CARD_VERTICAL_GAP_LOGICAL * dpi).unwrap_or(0.0);
    let first_visible =
        ((scroll_offset_px / card_stride_px).floor().max(0.0) as usize).min(card_count);
    let viewport_end = scroll_offset_px + viewport_rect.h;
    let visible_end = (viewport_end / card_stride_px).ceil().max(0.0) as usize;
    let range_start = first_visible.saturating_sub(VIRTUAL_CARD_OVERSCAN_COUNT);
    let range_end =
        visible_end.saturating_add(VIRTUAL_CARD_OVERSCAN_COUNT).min(card_count).max(range_start);
    let visible_range = range_start..range_end;

    VirtualCardListLayout {
        viewport_rect,
        card_count,
        visible_range,
        content_height_px,
        card_height_px,
        card_stride_px,
        scroll_offset_px,
        dpi,
    }
}
