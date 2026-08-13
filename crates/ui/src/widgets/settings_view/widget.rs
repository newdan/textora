use std::any::Any;
use std::borrow::Cow;

use crate::core::widget::{ControlAction, TextPayload, WidgetId};
use crate::core::{
    AccessibilityActionRequest, AccessibilityContext, AccessibilityNode, Event, EventCtx,
    LayoutCtx, PaintCtx, Rect, Widget, WidgetAction,
};
use crate::theme::SettingsTheme;
use crate::widgets::button::{Button, ButtonStyle};
use crate::widgets::form::{FormRow, FormRowStyle, FormSection, FormSectionStyle, FormView};
use crate::widgets::inline_group::{InlineChild, InlineGroup};
use crate::widgets::label::{Label, LabelForeground, LabelStyle};
use crate::widgets::switch::Switch;
use crate::widgets::text_box::TextBox;

use super::types::{
    SettingsCategory, SettingsPersistenceView, SettingsViewAction, SettingsViewInput,
    ValidationError, parse_font_family, parse_font_size, parse_line_height_ratio, parse_tab_width,
};
use crate::settings::ThemeMode;
use crate::view_mode::ViewMode;

const SETTINGS_SIDEBAR_WIDTH_LOGICAL: f32 = 160.0;
const SETTINGS_COMPACT_SIDEBAR_WIDTH_LOGICAL: f32 = 96.0;
const SETTINGS_COMPACT_LAYOUT_THRESHOLD_LOGICAL: f32 = 400.0;
const SETTINGS_SIDEBAR_TOP_INSET_LOGICAL: f32 = 12.0;
const SETTINGS_FORM_INSET_LOGICAL: f32 = 24.0;
const SETTINGS_COMPACT_FORM_INSET_LOGICAL: f32 = 12.0;
const SETTINGS_CATEGORY_HORIZONTAL_INSET_LOGICAL: f32 = 10.0;
const SETTINGS_CATEGORY_BUTTON_HEIGHT_LOGICAL: f32 = 34.0;
const SETTINGS_CATEGORY_BUTTON_GAP_LOGICAL: f32 = 4.0;
const SETTINGS_FORM_GAP_LOGICAL: f32 = 16.0;
const SETTINGS_COMPACT_FORM_GAP_LOGICAL: f32 = 8.0;
const SETTINGS_BUTTON_WIDTH_LOGICAL: f32 = 78.0;
const SETTINGS_TEXT_BOX_WIDTH_LOGICAL: f32 = 192.0;
const SETTINGS_CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
const SETTINGS_BUTTON_FONT_SIZE_LOGICAL: f32 = 14.0;
const SETTINGS_BUTTON_PADDING_LOGICAL: f32 = 12.0;
const SETTINGS_BUTTON_RADIUS_LOGICAL: f32 = 8.0;
const SETTINGS_ROW_HEIGHT_LOGICAL: f32 = 64.0;
const SETTINGS_ROW_LABEL_WIDTH_LOGICAL: f32 = 176.0;
const SETTINGS_ROW_COLUMN_GAP_LOGICAL: f32 = 12.0;
const SETTINGS_ROW_VERTICAL_INSET_LOGICAL: f32 = 10.0;
const SETTINGS_ROW_HORIZONTAL_INSET_LOGICAL: f32 = 16.0;
const SETTINGS_SECTION_TITLE_GAP_LOGICAL: f32 = 6.0;
const SETTINGS_SECTION_DESCRIPTION_GAP_LOGICAL: f32 = 14.0;
const SETTINGS_SECTION_CORNER_RADIUS_LOGICAL: f32 = 10.0;
const SETTINGS_SECTION_TITLE_FONT_SIZE_LOGICAL: f32 = 17.0;
const SETTINGS_ROW_LABEL_FONT_SIZE_LOGICAL: f32 = 14.0;
const SETTINGS_DESCRIPTION_FONT_SIZE_LOGICAL: f32 = 12.0;
const SETTINGS_SIDEBAR_SEPARATOR_WIDTH_LOGICAL: f32 = 1.0;
const SETTINGS_TRANSPARENT: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
const SETTINGS_ACTION_HOVER_ACCENT_BLEND: f32 = 0.06;
const SETTINGS_ACTION_PRESSED_ACCENT_BLEND: f32 = 0.16;
const SETTINGS_SEGMENT_HOVER_ACCENT_BLEND: f32 = 0.16;
const SETTINGS_SEGMENT_PRESSED_ACCENT_BLEND: f32 = 0.14;
const SETTINGS_CATEGORY_HOVER_ACCENT_BLEND: f32 = 0.05;
const SETTINGS_CATEGORY_PRESSED_ACCENT_BLEND: f32 = 0.09;
const SETTINGS_CATEGORY_SELECTED_ACCENT_BLEND: f32 = 0.14;
const SETTINGS_DISABLED_FOREGROUND_ALPHA: f32 = 0.45;

