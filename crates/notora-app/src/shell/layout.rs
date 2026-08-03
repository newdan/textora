use ui::Rect;

use crate::{CompactContent, CompactNavigation, ResponsiveLayoutMode};

pub const DEFAULT_NAVIGATION_WIDTH_LOGICAL: f32 = 220.0;
pub const DEFAULT_CARD_LIST_WIDTH_LOGICAL: f32 = 340.0;
pub const MINIMUM_NAVIGATION_WIDTH_LOGICAL: f32 = 180.0;
pub const MAXIMUM_NAVIGATION_WIDTH_LOGICAL: f32 = 320.0;
pub const MINIMUM_CARD_LIST_WIDTH_LOGICAL: f32 = 260.0;
pub const MAXIMUM_CARD_LIST_WIDTH_LOGICAL: f32 = 520.0;
pub const MINIMUM_EDITOR_WIDTH_LOGICAL: f32 = 300.0;
pub const SPLITTER_WIDTH_LOGICAL: f32 = 8.0;
pub const MINIMUM_WINDOW_WIDTH_LOGICAL: f32 = DEFAULT_NAVIGATION_WIDTH_LOGICAL
    + DEFAULT_CARD_LIST_WIDTH_LOGICAL
    + MINIMUM_EDITOR_WIDTH_LOGICAL
    + SPLITTER_WIDTH_LOGICAL * 2.0;
pub const MINIMUM_WINDOW_HEIGHT_LOGICAL: f32 = 600.0;

/// 三栏布局的纯输入；宽度以逻辑像素持久化，窗口尺寸以物理像素传入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayoutInput {
    pub window_width_px: f32,
    pub window_height_px: f32,
    pub dpi: f32,
    pub navigation_width_logical: f32,
    pub card_list_width_logical: f32,
    pub compact_content: CompactContent,
    pub compact_navigation: CompactNavigation,
}

/// 一帧 shell 的独立区域。overlay、menu 与 tooltip 位于 editor 之后绘制。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShellLayout {
    pub responsive_mode: ResponsiveLayoutMode,
    pub dpi: f32,
    pub navigation_rect: Rect,
    pub navigation_splitter_rect: Rect,
    pub card_list_rect: Rect,
    pub card_list_splitter_rect: Rect,
    pub editor_rect: Rect,
    pub overlay_rect: Rect,
    pub menu_rect: Rect,
    pub tooltip_rect: Rect,
    pub navigation_width_logical: f32,
    pub card_list_width_logical: f32,
}

impl ShellLayout {
    pub fn compute(input: ShellLayoutInput) -> Self {
        let dpi = input.dpi.max(1.0);
        let window_rect =
            Rect::new(0.0, 0.0, input.window_width_px.max(0.0), input.window_height_px.max(0.0));
        let navigation_width_logical = input
            .navigation_width_logical
            .clamp(MINIMUM_NAVIGATION_WIDTH_LOGICAL, MAXIMUM_NAVIGATION_WIDTH_LOGICAL);
        let requested_card_width_logical = input
            .card_list_width_logical
            .clamp(MINIMUM_CARD_LIST_WIDTH_LOGICAL, MAXIMUM_CARD_LIST_WIDTH_LOGICAL);
        let splitter_width_px = SPLITTER_WIDTH_LOGICAL * dpi;
        let minimum_three_pane_width_px = (navigation_width_logical
            + requested_card_width_logical
            + MINIMUM_EDITOR_WIDTH_LOGICAL
            + SPLITTER_WIDTH_LOGICAL * 2.0)
            * dpi;

        if window_rect.w >= minimum_three_pane_width_px {
            return Self::three_pane(
                window_rect,
                dpi,
                navigation_width_logical,
                requested_card_width_logical,
                splitter_width_px,
            );
        }
        if window_rect.w
            >= (requested_card_width_logical
                + MINIMUM_EDITOR_WIDTH_LOGICAL
                + SPLITTER_WIDTH_LOGICAL)
                * dpi
        {
            return Self::navigation_overlay(
                window_rect,
                dpi,
                requested_card_width_logical,
                splitter_width_px,
                input.compact_navigation,
            );
        }
        Self::editor_overlay(
            window_rect,
            dpi,
            requested_card_width_logical,
            input.compact_content,
            input.compact_navigation,
        )
    }

