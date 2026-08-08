//! tab_bar/tests.rs — 所有测试。

use super::layout::is_tab_in_clip;
use super::layout::layout_tabs;
use super::*;

use crate::core::geom::Rect;
use crate::settings::Settings;
use crate::widgets::popup_menu::{ContextMenuAction, PopupMenu, PopupMenuAction};
use std::path::Path;

/// Helper: create `count` unpinned TabInfo fixtures.
fn sample_tabs(count: usize) -> Vec<TabInfo> {
    (0..count)
        .map(|index| TabInfo {
            title: format!("tab-{index}"),
            file_path: None,
            is_dirty: false,
            pinned: false,
            language: String::new(),
        })
        .collect()
}

fn hit_test(x: f32, y: f32, layout: &TabBarLayout) -> Option<TabHit> {
    let mut state = TabBarState::new();
    state.set_layout_raw(layout.clone());
    state.hit_test_px(x, y)
}

#[cfg(test)]
mod tests {

    use super::layout::{clamp_tab_scroll, compute_disambiguation, layout_tabs, max_tab_scroll};
    use super::*;

    fn test_ctx() -> TabBarCtx {
        TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 }
    }

    #[test]
    fn tab_info_pinned_is_the_only_layout_source() {
        let mut tabs = sample_tabs(3);
        tabs[1].pinned = true;
        let ctx = test_ctx();

        let layout = layout_tabs(&tabs, 0, &ctx, tab_bar_height(ctx.dpi), false, false, 0.0, None);

        assert_eq!(
            layout
                .tabs
                .iter()
                .filter(|entry| entry.pinned)
                .map(|entry| entry.index)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn disambig_no_collision() {
        let paths: Vec<Option<&Path>> =
            vec![Some(Path::new("/a/main.rs")), Some(Path::new("/b/lib.rs"))];
        let result = compute_disambiguation(&paths);
        assert_eq!(result, vec![None, None]);
    }

    #[test]
    fn disambig_two_same_filename() {
        let paths: Vec<Option<&Path>> =
            vec![Some(Path::new("/a/README.md")), Some(Path::new("/b/README.md"))];
        let result = compute_disambiguation(&paths);
        assert_eq!(result, vec![Some("a".into()), Some("b".into())]);
    }

    #[test]
    fn disambig_three_same_parent_requires_grandparent() {
        let paths: Vec<Option<&Path>> = vec![
            Some(Path::new("/x/a/README.md")),
            Some(Path::new("/y/a/README.md")),
            Some(Path::new("/z/a/README.md")),
        ];
        let result = compute_disambiguation(&paths);
        // All have same filename "README.md" and same parent "a",
        // so need grandparent: "x/a", "y/a", "z/a"
        assert_eq!(result, vec![Some("x/a".into()), Some("y/a".into()), Some("z/a".into()),]);
    }

    #[test]
    fn disambig_mixed_depths() {
        let paths: Vec<Option<&Path>> = vec![
            Some(Path::new("/a/README.md")),
            Some(Path::new("/b/README.md")),
            Some(Path::new("/c/main.rs")), // unique filename
        ];
        let result = compute_disambiguation(&paths);
        assert_eq!(result, vec![Some("a".into()), Some("b".into()), None,]);
    }

    #[test]
    fn disambig_none_path() {
        let paths: Vec<Option<&Path>> = vec![None, Some(Path::new("/a/main.rs"))];
        let result = compute_disambiguation(&paths);
        assert_eq!(result, vec![None, None]);
    }

    // ── TabIndicator tests ──

    #[test]
    fn indicator_clean() {
        assert_eq!(TabIndicator::for_doc(false, false), TabIndicator::None);
    }

    #[test]
    fn indicator_dirty() {
        assert_eq!(TabIndicator::for_doc(true, false), TabIndicator::Dirty);
    }

    #[test]
    fn indicator_conflict_priority() {
        // Conflict takes priority over dirty when both are true
        // NOTE: currently conflict arg is ignored (TabInfo doesn't expose it);
        // this test documents the expected future behavior.
        assert_eq!(TabIndicator::for_doc(true, true), TabIndicator::Dirty); // placeholder
    }

    // ── Context menu hit_test ──

    #[test]
    fn context_menu_hit_first_item() {
        let _settings = Settings::test_default();
        let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
        let pm = PopupMenu::context_px(0, (200.0, 150.0), (ctx.screen_w, ctx.screen_h), false, 1.0);
        let action = pm.hit_test_px(240.0, 156.0);
        assert!(matches!(
            action,
            Some(PopupMenuAction::Context { action: ContextMenuAction::Close, .. })
        ));
    }

    #[test]
    fn context_menu_hit_last_item() {
        let _settings = Settings::test_default();
        let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
        let pm = PopupMenu::context_px(0, (200.0, 150.0), (ctx.screen_w, ctx.screen_h), false, 1.0);
        let action = pm.hit_test_px(240.0, 315.0);
        assert!(matches!(
            action,
            Some(PopupMenuAction::Context { action: ContextMenuAction::TogglePin, .. })
        ));
    }

    #[test]
    fn context_menu_hit_outside() {
        let _settings = Settings::test_default();
        let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
        let pm = PopupMenu::context_px(0, (200.0, 150.0), (ctx.screen_w, ctx.screen_h), false, 1.0);
        assert!(pm.hit_test_px(760.0, 30.0).is_none());
        assert!(pm.hit_test_px(40.0, 570.0).is_none());
    }

    #[test]
    fn context_menu_pin_label_toggles() {
        let _settings = Settings::test_default();
        let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
        let pm_pinned =
            PopupMenu::context_px(0, (200.0, 150.0), (ctx.screen_w, ctx.screen_h), true, 1.0);
        let pm_unpinned =
            PopupMenu::context_px(0, (200.0, 150.0), (ctx.screen_w, ctx.screen_h), false, 1.0);
        assert!(pm_pinned.items.iter().any(|i| i.label == "取消固定"));
        assert!(pm_unpinned.items.iter().any(|i| i.label == "固定标签"));
    }

    // ── Scroll tests ──

    #[test]
    fn max_scroll_zero_for_empty() {
        let _settings = Settings::test_default();
        let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
        assert_eq!(max_tab_scroll(0, &ctx, 32.0), 0.0);
    }

    #[test]
    fn max_scroll_zero_when_fit() {
        // 2 tabs at ~310px each = ~620px, fits in 800px screen
        let _settings = Settings::test_default();
        let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
        let scroll = max_tab_scroll(2, &ctx, 32.0);
        assert!(scroll < 10.0); // very small or zero
    }

    #[test]
    fn max_scroll_positive_when_overflow() {
        // 10 tabs at ~200px each = ~2000px, doesn't fit in 800px
        let _settings = Settings::test_default();
        let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
        let scroll = max_tab_scroll(10, &ctx, 32.0);
        assert!(scroll > 100.0);
    }

    #[test]
    fn clamp_scroll_in_range() {
        assert_eq!(clamp_tab_scroll(-10.0, 100.0), 0.0);
        assert_eq!(clamp_tab_scroll(50.0, 100.0), 50.0);
        assert_eq!(clamp_tab_scroll(150.0, 100.0), 100.0);
    }
}
// ── Overflow menu tests ──

#[test]
fn overflow_menu_not_empty() {
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 1440.0, screen_h: 900.0, dpi: 1.0 };
    let layout = TabBarLayout {
        tabs: (0..10)
            .map(|i| TabEntry {
                index: i,
                title: format!("file_{}.rs", i),
                indicator: TabIndicator::None,
                disambiguation: None,
                pinned: false,
                preview: false,
                rect_px: Rect::ZERO,
                close_rect_px: Rect::ZERO,
            })
            .collect(),
        overflow: true,
        scroll_offset: 0.0,
        max_scroll: 100.0,
        nav_buttons: NavButtonLayout {
            back_rect_px: Rect::ZERO,
            forward_rect_px: Rect::ZERO,
            back_enabled: false,
            forward_enabled: false,
        },

        clip_left_px: 0.0,
        clip_right_px: 0.0,
        dropdown_rect_px: Rect::new(1389.6, 4.5, 28.8, 27.0),
        overflow_left_rect_px: Rect::ZERO,
        overflow_right_rect_px: Rect::ZERO,
        new_tab_rect_px: Rect::ZERO,
        fade_left_rect_px: Rect::ZERO,
        fade_right_rect_px: Rect::ZERO,
        left_arrow_disabled: false,
        right_arrow_disabled: false,
        pinned_total_width: 0.0,
    };
    let dd = layout.dropdown_rect_px;
    let entries: Vec<crate::widgets::popup_menu::OverflowEntry> = layout
        .tabs
        .iter()
        .map(|e| crate::widgets::popup_menu::OverflowEntry {
            tab_index: e.index,
            title: e.title.clone(),
        })
        .collect();
    let menu = crate::widgets::popup_menu::PopupMenu::overflow_px(
        &entries,
        dd,
        (ctx.screen_w, ctx.screen_h),
        0,
        1.0,
    );
    assert!(!menu.items.is_empty(), "overflow menu must have items");
    assert_eq!(menu.items.len(), menu.item_rects.len());
    for (i, rect) in menu.item_rects.iter().enumerate() {
        assert!(rect.w > 0.0, "item {i} rect width <= 0: {rect:?}");
        assert!(rect.h > 0.0, "item {i} rect height <= 0: {rect:?}");
        assert!(rect.x >= 0.0 && rect.right() <= 1440.0, "item {i} out of x range: {rect:?}");
        assert!(rect.y >= 0.0 && rect.bottom() <= 900.0, "item {i} out of y range: {rect:?}");
    }
    // Menu background rect should be valid
    assert!(menu.menu_rect.w > 0.0, "menu bg width <= 0");
    assert!(menu.menu_rect.h > 0.0, "menu bg height <= 0");
}

