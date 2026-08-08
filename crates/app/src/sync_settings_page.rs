use std::any::Any;
use std::borrow::Cow;

use ui::button::{Button, ButtonStyle};
use ui::core::widget::{ControlAction, SensitiveText, TextPayload, WidgetId};
use ui::core::{
    AccessibilityActionRequest, AccessibilityContext, AccessibilityNode, Event, EventCtx,
    LayoutCtx, PaintCtx, Rect, Widget, WidgetAction,
};
use ui::form::{FormRow, FormRowStyle, FormSection, FormSectionStyle, FormView};
use ui::inline_group::{CrossAlignment, InlineChild, InlineGroup};
use ui::label::{Label, LabelForeground, LabelStyle};
use ui::text_box::TextBox;
use ui::theme::SettingsTheme;

use crate::sync_settings_types::{
    LibrarySyncState, LibraryView, SyncConnectionView, SyncNoticeSeverity, SyncSettingsAction,
    SyncSettingsInput,
};

const SYNC_ROW_HEIGHT_LOGICAL: f32 = 64.0;
const SYNC_ROW_LABEL_WIDTH_LOGICAL: f32 = 176.0;
const SYNC_ROW_HORIZONTAL_INSET_LOGICAL: f32 = 16.0;
const SYNC_ROW_VERTICAL_INSET_LOGICAL: f32 = 10.0;
pub(super) const SYNC_CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
const SYNC_TEXT_BOX_WIDTH_LOGICAL: f32 = 320.0;
const SYNC_CONNECTION_STACK_THRESHOLD_LOGICAL: f32 = 400.0;
const SYNC_CONNECTION_FIELD_STACK_THRESHOLD_LOGICAL: f32 = 240.0;
const SYNC_CONNECTION_ACTION_STACK_THRESHOLD_LOGICAL: f32 = 204.0;
const SYNC_COMPACT_CONNECTION_ROW_HEIGHT_LOGICAL: f32 = 96.0;
const SYNC_STACKED_ACTION_GAP_LOGICAL: f32 = 8.0;
const SYNC_BUTTON_WIDTH_LOGICAL: f32 = 94.0;
const SYNC_DYNAMIC_BUTTON_WIDTH_LOGICAL: f32 = 80.0;
const SYNC_WIDE_BUTTON_WIDTH_LOGICAL: f32 = 160.0;
const SYNC_BUTTON_FONT_SIZE_LOGICAL: f32 = 14.0;
const SYNC_BUTTON_PADDING_LOGICAL: f32 = 12.0;
const SYNC_BUTTON_RADIUS_LOGICAL: f32 = 8.0;
const SYNC_SECTION_TITLE_FONT_SIZE_LOGICAL: f32 = 17.0;
const SYNC_ROW_LABEL_FONT_SIZE_LOGICAL: f32 = 14.0;
const SYNC_DESCRIPTION_FONT_SIZE_LOGICAL: f32 = 12.0;
const SYNC_SECTION_GAP_LOGICAL: f32 = 24.0;
const SYNC_SECTION_TITLE_GAP_LOGICAL: f32 = 6.0;
const SYNC_SECTION_DESCRIPTION_GAP_LOGICAL: f32 = 14.0;
const SYNC_SECTION_CORNER_RADIUS_LOGICAL: f32 = 10.0;
const SYNC_TRANSPARENT: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
const SYNC_DISABLED_FOREGROUND_ALPHA: f32 = 0.45;

pub(super) const ENDPOINT_ID: WidgetId = WidgetId(0x7379_6e63_656e_6470);
pub(super) const API_KEY_ID: WidgetId = WidgetId(0x7379_6e63_6170_696b);
const REMOTE_DEVICE_ID: WidgetId = WidgetId(0x7379_6e63_7264_6576);
const REMOTE_NAME_ID: WidgetId = WidgetId(0x7379_6e63_726e_616d);
const REMOTE_ADDRESSES_ID: WidgetId = WidgetId(0x7379_6e63_7261_6464);
const TEST_CONNECTION_ID: WidgetId = WidgetId(0x7379_6e63_7465_7374);
const CONFIGURE_CONNECTION_ID: WidgetId = WidgetId(0x7379_6e63_636f_6e66);
const PUBLISH_LIBRARY_ID: WidgetId = WidgetId(0x7379_6e63_7075_626c);
const PENDING_ACCEPT_BASE_ID: WidgetId = WidgetId(0x7379_6e63_7065_6e00);
const SCAN_BASE_ID: WidgetId = WidgetId(0x7379_6e63_7363_6e00);
const PAUSE_BASE_ID: WidgetId = WidgetId(0x7379_6e63_7061_7500);
const REPAIR_BASE_ID: WidgetId = WidgetId(0x7379_6e63_7265_7000);
const REMOVE_MAPPING_BASE_ID: WidgetId = WidgetId(0x7379_6e63_726d_7600);
const UNREGISTER_BASE_ID: WidgetId = WidgetId(0x7379_6e63_756e_7200);

#[derive(Default)]
struct SyncSettingsDraft {
    endpoint: String,
    api_key: Option<SensitiveText>,
    remote_device_id: String,
    remote_name: String,
    remote_addresses: String,
}

struct StackedConnectionActions {
    rect: Rect,
    buttons: [Button; 2],
    button_rects: [Rect; 2],
    pointer_index: Option<usize>,
    hover_index: Option<usize>,
    focused_id: Option<WidgetId>,
}

impl StackedConnectionActions {
    fn new(test_button: Button, configure_button: Button) -> Self {
        Self {
            rect: Rect::ZERO,
            buttons: [test_button, configure_button],
            button_rects: [Rect::ZERO; 2],
            pointer_index: None,
            hover_index: None,
            focused_id: None,
        }
    }

    fn button_index_at(&self, px: f32, py: f32) -> Option<usize> {
        self.button_rects.iter().position(|rect| rect.contains(px, py))
    }

    fn focused_button_index(&self) -> Option<usize> {
        let focused_id = self.focused_id?;
        self.buttons.iter().position(|button| button.id() == Some(focused_id))
    }

    fn dispatch_to_button(
        &mut self,
        index: usize,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let button_rect = self.button_rects[index];
        let local_event: Cow<'_, Event> =
            ui::core::dock::Dock::to_local(event, button_rect.x, button_rect.y);
        self.buttons[index].on_event(local_event.as_ref(), ctx)
    }

    fn dispatch_outside_move_to_previous_hover(
        &mut self,
        next_hover_index: Option<usize>,
        event: &Event,
        ctx: &mut EventCtx,
    ) -> Option<WidgetAction> {
        let previous_hover_index = self.hover_index?;
        if Some(previous_hover_index) == next_hover_index {
            return None;
        }

        self.dispatch_to_button(previous_hover_index, event, ctx)
    }
}

impl Widget for StackedConnectionActions {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        let gap = (SYNC_STACKED_ACTION_GAP_LOGICAL * ctx.dpi).min(self.rect.h);
        let available_height = (self.rect.h - gap).max(0.0);
        let button_height = (SYNC_CONTROL_HEIGHT_LOGICAL * ctx.dpi).min(available_height * 0.5);
        let actions_height = button_height * 2.0 + gap;
        let first_y = ((self.rect.h - actions_height) * 0.5).max(0.0);
        self.button_rects = [
            Rect::new(0.0, first_y, self.rect.w, button_height),
            Rect::new(0.0, first_y + button_height + gap, self.rect.w, button_height),
        ];