    fn three_pane(
        window_rect: Rect,
        dpi: f32,
        navigation_width_logical: f32,
        requested_card_width_logical: f32,
        splitter_width_px: f32,
    ) -> Self {
        let navigation_width_px = navigation_width_logical * dpi;
        let card_width_px = requested_card_width_logical * dpi;
        let navigation_rect = Rect::new(0.0, 0.0, navigation_width_px, window_rect.h);
        let navigation_splitter_rect =
            Rect::new(navigation_rect.right(), 0.0, splitter_width_px, window_rect.h);
        let card_list_rect =
            Rect::new(navigation_splitter_rect.right(), 0.0, card_width_px, window_rect.h);
        let card_list_splitter_rect =
            Rect::new(card_list_rect.right(), 0.0, splitter_width_px, window_rect.h);
        let editor_rect = Rect::new(
            card_list_splitter_rect.right(),
            0.0,
            (window_rect.right() - card_list_splitter_rect.right()).max(0.0),
            window_rect.h,
        );
        Self {
            responsive_mode: ResponsiveLayoutMode::ThreePane,
            dpi,
            navigation_rect,
            navigation_splitter_rect,
            card_list_rect,
            card_list_splitter_rect,
            editor_rect,
            overlay_rect: window_rect,
            menu_rect: Rect::ZERO,
            tooltip_rect: Rect::ZERO,
            navigation_width_logical,
            card_list_width_logical: card_width_px / dpi,
        }
    }

    fn navigation_overlay(
        window_rect: Rect,
        dpi: f32,
        requested_card_width_logical: f32,
        splitter_width_px: f32,
        compact_navigation: CompactNavigation,
    ) -> Self {
        let card_width_px = requested_card_width_logical * dpi;
        let card_list_rect = Rect::new(0.0, 0.0, card_width_px, window_rect.h);
        let card_list_splitter_rect = Rect::new(
            card_list_rect.right(),
            0.0,
            splitter_width_px.min(window_rect.w),
            window_rect.h,
        );
        let editor_rect = Rect::new(
            card_list_splitter_rect.right(),
            0.0,
            (window_rect.right() - card_list_splitter_rect.right()).max(0.0),
            window_rect.h,
        );
        Self {
            responsive_mode: ResponsiveLayoutMode::NavigationOverlay,
            dpi,
            navigation_rect: compact_navigation_rect(window_rect, dpi, compact_navigation),
            navigation_splitter_rect: Rect::ZERO,
            card_list_rect,
            card_list_splitter_rect,
            editor_rect,
            overlay_rect: window_rect,
            menu_rect: Rect::ZERO,
            tooltip_rect: Rect::ZERO,
            navigation_width_logical: DEFAULT_NAVIGATION_WIDTH_LOGICAL,
            card_list_width_logical: requested_card_width_logical,
        }
    }

    fn editor_overlay(
        window_rect: Rect,
        dpi: f32,
        requested_card_width_logical: f32,
        compact_content: CompactContent,
        compact_navigation: CompactNavigation,
    ) -> Self {
        let (card_list_rect, editor_rect) = match compact_content {
            CompactContent::CardList => (window_rect, Rect::ZERO),
            CompactContent::Editor => (Rect::ZERO, window_rect),
        };
        Self {
            responsive_mode: ResponsiveLayoutMode::EditorOverlay,
            dpi,
            navigation_rect: compact_navigation_rect(window_rect, dpi, compact_navigation),
            navigation_splitter_rect: Rect::ZERO,
            card_list_rect,
            card_list_splitter_rect: Rect::ZERO,
            editor_rect,
            overlay_rect: window_rect,
            menu_rect: Rect::ZERO,
            tooltip_rect: Rect::ZERO,
            navigation_width_logical: DEFAULT_NAVIGATION_WIDTH_LOGICAL,
            card_list_width_logical: requested_card_width_logical.min(window_rect.w / dpi),
        }
    }
}

