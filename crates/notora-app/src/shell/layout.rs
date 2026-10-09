use ui::Rect;

use crate::{CompactContent, CompactNavigation, NavigationPaneVisibility, ResponsiveLayoutMode};

pub const DEFAULT_NAVIGATION_WIDTH_LOGICAL: f32 = 220.0;
pub const CONTENT_SURFACE_INSET_LOGICAL: f32 = 4.0;
pub const DEFAULT_CARD_LIST_WIDTH_LOGICAL: f32 = 340.0;
pub const MINIMUM_NAVIGATION_WIDTH_LOGICAL: f32 = 180.0;
pub const MAXIMUM_NAVIGATION_WIDTH_LOGICAL: f32 = 320.0;
pub const MINIMUM_CARD_LIST_WIDTH_LOGICAL: f32 = 260.0;
pub const MAXIMUM_CARD_LIST_WIDTH_LOGICAL: f32 = 520.0;
pub const MINIMUM_EDITOR_WIDTH_LOGICAL: f32 = 300.0;
/// 仅作为可点击、可拖动的命中范围，不占用栏间视觉宽度。
pub const SPLITTER_WIDTH_LOGICAL: f32 = 8.0;
pub const EDITOR_HEADER_HEIGHT_LOGICAL: f32 = 92.0;
pub const EDITOR_COMPACT_HEADER_HEIGHT_LOGICAL: f32 = 92.0;
/// 头部底部的属性行（所属工作区 + 标签）高度；仅工作区笔记展示该行。
pub const EDITOR_HEADER_PROPERTY_ROW_HEIGHT_LOGICAL: f32 = 28.0;
pub const EDITOR_TOOLBAR_HEIGHT_LOGICAL: f32 = 36.0;
pub const EDITOR_COMPACT_HEIGHT_THRESHOLD_LOGICAL: f32 = 480.0;
pub const MINIMUM_WINDOW_WIDTH_LOGICAL: f32 = DEFAULT_NAVIGATION_WIDTH_LOGICAL
    + DEFAULT_CARD_LIST_WIDTH_LOGICAL
    + MINIMUM_EDITOR_WIDTH_LOGICAL;
pub const MINIMUM_WINDOW_HEIGHT_LOGICAL: f32 = 600.0;

/// 三栏布局的纯输入；宽度以逻辑像素持久化，窗口尺寸以物理像素传入。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellLayoutInput {
    pub window_width_px: f32,
    pub window_height_px: f32,
    pub dpi: f32,
    pub navigation_width_logical: f32,
    pub card_list_width_logical: f32,
    pub navigation_pane_visibility: NavigationPaneVisibility,
    pub compact_content: CompactContent,
    pub compact_navigation: CompactNavigation,
    /// 头部属性行（所属工作区 + 标签）是否展示；不展示时头部收回该行高度。
    pub editor_property_row_visible: bool,
    /// 编辑器头部（标题/时间/保存状态）是否展示；外部文件只保留工具条与正文。
    pub editor_header_visible: bool,
}

#[derive(Clone, Copy)]
struct EditorChromeVisibility {
    property_row: bool,
    header: bool,
}

/// 一帧 shell 的独立区域。overlay 与 menu 位于 editor 之后绘制。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShellLayout {
    pub responsive_mode: ResponsiveLayoutMode,
    pub dpi: f32,
    pub navigation_rect: Rect,
    pub navigation_splitter_rect: Rect,
    pub card_list_rect: Rect,
    pub card_list_splitter_rect: Rect,
    pub editor_rect: Rect,
    pub editor_header_rect: Rect,
    pub editor_toolbar_rect: Rect,
    pub editor_body_rect: Rect,
    pub overlay_rect: Rect,
    pub menu_rect: Rect,
    pub navigation_width_logical: f32,
    pub card_list_width_logical: f32,
}

