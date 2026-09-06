//! Notora 独立设置界面。
//!
//! 本模块只复用 `ui` 的基础表单控件，不复用 Textora 的 `SettingsView` 业务页面。

use ui::ThemeMode;
use ui::button::{Button, ButtonStyle};
use ui::core::widget::{ControlAction, TextPayload, WidgetId};
use ui::core::{Dock, Event, EventCtx, LayoutCtx, PaintCtx, Rect, Widget, WidgetAction};
use ui::form::{FormRow, FormRowStyle, FormSection, FormSectionStyle, FormView, FormViewStyle};
use ui::inline_group::{CrossAlignment, InlineChild, InlineGroup, MainAlignment};
use ui::label::{Label, LabelForeground, LabelStyle};
use ui::switch::Switch;
use ui::text_box::TextBox;
use ui::theme::SettingsTheme;

use crate::settings_overlay::{
    NotoraSettingsPersistenceView, ProductSettingsUpdate, SettingsOverlayAction,
    SettingsOverlayInput,
};

const SIDEBAR_WIDTH_LOGICAL: f32 = 152.0;
const COMPACT_SIDEBAR_WIDTH_LOGICAL: f32 = 104.0;
const COMPACT_LAYOUT_THRESHOLD_LOGICAL: f32 = 520.0;
const SIDEBAR_TOP_INSET_LOGICAL: f32 = 12.0;
const CATEGORY_HORIZONTAL_INSET_LOGICAL: f32 = 10.0;
const CATEGORY_HEIGHT_LOGICAL: f32 = 34.0;
const CATEGORY_GAP_LOGICAL: f32 = 4.0;
const FORM_INSET_LOGICAL: f32 = 20.0;
const COMPACT_FORM_INSET_LOGICAL: f32 = 12.0;
const FORM_GAP_LOGICAL: f32 = 12.0;
const TEXT_BOX_WIDTH_LOGICAL: f32 = 200.0;
const CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
const SEGMENT_WIDTH_LOGICAL: f32 = 82.0;
const BUTTON_FONT_SIZE_LOGICAL: f32 = 14.0;
const BUTTON_PADDING_LOGICAL: f32 = 12.0;
const BUTTON_RADIUS_LOGICAL: f32 = 8.0;
const ROW_HEIGHT_LOGICAL: f32 = 64.0;
const ROW_LABEL_WIDTH_LOGICAL: f32 = 176.0;
const ROW_COLUMN_GAP_LOGICAL: f32 = 12.0;
const ROW_VERTICAL_INSET_LOGICAL: f32 = 10.0;
const ROW_HORIZONTAL_INSET_LOGICAL: f32 = 16.0;
const SECTION_TITLE_GAP_LOGICAL: f32 = 6.0;
const SECTION_DESCRIPTION_GAP_LOGICAL: f32 = 14.0;
const SECTION_CORNER_RADIUS_LOGICAL: f32 = 10.0;
const SECTION_TITLE_FONT_SIZE_LOGICAL: f32 = 17.0;
const ROW_LABEL_FONT_SIZE_LOGICAL: f32 = 14.0;
const DESCRIPTION_FONT_SIZE_LOGICAL: f32 = 12.0;
const SIDEBAR_SEPARATOR_WIDTH_LOGICAL: f32 = 1.0;
const MESSAGE_HEIGHT_LOGICAL: f32 = 40.0;
const MESSAGE_GAP_LOGICAL: f32 = 10.0;
const RETRY_BUTTON_WIDTH_LOGICAL: f32 = 84.0;
const TRANSPARENT: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
const CATEGORY_HOVER_ACCENT_BLEND: f32 = 0.05;
const CATEGORY_PRESSED_ACCENT_BLEND: f32 = 0.09;
const CATEGORY_SELECTED_ACCENT_BLEND: f32 = 0.14;
const SEGMENT_HOVER_ACCENT_BLEND: f32 = 0.16;
const SEGMENT_PRESSED_ACCENT_BLEND: f32 = 0.14;
const DISABLED_FOREGROUND_ALPHA: f32 = 0.45;

