//! 将外壳控件动作转换为 Notora 产品动作。

use crate::encrypted_note_dialog::EncryptedNoteDialogAction;
use crate::encrypted_note_unlock::EncryptedNoteUnlockAction;
use notora_core::{DocumentIdentity, DocumentKind};
use ui::WidgetAction;
use ui::core::widget::{ControlAction, TextPayload};
use ui::editor_toolbar::EditorToolbarAction;
use ui::popup_menu::{PopupMenuAction, PopupOutcome};
use ui::sidebar::NewDocumentKind;
use ui::splitter::SplitterAction;
use ui::tree_list::{TreeListAction, TreeRowKey};
use ui::virtual_card_list::VirtualCardListAction;

use crate::action::{MetadataMutation, NotoraAction, TrashOperation};
use crate::new_workspace_dialog::NewWorkspaceDialogAction;
use crate::settings_overlay::SettingsOverlayAction;
use crate::{FocusTarget, Pane};

use super::model::{TAG_ROOT_NAVIGATION_KEY, WORKSPACE_NAVIGATION_KEY};
use super::{
    GLOBAL_SEARCH_BOX_ID, NEW_NOTE_BUTTON_ID, NEW_NOTE_MENU_BUTTON_ID, NotoraShell,
    SET_WORKSPACE_ROOT_BUTTON_ID, SETTINGS_BUTTON_ID,
};

const CARD_LOAD_MORE_THRESHOLD_LOGICAL: f32 = 160.0;

