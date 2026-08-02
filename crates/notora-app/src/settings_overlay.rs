//! notora 设置弹层的纯 UI 输入映射。
//!
//! 此模块只把产品设置 DTO 映射到 `ui` 的通用 `SettingsView`，不访问 runtime、
//! 文件系统或产品状态。

use ui::core::widget::{ControlAction, TextPayload, WidgetId};
use ui::core::{Widget, WidgetAction};
use ui::settings_view::{
    SettingsPersistenceView, SettingsView, SettingsViewAction, SettingsViewInput,
};
use ui::text_box::TextBox;
use ui::{Dock, Event, EventCtx, LayoutCtx, PaintCtx, Rect, ThemeMode};

use crate::settings::ProductSettings;

const SETTINGS_PANEL_MAX_WIDTH_LOGICAL: f32 = 960.0;
const SETTINGS_PANEL_HORIZONTAL_INSET_LOGICAL: f32 = 24.0;
const SETTINGS_PANEL_VERTICAL_INSET_LOGICAL: f32 = 32.0;
const PRODUCT_SETTINGS_FOOTER_HEIGHT_LOGICAL: f32 = 92.0;
const PRODUCT_SETTINGS_FIELD_GAP_LOGICAL: f32 = 12.0;
const PRODUCT_SETTINGS_LABEL_FONT_SIZE_LOGICAL: f32 = 12.0;
const PRODUCT_SETTINGS_FIELD_HEIGHT_LOGICAL: f32 = 30.0;
const RUNTIME_TAB_LIMIT_FIELD_ID: WidgetId = WidgetId(9_100);
const AUTO_SAVE_DELAY_FIELD_ID: WidgetId = WidgetId(9_101);
const CATALOG_BACKUP_RETENTION_FIELD_ID: WidgetId = WidgetId(9_102);
const MINIMUM_RUNTIME_TAB_LIMIT: usize = 1;
const MAXIMUM_RUNTIME_TAB_LIMIT: usize = 128;
const MINIMUM_AUTO_SAVE_DELAY_MILLIS: u64 = 100;
const MAXIMUM_AUTO_SAVE_DELAY_MILLIS: u64 = 60_000;
const MINIMUM_CATALOG_BACKUP_RETENTION: usize = 1;
const MAXIMUM_CATALOG_BACKUP_RETENTION: usize = 100;

/// 弹层需要展示的纯输入。保存失败信息由产品层显式映射，而不是由 UI 执行保存。
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsOverlayInput {
    pub product_settings: ProductSettings,
    pub persistence: SettingsPersistenceView,
}

impl SettingsOverlayInput {
    pub fn from_product_settings(product_settings: &ProductSettings) -> Self {
        Self {
            product_settings: product_settings.clone(),
            persistence: SettingsPersistenceView::Saved,
        }
    }

    fn settings_view_input(&self) -> SettingsViewInput {
        let settings = &self.product_settings;
        SettingsViewInput {
            theme_mode: settings.appearance.theme_mode,
            font_family: settings.editor.font_family.clone(),
            font_size: settings.editor.font_size,
            line_height_ratio: settings.editor.line_height_ratio,
            word_wrap: settings.editor.word_wrap,
            show_line_numbers: settings.editor.show_line_numbers,
            tab_width: settings.editor.tab_width,
            view_mode: ui::view_mode::ViewMode::Sidebar,
            show_status_bar: settings.interface.show_status_bar,
            persistence: self.persistence.clone(),
        }
    }
}

impl Default for SettingsOverlayInput {
    fn default() -> Self {
        Self::from_product_settings(&ProductSettings::default())
    }
}

/// 通用和产品专属设置控件产生的产品级变更；数值均在控件边界完成校验。
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

/// 供 render 壳转换为产品 action 的弹层动作。
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsOverlayAction {
    Update(ProductSettingsUpdate),
    Dismiss,
}

/// 以统一 modal 画布承载 `ui::SettingsView` 的小型适配器。
pub struct SettingsOverlay {
    settings_view: SettingsView,
    panel_rect: Rect,
    settings_view_rect: Rect,
    product_settings_rect: Rect,
    runtime_tab_limit_box: TextBox,
    auto_save_delay_box: TextBox,
    catalog_backup_retention_box: TextBox,
    last_input: Option<SettingsOverlayInput>,
}

