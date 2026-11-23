mod buttons;
mod card_model;
mod editor_model;
mod event_routing;
mod focus;
mod model;
mod painting;
mod widget_actions;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::encrypted_note_dialog::EncryptedNoteDialog;
pub use crate::encrypted_note_dialog::{EncryptedNoteDialogInput, EncryptedNoteDialogMode};
use crate::encrypted_note_unlock::EncryptedNoteUnlock;
pub use crate::encrypted_note_unlock::EncryptedNoteUnlockInput;
use appkit_shell::inspector_panel::InspectorPanelStamp;
#[cfg(test)]
use notora_core::NavigationScope;
use notora_core::{DocumentIdentity, DocumentKind, NoteId};
use ui::Rect;
use ui::canvas_scrollbars::{CanvasScrollbarsInput, CanvasScrollbarsWidget};
use ui::core::widget::WidgetId;
use ui::inspector_panel::InspectorPanelWidget;
use ui::popup_menu::PopupMenuWidget;
use ui::split_button::{SplitButtonInput, SplitButtonWidget};
use ui::splitter::SplitterWidget;
use ui::status_state::{StatusStateInput, StatusStateKind, StatusStateWidget};
use ui::text_box::TextBox;
use ui::tooltip::TooltipHint;
use ui::tree_list::{TreeListInput, TreeListWidget, TreeRowActionKey, TreeRowKey};
use ui::virtual_card_list::{
    CardInput, CardKey, CardSelection, VirtualCardListInput, VirtualCardListWidget,
};
#[cfg(test)]
use ui::{Event, EventCtx};

use crate::action::{NotoraAction, TrashOperation};
use crate::editor_pane::EditorPaneChrome;
use crate::new_workspace_dialog::{NewWorkspaceDialog, NewWorkspaceDialogInput};
use crate::settings_overlay::SettingsOverlay;
use crate::{FocusTarget, OverlayState};

use self::buttons::{ChromeButton, ChromeButtonKey};
#[cfg(test)]
use self::card_model::note_toolbar_buttons;
use self::event_routing::{event_pointer_position, is_confirmation_overlay};
use self::model::{STARRED_NAVIGATION_KEY, TRASH_NAVIGATION_KEY};

pub use self::card_model::{NewNoteControlState, NoteToolbarButtonInput, RenderCard};
pub use self::editor_model::EditorPaneState;
pub(crate) use self::editor_model::{
    add_compact_editor_toolbar_commands, add_document_search_command, add_source_toggle_command,
    editor_toolbar_input_for_plugin, selected_editor_pane_mode,
};
pub use self::event_routing::{CanvasViewCommand, NotoraEventRoute};
pub use self::model::{ConfirmationOverlayInput, NotoraRenderModel, SaveConflictOverlayInput};
pub(crate) use self::painting::editor_inspector_rects;

const GLOBAL_SEARCH_BOX_ID: WidgetId = WidgetId(9_000);
const SETTINGS_BUTTON_ID: WidgetId = WidgetId(9_001);
const NEW_NOTE_BUTTON_ID: WidgetId = WidgetId(9_002);
const NEW_NOTE_MENU_BUTTON_ID: WidgetId = WidgetId(9_003);
const SET_WORKSPACE_ROOT_BUTTON_ID: WidgetId = WidgetId(9_004);
const NAVIGATION_TREE_ID: WidgetId = WidgetId(9_005);
const NOTE_TOOL_ICON_SIZE_LOGICAL: f32 = 14.0;
const SEARCH_ICON_AREA_WIDTH_LOGICAL: f32 = 32.0;
const SIDEBAR_ICON_SIZE_LOGICAL: f32 = 16.0;
pub(crate) const DOCUMENT_SEARCH_COMMAND_KEY: &str = "document_search";

struct RenderedToolbarButton {
    icon: Option<&'static str>,
    rect: Rect,
    label: String,
    action: NotoraAction,
}