impl ShellLayout {
    /// 标题栏由 UI 单独绘制；所有产品区域和弹窗均使用其下方的坐标空间。
    pub fn compute_below_title_bar(mut input: ShellLayoutInput, title_height_px: f32) -> Self {
        let title_height_px = title_height_px.clamp(0.0, input.window_height_px.max(0.0));
        input.window_height_px = (input.window_height_px - title_height_px).max(0.0);
        let mut layout = Self::compute(input);
        for rect in [
            &mut layout.navigation_rect,
            &mut layout.navigation_splitter_rect,
            &mut layout.card_list_rect,
            &mut layout.card_list_splitter_rect,
            &mut layout.editor_rect,
            &mut layout.editor_header_rect,
            &mut layout.editor_toolbar_rect,
            &mut layout.editor_body_rect,
            &mut layout.overlay_rect,
            &mut layout.menu_rect,
        ] {
            if *rect != Rect::ZERO {
                rect.y += title_height_px;
            }
        }
        if title_height_px > 0.0 {
            layout.inset_content_surface(input);
        }
        layout
    }

    pub fn has_immersive_title_bar(&self) -> bool {
        self.overlay_rect.y > 0.0
    }

    pub fn content_surface_rect(&self) -> Rect {
        if self.card_list_rect == Rect::ZERO {
            return self.editor_rect;
        }
        if self.editor_rect == Rect::ZERO {
            return self.card_list_rect;
        }
        Rect::new(
            self.card_list_rect.x,
            self.card_list_rect.y,
            self.editor_rect.right() - self.card_list_rect.x,
            self.card_list_rect.h,
        )
    }