impl Default for SettingsOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsOverlay {
    pub fn new() -> Self {
        let mut runtime_tab_limit_box = product_settings_text_box(RUNTIME_TAB_LIMIT_FIELD_ID);
        runtime_tab_limit_box.set_placeholder("1–128");
        let mut auto_save_delay_box = product_settings_text_box(AUTO_SAVE_DELAY_FIELD_ID);
        auto_save_delay_box.set_placeholder("100–60000 ms");
        let mut catalog_backup_retention_box =
            product_settings_text_box(CATALOG_BACKUP_RETENTION_FIELD_ID);
        catalog_backup_retention_box.set_placeholder("1–100");
        Self {
            settings_view: SettingsView::new(
                SettingsOverlayInput::from_product_settings(&ProductSettings::default())
                    .settings_view_input(),
            ),
            panel_rect: Rect::ZERO,
            settings_view_rect: Rect::ZERO,
            product_settings_rect: Rect::ZERO,
            runtime_tab_limit_box,
            auto_save_delay_box,
            catalog_backup_retention_box,
            last_input: None,
        }
    }

    pub fn panel_rect(&self) -> Rect {
        self.panel_rect
    }

    pub fn set_input(&mut self, input: SettingsOverlayInput) {
        if self.last_input.as_ref() == Some(&input) {
            return;
        }
        self.settings_view.set_input(input.settings_view_input());
        self.runtime_tab_limit_box
            .sync_text(&input.product_settings.interface.runtime_tab_limit.to_string());
        self.auto_save_delay_box
            .sync_text(&input.product_settings.workspace.auto_save_delay_millis.to_string());
        self.catalog_backup_retention_box
            .sync_text(&input.product_settings.workspace.catalog_backup_retention.to_string());
        self.last_input = Some(input);
    }

    pub fn set_rect(&mut self, overlay_rect: Rect, context: &mut LayoutCtx<'_>) {
        let dpi = context.dpi;
        let horizontal_inset = SETTINGS_PANEL_HORIZONTAL_INSET_LOGICAL * dpi;
        let vertical_inset = SETTINGS_PANEL_VERTICAL_INSET_LOGICAL * dpi;
        let panel_width = (SETTINGS_PANEL_MAX_WIDTH_LOGICAL * dpi)
            .min((overlay_rect.w - horizontal_inset * 2.0).max(0.0));
        let panel_height = (overlay_rect.h - vertical_inset * 2.0).max(0.0);
        self.panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - panel_width) * 0.5,
            overlay_rect.y + (overlay_rect.h - panel_height) * 0.5,
            panel_width,
            panel_height,
        );
        let footer_height = (PRODUCT_SETTINGS_FOOTER_HEIGHT_LOGICAL * dpi).min(panel_height);
        self.settings_view_rect = Rect::new(0.0, 0.0, panel_width, panel_height - footer_height);
        self.product_settings_rect = Rect::new(
            self.panel_rect.x,
            self.panel_rect.bottom() - footer_height,
            panel_width,
            footer_height,
        );
        self.settings_view.set_rect(self.settings_view_rect, context);
        let field_rects = product_settings_field_rects(self.product_settings_rect, dpi);
        self.runtime_tab_limit_box.set_rect(field_rects.runtime_tab_limit, context);
        self.auto_save_delay_box.set_rect(field_rects.auto_save_delay, context);
        self.catalog_backup_retention_box.set_rect(field_rects.catalog_backup_retention, context);
    }

    pub fn paint(&self, context: &mut PaintCtx<'_>) {
        context.list.fill_rounded(
            self.panel_rect,
            context.theme.settings_theme().modal_surface,
            12.0 * context.dpi,
        );
        let saved_offset = context.list.offset;
        context.list.offset =
            (saved_offset.0 + self.panel_rect.x, saved_offset.1 + self.panel_rect.y);
        self.settings_view.paint(context);
        context.list.offset = saved_offset;
        self.paint_product_settings(context);
    }

    pub fn route_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<SettingsOverlayAction> {
        if is_backdrop_click(event, self.panel_rect) {
            return Some(SettingsOverlayAction::Dismiss);
        }
        if let Some(action) = self.route_product_settings_event(event, context) {
            return Some(action);
        }
        if event_is_in_rect(event, self.product_settings_rect) {
            return None;
        }
        let local_event = Dock::to_local(event, self.panel_rect.x, self.panel_rect.y);
        let action = self.settings_view.on_event(local_event.as_ref(), context)?;
        settings_view_action(action)
    }

    fn paint_product_settings(&self, context: &mut PaintCtx<'_>) {
        if self.product_settings_rect == Rect::ZERO {
            return;
        }
        context.list.fill(self.product_settings_rect, context.theme.palette.bg_surface);
        let fields = product_settings_field_rects(self.product_settings_rect, context.dpi);
        paint_product_settings_label(context, fields.runtime_tab_limit, "Runtime tabs");
        paint_product_settings_label(context, fields.auto_save_delay, "Autosave delay (ms)");
        paint_product_settings_label(context, fields.catalog_backup_retention, "Catalog backups");
        self.runtime_tab_limit_box.paint(context);
        self.auto_save_delay_box.paint(context);
        self.catalog_backup_retention_box.paint(context);
    }

    fn route_product_settings_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<SettingsOverlayAction> {
        for text_box in [
            &mut self.runtime_tab_limit_box,
            &mut self.auto_save_delay_box,
            &mut self.catalog_backup_retention_box,
        ] {
            if let Some(action) = text_box.on_event(event, context) {
                return product_settings_text_box_action(action);
            }
        }
        None
    }
}