/// 三栏静态壳；仅持有通用 widget 与当帧键到产品动作的映射。
pub struct NotoraShell {
    search_box: TextBox,
    navigation_tree: TreeListWidget,
    active_tooltip: Option<TooltipHint>,
    card_list: VirtualCardListWidget,
    card_empty_state: StatusStateWidget,
    card_empty_state_visible: bool,
    editor_empty_state: StatusStateWidget,
    canvas_scrollbars: CanvasScrollbarsWidget,
    canvas_scrollbars_input: Option<CanvasScrollbarsInput>,
    canvas_rect: Rect,
    navigation_splitter: SplitterWidget,
    card_list_splitter: SplitterWidget,
    new_note_button: SplitButtonWidget,
    default_new_document_kind: DocumentKind,
    new_document_menu: Option<PopupMenuWidget>,
    editor_pane: EditorPaneChrome,
    editor_note_id: Option<NoteId>,
    editor_location_actions: HashMap<String, NotoraAction>,
    editor_tag_actions: HashMap<String, NotoraAction>,
    editor_command_actions: HashMap<String, NotoraAction>,
    inspector_panel: InspectorPanelWidget,
    inspector_panel_stamp: Option<InspectorPanelStamp>,
    inspector_panel_rect: Rect,
    settings_overlay: SettingsOverlay,
    settings_overlay_open: bool,
    new_workspace_dialog: Option<NewWorkspaceDialog>,
    new_workspace_dialog_input: NewWorkspaceDialogInput,
    new_workspace_dialog_open: bool,
    encrypted_note_dialog: Option<EncryptedNoteDialog>,
    encrypted_note_dialog_input: EncryptedNoteDialogInput,
    encrypted_note_dialog_open: bool,
    encrypted_note_unlock: Option<EncryptedNoteUnlock>,
    encrypted_note_unlock_input: EncryptedNoteUnlockInput,
    encrypted_note_unlock_open: bool,
    navigation_actions: HashMap<TreeRowKey, NotoraAction>,
    navigation_trailing_actions: HashMap<(TreeRowKey, TreeRowActionKey), NotoraAction>,
    navigation_expansion_paths: HashMap<TreeRowKey, std::path::PathBuf>,
    card_identities: HashMap<CardKey, DocumentIdentity>,
    card_keys: HashMap<DocumentIdentity, CardKey>,
    next_card_key: u64,
    search_rect: Rect,
    navigation_tree_rect: Rect,
    card_content_rect: Rect,
    editor_rect: Rect,
    settings_rect: Rect,
    new_document_menu_rect: Rect,
    new_document_menu_open: bool,
    note_toolbar_buttons: Vec<RenderedToolbarButton>,
    chrome_buttons: Vec<ChromeButton>,
    compact_navigation_rect: Rect,
    navigation_collapse_rect: Rect,
    navigation_expand_rect: Rect,
    compact_back_rect: Rect,
    confirmation_panel_rect: Rect,
    confirmation_confirm_rect: Rect,
    confirmation_cancel_rect: Rect,
    confirmation_action: Option<NotoraAction>,
    save_conflict_panel_rect: Rect,
    save_conflict_button_rects: [Rect; 4],
    save_conflict_actions: Option<[NotoraAction; 4]>,
    focused_text_input: Option<FocusTarget>,
}

impl Default for NotoraShell {
    fn default() -> Self {
        Self::new()
    }
}