const APPEARANCE_CATEGORY_ID: WidgetId = WidgetId(0x6e6f_746f_6170_7065);
const EDITOR_CATEGORY_ID: WidgetId = WidgetId(0x6e6f_746f_6564_6974);
const INTERFACE_CATEGORY_ID: WidgetId = WidgetId(0x6e6f_746f_696e_7466);
const WORKSPACE_CATEGORY_ID: WidgetId = WidgetId(0x6e6f_746f_776f_726b);
const THEME_SYSTEM_ID: WidgetId = WidgetId(0x6e6f_746f_7468_7379);
const THEME_DARK_ID: WidgetId = WidgetId(0x6e6f_746f_7468_6461);
const THEME_LIGHT_ID: WidgetId = WidgetId(0x6e6f_746f_7468_6c69);
const FONT_FAMILY_ID: WidgetId = WidgetId(0x6e6f_746f_666f_6e74);
const FONT_SIZE_ID: WidgetId = WidgetId(0x6e6f_746f_7369_7a65);
const LINE_HEIGHT_RATIO_ID: WidgetId = WidgetId(0x6e6f_746f_6c69_6e65);
const WORD_WRAP_ID: WidgetId = WidgetId(0x6e6f_746f_7772_6170);
const MARKDOWN_FIRST_LINE_INDENT_ID: WidgetId = WidgetId(0x6e6f_746f_6d64_696e);
const LINE_NUMBERS_ID: WidgetId = WidgetId(0x6e6f_746f_6e75_6d62);
const TAB_WIDTH_ID: WidgetId = WidgetId(0x6e6f_746f_7461_6273);
const STATUS_BAR_ID: WidgetId = WidgetId(0x6e6f_746f_7374_6174);
const RUNTIME_TAB_LIMIT_ID: WidgetId = WidgetId(0x6e6f_746f_7274_6c69);
const AUTO_SAVE_DELAY_ID: WidgetId = WidgetId(0x6e6f_746f_6175_746f);
const CATALOG_BACKUP_RETENTION_ID: WidgetId = WidgetId(0x6e6f_746f_6261_636b);
const RETRY_PERSISTENCE_ID: WidgetId = WidgetId(0x6e6f_746f_7265_7472);

