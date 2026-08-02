//! Notora 设置弹层边界。
//!
//! 弹层只承载 Notora 自己的设置界面，并把产品设置快照映射为类型化更新；不复用
//! Textora 的设置业务页面，也不访问文件系统或 editor runtime。

use ui::ThemeMode;
use ui::core::{Dock, Event, EventCtx, LayoutCtx, PaintCtx, Rect};

use crate::notora_settings_view::NotoraSettingsView;
use crate::settings::ProductSettings;

const PANEL_PREFERRED_WIDTH_LOGICAL: f32 = 720.0;
const PANEL_PREFERRED_HEIGHT_LOGICAL: f32 = 560.0;
const PANEL_MINIMUM_MARGIN_LOGICAL: f32 = 24.0;
const PANEL_MAXIMUM_WIDTH_RATIO: f32 = 0.92;
const PANEL_MAXIMUM_HEIGHT_RATIO: f32 = 0.90;
const PANEL_CORNER_RADIUS_LOGICAL: f32 = 12.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NotoraSettingsPersistenceView {
    #[default]
    Saved,
    SaveFailed {
        message: String,
    },
}

/// Notora 设置界面的完整纯输入。
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsOverlayInput {
    pub product_settings: ProductSettings,
    pub persistence: NotoraSettingsPersistenceView,
}

impl SettingsOverlayInput {
    pub fn from_product_settings(product_settings: &ProductSettings) -> Self {
        Self {
            product_settings: product_settings.clone(),
            persistence: NotoraSettingsPersistenceView::Saved,
        }
    }
}

impl Default for SettingsOverlayInput {
    fn default() -> Self {
        Self::from_product_settings(&ProductSettings::default())
    }
}

/// Notora 设置控件产生的产品级变更。
#[derive(Clone, Debug, PartialEq)]
pub enum ProductSettingsUpdate {
    ThemeMode(ThemeMode),
    FontFamily(String),
    FontSize(f32),
    LineHeightRatio(f32),
    WordWrap(bool),
    ShowLineNumbers(bool),
    TabWidth(usize),
    ShowStatusBar(bool),
    RuntimeTabLimit(usize),
    AutoSaveDelayMillis(u64),
    CatalogBackupRetention(usize),
}