        for (button, button_rect) in self.buttons.iter_mut().zip(self.button_rects) {
            button.set_rect(Rect::new(0.0, 0.0, button_rect.w, button_rect.h), ctx);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let saved_offset = ctx.list.offset;
        for (button, button_rect) in self.buttons.iter().zip(self.button_rects) {
            ctx.list.offset = (saved_offset.0 + button_rect.x, saved_offset.1 + button_rect.y);
            button.paint(ctx);
        }
        ctx.list.offset = saved_offset;
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        for button in &self.buttons {
            button.collect_focusable_ids(output);
        }
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.focused_id = focused_id;
        for button in &mut self.buttons {
            button.set_keyboard_focus(focused_id);
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        match event {
            Event::PointerLeave => {
                let hover_index = self.hover_index.take();
                let action =
                    hover_index.and_then(|index| self.dispatch_to_button(index, event, ctx));
                action.or_else(|| hover_index.map(|_| WidgetAction::Consumed))
            }
            Event::InteractionCancel => {
                let container_changed =
                    self.pointer_index.take().is_some() | self.hover_index.take().is_some();
                let mut first_action = None;
                for index in 0..self.buttons.len() {
                    if let Some(action) = self.dispatch_to_button(index, event, ctx)
                        && first_action.is_none()
                    {
                        first_action = Some(action);
                    }
                }
                first_action.or_else(|| container_changed.then_some(WidgetAction::Consumed))
            }
            Event::MouseDown { px, py, .. } => {
                let index = self.button_index_at(*px, *py)?;
                self.pointer_index = Some(index);
                self.dispatch_to_button(index, event, ctx)
            }
            Event::MouseMove { px, py } => {
                if let Some(index) = self.pointer_index {
                    return self.dispatch_to_button(index, event, ctx);
                }

                let next_hover_index = self.button_index_at(*px, *py);
                let previous_hover_action =
                    self.dispatch_outside_move_to_previous_hover(next_hover_index, event, ctx);
                self.hover_index = next_hover_index;

                if let Some(index) = next_hover_index {
                    return self.dispatch_to_button(index, event, ctx).or(previous_hover_action);
                }

                previous_hover_action
            }
            Event::MouseUp { .. } => {
                let index = self.pointer_index.take()?;
                self.dispatch_to_button(index, event, ctx)
            }
            Event::KeyDown(..)
            | Event::ImePreedit { .. }
            | Event::ImeCommit(..)
            | Event::ImeEnable
            | Event::ImeDisable => {
                let index = self.focused_button_index()?;
                self.dispatch_to_button(index, event, ctx)
            }
            Event::Wheel { .. } => None,
        }
    }

    fn is_capturing(&self) -> bool {
        self.pointer_index.is_some()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct SyncSettingsPage {
    rect: Rect,
    input: SyncSettingsInput,
    draft: SyncSettingsDraft,
    pending_action: Option<SyncSettingsAction>,
    form: FormView,
    form_needs_rebuild: bool,
    form_has_sections: bool,
    compact_connection_layout: bool,
    stack_connection_actions: bool,
    endpoint_dirty: bool,
    settings_theme: SettingsTheme,
}

impl SyncSettingsPage {
    pub fn new(input: SyncSettingsInput) -> Self {
        let draft =
            SyncSettingsDraft { endpoint: input.endpoint.clone(), ..SyncSettingsDraft::default() };
        Self {
            rect: Rect::ZERO,
            input,
            draft,
            pending_action: None,
            form: FormView::new(ui::form::FormViewStyle {
                section_gap_logical: SYNC_SECTION_GAP_LOGICAL,
            }),
            form_needs_rebuild: true,
            form_has_sections: false,
            compact_connection_layout: false,
            stack_connection_actions: false,
            endpoint_dirty: false,
            settings_theme: fallback_settings_theme(),
        }
    }

    pub fn set_input(&mut self, mut input: SyncSettingsInput) {
        let mut retained_notices = self.input.notices.clone();
        for notice in std::mem::take(&mut input.notices) {
            if !retained_notices.contains(&notice) {
                retained_notices.push(notice);
            }
        }
        input.notices = retained_notices;
        let input_changed = self.input != input;
        let previous_draft_endpoint = self.draft.endpoint.clone();
        if !self.endpoint_dirty || self.draft.endpoint == input.endpoint {
            self.draft.endpoint.clone_from(&input.endpoint);
            self.endpoint_dirty = false;
        }
        let endpoint_draft_changed = self.draft.endpoint != previous_draft_endpoint;
        if !input_changed && !endpoint_draft_changed {
            return;
        }
        self.input = input;
        self.form_needs_rebuild = true;
    }

    pub fn input(&self) -> &SyncSettingsInput {
        &self.input
    }

    pub fn take_pending_action(&mut self) -> Option<SyncSettingsAction> {
        self.pending_action.take()
    }

    pub(super) fn reset_scroll_and_focus_endpoint(&mut self) {
        self.form.reset_scroll();
        self.form.set_keyboard_focus(Some(ENDPOINT_ID));
    }

    pub(super) fn handle_control_action(&mut self, action: ControlAction) -> Option<WidgetAction> {
        match action {
            ControlAction::FocusRequested { id } => {
                self.form.set_keyboard_focus(Some(id));
                Some(WidgetAction::Consumed)
            }
            ControlAction::TextEdited { id, value }
            | ControlAction::TextCommitted { id, value } => self.handle_text_action(id, value),
            ControlAction::Activated { id } => self.handle_activation(id),
            ControlAction::Toggled { .. } => Some(WidgetAction::Consumed),
        }
    }

    fn handle_text_action(&mut self, id: WidgetId, value: TextPayload) -> Option<WidgetAction> {
        match (id, value) {
            (API_KEY_ID, TextPayload::Sensitive(value)) => {
                self.draft.api_key = Some(value);
                Some(WidgetAction::Consumed)
            }
            (ENDPOINT_ID, TextPayload::Plain(value)) => {
                self.draft.endpoint = value;
                self.endpoint_dirty = self.draft.endpoint != self.input.endpoint;
                Some(WidgetAction::Consumed)
            }
            (REMOTE_DEVICE_ID, TextPayload::Plain(value)) => {
                self.draft.remote_device_id = value;
                Some(WidgetAction::Consumed)
            }
            (REMOTE_NAME_ID, TextPayload::Plain(value)) => {
                self.draft.remote_name = value;
                Some(WidgetAction::Consumed)
            }
            (REMOTE_ADDRESSES_ID, TextPayload::Plain(value)) => {
                self.draft.remote_addresses = value;
                Some(WidgetAction::Consumed)
            }
            _ => Some(WidgetAction::Consumed),
        }
    }

    fn handle_activation(&mut self, id: WidgetId) -> Option<WidgetAction> {
        let action = match id {
            CONFIGURE_CONNECTION_ID => {
                self.form_needs_rebuild = true;
                SyncSettingsAction::ConfigureConnection {
                    endpoint: self.draft.endpoint.clone(),
                    api_key: self.take_api_key_draft(),
                }
            }
            TEST_CONNECTION_ID => {
                self.form_needs_rebuild = true;
                SyncSettingsAction::TestConnection {
                    endpoint: self.draft.endpoint.clone(),
                    api_key: self.take_api_key_draft(),
                }
            }
            PUBLISH_LIBRARY_ID => SyncSettingsAction::PublishLibrary {
                remote_device_id: self.draft.remote_device_id.clone(),
                remote_name: self.draft.remote_name.clone(),
                remote_addresses: parse_remote_addresses(&self.draft.remote_addresses),
            },
            id if self.pending_index_from_id(id).is_some() => {
                let pending_index =
                    self.pending_index_from_id(id).expect("pending ID was checked before mapping");
                SyncSettingsAction::AcceptRemoteLibrary { pending_index }
            }
            id if self.scan_index_from_id(id).is_some() => {
                let library_index =
                    self.scan_index_from_id(id).expect("scan ID was checked before mapping");
                SyncSettingsAction::ScanLibrary { library_index }
            }
            id if self.pause_index_from_id(id).is_some() => {
                let library_index =
                    self.pause_index_from_id(id).expect("pause ID was checked before mapping");
                let paused =
                    !matches!(self.input.libraries[library_index].state, LibrarySyncState::Paused);
                SyncSettingsAction::SetLibraryPaused { library_index, paused }
            }
            id if self.repair_index_from_id(id).is_some() => {
                let library_index =
                    self.repair_index_from_id(id).expect("repair ID was checked before mapping");
                SyncSettingsAction::RepairLibrary { library_index }
            }
            id if self.remove_mapping_index_from_id(id).is_some() => {
                let library_index = self
                    .remove_mapping_index_from_id(id)
                    .expect("remove mapping ID was checked before mapping");
                SyncSettingsAction::RemoveLibraryMapping { library_index }
            }
            id if self.unregister_index_from_id(id).is_some() => {
                let library_index = self
                    .unregister_index_from_id(id)
                    .expect("unregister ID was checked before mapping");
                SyncSettingsAction::UnregisterLibrary { library_index }
            }
            _ => return Some(WidgetAction::Consumed),
        };
        self.pending_action = Some(action);
        Some(WidgetAction::Consumed)
    }

    fn take_api_key_draft(&mut self) -> SensitiveText {
        self.draft.api_key.take().unwrap_or_else(|| SensitiveText::new(String::new()))
    }

    fn index_from_id(&self, id: WidgetId, base: WidgetId, count: usize) -> Option<usize> {
        let offset = id.0.checked_sub(base.0)?;
        let index = usize::try_from(offset).ok()?;
        (index < count).then_some(index)
    }

    fn pending_index_from_id(&self, id: WidgetId) -> Option<usize> {
        self.index_from_id(id, PENDING_ACCEPT_BASE_ID, self.input.pending_folders.len())
    }

    fn scan_index_from_id(&self, id: WidgetId) -> Option<usize> {
        self.index_from_id(id, SCAN_BASE_ID, self.input.libraries.len())
    }

    fn pause_index_from_id(&self, id: WidgetId) -> Option<usize> {
        self.index_from_id(id, PAUSE_BASE_ID, self.input.libraries.len())
    }

    fn repair_index_from_id(&self, id: WidgetId) -> Option<usize> {
        self.index_from_id(id, REPAIR_BASE_ID, self.input.libraries.len())
    }

    fn remove_mapping_index_from_id(&self, id: WidgetId) -> Option<usize> {
        self.index_from_id(id, REMOVE_MAPPING_BASE_ID, self.input.libraries.len())
    }

    fn unregister_index_from_id(&self, id: WidgetId) -> Option<usize> {
        self.index_from_id(id, UNREGISTER_BASE_ID, self.input.libraries.len())
    }

    fn rebuild_form(&mut self, ctx: &mut LayoutCtx) {
        let sections = self.build_sections();
        if self.form_has_sections {
            self.form.replace_sections_preserving_state(sections, ctx);
        } else {
            self.form.set_sections(sections, ctx);
            self.form_has_sections = true;
        }
        self.form_needs_rebuild = false;
    }

    fn build_sections(&self) -> Vec<FormSection> {
        vec![
            self.connection_section(),
            self.publish_section(),
            self.pending_folders_section(),
            self.libraries_section(),
            self.notices_section(),
        ]
    }

    fn connection_section(&self) -> FormSection {
        let mut endpoint = sync_text_box(ENDPOINT_ID);
        endpoint.set_placeholder("http://127.0.0.1:8384");
        endpoint.set_max_len_bytes(512);
        endpoint.set_text(&self.draft.endpoint);

        let mut api_key = sync_text_box(API_KEY_ID);
        api_key.set_placeholder(if self.input.has_api_key {
            "已保存 API Key（输入以替换）"
        } else {
            "API Key"
        });
        api_key.set_password_mode(true);
        api_key.set_max_len_bytes(512);
        if let Some(value) = &self.draft.api_key {
            api_key.set_text(value.expose());
        }

        FormSection::new(
            section_title_label("连接"),
            Some(section_description_label(&connection_description(&self.input.connection))),
            vec![
                connection_field_row(
                    "Syncthing 地址",
                    Some("仅支持本机 Syncthing 服务。"),
                    Box::new(endpoint),
                ),
                connection_field_row(
                    "API Key",
                    Some("密钥仅保留在本次操作中。"),
                    Box::new(api_key),
                ),
                connection_action_row(
                    "连接操作",
                    self.connection_actions(),
                    self.stack_connection_actions,
                ),
            ],
            connection_section_style(self.compact_connection_layout),
        )
    }

    fn connection_actions(&self) -> Box<dyn Widget> {
        let test_button = action_button(TEST_CONNECTION_ID, "测试连接", true, self.settings_theme);
        let configure_button =
            action_button(CONFIGURE_CONNECTION_ID, "保存连接", true, self.settings_theme);
        if self.stack_connection_actions {
            return Box::new(StackedConnectionActions::new(test_button, configure_button));
        }

        Box::new(
            InlineGroup::new(vec![
                InlineChild::fixed(Box::new(test_button), SYNC_BUTTON_WIDTH_LOGICAL)
                    .with_cross_size(SYNC_CONTROL_HEIGHT_LOGICAL),
                InlineChild::fixed(Box::new(configure_button), SYNC_BUTTON_WIDTH_LOGICAL)
                    .with_cross_size(SYNC_CONTROL_HEIGHT_LOGICAL),
            ])
            .with_alignment(CrossAlignment::Center),
        )
    }

    fn publish_section(&self) -> FormSection {
        let mut remote_device_id = sync_text_box(REMOTE_DEVICE_ID);
        remote_device_id.set_placeholder("远端 Device ID");
        remote_device_id.set_max_len_bytes(256);
        remote_device_id.set_text(&self.draft.remote_device_id);

        let mut remote_name = sync_text_box(REMOTE_NAME_ID);
        remote_name.set_placeholder("远端设备名称");
        remote_name.set_max_len_bytes(256);
        remote_name.set_text(&self.draft.remote_name);

        let mut remote_addresses = sync_text_box(REMOTE_ADDRESSES_ID);
        remote_addresses.set_placeholder("tcp://host:22000, dynamic");
        remote_addresses.set_max_len_bytes(1024);
        remote_addresses.set_text(&self.draft.remote_addresses);

        FormSection::new(
            section_title_label("发布资料库"),
            Some(section_description_label("选择本地目录后，将其共享给指定远端设备。")),
            vec![
                form_row("远端 Device ID", None, Box::new(remote_device_id)),
                form_row("远端名称", None, Box::new(remote_name)),
                form_row("远端地址", Some("多个地址以逗号分隔。"), Box::new(remote_addresses)),
                form_row(
                    "发布",
                    None,
                    Box::new(single_button_group(
                        PUBLISH_LIBRARY_ID,
                        "选择目录并发布资料库",
                        true,
                        self.settings_theme,
                    )),
                ),
            ],
            section_style(),
        )
    }

    fn pending_folders_section(&self) -> FormSection {
        let rows = self
            .input
            .pending_folders
            .iter()
            .enumerate()
            .map(|(index, folder)| {
                form_row(
                    &folder.folder_id,
                    Some(&format!("来自设备 {}", folder.offered_by)),
                    Box::new(single_button_group(
                        indexed_id(PENDING_ACCEPT_BASE_ID, index),
                        "选择空目录",
                        true,
                        self.settings_theme,
                    )),
                )
            })
            .collect();
        FormSection::new(
            section_title_label("待接收资料库"),
            Some(section_description_label("接受前请选择一个空目录。")),
            rows,
            section_style(),
        )
    }

    fn libraries_section(&self) -> FormSection {
        let rows = self
            .input
            .libraries
            .iter()
            .enumerate()
            .flat_map(|(index, library)| self.library_action_rows(index, library))
            .collect();
        FormSection::new(
            section_title_label("已注册资料库"),
            Some(section_description_label("管理同步、映射和修复操作。")),
            rows,
            section_style(),
        )
    }

    fn library_action_rows(&self, index: usize, library: &LibraryView) -> Vec<FormRow> {
        let library_description =
            format!("{} · {}", library.root_display, library_state_label(&library.state));
        vec![
            form_row(
                &library.name,
                Some(&library_description),
                Box::new(single_button_group(
                    indexed_id(SCAN_BASE_ID, index),
                    "扫描",
                    true,
                    self.settings_theme,
                )),
            ),
            form_row(
                "同步状态",
                None,
                Box::new(single_button_group(
                    indexed_id(PAUSE_BASE_ID, index),
                    if matches!(library.state, LibrarySyncState::Paused) {
                        "继续"
                    } else {
                        "暂停"
                    },
                    true,
                    self.settings_theme,
                )),
            ),
            form_row(
                "修复索引",
                None,
                Box::new(single_button_group(
                    indexed_id(REPAIR_BASE_ID, index),
                    "修复",
                    library.can_repair,
                    self.settings_theme,
                )),
            ),
            form_row(
                "同步映射",
                None,
                Box::new(single_button_group(
                    indexed_id(REMOVE_MAPPING_BASE_ID, index),
                    "移除映射",
                    library.can_remove_mapping,
                    self.settings_theme,
                )),
            ),
            form_row(
                "资料库注册",
                None,
                Box::new(single_button_group(
                    indexed_id(UNREGISTER_BASE_ID, index),
                    "注销",
                    library.can_unregister,
                    self.settings_theme,
                )),
            ),
        ]
    }

    fn notices_section(&self) -> FormSection {
        let rows = self
            .input
            .notices
            .iter()
            .map(|notice| {
                form_row(
                    notice_severity_label(&notice.severity),
                    None,
                    Box::new(description_label(&notice.message)),
                )
            })
            .collect();
        FormSection::new(
            section_title_label("同步通知"),
            Some(section_description_label("连接和资料库操作的最新结果。")),
            rows,
            section_style(),
        )
    }

    pub(super) fn focused_ime_cursor_rect(&self) -> Option<Rect> {
        self.form.focused_ime_cursor_rect()
    }

    #[cfg(test)]
    fn scroll_for_test(&mut self, delta: f32) {
        let theme = ui::theme::test_theme();
        let mut ctx = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
        let _ =
            self.form.on_event(&Event::Wheel { dx: 0.0, dy: -delta, px: 0.0, py: 0.0 }, &mut ctx);
    }

    #[cfg(test)]
    fn rebuild_for_test(&mut self) {
        let theme = ui::theme::test_theme();
        let mut measure = ui::core::measure::NoopMeasure;
        let mut layout =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        self.form.set_rect(self.rect, &mut layout);
        self.rebuild_form(&mut layout);
    }

    #[cfg(test)]
    pub(super) fn focused_id(&self) -> Option<WidgetId> {
        self.form.focused_id()
    }

    #[cfg(test)]
    pub(super) fn scroll_offset(&self) -> f32 {
        self.form.scroll_offset()
    }

    #[cfg(test)]
    pub(super) fn api_key_draft_for_test(&self) -> Option<&str> {
        self.draft.api_key.as_ref().map(SensitiveText::expose)
    }

    #[cfg(test)]
    fn dynamic_row_rects_for_test(&self) -> Vec<Rect> {
        let dynamic_row_count = self.input.pending_folders.len()
            + self.input.libraries.len()
            + self.input.notices.len();
        (0..dynamic_row_count)
            .map(|index| {
                Rect::new(
                    0.0,
                    index as f32 * SYNC_ROW_HEIGHT_LOGICAL,
                    self.rect.w,
                    SYNC_ROW_HEIGHT_LOGICAL,
                )
            })
            .collect()
    }
}

impl Widget for SyncSettingsPage {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
        self.rect = Rect::new(0.0, 0.0, rect.w.max(0.0), rect.h.max(0.0));
        let connection_content_width =
            (self.rect.w - 2.0 * SYNC_ROW_HORIZONTAL_INSET_LOGICAL * ctx.dpi).max(0.0);
        let compact_connection_layout =
            connection_content_width < SYNC_CONNECTION_STACK_THRESHOLD_LOGICAL * ctx.dpi;
        let stack_connection_actions =
            connection_content_width < SYNC_CONNECTION_ACTION_STACK_THRESHOLD_LOGICAL * ctx.dpi;
        if self.compact_connection_layout != compact_connection_layout {
            self.compact_connection_layout = compact_connection_layout;
            self.form_needs_rebuild = true;
        }
        if self.stack_connection_actions != stack_connection_actions {
            self.stack_connection_actions = stack_connection_actions;
            self.form_needs_rebuild = true;
        }
        let settings_theme = ctx.theme.settings_theme();
        if self.settings_theme != settings_theme {
            self.settings_theme = settings_theme;
            self.form_needs_rebuild = true;
        }
        self.form.set_rect(self.rect, ctx);
        if self.form_needs_rebuild {
            self.rebuild_form(ctx);
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        self.form.paint(ctx);
    }

    fn hit(&self, px: f32, py: f32) -> bool {
        self.rect.contains(px, py)
    }

    fn collect_focusable_ids(&self, output: &mut Vec<WidgetId>) {
        self.form.collect_focusable_ids(output);
    }

    fn set_keyboard_focus(&mut self, focused_id: Option<WidgetId>) {
        self.form.set_keyboard_focus(focused_id);
    }

    fn collect_accessibility_nodes(
        &self,
        context: &AccessibilityContext,
        output: &mut Vec<AccessibilityNode>,
    ) {
        self.form.collect_accessibility_nodes(context, output);
    }

    fn on_accessibility_action(
        &mut self,
        request: &AccessibilityActionRequest,
    ) -> Option<WidgetAction> {
        let action = self.form.on_accessibility_action(request)?;
        match action {
            WidgetAction::Control(control_action) => self.handle_control_action(control_action),
            other => Some(other),
        }
    }

    fn on_event(&mut self, event: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
        let action = self.form.on_event(event, ctx)?;
        match action {
            WidgetAction::Control(control_action) => self.handle_control_action(control_action),
            other => Some(other),
        }
    }

    fn is_capturing(&self) -> bool {
        self.form.is_capturing()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn sync_text_box(id: WidgetId) -> TextBox {
    let mut text_box = TextBox::with_id(id);
    text_box.set_fixed_size_logical(SYNC_TEXT_BOX_WIDTH_LOGICAL, SYNC_CONTROL_HEIGHT_LOGICAL);
    text_box.set_blink(true);
    text_box
}

fn parse_remote_addresses(raw_addresses: &str) -> Vec<String> {
    raw_addresses
        .split(',')
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .collect()
}

fn indexed_id(base: WidgetId, index: usize) -> WidgetId {
    let index = u64::try_from(index).expect("widget collection index must fit in u64");
    WidgetId(base.0.checked_add(index).expect("widget ID index must not overflow u64"))
}

fn connection_description(connection: &SyncConnectionView) -> String {
    match connection {
        SyncConnectionView::NotConfigured => "尚未配置 Syncthing。".to_owned(),
        SyncConnectionView::Connecting => "正在连接 Syncthing…".to_owned(),
        SyncConnectionView::Connected { device_id, version } => {
            format!("已连接 {version} · Device {}", device_id.chars().take(8).collect::<String>())
        }
        SyncConnectionView::AuthenticationRequired => "需要有效的 API Key。".to_owned(),
        SyncConnectionView::Incompatible { found } => format!("版本不兼容：{found}"),
        SyncConnectionView::Unavailable { message } => format!("连接不可用：{message}"),
    }
}

fn library_state_label(state: &LibrarySyncState) -> &str {
    match state {
        LibrarySyncState::Pending => "等待处理",
        LibrarySyncState::Scanning => "正在扫描",
        LibrarySyncState::Syncing => "正在同步",
        LibrarySyncState::UpToDate => "已是最新",
        LibrarySyncState::Paused => "已暂停",
        LibrarySyncState::AwaitingRemoteAcceptance => "等待远端接受",
        LibrarySyncState::ConfigurationMismatch => "配置不一致",
        LibrarySyncState::Error { .. } => "发生错误",
    }
}

fn notice_severity_label(severity: &SyncNoticeSeverity) -> &'static str {
    match severity {
        SyncNoticeSeverity::Info => "提示",
        SyncNoticeSeverity::Warning => "警告",
        SyncNoticeSeverity::Error => "错误",
    }
}

fn form_row(label: &str, description: Option<&str>, control: Box<dyn Widget>) -> FormRow {
    FormRow::new(row_label(label), description.map(description_label), control, row_style())
}

fn connection_field_row(
    label: &str,
    description: Option<&str>,
    control: Box<dyn Widget>,
) -> FormRow {
    FormRow::new(
        row_label(label),
        description.map(description_label),
        control,
        connection_field_row_style(),
    )
}

fn connection_action_row(label: &str, control: Box<dyn Widget>, stacked: bool) -> FormRow {
    let style =
        if stacked { stacked_connection_action_row_style() } else { connection_action_row_style() };
    FormRow::new(row_label(label), None, control, style)
}

fn row_style() -> FormRowStyle {
    FormRowStyle {
        min_height_logical: SYNC_ROW_HEIGHT_LOGICAL,
        label_width_logical: SYNC_ROW_LABEL_WIDTH_LOGICAL,
        column_gap_logical: 12.0,
        responsive_threshold_logical: 0.0,
        padding_logical: [
            SYNC_ROW_VERTICAL_INSET_LOGICAL,
            SYNC_ROW_HORIZONTAL_INSET_LOGICAL,
            SYNC_ROW_VERTICAL_INSET_LOGICAL,
            SYNC_ROW_HORIZONTAL_INSET_LOGICAL,
        ],
        ..FormRowStyle::default()
    }
}

fn connection_action_row_style() -> FormRowStyle {
    FormRowStyle {
        responsive_threshold_logical: SYNC_CONNECTION_STACK_THRESHOLD_LOGICAL,
        ..row_style()
    }
}

fn connection_field_row_style() -> FormRowStyle {
    FormRowStyle {
        responsive_threshold_logical: SYNC_CONNECTION_FIELD_STACK_THRESHOLD_LOGICAL,
        ..row_style()
    }
}

fn stacked_connection_action_row_style() -> FormRowStyle {
    FormRowStyle {
        label_width_logical: 0.0,
        column_gap_logical: 0.0,
        responsive_threshold_logical: 0.0,
        ..row_style()
    }
}

fn connection_section_style(compact_layout: bool) -> FormSectionStyle {
    FormSectionStyle {
        row_height_logical: if compact_layout {
            SYNC_COMPACT_CONNECTION_ROW_HEIGHT_LOGICAL
        } else {
            SYNC_ROW_HEIGHT_LOGICAL
        },
        ..section_style()
    }
}

fn section_style() -> FormSectionStyle {
    FormSectionStyle {
        title_gap_logical: SYNC_SECTION_TITLE_GAP_LOGICAL,
        description_gap_logical: SYNC_SECTION_DESCRIPTION_GAP_LOGICAL,
        row_height_logical: SYNC_ROW_HEIGHT_LOGICAL,
        corner_radius_logical: SYNC_SECTION_CORNER_RADIUS_LOGICAL,
        ..FormSectionStyle::default()
    }
}

fn action_button(id: WidgetId, text: &str, enabled: bool, settings_theme: SettingsTheme) -> Button {
    let mut button = Button::new(id, action_button_style(settings_theme));
    button.set_text(Some(text.to_owned()));
    button.set_enabled(enabled);
    button
}

fn single_button_group(
    id: WidgetId,
    text: &str,
    enabled: bool,
    settings_theme: SettingsTheme,
) -> InlineGroup {
    InlineGroup::new(vec![
        InlineChild::fixed(
            Box::new(action_button(id, text, enabled, settings_theme)),
            SYNC_WIDE_BUTTON_WIDTH_LOGICAL,
        )
        .with_cross_size(SYNC_CONTROL_HEIGHT_LOGICAL),
    ])
    .with_alignment(CrossAlignment::Center)
}

fn action_button_style(settings: SettingsTheme) -> ButtonStyle {
    ButtonStyle {
        font_size_logical: SYNC_BUTTON_FONT_SIZE_LOGICAL,
        pad_x_logical: SYNC_BUTTON_PADDING_LOGICAL,
        foreground: settings.text_primary,
        selected_foreground: settings.text_primary,
        background: settings.control_surface,
        border: settings.control_border,
        hover_background: blend_color(settings.control_surface, settings.accent, 0.06),
        pressed_background: blend_color(settings.control_surface, settings.accent, 0.16),
        selected_background: settings.control_surface,
        disabled_foreground: with_alpha(settings.text_primary, SYNC_DISABLED_FOREGROUND_ALPHA),
        disabled_background: settings.control_surface,
        corner_radius_logical: SYNC_BUTTON_RADIUS_LOGICAL,
    }
}

fn section_title_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SYNC_SECTION_TITLE_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::MEDIUM,
            ..LabelStyle::default()
        },
    )
}

fn row_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SYNC_ROW_LABEL_FONT_SIZE_LOGICAL,
            font_weight: shaping::Weight::MEDIUM,
            ..LabelStyle::default()
        },
    )
}

