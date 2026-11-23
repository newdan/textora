//! 从产品状态构建整帧输入，包含导航树和弹层。

use std::collections::HashMap;

use crate::encrypted_note_dialog::EncryptedNoteDialogInput;
use crate::encrypted_note_unlock::EncryptedNoteUnlockInput;
use appkit_shell::mindmap_style_panel::MindmapStylePanelInput;
use notora_core::{DocumentIdentity, NavigationScope, NoteId};
use ui::status_state::StatusStateInput;
use ui::tree_list::{
    TreeRowActionInput, TreeRowActionKey, TreeRowEditorInput, TreeRowExpansion, TreeRowInput,
    TreeRowKey, TreeRowSelection,
};

use crate::action::{ConflictResolution, NotoraAction, TrashOperation};
use crate::editor_pane::EditorPaneInput;
use crate::new_workspace_dialog::NewWorkspaceDialogInput;
use crate::settings::ProductSettings;
use crate::settings_overlay::SettingsOverlayInput;
use crate::state::{
    DirectoryCreationState, EncryptedConflictCopyState, EncryptedNoteCreationState,
    EncryptedNoteUnlockState, WorkspaceCreationState,
};
use crate::{NotoraState, OverlayState, WorkspaceRootState};

use super::card_model::{
    NewNoteControlState, NoteToolbarButtonInput, RenderCard, card_empty_state_input,
    card_list_title, new_note_control_state, note_toolbar_buttons, render_cards,
};
use super::editor_model::{
    EditorPaneState, editor_action_maps, editor_command_actions, editor_pane_input,
};

pub(super) const WORKSPACE_NAVIGATION_KEY: u64 = 1;
pub(super) const STARRED_NAVIGATION_KEY: u64 = 2;
pub(super) const TRASH_NAVIGATION_KEY: u64 = 3;
const EXTERNAL_FILES_NAVIGATION_KEY: u64 = 4;
pub(super) const TAG_ROOT_NAVIGATION_KEY: u64 = 5;
const DIRECTORY_NAVIGATION_KEY_START: u64 = 100;
const TAG_NAVIGATION_KEY_START: u64 = 10_000;
pub(super) const NEW_WORKSPACE_ACTION_KEY: TreeRowActionKey = TreeRowActionKey(1);
pub(super) const OPEN_WORKSPACE_ACTION_KEY: TreeRowActionKey = TreeRowActionKey(2);
pub(super) const NEW_DIRECTORY_ACTION_KEY: TreeRowActionKey = TreeRowActionKey(3);
pub(super) const DIRECTORY_EDITOR_KEY: TreeRowKey = TreeRowKey(u64::MAX - 1);

/// 静态产品壳所需的纯展示输入。领域状态在此映射后不再传入 widget。
#[derive(Clone, Debug, Default)]
pub struct NotoraRenderModel {
    pub search_query: String,
    pub navigation_rows: Vec<TreeRowInput>,
    pub navigation_actions: HashMap<TreeRowKey, NotoraAction>,
    pub navigation_trailing_actions: HashMap<(TreeRowKey, TreeRowActionKey), NotoraAction>,
    pub navigation_expansion_paths: HashMap<TreeRowKey, std::path::PathBuf>,
    pub navigation_editor: Option<TreeRowEditorInput>,
    pub cards: Vec<RenderCard>,
    pub selected_card: Option<DocumentIdentity>,
    pub card_scroll_offset_px: f32,
    pub card_list_title: String,
    pub card_empty_state: StatusStateInput,
    pub show_settings_overlay: bool,
    pub settings_overlay: SettingsOverlayInput,
    pub new_workspace_dialog: Option<NewWorkspaceDialogInput>,
    pub encrypted_note_dialog: Option<EncryptedNoteDialogInput>,
    pub encrypted_note_unlock: Option<EncryptedNoteUnlockInput>,
    pub confirmation: Option<ConfirmationOverlayInput>,
    pub show_new_document_menu: bool,
    pub new_note_control: NewNoteControlState,
    pub note_toolbar: Vec<NoteToolbarButtonInput>,
    pub save_conflict: Option<SaveConflictOverlayInput>,
    pub editor_chrome: EditorPaneInput,
    pub editor_note_id: Option<NoteId>,
    pub editor_location_actions: HashMap<String, NotoraAction>,
    pub editor_tag_actions: HashMap<String, NotoraAction>,
    pub editor_command_actions: HashMap<String, NotoraAction>,
    pub mindmap_style_panel: Option<MindmapStylePanelInput>,
    pub editor_pane: EditorPaneState,
}