const APPEARANCE_CATEGORY_ID: WidgetId = WidgetId(0x7365_7474_6170_7065);
const EDITOR_CATEGORY_ID: WidgetId = WidgetId(0x7365_7474_6564_6974);
const INTERFACE_CATEGORY_ID: WidgetId = WidgetId(0x7365_7474_696e_7466);
const THEME_SYSTEM_ID: WidgetId = WidgetId(0x7365_7474_7468_7379);
const THEME_DARK_ID: WidgetId = WidgetId(0x7365_7474_7468_6461);
const THEME_LIGHT_ID: WidgetId = WidgetId(0x7365_7474_7468_6c69);
const FONT_FAMILY_ID: WidgetId = WidgetId(0x7365_7474_666f_6e74);
const FONT_SIZE_ID: WidgetId = WidgetId(0x7365_7474_7369_7a65);
const LINE_HEIGHT_RATIO_ID: WidgetId = WidgetId(0x7365_7474_6c69_6e65);
const WORD_WRAP_ID: WidgetId = WidgetId(0x7365_7474_7772_6170);
const LINE_NUMBERS_ID: WidgetId = WidgetId(0x7365_7474_6e75_6d62);
const TAB_WIDTH_ID: WidgetId = WidgetId(0x7365_7474_7461_6273);
const VIEW_SIDEBAR_ID: WidgetId = WidgetId(0x7365_7474_7669_6577);
const VIEW_TABS_ID: WidgetId = WidgetId(0x7365_7474_7669_6574);
const STATUS_BAR_ID: WidgetId = WidgetId(0x7365_7474_7374_6174);
const RETRY_PERSISTENCE_ID: WidgetId = WidgetId(0x7365_7474_7265_7472);
const SETTINGS_BANNER_HEIGHT_LOGICAL: f32 = 40.0;
const SETTINGS_BANNER_GAP_LOGICAL: f32 = 12.0;
const SETTINGS_BANNER_LABEL_WIDTH_LOGICAL: f32 = 220.0;
const SETTINGS_BANNER_BUTTON_WIDTH_LOGICAL: f32 = 92.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldValidation {
    field_id: WidgetId,
    error: ValidationError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsHoverTarget {
    Category(usize),
    Content,
    PersistenceBanner,
}

pub struct SettingsView {
    rect: Rect,
    sidebar_width: f32,
    input: SettingsViewInput,
    active_category: SettingsCategory,
    category_navigation_visible: bool,
    category_buttons: Vec<(SettingsCategory, Button)>,
    category_rects: Vec<Rect>,
    category_pointer_index: Option<usize>,
    hover_target: Option<SettingsHoverTarget>,
    form: FormView,
    form_rect: Rect,
    form_needs_layout: bool,
    focus_first_form_field_after_layout: bool,
    focused_id: Option<WidgetId>,
    validation: Option<FieldValidation>,
    persistence_banner: Option<InlineGroup>,
    persistence_banner_rect: Rect,
    settings_theme: SettingsTheme,
}

impl SettingsView {
    pub fn new(input: SettingsViewInput) -> Self {
        let active_category = SettingsCategory::default();
        let settings_theme = fallback_settings_theme();
        let mut view = Self {
            rect: Rect::ZERO,
            sidebar_width: 0.0,
            input,
            active_category,
            category_navigation_visible: true,
            category_buttons: Vec::new(),
            category_rects: Vec::new(),
            category_pointer_index: None,
            hover_target: None,
            form: FormView::new(crate::widgets::form::FormViewStyle::default()),
            form_rect: Rect::ZERO,
            form_needs_layout: true,
            focus_first_form_field_after_layout: false,
            focused_id: None,
            validation: None,
            persistence_banner: None,
            persistence_banner_rect: Rect::ZERO,
            settings_theme,
        };
        view.category_buttons = view.build_category_buttons();
        view.refresh_persistence_banner();
        view
    }

    pub fn input(&self) -> &SettingsViewInput {
        &self.input
    }

    pub fn set_input(&mut self, input: SettingsViewInput) {
        self.input = input;
        self.validation = None;
        self.form_needs_layout = true;
        self.rebuild_category_buttons();
        self.refresh_persistence_banner();
    }

    pub fn active_category(&self) -> SettingsCategory {
        self.active_category
    }

    pub fn set_category_navigation_visible(&mut self, visible: bool) {
        self.category_navigation_visible = visible;
        if visible {
            return;
        }
        self.category_rects.clear();
        self.category_pointer_index = None;
        if matches!(self.hover_target, Some(SettingsHoverTarget::Category(_))) {
            self.hover_target = None;
        }
    }

    pub fn set_active_category(&mut self, category: SettingsCategory) {
        if !matches!(
            category,
            SettingsCategory::Appearance | SettingsCategory::Editor | SettingsCategory::Interface
        ) {
            return;
        }
        self.activate_category(category);
    }

    pub fn category_is_selected(&self, category: SettingsCategory) -> bool {
        self.active_category == category
    }

    pub fn has_validation_error(&self) -> bool {
        self.validation.is_some()
    }

    /// 聚焦输入框的 IME 光标位置（SettingsView 本地坐标），用于定位系统输入法候选窗。
    pub fn focused_ime_cursor_rect(&self) -> Option<Rect> {
        let local = self.form.focused_ime_cursor_rect()?;
        Some(Rect::new(self.form_rect.x + local.x, self.form_rect.y + local.y, local.w, local.h))
    }

    pub fn validation_message(&self) -> Option<&'static str> {
        self.validation.map(|validation| validation.error.message())
    }

    pub fn visible_text(&self) -> String {
        match &self.input.persistence {
            SettingsPersistenceView::Saved => String::new(),
            SettingsPersistenceView::SaveFailed { message } => {
                format!("当前修改尚未保存：{message}")
            }
        }
    }

    fn build_category_buttons(&self) -> Vec<(SettingsCategory, Button)> {
        [
            (SettingsCategory::Appearance, "外观", APPEARANCE_CATEGORY_ID),
            (SettingsCategory::Editor, "编辑器", EDITOR_CATEGORY_ID),
            (SettingsCategory::Interface, "界面", INTERFACE_CATEGORY_ID),
        ]
        .into_iter()
        .map(|(category, title, id)| {
            let mut button = Button::new(id, category_button_style(self.settings_theme));
            button.set_text(Some(title.to_owned()));
            button.set_selected(category == self.active_category);
            (category, button)
        })
        .collect()
    }

    fn sync_category_selection(&mut self) {
        for (category, button) in &mut self.category_buttons {
            button.set_selected(*category == self.active_category);
        }
    }

    fn rebuild_category_buttons(&mut self) {
        self.category_buttons = self.build_category_buttons();
    }

    fn refresh_persistence_banner(&mut self) {
        self.persistence_banner = match self.input.persistence {
            SettingsPersistenceView::Saved => None,
            SettingsPersistenceView::SaveFailed { .. } => Some(self.build_persistence_banner()),
        };
    }

    fn build_persistence_banner(&self) -> InlineGroup {
        let mut warning_label = description_label("当前修改尚未保存");
        warning_label.set_leading_icon(Some("warning".into()));
        let mut retry_button =
            Button::new(RETRY_PERSISTENCE_ID, action_button_style(self.settings_theme));
        retry_button.set_text(Some("重试".into()));
        InlineGroup::new(vec![
            InlineChild::fixed(Box::new(warning_label), SETTINGS_BANNER_LABEL_WIDTH_LOGICAL),
            InlineChild::fixed(Box::new(retry_button), SETTINGS_BANNER_BUTTON_WIDTH_LOGICAL)
                .with_cross_size(SETTINGS_CONTROL_HEIGHT_LOGICAL),
        ])
        .with_alignment(crate::widgets::inline_group::CrossAlignment::Center)
    }

    fn rebuild_form(&mut self, ctx: &mut LayoutCtx) {
        let sections = match self.active_category {
            SettingsCategory::Appearance => self.build_appearance_sections(),
            SettingsCategory::Editor => self.build_editor_sections(),
            SettingsCategory::Interface => self.build_interface_sections(),
        };
        self.form.set_sections(sections, ctx);
        if self.focus_first_form_field_after_layout {
            let mut focusable_ids = Vec::new();
            self.form.collect_focusable_ids(&mut focusable_ids);
            self.focused_id = focusable_ids.first().copied();
            self.focus_first_form_field_after_layout = false;
        }
        self.form.set_keyboard_focus(self.focused_id);
        self.form_needs_layout = false;
    }

    fn build_appearance_sections(&self) -> Vec<FormSection> {
        let rows = vec![
            self.theme_mode_row(),
            self.font_family_row(),
            self.font_size_row(),
            self.line_height_ratio_row(),
        ];
        vec![FormSection::new(
            section_title_label("外观"),
            Some(section_description_label("调整主题与编辑器文字显示。")),
            rows,
            settings_section_style(),
        )]
    }

    fn theme_mode_row(&self) -> FormRow {
        let buttons = [
            (THEME_SYSTEM_ID, "跟随系统", self.input.theme_mode == ThemeMode::System),
            (THEME_DARK_ID, "深色", self.input.theme_mode == ThemeMode::Dark),
            (THEME_LIGHT_ID, "浅色", self.input.theme_mode == ThemeMode::Light),
        ]
        .into_iter()
        .map(|(id, title, selected)| {
            let mut button = Button::new(id, segmented_button_style(self.settings_theme));
            button.set_text(Some(title.to_owned()));
            button.set_selected(selected);
            InlineChild::fixed(Box::new(button), SETTINGS_BUTTON_WIDTH_LOGICAL)
                .with_cross_size(SETTINGS_CONTROL_HEIGHT_LOGICAL)
        })
        .collect();
        let group = InlineGroup::new(buttons)
            .with_gap(0.0)
            .with_main_alignment(crate::widgets::inline_group::MainAlignment::End)
            .with_alignment(crate::widgets::inline_group::CrossAlignment::Center);
        FormRow::new(row_label("主题"), None, Box::new(group), settings_row_style())
    }

    fn font_family_row(&self) -> FormRow {
        let mut text_box = settings_text_box(FONT_FAMILY_ID);
        text_box.set_text(&self.input.font_family);
        text_box.set_placeholder("字体名称");
        FormRow::new(
            row_label("字体"),
            Some(description_label("输入编辑器使用的字体名称。")),
            Box::new(text_box),
            settings_row_style(),
        )
    }

    fn font_size_row(&self) -> FormRow {
        let mut text_box = settings_text_box(FONT_SIZE_ID);
        text_box.set_text(&format_float(self.input.font_size));
        text_box.set_placeholder("6–72");
        FormRow::new(
            row_label("字号"),
            Some(description_label("允许范围：6–72。")),
            Box::new(text_box),
            settings_row_style(),
        )
    }

    fn line_height_ratio_row(&self) -> FormRow {
        let mut text_box = settings_text_box(LINE_HEIGHT_RATIO_ID);
        text_box.set_text(&format_float(self.input.line_height_ratio));
        text_box.set_placeholder("1–3");
        FormRow::new(
            row_label("行高比例"),
            Some(description_label("允许范围：1–3。")),
            Box::new(text_box),
            settings_row_style(),
        )
    }

    fn build_editor_sections(&self) -> Vec<FormSection> {
        vec![FormSection::new(
            section_title_label("编辑器"),
            Some(section_description_label("调整编辑行为与制表符宽度。")),
            vec![self.word_wrap_row(), self.line_numbers_row(), self.tab_width_row()],
            settings_section_style(),
        )]
    }

    fn word_wrap_row(&self) -> FormRow {
        FormRow::new(
            row_label("自动换行"),
            Some(description_label("在编辑区域宽度不足时折行显示。")),
            Box::new(Switch::new(WORD_WRAP_ID, self.input.word_wrap)),
            settings_row_style(),
        )
    }

    fn line_numbers_row(&self) -> FormRow {
        FormRow::new(
            row_label("显示行号"),
            Some(description_label("在编辑器左侧显示行号。")),
            Box::new(Switch::new(LINE_NUMBERS_ID, self.input.show_line_numbers)),
            settings_row_style(),
        )
    }

    fn tab_width_row(&self) -> FormRow {
        let mut text_box = settings_text_box(TAB_WIDTH_ID);
        text_box.set_text(&self.input.tab_width.to_string());
        text_box.set_placeholder("1–16");
        FormRow::new(
            row_label("Tab 宽度"),
            Some(description_label("允许范围：1–16 个空格。")),
            Box::new(text_box),
            settings_row_style(),
        )
    }

    fn build_interface_sections(&self) -> Vec<FormSection> {
        vec![FormSection::new(
            section_title_label("界面"),
            Some(section_description_label("选择主界面布局与状态栏显示。")),
            vec![self.view_mode_row(), self.status_bar_row()],
            settings_section_style(),
        )]
    }

    fn view_mode_row(&self) -> FormRow {
        let buttons = [
            (VIEW_SIDEBAR_ID, "侧边栏", self.input.view_mode == ViewMode::Sidebar),
            (VIEW_TABS_ID, "标签页", self.input.view_mode == ViewMode::Tabs),
        ]
        .into_iter()
        .map(|(id, title, selected)| {
            let mut button = Button::new(id, segmented_button_style(self.settings_theme));
            button.set_text(Some(title.to_owned()));
            button.set_selected(selected);
            InlineChild::fixed(Box::new(button), SETTINGS_BUTTON_WIDTH_LOGICAL)
                .with_cross_size(SETTINGS_CONTROL_HEIGHT_LOGICAL)
        })
        .collect();
        let group = InlineGroup::new(buttons)
            .with_gap(0.0)
            .with_main_alignment(crate::widgets::inline_group::MainAlignment::End)
            .with_alignment(crate::widgets::inline_group::CrossAlignment::Center);
        FormRow::new(row_label("视图模式"), None, Box::new(group), settings_row_style())
    }

    fn status_bar_row(&self) -> FormRow {
        FormRow::new(
            row_label("显示状态栏"),
            Some(description_label("在窗口底部显示当前文档状态。")),
            Box::new(Switch::new(STATUS_BAR_ID, self.input.show_status_bar)),
            settings_row_style(),
        )
    }

    fn category_index_at(&self, px: f32, py: f32) -> Option<usize> {
        if !self.category_navigation_visible {
            return None;
        }
        self.category_rects.iter().position(|rect| rect.contains(px, py))
    }

    fn category_event(event: &Event, rect: Rect) -> Event {
        match event {
            Event::MouseMove { px, py } => Event::MouseMove { px: px - rect.x, py: py - rect.y },
            Event::MouseDown { px, py, button } => {
                Event::MouseDown { px: px - rect.x, py: py - rect.y, button: *button }
            }
            Event::MouseUp { px, py, button } => {
                Event::MouseUp { px: px - rect.x, py: py - rect.y, button: *button }
            }
            other => other.clone(),
        }
    }

    fn form_event<'a>(event: &'a Event, rect: Rect) -> Cow<'a, Event> {
        crate::core::dock::Dock::to_local(event, rect.x, rect.y)
    }

    fn dispatch_category_event(
        &mut self,
        index: usize,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let rect = *self.category_rects.get(index)?;
        let action =
            self.category_buttons[index].1.on_event(&Self::category_event(event, rect), ctx)?;
        match action {
            WidgetAction::Control(control_action) => self.handle_control_action(control_action),
            other => Some(other),
        }
    }

    fn set_focused_control(&mut self, focused_id: Option<WidgetId>) {
        self.focused_id = focused_id;
        for (_, button) in &mut self.category_buttons {
            button.set_keyboard_focus(focused_id);
        }
        self.form.set_keyboard_focus(focused_id);
        if let Some(banner) = self.persistence_banner.as_mut() {
            banner.set_keyboard_focus(focused_id);
        }
    }

    fn handle_category_action(&mut self, action: ControlAction) -> Option<WidgetAction> {
        let ControlAction::Activated { id } = action else {
            return Some(WidgetAction::Consumed);
        };
        let Some(category) = self.category_for_id(id) else {
            return Some(WidgetAction::Consumed);
        };
        self.activate_category(category);
        Some(WidgetAction::Consumed)
    }

    fn activate_category(&mut self, category: SettingsCategory) {
        self.active_category = category;
        self.sync_category_selection();
        self.focused_id = None;
        self.focus_first_form_field_after_layout = true;
        self.form_needs_layout = true;
    }

    fn category_for_id(&self, id: WidgetId) -> Option<SettingsCategory> {
        match id {
            APPEARANCE_CATEGORY_ID => Some(SettingsCategory::Appearance),
            EDITOR_CATEGORY_ID => Some(SettingsCategory::Editor),
            INTERFACE_CATEGORY_ID => Some(SettingsCategory::Interface),
            _ => None,
        }
    }

    fn handle_control_action(&mut self, action: ControlAction) -> Option<WidgetAction> {
        match action {
            ControlAction::Activated { id } if self.category_for_id(id).is_some() => {
                self.handle_category_action(ControlAction::Activated { id })
            }
            ControlAction::Activated { id } => self.handle_activated_control(id),
            ControlAction::TextEdited { .. } => Some(WidgetAction::Consumed),
            ControlAction::TextCommitted { id, value } => self.handle_text_commit(id, value),
            ControlAction::Toggled { id, checked } => self.handle_toggle(id, checked),
            ControlAction::FocusRequested { id } => {
                self.set_focused_control(Some(id));
                Some(WidgetAction::Consumed)
            }
        }
    }

    fn handle_activated_control(&mut self, id: WidgetId) -> Option<WidgetAction> {
        match id {
            THEME_SYSTEM_ID | THEME_DARK_ID | THEME_LIGHT_ID => {
                let theme_mode = match id {
                    THEME_SYSTEM_ID => ThemeMode::System,
                    THEME_DARK_ID => ThemeMode::Dark,
                    THEME_LIGHT_ID => ThemeMode::Light,
                    _ => unreachable!("theme id was checked before dispatch"),
                };
                self.input.theme_mode = theme_mode;
                self.validation = None;
                Some(WidgetAction::Settings(SettingsViewAction::SetThemeMode(theme_mode)))
            }
            VIEW_SIDEBAR_ID => self.map_view_mode(ViewMode::Sidebar),
            VIEW_TABS_ID => self.map_view_mode(ViewMode::Tabs),
            RETRY_PERSISTENCE_ID => {
                Some(WidgetAction::Settings(SettingsViewAction::RetryPersistence))
            }
            _ => Some(WidgetAction::Consumed),
        }
    }

    fn map_view_mode(&mut self, view_mode: ViewMode) -> Option<WidgetAction> {
        self.input.view_mode = view_mode;
        self.validation = None;
        Some(WidgetAction::Settings(SettingsViewAction::SetViewMode(view_mode)))
    }

    fn handle_toggle(&mut self, id: WidgetId, checked: bool) -> Option<WidgetAction> {
        match id {
            WORD_WRAP_ID => {
                self.input.word_wrap = checked;
                Some(WidgetAction::Settings(SettingsViewAction::SetWordWrap(checked)))
            }
            LINE_NUMBERS_ID => {
                self.input.show_line_numbers = checked;
                Some(WidgetAction::Settings(SettingsViewAction::SetShowLineNumbers(checked)))
            }
            STATUS_BAR_ID => {
                self.input.show_status_bar = checked;
                Some(WidgetAction::Settings(SettingsViewAction::SetShowStatusBar(checked)))
            }
            _ => Some(WidgetAction::Consumed),
        }
    }

    fn handle_text_commit(&mut self, id: WidgetId, value: TextPayload) -> Option<WidgetAction> {
        let TextPayload::Plain(value) = value else {
            self.set_validation(id, ValidationError::InvalidNumber);
            return None;
        };
        match id {
            FONT_FAMILY_ID => self.map_font_family(id, &value),
            FONT_SIZE_ID => self.map_font_size(id, &value),
            LINE_HEIGHT_RATIO_ID => self.map_line_height_ratio(id, &value),
            TAB_WIDTH_ID => self.map_tab_width(id, &value),
            _ => Some(WidgetAction::Consumed),
        }
    }

    fn map_font_family(&mut self, id: WidgetId, value: &str) -> Option<WidgetAction> {
        match parse_font_family(value) {
            Ok(font_family) => {
                self.input.font_family = font_family.clone();
                self.validation = None;
                Some(WidgetAction::Settings(SettingsViewAction::SetFontFamily(font_family)))
            }
            Err(error) => {
                self.set_validation(id, error);
                None
            }
        }
    }

    fn map_font_size(&mut self, id: WidgetId, value: &str) -> Option<WidgetAction> {
        match parse_font_size(value) {
            Ok(font_size) => {
                self.input.font_size = font_size;
                self.validation = None;
                Some(WidgetAction::Settings(SettingsViewAction::SetFontSize(font_size)))
            }
            Err(error) => {
                self.set_validation(id, error);
                None
            }
        }
    }

    fn map_line_height_ratio(&mut self, id: WidgetId, value: &str) -> Option<WidgetAction> {
        match parse_line_height_ratio(value) {
            Ok(line_height_ratio) => {
                self.input.line_height_ratio = line_height_ratio;
                self.validation = None;
                Some(WidgetAction::Settings(SettingsViewAction::SetLineHeightRatio(
                    line_height_ratio,
                )))
            }
            Err(error) => {
                self.set_validation(id, error);
                None
            }
        }
    }

    fn map_tab_width(&mut self, id: WidgetId, value: &str) -> Option<WidgetAction> {
        match parse_tab_width(value) {
            Ok(tab_width) => {
                self.input.tab_width = tab_width;
                self.validation = None;
                Some(WidgetAction::Settings(SettingsViewAction::SetTabWidth(tab_width)))
            }
            Err(error) => {
                self.set_validation(id, error);
                None
            }
        }
    }

    fn set_validation(&mut self, field_id: WidgetId, error: ValidationError) {
        self.validation = Some(FieldValidation { field_id, error });
    }

    fn dispatch_form_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let action = self.form.on_event(&Self::form_event(event, self.form_rect), ctx)?;
        match action {
            WidgetAction::Control(control_action) => {
                if let ControlAction::FocusRequested { id } = control_action {
                    self.set_focused_control(Some(id));
                    return Some(WidgetAction::Consumed);
                }
                self.handle_control_action(control_action)
            }
            other => Some(other),
        }
    }

    fn dispatch_active_page_event(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        self.dispatch_form_event(event, ctx)
    }

    fn dispatch_banner_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let banner = self.persistence_banner.as_mut()?;
        let local_event = Self::form_event(event, self.persistence_banner_rect);
        let action = banner.on_event(&local_event, ctx)?;
        match action {
            WidgetAction::Control(control_action) => self.handle_control_action(control_action),
            other => Some(other),
        }
    }

    fn hover_target_at(&self, px: f32, py: f32) -> Option<SettingsHoverTarget> {
        if self.persistence_banner_rect.contains(px, py) && self.persistence_banner.is_some() {
            return Some(SettingsHoverTarget::PersistenceBanner);
        }
        if let Some(index) = self.category_index_at(px, py) {
            return Some(SettingsHoverTarget::Category(index));
        }
        self.form_rect.contains(px, py).then_some(SettingsHoverTarget::Content)
    }

    fn dispatch_to_hover_target(
        &mut self,
        target: SettingsHoverTarget,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        match target {
            SettingsHoverTarget::Category(index) => self.dispatch_category_event(index, event, ctx),
            SettingsHoverTarget::Content => self.dispatch_active_page_event(event, ctx),
            SettingsHoverTarget::PersistenceBanner => self.dispatch_banner_event(event, ctx),
        }
    }

    fn dispatch_interaction_lifecycle(
        &mut self,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let container_changed = if matches!(event, Event::InteractionCancel) {
            self.category_pointer_index.take().is_some() | self.hover_target.take().is_some()
        } else {
            self.hover_target.take().is_some()
        };
        let mut first_action = None;
        for category_index in 0..self.category_buttons.len() {
            if let Some(action) = self.dispatch_category_event(category_index, event, ctx)
                && first_action.is_none()
            {
                first_action = Some(action);
            }
        }
        if let Some(action) = self.dispatch_active_page_event(event, ctx)
            && first_action.is_none()
        {
            first_action = Some(action);
        }
        if let Some(action) = self.dispatch_banner_event(event, ctx)
            && first_action.is_none()
        {
            first_action = Some(action);
        }
        first_action.or_else(|| container_changed.then_some(WidgetAction::Consumed))
    }
}