impl NotoraShell {
    pub fn translate_widget_action(&self, action: &WidgetAction) -> Option<NotoraAction> {
        match action {
            WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::location_picker::LOCATION_PICKER_SELECT_ID,
                value: TextPayload::Plain(row_key),
            }) => self.editor_location_actions.get(row_key).cloned(),
            WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::tag_editor::TAG_EDITOR_SUBMIT_ID,
                value: TextPayload::Plain(display_name),
            }) => self.editor_note_id.map(|note_id| {
                NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTagByName {
                    note_id,
                    display_name: display_name.clone(),
                })
            }),
            WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::tag_editor::TAG_EDITOR_REMOVE_ID,
                value: TextPayload::Plain(chip_key),
            }) => self.editor_tag_actions.get(chip_key).cloned(),
            WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::tag_editor::TAG_EDITOR_SUGGESTION_ID,
                value: TextPayload::Plain(option_key),
            }) => self.editor_tag_actions.get(option_key).cloned(),
            WidgetAction::EditorToolbar(EditorToolbarAction::CommandInvoked(command_key)) => {
                self.editor_command_actions.get(command_key).cloned()
            }
            WidgetAction::InspectorPanel(action) => self
                .inspector_panel_stamp
                .as_ref()?
                .stamp(action.clone())
                .map(NotoraAction::InspectorPanel),
            WidgetAction::Control(ControlAction::TextEdited {
                id: ui::editor_header::EDITOR_HEADER_TITLE_ID,
                value,
            }) => match value {
                TextPayload::Plain(title) => Some(NotoraAction::TitleTextChanged(title.clone())),
                TextPayload::Sensitive(_) => None,
            },
            WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::editor_header::EDITOR_HEADER_TITLE_ID,
                value,
            }) => match value {
                TextPayload::Plain(title) => {
                    Some(NotoraAction::TitleCommitRequested(title.clone()))
                }
                TextPayload::Sensitive(_) => None,
            },
            WidgetAction::Control(ControlAction::Activated {
                id: ui::editor_header::EDITOR_HEADER_CANCEL_TITLE_ID,
            }) => Some(NotoraAction::FocusRequested(FocusTarget::Editor)),
            WidgetAction::Control(ControlAction::Activated {
                id: ui::tag_editor::TAG_EDITOR_CANCEL_ID,
            }) => Some(NotoraAction::FocusRequested(FocusTarget::Editor)),
            WidgetAction::Control(ControlAction::Activated {
                id: ui::editor_header::EDITOR_HEADER_STAR_ID,
            }) => self.editor_note_id.map(|note_id| {
                NotoraAction::MetadataMutationRequested(MetadataMutation::ToggleStar { note_id })
            }),
            WidgetAction::Control(ControlAction::Activated {
                id: ui::editor_header::EDITOR_HEADER_DELETE_ID,
            }) => self.editor_note_id.map(|note_id| {
                NotoraAction::TrashOperationRequested(TrashOperation::MoveToTrash { note_id })
            }),
            WidgetAction::TreeList(TreeListAction::Selected(key)) => {
                self.navigation_actions.get(key).cloned()
            }
            WidgetAction::TreeList(TreeListAction::ExpansionToggled(key)) => self
                .navigation_expansion_paths
                .get(key)
                .cloned()
                .map(NotoraAction::NavigationExpansionToggled)
                .or_else(|| {
                    (*key == TreeRowKey(WORKSPACE_NAVIGATION_KEY))
                        .then_some(NotoraAction::WorkspaceRootExpansionToggled)
                })
                .or_else(|| {
                    (*key == TreeRowKey(TAG_ROOT_NAVIGATION_KEY))
                        .then_some(NotoraAction::TagRootExpansionToggled)
                }),
            WidgetAction::TreeList(TreeListAction::TrailingActionActivated {
                row_key,
                action_key,
            }) => self.navigation_trailing_actions.get(&(*row_key, *action_key)).cloned(),
            WidgetAction::TreeList(TreeListAction::EditorTextChanged { value, .. }) => {
                Some(NotoraAction::DirectoryCreationTextChanged(value.clone()))
            }
            WidgetAction::TreeList(TreeListAction::EditorCommitRequested { .. }) => {
                Some(NotoraAction::DirectoryCreationCommitRequested)
            }
            WidgetAction::TreeList(TreeListAction::EditorCancelled { .. }) => {
                Some(NotoraAction::DirectoryCreationCancelled)
            }
            WidgetAction::VirtualCardList(VirtualCardListAction::Selected(key)) => {
                self.card_identities.get(key).copied().map(NotoraAction::CardSelected)
            }
            WidgetAction::VirtualCardList(VirtualCardListAction::Activated(key)) => {
                self.card_identities.get(key).copied().map(NotoraAction::CardActivated)
            }
            WidgetAction::VirtualCardList(VirtualCardListAction::CloseRequested(key)) => {
                self.card_identities.get(key).and_then(|identity| match identity {
                    DocumentIdentity::ExternalFile(external_file_id) => {
                        Some(NotoraAction::ExternalFileCloseRequested(*external_file_id))
                    }
                    DocumentIdentity::Note(_) => None,
                })
            }
            WidgetAction::VirtualCardList(VirtualCardListAction::ScrollOffsetChanged(
                offset_px,
            )) => {
                let layout = self.card_list.layout();
                let remaining_px = layout.content_height_px - (*offset_px + layout.viewport_rect.h);
                Some(NotoraAction::CardListScrolled {
                    offset_px: *offset_px,
                    near_end: remaining_px <= CARD_LOAD_MORE_THRESHOLD_LOGICAL,
                })
            }
            WidgetAction::Control(ui::core::widget::ControlAction::Activated { id })
                if *id == SETTINGS_BUTTON_ID =>
            {
                Some(NotoraAction::OpenSettings)
            }
            WidgetAction::Control(ControlAction::Activated { id })
                if *id == SET_WORKSPACE_ROOT_BUTTON_ID =>
            {
                Some(NotoraAction::WorkspaceRootSelectionRequested)
            }
            WidgetAction::Control(ControlAction::Activated { id }) if *id == NEW_NOTE_BUTTON_ID => {
                Some(NotoraAction::CreateRequested(self.default_new_document_kind))
            }
            WidgetAction::Control(ControlAction::Activated { id })
                if *id == NEW_NOTE_MENU_BUTTON_ID =>
            {
                Some(NotoraAction::OpenNewDocumentMenu)
            }
            WidgetAction::Control(ControlAction::TextEdited {
                id: GLOBAL_SEARCH_BOX_ID,
                value: TextPayload::Plain(query),
            }) => Some(NotoraAction::SearchTextChanged(query.clone())),
            WidgetAction::Control(ControlAction::FocusRequested { id: GLOBAL_SEARCH_BOX_ID }) => {
                Some(NotoraAction::FocusRequested(FocusTarget::NavigationSearch))
            }
            WidgetAction::Control(ControlAction::FocusRequested {
                id: ui::editor_header::EDITOR_HEADER_TITLE_ID,
            }) => Some(NotoraAction::FocusRequested(FocusTarget::EditorTitle)),
            WidgetAction::Control(ControlAction::FocusRequested {
                id: ui::tag_editor::TAG_EDITOR_INPUT_ID,
            }) => Some(NotoraAction::FocusRequested(FocusTarget::EditorTag)),
            WidgetAction::Control(ControlAction::TextCommitted {
                id: GLOBAL_SEARCH_BOX_ID,
                ..
            }) => Some(NotoraAction::FocusRequested(FocusTarget::CardList)),
            _ => None,
        }
    }
}

