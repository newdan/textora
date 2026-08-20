//! Settings menu builder — standalone free function.

use crate::constants;
use crate::core::Rect;
use crate::view_mode::ViewMode;
use crate::widgets::popup_menu::{PopupMenu, PopupMenuAction as PMA, PopupMenuItem};

use super::types::{NewDocumentKind, SidebarSettingsInput};

/// Build the typed new-document menu below the sidebar split button.
pub fn build_new_document_menu(
    split_button_rect: Rect,
    screen_size: (f32, f32),
    metrics: &crate::settings::UiMetrics,
    include_encrypted_markdown: bool,
) -> PopupMenu {
    let (screen_w, screen_h) = screen_size;
    let dpi = metrics.dpi;
    let item_h = constants::ROW_HEIGHT * dpi;
    let menu_w = 200.0 * dpi;
    let anchor_x = split_button_rect.x + split_button_rect.w / 2.0 - menu_w / 2.0;
    let anchor_y = split_button_rect.bottom() + 2.0 * dpi;
    let mut items = vec![
        PopupMenuItem::action("新建 TXT", PMA::NewDocument(NewDocumentKind::Text)),
        PopupMenuItem::action("新建 MMAP", PMA::NewDocument(NewDocumentKind::Mindmap)),
        PopupMenuItem::action("新建 MD", PMA::NewDocument(NewDocumentKind::Markdown)),
    ];
    if include_encrypted_markdown {
        items.push(PopupMenuItem::action(
            "加密笔记",
            PMA::NewDocument(NewDocumentKind::EncryptedMarkdown),
        ));
    }

    let menu_left = anchor_x.min(screen_w - menu_w).max(0.0);
    let menu_right = menu_left + menu_w;
    let pad_x = 4.0 * dpi;
    let pad_y = 4.0 * dpi;
    let mut top_px = anchor_y + pad_y;
    let mut item_rects = Vec::with_capacity(items.len());
    for item in &items {
        let item_height = if item.is_separator { 8.0 * dpi } else { item_h };
        item_rects.push(Rect::new(
            menu_left + pad_x,
            top_px,
            menu_right - menu_left - pad_x * 2.0,
            item_height,
        ));
        top_px += item_height;
    }
    let menu_h = top_px - anchor_y + pad_y;
    let final_top = if anchor_y + menu_h > screen_h - 4.0 * dpi {
        (anchor_y - menu_h).max(4.0 * dpi)
    } else {
        anchor_y
    };
    let offset = final_top - anchor_y;
    let adjusted_rects: Vec<Rect> =
        item_rects.iter().map(|r| Rect::new(r.x, r.y + offset, r.w, r.h)).collect();

    PopupMenu {
        items,
        item_rects: adjusted_rects,
        menu_rect: Rect::new(menu_left, final_top, menu_right - menu_left, menu_h),
        screen_size,
        show_checkmarks: false,
    }
}