fn is_backdrop_click(event: &Event, panel_rect: Rect) -> bool {
    let Event::MouseDown { px, py, button: ui::MouseButton::Left } = event else {
        return false;
    };
    !panel_rect.contains(*px, *py)
}

#[derive(Clone, Copy)]
struct ProductSettingsFieldRects {
    runtime_tab_limit: Rect,
    auto_save_delay: Rect,
    catalog_backup_retention: Rect,
}

fn product_settings_field_rects(
    product_settings_rect: Rect,
    dpi: f32,
) -> ProductSettingsFieldRects {
    let gap = PRODUCT_SETTINGS_FIELD_GAP_LOGICAL * dpi;
    let horizontal_inset = gap;
    let available_width = (product_settings_rect.w - horizontal_inset * 2.0 - gap * 2.0).max(0.0);
    let field_width = available_width / 3.0;
    let field_y = product_settings_rect.y + 36.0 * dpi;
    let field_height = PRODUCT_SETTINGS_FIELD_HEIGHT_LOGICAL * dpi;
    ProductSettingsFieldRects {
        runtime_tab_limit: Rect::new(
            product_settings_rect.x + horizontal_inset,
            field_y,
            field_width,
            field_height,
        ),
        auto_save_delay: Rect::new(
            product_settings_rect.x + horizontal_inset + field_width + gap,
            field_y,
            field_width,
            field_height,
        ),
        catalog_backup_retention: Rect::new(
            product_settings_rect.x + horizontal_inset + (field_width + gap) * 2.0,
            field_y,
            field_width,
            field_height,
        ),
    }
}

fn product_settings_text_box(id: WidgetId) -> TextBox {
    let mut text_box = TextBox::with_id(id);
    text_box.set_blink(true);
    text_box
}

fn paint_product_settings_label(context: &mut PaintCtx<'_>, field_rect: Rect, label: &str) {
    context.text(
        field_rect.x,
        field_rect.y - 8.0 * context.dpi,
        PRODUCT_SETTINGS_LABEL_FONT_SIZE_LOGICAL * context.dpi,
        context.theme.palette.text_muted,
        label,
    );
}

fn event_is_in_rect(event: &Event, rect: Rect) -> bool {
    match event {
        Event::MouseMove { px, py }
        | Event::MouseDown { px, py, .. }
        | Event::MouseUp { px, py, .. }
        | Event::Wheel { px, py, .. } => rect.contains(*px, *py),
        Event::KeyDown(..)
        | Event::ImePreedit { .. }
        | Event::ImeCommit(_)
        | Event::ImeEnable
        | Event::ImeDisable => false,
    }
}

fn product_settings_text_box_action(action: WidgetAction) -> Option<SettingsOverlayAction> {
    let WidgetAction::Control(ControlAction::TextCommitted {
        id,
        value: TextPayload::Plain(value),
    }) = action
    else {
        return None;
    };
    match id {
        RUNTIME_TAB_LIMIT_FIELD_ID => {
            parse_usize_in_range(&value, MINIMUM_RUNTIME_TAB_LIMIT, MAXIMUM_RUNTIME_TAB_LIMIT)
                .map(ProductSettingsUpdate::RuntimeTabLimit)
                .map(SettingsOverlayAction::Update)
        }
        AUTO_SAVE_DELAY_FIELD_ID => parse_u64_in_range(
            &value,
            MINIMUM_AUTO_SAVE_DELAY_MILLIS,
            MAXIMUM_AUTO_SAVE_DELAY_MILLIS,
        )
        .map(ProductSettingsUpdate::AutoSaveDelayMillis)
        .map(SettingsOverlayAction::Update),
        CATALOG_BACKUP_RETENTION_FIELD_ID => parse_usize_in_range(
            &value,
            MINIMUM_CATALOG_BACKUP_RETENTION,
            MAXIMUM_CATALOG_BACKUP_RETENTION,
        )
        .map(ProductSettingsUpdate::CatalogBackupRetention)
        .map(SettingsOverlayAction::Update),
        _ => None,
    }
}