/// 保存竞态发生后展示的四路显式决策；按钮只保存产品 action，不保存 runtime tab。
#[derive(Clone, Debug, PartialEq)]
pub struct SaveConflictOverlayInput {
    pub identity: DocumentIdentity,
    pub content_revision: u64,
    pub actions: [NotoraAction; 4],
}

/// 产品层已决定的确认弹层纯展示输入；UI 不保存 TagId 或 Trash entry。
#[derive(Clone, Debug, PartialEq)]
pub struct ConfirmationOverlayInput {
    pub title: String,
    pub description: String,
    pub confirm_label: String,
    pub confirm_action: NotoraAction,
}

impl NotoraRenderModel {
    pub fn from_state(state: &NotoraState) -> Self {
        Self::from_state_and_settings(state, &ProductSettings::default())
    }

    pub fn from_state_and_settings(
        state: &NotoraState,
        product_settings: &ProductSettings,
    ) -> Self {
        let selected_scope = &state.library.navigation_scope;
        let mut navigation_rows = Vec::new();
        let mut navigation_actions = HashMap::new();
        let mut navigation_trailing_actions = HashMap::new();
        let mut navigation_expansion_paths = HashMap::new();
        let mut directory_row_keys = HashMap::new();
        let workspace_has_directories = state.workspace_root == WorkspaceRootState::Active
            && !state.library.navigation_tree.directories.is_empty();
        push_workspace_navigation_row(
            &mut navigation_rows,
            &mut navigation_actions,
            selected_scope,
            workspace_has_directories,
            state.library.navigation_tree.workspace_root_expanded,
            state.workspace_root_path.as_deref(),
        );
        let workspace_row_key = TreeRowKey(WORKSPACE_NAVIGATION_KEY);
        directory_row_keys.insert(std::path::PathBuf::new(), workspace_row_key);
        let workspace_row = navigation_rows
            .last_mut()
            .expect("workspace row was inserted immediately before its actions");
        workspace_row.trailing_actions.extend([
            new_workspace_action(),
            open_workspace_action(),
            new_directory_action(state.workspace_root == WorkspaceRootState::Active),
        ]);
        navigation_trailing_actions.insert(
            (workspace_row_key, NEW_WORKSPACE_ACTION_KEY),
            NotoraAction::OpenWorkspaceCreationRequested,
        );
        navigation_trailing_actions.insert(
            (workspace_row_key, OPEN_WORKSPACE_ACTION_KEY),
            NotoraAction::WorkspaceRootSelectionRequested,
        );
        if state.workspace_root == WorkspaceRootState::Active {
            navigation_trailing_actions.insert(
                (workspace_row_key, NEW_DIRECTORY_ACTION_KEY),
                NotoraAction::BeginDirectoryCreation {
                    parent_relative_path: std::path::PathBuf::new(),
                },
            );
        }
        for (index, directory) in state.library.navigation_tree.directories.iter().enumerate() {
            if state.workspace_root != WorkspaceRootState::Active
                || !state.library.navigation_tree.workspace_root_expanded
            {
                continue;
            }
            if !directory_is_visible(directory, &state.library.navigation_tree.expanded_directories)
            {
                continue;
            }
            let key = DIRECTORY_NAVIGATION_KEY_START + index as u64;
            let label = directory
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| directory.to_string_lossy().to_string());
            let depth = directory.components().count();
            let row_key = TreeRowKey(key);
            directory_row_keys.insert(directory.clone(), row_key);
            navigation_expansion_paths.insert(row_key, directory.clone());
            push_navigation_directory_row(
                &mut navigation_rows,
                &mut navigation_actions,
                key,
                label,
                "folder",
                depth,
                None,
                NavigationScope::Directory { relative_path: directory.clone() },
                selected_scope,
                directory_has_children(directory, &state.library.navigation_tree.directories),
                state.library.navigation_tree.expanded_directories.contains(directory),
            );
            navigation_rows
                .last_mut()
                .expect("directory row was inserted immediately before its actions")
                .trailing_actions
                .push(new_directory_action(true));
            navigation_trailing_actions.insert(
                (row_key, NEW_DIRECTORY_ACTION_KEY),
                NotoraAction::BeginDirectoryCreation { parent_relative_path: directory.clone() },
            );
        }
        let navigation_editor = match &state.directory_creation {
            DirectoryCreationState::Editing { parent_relative_path, draft_name } => {
                directory_row_keys.get(parent_relative_path).map(|parent_key| TreeRowEditorInput {
                    key: DIRECTORY_EDITOR_KEY,
                    parent_key: *parent_key,
                    depth: parent_relative_path.components().count() + 1,
                    value: draft_name.clone(),
                    placeholder: "新目录名称".to_owned(),
                })
            }
            DirectoryCreationState::Inactive | DirectoryCreationState::Submitting { .. } => None,
        };
        push_navigation_row(
            &mut navigation_rows,
            &mut navigation_actions,
            STARRED_NAVIGATION_KEY,
            "星标".to_owned(),
            "star",
            0,
            None,
            NavigationScope::Starred,
            selected_scope,
        );
        let visible_tags = state
            .library
            .navigation_tree
            .tags
            .iter()
            .filter(|tag| tag.active_note_count > 0)
            .collect::<Vec<_>>();
        push_tag_root_navigation_row(
            &mut navigation_rows,
            &mut navigation_actions,
            !visible_tags.is_empty(),
            state.library.navigation_tree.tag_root_expanded,
        );
        if state.library.navigation_tree.tag_root_expanded {
            for (index, tag) in visible_tags.into_iter().enumerate() {
                let key = TAG_NAVIGATION_KEY_START + index as u64;
                push_navigation_row(
                    &mut navigation_rows,
                    &mut navigation_actions,
                    key,
                    tag.display_name.clone(),
                    "tag",
                    1,
                    u32::try_from(tag.active_note_count).ok(),
                    NavigationScope::Tag { tag_id: tag.tag_id },
                    selected_scope,
                );
            }
        }
        push_navigation_row(
            &mut navigation_rows,
            &mut navigation_actions,
            TRASH_NAVIGATION_KEY,
            "回收站".to_owned(),
            "trash-2",
            0,
            None,
            NavigationScope::Trash,
            selected_scope,
        );
        push_navigation_row(
            &mut navigation_rows,
            &mut navigation_actions,
            EXTERNAL_FILES_NAVIGATION_KEY,
            "文件".to_owned(),
            "file",
            0,
            None,
            NavigationScope::ExternalFiles,
            selected_scope,
        );
        let search_query = state.library.search_text.clone();
        let cards = render_cards(state);
        let selected_note_id = state.library.selected_card.and_then(|identity| match identity {
            DocumentIdentity::Note(note_id)
                if cards.iter().any(|card| card.identity == identity) =>
            {
                Some(note_id)
            }
            DocumentIdentity::ExternalFile(_) => None,
            DocumentIdentity::Note(_) => None,
        });
        let editor_note_id = state.library.selected_card.and_then(|identity| match identity {
            DocumentIdentity::Note(note_id) => Some(note_id),
            DocumentIdentity::ExternalFile(_) => None,
        });
        let editor_chrome = editor_pane_input(state, &cards);
        let external_files_present =
            *selected_scope == NavigationScope::ExternalFiles && !cards.is_empty();
        let (editor_location_actions, editor_tag_actions) = editor_action_maps(state);
        let mut editor_command_actions = editor_command_actions();
        if let Some(note_id) = editor_note_id
            && state.library.navigation_scope != NavigationScope::Trash
            && state.library.navigation_scope != NavigationScope::ExternalFiles
        {
            editor_command_actions.insert(
                "delete".to_owned(),
                NotoraAction::TrashOperationRequested(TrashOperation::MoveToTrash { note_id }),
            );
        }
        Self {
            search_query,
            navigation_rows,
            navigation_actions,
            navigation_trailing_actions,
            navigation_expansion_paths,
            navigation_editor,
            cards,
            selected_card: state.library.selected_card,
            card_scroll_offset_px: state.library.card_scroll_offset_px,
            card_list_title: card_list_title(selected_scope).to_owned(),
            card_empty_state: card_empty_state_input(state),
            show_settings_overlay: state.layout.overlay == OverlayState::Settings,
            settings_overlay: SettingsOverlayInput::from_product_settings(product_settings),
            new_workspace_dialog: match &state.workspace_creation {
                WorkspaceCreationState::Editing { name, parent_directory } => {
                    Some(NewWorkspaceDialogInput {
                        name: name.clone(),
                        parent_directory: parent_directory.clone(),
                        error_message: state.library.last_command_error.clone(),
                    })
                }
                WorkspaceCreationState::Inactive => None,
            },
            encrypted_note_dialog: encrypted_note_dialog_input(state),
            encrypted_note_unlock: encrypted_note_unlock_input(&state.encrypted_note_unlock),
            confirmation: confirmation_overlay_input(state.layout.overlay),
            show_new_document_menu: state.layout.overlay == OverlayState::NewDocumentMenu,
            new_note_control: new_note_control_state(selected_scope, state.workspace_root),
            note_toolbar: note_toolbar_buttons(
                selected_scope,
                selected_note_id,
                external_files_present,
            ),
            save_conflict: state.library.save_conflict.map(|conflict| SaveConflictOverlayInput {
                identity: conflict.identity,
                content_revision: conflict.content_revision,
                actions: [
                    NotoraAction::SaveConflictResolutionRequested(
                        ConflictResolution::ReloadFromDisk,
                    ),
                    NotoraAction::SaveConflictResolutionRequested(ConflictResolution::SaveCopy),
                    NotoraAction::SaveConflictResolutionRequested(ConflictResolution::RetrySave),
                    NotoraAction::SaveConflictResolutionRequested(ConflictResolution::Cancel),
                ],
            }),
            editor_chrome,
            editor_note_id,
            editor_location_actions,
            editor_tag_actions,
            editor_command_actions,
            mindmap_style_panel: None,
            editor_pane: EditorPaneState::Empty,
        }
    }
}