fn description_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SYNC_DESCRIPTION_FONT_SIZE_LOGICAL,
            foreground: LabelForeground::ThemeMuted,
            ..LabelStyle::default()
        },
    )
}

fn section_description_label(text: &str) -> Label {
    Label::new(
        text,
        LabelStyle {
            font_size_logical: SYNC_DESCRIPTION_FONT_SIZE_LOGICAL - 0.5,
            foreground: LabelForeground::ThemeMuted,
            ..LabelStyle::default()
        },
    )
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
    ui::theme::test_theme().settings_theme()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_settings_types::{
        LibrarySyncState, LibraryView, PendingFolderView, SyncConnectionView, SyncNoticeSeverity,
        SyncNoticeView, SyncSettingsAction, SyncSettingsInput,
    };
    use ui::core::PaintCtx;
    use ui::core::paint::{DrawCmd, DrawList};
    use ui::core::widget::{ControlAction, SensitiveText, TextPayload};

    #[test]
    fn page_starts_with_pure_not_configured_input() {
        let page = SyncSettingsPage::new(SyncSettingsInput::default());
        assert_eq!(page.input().connection, SyncConnectionView::NotConfigured);
        assert!(!page.input().has_api_key);
    }

    #[test]
    fn sync_page_exposes_sensitive_field_without_value_and_routes_set_value() {
        let mut page = laid_out_page(SyncSettingsInput::default());
        let mut nodes = Vec::new();
        page.collect_accessibility_nodes(
            &ui::core::AccessibilityContext::new(10.0, 20.0),
            &mut nodes,
        );
        let api_key_id = ui::core::AccessibilityId::from(API_KEY_ID);
        let api_key = semantic_node_with_id(&nodes, api_key_id)
            .expect("API key field must be exposed through the sync page");

        assert_eq!(api_key.role, ui::core::AccessibilityRole::TextField);
        assert!(api_key.state.sensitive);
        assert_eq!(api_key.value, None);
        assert_eq!(
            page.on_accessibility_action(&ui::core::AccessibilityActionRequest::set_value(
                api_key_id,
                TextPayload::Sensitive(SensitiveText::new("voiceover-secret".to_owned())),
            )),
            Some(WidgetAction::Consumed)
        );
        assert_eq!(page.api_key_draft_for_test(), Some("voiceover-secret"));
    }

    #[test]
    fn configure_activation_is_consumed_and_can_be_taken_as_a_product_action() {
        let mut page = SyncSettingsPage::new(SyncSettingsInput::default());
        page.handle_control_action(ControlAction::TextEdited {
            id: ENDPOINT_ID,
            value: TextPayload::Plain("http://127.0.0.1:8384".to_owned()),
        });
        page.handle_control_action(ControlAction::TextEdited {
            id: API_KEY_ID,
            value: TextPayload::Sensitive(SensitiveText::new("secret".to_owned())),
        });

        assert_eq!(
            page.handle_control_action(ControlAction::Activated { id: CONFIGURE_CONNECTION_ID }),
            Some(WidgetAction::Consumed),
        );
        assert!(matches!(
            page.take_pending_action(),
            Some(SyncSettingsAction::ConfigureConnection { .. })
        ));
        assert_eq!(page.take_pending_action(), None);
    }

    #[test]
    fn configure_button_emits_redacted_sync_settings_action() {
        let mut page = SyncSettingsPage::new(SyncSettingsInput::default());
        page.handle_control_action(ControlAction::TextEdited {
            id: ENDPOINT_ID,
            value: TextPayload::Plain("http://127.0.0.1:8384".to_owned()),
        });
        page.handle_control_action(ControlAction::TextEdited {
            id: API_KEY_ID,
            value: TextPayload::Sensitive(SensitiveText::new("never-print-me".to_owned())),
        });

        assert_eq!(
            page.handle_control_action(ControlAction::Activated { id: CONFIGURE_CONNECTION_ID }),
            Some(WidgetAction::Consumed),
        );
        let action = page.take_pending_action();
        assert!(matches!(action, Some(SyncSettingsAction::ConfigureConnection { .. })));
        assert!(!format!("{action:?}").contains("never-print-me"));
    }

    #[test]
    fn snapshot_refresh_preserves_drafts_focus_and_scroll() {
        let mut page = laid_out_scrollable_page();
        page.handle_control_action(ControlAction::TextEdited {
            id: API_KEY_ID,
            value: TextPayload::Sensitive(SensitiveText::new("draft-key".to_owned())),
        });
        page.set_keyboard_focus(Some(API_KEY_ID));
        page.scroll_for_test(96.0);
        let previous_scroll = page.scroll_offset();

        page.set_input(connected_input_with_two_libraries());
        page.rebuild_for_test();

        assert_eq!(page.focused_id(), Some(API_KEY_ID));
        assert_eq!(page.scroll_offset(), previous_scroll);
        assert_eq!(page.api_key_draft_for_test(), Some("draft-key"));
    }

    #[test]
    fn snapshot_refresh_preserves_dirty_endpoint_after_focus_moves_to_api_key() {
        const ORIGINAL_ENDPOINT: &str = "http://127.0.0.1:8384";
        const DRAFT_ENDPOINT: &str = "http://127.0.0.1:8385";

        let mut page = laid_out_page(connected_input_with_two_libraries());
        page.handle_control_action(ControlAction::TextEdited {
            id: ENDPOINT_ID,
            value: TextPayload::Plain(DRAFT_ENDPOINT.to_owned()),
        });
        page.set_keyboard_focus(Some(API_KEY_ID));

        let mut refreshed_input = connected_input_with_two_libraries();
        refreshed_input.connection = SyncConnectionView::Connecting;
        assert_eq!(refreshed_input.endpoint, ORIGINAL_ENDPOINT);
        page.set_input(refreshed_input);

        assert_eq!(page.draft.endpoint, DRAFT_ENDPOINT);
    }

    #[test]
    fn matching_snapshot_clears_endpoint_dirty_state() {
        const DRAFT_ENDPOINT: &str = "http://127.0.0.1:8385";

        let mut page = laid_out_page(connected_input_with_two_libraries());
        page.handle_control_action(ControlAction::TextEdited {
            id: ENDPOINT_ID,
            value: TextPayload::Plain(DRAFT_ENDPOINT.to_owned()),
        });

        let mut confirmed_input = connected_input_with_two_libraries();
        confirmed_input.endpoint = DRAFT_ENDPOINT.to_owned();
        page.set_input(confirmed_input);

        assert_eq!(page.draft.endpoint, DRAFT_ENDPOINT);

        let mut later_input = connected_input_with_two_libraries();
        later_input.endpoint = "http://127.0.0.1:8386".to_owned();
        page.set_input(later_input);
        assert_eq!(page.draft.endpoint, "http://127.0.0.1:8386");
    }

    #[test]
    fn testing_connection_preserves_endpoint_draft_across_stale_snapshot_refresh() {
        const ORIGINAL_ENDPOINT: &str = "http://127.0.0.1:8384";
        const DRAFT_ENDPOINT: &str = "http://127.0.0.1:8385";

        let mut page = laid_out_page(connected_input_with_two_libraries());
        page.handle_control_action(ControlAction::TextEdited {
            id: ENDPOINT_ID,
            value: TextPayload::Plain(DRAFT_ENDPOINT.to_owned()),
        });

        assert_eq!(
            page.handle_control_action(ControlAction::Activated { id: TEST_CONNECTION_ID }),
            Some(WidgetAction::Consumed),
        );
        assert!(matches!(
            page.take_pending_action(),
            Some(SyncSettingsAction::TestConnection { endpoint, .. })
                if endpoint == DRAFT_ENDPOINT
        ));

        page.set_input(connected_input_with_two_libraries());
        assert_eq!(page.input.endpoint, ORIGINAL_ENDPOINT);
        assert_eq!(page.draft.endpoint, DRAFT_ENDPOINT);
    }

    #[test]
    fn configuring_connection_preserves_endpoint_until_snapshot_confirms_it() {
        const ORIGINAL_ENDPOINT: &str = "http://127.0.0.1:8384";
        const DRAFT_ENDPOINT: &str = "http://127.0.0.1:8385";
        const LATER_ENDPOINT: &str = "http://127.0.0.1:8386";

        let mut page = laid_out_page(connected_input_with_two_libraries());
        page.handle_control_action(ControlAction::TextEdited {
            id: ENDPOINT_ID,
            value: TextPayload::Plain(DRAFT_ENDPOINT.to_owned()),
        });

        assert_eq!(
            page.handle_control_action(ControlAction::Activated { id: CONFIGURE_CONNECTION_ID }),
            Some(WidgetAction::Consumed),
        );
        assert!(matches!(
            page.take_pending_action(),
            Some(SyncSettingsAction::ConfigureConnection { endpoint, .. })
                if endpoint == DRAFT_ENDPOINT
        ));

        page.set_input(connected_input_with_two_libraries());
        assert_eq!(page.input.endpoint, ORIGINAL_ENDPOINT);
        assert_eq!(page.draft.endpoint, DRAFT_ENDPOINT, "未确认前必须保留提交中的草稿");

        let mut confirmed_input = connected_input_with_two_libraries();
        confirmed_input.endpoint = DRAFT_ENDPOINT.to_owned();
        page.set_input(confirmed_input);
        assert_eq!(page.draft.endpoint, DRAFT_ENDPOINT);

        let mut later_input = connected_input_with_two_libraries();
        later_input.endpoint = LATER_ENDPOINT.to_owned();
        page.set_input(later_input);
        assert_eq!(page.draft.endpoint, LATER_ENDPOINT, "确认后应恢复跟随后台快照");
    }

    #[test]
    fn empty_snapshot_refresh_keeps_notices_without_rebuilding_the_form() {
        let mut input = SyncSettingsInput::default();
        input.notices.push(SyncNoticeView {
            severity: SyncNoticeSeverity::Error,
            message: "同步操作失败".to_owned(),
        });
        let mut page = laid_out_page(input);
        assert!(!page.form_needs_rebuild);

        page.set_input(SyncSettingsInput::default());

        assert_eq!(page.input().notices.len(), 1);
        assert!(!page.form_needs_rebuild);
    }

    #[test]
    fn testing_connection_rebuilds_the_cleared_api_key_field() {
        let mut page = laid_out_page(SyncSettingsInput::default());
        page.handle_control_action(ControlAction::TextEdited {
            id: API_KEY_ID,
            value: TextPayload::Sensitive(SensitiveText::new("test-only-key".to_owned())),
        });

        assert_eq!(
            page.handle_control_action(ControlAction::Activated { id: TEST_CONNECTION_ID }),
            Some(WidgetAction::Consumed),
        );
        assert!(matches!(
            page.take_pending_action(),
            Some(SyncSettingsAction::TestConnection { .. })
        ));
        assert_eq!(page.api_key_draft_for_test(), None);
        assert!(page.form_needs_rebuild, "测试连接取走 API Key 后必须重建表单，以清除旧的掩码文本");
        page.rebuild_for_test();
        let contains_masked_api_key = paint_for_test(&page).cmds.iter().any(|command| {
            matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text.contains('•'))
        });
        assert!(!contains_masked_api_key, "重建后的 API Key 输入框应为空");
    }

    #[test]
    fn stacked_connection_actions_clear_previous_hover_target() {
        let theme = ui::theme::test_theme();
        let settings_theme = theme.settings_theme();
        let mut measure = ui::core::measure::NoopMeasure;
        let mut layout =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        let mut actions = StackedConnectionActions::new(
            action_button(TEST_CONNECTION_ID, "测试连接", true, settings_theme),
            action_button(CONFIGURE_CONNECTION_ID, "保存连接", true, settings_theme),
        );
        actions.set_rect(Rect::new(0.0, 0.0, 160.0, 80.0), &mut layout);
        let mut event_ctx = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };

        let first_button = actions.button_rects[0];
        let second_button = actions.button_rects[1];
        let _ = actions.on_event(
            &Event::MouseMove {
                px: first_button.x + first_button.w * 0.5,
                py: first_button.y + first_button.h * 0.5,
            },
            &mut event_ctx,
        );
        assert_eq!(
            stacked_action_backgrounds(&actions),
            [settings_theme_hover_color(settings_theme), settings_theme.control_surface],
        );

        let _ = actions.on_event(
            &Event::MouseMove {
                px: second_button.x + second_button.w * 0.5,
                py: second_button.y + second_button.h * 0.5,
            },
            &mut event_ctx,
        );
        assert_eq!(
            stacked_action_backgrounds(&actions),
            [settings_theme.control_surface, settings_theme_hover_color(settings_theme)],
            "同组按钮不得同时保持 hover",
        );

        let _ = actions.on_event(
            &Event::MouseMove { px: actions.rect.right() + 1.0, py: actions.rect.bottom() + 1.0 },
            &mut event_ctx,
        );
        assert_eq!(
            stacked_action_backgrounds(&actions),
            [settings_theme.control_surface; 2],
            "鼠标移出纵向按钮组后应清除全部 hover",
        );
    }

    #[test]
    fn stacked_connection_actions_cancel_pressed_button_without_activation() {
        let theme = ui::theme::test_theme();
        let settings_theme = theme.settings_theme();
        let mut measure = ui::core::measure::NoopMeasure;
        let mut layout =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        let mut actions = StackedConnectionActions::new(
            action_button(TEST_CONNECTION_ID, "测试连接", true, settings_theme),
            action_button(CONFIGURE_CONNECTION_ID, "保存连接", true, settings_theme),
        );
        actions.set_rect(Rect::new(0.0, 0.0, 160.0, 80.0), &mut layout);
        let mut event_ctx = EventCtx { theme: &theme, dpi: 1.0, cursor_hint: None };
        let first_button = actions.button_rects[0];
        let pointer =
            (first_button.x + first_button.w * 0.5, first_button.y + first_button.h * 0.5);

        assert!(
            actions
                .on_event(
                    &Event::MouseDown {
                        px: pointer.0,
                        py: pointer.1,
                        button: ui::core::MouseButton::Left,
                    },
                    &mut event_ctx,
                )
                .is_some()
        );
        assert!(actions.is_capturing());
        let _ = actions.on_event(&Event::PointerLeave, &mut event_ctx);
        assert!(actions.is_capturing());

        assert_eq!(
            actions.on_event(&Event::InteractionCancel, &mut event_ctx),
            Some(WidgetAction::Consumed)
        );
        assert!(!actions.is_capturing());
        assert_eq!(actions.on_event(&Event::InteractionCancel, &mut event_ctx), None);
        assert_eq!(
            actions.on_event(
                &Event::MouseUp {
                    px: pointer.0,
                    py: pointer.1,
                    button: ui::core::MouseButton::Left,
                },
                &mut event_ctx,
            ),
            None
        );
    }

    #[test]
    fn narrow_library_rows_paint_all_five_action_buttons_at_full_width() {
        let page = laid_out_page_at_width(380.0, 1_200.0, connected_input_with_two_libraries());
        let draw = paint_for_test(&page);
        let settings_theme = ui::theme::test_theme().settings_theme();
        let button_rects = draw
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::FillRect { rect, color, radius }
                    if *color == settings_theme.control_surface
                        && *radius == SYNC_BUTTON_RADIUS_LOGICAL
                        && rect.h == SYNC_CONTROL_HEIGHT_LOGICAL
                        && rect.y >= 750.0 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(button_rects.len(), 10, "两个资料库应各有五个可见操作按钮");
        assert!(button_rects.iter().all(|rect| rect.w >= SYNC_DYNAMIC_BUTTON_WIDTH_LOGICAL));
    }

    #[test]
    fn dynamic_rows_and_notices_have_distinct_vertical_positions() {
        let page = laid_out_page(sync_input_with_two_pending_two_libraries_two_notices());
        let rows = page.dynamic_row_rects_for_test();
        assert_eq!(rows.len(), 6);
        assert!(rows.windows(2).all(|pair| pair[0].y < pair[1].y));
    }

    fn laid_out_page(input: SyncSettingsInput) -> SyncSettingsPage {
        laid_out_page_at_width(520.0, 260.0, input)
    }

    fn semantic_node_with_id(
        nodes: &[ui::core::AccessibilityNode],
        id: ui::core::AccessibilityId,
    ) -> Option<&ui::core::AccessibilityNode> {
        nodes.iter().find_map(|node| {
            if node.id == id {
                return Some(node);
            }
            semantic_node_with_id(&node.children, id)
        })
    }

    fn laid_out_page_at_width(
        width: f32,
        height: f32,
        input: SyncSettingsInput,
    ) -> SyncSettingsPage {
        let theme = ui::theme::test_theme();
        let mut measure = ui::core::measure::NoopMeasure;
        let mut layout =
            LayoutCtx { measure: &mut measure, ui_measure: None, theme: &theme, dpi: 1.0 };
        let mut page = SyncSettingsPage::new(input);
        page.set_rect(Rect::new(0.0, 0.0, width, height), &mut layout);
        page
    }

    fn laid_out_scrollable_page() -> SyncSettingsPage {
        laid_out_page(sync_input_with_two_pending_two_libraries_two_notices())
    }

    fn connected_input_with_two_libraries() -> SyncSettingsInput {
        SyncSettingsInput {
            endpoint: "http://127.0.0.1:8384".to_owned(),
            has_api_key: true,
            connection: SyncConnectionView::Connected {
                device_id: "LOCAL-DEVICE".to_owned(),
                version: "2.1.1".to_owned(),
            },
            libraries: vec![
                test_library("Notes", "/tmp/notes"),
                test_library("Archive", "/tmp/archive"),
            ],
            pending_folders: Vec::new(),
            notices: Vec::new(),
        }
    }

    fn sync_input_with_two_pending_two_libraries_two_notices() -> SyncSettingsInput {
        let mut input = connected_input_with_two_libraries();
        input.pending_folders = vec![
            PendingFolderView {
                folder_id: "incoming-a".to_owned(),
                offered_by: "REMOTE-A".to_owned(),
            },
            PendingFolderView {
                folder_id: "incoming-b".to_owned(),
                offered_by: "REMOTE-B".to_owned(),
            },
        ];
        input.notices = vec![
            SyncNoticeView {
                severity: SyncNoticeSeverity::Info, message: "同步完成".to_owned()
            },
            SyncNoticeView {
                severity: SyncNoticeSeverity::Warning,
                message: "需要刷新".to_owned(),
            },
        ];
        input
    }

    fn test_library(name: &str, root_display: &str) -> LibraryView {
        LibraryView {
            name: name.to_owned(),
            root_display: root_display.to_owned(),
            state: LibrarySyncState::UpToDate,
            can_repair: false,
            can_remove_mapping: true,
            can_unregister: true,
        }
    }

    fn paint_for_test(page: &SyncSettingsPage) -> DrawList {
        let theme = ui::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        page.paint(&mut paint);
        draw_list
    }

    fn stacked_action_backgrounds(actions: &StackedConnectionActions) -> [[f32; 4]; 2] {
        let theme = ui::theme::test_theme();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let mut paint = PaintCtx {
            global_alpha: 1.0,
            list: &mut draw_list,
            theme: &theme,
            dpi: 1.0,
            offset: (0.0, 0.0),
            shaper: Some(&mut shaper),
        };
        actions.paint(&mut paint);

        let mut backgrounds = draw_list.cmds.iter().filter_map(|command| match command {
            DrawCmd::FillRect { rect, color, radius }
                if *radius == SYNC_BUTTON_RADIUS_LOGICAL && actions.button_rects.contains(rect) =>
            {
                Some(*color)
            }
            _ => None,
        });
        [
            backgrounds.next().expect("first stacked button background should be painted"),
            backgrounds.next().expect("second stacked button background should be painted"),
        ]
    }

    fn settings_theme_hover_color(settings_theme: SettingsTheme) -> [f32; 4] {
        blend_color(settings_theme.control_surface, settings_theme.accent, 0.06)
    }
}