fn compact_navigation_rect(
    window_rect: Rect,
    dpi: f32,
    compact_navigation: CompactNavigation,
) -> Rect {
    if compact_navigation != CompactNavigation::Visible {
        return Rect::ZERO;
    }
    Rect::new(
        window_rect.x,
        window_rect.y,
        (MAXIMUM_NAVIGATION_WIDTH_LOGICAL * dpi).min(window_rect.w),
        window_rect.h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(width: f32, dpi: f32) -> ShellLayoutInput {
        ShellLayoutInput {
            window_width_px: width,
            window_height_px: MINIMUM_WINDOW_HEIGHT_LOGICAL * dpi,
            dpi,
            navigation_width_logical: DEFAULT_NAVIGATION_WIDTH_LOGICAL,
            card_list_width_logical: DEFAULT_CARD_LIST_WIDTH_LOGICAL,
            compact_content: CompactContent::CardList,
            compact_navigation: CompactNavigation::Hidden,
        }
    }

    fn assert_non_negative(layout: ShellLayout) {
        for rect in [
            layout.navigation_rect,
            layout.navigation_splitter_rect,
            layout.card_list_rect,
            layout.card_list_splitter_rect,
            layout.editor_rect,
            layout.overlay_rect,
        ] {
            assert!(rect.w >= 0.0 && rect.h >= 0.0, "rect must not be negative: {rect:?}");
        }
    }

    #[test]
    fn default_minimum_window_uses_three_panes_without_editor_overlap() {
        let layout = ShellLayout::compute(input(880.0, 1.0));

        assert_eq!(layout.responsive_mode, ResponsiveLayoutMode::ThreePane);
        assert_eq!(layout.navigation_rect.w, DEFAULT_NAVIGATION_WIDTH_LOGICAL);
        assert_eq!(layout.card_list_rect.w, DEFAULT_CARD_LIST_WIDTH_LOGICAL);
        assert!(layout.editor_rect.x >= layout.card_list_splitter_rect.right());
    }

    #[test]
    fn minimum_window_width_matches_default_fixed_panes_and_editor_minimum() {
        assert_eq!(MINIMUM_WINDOW_WIDTH_LOGICAL, 876.0);
    }

    #[test]
    fn configured_side_panes_are_not_shrunk_to_force_three_pane_mode() {
        let compact_layout = ShellLayout::compute(input(800.0, 1.0));
        let three_pane_layout = ShellLayout::compute(input(880.0, 1.0));

        assert_eq!(compact_layout.responsive_mode, ResponsiveLayoutMode::NavigationOverlay);
        assert_eq!(compact_layout.card_list_rect.w, DEFAULT_CARD_LIST_WIDTH_LOGICAL);
        assert_eq!(three_pane_layout.responsive_mode, ResponsiveLayoutMode::ThreePane);
        assert_eq!(three_pane_layout.navigation_rect.w, DEFAULT_NAVIGATION_WIDTH_LOGICAL);
        assert_eq!(three_pane_layout.card_list_rect.w, DEFAULT_CARD_LIST_WIDTH_LOGICAL);
        assert!(three_pane_layout.editor_rect.w >= MINIMUM_EDITOR_WIDTH_LOGICAL);
    }

    #[test]
    fn high_dpi_preserves_logical_widths() {
        let layout = ShellLayout::compute(input(1760.0, 2.0));

        assert_eq!(layout.navigation_rect.w, DEFAULT_NAVIGATION_WIDTH_LOGICAL * 2.0);
        assert_eq!(layout.card_list_rect.w, DEFAULT_CARD_LIST_WIDTH_LOGICAL * 2.0);
        assert_eq!(layout.navigation_width_logical, DEFAULT_NAVIGATION_WIDTH_LOGICAL);
    }

    #[test]
    fn narrow_windows_switch_modes_without_negative_rects() {
        let navigation_overlay = ShellLayout::compute(input(700.0, 1.0));
        let editor_overlay = ShellLayout::compute(input(400.0, 1.0));

        assert_eq!(navigation_overlay.responsive_mode, ResponsiveLayoutMode::NavigationOverlay);
        assert_eq!(editor_overlay.responsive_mode, ResponsiveLayoutMode::EditorOverlay);
        assert_non_negative(navigation_overlay);
        assert_non_negative(editor_overlay);
    }

    #[test]
    fn responsive_layout_uses_editor_or_cards_and_can_overlay_navigation() {
        let mut compact_input = input(400.0, 1.0);
        compact_input.compact_content = CompactContent::Editor;
        compact_input.compact_navigation = CompactNavigation::Visible;

        let layout = ShellLayout::compute(compact_input);

        assert_eq!(layout.responsive_mode, ResponsiveLayoutMode::EditorOverlay);
        assert_eq!(layout.card_list_rect, Rect::ZERO);
        assert_eq!(layout.editor_rect.w, 400.0);
        assert!(layout.navigation_rect.w > 0.0);
    }

    #[test]
    fn splitter_width_round_trip_keeps_logical_precision() {
        let mut layout_input = input(1800.0, 1.5);
        layout_input.navigation_width_logical = 247.25;
        layout_input.card_list_width_logical = 401.5;
        let layout = ShellLayout::compute(layout_input);

        assert_eq!(layout.navigation_width_logical, 247.25);
        assert_eq!(layout.card_list_width_logical, 401.5);
    }
}