impl NotoraShell {
    pub fn new() -> Self {
        let mut search_box = TextBox::with_id(GLOBAL_SEARCH_BOX_ID);
        search_box.set_placeholder("搜索笔记...");
        search_box.set_max_len_bytes(2_048);
        search_box.set_leading_content_inset_logical(SEARCH_ICON_AREA_WIDTH_LOGICAL);
        let mut new_note_button = SplitButtonWidget::new();
        new_note_button.set_presentation(ui::split_button::SplitButtonPresentation::Toolbar);
        new_note_button.set_action_ids(NEW_NOTE_BUTTON_ID, NEW_NOTE_MENU_BUTTON_ID);
        new_note_button.set_icon(Some("plus".to_owned()));
        Self {
            search_box,
            navigation_tree: TreeListWidget::new().with_id(NAVIGATION_TREE_ID),
            active_tooltip: None,
            card_list: VirtualCardListWidget::new(),
            card_empty_state: StatusStateWidget::new(),
            card_empty_state_visible: false,
            editor_empty_state: StatusStateWidget::new(),
            canvas_scrollbars: CanvasScrollbarsWidget::new(),
            canvas_scrollbars_input: None,
            canvas_rect: Rect::ZERO,
            navigation_splitter: SplitterWidget::new(),
            card_list_splitter: SplitterWidget::new(),
            new_note_button,
            default_new_document_kind: DocumentKind::Markdown,
            new_document_menu: None,
            editor_pane: EditorPaneChrome::new(),
            editor_note_id: None,
            editor_location_actions: HashMap::new(),
            editor_tag_actions: HashMap::new(),
            editor_command_actions: HashMap::new(),
            inspector_panel: InspectorPanelWidget::new(),
            inspector_panel_stamp: None,
            inspector_panel_rect: Rect::ZERO,
            settings_overlay: SettingsOverlay::new(),
            settings_overlay_open: false,
            new_workspace_dialog: None,
            new_workspace_dialog_input: NewWorkspaceDialogInput::default(),
            new_workspace_dialog_open: false,
            encrypted_note_dialog: None,
            encrypted_note_dialog_input: EncryptedNoteDialogInput::create(),
            encrypted_note_dialog_open: false,
            encrypted_note_unlock: None,
            encrypted_note_unlock_input: EncryptedNoteUnlockInput::new(String::new()),
            encrypted_note_unlock_open: false,
            navigation_actions: HashMap::new(),
            navigation_trailing_actions: HashMap::new(),
            navigation_expansion_paths: HashMap::new(),
            card_identities: HashMap::new(),
            card_keys: HashMap::new(),
            next_card_key: 1,
            search_rect: Rect::ZERO,
            navigation_tree_rect: Rect::ZERO,
            card_content_rect: Rect::ZERO,
            editor_rect: Rect::ZERO,
            settings_rect: Rect::ZERO,
            new_document_menu_rect: Rect::ZERO,
            new_document_menu_open: false,
            note_toolbar_buttons: Vec::new(),
            chrome_buttons: Vec::new(),
            compact_navigation_rect: Rect::ZERO,
            navigation_collapse_rect: Rect::ZERO,
            navigation_expand_rect: Rect::ZERO,
            compact_back_rect: Rect::ZERO,
            confirmation_panel_rect: Rect::ZERO,
            confirmation_confirm_rect: Rect::ZERO,
            confirmation_cancel_rect: Rect::ZERO,
            confirmation_action: None,
            save_conflict_panel_rect: Rect::ZERO,
            save_conflict_button_rects: [Rect::ZERO; 4],
            save_conflict_actions: None,
            focused_text_input: None,
        }
    }

    pub(crate) fn editor_title_text(&self) -> &str {
        self.editor_pane.title_text()
    }

    pub(crate) fn editor_popup_is_open(&self) -> bool {
        self.editor_pane.has_open_popup()
    }

    pub(crate) fn navigation_toggle_available(&self, overlay: OverlayState) -> bool {
        self.chrome_button_available(ChromeButtonKey::NavigationCollapse, Some(overlay), None)
    }