#[test]
fn hit_test_dropdown_button() {
    let layout = TabBarLayout {
        tabs: vec![TabEntry {
            index: 0,
            title: "test.rs".into(),
            indicator: TabIndicator::None,
            disambiguation: None,
            pinned: false,
            preview: false,
            rect_px: Rect::ZERO,
            close_rect_px: Rect::ZERO,
        }],
        overflow: true,
        scroll_offset: 0.0,
        max_scroll: 0.0,
        nav_buttons: NavButtonLayout {
            back_rect_px: Rect::ZERO,
            forward_rect_px: Rect::ZERO,
            back_enabled: false,
            forward_enabled: false,
        },

        clip_left_px: 0.0,
        clip_right_px: 0.0,
        dropdown_rect_px: Rect::new(1389.6, 4.5, 28.8, 27.0),
        overflow_left_rect_px: Rect::ZERO,
        overflow_right_rect_px: Rect::ZERO,
        new_tab_rect_px: Rect::ZERO,
        fade_left_rect_px: Rect::ZERO,
        fade_right_rect_px: Rect::ZERO,
        left_arrow_disabled: false,
        right_arrow_disabled: false,
        pinned_total_width: 0.0,
    };
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 1440.0, screen_h: 900.0, dpi: 1.0 };
    // Click on the dropdown button center
    let dd_cx_px = (0.93 + 0.97) / 2.0; // NDC x center: 0.95
    let dd_cy_px = (0.99 + 0.93) / 2.0; // NDC y center: 0.96
    let px = (dd_cx_px + 1.0) / 2.0 * ctx.screen_w;
    let py = (1.0 - dd_cy_px) / 2.0 * ctx.screen_h;
    let hit = hit_test(px, py, &layout);
    assert_eq!(hit, Some(TabHit::Dropdown), "Expected Dropdown hit, got {:?}", hit);
}