fn parse_usize_in_range(value: &str, minimum: usize, maximum: usize) -> Option<usize> {
    let parsed = value.trim().parse::<usize>().ok()?;
    (minimum..=maximum).contains(&parsed).then_some(parsed)
}

fn parse_u64_in_range(value: &str, minimum: u64, maximum: u64) -> Option<u64> {
    let parsed = value.trim().parse::<u64>().ok()?;
    (minimum..=maximum).contains(&parsed).then_some(parsed)
}

fn settings_view_action(action: WidgetAction) -> Option<SettingsOverlayAction> {
    let WidgetAction::Settings(action) = action else {
        return None;
    };
    let update = match action {
        SettingsViewAction::SetThemeMode(theme_mode) => {
            ProductSettingsUpdate::ThemeMode(theme_mode)
        }
        SettingsViewAction::SetFontFamily(font_family) => {
            ProductSettingsUpdate::FontFamily(font_family)
        }
        SettingsViewAction::SetFontSize(font_size) => ProductSettingsUpdate::FontSize(font_size),
        SettingsViewAction::SetLineHeightRatio(line_height_ratio) => {
            ProductSettingsUpdate::LineHeightRatio(line_height_ratio)
        }
        SettingsViewAction::SetWordWrap(word_wrap) => ProductSettingsUpdate::WordWrap(word_wrap),
        SettingsViewAction::SetShowLineNumbers(show_line_numbers) => {
            ProductSettingsUpdate::ShowLineNumbers(show_line_numbers)
        }
        SettingsViewAction::SetTabWidth(tab_width) => ProductSettingsUpdate::TabWidth(tab_width),
        SettingsViewAction::SetShowStatusBar(show_status_bar) => {
            ProductSettingsUpdate::ShowStatusBar(show_status_bar)
        }
        SettingsViewAction::SetViewMode(_) | SettingsViewAction::RetryPersistence => return None,
    };
    Some(SettingsOverlayAction::Update(update))
}

#[cfg(test)]
mod tests {
    use super::{
        AUTO_SAVE_DELAY_FIELD_ID, CATALOG_BACKUP_RETENTION_FIELD_ID, ProductSettingsUpdate,
        RUNTIME_TAB_LIMIT_FIELD_ID, SettingsOverlayAction, SettingsOverlayInput,
        product_settings_text_box_action, settings_view_action,
    };
    use crate::settings::ProductSettings;

    #[test]
    fn maps_product_settings_into_the_shared_form_input() {
        let mut settings = ProductSettings::default();
        settings.editor.font_size = 18.0;
        settings.editor.word_wrap = false;

        let input = SettingsOverlayInput::from_product_settings(&settings).settings_view_input();
        assert_eq!(input.font_size, 18.0);
        assert!(!input.word_wrap);
        assert_eq!(input.show_status_bar, settings.interface.show_status_bar);
    }

    #[test]
    fn typed_updates_only_change_their_matching_product_field() {
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
    fn settings_view_actions_remain_typed_at_the_product_boundary() {
        let action = settings_view_action(ui::core::WidgetAction::Settings(
            ui::settings_view::SettingsViewAction::SetWordWrap(false),
        ));

        assert_eq!(
            action,
            Some(super::SettingsOverlayAction::Update(ProductSettingsUpdate::WordWrap(false)))
        );
    }

    #[test]
    fn product_specific_form_fields_validate_and_emit_typed_updates() {
        let committed = |id: ui::core::widget::WidgetId, value: &str| {
            ui::core::WidgetAction::Control(ui::core::widget::ControlAction::TextCommitted {
                id,
                value: ui::core::widget::TextPayload::Plain(value.to_owned()),
            })
        };
        assert_eq!(
            product_settings_text_box_action(committed(RUNTIME_TAB_LIMIT_FIELD_ID, "24")),
            Some(SettingsOverlayAction::Update(ProductSettingsUpdate::RuntimeTabLimit(24)))
        );
        assert_eq!(
            product_settings_text_box_action(committed(AUTO_SAVE_DELAY_FIELD_ID, "1200")),
            Some(SettingsOverlayAction::Update(ProductSettingsUpdate::AutoSaveDelayMillis(1_200)))
        );
        assert_eq!(
            product_settings_text_box_action(committed(CATALOG_BACKUP_RETENTION_FIELD_ID, "4")),
            Some(SettingsOverlayAction::Update(ProductSettingsUpdate::CatalogBackupRetention(4)))
        );
        assert_eq!(
            product_settings_text_box_action(committed(RUNTIME_TAB_LIMIT_FIELD_ID, "0")),
            None
        );
    }
}