    fn inset_content_surface(&mut self, input: ShellLayoutInput) {
        let inset = CONTENT_SURFACE_INSET_LOGICAL * self.dpi;
        let right = (input.window_width_px - inset).max(0.0);
        for rect in [&mut self.card_list_rect, &mut self.editor_rect] {
            if *rect == Rect::ZERO {
                continue;
            }
            let original_right = rect.right().min(right);
            rect.x = rect.x.max(inset.min(original_right));
            rect.w = (original_right - rect.x).max(0.0);
            rect.h = (rect.h - inset).max(0.0);
        }
        for rect in [&mut self.navigation_splitter_rect, &mut self.card_list_splitter_rect] {
            rect.h = (rect.h - inset).max(0.0);
        }
        (self.editor_header_rect, self.editor_toolbar_rect, self.editor_body_rect) =
            editor_chrome_rects(
                self.editor_rect,
                self.dpi,
                EditorChromeVisibility {
                    property_row: input.editor_property_row_visible,
                    header: input.editor_header_visible,
                },
            );
    }

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
            + MINIMUM_EDITOR_WIDTH_LOGICAL)
            * dpi;
        let editor_chrome_visibility = EditorChromeVisibility {
            property_row: input.editor_property_row_visible,
            header: input.editor_header_visible,
        };

        if window_rect.w >= minimum_three_pane_width_px {
            return Self::three_pane(
                window_rect,
                dpi,
                navigation_width_logical,
                requested_card_width_logical,
                splitter_width_px,
                input.navigation_pane_visibility,
                editor_chrome_visibility,
            );
        }
        if window_rect.w >= (requested_card_width_logical + MINIMUM_EDITOR_WIDTH_LOGICAL) * dpi {
            return Self::navigation_overlay(
                window_rect,
                dpi,
                requested_card_width_logical,
                splitter_width_px,
                input.compact_navigation,
                editor_chrome_visibility,
            );
        }
        Self::editor_overlay(
            window_rect,
            dpi,
            requested_card_width_logical,
            input.compact_content,
            input.compact_navigation,
            editor_chrome_visibility,
        )
    }

    fn three_pane(
        window_rect: Rect,
        dpi: f32,
        navigation_width_logical: f32,
        requested_card_width_logical: f32,
        splitter_width_px: f32,
        navigation_pane_visibility: NavigationPaneVisibility,
        editor_chrome_visibility: EditorChromeVisibility,
    ) -> Self {
        let navigation_width_px = match navigation_pane_visibility {
            NavigationPaneVisibility::Expanded => navigation_width_logical * dpi,
            NavigationPaneVisibility::Collapsed => 0.0,
        };
        let card_width_px = requested_card_width_logical * dpi;
        let navigation_rect = match navigation_pane_visibility {
            NavigationPaneVisibility::Expanded => {
                Rect::new(0.0, 0.0, navigation_width_px, window_rect.h)
            }
            NavigationPaneVisibility::Collapsed => Rect::ZERO,
        };
        let navigation_splitter_rect = match navigation_pane_visibility {
            NavigationPaneVisibility::Expanded => {
                centered_splitter_rect(navigation_rect.right(), window_rect.h, splitter_width_px)
            }
            NavigationPaneVisibility::Collapsed => Rect::ZERO,
        };
        let card_list_rect = Rect::new(navigation_rect.right(), 0.0, card_width_px, window_rect.h);
        let card_list_splitter_rect =
            centered_splitter_rect(card_list_rect.right(), window_rect.h, splitter_width_px);
        let editor_rect = Rect::new(
            card_list_rect.right(),
            0.0,
            (window_rect.right() - card_list_rect.right()).max(0.0),
            window_rect.h,
        );
        let (editor_header_rect, editor_toolbar_rect, editor_body_rect) =
            editor_chrome_rects(editor_rect, dpi, editor_chrome_visibility);
        Self {
            responsive_mode: ResponsiveLayoutMode::ThreePane,
            dpi,
            navigation_rect,
            navigation_splitter_rect,
            card_list_rect,
            card_list_splitter_rect,
            editor_rect,
            editor_header_rect,
            editor_toolbar_rect,
            editor_body_rect,
            overlay_rect: window_rect,
            menu_rect: Rect::ZERO,
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
        editor_chrome_visibility: EditorChromeVisibility,
    ) -> Self {
        let card_width_px = requested_card_width_logical * dpi;
        let card_list_rect = Rect::new(0.0, 0.0, card_width_px, window_rect.h);
        let card_list_splitter_rect = centered_splitter_rect(
            card_list_rect.right(),
            window_rect.h,
            splitter_width_px.min(window_rect.w),
        );
        let editor_rect = Rect::new(
            card_list_rect.right(),
            0.0,
            (window_rect.right() - card_list_rect.right()).max(0.0),
            window_rect.h,
        );
        let (editor_header_rect, editor_toolbar_rect, editor_body_rect) =
            editor_chrome_rects(editor_rect, dpi, editor_chrome_visibility);
        Self {
            responsive_mode: ResponsiveLayoutMode::NavigationOverlay,
            dpi,
            navigation_rect: compact_navigation_rect(window_rect, dpi, compact_navigation),
            navigation_splitter_rect: Rect::ZERO,
            card_list_rect,
            card_list_splitter_rect,
            editor_rect,
            editor_header_rect,
            editor_toolbar_rect,
            editor_body_rect,
            overlay_rect: window_rect,
            menu_rect: Rect::ZERO,
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
        editor_chrome_visibility: EditorChromeVisibility,
    ) -> Self {
        let (card_list_rect, editor_rect) = match compact_content {
            CompactContent::CardList => (window_rect, Rect::ZERO),
            CompactContent::Editor => (Rect::ZERO, window_rect),
        };
        let (editor_header_rect, editor_toolbar_rect, editor_body_rect) =
            editor_chrome_rects(editor_rect, dpi, editor_chrome_visibility);
        Self {
            responsive_mode: ResponsiveLayoutMode::EditorOverlay,
            dpi,
            navigation_rect: compact_navigation_rect(window_rect, dpi, compact_navigation),
            navigation_splitter_rect: Rect::ZERO,
            card_list_rect,
            card_list_splitter_rect: Rect::ZERO,
            editor_rect,
            editor_header_rect,
            editor_toolbar_rect,
            editor_body_rect,
            overlay_rect: window_rect,
            menu_rect: Rect::ZERO,
            navigation_width_logical: DEFAULT_NAVIGATION_WIDTH_LOGICAL,
            card_list_width_logical: requested_card_width_logical.min(window_rect.w / dpi),
        }
    }
}

fn centered_splitter_rect(boundary_x: f32, height: f32, hit_width: f32) -> Rect {
    Rect::new(boundary_x - hit_width * 0.5, 0.0, hit_width, height)
}

fn editor_chrome_rects(
    editor_rect: Rect,
    dpi: f32,
    visibility: EditorChromeVisibility,
) -> (Rect, Rect, Rect) {
    if editor_rect.w <= 0.0 || editor_rect.h <= 0.0 {
        return (Rect::ZERO, Rect::ZERO, Rect::ZERO);
    }

    let available_height_logical = editor_rect.h / dpi;
    let mut header_height_logical = if !visibility.header {
        0.0
    } else if available_height_logical < EDITOR_COMPACT_HEIGHT_THRESHOLD_LOGICAL {
        EDITOR_COMPACT_HEADER_HEIGHT_LOGICAL
    } else {
        EDITOR_HEADER_HEIGHT_LOGICAL
    };
    if visibility.header && !visibility.property_row {
        header_height_logical -= EDITOR_HEADER_PROPERTY_ROW_HEIGHT_LOGICAL;
    }
    let header_height_px = (header_height_logical * dpi).min(editor_rect.h);
    let remaining_after_header = (editor_rect.h - header_height_px).max(0.0);
    let toolbar_height_px = (EDITOR_TOOLBAR_HEIGHT_LOGICAL * dpi).min(remaining_after_header);
    let body_height_px = (remaining_after_header - toolbar_height_px).max(0.0);
    let header_rect = Rect::new(editor_rect.x, editor_rect.y, editor_rect.w, header_height_px);
    let toolbar_rect =
        Rect::new(editor_rect.x, header_rect.bottom(), editor_rect.w, toolbar_height_px);
    let body_rect = Rect::new(editor_rect.x, toolbar_rect.bottom(), editor_rect.w, body_height_px);
    (header_rect, toolbar_rect, body_rect)
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
            navigation_pane_visibility: NavigationPaneVisibility::Expanded,
            compact_content: CompactContent::CardList,
            compact_navigation: CompactNavigation::Hidden,
            editor_property_row_visible: true,
            editor_header_visible: true,
        }
    }

    #[test]
    fn immersive_title_bar_reserves_space_in_every_responsive_mode_and_dpi() {
        for dpi in [1.0, 1.5, 2.0] {
            for width in [500.0, 700.0, 1200.0] {
                let mut shell_input = input(width * dpi, dpi);
                shell_input.compact_navigation = CompactNavigation::Visible;
                let title_height = ui::window_frame::WindowFrameState::Restored.title_height(dpi);
                let layout = ShellLayout::compute_below_title_bar(shell_input, title_height);
                for rect in [
                    layout.navigation_rect,
                    layout.card_list_rect,
                    layout.editor_rect,
                    layout.overlay_rect,
                ] {
                    if rect != Rect::ZERO {
                        assert_eq!(rect.y, title_height);
                        let bottom_inset =
                            if rect == layout.card_list_rect || rect == layout.editor_rect {
                                CONTENT_SURFACE_INSET_LOGICAL * dpi
                            } else {
                                0.0
                            };
                        assert_eq!(rect.bottom(), shell_input.window_height_px - bottom_inset);
                    }
                }
                assert_non_negative(layout);
                assert_editor_chrome_is_partitioned(layout);
            }
        }
    }

    #[test]
    fn immersive_content_has_one_shared_outline_when_navigation_is_visible_or_hidden() {
        for dpi in [1.0, 1.5, 2.0] {
            for visibility in
                [NavigationPaneVisibility::Expanded, NavigationPaneVisibility::Collapsed]
            {
                let mut shell_input = input(1200.0 * dpi, dpi);
                shell_input.navigation_pane_visibility = visibility;
                let layout = ShellLayout::compute_below_title_bar(shell_input, 36.0 * dpi);
                let surface = layout.content_surface_rect();
                assert_eq!(surface.x, layout.card_list_rect.x);
                assert_eq!(
                    surface.right(),
                    shell_input.window_width_px - CONTENT_SURFACE_INSET_LOGICAL * dpi
                );
                assert_eq!(
                    surface.bottom(),
                    shell_input.window_height_px - CONTENT_SURFACE_INSET_LOGICAL * dpi
                );
                assert_eq!(layout.card_list_rect.right(), layout.editor_rect.x);
                assert_eq!(layout.card_list_rect.y, layout.editor_rect.y);
                assert_eq!(layout.card_list_rect.bottom(), layout.editor_rect.bottom());
                if visibility == NavigationPaneVisibility::Collapsed {
                    assert_eq!(surface.x, CONTENT_SURFACE_INSET_LOGICAL * dpi);
                } else {
                    assert_eq!(surface.x, layout.navigation_rect.right());
                }
            }
        }
    }

    fn assert_non_negative(layout: ShellLayout) {
        for rect in [
            layout.navigation_rect,
            layout.navigation_splitter_rect,
            layout.card_list_rect,
            layout.card_list_splitter_rect,
            layout.editor_rect,
            layout.editor_header_rect,
            layout.editor_toolbar_rect,
            layout.editor_body_rect,
            layout.overlay_rect,
        ] {
            assert!(rect.w >= 0.0 && rect.h >= 0.0, "rect must not be negative: {rect:?}");
        }
    }

    fn assert_editor_chrome_is_partitioned(layout: ShellLayout) {
        if layout.editor_rect == Rect::ZERO {
            assert_eq!(layout.editor_header_rect, Rect::ZERO);
            assert_eq!(layout.editor_toolbar_rect, Rect::ZERO);
            assert_eq!(layout.editor_body_rect, Rect::ZERO);
            return;
        }
        assert_eq!(layout.editor_header_rect.x, layout.editor_rect.x);
        assert_eq!(layout.editor_header_rect.y, layout.editor_rect.y);
        assert_eq!(layout.editor_header_rect.w, layout.editor_rect.w);
        assert_eq!(layout.editor_toolbar_rect.x, layout.editor_rect.x);
        assert_eq!(layout.editor_toolbar_rect.w, layout.editor_rect.w);
        assert_eq!(layout.editor_body_rect.x, layout.editor_rect.x);
        assert_eq!(layout.editor_body_rect.w, layout.editor_rect.w);
        assert_eq!(layout.editor_header_rect.bottom(), layout.editor_toolbar_rect.y);
        assert_eq!(layout.editor_toolbar_rect.bottom(), layout.editor_body_rect.y);
        assert_eq!(layout.editor_body_rect.bottom(), layout.editor_rect.bottom());
    }

    #[test]
    fn default_minimum_window_uses_three_panes_without_editor_overlap() {
        let layout = ShellLayout::compute(input(880.0, 1.0));

        assert_eq!(layout.responsive_mode, ResponsiveLayoutMode::ThreePane);
        assert_eq!(layout.navigation_rect.w, DEFAULT_NAVIGATION_WIDTH_LOGICAL);
        assert_eq!(layout.card_list_rect.w, DEFAULT_CARD_LIST_WIDTH_LOGICAL);
        assert_eq!(layout.editor_header_rect.h, 92.0);
        assert_eq!(layout.editor_rect.x, layout.card_list_rect.right());
        assert_editor_chrome_is_partitioned(layout);
    }

    #[test]
    fn minimum_window_width_matches_default_fixed_panes_and_editor_minimum() {
        assert_eq!(MINIMUM_WINDOW_WIDTH_LOGICAL, 860.0);
    }

    #[test]
    fn splitter_hit_targets_overlay_adjacent_panes_without_visible_gutters() {
        let layout = ShellLayout::compute(input(880.0, 1.0));

        assert_eq!(layout.navigation_rect.right(), layout.card_list_rect.x);
        assert_eq!(layout.card_list_rect.right(), layout.editor_rect.x);
        assert!(layout.navigation_splitter_rect.x < layout.card_list_rect.x);
        assert!(layout.navigation_splitter_rect.right() > layout.card_list_rect.x);
        assert!(layout.card_list_splitter_rect.x < layout.editor_rect.x);
        assert!(layout.card_list_splitter_rect.right() > layout.editor_rect.x);
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
        assert_editor_chrome_is_partitioned(three_pane_layout);
    }

    #[test]
    fn high_dpi_preserves_logical_widths() {
        let layout = ShellLayout::compute(input(1760.0, 2.0));

        assert_eq!(layout.navigation_rect.w, DEFAULT_NAVIGATION_WIDTH_LOGICAL * 2.0);
        assert_eq!(layout.card_list_rect.w, DEFAULT_CARD_LIST_WIDTH_LOGICAL * 2.0);
        assert_eq!(layout.navigation_width_logical, DEFAULT_NAVIGATION_WIDTH_LOGICAL);
        assert_editor_chrome_is_partitioned(layout);
    }

    #[test]
    fn collapsed_navigation_releases_its_width_and_disables_its_splitter() {
        let mut layout_input = input(880.0, 1.0);
        layout_input.navigation_pane_visibility = NavigationPaneVisibility::Collapsed;

        let layout = ShellLayout::compute(layout_input);

        assert_eq!(layout.responsive_mode, ResponsiveLayoutMode::ThreePane);
        assert_eq!(layout.navigation_rect, Rect::ZERO);
        assert_eq!(layout.navigation_splitter_rect, Rect::ZERO);
        assert_eq!(layout.card_list_rect.x, 0.0);
        assert_eq!(layout.editor_rect.w, 540.0);
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
        assert_editor_chrome_is_partitioned(navigation_overlay);
        assert_editor_chrome_is_partitioned(editor_overlay);
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
        assert_editor_chrome_is_partitioned(layout);
    }

    #[test]
    fn minimum_window_height_keeps_editor_body_as_the_remaining_region() {
        let mut layout_input = input(880.0, 1.0);
        layout_input.window_height_px = 1.0;
        let layout = ShellLayout::compute(layout_input);

        assert_editor_chrome_is_partitioned(layout);
        assert!(layout.editor_body_rect.h >= 0.0);
    }

    #[test]
    fn hidden_editor_property_row_reclaims_its_height_from_the_header() {
        let mut layout_input = input(880.0, 1.0);
        layout_input.editor_property_row_visible = false;
        let layout = ShellLayout::compute(layout_input);

        assert_eq!(
            layout.editor_header_rect.h,
            EDITOR_HEADER_HEIGHT_LOGICAL - EDITOR_HEADER_PROPERTY_ROW_HEIGHT_LOGICAL
        );
        assert_editor_chrome_is_partitioned(layout);
    }

    #[test]
    fn hidden_editor_header_gives_the_toolbar_the_editor_top_edge() {
        let mut layout_input = input(880.0, 1.0);
        layout_input.editor_header_visible = false;
        let layout = ShellLayout::compute(layout_input);

        assert_eq!(layout.editor_header_rect.h, 0.0);
        assert_eq!(layout.editor_toolbar_rect.y, layout.editor_rect.y);
        assert_eq!(layout.editor_toolbar_rect.h, EDITOR_TOOLBAR_HEIGHT_LOGICAL);
        assert_editor_chrome_is_partitioned(layout);
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