impl Widget for SettingsView {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        let settings_theme = ctx.theme.settings_theme();
        let theme_changed = self.settings_theme != settings_theme;
        self.settings_theme = settings_theme;
        if theme_changed {
            self.form_needs_layout = true;
            if self.persistence_banner.is_some() {
                self.refresh_persistence_banner();
            }
        }
        let compact_layout = self.rect.w < SETTINGS_COMPACT_LAYOUT_THRESHOLD_LOGICAL * ctx.dpi;
        let sidebar_width_logical = if compact_layout {
            SETTINGS_COMPACT_SIDEBAR_WIDTH_LOGICAL
        } else {
            SETTINGS_SIDEBAR_WIDTH_LOGICAL
        };
        let sidebar_width = if self.category_navigation_visible {
            (sidebar_width_logical * ctx.dpi).min(self.rect.w)
        } else {
            0.0
        };
        self.sidebar_width = sidebar_width;
        self.category_rects.clear();
        if self.category_navigation_visible {
            let mut category_y = SETTINGS_SIDEBAR_TOP_INSET_LOGICAL * ctx.dpi;
            for (_, button) in &mut self.category_buttons {
                button.set_style(category_button_style(settings_theme));
                let category_rect = Rect::new(
                    SETTINGS_CATEGORY_HORIZONTAL_INSET_LOGICAL * ctx.dpi,
                    category_y,
                    (sidebar_width - 2.0 * SETTINGS_CATEGORY_HORIZONTAL_INSET_LOGICAL * ctx.dpi)
                        .max(0.0),
                    SETTINGS_CATEGORY_BUTTON_HEIGHT_LOGICAL * ctx.dpi,
                );
                self.category_rects.push(category_rect);
                button.set_rect(Rect::new(0.0, 0.0, category_rect.w, category_rect.h), ctx);
                category_y += category_rect.h + SETTINGS_CATEGORY_BUTTON_GAP_LOGICAL * ctx.dpi;
            }
        }