pub fn build_settings_menu(
    settings_btn_rect: Option<crate::core::Rect>,
    input: &SidebarSettingsInput,
    screen_w: f32,
    screen_h: f32,
    metrics: &crate::settings::UiMetrics,
) -> Option<PopupMenu> {
    let dpi = metrics.dpi;
    let show_line_numbers = input.show_line_numbers;
    let word_wrap = input.word_wrap;
    let show_status_bar = input.show_status_bar;
    let theme_mode = input.theme_mode;
    let current_mode = input.view_mode;
    let item_h = constants::ROW_HEIGHT * dpi;
    let menu_w = 200.0 * dpi;
    let (anchor_x, anchor_y) = if let Some(rect) = settings_btn_rect {
        (rect.x + rect.w / 2.0 - menu_w / 2.0, rect.bottom() + 2.0 * dpi)
    } else {
        (screen_w * 0.025, screen_h * 0.65)
    };
    let items = vec![
        PopupMenuItem::action("显示行号", PMA::ToggleLineNumbers).with_active(show_line_numbers),
        PopupMenuItem::action("自动换行", PMA::ToggleWordWrap).with_active(word_wrap),
        PopupMenuItem::action("显示状态栏", PMA::ToggleStatusBar).with_active(show_status_bar),
        PopupMenuItem::separator(PMA::SetViewMode(ViewMode::Sidebar)),
        PopupMenuItem::action("跟随系统", PMA::SetThemeMode(crate::settings::ThemeMode::System))
            .with_active(theme_mode == crate::settings::ThemeMode::System),
        PopupMenuItem::action("深色模式", PMA::SetThemeMode(crate::settings::ThemeMode::Dark))
            .with_active(theme_mode == crate::settings::ThemeMode::Dark),
        PopupMenuItem::action("浅色模式", PMA::SetThemeMode(crate::settings::ThemeMode::Light))
            .with_active(theme_mode == crate::settings::ThemeMode::Light),
        PopupMenuItem::separator(PMA::SetViewMode(ViewMode::Sidebar)),
        PopupMenuItem::action("Sidebar 模式", PMA::SetViewMode(ViewMode::Sidebar))
            .with_active(current_mode == ViewMode::Sidebar),
        PopupMenuItem::action("Tabs 模式", PMA::SetViewMode(ViewMode::Tabs))
            .with_active(current_mode == ViewMode::Tabs),
        PopupMenuItem::separator(PMA::SetViewMode(ViewMode::Sidebar)),
        PopupMenuItem::action("打开Settings", PMA::OpenSettingsFile),
    ];
    let menu_left = anchor_x.min(screen_w - menu_w).max(0.0);
    let menu_right = menu_left + menu_w;
    let pad_x = 4.0 * dpi;
    let pad_y = 4.0 * dpi;
    let mut top_px = anchor_y + pad_y;
    let mut item_rects = Vec::with_capacity(items.len());
    for item in &items {
        let h = if item.is_separator { 8.0 * dpi } else { item_h };
        item_rects.push(Rect::new(
            menu_left + pad_x,
            top_px,
            menu_right - menu_left - pad_x * 2.0,
            h,
        ));
        top_px += h;
    }
    let menu_h = top_px - anchor_y + pad_y;
    // Overflow protection: flip menu upward if it extends below screen
    let final_top = if anchor_y + menu_h > screen_h - 4.0 * dpi {
        (anchor_y - menu_h).max(4.0 * dpi)
    } else {
        anchor_y
    };
    let offset = final_top - anchor_y;
    let adjusted_rects: Vec<Rect> =
        item_rects.iter().map(|r| Rect::new(r.x, r.y + offset, r.w, r.h)).collect();
    Some(PopupMenu {
        items,
        item_rects: adjusted_rects,
        menu_rect: Rect::new(menu_left, final_top, menu_right - menu_left, menu_h),
        screen_size: (screen_w, screen_h),
        show_checkmarks: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::sidebar::types::NewDocumentKind;

    #[test]
    fn menu_geometry_uses_metrics_and_checks_use_behavior_input() {
        let settings = crate::settings::Settings::new();
        let metrics = crate::settings::UiMetrics::from_settings(&settings, 2.0);
        let input = SidebarSettingsInput {
            show_line_numbers: false,
            word_wrap: false,
            show_status_bar: true,
            theme_mode: crate::settings::ThemeMode::Dark,
            view_mode: ViewMode::Tabs,
        };
        let menu = build_settings_menu(None, &input, 800.0, 600.0, &metrics).expect("menu");

        assert_eq!(menu.menu_rect.w, 400.0);
        assert!(!menu.items[0].is_active);
        assert!(!menu.items[1].is_active);
        assert!(menu.items[2].is_active);
        assert!(menu.items[5].is_active);
        assert!(menu.items[9].is_active);
    }

    #[test]
    fn settings_menu_does_not_expose_a_second_sync_entry() {
        let settings = crate::settings::Settings::new();
        let metrics = crate::settings::UiMetrics::from_settings(&settings, 1.0);
        let menu =
            build_settings_menu(None, &SidebarSettingsInput::default(), 800.0, 600.0, &metrics)
                .expect("settings menu should be constructed");

        assert!(menu.items.iter().all(|item| item.label != "打开同步面板"));
    }

    #[test]
    fn new_document_menu_has_required_order_and_typed_actions() {
        let settings = crate::settings::Settings::new();
        let metrics = crate::settings::UiMetrics::from_settings(&settings, 1.0);
        let anchor = Rect::new(12.0, 40.0, 196.0, 28.0);

        let menu = build_new_document_menu(anchor, (800.0, 600.0), &metrics, true);

        let labels: Vec<&str> = menu.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, vec!["新建 TXT", "新建 MMAP", "新建 MD", "加密笔记"]);
        assert!(matches!(menu.items[0].action, PMA::NewDocument(NewDocumentKind::Text)));
        assert!(matches!(menu.items[1].action, PMA::NewDocument(NewDocumentKind::Mindmap)));
        assert!(matches!(menu.items[2].action, PMA::NewDocument(NewDocumentKind::Markdown)));
        assert!(matches!(
            menu.items[3].action,
            PMA::NewDocument(NewDocumentKind::EncryptedMarkdown)
        ));
    }

    #[test]
    fn new_document_menu_flips_above_when_below_screen() {
        let settings = crate::settings::Settings::new();
        let metrics = crate::settings::UiMetrics::from_settings(&settings, 1.0);
        let anchor = Rect::new(12.0, 560.0, 196.0, 28.0);

        let menu = build_new_document_menu(anchor, (800.0, 600.0), &metrics, true);

        assert!(menu.menu_rect.bottom() <= 600.0);
        assert!(menu.menu_rect.y < anchor.y);
    }
}