fn new_directory_action(enabled: bool) -> TreeRowActionInput {
    TreeRowActionInput {
        key: NEW_DIRECTORY_ACTION_KEY,
        icon: "folder-plus".to_owned(),
        tooltip: "新建目录".to_owned(),
        accessibility_label: "在此目录中新建目录".to_owned(),
        enabled,
    }
}

fn new_workspace_action() -> TreeRowActionInput {
    TreeRowActionInput {
        key: NEW_WORKSPACE_ACTION_KEY,
        icon: "workspace-plus".to_owned(),
        tooltip: "新建工作区".to_owned(),
        accessibility_label: "新建工作区".to_owned(),
        enabled: true,
    }
}

fn open_workspace_action() -> TreeRowActionInput {
    TreeRowActionInput {
        key: OPEN_WORKSPACE_ACTION_KEY,
        icon: "folder-open".to_owned(),
        tooltip: "打开工作区".to_owned(),
        accessibility_label: "打开工作区".to_owned(),
        enabled: true,
    }
}

fn encrypted_note_dialog_input(state: &NotoraState) -> Option<EncryptedNoteDialogInput> {
    encrypted_conflict_copy_dialog_input(&state.encrypted_conflict_copy)
        .or_else(|| encrypted_note_creation_dialog_input(&state.encrypted_note_creation))
}