        let banner_height = self
            .persistence_banner
            .as_ref()
            .map(|_| (SETTINGS_BANNER_HEIGHT_LOGICAL * ctx.dpi).min(self.rect.h))
            .unwrap_or(0.0);
        let banner_gap =
            if banner_height > 0.0 { SETTINGS_BANNER_GAP_LOGICAL * ctx.dpi } else { 0.0 };
        let form_inset_logical = if compact_layout {
            SETTINGS_COMPACT_FORM_INSET_LOGICAL
        } else {
            SETTINGS_FORM_INSET_LOGICAL
        };
        let form_gap_logical = if compact_layout {
            SETTINGS_COMPACT_FORM_GAP_LOGICAL
        } else {
            SETTINGS_FORM_GAP_LOGICAL
        };
        let form_inset = form_inset_logical * ctx.dpi;
        let form_gap = form_gap_logical * ctx.dpi;
        self.form_rect = if self.category_navigation_visible {
            Rect::new(
                sidebar_width + form_gap + form_inset,
                form_inset,
                (self.rect.w - sidebar_width - form_gap - form_inset * 2.0).max(0.0),
                (self.rect.h - banner_height - banner_gap - form_inset * 2.0).max(0.0),
            )
        } else {
            Rect::new(
                form_inset,
                form_inset,
                (self.rect.w - form_inset * 2.0).max(0.0),
                (self.rect.h - banner_height - banner_gap - form_inset * 2.0).max(0.0),
            )
        };
        self.persistence_banner_rect = Rect::new(
            self.form_rect.x,
            self.form_rect.y + self.form_rect.h + banner_gap,
            self.form_rect.w,
            banner_height,
        );
        if self.form_needs_layout {
            self.rebuild_form(ctx);
        }
        self.form.set_rect(Rect::new(0.0, 0.0, self.form_rect.w, self.form_rect.h), ctx);
        if let Some(banner) = self.persistence_banner.as_mut() {
            banner.set_rect(
                Rect::new(0.0, 0.0, self.persistence_banner_rect.w, self.persistence_banner_rect.h),
                ctx,
            );
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let settings_theme = ctx.theme.settings_theme();
        if self.category_navigation_visible {
            ctx.list.fill(
                Rect::new(0.0, 0.0, self.sidebar_width, self.rect.h),
                settings_theme.sidebar_surface,
            );

            let sidebar_separator_width = SETTINGS_SIDEBAR_SEPARATOR_WIDTH_LOGICAL * ctx.dpi;
            ctx.list.fill(
                Rect::new(
                    (self.sidebar_width - sidebar_separator_width).max(0.0),
                    0.0,
                    sidebar_separator_width,
                    self.rect.h,
                ),
                settings_theme.separator,
            );

            for ((_, button), rect) in self.category_buttons.iter().zip(&self.category_rects) {
                let saved_offset = ctx.list.offset;
                ctx.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
                button.paint(ctx);
                ctx.list.offset = saved_offset;
            }
        }

        let saved_offset = ctx.list.offset;
        ctx.list.offset = (saved_offset.0 + self.form_rect.x, saved_offset.1 + self.form_rect.y);
        self.form.paint(ctx);
        ctx.list.offset = saved_offset;

        if let Some(banner) = &self.persistence_banner {
            let saved_offset = ctx.list.offset;
            ctx.list.offset = (
                saved_offset.0 + self.persistence_banner_rect.x,
                saved_offset.1 + self.persistence_banner_rect.y,
            );
            banner.paint(ctx);
            ctx.list.offset = saved_offset;
        }
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        if self.category_navigation_visible {
            for (_, button) in &self.category_buttons {
                button.collect_focusable_ids(output);
            }
        }
        self.form.collect_focusable_ids(output);
        if let Some(banner) = &self.persistence_banner {
            banner.collect_focusable_ids(output);
        }
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.set_focused_control(focused_id);
    }