impl ProductSettingsUpdate {
    pub fn apply_to(&self, settings: &mut ProductSettings) {
        match self {
            Self::ThemeMode(theme_mode) => settings.appearance.theme_mode = *theme_mode,
            Self::FontFamily(font_family) => settings.editor.font_family = font_family.clone(),
            Self::FontSize(font_size) => settings.editor.font_size = *font_size,
            Self::LineHeightRatio(line_height_ratio) => {
                settings.editor.line_height_ratio = *line_height_ratio
            }
            Self::WordWrap(word_wrap) => settings.editor.word_wrap = *word_wrap,
            Self::ShowLineNumbers(show_line_numbers) => {
                settings.editor.show_line_numbers = *show_line_numbers
            }
            Self::TabWidth(tab_width) => settings.editor.tab_width = *tab_width,
            Self::ShowStatusBar(show_status_bar) => {
                settings.interface.show_status_bar = *show_status_bar
            }
            Self::RuntimeTabLimit(runtime_tab_limit) => {
                settings.interface.runtime_tab_limit = *runtime_tab_limit
            }
            Self::AutoSaveDelayMillis(auto_save_delay_millis) => {
                settings.workspace.auto_save_delay_millis = *auto_save_delay_millis
            }
            Self::CatalogBackupRetention(catalog_backup_retention) => {
                settings.workspace.catalog_backup_retention = *catalog_backup_retention
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsOverlayAction {
    Update(ProductSettingsUpdate),
    RetryPersistence,
    ViewChanged,
    Dismiss,
}

pub struct SettingsOverlay {
    view: NotoraSettingsView,
    panel_rect: Rect,
}

impl Default for SettingsOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsOverlay {
    pub fn new() -> Self {
        Self {
            view: NotoraSettingsView::new(SettingsOverlayInput::default()),
            panel_rect: Rect::ZERO,
        }
    }

    pub fn panel_rect(&self) -> Rect {
        self.panel_rect
    }

    pub fn set_input(&mut self, input: SettingsOverlayInput) {
        self.view.set_input(input);
    }

    pub fn set_rect(&mut self, overlay_rect: Rect, context: &mut LayoutCtx<'_>) {
        self.panel_rect = resolve_panel_rect(overlay_rect, context.dpi);
        self.view.set_rect(Rect::new(0.0, 0.0, self.panel_rect.w, self.panel_rect.h), context);
    }

    pub fn paint(&self, context: &mut PaintCtx<'_>) {
        context.list.fill_rounded(
            self.panel_rect,
            context.theme.settings_theme().modal_surface,
            PANEL_CORNER_RADIUS_LOGICAL * context.dpi,
        );
        let saved_offset = context.list.offset;
        context.list.offset =
            (saved_offset.0 + self.panel_rect.x, saved_offset.1 + self.panel_rect.y);
        self.view.paint(context);
        context.list.offset = saved_offset;
    }

    pub fn route_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<SettingsOverlayAction> {
        if is_backdrop_click(event, self.panel_rect) {
            return Some(SettingsOverlayAction::Dismiss);
        }
        let local_event = Dock::to_local(event, self.panel_rect.x, self.panel_rect.y);
        self.view.route_event(local_event.as_ref(), context)
    }
}

fn resolve_panel_rect(overlay_rect: Rect, dpi: f32) -> Rect {
    let minimum_margin = PANEL_MINIMUM_MARGIN_LOGICAL * dpi;
    let available_width = (overlay_rect.w - minimum_margin * 2.0).max(0.0);
    let available_height = (overlay_rect.h - minimum_margin * 2.0).max(0.0);
    let panel_width = (PANEL_PREFERRED_WIDTH_LOGICAL * dpi)
        .min(overlay_rect.w * PANEL_MAXIMUM_WIDTH_RATIO)
        .min(available_width);
    let panel_height = (PANEL_PREFERRED_HEIGHT_LOGICAL * dpi)
        .min(overlay_rect.h * PANEL_MAXIMUM_HEIGHT_RATIO)
        .min(available_height);
    Rect::new(
        overlay_rect.x + (overlay_rect.w - panel_width) * 0.5,
        overlay_rect.y + (overlay_rect.h - panel_height) * 0.5,
        panel_width,
        panel_height,
    )
}

fn is_backdrop_click(event: &Event, panel_rect: Rect) -> bool {
    let Event::MouseDown { px, py, button: ui::MouseButton::Left } = event else {
        return false;
    };
    !panel_rect.contains(*px, *py)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_input_is_notora_owned() {
        let mut settings = ProductSettings::default();
        settings.editor.font_size = 18.0;
        settings.workspace.auto_save_delay_millis = 1_200;

        let input = SettingsOverlayInput::from_product_settings(&settings);

        assert_eq!(input.product_settings.editor.font_size, 18.0);
        assert_eq!(input.product_settings.workspace.auto_save_delay_millis, 1_200);
        assert_eq!(input.persistence, NotoraSettingsPersistenceView::Saved);
    }

    #[test]
    fn typed_updates_only_change_their_matching_product_fields() {
        let mut settings = ProductSettings::default();
        ProductSettingsUpdate::TabWidth(6).apply_to(&mut settings);
        ProductSettingsUpdate::RuntimeTabLimit(6).apply_to(&mut settings);
        ProductSettingsUpdate::AutoSaveDelayMillis(1_200).apply_to(&mut settings);
        ProductSettingsUpdate::CatalogBackupRetention(4).apply_to(&mut settings);

        assert_eq!(settings.editor.tab_width, 6);
        assert_eq!(settings.interface.runtime_tab_limit, 6);
        assert_eq!(settings.workspace.auto_save_delay_millis, 1_200);
        assert_eq!(settings.workspace.catalog_backup_retention, 4);
    }

    #[test]
    fn panel_uses_notora_preferred_size_and_respects_window_margins() {
        assert_eq!(
            resolve_panel_rect(Rect::new(0.0, 0.0, 1_200.0, 800.0), 1.0),
            Rect::new(240.0, 120.0, 720.0, 560.0),
        );
        let compact = resolve_panel_rect(Rect::new(0.0, 0.0, 500.0, 400.0), 1.0);
        assert!(compact.w <= 452.0);
        assert!(compact.h <= 352.0);
    }
}