fn encrypted_conflict_copy_dialog_input(
    state: &EncryptedConflictCopyState,
) -> Option<EncryptedNoteDialogInput> {
    let (target_path, password, confirmation, submitting, error_message, failure_generation) =
        match state {
            EncryptedConflictCopyState::Inactive => return None,
            EncryptedConflictCopyState::Editing {
                target_path,
                password,
                confirmation,
                error_message,
                failure_generation,
                ..
            } => (
                target_path,
                password.clone(),
                confirmation.clone(),
                false,
                error_message.clone(),
                *failure_generation,
            ),
            EncryptedConflictCopyState::Submitting { target_path, .. } => {
                (target_path, empty_sensitive_text(), empty_sensitive_text(), true, None, 0)
            }
        };
    Some(EncryptedNoteDialogInput {
        mode: crate::encrypted_note_dialog::EncryptedNoteDialogMode::ConflictCopy {
            file_name: target_path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("加密冲突副本.md")
                .to_owned(),
        },
        password,
        confirmation: Some(confirmation),
        submitting,
        error_message,
        failure_generation,
    })
}

fn encrypted_note_creation_dialog_input(
    state: &EncryptedNoteCreationState,
) -> Option<EncryptedNoteDialogInput> {
    match state {
        EncryptedNoteCreationState::Inactive => None,
        EncryptedNoteCreationState::Editing {
            password,
            confirmation,
            error_message,
            failure_generation,
            ..
        } => Some(EncryptedNoteDialogInput {
            mode: crate::encrypted_note_dialog::EncryptedNoteDialogMode::Create,
            password: password.clone(),
            confirmation: Some(confirmation.clone()),
            submitting: false,
            error_message: error_message.clone(),
            failure_generation: *failure_generation,
        }),
        EncryptedNoteCreationState::Submitting { .. } => Some(EncryptedNoteDialogInput {
            mode: crate::encrypted_note_dialog::EncryptedNoteDialogMode::Create,
            password: empty_sensitive_text(),
            confirmation: Some(empty_sensitive_text()),
            submitting: true,
            error_message: None,
            failure_generation: 0,
        }),
    }
}