const MINIMUM_FONT_SIZE: f32 = 6.0;
const MAXIMUM_FONT_SIZE: f32 = 72.0;
const MINIMUM_LINE_HEIGHT_RATIO: f32 = 1.0;
const MAXIMUM_LINE_HEIGHT_RATIO: f32 = 3.0;
const MINIMUM_TAB_WIDTH: usize = 1;
const MAXIMUM_TAB_WIDTH: usize = 16;
const MINIMUM_RUNTIME_TAB_LIMIT: usize = 1;
const MAXIMUM_RUNTIME_TAB_LIMIT: usize = 128;
const MINIMUM_AUTO_SAVE_DELAY_MILLIS: u64 = 100;
const MAXIMUM_AUTO_SAVE_DELAY_MILLIS: u64 = 60_000;
const MINIMUM_CATALOG_BACKUP_RETENTION: usize = 1;
const MAXIMUM_CATALOG_BACKUP_RETENTION: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NotoraSettingsCategory {
    #[default]
    Appearance,
    Editor,
    Interface,
    Workspace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FieldValidation {
    message: &'static str,
}

pub(super) struct NotoraSettingsView {
    rect: Rect,
    dpi: f32,
    sidebar_width: f32,
    input: SettingsOverlayInput,
    active_category: NotoraSettingsCategory,
    category_buttons: Vec<(NotoraSettingsCategory, Button)>,
    category_rects: Vec<Rect>,
    category_pointer_index: Option<usize>,
    retry_pointer_active: bool,
    form: FormView,
    form_rect: Rect,
    form_needs_layout: bool,
    focused_id: Option<WidgetId>,
    validation: Option<FieldValidation>,
    message_rect: Rect,
    retry_button: Button,
    settings_theme: SettingsTheme,
}

impl NotoraSettingsView {
    pub(super) fn new(input: SettingsOverlayInput) -> Self {
        let settings_theme = fallback_settings_theme();
        let active_category = NotoraSettingsCategory::default();
        let mut view = Self {
            rect: Rect::ZERO,
            dpi: 1.0,
            sidebar_width: 0.0,
            input,
            active_category,
            category_buttons: Vec::new(),
            category_rects: Vec::new(),
            category_pointer_index: None,
            retry_pointer_active: false,
            form: FormView::new(FormViewStyle::default()),
            form_rect: Rect::ZERO,
            form_needs_layout: true,
            focused_id: None,
            validation: None,
            message_rect: Rect::ZERO,
            retry_button: Button::new(RETRY_PERSISTENCE_ID, action_button_style(settings_theme)),
            settings_theme,
        };
        view.retry_button.set_text(Some("重试".to_owned()));
        view.category_buttons = view.build_category_buttons();
        view
    }

    pub(super) fn set_input(&mut self, input: SettingsOverlayInput) {
        if self.input == input {
            return;
        }
        self.input = input;
        self.form_needs_layout = true;
        self.validation = None;
        self.sync_category_selection();
    }

    pub(super) fn set_rect(&mut self, rect: Rect, context: &mut LayoutCtx<'_>) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        self.dpi = context.dpi;
        let next_theme = context.theme.settings_theme();
        if self.settings_theme != next_theme {
            self.settings_theme = next_theme;
            self.category_buttons = self.build_category_buttons();
            self.retry_button.set_style(action_button_style(next_theme));
            self.form_needs_layout = true;
        }
        self.layout_categories(context);
        self.layout_form_and_message(context);
    }

    pub(super) fn paint(&self, context: &mut PaintCtx<'_>) {
        context.list.fill(
            Rect::new(0.0, 0.0, self.sidebar_width, self.rect.h),
            self.settings_theme.sidebar_surface,
        );
        let separator_width = SIDEBAR_SEPARATOR_WIDTH_LOGICAL * context.dpi;
        context.list.fill(
            Rect::new(
                (self.sidebar_width - separator_width).max(0.0),
                0.0,
                separator_width,
                self.rect.h,
            ),
            self.settings_theme.separator,
        );
        for ((_, button), rect) in self.category_buttons.iter().zip(&self.category_rects) {
            paint_widget_at(button, *rect, context);
        }
        paint_widget_at(&self.form, self.form_rect, context);
        self.paint_message(context);
    }

    pub(super) fn route_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<SettingsOverlayAction> {
        if self.form.is_capturing()
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            return self.dispatch_form_event(event, context);
        }
        if self.retry_pointer_active
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            let action = self.dispatch_retry_event(event, context);
            if matches!(event, Event::MouseUp { .. }) {
                self.retry_pointer_active = false;
            }
            return action;
        }
        if let Some(index) = self.category_pointer_index
            && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
        {
            let action = self.dispatch_category_event(index, event, context);
            if matches!(event, Event::MouseUp { .. }) {
                self.category_pointer_index = None;
            }
            return action;
        }

        match event {
            Event::MouseDown { px, py, .. } => {
                if self.retry_is_visible() && self.retry_button_rect().contains(*px, *py) {
                    self.retry_pointer_active = true;
                    return self.dispatch_retry_event(event, context);
                }
                if let Some(index) = self.category_index_at(*px, *py) {
                    self.category_pointer_index = Some(index);
                    return self.dispatch_category_event(index, event, context);
                }
                self.dispatch_form_event(event, context)
            }
            Event::MouseMove { .. } => {
                self.dispatch_category_hover(event, context);
                if self.retry_is_visible() {
                    let _ = self.dispatch_retry_event(event, context);
                }
                self.dispatch_form_event(event, context)
                    .or(Some(SettingsOverlayAction::ViewChanged))
            }
            _ => self.dispatch_form_event(event, context),
        }
    }

    #[cfg(test)]
    pub(super) fn active_category_name(&self) -> &'static str {
        match self.active_category {
            NotoraSettingsCategory::Appearance => "appearance",
            NotoraSettingsCategory::Editor => "editor",
            NotoraSettingsCategory::Interface => "interface",
            NotoraSettingsCategory::Workspace => "workspace",
        }
    }

    fn build_category_buttons(&self) -> Vec<(NotoraSettingsCategory, Button)> {
        [
            (NotoraSettingsCategory::Appearance, "外观", APPEARANCE_CATEGORY_ID),
            (NotoraSettingsCategory::Editor, "编辑器", EDITOR_CATEGORY_ID),
            (NotoraSettingsCategory::Interface, "界面", INTERFACE_CATEGORY_ID),
            (NotoraSettingsCategory::Workspace, "工作区", WORKSPACE_CATEGORY_ID),
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

    fn layout_categories(&mut self, context: &mut LayoutCtx<'_>) {
        let compact = self.rect.w < COMPACT_LAYOUT_THRESHOLD_LOGICAL * context.dpi;
        let sidebar_width_logical =
            if compact { COMPACT_SIDEBAR_WIDTH_LOGICAL } else { SIDEBAR_WIDTH_LOGICAL };
        self.sidebar_width = (sidebar_width_logical * context.dpi).min(self.rect.w);
        self.category_rects.clear();
        let mut category_y = SIDEBAR_TOP_INSET_LOGICAL * context.dpi;
        for (_, button) in &mut self.category_buttons {
            button.set_style(category_button_style(self.settings_theme));
            let rect = Rect::new(
                CATEGORY_HORIZONTAL_INSET_LOGICAL * context.dpi,
                category_y,
                (self.sidebar_width - CATEGORY_HORIZONTAL_INSET_LOGICAL * 2.0 * context.dpi)
                    .max(0.0),
                CATEGORY_HEIGHT_LOGICAL * context.dpi,
            );
            button.set_rect(Rect::new(0.0, 0.0, rect.w, rect.h), context);
            self.category_rects.push(rect);
            category_y += rect.h + CATEGORY_GAP_LOGICAL * context.dpi;
        }
    }

    fn layout_form_and_message(&mut self, context: &mut LayoutCtx<'_>) {
        let compact = self.rect.w < COMPACT_LAYOUT_THRESHOLD_LOGICAL * context.dpi;
        let form_inset_logical =
            if compact { COMPACT_FORM_INSET_LOGICAL } else { FORM_INSET_LOGICAL };
        let form_inset = form_inset_logical * context.dpi;
        let form_gap = FORM_GAP_LOGICAL * context.dpi;
        let message_height =
            if self.message_text().is_some() { MESSAGE_HEIGHT_LOGICAL * context.dpi } else { 0.0 };
        let message_gap =
            if message_height > 0.0 { MESSAGE_GAP_LOGICAL * context.dpi } else { 0.0 };
        self.form_rect = Rect::new(
            self.sidebar_width + form_gap + form_inset,
            form_inset,
            (self.rect.w - self.sidebar_width - form_gap - form_inset * 2.0).max(0.0),
            (self.rect.h - form_inset * 2.0 - message_height - message_gap).max(0.0),
        );
        self.message_rect = Rect::new(
            self.form_rect.x,
            self.form_rect.bottom() + message_gap,
            self.form_rect.w,
            message_height,
        );
        if self.form_needs_layout {
            self.rebuild_form(context);
        }
        self.form.set_rect(Rect::new(0.0, 0.0, self.form_rect.w, self.form_rect.h), context);
        let retry_rect = self.retry_button_rect();
        self.retry_button.set_rect(Rect::new(0.0, 0.0, retry_rect.w, retry_rect.h), context);
    }

    fn rebuild_form(&mut self, context: &mut LayoutCtx<'_>) {
        let sections = match self.active_category {
            NotoraSettingsCategory::Appearance => self.appearance_sections(),
            NotoraSettingsCategory::Editor => self.editor_sections(),
            NotoraSettingsCategory::Interface => self.interface_sections(),
            NotoraSettingsCategory::Workspace => self.workspace_sections(),
        };
        self.form.set_sections(sections, context);
        self.form.set_keyboard_focus(self.focused_id);
        self.form_needs_layout = false;
    }

    fn appearance_sections(&self) -> Vec<FormSection> {
        vec![settings_section(
            "外观",
            "选择 Notora 的浅色、深色或系统外观。",
            vec![self.theme_mode_row()],
        )]
    }

    fn editor_sections(&self) -> Vec<FormSection> {
        let editor = &self.input.product_settings.editor;
        vec![settings_section(
            "编辑器",
            "调整 Notora 文档的文字显示与编辑行为。",
            vec![
                text_row(
                    FONT_FAMILY_ID,
                    "字体",
                    "输入编辑器字体名称。",
                    &editor.font_family,
                    "字体名称",
                ),
                text_row(
                    FONT_SIZE_ID,
                    "字号",
                    "允许范围：6–72。",
                    &format_float(editor.font_size),
                    "6–72",
                ),
                text_row(
                    LINE_HEIGHT_RATIO_ID,
                    "行高比例",
                    "允许范围：1–3。",
                    &format_float(editor.line_height_ratio),
                    "1–3",
                ),
                switch_row(
                    WORD_WRAP_ID,
                    "自动换行",
                    "在编辑区域宽度不足时折行。",
                    editor.word_wrap,
                ),
                switch_row(
                    MARKDOWN_FIRST_LINE_INDENT_ID,
                    "Markdown 首行缩进",
                    "普通段落首行缩进两个字符，仅影响排版",
                    editor.markdown_first_line_indent,
                ),
                switch_row(
                    LINE_NUMBERS_ID,
                    "显示行号",
                    "在编辑器左侧显示行号。",
                    editor.show_line_numbers,
                ),
                text_row(
                    TAB_WIDTH_ID,
                    "制表符宽度",
                    "允许范围：1–16 个空格。",
                    &editor.tab_width.to_string(),
                    "1–16",
                ),
            ],
        )]
    }

    fn interface_sections(&self) -> Vec<FormSection> {
        let interface = &self.input.product_settings.interface;
        vec![settings_section(
            "界面",
            "调整 Notora 外壳与运行时资源使用。",
            vec![
                switch_row(
                    STATUS_BAR_ID,
                    "显示状态栏",
                    "在编辑器底部显示文档状态。",
                    interface.show_status_bar,
                ),
                text_row(
                    RUNTIME_TAB_LIMIT_ID,
                    "运行时 Tab 上限",
                    "允许范围：1–128。",
                    &interface.runtime_tab_limit.to_string(),
                    "1–128",
                ),
            ],
        )]
    }

    fn workspace_sections(&self) -> Vec<FormSection> {
        let workspace = &self.input.product_settings.workspace;
        vec![settings_section(
            "工作区",
            "调整自动保存节奏和 catalog 备份策略。",
            vec![
                text_row(
                    AUTO_SAVE_DELAY_ID,
                    "自动保存延迟",
                    "允许范围：100–60000 毫秒。",
                    &workspace.auto_save_delay_millis.to_string(),
                    "100–60000",
                ),
                text_row(
                    CATALOG_BACKUP_RETENTION_ID,
                    "目录索引备份数",
                    "允许范围：1–100。",
                    &workspace.catalog_backup_retention.to_string(),
                    "1–100",
                ),
            ],
        )]
    }

    fn theme_mode_row(&self) -> FormRow {
        let mode = self.input.product_settings.appearance.theme_mode;
        let buttons = [
            (THEME_SYSTEM_ID, "跟随系统", mode == ThemeMode::System),
            (THEME_DARK_ID, "深色", mode == ThemeMode::Dark),
            (THEME_LIGHT_ID, "浅色", mode == ThemeMode::Light),
        ]
        .into_iter()
        .map(|(id, title, selected)| {
            let mut button = Button::new(id, segmented_button_style(self.settings_theme));
            button.set_text(Some(title.to_owned()));
            button.set_selected(selected);
            InlineChild::fixed(Box::new(button), SEGMENT_WIDTH_LOGICAL)
                .with_cross_size(CONTROL_HEIGHT_LOGICAL)
        })
        .collect();
        let group = InlineGroup::new(buttons)
            .with_gap(0.0)
            .with_main_alignment(MainAlignment::End)
            .with_alignment(CrossAlignment::Center);
        FormRow::new(row_label("主题"), None, Box::new(group), settings_row_style())
    }

    fn dispatch_form_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<SettingsOverlayAction> {
        let local_event = Dock::to_local(event, self.form_rect.x, self.form_rect.y);
        let action = self.form.on_event(local_event.as_ref(), context)?;
        self.map_widget_action(action).or(Some(SettingsOverlayAction::ViewChanged))
    }

    fn dispatch_category_event(
        &mut self,
        index: usize,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<SettingsOverlayAction> {
        let rect = *self.category_rects.get(index)?;
        let local_event = Dock::to_local(event, rect.x, rect.y);
        let action = self.category_buttons[index].1.on_event(local_event.as_ref(), context)?;
        if matches!(action, WidgetAction::Control(ControlAction::Activated { .. })) {
            self.active_category = self.category_buttons[index].0;
            self.focused_id = None;
            self.validation = None;
            self.form_needs_layout = true;
            self.sync_category_selection();
        }
        Some(SettingsOverlayAction::ViewChanged)
    }

    fn dispatch_category_hover(&mut self, event: &Event, context: &mut EventCtx<'_>) {
        for index in 0..self.category_buttons.len() {
            let _ = self.dispatch_category_event(index, event, context);
        }
    }

    fn dispatch_retry_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<SettingsOverlayAction> {
        let retry_rect = self.retry_button_rect();
        let local_event = Dock::to_local(event, retry_rect.x, retry_rect.y);
        match self.retry_button.on_event(local_event.as_ref(), context) {
            Some(WidgetAction::Control(ControlAction::Activated { .. })) => {
                Some(SettingsOverlayAction::RetryPersistence)
            }
            Some(_) => Some(SettingsOverlayAction::ViewChanged),
            None => None,
        }
    }

    fn map_widget_action(&mut self, action: WidgetAction) -> Option<SettingsOverlayAction> {
        let WidgetAction::Control(control_action) = action else {
            return None;
        };
        match control_action {
            ControlAction::Activated { id } => self.map_activated_control(id),
            ControlAction::Toggled { id, checked } => self.map_toggle(id, checked),
            ControlAction::TextCommitted { id, value: TextPayload::Plain(value) } => {
                self.map_text_commit(id, &value)
            }
            ControlAction::TextCommitted { .. } => {
                self.set_validation("请输入有效值");
                None
            }
            ControlAction::FocusRequested { id } => {
                self.focused_id = Some(id);
                self.form.set_keyboard_focus(Some(id));
                Some(SettingsOverlayAction::ViewChanged)
            }
            ControlAction::TextEdited { .. } => Some(SettingsOverlayAction::ViewChanged),
        }
    }

    fn map_activated_control(&mut self, id: WidgetId) -> Option<SettingsOverlayAction> {
        let theme_mode = match id {
            THEME_SYSTEM_ID => ThemeMode::System,
            THEME_DARK_ID => ThemeMode::Dark,
            THEME_LIGHT_ID => ThemeMode::Light,
            _ => return None,
        };
        self.validation = None;
        Some(SettingsOverlayAction::Update(ProductSettingsUpdate::ThemeMode(theme_mode)))
    }

    fn map_toggle(&mut self, id: WidgetId, checked: bool) -> Option<SettingsOverlayAction> {
        let update = match id {
            WORD_WRAP_ID => ProductSettingsUpdate::WordWrap(checked),
            MARKDOWN_FIRST_LINE_INDENT_ID => {
                ProductSettingsUpdate::MarkdownFirstLineIndent(checked)
            }
            LINE_NUMBERS_ID => ProductSettingsUpdate::ShowLineNumbers(checked),
            STATUS_BAR_ID => ProductSettingsUpdate::ShowStatusBar(checked),
            _ => return None,
        };
        self.validation = None;
        Some(SettingsOverlayAction::Update(update))
    }

    fn map_text_commit(&mut self, id: WidgetId, value: &str) -> Option<SettingsOverlayAction> {
        let update = match id {
            FONT_FAMILY_ID => ProductSettingsUpdate::FontFamily(self.parse_font_family(value)?),
            FONT_SIZE_ID => ProductSettingsUpdate::FontSize(self.parse_f32_field(
                value,
                MINIMUM_FONT_SIZE,
                MAXIMUM_FONT_SIZE,
            )?),
            LINE_HEIGHT_RATIO_ID => ProductSettingsUpdate::LineHeightRatio(self.parse_f32_field(
                value,
                MINIMUM_LINE_HEIGHT_RATIO,
                MAXIMUM_LINE_HEIGHT_RATIO,
            )?),
            TAB_WIDTH_ID => ProductSettingsUpdate::TabWidth(self.parse_usize_field(
                value,
                MINIMUM_TAB_WIDTH,
                MAXIMUM_TAB_WIDTH,
            )?),
            RUNTIME_TAB_LIMIT_ID => {
                ProductSettingsUpdate::RuntimeTabLimit(self.parse_usize_field(
                    value,
                    MINIMUM_RUNTIME_TAB_LIMIT,
                    MAXIMUM_RUNTIME_TAB_LIMIT,
                )?)
            }
            AUTO_SAVE_DELAY_ID => {
                ProductSettingsUpdate::AutoSaveDelayMillis(self.parse_u64_field(
                    value,
                    MINIMUM_AUTO_SAVE_DELAY_MILLIS,
                    MAXIMUM_AUTO_SAVE_DELAY_MILLIS,
                )?)
            }
            CATALOG_BACKUP_RETENTION_ID => {
                ProductSettingsUpdate::CatalogBackupRetention(self.parse_usize_field(
                    value,
                    MINIMUM_CATALOG_BACKUP_RETENTION,
                    MAXIMUM_CATALOG_BACKUP_RETENTION,
                )?)
            }
            _ => return None,
        };
        self.validation = None;
        Some(SettingsOverlayAction::Update(update))
    }

    fn parse_font_family(&mut self, value: &str) -> Option<String> {
        let font_family = parse_non_empty(value);
        if font_family.is_none() {
            self.set_validation("字体名称不能为空");
        }
        font_family
    }

    fn parse_f32_field(&mut self, value: &str, minimum: f32, maximum: f32) -> Option<f32> {
        let parsed = value.trim().parse::<f32>().ok();
        let valid =
            parsed.filter(|number| number.is_finite() && (minimum..=maximum).contains(number));
        if valid.is_none() {
            self.set_validation("数值超出允许范围");
        }
        valid
    }

    fn parse_usize_field(&mut self, value: &str, minimum: usize, maximum: usize) -> Option<usize> {
        let parsed = value.trim().parse::<usize>().ok();
        let valid = parsed.filter(|number| (minimum..=maximum).contains(number));
        if valid.is_none() {
            self.set_validation("数值超出允许范围");
        }
        valid
    }

    fn parse_u64_field(&mut self, value: &str, minimum: u64, maximum: u64) -> Option<u64> {
        let parsed = value.trim().parse::<u64>().ok();
        let valid = parsed.filter(|number| (minimum..=maximum).contains(number));
        if valid.is_none() {
            self.set_validation("数值超出允许范围");
        }
        valid
    }

    fn set_validation(&mut self, message: &'static str) {
        self.validation = Some(FieldValidation { message });
    }

    fn category_index_at(&self, px: f32, py: f32) -> Option<usize> {
        self.category_rects.iter().position(|rect| rect.contains(px, py))
    }

    fn retry_is_visible(&self) -> bool {
        matches!(self.input.persistence, NotoraSettingsPersistenceView::SaveFailed { .. })
    }

    fn retry_button_rect(&self) -> Rect {
        let width = (RETRY_BUTTON_WIDTH_LOGICAL * self.dpi).min(self.message_rect.w);
        Rect::new(
            self.message_rect.right() - width,
            self.message_rect.y + (self.message_rect.h - CONTROL_HEIGHT_LOGICAL * self.dpi) * 0.5,
            width,
            (CONTROL_HEIGHT_LOGICAL * self.dpi).min(self.message_rect.h),
        )
    }

    fn message_text(&self) -> Option<&str> {
        if let Some(validation) = self.validation {
            return Some(validation.message);
        }
        match &self.input.persistence {
            NotoraSettingsPersistenceView::Saved => None,
            NotoraSettingsPersistenceView::SaveFailed { message } => Some(message),
        }
    }

    fn paint_message(&self, context: &mut PaintCtx<'_>) {
        let Some(message) = self.message_text() else {
            return;
        };
        context.text(
            self.message_rect.x,
            self.message_rect.y + self.message_rect.h * 0.5 + 4.0 * context.dpi,
            DESCRIPTION_FONT_SIZE_LOGICAL * context.dpi,
            self.settings_theme.text_secondary,
            message,
        );
        if self.retry_is_visible() {
            paint_widget_at(&self.retry_button, self.retry_button_rect(), context);
        }
    }
}

fn settings_section(title: &str, description: &str, rows: Vec<FormRow>) -> FormSection {
    FormSection::new(
        section_title_label(title),
        Some(description_label(description)),
        rows,
        settings_section_style(),
    )
}

fn text_row(
    id: WidgetId,
    title: &str,
    description: &str,
    value: &str,
    placeholder: &str,
) -> FormRow {
    let mut text_box = settings_text_box(id);
    text_box.set_text(value);
    text_box.set_placeholder(placeholder);
    FormRow::new(
        row_label(title),
        Some(description_label(description)),
        Box::new(text_box),
        settings_row_style(),
    )
}

fn switch_row(id: WidgetId, title: &str, description: &str, checked: bool) -> FormRow {
    FormRow::new(
        row_label(title),
        Some(description_label(description)),
        Box::new(Switch::new(id, checked)),
        settings_row_style(),
    )
}

fn settings_text_box(id: WidgetId) -> TextBox {
    let mut text_box = TextBox::with_id(id);
    text_box.set_fixed_size_logical(TEXT_BOX_WIDTH_LOGICAL, CONTROL_HEIGHT_LOGICAL);
    text_box.set_blink(true);
    text_box
}

fn settings_row_style() -> FormRowStyle {
    FormRowStyle {
        min_height_logical: ROW_HEIGHT_LOGICAL,
        label_width_logical: ROW_LABEL_WIDTH_LOGICAL,
        column_gap_logical: ROW_COLUMN_GAP_LOGICAL,
        responsive_threshold_logical: 0.0,
        padding_logical: [
            ROW_VERTICAL_INSET_LOGICAL,
            ROW_HORIZONTAL_INSET_LOGICAL,
            ROW_VERTICAL_INSET_LOGICAL,
            ROW_HORIZONTAL_INSET_LOGICAL,
        ],
        ..FormRowStyle::default()
    }
}

fn settings_section_style() -> FormSectionStyle {
    FormSectionStyle {
        title_gap_logical: SECTION_TITLE_GAP_LOGICAL,
        description_gap_logical: SECTION_DESCRIPTION_GAP_LOGICAL,
        row_height_logical: ROW_HEIGHT_LOGICAL,
        corner_radius_logical: SECTION_CORNER_RADIUS_LOGICAL,
        ..FormSectionStyle::default()
    }
}

fn section_title_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SECTION_TITLE_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::MEDIUM,
            ..LabelStyle::default()
        },
    )
}