    fn collect_accessibility_nodes(
        &self,
        context: &AccessibilityContext,
        output: &mut Vec<AccessibilityNode>,
    ) {
        if self.category_navigation_visible {
            for ((_, button), rect) in self.category_buttons.iter().zip(&self.category_rects) {
                if rect.w > 0.0 && rect.h > 0.0 {
                    button.collect_accessibility_nodes(&context.offset_by(rect.x, rect.y), output);
                }
            }
        }
        if self.form_rect.w > 0.0 && self.form_rect.h > 0.0 {
            self.form.collect_accessibility_nodes(
                &context.offset_by(self.form_rect.x, self.form_rect.y),
                output,
            );
        }
        if let Some(banner) = &self.persistence_banner
            && self.persistence_banner_rect.w > 0.0
            && self.persistence_banner_rect.h > 0.0
        {
            banner.collect_accessibility_nodes(
                &context.offset_by(self.persistence_banner_rect.x, self.persistence_banner_rect.y),
                output,
            );
        }
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        if self.category_navigation_visible {
            for index in 0..self.category_buttons.len() {
                let action = self.category_buttons[index].1.on_accessibility_action(request);
                if let Some(action) = action {
                    return match action {
                        WidgetAction::Control(control_action) => {
                            self.handle_control_action(control_action)
                        }
                        other => Some(other),
                    };
                }
            }
        }
        if let Some(action) = self.form.on_accessibility_action(request) {
            return match action {
                WidgetAction::Control(control_action) => self.handle_control_action(control_action),
                other => Some(other),
            };
        }
        let action = self.persistence_banner.as_mut()?.on_accessibility_action(request)?;
        match action {
            WidgetAction::Control(control_action) => self.handle_control_action(control_action),
            other => Some(other),
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        if matches!(event, Event::PointerLeave | Event::InteractionCancel) {
            return self.dispatch_interaction_lifecycle(event, ctx);
        }

        if matches!(event, Event::KeyDown(..)) {
            if let Some(index) = self.category_buttons.iter().position(|(_, button)| {
                button.id() == self.focused_id && self.category_navigation_visible
            }) {
                return self.dispatch_category_event(index, event, ctx);
            }
            if self.focused_id == Some(RETRY_PERSISTENCE_ID) && self.persistence_banner.is_some() {
                return self.dispatch_banner_event(event, ctx);
            }
        }

        if self.persistence_banner.as_ref().is_some_and(Widget::is_capturing)
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            return self.dispatch_banner_event(event, ctx);
        }

        if let Some(index) = self.category_pointer_index
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            let action = self.dispatch_category_event(index, event, ctx);
            if matches!(event, Event::MouseUp { .. }) {
                self.category_pointer_index = None;
            }
            return action;
        }

        match event {
            Event::MouseDown { px, py, .. } => {
                if self.persistence_banner_rect.contains(*px, *py) {
                    return self.dispatch_banner_event(event, ctx);
                }
                if let Some(index) = self.category_index_at(*px, *py) {
                    self.category_pointer_index = Some(index);
                    return self.dispatch_category_event(index, event, ctx);
                }
                self.dispatch_active_page_event(event, ctx)
            }
            Event::MouseMove { px, py } => {
                let next_hover_target = self.hover_target_at(*px, *py);
                let previous_hover_action = if self.hover_target != next_hover_target {
                    self.hover_target.and_then(|target| {
                        let saved_cursor_hint = ctx.cursor_hint;
                        let action = self.dispatch_to_hover_target(target, event, ctx);
                        ctx.cursor_hint = saved_cursor_hint;
                        action
                    })
                } else {
                    None
                };
                self.hover_target = next_hover_target;

                if let Some(target) = next_hover_target {
                    return self
                        .dispatch_to_hover_target(target, event, ctx)
                        .or(previous_hover_action);
                }

                previous_hover_action
            }
            Event::MouseUp { .. } => self.dispatch_active_page_event(event, ctx),
            _ => self.dispatch_active_page_event(event, ctx),
        }
    }

    fn is_capturing(&self) -> bool {
        self.category_pointer_index.is_some()
            || self.form.is_capturing()
            || self.persistence_banner.as_ref().is_some_and(Widget::is_capturing)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn action_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        font_size_logical: SETTINGS_BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: SETTINGS_BUTTON_PADDING_LOGICAL,
        foreground: settings.text_primary,
        selected_foreground: settings.text_primary,
        background: settings.control_surface,
        border: settings.control_border,
        hover_background: blend_color(
            settings.control_surface,
            settings.accent,
            SETTINGS_ACTION_HOVER_ACCENT_BLEND,
        ),
        pressed_background: blend_color(
            settings.control_surface,
            settings.accent,
            SETTINGS_ACTION_PRESSED_ACCENT_BLEND,
        ),
        selected_background: settings.control_surface,
        disabled_foreground: with_alpha(settings.text_primary, SETTINGS_DISABLED_FOREGROUND_ALPHA),
        disabled_background: settings.control_surface,
        corner_radius_logical: SETTINGS_BUTTON_RADIUS_LOGICAL,
    }
}

fn segmented_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        font_size_logical: SETTINGS_BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: SETTINGS_BUTTON_PADDING_LOGICAL,
        foreground: settings.text_primary,
        selected_foreground: settings.text_inverse,
        background: SETTINGS_TRANSPARENT,
        border: SETTINGS_TRANSPARENT,
        hover_background: blend_color(
            settings.modal_surface,
            settings.accent,
            SETTINGS_SEGMENT_HOVER_ACCENT_BLEND,
        ),
        pressed_background: blend_color(
            settings.modal_surface,
            settings.accent,
            SETTINGS_SEGMENT_PRESSED_ACCENT_BLEND,
        ),
        selected_background: settings.accent,
        disabled_foreground: with_alpha(settings.text_primary, SETTINGS_DISABLED_FOREGROUND_ALPHA),
        disabled_background: SETTINGS_TRANSPARENT,
        corner_radius_logical: SETTINGS_BUTTON_RADIUS_LOGICAL,
    }
}