fn encrypted_note_unlock_input(
    state: &EncryptedNoteUnlockState,
) -> Option<EncryptedNoteUnlockInput> {
    match state {
        EncryptedNoteUnlockState::Inactive => None,
        EncryptedNoteUnlockState::Editing {
            title,
            password,
            error_message,
            failure_generation,
            ..
        } => Some(EncryptedNoteUnlockInput {
            title: title.clone(),
            password: password.clone(),
            submitting: false,
            error_message: error_message.clone(),
            failure_generation: *failure_generation,
        }),
        EncryptedNoteUnlockState::Submitting { title, .. } => Some(EncryptedNoteUnlockInput {
            title: title.clone(),
            password: empty_sensitive_text(),
            submitting: true,
            error_message: None,
            failure_generation: 0,
        }),
    }
}

fn empty_sensitive_text() -> ui::core::widget::SensitiveText {
    ui::core::widget::SensitiveText::new(String::new())
}

pub(super) fn confirmation_overlay_input(
    overlay: OverlayState,
) -> Option<ConfirmationOverlayInput> {
    match overlay {
        OverlayState::TrashPermanentDeletionConfirmation { operation } => {
            let (title, description) = match operation {
                crate::action::TrashOperation::PermanentlyDelete { .. } => {
                    ("永久删除笔记？", "所选回收站项目将被删除，且无法撤销。")
                }
                crate::action::TrashOperation::Empty => {
                    ("清空回收站？", "当前回收站中的所有项目都将被删除，且无法撤销。")
                }
                crate::action::TrashOperation::MoveToTrash { .. }
                | crate::action::TrashOperation::Restore { .. }
                | crate::action::TrashOperation::RestoreWithRenamedPath { .. } => return None,
            };
            Some(ConfirmationOverlayInput {
                title: title.to_owned(),
                description: description.to_owned(),
                confirm_label: "删除".to_owned(),
                confirm_action: NotoraAction::TrashPermanentDeletionConfirmed,
            })
        }
        OverlayState::TrashRestoreConflictConfirmation { .. } => Some(ConfirmationOverlayInput {
            title: "文件已存在".to_owned(),
            description: "请使用其他名称恢复此笔记，现有文件不会被更改。".to_owned(),
            confirm_label: "恢复副本".to_owned(),
            confirm_action: NotoraAction::TrashRestoreWithRenamedPathConfirmed,
        }),
        OverlayState::None
        | OverlayState::Settings
        | OverlayState::NewDocumentMenu
        | OverlayState::EncryptedNoteDialog
        | OverlayState::NewWorkspace
        | OverlayState::SaveConflict => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_navigation_row(
    rows: &mut Vec<TreeRowInput>,
    actions: &mut HashMap<TreeRowKey, NotoraAction>,
    key: u64,
    label: String,
    icon: &str,
    depth: usize,
    badge: Option<u32>,
    scope: NavigationScope,
    selected_scope: &NavigationScope,
) {
    let row_key = TreeRowKey(key);
    actions.insert(row_key, NotoraAction::NavigationSelected(scope.clone()));
    rows.push(TreeRowInput {
        key: row_key,
        label,
        icon: Some(icon.to_owned()),
        depth,
        expansion: TreeRowExpansion::Leaf,
        selection: if scope == *selected_scope {
            TreeRowSelection::Selected
        } else {
            TreeRowSelection::Unselected
        },
        badge,
        tooltip: None,
        trailing_actions: Vec::new(),
    });
}

fn push_workspace_navigation_row(
    rows: &mut Vec<TreeRowInput>,
    actions: &mut HashMap<TreeRowKey, NotoraAction>,
    selected_scope: &NavigationScope,
    has_directories: bool,
    expanded: bool,
    workspace_root: Option<&std::path::Path>,
) {
    let row_key = TreeRowKey(WORKSPACE_NAVIGATION_KEY);
    actions.insert(row_key, NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot));
    rows.push(TreeRowInput {
        key: row_key,
        label: "工作区".to_owned(),
        icon: Some("folder-open".to_owned()),
        depth: 0,
        expansion: match (has_directories, expanded) {
            (false, _) => TreeRowExpansion::Leaf,
            (true, true) => TreeRowExpansion::Expanded,
            (true, false) => TreeRowExpansion::Collapsed,
        },
        selection: if *selected_scope == NavigationScope::WorkspaceRoot {
            TreeRowSelection::Selected
        } else {
            TreeRowSelection::Unselected
        },
        badge: None,
        tooltip: workspace_root.map(|root| root.display().to_string()),
        trailing_actions: Vec::new(),
    });
}

fn push_tag_root_navigation_row(
    rows: &mut Vec<TreeRowInput>,
    actions: &mut HashMap<TreeRowKey, NotoraAction>,
    has_tags: bool,
    expanded: bool,
) {
    let row_key = TreeRowKey(TAG_ROOT_NAVIGATION_KEY);
    actions.insert(row_key, NotoraAction::TagRootExpansionToggled);
    rows.push(TreeRowInput {
        key: row_key,
        label: "标签".to_owned(),
        icon: Some("tag".to_owned()),
        depth: 0,
        expansion: match (has_tags, expanded) {
            (false, _) => TreeRowExpansion::Leaf,
            (true, true) => TreeRowExpansion::Expanded,
            (true, false) => TreeRowExpansion::Collapsed,
        },
        selection: TreeRowSelection::Unselected,
        badge: None,
        tooltip: None,
        trailing_actions: Vec::new(),
    });
}

#[allow(clippy::too_many_arguments)]
fn push_navigation_directory_row(
    rows: &mut Vec<TreeRowInput>,
    actions: &mut HashMap<TreeRowKey, NotoraAction>,
    key: u64,
    label: String,
    icon: &str,
    depth: usize,
    badge: Option<u32>,
    scope: NavigationScope,
    selected_scope: &NavigationScope,
    has_children: bool,
    expanded: bool,
) {
    let row_key = TreeRowKey(key);
    actions.insert(row_key, NotoraAction::NavigationSelected(scope.clone()));
    rows.push(TreeRowInput {
        key: row_key,
        label,
        icon: Some(icon.to_owned()),
        depth,
        expansion: match (has_children, expanded) {
            (false, _) => TreeRowExpansion::Leaf,
            (true, true) => TreeRowExpansion::Expanded,
            (true, false) => TreeRowExpansion::Collapsed,
        },
        selection: if scope == *selected_scope {
            TreeRowSelection::Selected
        } else {
            TreeRowSelection::Unselected
        },
        badge,
        tooltip: None,
        trailing_actions: Vec::new(),
    });
}

fn directory_is_visible(
    directory: &std::path::Path,
    expanded_directories: &std::collections::BTreeSet<std::path::PathBuf>,
) -> bool {
    directory
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .all(|ancestor| expanded_directories.contains(ancestor))
}

fn directory_has_children(directory: &std::path::Path, directories: &[std::path::PathBuf]) -> bool {
    directories.iter().any(|candidate| candidate.parent().is_some_and(|parent| parent == directory))
}