    pub fn update_model(&mut self, model: &NotoraRenderModel) {
        self.navigation_actions.clone_from(&model.navigation_actions);
        self.navigation_trailing_actions.clone_from(&model.navigation_trailing_actions);
        self.navigation_expansion_paths.clone_from(&model.navigation_expansion_paths);
        self.card_identities.clear();
        let cards = model
            .cards
            .iter()
            .map(|card| {
                let key = self.card_key_for(card.identity);
                self.card_identities.insert(key, card.identity);
                CardInput {
                    key,
                    title: card.title.clone(),
                    excerpt: card.excerpt.clone(),
                    timestamp: card.timestamp.clone(),
                    icon: card.icon.clone(),
                    tag_summary: card.tag_summary.clone(),
                    selection: if model.selected_card == Some(card.identity) {
                        CardSelection::Selected
                    } else {
                        CardSelection::Unselected
                    },
                    closable: card.closable,
                }
            })
            .collect();
        self.navigation_tree.set_section_starts(vec![
            TreeRowKey(STARRED_NAVIGATION_KEY),
            TreeRowKey(TRASH_NAVIGATION_KEY),
        ]);
        self.navigation_tree.set_input(TreeListInput {
            rows: model.navigation_rows.clone(),
            editor: model.navigation_editor.clone(),
            scroll_offset_px: 0.0,
        });
        self.card_list.set_input(VirtualCardListInput {
            cards,
            scroll_offset_px: model.card_scroll_offset_px,
        });
        self.search_box.sync_text(&model.search_query);
        self.default_new_document_kind =
            if model.new_note_control == NewNoteControlState::EnabledForFiles {
                DocumentKind::Text
            } else {
                DocumentKind::Markdown
            };
        self.new_note_button.set_input(SplitButtonInput {
            label: "新建".to_owned(),
            enabled: model.new_note_control.is_enabled(),
        });
        self.new_note_button.set_menu_open(model.show_new_document_menu);
        self.settings_overlay.set_input(model.settings_overlay.clone());
        self.settings_overlay_open = model.show_settings_overlay;
        self.new_workspace_dialog_input = model.new_workspace_dialog.clone().unwrap_or_default();
        self.new_workspace_dialog_open = model.new_workspace_dialog.is_some();
        self.encrypted_note_dialog_input =
            model.encrypted_note_dialog.clone().unwrap_or_else(EncryptedNoteDialogInput::create);
        self.encrypted_note_dialog_open = model.encrypted_note_dialog.is_some();
        self.encrypted_note_unlock_input = model
            .encrypted_note_unlock
            .clone()
            .unwrap_or_else(|| EncryptedNoteUnlockInput::new(String::new()));
        self.encrypted_note_unlock_open = model.encrypted_note_unlock.is_some();
        self.confirmation_action =
            model.confirmation.as_ref().map(|input| input.confirm_action.clone());
        self.new_document_menu_open = model.show_new_document_menu;
        self.save_conflict_actions =
            model.save_conflict.as_ref().map(|conflict| conflict.actions.clone());
        self.editor_pane.set_input(model.editor_chrome.clone());
        self.editor_note_id = model.editor_note_id;
        self.editor_location_actions.clone_from(&model.editor_location_actions);
        self.editor_tag_actions.clone_from(&model.editor_tag_actions);
        self.editor_command_actions.clone_from(&model.editor_command_actions);
        self.card_empty_state.set_input(model.card_empty_state.clone());
        self.card_empty_state_visible = model.cards.is_empty();
        self.editor_empty_state.set_input(StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "请选择笔记".to_owned(),
            description: "编辑器将在此处显示。".to_owned(),
            icon: Some("file-text".to_owned()),
            action_label: None,
            action_id: None,
        });
    }

    pub fn settings_button_id(&self) -> WidgetId {
        SETTINGS_BUTTON_ID
    }

    fn card_key_for(&mut self, identity: DocumentIdentity) -> CardKey {
        if let Some(key) = self.card_keys.get(&identity) {
            return *key;
        }
        let key = CardKey(self.next_card_key);
        self.next_card_key = self.next_card_key.wrapping_add(1).max(1);
        self.card_keys.insert(identity, key);
        key
    }
}