fn category_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        font_size_logical: SETTINGS_BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: SETTINGS_BUTTON_PADDING_LOGICAL,
        foreground: settings.text_primary,
        selected_foreground: settings.accent,
        background: SETTINGS_TRANSPARENT,
        border: SETTINGS_TRANSPARENT,
        hover_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            SETTINGS_CATEGORY_HOVER_ACCENT_BLEND,
        ),
        pressed_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            SETTINGS_CATEGORY_PRESSED_ACCENT_BLEND,
        ),
        selected_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            SETTINGS_CATEGORY_SELECTED_ACCENT_BLEND,
        ),
        disabled_foreground: with_alpha(settings.text_primary, SETTINGS_DISABLED_FOREGROUND_ALPHA),
        disabled_background: SETTINGS_TRANSPARENT,
        corner_radius_logical: SETTINGS_BUTTON_RADIUS_LOGICAL,
    }
}

fn blend_color(base: [f32; 4], accent: [f32; 4], accent_factor: f32) -> [f32; 4] {
    let base_factor = 1.0 - accent_factor;
    [
        base[0] * base_factor + accent[0] * accent_factor,
        base[1] * base_factor + accent[1] * accent_factor,
        base[2] * base_factor + accent[2] * accent_factor,
        base[3] * base_factor + accent[3] * accent_factor,
    ]
}

fn with_alpha(mut color: [f32; 4], alpha: f32) -> [f32; 4] {
    color[3] *= alpha;
    color
}

fn fallback_settings_theme() -> SettingsTheme {
    crate::theme::test_theme().settings_theme()
}

fn settings_text_box(id: WidgetId) -> TextBox {
    let mut text_box = TextBox::with_id(id);
    text_box
        .set_fixed_size_logical(SETTINGS_TEXT_BOX_WIDTH_LOGICAL, SETTINGS_CONTROL_HEIGHT_LOGICAL);
    text_box.set_blink(true);
    text_box
}

fn settings_row_style() -> FormRowStyle {
    FormRowStyle {
        min_height_logical: SETTINGS_ROW_HEIGHT_LOGICAL,
        label_width_logical: SETTINGS_ROW_LABEL_WIDTH_LOGICAL,
        column_gap_logical: SETTINGS_ROW_COLUMN_GAP_LOGICAL,
        responsive_threshold_logical: 0.0,
        padding_logical: [
            SETTINGS_ROW_VERTICAL_INSET_LOGICAL,
            SETTINGS_ROW_HORIZONTAL_INSET_LOGICAL,
            SETTINGS_ROW_VERTICAL_INSET_LOGICAL,
            SETTINGS_ROW_HORIZONTAL_INSET_LOGICAL,
        ],
        ..FormRowStyle::default()
    }
}

fn settings_section_style() -> FormSectionStyle {
    FormSectionStyle {
        title_gap_logical: SETTINGS_SECTION_TITLE_GAP_LOGICAL,
        description_gap_logical: SETTINGS_SECTION_DESCRIPTION_GAP_LOGICAL,
        row_height_logical: SETTINGS_ROW_HEIGHT_LOGICAL,
        corner_radius_logical: SETTINGS_SECTION_CORNER_RADIUS_LOGICAL,
        ..FormSectionStyle::default()
    }
}

fn section_title_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SETTINGS_SECTION_TITLE_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::MEDIUM,
            ..LabelStyle::default()
        },
    )
}

fn row_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SETTINGS_ROW_LABEL_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::MEDIUM,
            ..LabelStyle::default()
        },
    )
}

fn description_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SETTINGS_DESCRIPTION_FONT_SIZE_LOGICAL,
            foreground: LabelForeground::ThemeMuted,
            ..LabelStyle::default()
        },
    )
}

fn section_description_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SETTINGS_DESCRIPTION_FONT_SIZE_LOGICAL - 0.5,
            foreground: LabelForeground::ThemeMuted,
            ..LabelStyle::default()
        },
    )
}