pub(super) fn splitter_action_to_notora_action(
    action: &WidgetAction,
    pane: Pane,
) -> Option<NotoraAction> {
    match action {
        WidgetAction::Splitter(
            SplitterAction::LogicalPositionChanged(logical_width)
            | SplitterAction::DragEnded(logical_width),
        ) => Some(NotoraAction::SplitterDragged { pane, logical_width: *logical_width }),
        _ => None,
    }
}

pub(super) fn settings_overlay_action_to_notora_action(
    action: SettingsOverlayAction,
) -> NotoraAction {
    match action {
        SettingsOverlayAction::Update(update) => {
            NotoraAction::ProductSettingsUpdateRequested(update)
        }
        SettingsOverlayAction::RetryPersistence => NotoraAction::RetryProductSettingsPersistence,
        SettingsOverlayAction::ViewChanged => NotoraAction::SettingsViewChanged,
        SettingsOverlayAction::Dismiss => NotoraAction::OverlayDismissed,
    }
}

pub(super) fn new_workspace_dialog_action_to_notora_action(
    action: NewWorkspaceDialogAction,
) -> NotoraAction {
    match action {
        NewWorkspaceDialogAction::NameChanged(name) => {
            NotoraAction::WorkspaceCreationNameChanged(name)
        }
        NewWorkspaceDialogAction::ChooseLocation => {
            NotoraAction::WorkspaceCreationLocationRequested
        }
        NewWorkspaceDialogAction::Create => NotoraAction::WorkspaceCreationCommitRequested,
        NewWorkspaceDialogAction::Cancel => NotoraAction::OverlayDismissed,
    }
}

pub(super) fn encrypted_note_dialog_action_to_notora_action(
    action: EncryptedNoteDialogAction,
) -> NotoraAction {
    match action {
        EncryptedNoteDialogAction::PasswordChanged(password) => {
            NotoraAction::EncryptedNotePasswordChanged(password)
        }
        EncryptedNoteDialogAction::ConfirmationChanged(confirmation) => {
            NotoraAction::EncryptedNoteConfirmationChanged(confirmation)
        }
        EncryptedNoteDialogAction::Submit => NotoraAction::EncryptedNoteDialogSubmitRequested,
        EncryptedNoteDialogAction::Cancel => NotoraAction::OverlayDismissed,
    }
}

pub(super) fn encrypted_note_unlock_action_to_notora_action(
    action: EncryptedNoteUnlockAction,
) -> NotoraAction {
    match action {
        EncryptedNoteUnlockAction::PasswordChanged(password) => {
            NotoraAction::EncryptedNotePasswordChanged(password)
        }
        EncryptedNoteUnlockAction::Submit => NotoraAction::EncryptedNoteDialogSubmitRequested,
    }
}

pub(super) fn new_document_menu_action(action: &WidgetAction) -> Option<NotoraAction> {
    let WidgetAction::Popup(outcome) = action else {
        return None;
    };
    match outcome {
        PopupOutcome::Selected(PopupMenuAction::NewDocument(kind)) => {
            Some(NotoraAction::CreateRequested(match kind {
                NewDocumentKind::Text => DocumentKind::Text,
                NewDocumentKind::Mindmap => DocumentKind::Mindmap,
                NewDocumentKind::Markdown => DocumentKind::Markdown,
                NewDocumentKind::EncryptedMarkdown => {
                    return Some(NotoraAction::BeginEncryptedNoteCreation);
                }
            }))
        }
        PopupOutcome::Dismiss => Some(NotoraAction::OverlayDismissed),
        PopupOutcome::Selected(_) => None,
    }
}