// ── Pinned tab scroll behavior tests ──

#[test]
fn pinned_tabs_stay_fixed_while_others_scroll() {
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 400.0, screen_h: 600.0, dpi: 1.0 };
    let mut tabs = sample_tabs(8);
    tabs[0].pinned = true;
    tabs[1].pinned = true;

    // Layout at scroll_offset = 0
    let layout0 = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);
    let pinned_x0: Vec<f32> =
        layout0.tabs.iter().filter(|t| t.pinned).map(|t| t.rect_px.x).collect();

    // Layout at scroll_offset = 50
    let layout1 = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 50.0, None);
    let pinned_x1: Vec<f32> =
        layout1.tabs.iter().filter(|t| t.pinned).map(|t| t.rect_px.x).collect();

    // Pinned tabs should have identical positions regardless of scroll
    assert_eq!(pinned_x0, pinned_x1, "pinned tab positions must not change with scroll");

    // Non-pinned tabs should shift left with scroll
    let unpinned0: Vec<f32> =
        layout0.tabs.iter().filter(|t| !t.pinned).map(|t| t.rect_px.x).collect();
    let unpinned1: Vec<f32> =
        layout1.tabs.iter().filter(|t| !t.pinned).map(|t| t.rect_px.x).collect();
    for (a, b) in unpinned0.iter().zip(unpinned1.iter()) {
        assert!(*b < *a, "non-pinned tabs should scroll left: offset0={}, offset1={}", a, b);
    }
}

#[test]
fn pinned_tabs_start_at_tab_area_beginning() {
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
    let mut tabs = sample_tabs(4);
    tabs[0].pinned = true;
    tabs[1].pinned = true;

    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 100.0, None);
    let first_pinned = layout.tabs.iter().find(|t| t.pinned).unwrap();
    assert!(
        first_pinned.rect_px.x >= 0.0,
        "first pinned tab must start at or after tab area start"
    );
    assert!(
        first_pinned.rect_px.x < 100.0,
        "first pinned tab should be near left edge, got {}",
        first_pinned.rect_px.x
    );
}