fn row_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: ROW_LABEL_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::MEDIUM,
            ..LabelStyle::default()
        },
    )
}

fn description_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: DESCRIPTION_FONT_SIZE_LOGICAL,
            foreground: LabelForeground::ThemeMuted,
            ..LabelStyle::default()
        },
    )
}

fn category_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        font_size_logical: BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: BUTTON_PADDING_LOGICAL,
        foreground: settings.text_primary,
        selected_foreground: settings.accent,
        background: TRANSPARENT,
        border: TRANSPARENT,
        hover_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            CATEGORY_HOVER_ACCENT_BLEND,
        ),
        pressed_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            CATEGORY_PRESSED_ACCENT_BLEND,
        ),
        selected_background: blend_color(
            settings.sidebar_surface,
            settings.accent,
            CATEGORY_SELECTED_ACCENT_BLEND,
        ),
        disabled_foreground: with_alpha(settings.text_primary, DISABLED_FOREGROUND_ALPHA),
        disabled_background: TRANSPARENT,
        corner_radius_logical: BUTTON_RADIUS_LOGICAL,
    }
}

fn segmented_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        font_size_logical: BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: BUTTON_PADDING_LOGICAL,
        foreground: settings.text_primary,
        selected_foreground: settings.text_inverse,
        background: TRANSPARENT,
        border: TRANSPARENT,
        hover_background: blend_color(
            settings.modal_surface,
            settings.accent,
            SEGMENT_HOVER_ACCENT_BLEND,
        ),
        pressed_background: blend_color(
            settings.modal_surface,
            settings.accent,
            SEGMENT_PRESSED_ACCENT_BLEND,
        ),
        selected_background: settings.accent,
        disabled_foreground: with_alpha(settings.text_primary, DISABLED_FOREGROUND_ALPHA),
        disabled_background: TRANSPARENT,
        corner_radius_logical: BUTTON_RADIUS_LOGICAL,
    }
}