fn format_float(value: f32) -> String {
    format!("{value:.3}").trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::super::types::SettingsPersistenceView;
    use super::*;
    use crate::core::measure::NoopMeasure;
    use crate::core::paint::{DrawCmd, DrawList};
    use crate::core::{KeyCode, Modifiers};
    use crate::view_mode::ViewMode;

    fn settings_fixture(category: SettingsCategory) -> SettingsView {
        let input = SettingsViewInput {
            theme_mode: ThemeMode::System,
            font_family: "Menlo".into(),
            font_size: 15.0,
            line_height_ratio: 1.618,
            word_wrap: true,
            show_line_numbers: true,
            tab_width: 4,
            view_mode: ViewMode::Sidebar,
            show_status_bar: false,
            persistence: SettingsPersistenceView::Saved,
        };
        let mut view = SettingsView::new(input);
        view.active_category = category;
        view
    }

    fn layout_settings_view(view: &mut SettingsView, theme: &crate::theme::Theme, rect: Rect) {
        let mut measure = NoopMeasure;
        let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi: 1.0 };
        view.set_rect(rect, &mut layout);
    }

    fn dispatch_settings_event(view: &mut SettingsView, event: Event) -> Option<WidgetAction> {
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        view.on_event(&event, &mut ctx)
    }

    fn semantic_node_with_id(
        nodes: &[crate::core::AccessibilityNode],
        id: crate::core::AccessibilityId,
    ) -> Option<&crate::core::AccessibilityNode> {
        nodes.iter().find_map(|node| {
            if node.id == id {
                return Some(node);
            }
            semantic_node_with_id(&node.children, id)
        })
    }

    #[test]
    fn appearance_category_uses_selected_buttons_and_validated_textboxes() {
        let mut view = settings_fixture(SettingsCategory::Appearance);
        assert!(view.category_is_selected(SettingsCategory::Appearance));
        assert_eq!(
            view.handle_control_action(ControlAction::Activated { id: THEME_DARK_ID }),
            Some(WidgetAction::Settings(SettingsViewAction::SetThemeMode(ThemeMode::Dark)))
        );
        assert_eq!(
            view.handle_control_action(ControlAction::TextCommitted {
                id: FONT_SIZE_ID,
                value: TextPayload::Plain("18".into()),
            }),
            Some(WidgetAction::Settings(SettingsViewAction::SetFontSize(18.0)))
        );
        assert!(
            view.handle_control_action(ControlAction::TextCommitted {
                id: FONT_SIZE_ID,
                value: TextPayload::Plain("999".into()),
            })
            .is_none()
        );
        assert!(view.has_validation_error());
    }

    #[test]
    fn settings_semantics_reach_nested_fields_and_reuse_category_actions() {
        let theme = crate::theme::test_theme();
        let mut view = settings_fixture(SettingsCategory::Appearance);
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 900.0, 600.0));
        let mut nodes = Vec::new();
        view.collect_accessibility_nodes(
            &crate::core::AccessibilityContext::new(20.0, 30.0),
            &mut nodes,
        );

        let font_size_id = crate::core::AccessibilityId::from(FONT_SIZE_ID);
        let font_size = semantic_node_with_id(&nodes, font_size_id)
            .expect("nested font-size field must be exposed");
        assert_eq!(font_size.role, crate::core::AccessibilityRole::TextField);
        let [label_id] = font_size.labelled_by.as_slice() else {
            panic!("font-size field must reference its row label");
        };
        assert_eq!(
            semantic_node_with_id(&nodes, *label_id).and_then(|label| label.name.as_deref()),
            Some("字号")
        );
        assert!(font_size.bounds.x >= 20.0);
        assert!(font_size.bounds.y >= 30.0);

        let editor_category_id = crate::core::AccessibilityId::from(EDITOR_CATEGORY_ID);
        let mut root = crate::core::AccessibilityNode::new(
            crate::core::AccessibilityId(0x7365_7474_696e_6773),
            crate::core::AccessibilityRole::Group,
            Rect::new(20.0, 30.0, 900.0, 600.0),
        );
        root.children = nodes;
        assert_eq!(crate::core::AccessibilityTree::new(root, None).validate(), Ok(()));
        assert_eq!(
            view.on_accessibility_action(&crate::core::AccessibilityActionRequest::new(
                editor_category_id,
                crate::core::AccessibilityAction::Activate,
            )),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(view.active_category(), SettingsCategory::Editor);
    }

    #[test]
    fn editor_and_interface_controls_emit_typed_actions() {
        let mut editor = settings_fixture(SettingsCategory::Editor);
        assert_eq!(
            editor
                .handle_control_action(ControlAction::Toggled { id: WORD_WRAP_ID, checked: false }),
            Some(WidgetAction::Settings(SettingsViewAction::SetWordWrap(false)))
        );
        assert_eq!(
            editor.handle_control_action(ControlAction::Toggled {
                id: LINE_NUMBERS_ID,
                checked: false,
            }),
            Some(WidgetAction::Settings(SettingsViewAction::SetShowLineNumbers(false)))
        );
        assert_eq!(
            editor.handle_control_action(ControlAction::TextCommitted {
                id: TAB_WIDTH_ID,
                value: TextPayload::Plain("8".into()),
            }),
            Some(WidgetAction::Settings(SettingsViewAction::SetTabWidth(8)))
        );

        let mut interface = settings_fixture(SettingsCategory::Interface);
        assert_eq!(
            interface.handle_control_action(ControlAction::Activated { id: VIEW_TABS_ID }),
            Some(WidgetAction::Settings(SettingsViewAction::SetViewMode(ViewMode::Tabs)))
        );
        assert_eq!(
            interface
                .handle_control_action(ControlAction::Toggled { id: STATUS_BAR_ID, checked: true }),
            Some(WidgetAction::Settings(SettingsViewAction::SetShowStatusBar(true)))
        );
    }

    #[test]
    fn save_failure_shows_icon_label_and_retry_button() {
        let mut input = SettingsViewInput {
            theme_mode: ThemeMode::System,
            font_family: "Menlo".into(),
            font_size: 15.0,
            line_height_ratio: 1.618,
            word_wrap: true,
            show_line_numbers: true,
            tab_width: 4,
            view_mode: ViewMode::Sidebar,
            show_status_bar: false,
            persistence: SettingsPersistenceView::Saved,
        };
        input.persistence =
            SettingsPersistenceView::SaveFailed { message: "permission denied".into() };
        let mut view = SettingsView::new(input);
        assert!(view.visible_text().contains("当前修改尚未保存"));
        assert_eq!(
            view.handle_control_action(ControlAction::Activated { id: RETRY_PERSISTENCE_ID }),
            Some(WidgetAction::Settings(SettingsViewAction::RetryPersistence))
        );
    }

    #[test]
    fn settings_view_layout_builds_appearance_form() {
        let mut view = settings_fixture(SettingsCategory::Appearance);
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        view.set_rect(Rect::new(0.0, 0.0, 900.0, 600.0), &mut ctx);
        assert!(!view.form_needs_layout);
        assert!(view.form_rect.w > 0.0);
    }

    #[test]
    fn settings_view_exposes_only_generic_categories() {
        let view = settings_fixture(SettingsCategory::Appearance);
        let categories: Vec<SettingsCategory> =
            view.category_buttons.iter().map(|(category, _)| *category).collect();

        assert_eq!(
            categories,
            vec![
                SettingsCategory::Appearance,
                SettingsCategory::Editor,
                SettingsCategory::Interface,
            ],
        );
    }

    #[test]
    fn clicking_editor_category_focuses_its_first_field_after_layout() {
        let mut view = settings_fixture(SettingsCategory::Appearance);
        let theme = crate::theme::test_theme();
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 720.0, 480.0));
        click_category_button(&mut view, 1);
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 720.0, 480.0));

        assert_eq!(view.active_category(), SettingsCategory::Editor);
        assert_eq!(view.form.focused_id(), Some(WORD_WRAP_ID));
    }

    #[test]
    fn category_press_survives_pointer_leave_and_is_cleared_by_interaction_cancel() {
        let mut view = settings_fixture(SettingsCategory::Appearance);
        let theme = crate::theme::test_theme();
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 720.0, 480.0));
        let editor_rect = view.category_rects[1];
        let pointer = (editor_rect.x + editor_rect.w * 0.5, editor_rect.y + editor_rect.h * 0.5);

        assert!(
            dispatch_settings_event(
                &mut view,
                Event::MouseDown {
                    px: pointer.0,
                    py: pointer.1,
                    button: crate::core::widget::MouseButton::Left,
                },
            )
            .is_some()
        );
        assert!(view.is_capturing());

        dispatch_settings_event(&mut view, Event::PointerLeave);
        assert!(view.is_capturing());

        assert_eq!(
            dispatch_settings_event(&mut view, Event::InteractionCancel),
            Some(WidgetAction::Consumed)
        );
        assert!(!view.is_capturing());
        assert_eq!(dispatch_settings_event(&mut view, Event::InteractionCancel), None);
        assert_eq!(
            dispatch_settings_event(
                &mut view,
                Event::MouseUp {
                    px: pointer.0,
                    py: pointer.1,
                    button: crate::core::widget::MouseButton::Left,
                },
            ),
            None
        );
        assert_eq!(view.active_category(), SettingsCategory::Appearance);
    }

    fn click_category_button(view: &mut SettingsView, category_index: usize) {
        let category_rect = view.category_rects[category_index];
        let click_x = category_rect.x + category_rect.w * 0.5;
        let click_y = category_rect.y + category_rect.h * 0.5;
        click_at(view, click_x, click_y);
    }

    fn click_at(view: &mut SettingsView, click_x: f32, click_y: f32) {
        let mouse_button = crate::core::widget::MouseButton::Left;

        dispatch_settings_event(
            view,
            Event::MouseDown { px: click_x, py: click_y, button: mouse_button },
        );
        dispatch_settings_event(
            view,
            Event::MouseUp { px: click_x, py: click_y, button: mouse_button },
        );
    }

    fn paint_settings_view_for_test(view: &SettingsView, theme: &crate::theme::Theme) -> DrawList {
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        view.paint(&mut PaintCtx {
            list: &mut draw_list,
            theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        });
        draw_list
    }

    #[test]
    fn settings_form_uses_balanced_insets_and_sidebar_width() {
        let mut view = settings_fixture(SettingsCategory::Editor);
        let theme = crate::theme::test_theme();
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 680.0, 480.0));

        assert_eq!(view.sidebar_width, 160.0);
        assert_eq!(view.form_rect, Rect::new(200.0, 24.0, 456.0, 432.0));
    }

    #[test]
    fn hidden_category_navigation_does_not_layout_paint_or_hit_category_buttons() {
        let mut view = settings_fixture(SettingsCategory::Appearance);
        let theme = crate::theme::test_theme();
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 600.0, 400.0));
        let former_editor_category_rect = view.category_rects[1];

        view.set_category_navigation_visible(false);
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 600.0, 400.0));

        assert!(view.category_rects.is_empty());
        assert_eq!(view.sidebar_width, 0.0);
        assert_eq!(
            view.category_index_at(former_editor_category_rect.x, former_editor_category_rect.y),
            None
        );
        let draw_list = paint_settings_view_for_test(&view, &theme);
        assert!(!draw_list.cmds.iter().any(|command| {
            matches!(command, DrawCmd::FillRect { rect, .. } if *rect == Rect::new(0.0, 0.0, 160.0, 400.0))
        }));

        click_at(&mut view, former_editor_category_rect.x, former_editor_category_rect.y);
        assert_eq!(view.active_category(), SettingsCategory::Appearance);
    }

    #[test]
    fn external_category_switch_rebuilds_the_editor_form() {
        let mut view = settings_fixture(SettingsCategory::Appearance);
        let theme = crate::theme::test_theme();
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 720.0, 480.0));

        view.set_active_category(SettingsCategory::Editor);
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 720.0, 480.0));

        assert_eq!(view.active_category(), SettingsCategory::Editor);
        assert_eq!(view.form.focused_id(), Some(WORD_WRAP_ID));
    }

    #[test]
    fn editor_rows_keep_labels_and_descriptions_separate_in_two_columns() {
        let view = settings_fixture(SettingsCategory::Editor);
        let mut row = view.word_wrap_row();
        let theme = crate::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut ctx =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };

        row.set_rect(Rect::new(0.0, 0.0, 456.0, SETTINGS_ROW_HEIGHT_LOGICAL), &mut ctx);

        let description_rect = row.description_rect().expect("editor row has a description");
        assert_eq!(row.layout_mode(), crate::widgets::form::FormRowLayoutMode::Columns);
        assert!(row.label_rect().bottom() <= description_rect.y);
        assert!(description_rect.bottom() <= SETTINGS_ROW_HEIGHT_LOGICAL);
        assert!(row.label_rect().right() < row.control_rect().x);
    }

    #[test]
    fn segmented_buttons_use_inverse_selected_text_and_visible_hover() {
        let settings = crate::theme::test_theme().settings_theme();
        let style = segmented_button_style(settings);

        assert_eq!(style.foreground, settings.text_primary);
        assert_eq!(style.selected_foreground, settings.text_inverse);
        assert_eq!(style.selected_background, settings.accent);
        assert_eq!(style.background, SETTINGS_TRANSPARENT);
        let hover_contrast: f32 = style
            .hover_background
            .iter()
            .zip(settings.modal_surface)
            .map(|(hover_channel, surface_channel)| (hover_channel - surface_channel).abs())
            .sum();
        assert!(hover_contrast >= 0.15, "hover contrast {hover_contrast} is too subtle");
    }

    #[test]
    fn category_navigation_uses_spacious_geometry_and_interaction_states() {
        let mut view = settings_fixture(SettingsCategory::Appearance);
        let theme = crate::theme::test_theme();
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 600.0, 400.0));

        assert_eq!(view.category_rects[0].x, 10.0);
        assert_eq!(view.category_rects[0].y, 12.0);
        assert_eq!(view.category_rects[0].h, 34.0);
        assert_eq!(view.category_rects[1].y - view.category_rects[0].bottom(), 4.0);
        let category_style = category_button_style(theme.settings_theme());
        assert_eq!(category_style.background, SETTINGS_TRANSPARENT);
        assert_eq!(category_style.border, SETTINGS_TRANSPARENT);
        assert_ne!(category_style.hover_background, SETTINGS_TRANSPARENT);
        assert_ne!(category_style.pressed_background, SETTINGS_TRANSPARENT);
        assert_ne!(category_style.selected_background, SETTINGS_TRANSPARENT);
        assert_eq!(category_style.selected_foreground, theme.settings_theme().accent);

        let mut draw_list = DrawList::new();
        view.paint(&mut PaintCtx::new(&mut draw_list, &theme, 1.0));
        assert!(matches!(
            draw_list.cmds.first(),
            Some(DrawCmd::FillRect { rect, .. }) if rect.w == 160.0
        ));
    }

    #[test]
    fn clicking_appearance_text_box_focuses_it_and_accepts_typed_input() {
        use crate::core::widget::{KeyCode, Modifiers};

        for dpi in [1.0, 2.0] {
            let mut view = settings_fixture(SettingsCategory::Appearance);
            let theme = crate::theme::test_theme();
            let mut measure = NoopMeasure;
            let mut layout =
                LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi };
            view.set_rect(Rect::new(0.0, 0.0, 1440.0 * dpi / 2.0, 880.0 * dpi / 2.0), &mut layout);

            let mut draw_list = DrawList::new();
            let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
            view.paint(&mut PaintCtx {
                list: &mut draw_list,
                theme: &theme,
                dpi,
                offset: (0.0, 0.0),
                global_alpha: 1.0,
                shaper: Some(&mut shaper),
            });
            let text_box_rect = {
                let text_index = draw_list
                    .cmds
                    .iter()
                    .rposition(|command| {
                        matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == "15")
                    })
                    .expect("expected text box text to be painted");
                draw_list.cmds[..text_index]
                    .iter()
                    .rev()
                    .find_map(|command| match command {
                        DrawCmd::FillRect { rect, radius, .. } if *radius == 3.0 * dpi => {
                            Some(*rect)
                        }
                        _ => None,
                    })
                    .expect("expected text box background before its text")
            };
            assert_eq!(
                text_box_rect.h,
                SETTINGS_CONTROL_HEIGHT_LOGICAL * dpi,
                "dpi={dpi} 时输入框应渲染为控件高度",
            );
            let click_x = text_box_rect.x + text_box_rect.w * 0.5;
            let click_y = text_box_rect.y + text_box_rect.h * 0.5;
            let mouse_button = crate::core::widget::MouseButton::Left;
            let mut ctx = EventCtx::new(&theme, dpi);
            let _ = view.on_event(
                &Event::MouseDown { px: click_x, py: click_y, button: mouse_button },
                &mut ctx,
            );
            let _ = view.on_event(
                &Event::MouseUp { px: click_x, py: click_y, button: mouse_button },
                &mut ctx,
            );
            assert_eq!(view.focused_id, Some(FONT_SIZE_ID), "dpi={dpi} 点击后应聚焦");

            let _ = view.on_event(
                &Event::KeyDown(KeyCode::Char('a'), Modifiers { cmd: true, ..Modifiers::NONE }),
                &mut ctx,
            );
            let typed =
                view.on_event(&Event::KeyDown(KeyCode::Char('9'), Modifiers::NONE), &mut ctx);
            assert_eq!(typed, Some(WidgetAction::Consumed), "dpi={dpi} 按键应被消费");

            let committed =
                view.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut ctx);
            assert_eq!(
                committed,
                Some(WidgetAction::Settings(SettingsViewAction::SetFontSize(9.0))),
                "dpi={dpi} 回车应提交新字号",
            );
        }
    }

    #[test]
    fn theme_switch_restyles_existing_settings_controls() {
        let mut view = settings_fixture(SettingsCategory::Interface);
        let dark_theme = crate::theme::test_theme();
        layout_settings_view(&mut view, &dark_theme, Rect::new(0.0, 0.0, 600.0, 400.0));

        let light_theme =
            crate::theme::Theme::from_definition(&crate::theme::ThemeDefinition::default_light());
        let light_settings = light_theme.settings_theme();
        let mut updated_input = view.input().clone();
        updated_input.theme_mode = ThemeMode::Light;
        updated_input.persistence =
            SettingsPersistenceView::SaveFailed { message: "permission denied".into() };
        view.set_input(updated_input);
        layout_settings_view(&mut view, &light_theme, Rect::new(0.0, 0.0, 600.0, 400.0));

        let mut draw_list = DrawList::new();
        view.paint(&mut PaintCtx::new(&mut draw_list, &light_theme, 1.0));
        let category_selected = category_button_style(light_settings).selected_background;
        let parameter_selected = segmented_button_style(light_settings).selected_background;

        assert_eq!(view.settings_theme, light_settings);
        assert!(draw_list.cmds.iter().any(
            |command| matches!(command, DrawCmd::FillRect { color, .. } if *color == category_selected)
        ));
        assert!(draw_list.cmds.iter().any(
            |command| matches!(command, DrawCmd::FillRect { color, .. } if *color == parameter_selected)
        ));
        assert!(matches!(
            draw_list.cmds.last(),
            Some(DrawCmd::StrokeRect { color, .. }) if *color == light_settings.control_border
        ));
    }

    #[test]
    fn category_and_segmented_buttons_share_settings_focus_protocol() {
        let theme = crate::theme::test_theme();
        let mut view = settings_fixture(SettingsCategory::Appearance);
        layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 600.0, 400.0));

        let mut focusable_ids = Vec::new();
        view.collect_focusable_ids(&mut focusable_ids);
        assert!(focusable_ids.contains(&APPEARANCE_CATEGORY_ID));
        assert!(focusable_ids.contains(&THEME_DARK_ID));

        view.set_keyboard_focus(Some(THEME_DARK_ID));
        let mut ctx = EventCtx::new(&theme, 1.0);
        assert_eq!(
            view.on_event(&Event::KeyDown(KeyCode::Enter, Modifiers::NONE), &mut ctx),
            Some(WidgetAction::Settings(SettingsViewAction::SetThemeMode(ThemeMode::Dark)))
        );
    }
}