#[test]
fn max_scroll_only_counts_non_pinned_tabs() {
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 400.0, screen_h: 600.0, dpi: 1.0 };
    let mut tabs = sample_tabs(6);
    for tab in tabs.iter_mut() {
        tab.pinned = true;
    }

    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);
    let non_pinned_count = layout.tabs.iter().filter(|t| !t.pinned).count();
    assert_eq!(non_pinned_count, 0);
    assert_eq!(layout.max_scroll, 0.0, "all-pinned tabs should have zero max_scroll");
}

// ── Hit test clip boundary tests ──

#[test]
fn hit_test_rejects_scrolled_out_non_pinned_tab() {
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 400.0, screen_h: 600.0, dpi: 1.0 };
    let tabs = sample_tabs(8);

    // Large scroll so tabs are far to the left, past clip_left_px
    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 500.0, None);

    let first_tab = &layout.tabs[0];
    let click_x = first_tab.rect_px.x + 5.0;
    let click_y = first_tab.rect_px.y + 5.0;

    if first_tab.rect_px.right() < layout.clip_left_px {
        let hit = hit_test(click_x, click_y, &layout);
        assert!(hit.is_none(), "scrolled-out tab should not be clickable, got {:?}", hit);
    }
}

#[test]
fn hit_test_accepts_visible_non_pinned_tab() {
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
    let tabs = sample_tabs(3);

    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);
    let first_tab = &layout.tabs[0];

    let click_x = first_tab.rect_px.x + 5.0;
    let click_y = first_tab.rect_px.y + 5.0;
    let hit = hit_test(click_x, click_y, &layout);
    assert_eq!(hit, Some(TabHit::Tab(0)), "visible tab should be clickable");
}

#[test]
fn hit_test_accepts_pinned_tab_even_near_edge() {
    let _settings = Settings::test_default();
    let ctx = TabBarCtx { screen_w: 400.0, screen_h: 600.0, dpi: 1.0 };
    let mut tabs = sample_tabs(2);
    tabs[0].pinned = true;

    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);
    let pinned_tab = &layout.tabs[0];

    let click_x = pinned_tab.rect_px.x + 5.0;
    let click_y = pinned_tab.rect_px.y + 5.0;
    let hit = hit_test(click_x, click_y, &layout);
    assert_eq!(hit, Some(TabHit::Tab(0)), "pinned tab should always be clickable");
}

// ── is_tab_in_clip tests ──

#[test]
fn is_tab_in_clip_within_bounds() {
    let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
    let tabs = sample_tabs(3);
    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);
    let x = layout.tabs[0].rect_px.x + 5.0;
    assert!(is_tab_in_clip(x, &layout), "tab within clip bounds should return true");
}

#[test]
fn is_tab_in_clip_outside_left() {
    let ctx = TabBarCtx { screen_w: 400.0, screen_h: 600.0, dpi: 1.0 };
    let tabs = sample_tabs(8);
    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 500.0, None);
    assert!(!is_tab_in_clip(-10.0, &layout), "point left of clip should return false");
}

#[test]
fn is_tab_in_clip_outside_right() {
    let ctx = TabBarCtx { screen_w: 400.0, screen_h: 600.0, dpi: 1.0 };
    let tabs = sample_tabs(3);
    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);
    assert!(!is_tab_in_clip(9999.0, &layout), "point right of clip should return false");
}

// ── Pinned tab width optimization tests ──

#[test]
fn pinned_tab_narrower_than_normal_tab() {
    let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
    let mut tabs = sample_tabs(2);
    tabs[0].pinned = true;

    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);

    let pinned_w = layout.tabs.iter().find(|t| t.pinned).unwrap().rect_px.w;
    let normal_w = layout.tabs.iter().find(|t| !t.pinned).unwrap().rect_px.w;
    assert!(
        pinned_w < normal_w,
        "pinned tab ({pinned_w}px) should be narrower than normal tab ({normal_w}px)"
    );
}

#[test]
fn pinned_total_width_has_no_trailing_gap() {
    let ctx = TabBarCtx { screen_w: 800.0, screen_h: 600.0, dpi: 1.0 };
    let mut tabs = sample_tabs(2);
    tabs[0].pinned = true;

    let layout = layout_tabs(&tabs, 0, &ctx, 32.0, false, false, 0.0, None);

    let pinned_tab = layout.tabs.iter().find(|t| t.pinned).unwrap();
    let normal_tab = layout.tabs.iter().find(|t| !t.pinned).unwrap();

    assert_eq!(
        layout.pinned_total_width, pinned_tab.rect_px.w,
        "pinned_total_width should equal pinned tab width, no trailing gap"
    );

    let expected_start = pinned_tab.rect_px.x + pinned_tab.rect_px.w;
    assert!(
        (normal_tab.rect_px.x - expected_start).abs() < 0.5,
        "non-pinned tab should start at pinned_right (no trailing gap), got {} expected {}",
        normal_tab.rect_px.x,
        expected_start
    );
}