fn action_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        foreground: settings.text_primary,
        selected_foreground: settings.text_primary,
        background: settings.control_surface,
        border: settings.control_border,
        hover_background: settings.control_surface,
        pressed_background: settings.control_surface,
        selected_background: settings.control_surface,
        disabled_foreground: with_alpha(settings.text_primary, DISABLED_FOREGROUND_ALPHA),
        disabled_background: settings.control_surface,
        font_size_logical: BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: BUTTON_PADDING_LOGICAL,
        corner_radius_logical: BUTTON_RADIUS_LOGICAL,
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

fn paint_widget_at(widget: &impl Widget, rect: Rect, context: &mut PaintCtx<'_>) {
    let saved_offset = context.list.offset;
    context.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
    widget.paint(context);
    context.list.offset = saved_offset;
}

fn parse_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn format_float(value: f32) -> String {
    format!("{value:.3}").trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn fallback_settings_theme() -> SettingsTheme {
    ui::theme::test_theme().settings_theme()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::core::measure::NoopMeasure;
    use ui::core::paint::{DrawCmd, DrawList};
    use ui::core::widget::MouseButton;

    struct TestClipboard(String);

    impl ui::core::Clipboard for TestClipboard {
        fn read_text(&mut self) -> Option<String> {
            Some(self.0.clone())
        }

        fn write_text(&mut self, text: &str) -> bool {
            self.0 = text.to_owned();
            true
        }
    }

    fn laid_out_view() -> NotoraSettingsView {
        laid_out_view_with_input(SettingsOverlayInput::default())
    }

    fn laid_out_view_with_input(input: SettingsOverlayInput) -> NotoraSettingsView {
        let theme = ui::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut view = NotoraSettingsView::new(input);
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 560.0), &mut context);
        view
    }

    #[test]
    fn settings_text_fields_receive_clipboard_shortcuts_after_form_rebuild() {
        let theme = ui::theme::test_theme();
        let mut measure = NoopMeasure;
        let mut layout_context =
            LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let mut view = NotoraSettingsView::new(SettingsOverlayInput::default());
        view.active_category = NotoraSettingsCategory::Editor;
        view.form_needs_layout = true;
        view.set_rect(Rect::new(0.0, 0.0, 720.0, 560.0), &mut layout_context);
        view.focused_id = Some(FONT_FAMILY_ID);
        view.form.set_keyboard_focus(Some(FONT_FAMILY_ID));
        let mut clipboard = TestClipboard("JetBrains Mono".to_owned());
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        let command = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };

        let _ =
            view.route_event(&Event::KeyDown(ui::KeyCode::Char('a'), command), &mut event_context);
        let _ =
            view.route_event(&Event::KeyDown(ui::KeyCode::Char('v'), command), &mut event_context);
        let commit = view.route_event(
            &Event::KeyDown(ui::KeyCode::Enter, ui::core::Modifiers::NONE),
            &mut event_context,
        );

        assert_eq!(
            commit,
            Some(SettingsOverlayAction::Update(ProductSettingsUpdate::FontFamily(
                "JetBrains Mono".to_owned(),
            )))
        );
    }

    #[test]
    fn notora_owns_four_product_categories() {
        let view = laid_out_view();
        assert_eq!(view.category_buttons.len(), 4);
        assert_eq!(view.active_category_name(), "appearance");
    }

    #[test]
    fn notora_paints_its_own_categories_without_textora_view_mode_controls() {
        let view = laid_out_view();
        let theme = ui::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("settings paint test shaper should exist");
        view.paint(&mut PaintCtx {
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        });
        let visible_text: Vec<_> = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect();

        assert!(visible_text.contains(&"外观"));
        assert!(visible_text.contains(&"工作区"));
        assert!(!visible_text.contains(&"视图模式"));
        assert!(!visible_text.contains(&"侧边栏"));
    }

    #[test]
    fn notora_settings_view_does_not_import_the_textora_settings_page() {
        let production_source = include_str!("notora_settings_view.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source should precede tests");

        assert!(!production_source.contains("ui::settings_view"));
        assert!(!production_source.contains("SettingsViewInput"));
    }

    #[test]
    fn category_switch_is_local_and_requests_a_redraw() {
        let theme = ui::theme::test_theme();
        let mut view = laid_out_view();
        let category_rect = view.category_rects[3];
        let click_x = category_rect.x + category_rect.w * 0.5;
        let click_y = category_rect.y + category_rect.h * 0.5;
        let mut context = EventCtx::new(&theme, 1.0);

        let _ = view.route_event(
            &Event::MouseDown { px: click_x, py: click_y, button: MouseButton::Left },
            &mut context,
        );
        let action = view.route_event(
            &Event::MouseUp { px: click_x, py: click_y, button: MouseButton::Left },
            &mut context,
        );

        assert_eq!(view.active_category_name(), "workspace");
        assert_eq!(action, Some(SettingsOverlayAction::ViewChanged));
    }

    #[test]
    fn notora_numeric_fields_use_product_ranges() {
        let mut view = laid_out_view();
        assert_eq!(
            view.map_text_commit(RUNTIME_TAB_LIMIT_ID, "24"),
            Some(SettingsOverlayAction::Update(ProductSettingsUpdate::RuntimeTabLimit(24)))
        );
        assert_eq!(view.map_text_commit(RUNTIME_TAB_LIMIT_ID, "0"), None);
        assert!(view.validation.is_some());
    }

    #[test]
    fn text_editing_and_persistence_retry_emit_notora_view_actions() {
        let input = SettingsOverlayInput {
            persistence: NotoraSettingsPersistenceView::SaveFailed {
                message: "permission denied".to_owned(),
            },
            ..SettingsOverlayInput::default()
        };
        let mut view = laid_out_view_with_input(input);
        assert_eq!(
            view.map_widget_action(WidgetAction::Control(ControlAction::TextEdited {
                id: FONT_FAMILY_ID,
                value: TextPayload::Plain("Mono".to_owned()),
            })),
            Some(SettingsOverlayAction::ViewChanged),
        );

        let theme = ui::theme::test_theme();
        let retry_rect = view.retry_button_rect();
        let click_x = retry_rect.x + retry_rect.w * 0.5;
        let click_y = retry_rect.y + retry_rect.h * 0.5;
        let mut context = EventCtx::new(&theme, 1.0);
        let _ = view.route_event(
            &Event::MouseDown { px: click_x, py: click_y, button: MouseButton::Left },
            &mut context,
        );
        assert_eq!(
            view.route_event(
                &Event::MouseUp { px: click_x, py: click_y, button: MouseButton::Left },
                &mut context,
            ),
            Some(SettingsOverlayAction::RetryPersistence),
        );
    }
}
