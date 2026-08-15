use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use appkit_shell::editor_runtime::{EditorFrame, RenderError};
use notora_core::{DocumentIdentity, DocumentKind, NavigationScope, NoteId};
use ui::canvas_scrollbars::{
    CanvasScrollbarsAction, CanvasScrollbarsInput, CanvasScrollbarsWidget,
};
use ui::core::WidgetAction;
use ui::core::widget::{ControlAction, TextPayload, WidgetId};
use ui::icon::draw_icon;
use ui::mindmap_style_panel::{
    MindmapStylePanelInput, MindmapStylePanelWidget, PANEL_WIDTH_LOGICAL,
};
use ui::popup_menu::{PopupMenuAction, PopupMenuWidget, PopupOutcome};
use ui::sidebar::NewDocumentKind;
use ui::split_button::{SplitButtonInput, SplitButtonWidget};
use ui::splitter::{SplitterAction, SplitterInput, SplitterWidget};
use ui::status_state::{StatusStateInput, StatusStateKind, StatusStateWidget};
use ui::text_box::TextBox;
use ui::tooltip::{TooltipHint, TooltipWidget};
use ui::tree_list::{
    TreeListAction, TreeListInput, TreeListWidget, TreeRowActionInput, TreeRowActionKey,
    TreeRowEditorInput, TreeRowExpansion, TreeRowInput, TreeRowKey, TreeRowSelection,
};
use ui::virtual_card_list::{
    CardInput, CardKey, CardSelection, VirtualCardListAction, VirtualCardListInput,
    VirtualCardListWidget,
};
use ui::{Event, EventCtx, Rect, Widget};

use crate::action::{ConflictResolution, MetadataMutation, NotoraAction, TrashOperation};
use crate::editor_pane::{EditorPaneChrome, EditorPaneInput, EditorPaneMode, EditorPaneRects};
use crate::external_files::ExternalFileSession;
use crate::new_workspace_dialog::{
    NewWorkspaceDialog, NewWorkspaceDialogAction, NewWorkspaceDialogInput,
};
use crate::settings::ProductSettings;
use crate::settings_overlay::{SettingsOverlay, SettingsOverlayAction, SettingsOverlayInput};
use crate::shell::layout::ShellLayout;
use crate::state::{CardPageState, DirectoryCreationState, WorkspaceCreationState};
use crate::{
    FocusTarget, NotoraState, OverlayState, Pane, ResponsiveLayoutMode, WorkspaceRootState,
};

const GLOBAL_SEARCH_BOX_ID: WidgetId = WidgetId(9_000);
const SETTINGS_BUTTON_ID: WidgetId = WidgetId(9_001);
const NEW_NOTE_BUTTON_ID: WidgetId = WidgetId(9_002);
const NEW_NOTE_MENU_BUTTON_ID: WidgetId = WidgetId(9_003);
const SET_WORKSPACE_ROOT_BUTTON_ID: WidgetId = WidgetId(9_004);
const NEW_NOTE_BUTTON_WIDTH_LOGICAL: f32 = 128.0;
const NOTE_TOOL_BUTTON_WIDTH_LOGICAL: f32 = 64.0;
const NOTE_TOOL_BUTTON_HEIGHT_LOGICAL: f32 = 28.0;
const NOTE_TOOL_BUTTON_GAP_LOGICAL: f32 = 6.0;
const CARD_HEADER_TITLE_FONT_SIZE_LOGICAL: f32 = 16.0;
const CARD_HEADER_TITLE_BASELINE_LOGICAL: f32 = 28.0;
const CARD_HEADER_CONTROL_TOP_LOGICAL: f32 = 8.0;
const CARD_HEADER_WRAPPED_CONTROL_TOP_LOGICAL: f32 = 44.0;
const CARD_HEADER_CONTENT_TOP_LOGICAL: f32 = 44.0;
const CARD_HEADER_WRAPPED_CONTENT_TOP_LOGICAL: f32 = 80.0;
const SEARCH_BAR_HEIGHT_LOGICAL: f32 = 32.0;
const SEARCH_ICON_AREA_WIDTH_LOGICAL: f32 = 32.0;
const SHELL_PADDING_LOGICAL: f32 = 12.0;
const SIDEBAR_CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
const SIDEBAR_ICON_SIZE_LOGICAL: f32 = 16.0;
const SIDEBAR_LABEL_FONT_SIZE_LOGICAL: f32 = 15.0;
const CARD_LOAD_MORE_THRESHOLD_LOGICAL: f32 = 160.0;
const COMPACT_NAVIGATION_BUTTON_WIDTH_LOGICAL: f32 = 72.0;
const COMPACT_BACK_BUTTON_WIDTH_LOGICAL: f32 = 64.0;
const CONFIRMATION_PANEL_WIDTH_LOGICAL: f32 = 360.0;
const CONFIRMATION_PANEL_HEIGHT_LOGICAL: f32 = 160.0;
const CONFIRMATION_BUTTON_WIDTH_LOGICAL: f32 = 88.0;
const CONFIRMATION_BUTTON_HEIGHT_LOGICAL: f32 = 32.0;
const SAVE_CONFLICT_PANEL_WIDTH_LOGICAL: f32 = 440.0;
const SAVE_CONFLICT_PANEL_HEIGHT_LOGICAL: f32 = 196.0;
const TEXT_CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);
const WORKSPACE_NAVIGATION_KEY: u64 = 1;
const STARRED_NAVIGATION_KEY: u64 = 2;
const TRASH_NAVIGATION_KEY: u64 = 3;
const EXTERNAL_FILES_NAVIGATION_KEY: u64 = 4;
const DIRECTORY_NAVIGATION_KEY_START: u64 = 100;
const TAG_NAVIGATION_KEY_START: u64 = 10_000;
const NEW_WORKSPACE_ACTION_KEY: TreeRowActionKey = TreeRowActionKey(1);
const OPEN_WORKSPACE_ACTION_KEY: TreeRowActionKey = TreeRowActionKey(2);
const NEW_DIRECTORY_ACTION_KEY: TreeRowActionKey = TreeRowActionKey(3);
const DIRECTORY_EDITOR_KEY: TreeRowKey = TreeRowKey(u64::MAX - 1);
const NAVIGATION_TREE_ID: WidgetId = WidgetId(9_005);
const EDITOR_ROOT_DIRECTORY_ROW_KEY: &str = "root";
const EDITOR_TAG_SUGGESTION_KEY_PREFIX: &str = "suggestion:";

/// UI 之前的产品展示卡片；保持领域身份，避免将 app 状态泄漏给 ui crate。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderCard {
    pub identity: DocumentIdentity,
    pub title: String,
    pub excerpt: String,
    pub timestamp: String,
    pub icon: Option<String>,
    pub tag_summary: String,
    pub closable: bool,
}

/// 产品层计算好的工具栏动作；widget 不保存 NoteId、TagId 或导航范围。
#[derive(Clone, Debug, PartialEq)]
pub struct NoteToolbarButtonInput {
    pub label: String,
    pub action: NotoraAction,
}

/// 保存竞态发生后展示的四路显式决策；按钮只保存产品 action，不保存 runtime tab。
#[derive(Clone, Debug, PartialEq)]
pub struct SaveConflictOverlayInput {
    pub identity: DocumentIdentity,
    pub content_revision: u64,
    pub actions: [NotoraAction; 4],
}

/// 中栏标题栏的新建入口状态；隐藏态不保留可点击矩形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewNoteControlState {
    #[default]
    Hidden,
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CardHeaderLayout {
    title_baseline_y: f32,
    control_top_y: f32,
    content_top_y: f32,
}

impl NewNoteControlState {
    fn is_visible(self) -> bool {
        self != Self::Hidden
    }

    fn is_enabled(self) -> bool {
        self == Self::Enabled
    }
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorPaneState {
    #[default]
    Empty,
    Active,
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
        for (index, tag) in state.library.navigation_tree.tags.iter().enumerate() {
            let key = TAG_NAVIGATION_KEY_START + index as u64;
            push_navigation_row(
                &mut navigation_rows,
                &mut navigation_actions,
                key,
                tag.display_name.clone(),
                "tag",
                0,
                u32::try_from(tag.active_note_count).ok(),
                NavigationScope::Tag { tag_id: tag.tag_id },
                selected_scope,
            );
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

fn editor_pane_input(state: &NotoraState, cards: &[RenderCard]) -> EditorPaneInput {
    let Some(identity) = state.library.selected_card else {
        return EditorPaneInput::default();
    };
    let selected_card = cards.iter().find(|card| card.identity == identity);
    let mode = match identity {
        DocumentIdentity::ExternalFile(_) => EditorPaneMode::ExternalFile,
        DocumentIdentity::Note(_) if state.library.navigation_scope == NavigationScope::Trash => {
            EditorPaneMode::TrashNote
        }
        DocumentIdentity::Note(_) => EditorPaneMode::WorkspaceNote,
    };
    let document_key = match identity {
        DocumentIdentity::Note(note_id) => format!("note:{note_id}"),
        DocumentIdentity::ExternalFile(file_id) => format!("external:{file_id}"),
    };
    let title = state
        .library
        .pending_title_commit
        .as_ref()
        .filter(|pending_title| pending_title.identity == identity)
        .map(|pending_title| pending_title.title.clone())
        .or_else(|| selected_card.map(|card| card.title.clone()))
        .unwrap_or_else(|| match mode {
            EditorPaneMode::TrashNote => "回收站笔记".to_owned(),
            EditorPaneMode::WorkspaceNote => "无标题".to_owned(),
            EditorPaneMode::ExternalFile | EditorPaneMode::Empty => "未命名".to_owned(),
        });
    let starred = selected_card.is_some_and(|card| card.tag_summary.starts_with('★'));
    let tags = selected_card
        .map(|card| {
            card.tag_summary
                .split_whitespace()
                .filter_map(|tag| tag.strip_prefix('#'))
                .enumerate()
                .map(|(index, label)| ui::tag_editor::TagChipInput {
                    chip_key: format!("tag-{index}"),
                    label: label.to_owned(),
                    removable: true,
                })
                .collect()
        })
        .unwrap_or_default();
    let location_directories = if mode == EditorPaneMode::WorkspaceNote {
        let mut directories = vec![ui::location_picker::LocationDirectoryInput {
            row_key: EDITOR_ROOT_DIRECTORY_ROW_KEY.to_owned(),
            label: "工作区根目录".to_owned(),
            depth: 0,
            expanded: true,
            has_children: !state.library.navigation_tree.directories.is_empty(),
            enabled: true,
        }];
        directories.extend(state.library.navigation_tree.directories.iter().map(|directory| {
            ui::location_picker::LocationDirectoryInput {
                row_key: directory.to_string_lossy().to_string(),
                label: directory
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| directory.to_string_lossy().to_string()),
                depth: directory.components().count().saturating_sub(1),
                expanded: state.library.navigation_tree.expanded_directories.contains(directory),
                has_children: state
                    .library
                    .navigation_tree
                    .directories
                    .iter()
                    .any(|candidate| candidate.parent().is_some_and(|parent| parent == directory)),
                enabled: true,
            }
        }));
        directories
    } else {
        Vec::new()
    };
    let mut input = EditorPaneInput {
        mode,
        document_key,
        header: ui::editor_header::EditorHeaderInput {
            title,
            title_editable: mode == EditorPaneMode::WorkspaceNote,
            created_at_text: String::new(),
            modified_at_text: selected_card.map(|card| card.timestamp.clone()).unwrap_or_default(),
            save_status_text: match mode {
                EditorPaneMode::ExternalFile => "外部文件".to_owned(),
                EditorPaneMode::TrashNote => "回收站".to_owned(),
                EditorPaneMode::WorkspaceNote => "已保存".to_owned(),
                EditorPaneMode::Empty => String::new(),
            },
            starred,
            star_enabled: mode == EditorPaneMode::WorkspaceNote,
            encryption: match mode {
                EditorPaneMode::ExternalFile => ui::editor_header::EncryptionStatusInput::Hidden,
                EditorPaneMode::Empty
                | EditorPaneMode::WorkspaceNote
                | EditorPaneMode::TrashNote => {
                    ui::editor_header::EncryptionStatusInput::Unencrypted
                }
            },
            delete_visible: mode == EditorPaneMode::WorkspaceNote,
            delete_enabled: mode == EditorPaneMode::WorkspaceNote,
            compact: false,
        },
        location: ui::location_picker::LocationPickerInput {
            workspace_name: if mode == EditorPaneMode::WorkspaceNote {
                "Notora".to_owned()
            } else {
                String::new()
            },
            directories: location_directories,
            ..ui::location_picker::LocationPickerInput::default()
        },
        tags: ui::tag_editor::TagEditorInput {
            chips: tags,
            enabled: mode == EditorPaneMode::WorkspaceNote,
            ..ui::tag_editor::TagEditorInput::default()
        },
        toolbar: editor_toolbar_input(mode),
    };
    if let Some(snapshot) = state.library.active_editor_metadata.as_ref()
        && snapshot.identity == identity
    {
        input.header.created_at_text = format_created_time(snapshot.metadata.created_at);
        input.header.modified_at_text =
            format_modified_time(snapshot.metadata.modified_at, SystemTime::now());
        input.header.encryption = match snapshot.metadata.encryption {
            notora_core::NoteEncryption::Unencrypted => {
                ui::editor_header::EncryptionStatusInput::Unencrypted
            }
            notora_core::NoteEncryption::Encrypted => {
                ui::editor_header::EncryptionStatusInput::Encrypted
            }
        };
        input.tags.chips = snapshot
            .tags
            .iter()
            .map(|tag| ui::tag_editor::TagChipInput {
                chip_key: tag.tag_id.to_string(),
                label: tag.display_name.clone(),
                removable: true,
            })
            .collect();
        input.tags.suggestions = state
            .library
            .navigation_tree
            .tags
            .iter()
            .filter(|candidate| {
                !snapshot.tags.iter().any(|attached| attached.tag_id == candidate.tag_id)
            })
            .map(|candidate| ui::tag_editor::TagSuggestionInput {
                option_key: editor_tag_suggestion_key(candidate.tag_id),
                label: candidate.display_name.clone(),
                enabled: true,
            })
            .collect();
    }
    input
}

fn format_created_time(timestamp: SystemTime) -> String {
    let Some((year, month, day)) = utc_calendar_date(timestamp) else {
        return "创建时间未知".to_owned();
    };
    format!("创建 {year:04}/{month:02}/{day:02} UTC")
}

fn format_modified_time(timestamp: SystemTime, now: SystemTime) -> String {
    let elapsed_seconds =
        now.duration_since(timestamp).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    if elapsed_seconds < 60 {
        return "修改 刚刚".to_owned();
    }
    if elapsed_seconds < 3_600 {
        return format!("修改 {} 分钟前", elapsed_seconds / 60);
    }
    if elapsed_seconds < 86_400 {
        return format!("修改 {} 小时前", elapsed_seconds / 3_600);
    }
    format!("修改 {} 天前", elapsed_seconds / 86_400)
}

fn utc_calendar_date(timestamp: SystemTime) -> Option<(i64, i64, i64)> {
    let elapsed_seconds = timestamp.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days_since_epoch = i64::try_from(elapsed_seconds / 86_400).ok()?;
    let shifted_days = days_since_epoch.checked_add(719_468)?;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_parameter = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_parameter + 2) / 5 + 1;
    let month = month_parameter + if month_parameter < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    Some((year, month, day))
}

fn editor_action_maps(
    state: &NotoraState,
) -> (HashMap<String, NotoraAction>, HashMap<String, NotoraAction>) {
    let Some(DocumentIdentity::Note(note_id)) = state.library.selected_card else {
        return (HashMap::new(), HashMap::new());
    };
    if state.library.navigation_scope == NavigationScope::Trash
        || state.library.navigation_scope == NavigationScope::ExternalFiles
    {
        return (HashMap::new(), HashMap::new());
    }
    let mut location_actions = HashMap::new();
    location_actions.insert(
        EDITOR_ROOT_DIRECTORY_ROW_KEY.to_owned(),
        NotoraAction::MoveRequested { note_id, target_directory: std::path::PathBuf::new() },
    );
    for directory in &state.library.navigation_tree.directories {
        let row_key = directory.to_string_lossy().to_string();
        location_actions.insert(
            row_key,
            NotoraAction::MoveRequested { note_id, target_directory: directory.clone() },
        );
    }
    let mut tag_actions: HashMap<String, NotoraAction> = state
        .library
        .active_editor_metadata
        .as_ref()
        .filter(|snapshot| {
            snapshot.identity == DocumentIdentity::Note(note_id)
                && snapshot.selection_generation == state.library.selected_document_generation
        })
        .map(|snapshot| {
            snapshot
                .tags
                .iter()
                .map(|tag| {
                    (
                        tag.tag_id.to_string(),
                        NotoraAction::MetadataMutationRequested(MetadataMutation::DetachTag {
                            note_id,
                            tag_id: tag.tag_id,
                        }),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    tag_actions.extend(state.library.navigation_tree.tags.iter().map(|tag| {
        (
            editor_tag_suggestion_key(tag.tag_id),
            NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTagByName {
                note_id,
                display_name: tag.display_name.clone(),
            }),
        )
    }));
    (location_actions, tag_actions)
}

fn editor_tag_suggestion_key(tag_id: notora_core::TagId) -> String {
    format!("{EDITOR_TAG_SUGGESTION_KEY_PREFIX}{tag_id}")
}

fn editor_toolbar_input(mode: EditorPaneMode) -> ui::editor_toolbar::EditorToolbarInput {
    match mode {
        EditorPaneMode::WorkspaceNote => markdown_toolbar_input(),
        EditorPaneMode::ExternalFile => history_toolbar_input(),
        EditorPaneMode::Empty | EditorPaneMode::TrashNote => {
            ui::editor_toolbar::EditorToolbarInput::default()
        }
    }
}

pub(crate) fn editor_toolbar_input_for_plugin(
    mode: EditorPaneMode,
    plugin_name: &str,
) -> ui::editor_toolbar::EditorToolbarInput {
    if mode != EditorPaneMode::WorkspaceNote {
        return editor_toolbar_input(mode);
    }
    match plugin_name {
        ui::plugin::PLUGIN_MARKDOWN_EDITOR => markdown_toolbar_input(),
        ui::plugin::PLUGIN_MINDMAP => mindmap_toolbar_input(),
        _ => history_toolbar_input(),
    }
}

pub(crate) fn add_compact_editor_toolbar_commands(
    toolbar: &mut ui::editor_toolbar::EditorToolbarInput,
) {
    let Some(group) = toolbar.groups.first_mut() else {
        return;
    };
    if group.commands.iter().any(|command| command.command_key == "delete") {
        return;
    }
    group.commands.push(ui::editor_toolbar::EditorToolbarCommandInput {
        command_key: "delete".to_owned(),
        label: "移入回收站".to_owned(),
        enabled: true,
        overflow_priority: u8::MAX,
    });
}

pub(crate) fn add_source_toggle_command(
    toolbar: &mut ui::editor_toolbar::EditorToolbarInput,
    showing_source: bool,
) {
    let Some(group) = toolbar.groups.first_mut() else {
        return;
    };
    if group.commands.iter().any(|command| command.command_key == "toggle_source") {
        return;
    }
    let insertion_index = group
        .commands
        .iter()
        .position(|command| command.overflow_priority > 0)
        .unwrap_or(group.commands.len());
    group.commands.insert(
        insertion_index,
        ui::editor_toolbar::EditorToolbarCommandInput {
            command_key: "toggle_source".to_owned(),
            label: if showing_source { "可视化" } else { "源码" }.to_owned(),
            enabled: true,
            overflow_priority: 0,
        },
    );
}

fn history_toolbar_input() -> ui::editor_toolbar::EditorToolbarInput {
    toolbar_input(&[("undo", "撤销", 0), ("redo", "重做", 0)])
}

fn markdown_toolbar_input() -> ui::editor_toolbar::EditorToolbarInput {
    toolbar_input(&[
        ("undo", "撤销", 0),
        ("redo", "重做", 0),
        ("heading", "标题", 3),
        ("bold", "粗体", 4),
        ("italic", "斜体", 5),
        ("strike", "删除线", 6),
        ("inline_code", "行内代码", 7),
        ("unordered_list", "项目列表", 8),
        ("ordered_list", "编号列表", 9),
        ("task_list", "任务列表", 10),
        ("quote", "引用", 11),
        ("code_block", "代码块", 12),
        ("link", "链接", 13),
    ])
}

fn mindmap_toolbar_input() -> ui::editor_toolbar::EditorToolbarInput {
    toolbar_input(&[
        ("undo", "撤销", 0),
        ("redo", "重做", 0),
        ("mindmap_style", "主题", 0),
        ("promote", "提升层级", 3),
        ("demote", "降低层级", 4),
    ])
}

fn toolbar_input(commands: &[(&str, &str, u8)]) -> ui::editor_toolbar::EditorToolbarInput {
    ui::editor_toolbar::EditorToolbarInput {
        groups: vec![ui::editor_toolbar::EditorToolbarGroupInput {
            label: "编辑".to_owned(),
            commands: commands
                .iter()
                .map(|(command_key, label, overflow_priority)| {
                    ui::editor_toolbar::EditorToolbarCommandInput {
                        command_key: (*command_key).to_owned(),
                        label: (*label).to_owned(),
                        enabled: true,
                        overflow_priority: *overflow_priority,
                    }
                })
                .collect(),
        }],
        overflow_open: false,
    }
}

fn editor_command_actions() -> HashMap<String, NotoraAction> {
    use ui::plugin::SemanticEditCommand;

    [
        ("undo", SemanticEditCommand::Undo),
        ("redo", SemanticEditCommand::Redo),
        ("heading", SemanticEditCommand::SetHeadingLevel(2)),
        ("bold", SemanticEditCommand::ToggleBold),
        ("italic", SemanticEditCommand::ToggleItalic),
        ("strike", SemanticEditCommand::ToggleStrikethrough),
        ("inline_code", SemanticEditCommand::ToggleInlineCode),
        ("unordered_list", SemanticEditCommand::UnorderedList),
        ("ordered_list", SemanticEditCommand::OrderedList),
        ("task_list", SemanticEditCommand::TaskList),
        ("quote", SemanticEditCommand::Quote),
        ("code_block", SemanticEditCommand::CodeBlock),
        ("link", SemanticEditCommand::InsertLink),
        ("promote", SemanticEditCommand::PromoteObject),
        ("demote", SemanticEditCommand::DemoteObject),
    ]
    .into_iter()
    .map(|(key, command)| (key.to_owned(), NotoraAction::SemanticEditRequested(command)))
    .chain([
        ("toggle_source".to_owned(), NotoraAction::ToggleSourceViewRequested),
        ("mindmap_style".to_owned(), NotoraAction::ToggleMindmapStylePanelRequested),
    ])
    .collect()
}

fn note_toolbar_buttons(
    scope: &NavigationScope,
    selected_note_id: Option<NoteId>,
    external_files_present: bool,
) -> Vec<NoteToolbarButtonInput> {
    if *scope == NavigationScope::ExternalFiles {
        let mut buttons = vec![NoteToolbarButtonInput {
            label: "打开".to_owned(),
            action: NotoraAction::OpenExternalFileDialogRequested,
        }];
        if external_files_present {
            buttons.push(NoteToolbarButtonInput {
                label: "清空".to_owned(),
                action: NotoraAction::ExternalFilesClearRequested,
            });
        }
        return buttons;
    }
    if matches!(scope, NavigationScope::Search { .. }) {
        return vec![NoteToolbarButtonInput {
            label: "清除".to_owned(),
            action: NotoraAction::SearchCommitted { query: String::new(), search_generation: None },
        }];
    }
    if *scope == NavigationScope::Trash {
        let Some(note_id) = selected_note_id else {
            return vec![NoteToolbarButtonInput {
                label: "清空".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::Empty),
            }];
        };
        return vec![
            NoteToolbarButtonInput {
                label: "恢复".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::Restore { note_id }),
            },
            NoteToolbarButtonInput {
                label: "删除".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::PermanentlyDelete {
                    note_id,
                }),
            },
        ];
    }
    let _ = selected_note_id;
    Vec::new()
}

fn new_note_control_state(
    scope: &NavigationScope,
    workspace_root: WorkspaceRootState,
) -> NewNoteControlState {
    match scope {
        NavigationScope::Search { .. }
        | NavigationScope::Starred
        | NavigationScope::Trash
        | NavigationScope::Tag { .. }
        | NavigationScope::ExternalFiles => NewNoteControlState::Hidden,
        NavigationScope::WorkspaceRoot | NavigationScope::Directory { .. } => {
            match workspace_root {
                WorkspaceRootState::Missing => NewNoteControlState::Disabled,
                WorkspaceRootState::Active => NewNoteControlState::Enabled,
            }
        }
    }
}

fn card_empty_state_input(state: &NotoraState) -> StatusStateInput {
    if state.workspace_root == WorkspaceRootState::Missing {
        return StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "尚未设置工作区根目录".to_owned(),
            description: "请先选择一个文件夹作为工作区根目录。".to_owned(),
            icon: Some("folder-open".to_owned()),
            action_label: Some("设置根目录".to_owned()),
            action_id: Some(SET_WORKSPACE_ROOT_BUTTON_ID),
        };
    }
    StatusStateInput {
        kind: StatusStateKind::Empty,
        title: "暂无笔记".to_owned(),
        description: "新建一篇笔记，或者从左侧选择其他位置。".to_owned(),
        icon: Some("notebook-pen".to_owned()),
        action_label: None,
        action_id: None,
    }
}

/// 产品层已决定的确认弹层纯展示输入；UI 不保存 TagId 或 Trash entry。
#[derive(Clone, Debug, PartialEq)]
pub struct ConfirmationOverlayInput {
    pub title: String,
    pub description: String,
    pub confirm_label: String,
    pub confirm_action: NotoraAction,
}

struct RenderedToolbarButton {
    rect: Rect,
    label: String,
    action: NotoraAction,
}

/// 产品事件的路由结果；控件可以只消费事件而不立即产生产品动作。
pub struct NotoraEventRoute {
    pub actions: Vec<NotoraAction>,
    pub consumed: bool,
    pub canvas_scrollbar_action: Option<CanvasScrollbarsAction>,
    pub cursor_hint: Option<winit::window::CursorIcon>,
}

impl NotoraEventRoute {
    fn ignored() -> Self {
        Self {
            actions: Vec::new(),
            consumed: false,
            canvas_scrollbar_action: None,
            cursor_hint: None,
        }
    }

    fn consumed(action: Option<NotoraAction>) -> Self {
        Self {
            actions: action.into_iter().collect(),
            consumed: true,
            canvas_scrollbar_action: None,
            cursor_hint: None,
        }
    }

    fn passthrough(action: NotoraAction) -> Self {
        Self {
            actions: vec![action],
            consumed: false,
            canvas_scrollbar_action: None,
            cursor_hint: None,
        }
    }

    fn canvas_scrollbar(action: Option<CanvasScrollbarsAction>) -> Self {
        Self {
            actions: Vec::new(),
            consumed: true,
            canvas_scrollbar_action: action,
            cursor_hint: None,
        }
    }
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
    new_document_menu: Option<PopupMenuWidget>,
    editor_pane: EditorPaneChrome,
    editor_note_id: Option<NoteId>,
    editor_location_actions: HashMap<String, NotoraAction>,
    editor_tag_actions: HashMap<String, NotoraAction>,
    editor_command_actions: HashMap<String, NotoraAction>,
    mindmap_style_panel: MindmapStylePanelWidget,
    mindmap_style_panel_open: bool,
    mindmap_style_panel_rect: Rect,
    settings_overlay: SettingsOverlay,
    settings_overlay_open: bool,
    new_workspace_dialog: Option<NewWorkspaceDialog>,
    new_workspace_dialog_input: NewWorkspaceDialogInput,
    new_workspace_dialog_open: bool,
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
    compact_navigation_rect: Rect,
    compact_back_rect: Rect,
    confirmation_panel_rect: Rect,
    confirmation_confirm_rect: Rect,
    confirmation_cancel_rect: Rect,
    confirmation_action: Option<NotoraAction>,
    save_conflict_panel_rect: Rect,
    save_conflict_button_rects: [Rect; 4],
    save_conflict_actions: Option<[NotoraAction; 4]>,
    focused_text_input: Option<FocusTarget>,
    text_cursor_visible: bool,
    next_text_cursor_blink_at: Option<Instant>,
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
        search_box.set_blink(true);
        let mut new_note_button = SplitButtonWidget::new();
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
            new_document_menu: None,
            editor_pane: EditorPaneChrome::new(),
            editor_note_id: None,
            editor_location_actions: HashMap::new(),
            editor_tag_actions: HashMap::new(),
            editor_command_actions: HashMap::new(),
            mindmap_style_panel: MindmapStylePanelWidget::new(),
            mindmap_style_panel_open: false,
            mindmap_style_panel_rect: Rect::ZERO,
            settings_overlay: SettingsOverlay::new(),
            settings_overlay_open: false,
            new_workspace_dialog: None,
            new_workspace_dialog_input: NewWorkspaceDialogInput::default(),
            new_workspace_dialog_open: false,
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
            compact_navigation_rect: Rect::ZERO,
            compact_back_rect: Rect::ZERO,
            confirmation_panel_rect: Rect::ZERO,
            confirmation_confirm_rect: Rect::ZERO,
            confirmation_cancel_rect: Rect::ZERO,
            confirmation_action: None,
            save_conflict_panel_rect: Rect::ZERO,
            save_conflict_button_rects: [Rect::ZERO; 4],
            save_conflict_actions: None,
            focused_text_input: None,
            text_cursor_visible: true,
            next_text_cursor_blink_at: None,
        }
    }

    pub(crate) fn synchronize_focus(&mut self, focus_target: FocusTarget, now: Instant) {
        let editor_focused_id = match focus_target {
            FocusTarget::EditorTitle => Some(ui::editor_header::EDITOR_HEADER_TITLE_ID),
            FocusTarget::EditorTag => Some(ui::tag_editor::TAG_EDITOR_INPUT_ID),
            _ => None,
        };
        self.editor_pane.set_keyboard_focus(editor_focused_id);
        let focused_text_input = if focus_target == FocusTarget::EditorTag
            && self.editor_pane.tag_editor_has_keyboard_focus()
        {
            Some(FocusTarget::EditorTag)
        } else {
            match focus_target {
                FocusTarget::NavigationSearch | FocusTarget::EditorTitle => Some(focus_target),
                FocusTarget::NavigationTree if self.navigation_tree.input().editor.is_some() => {
                    Some(FocusTarget::NavigationTree)
                }
                FocusTarget::Overlay if self.new_workspace_dialog_open => {
                    Some(FocusTarget::Overlay)
                }
                _ => None,
            }
        };
        if self.focused_text_input != focused_text_input {
            self.focused_text_input = focused_text_input;
            self.text_cursor_visible = true;
            self.next_text_cursor_blink_at =
                focused_text_input.map(|_| now + TEXT_CURSOR_BLINK_INTERVAL);
        }
        self.search_box.set_keyboard_focus(
            (focus_target == FocusTarget::NavigationSearch).then_some(GLOBAL_SEARCH_BOX_ID),
        );
        self.navigation_tree.set_keyboard_focus(
            (focus_target == FocusTarget::NavigationTree).then_some(NAVIGATION_TREE_ID),
        );
        self.apply_text_cursor_visibility();
    }

    pub(crate) fn editor_title_text(&self) -> &str {
        self.editor_pane.title_text()
    }

    pub fn set_canvas_scrollbars_input(
        &mut self,
        input: Option<CanvasScrollbarsInput>,
        canvas_rect: Rect,
        context: &mut ui::LayoutCtx<'_>,
    ) {
        self.canvas_scrollbars_input = input;
        self.canvas_rect = canvas_rect;
        self.canvas_scrollbars.set_input(input.unwrap_or_default());
        self.canvas_scrollbars.set_rect(local_rect(canvas_rect), context);
    }

    pub fn paint_canvas_scrollbars(&self, context: &mut ui::PaintCtx<'_>) {
        if self.canvas_scrollbars_input.is_none() {
            return;
        }
        paint_at(context, self.canvas_rect, |context| self.canvas_scrollbars.paint(context));
    }

    pub(crate) fn advance_text_cursor_blink(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.next_text_cursor_blink_at else {
            return false;
        };
        if now < deadline {
            return false;
        }
        self.text_cursor_visible = !self.text_cursor_visible;
        self.next_text_cursor_blink_at = Some(now + TEXT_CURSOR_BLINK_INTERVAL);
        self.apply_text_cursor_visibility();
        true
    }

    pub(crate) fn next_text_cursor_blink_at(&self) -> Option<Instant> {
        self.next_text_cursor_blink_at
    }

    pub(crate) fn focused_text_input_ime_cursor_rect(&self) -> Option<Rect> {
        match self.focused_text_input? {
            FocusTarget::NavigationSearch => Some(self.search_box.ime_cursor_rect()),
            FocusTarget::EditorTitle | FocusTarget::EditorTag => {
                self.editor_pane.focused_ime_cursor_rect()
            }
            FocusTarget::NavigationTree => self.navigation_tree.ime_cursor_rect(),
            FocusTarget::Overlay => {
                self.new_workspace_dialog.as_ref().and_then(NewWorkspaceDialog::ime_cursor_rect)
            }
            FocusTarget::CardList | FocusTarget::Editor => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn search_box_is_focused(&self) -> bool {
        self.search_box.is_focused()
    }

    #[cfg(test)]
    pub(crate) fn search_box_rect(&self) -> Rect {
        self.search_box.rect()
    }

    fn apply_text_cursor_visibility(&mut self) {
        self.search_box.set_blink(self.text_cursor_visible);
        self.navigation_tree.set_editor_blink(self.text_cursor_visible);
        self.editor_pane.set_title_blink_visible(self.text_cursor_visible);
        self.editor_pane.set_tag_blink_visible(self.text_cursor_visible);
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
        self.new_note_button.set_input(SplitButtonInput {
            label: "新建笔记".to_owned(),
            enabled: model.new_note_control.is_enabled(),
        });
        self.new_note_button.set_menu_open(model.show_new_document_menu);
        self.settings_overlay.set_input(model.settings_overlay.clone());
        self.settings_overlay_open = model.show_settings_overlay;
        self.new_workspace_dialog_input = model.new_workspace_dialog.clone().unwrap_or_default();
        self.new_workspace_dialog_open = model.new_workspace_dialog.is_some();
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
        self.mindmap_style_panel_open = model.mindmap_style_panel.is_some();
        if let Some(input) = model.mindmap_style_panel.as_ref() {
            self.mindmap_style_panel.set_input(input.clone());
        }
        self.mindmap_style_panel.set_keyboard_focus(
            self.mindmap_style_panel_open.then_some(ui::core::widget::ids::MINDMAP_STYLE_PANEL),
        );
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

    fn synchronize_new_document_menu(&mut self, menu: ui::popup_menu::PopupMenu) {
        let menu_size_changed = self.new_document_menu_rect.w != menu.menu_rect.w
            || self.new_document_menu_rect.h != menu.menu_rect.h;
        self.new_document_menu_rect = menu.menu_rect;
        if self.new_document_menu.is_none() || menu_size_changed {
            self.new_document_menu = Some(PopupMenuWidget::new(menu));
        }
    }

    pub fn render(
        &mut self,
        frame: &mut EditorFrame,
        layout: ShellLayout,
        model: &NotoraRenderModel,
    ) -> Result<(), RenderError> {
        self.update_model(model);
        let dpi = layout.dpi;
        let padding = SHELL_PADDING_LOGICAL * dpi;
        let search_rect = Rect::new(
            layout.navigation_rect.x + padding,
            layout.navigation_rect.y + padding,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            SEARCH_BAR_HEIGHT_LOGICAL * dpi,
        );
        let search_icon_area_width = SEARCH_ICON_AREA_WIDTH_LOGICAL * dpi;
        let settings_rect = Rect::new(
            layout.navigation_rect.x + padding,
            layout.navigation_rect.bottom() - (SIDEBAR_CONTROL_HEIGHT_LOGICAL + 10.0) * dpi,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            SIDEBAR_CONTROL_HEIGHT_LOGICAL * dpi,
        );
        let tree_rect = Rect::new(
            layout.navigation_rect.x + padding,
            search_rect.bottom() + padding,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            (settings_rect.y - search_rect.bottom() - padding * 2.0).max(0.0),
        );
        self.navigation_tree_rect = tree_rect;
        self.search_rect = search_rect;
        self.editor_rect = layout.editor_rect;
        let mindmap_panel_width =
            (PANEL_WIDTH_LOGICAL * dpi).min(layout.editor_body_rect.w.max(0.0));
        self.mindmap_style_panel_rect = if self.mindmap_style_panel_open {
            Rect::new(
                layout.editor_body_rect.right() - mindmap_panel_width,
                layout.editor_body_rect.y,
                mindmap_panel_width,
                layout.editor_body_rect.h,
            )
        } else {
            Rect::ZERO
        };
        frame.with_layout_context(|context| {
            self.editor_pane.set_rects(
                EditorPaneRects {
                    header: layout.editor_header_rect,
                    toolbar: layout.editor_toolbar_rect,
                    body: layout.editor_body_rect,
                },
                context,
            );
            self.mindmap_style_panel.set_rect(local_rect(self.mindmap_style_panel_rect), context);
        });
        self.settings_rect = settings_rect;
        let card_header = card_header_layout(
            layout.card_list_rect,
            dpi,
            &model.card_list_title,
            model.new_note_control,
            model.note_toolbar.len(),
        );
        let new_note_rect = new_note_button_rect(
            layout.card_list_rect,
            dpi,
            model.new_note_control,
            card_header.control_top_y,
        );
        if model.show_new_document_menu {
            let metrics =
                ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), dpi);
            let menu = ui::sidebar::build_new_document_menu(
                new_note_rect,
                (layout.overlay_rect.right(), layout.overlay_rect.bottom()),
                &metrics,
            );
            self.synchronize_new_document_menu(menu);
        } else {
            self.new_document_menu_rect = Rect::ZERO;
            self.new_document_menu = None;
        }
        let splitters_enabled = layout.responsive_mode == ResponsiveLayoutMode::ThreePane;
        self.navigation_splitter.set_input(SplitterInput {
            logical_position: layout.navigation_width_logical,
            minimum_logical_position: crate::shell::layout::MINIMUM_NAVIGATION_WIDTH_LOGICAL,
            maximum_logical_position: crate::shell::layout::MAXIMUM_NAVIGATION_WIDTH_LOGICAL,
            enabled: splitters_enabled,
        });
        self.card_list_splitter.set_input(SplitterInput {
            logical_position: layout.card_list_width_logical,
            minimum_logical_position: crate::shell::layout::MINIMUM_CARD_LIST_WIDTH_LOGICAL,
            maximum_logical_position: crate::shell::layout::MAXIMUM_CARD_LIST_WIDTH_LOGICAL,
            enabled: layout.responsive_mode != ResponsiveLayoutMode::EditorOverlay,
        });
        let card_content_rect = Rect::new(
            layout.card_list_rect.x + padding,
            card_header.content_top_y,
            (layout.card_list_rect.w - padding * 2.0).max(0.0),
            (layout.card_list_rect.bottom() - card_header.content_top_y - padding).max(0.0),
        );
        self.card_content_rect = card_content_rect;
        let tool_button_width = NOTE_TOOL_BUTTON_WIDTH_LOGICAL * dpi;
        let tool_button_height = NOTE_TOOL_BUTTON_HEIGHT_LOGICAL * dpi;
        let toolbar_trailing_inset = if model.new_note_control.is_visible() {
            layout.card_list_rect.right() - new_note_rect.x + NOTE_TOOL_BUTTON_GAP_LOGICAL * dpi
        } else {
            padding
        };
        self.note_toolbar_buttons = layout_note_toolbar(
            layout.card_list_rect,
            padding,
            toolbar_trailing_inset,
            tool_button_width,
            tool_button_height,
            card_header.control_top_y,
            &model.note_toolbar,
        );
        self.compact_navigation_rect = if layout.responsive_mode != ResponsiveLayoutMode::ThreePane
            && layout.navigation_rect == Rect::ZERO
        {
            Rect::new(
                layout.card_list_rect.x + padding,
                layout.card_list_rect.y + 8.0 * dpi,
                COMPACT_NAVIGATION_BUTTON_WIDTH_LOGICAL * dpi,
                tool_button_height,
            )
        } else {
            Rect::ZERO
        };
        self.compact_back_rect = if layout.responsive_mode == ResponsiveLayoutMode::EditorOverlay
            && layout.editor_rect != Rect::ZERO
        {
            Rect::new(
                layout.editor_rect.x + padding,
                layout.editor_rect.y + 8.0 * dpi,
                COMPACT_BACK_BUTTON_WIDTH_LOGICAL * dpi,
                tool_button_height,
            )
        } else {
            Rect::ZERO
        };
        self.layout_confirmation_overlay(layout.overlay_rect, dpi, model.confirmation.is_some());
        self.layout_save_conflict_overlay(layout.overlay_rect, dpi, model.save_conflict.is_some());
        frame.with_layout_context(|context| {
            self.search_box.set_rect(search_rect, context);
            self.navigation_tree.set_rect(tree_rect, context);
            self.card_list.set_rect(card_content_rect, context);
            self.card_empty_state.set_rect(card_content_rect, context);
            self.editor_empty_state.set_rect(layout.editor_body_rect, context);
            self.navigation_splitter.set_rect(layout.navigation_splitter_rect, context);
            self.card_list_splitter.set_rect(layout.card_list_splitter_rect, context);
            self.new_note_button.set_rect(new_note_rect, context);
            if let Some(menu) = self.new_document_menu.as_mut() {
                menu.set_rect(local_rect(self.new_document_menu_rect), context);
            }
            if model.show_settings_overlay {
                self.settings_overlay.set_rect(layout.overlay_rect, context);
            }
            if self.new_workspace_dialog_open {
                let dialog = self
                    .new_workspace_dialog
                    .get_or_insert_with(|| NewWorkspaceDialog::new(context.theme));
                dialog.set_input(self.new_workspace_dialog_input.clone(), true);
                dialog.set_rect(layout.overlay_rect, context);
            } else if let Some(dialog) = self.new_workspace_dialog.as_mut() {
                dialog.set_input(NewWorkspaceDialogInput::default(), false);
            }
        });
        frame.with_underlay_paint_context(|context| {
            let application_theme = context.theme.application_theme();
            context.list.fill(layout.navigation_rect, application_theme.navigation_surface);
            context.list.fill(layout.card_list_rect, application_theme.content_surface);
            context.list.fill(layout.editor_rect, application_theme.editor_surface);
            self.editor_pane.paint_underlay(context);
            self.search_box.paint(context);
            let search_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
            draw_icon(
                context.list,
                "search",
                search_rect.x + (search_icon_area_width - search_icon_size) * 0.5,
                search_rect.y + (search_rect.h - search_icon_size) * 0.5,
                search_icon_size,
                application_theme.text_secondary,
            );
            self.navigation_tree.paint(context);
            self.navigation_splitter.paint(context);
            self.card_list_splitter.paint(context);
            self.new_note_button.paint(context);
            let settings_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
            let settings_horizontal_inset = SHELL_PADDING_LOGICAL * context.dpi;
            draw_icon(
                context.list,
                "settings",
                settings_rect.x + settings_horizontal_inset,
                settings_rect.y + (settings_rect.h - settings_icon_size) * 0.5,
                settings_icon_size,
                application_theme.text_secondary,
            );
            context.text(
                settings_rect.x
                    + settings_horizontal_inset
                    + settings_icon_size
                    + 2.0 * context.dpi,
                settings_rect.y
                    + settings_rect.h * 0.5
                    + SIDEBAR_LABEL_FONT_SIZE_LOGICAL * 0.35 * context.dpi,
                SIDEBAR_LABEL_FONT_SIZE_LOGICAL * context.dpi,
                application_theme.text_secondary,
                "设置",
            );
            context.text(
                layout.card_list_rect.x + padding,
                card_header.title_baseline_y,
                CARD_HEADER_TITLE_FONT_SIZE_LOGICAL * context.dpi,
                application_theme.text_primary,
                &model.card_list_title,
            );
            if self.compact_navigation_rect != Rect::ZERO {
                paint_note_tool_button(context, self.compact_navigation_rect, "笔记库");
            }
            for button in &self.note_toolbar_buttons {
                paint_note_tool_button(context, button.rect, &button.label);
            }
            if model.cards.is_empty() {
                self.card_empty_state.paint(context);
            } else {
                self.card_list.paint(context);
            }
        });
        match model.editor_pane {
            EditorPaneState::Empty => {
                frame.paint_editor_with(layout.editor_body_rect, |context| {
                    self.editor_empty_state.paint(context)
                })?;
            }
            EditorPaneState::Active => frame.paint_editor(layout.editor_body_rect)?,
        }
        frame.with_paint_context(|context| self.editor_pane.paint_overlay(context));
        if self.mindmap_style_panel_open {
            frame.with_paint_context(|context| {
                paint_at(context, self.mindmap_style_panel_rect, |context| {
                    self.mindmap_style_panel.paint(context)
                });
            });
        }
        if layout.responsive_mode != ResponsiveLayoutMode::ThreePane
            && layout.navigation_rect != Rect::ZERO
        {
            frame.with_paint_context(|context| self.paint_compact_navigation_overlay(context));
        }
        if self.compact_back_rect != Rect::ZERO {
            frame.with_paint_context(|context| {
                paint_note_tool_button(context, self.compact_back_rect, "返回");
            });
        }
        if model.show_settings_overlay {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.overlay_rect,
                    context.theme.application_theme().modal_scrim,
                    0.0,
                );
                self.settings_overlay.paint(context);
            });
        }
        if self.new_workspace_dialog_open
            && let Some(dialog) = self.new_workspace_dialog.as_ref()
        {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.overlay_rect,
                    context.theme.application_theme().modal_scrim,
                    0.0,
                );
                dialog.paint(context);
            });
        }
        if let Some(confirmation) = &model.confirmation {
            frame.with_paint_context(|context| {
                let application_theme = context.theme.application_theme();
                context.list.fill_rounded(layout.overlay_rect, application_theme.modal_scrim, 0.0);
                context.list.fill_rounded(
                    self.confirmation_panel_rect,
                    application_theme.overlay_surface,
                    10.0 * context.dpi,
                );
                context.text(
                    self.confirmation_panel_rect.x + 20.0 * context.dpi,
                    self.confirmation_panel_rect.y + 38.0 * context.dpi,
                    17.0 * context.dpi,
                    application_theme.text_primary,
                    &confirmation.title,
                );
                context.text(
                    self.confirmation_panel_rect.x + 20.0 * context.dpi,
                    self.confirmation_panel_rect.y + 72.0 * context.dpi,
                    13.0 * context.dpi,
                    application_theme.text_secondary,
                    &confirmation.description,
                );
                paint_note_tool_button(context, self.confirmation_cancel_rect, "取消");
                paint_note_tool_button(
                    context,
                    self.confirmation_confirm_rect,
                    &confirmation.confirm_label,
                );
            });
        }
        if model.save_conflict.is_some() {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.overlay_rect,
                    context.theme.application_theme().modal_scrim,
                    0.0,
                );
                self.paint_save_conflict_overlay(context);
            });
        }
        if let Some(menu) = self.new_document_menu.as_ref() {
            frame.with_paint_context(|context| {
                paint_at(context, self.new_document_menu_rect, |context| menu.paint(context));
            });
        }
        let product_overlay_visible = model.show_settings_overlay
            || model.new_workspace_dialog.is_some()
            || model.confirmation.is_some()
            || model.save_conflict.is_some()
            || model.show_new_document_menu;
        if !product_overlay_visible && let Some(hint) = self.active_tooltip.as_ref() {
            let (tooltip, tooltip_rect) =
                TooltipWidget::new(hint, layout.dpi, layout.overlay_rect.w, layout.overlay_rect.h);
            frame.with_paint_context(|context| {
                paint_at(context, tooltip_rect, |context| tooltip.paint(context));
            });
        }
        Ok(())
    }

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
            WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
                value: TextPayload::Plain(command_key),
            }) => self.editor_command_actions.get(command_key).cloned(),
            WidgetAction::MindmapStylePanel(action) => {
                Some(NotoraAction::MindmapStylePanel(action.clone()))
            }
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
                Some(NotoraAction::CreateRequested(DocumentKind::Markdown))
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

    /// 产品事件先路由给本产品 widget，并独立返回消费状态与产品动作。
    pub fn route_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        theme: &ui::Theme,
        dpi: f32,
    ) -> NotoraEventRoute {
        let mut clipboard = appkit_shell::SystemClipboard;
        let mut event_context = EventCtx::with_clipboard(theme, dpi, &mut clipboard);
        self.synchronize_focus(focus_target, Instant::now());
        let mut route =
            self.route_event_with_context(event, focus_target, None, &mut event_context);
        let tooltip_changed = self.synchronize_tooltip(event, None);
        route.consumed |= tooltip_changed;
        route.cursor_hint = event_context.cursor_hint;
        route
    }

    pub(crate) fn route_event_with_overlay(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        overlay: OverlayState,
        theme: &ui::Theme,
        dpi: f32,
    ) -> NotoraEventRoute {
        let mut clipboard = appkit_shell::SystemClipboard;
        let mut event_context = EventCtx::with_clipboard(theme, dpi, &mut clipboard);
        self.synchronize_focus(focus_target, Instant::now());
        let mut route =
            self.route_event_with_context(event, focus_target, Some(overlay), &mut event_context);
        let tooltip_changed = self.synchronize_tooltip(event, Some(overlay));
        route.consumed |= tooltip_changed;
        route.cursor_hint = event_context.cursor_hint;
        route
    }

    fn route_event_with_context(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        product_overlay: Option<OverlayState>,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        if let Some(route) = self.route_product_overlay_event(event, product_overlay, event_context)
        {
            return route;
        }
        if let Some(route) = self.route_mindmap_style_panel_event(event, event_context) {
            return route;
        }
        if self.editor_pane.has_open_popup() {
            return self.route_editor_popup_event(event, event_context);
        }
        if event_is_keyboard(event) {
            return self.route_keyboard_or_ime_event(event, focus_target, event_context);
        }
        self.route_pointer_event(event, focus_target, event_context)
    }

    fn synchronize_tooltip(
        &mut self,
        event: &Event,
        product_overlay: Option<OverlayState>,
    ) -> bool {
        let next_tooltip = if product_overlay.is_some_and(|overlay| overlay != OverlayState::None) {
            None
        } else {
            match event {
                Event::MouseMove { px, py } => self.tooltip_at(*px, *py),
                Event::MouseDown { .. }
                | Event::PointerLeave
                | Event::InteractionCancel
                | Event::Wheel { .. } => None,
                Event::MouseUp { .. }
                | Event::KeyDown(..)
                | Event::ImePreedit { .. }
                | Event::ImeCommit(_)
                | Event::ImeEnable
                | Event::ImeDisable => return false,
            }
        };
        if self.active_tooltip == next_tooltip {
            return false;
        }
        self.active_tooltip = next_tooltip;
        true
    }

    fn tooltip_at(&self, px: f32, py: f32) -> Option<TooltipHint> {
        if self.editor_pane.has_open_popup() {
            return None;
        }
        if self.mindmap_style_panel_open && self.mindmap_style_panel_rect.contains(px, py) {
            let hint = self.mindmap_style_panel.tooltip_at(
                px - self.mindmap_style_panel_rect.x,
                py - self.mindmap_style_panel_rect.y,
            )?;
            return Some(offset_tooltip_hint(hint, self.mindmap_style_panel_rect));
        }
        self.new_note_button
            .tooltip_at(px, py)
            .or_else(|| self.navigation_tree.tooltip_at(px, py))
            .or_else(|| self.card_list.tooltip_at(px, py))
            .or_else(|| self.editor_pane.tooltip_at(px, py))
    }

    fn route_product_overlay_event(
        &mut self,
        event: &Event,
        product_overlay: Option<OverlayState>,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        if product_overlay.is_some_and(|overlay| {
            overlay != OverlayState::None && !self.modal_input_is_ready(overlay)
        }) {
            return Some(NotoraEventRoute::consumed(escape_dismiss_action(event)));
        }
        if self.settings_overlay_open()
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::Settings)
        {
            let action = self
                .settings_overlay
                .route_event(event, event_context)
                .map(settings_overlay_action_to_notora_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.new_workspace_dialog_open
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::NewWorkspace)
        {
            let action = self
                .new_workspace_dialog
                .as_mut()
                .and_then(|dialog| dialog.route_event(event, event_context))
                .map(new_workspace_dialog_action_to_notora_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.save_conflict_actions.is_some()
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::SaveConflict)
        {
            let action =
                self.route_save_conflict_event(event).or_else(|| escape_dismiss_action(event));
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.confirmation_action.is_some() && product_overlay.is_none_or(is_confirmation_overlay)
        {
            let action =
                self.confirmation_overlay_action(event).or_else(|| escape_dismiss_action(event));
            return Some(NotoraEventRoute::consumed(action));
        }
        if self.new_document_menu_open
            && product_overlay.is_none_or(|overlay| overlay == OverlayState::NewDocumentMenu)
        {
            return Some(self.route_new_document_menu_event(event, event_context));
        }
        None
    }

    fn route_new_document_menu_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        let local_event =
            translate_event(event, self.new_document_menu_rect.x, self.new_document_menu_rect.y);
        let action = self
            .new_document_menu
            .as_mut()
            .and_then(|menu| menu.on_event(&local_event, event_context))
            .as_ref()
            .and_then(new_document_menu_action);
        NotoraEventRoute::consumed(action)
    }

    fn route_editor_popup_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        let widget_action = self.editor_pane.route_event(event, event_context);
        let action = widget_action.as_ref().and_then(|action| self.translate_widget_action(action));
        NotoraEventRoute::consumed(action)
    }

    fn route_mindmap_style_panel_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        if !self.mindmap_style_panel_open {
            return None;
        }
        let local_event = translate_event(
            event,
            self.mindmap_style_panel_rect.x,
            self.mindmap_style_panel_rect.y,
        );
        let widget_action = self.mindmap_style_panel.on_event(&local_event, event_context);
        let action = widget_action.as_ref().and_then(|action| self.translate_widget_action(action));
        let pointer_inside = event_pointer_position(event)
            .is_some_and(|(px, py)| self.mindmap_style_panel_rect.contains(px, py));
        if widget_action.is_some() || event_is_keyboard(event) || pointer_inside {
            return Some(NotoraEventRoute::consumed(action));
        }
        None
    }

    fn route_keyboard_or_ime_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        if matches!(focus_target, FocusTarget::EditorTitle | FocusTarget::EditorTag)
            && let Some(widget_action) = self.editor_pane.route_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return NotoraEventRoute::consumed(action);
        }
        let widget_action = self.route_focused_widget_event(event, focus_target, event_context);
        let action = widget_action
            .as_ref()
            .and_then(|widget_action| self.translate_widget_action(widget_action));
        if action.is_some() || widget_action.is_some() {
            return NotoraEventRoute::consumed(action);
        }
        if matches!(focus_target, FocusTarget::EditorTag | FocusTarget::Overlay) {
            return NotoraEventRoute::consumed(None);
        }
        NotoraEventRoute::ignored()
    }

    fn route_pointer_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        let pointer_focus = pointer_target(event, self);
        let card_hover_cleared = matches!(event, Event::MouseMove { .. })
            && pointer_focus != Some(FocusTarget::CardList)
            && self.card_list.on_event(event, event_context).is_some();
        if let Some(mut route) = self.route_pointer_chrome_event(event, event_context) {
            route.consumed |= card_hover_cleared;
            return route;
        }
        let widget_focus = pointer_focus.unwrap_or(focus_target);
        let widget_action = self.route_focused_widget_event(event, widget_focus, event_context);
        let action = widget_action
            .as_ref()
            .and_then(|widget_action| self.translate_widget_action(widget_action));
        if action.is_some() {
            return NotoraEventRoute::consumed(action);
        }
        if is_left_mouse_down(event)
            && let Some(focus_target) = pointer_focus
        {
            let focus_action = NotoraAction::FocusRequested(focus_target);
            if focus_target == FocusTarget::Editor {
                return NotoraEventRoute::passthrough(focus_action);
            }
            return NotoraEventRoute::consumed(Some(focus_action));
        }
        if widget_action.is_some() || card_hover_cleared {
            return NotoraEventRoute::consumed(None);
        }
        NotoraEventRoute::ignored()
    }

    fn route_pointer_chrome_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        if let Some(action) =
            compact_layout_action(event, self.compact_navigation_rect, self.compact_back_rect)
        {
            return Some(NotoraEventRoute::consumed(Some(action)));
        }
        if let Some(route) = self.route_canvas_scrollbars_event(event, event_context) {
            return Some(route);
        }
        if let Some(widget_action) = self.editor_pane.route_event(event, event_context) {
            let action = self.translate_widget_action(&widget_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if let Some(action) = note_toolbar_action(event, &self.note_toolbar_buttons) {
            return Some(NotoraEventRoute::consumed(Some(action)));
        }
        if is_splitter_pointer_event(event)
            && let Some(widget_action) = self.new_note_button.on_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        if let Some(action) = self.route_splitter_event(event, event_context) {
            return Some(NotoraEventRoute::consumed(action));
        }
        if let Some(action) = settings_button_action(event, self.settings_rect) {
            return Some(NotoraEventRoute::consumed(Some(action)));
        }
        if self.card_empty_state_visible
            && let Some(widget_action) = self.card_empty_state.on_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return Some(NotoraEventRoute::consumed(action));
        }
        None
    }

    fn route_focused_widget_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        event_context: &mut EventCtx,
    ) -> Option<WidgetAction> {
        match focus_target {
            FocusTarget::NavigationSearch => self.search_box.on_event(event, event_context),
            FocusTarget::NavigationTree => self.navigation_tree.on_event(event, event_context),
            FocusTarget::CardList => self.card_list.on_event(event, event_context),
            FocusTarget::Editor
            | FocusTarget::EditorTitle
            | FocusTarget::EditorTag
            | FocusTarget::Overlay => None,
        }
    }

    fn modal_input_is_ready(&self, overlay: OverlayState) -> bool {
        match overlay {
            OverlayState::None => true,
            OverlayState::Settings => self.settings_overlay_open,
            OverlayState::NewDocumentMenu => self.new_document_menu_open,
            OverlayState::NewWorkspace => {
                self.new_workspace_dialog_open && self.new_workspace_dialog.is_some()
            }
            OverlayState::TrashPermanentDeletionConfirmation { .. }
            | OverlayState::TrashRestoreConflictConfirmation { .. } => {
                self.confirmation_action.is_some()
            }
            OverlayState::SaveConflict => self.save_conflict_actions.is_some(),
        }
    }

    fn route_canvas_scrollbars_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<NotoraEventRoute> {
        if !matches!(
            event,
            Event::MouseMove { .. }
                | Event::MouseDown { .. }
                | Event::MouseUp { .. }
                | Event::Wheel { .. }
        ) {
            return None;
        }
        let was_capturing = self.canvas_scrollbars.is_capturing();
        if self.canvas_scrollbars_input.is_none() && !was_capturing {
            return None;
        }
        let local_event = translate_event(event, self.canvas_rect.x, self.canvas_rect.y);
        let pointer_hits_scrollbar = match &local_event {
            Event::MouseMove { px, py }
            | Event::MouseDown { px, py, .. }
            | Event::MouseUp { px, py, .. }
            | Event::Wheel { px, py, .. } => self.canvas_scrollbars.hit(*px, *py),
            _ => false,
        };
        let should_dispatch =
            was_capturing || pointer_hits_scrollbar || matches!(event, Event::MouseMove { .. });
        if !should_dispatch {
            return None;
        }
        let widget_action = self.canvas_scrollbars.on_event(&local_event, event_context);
        match widget_action {
            Some(WidgetAction::CanvasScrollbars(action)) => {
                Some(NotoraEventRoute::canvas_scrollbar(Some(action)))
            }
            Some(_) => Some(NotoraEventRoute::canvas_scrollbar(None)),
            None if matches!(event, Event::Wheel { .. }) => None,
            None if was_capturing || pointer_hits_scrollbar => {
                Some(NotoraEventRoute::canvas_scrollbar(None))
            }
            None => None,
        }
    }

    fn route_splitter_event(
        &mut self,
        event: &Event,
        event_context: &mut EventCtx,
    ) -> Option<Option<NotoraAction>> {
        if !is_splitter_pointer_event(event) {
            return None;
        }
        let navigation_action = self.navigation_splitter.on_event(event, event_context);
        if navigation_action.is_some() {
            return Some(
                navigation_action
                    .as_ref()
                    .and_then(|action| splitter_action_to_notora_action(action, Pane::Navigation)),
            );
        }
        let card_list_action = self.card_list_splitter.on_event(event, event_context);
        card_list_action.map(|action| splitter_action_to_notora_action(&action, Pane::CardList))
    }

    pub fn settings_button_id(&self) -> WidgetId {
        SETTINGS_BUTTON_ID
    }

    fn settings_overlay_open(&self) -> bool {
        self.settings_overlay_open
    }

    fn layout_confirmation_overlay(&mut self, overlay_rect: Rect, dpi: f32, visible: bool) {
        if !visible {
            self.confirmation_panel_rect = Rect::ZERO;
            self.confirmation_confirm_rect = Rect::ZERO;
            self.confirmation_cancel_rect = Rect::ZERO;
            return;
        }
        let panel_width = (CONFIRMATION_PANEL_WIDTH_LOGICAL * dpi).min(overlay_rect.w.max(0.0));
        let panel_height = (CONFIRMATION_PANEL_HEIGHT_LOGICAL * dpi).min(overlay_rect.h.max(0.0));
        self.confirmation_panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - panel_width) * 0.5,
            overlay_rect.y + (overlay_rect.h - panel_height) * 0.5,
            panel_width,
            panel_height,
        );
        let button_width = CONFIRMATION_BUTTON_WIDTH_LOGICAL * dpi;
        let button_height = CONFIRMATION_BUTTON_HEIGHT_LOGICAL * dpi;
        let button_y = self.confirmation_panel_rect.bottom() - button_height - 16.0 * dpi;
        self.confirmation_confirm_rect = Rect::new(
            self.confirmation_panel_rect.right() - button_width - 20.0 * dpi,
            button_y,
            button_width,
            button_height,
        );
        self.confirmation_cancel_rect = Rect::new(
            self.confirmation_confirm_rect.x - button_width - 8.0 * dpi,
            button_y,
            button_width,
            button_height,
        );
    }

    fn layout_save_conflict_overlay(&mut self, overlay_rect: Rect, dpi: f32, visible: bool) {
        if !visible {
            self.save_conflict_panel_rect = Rect::ZERO;
            self.save_conflict_button_rects = [Rect::ZERO; 4];
            return;
        }
        let panel_width = (SAVE_CONFLICT_PANEL_WIDTH_LOGICAL * dpi).min(overlay_rect.w.max(0.0));
        let panel_height = (SAVE_CONFLICT_PANEL_HEIGHT_LOGICAL * dpi).min(overlay_rect.h.max(0.0));
        self.save_conflict_panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - panel_width) * 0.5,
            overlay_rect.y + (overlay_rect.h - panel_height) * 0.5,
            panel_width,
            panel_height,
        );
        let gap = 8.0 * dpi;
        let horizontal_padding = 20.0 * dpi;
        let button_height = CONFIRMATION_BUTTON_HEIGHT_LOGICAL * dpi;
        let button_width = ((panel_width - horizontal_padding * 2.0 - gap * 3.0) / 4.0).max(0.0);
        let button_y = self.save_conflict_panel_rect.bottom() - button_height - 18.0 * dpi;
        self.save_conflict_button_rects = std::array::from_fn(|index| {
            Rect::new(
                self.save_conflict_panel_rect.x
                    + horizontal_padding
                    + index as f32 * (button_width + gap),
                button_y,
                button_width,
                button_height,
            )
        });
    }

    fn route_save_conflict_event(&self, event: &Event) -> Option<NotoraAction> {
        let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
            return None;
        };
        let actions = self.save_conflict_actions.as_ref()?;
        self.save_conflict_button_rects
            .iter()
            .position(|rect| rect.contains(*px, *py))
            .map(|index| actions[index].clone())
    }

    fn paint_save_conflict_overlay(&self, context: &mut ui::PaintCtx<'_>) {
        let application_theme = context.theme.application_theme();
        context.list.fill_rounded(
            self.save_conflict_panel_rect,
            application_theme.overlay_surface,
            10.0 * context.dpi,
        );
        context.text(
            self.save_conflict_panel_rect.x + 20.0 * context.dpi,
            self.save_conflict_panel_rect.y + 38.0 * context.dpi,
            17.0 * context.dpi,
            application_theme.text_primary,
            "磁盘文件已更改",
        );
        context.text(
            self.save_conflict_panel_rect.x + 20.0 * context.dpi,
            self.save_conflict_panel_rect.y + 72.0 * context.dpi,
            13.0 * context.dpi,
            application_theme.text_secondary,
            "请选择如何处理本地编辑，文件不会被静默覆盖。",
        );
        for (rect, label) in
            self.save_conflict_button_rects.iter().zip(["重新载入", "保存副本", "重试", "取消"])
        {
            paint_note_tool_button(context, *rect, label);
        }
    }

    fn confirmation_overlay_action(&self, event: &Event) -> Option<NotoraAction> {
        let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
            return None;
        };
        if self.confirmation_confirm_rect.contains(*px, *py) {
            return self.confirmation_action.clone();
        }
        if self.confirmation_cancel_rect.contains(*px, *py)
            || (self.confirmation_panel_rect != Rect::ZERO
                && !self.confirmation_panel_rect.contains(*px, *py))
        {
            return Some(NotoraAction::OverlayDismissed);
        }
        None
    }

    fn paint_compact_navigation_overlay(&self, context: &mut ui::PaintCtx<'_>) {
        let application_theme = context.theme.application_theme();
        let panel_rect = Rect::new(
            self.search_rect.x - SHELL_PADDING_LOGICAL * context.dpi,
            0.0,
            self.search_rect.w + SHELL_PADDING_LOGICAL * context.dpi * 2.0,
            self.settings_rect.bottom() + 10.0 * context.dpi,
        );
        context.list.fill(panel_rect, application_theme.navigation_surface);
        self.search_box.paint(context);
        let search_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
        draw_icon(
            context.list,
            "search",
            self.search_rect.x
                + (SEARCH_ICON_AREA_WIDTH_LOGICAL * context.dpi - search_icon_size) * 0.5,
            self.search_rect.y + (self.search_rect.h - search_icon_size) * 0.5,
            search_icon_size,
            application_theme.text_secondary,
        );
        self.navigation_tree.paint(context);
        let settings_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
        let settings_horizontal_inset = SHELL_PADDING_LOGICAL * context.dpi;
        draw_icon(
            context.list,
            "settings",
            self.settings_rect.x + settings_horizontal_inset,
            self.settings_rect.y + (self.settings_rect.h - settings_icon_size) * 0.5,
            settings_icon_size,
            application_theme.text_secondary,
        );
        context.text(
            self.settings_rect.x
                + settings_horizontal_inset
                + settings_icon_size
                + 2.0 * context.dpi,
            self.settings_rect.y
                + self.settings_rect.h * 0.5
                + SIDEBAR_LABEL_FONT_SIZE_LOGICAL * 0.35 * context.dpi,
            SIDEBAR_LABEL_FONT_SIZE_LOGICAL * context.dpi,
            application_theme.text_secondary,
            "设置",
        );
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

fn render_cards(state: &NotoraState) -> Vec<RenderCard> {
    if state.library.navigation_scope == NavigationScope::ExternalFiles {
        return state.external_files.sessions().iter().map(render_external_file_card).collect();
    }
    let mut cards = match &state.library.card_page {
        CardPageState::Ready { cards, .. }
        | CardPageState::LoadingNextPage { cards, .. }
        | CardPageState::Refreshing { cards, .. }
        | CardPageState::Failed { cards, .. } => cards.iter().map(render_catalog_card).collect(),
        CardPageState::Idle
        | CardPageState::LoadingInitial { .. }
        | CardPageState::Empty { .. } => Vec::new(),
    };
    let Some(selected_identity) = state.library.selected_card else {
        return cards;
    };
    let selected_title = state.library.title_draft.as_ref().or_else(|| {
        state
            .library
            .pending_title_commit
            .as_ref()
            .filter(|pending_title| pending_title.identity == selected_identity)
            .map(|pending_title| &pending_title.title)
    });
    let Some(selected_title) = selected_title else {
        return cards;
    };
    if let Some(selected_card) = cards.iter_mut().find(|card| card.identity == selected_identity) {
        selected_card.title.clone_from(selected_title);
    }
    cards
}

fn render_catalog_card(card: &notora_core::CatalogCard) -> RenderCard {
    let mut summary_parts = Vec::with_capacity(card.tags.len() + usize::from(card.starred));
    if card.starred {
        summary_parts.push("★".to_owned());
    }
    summary_parts.extend(card.tags.iter().map(|tag| format!("#{tag}")));
    RenderCard {
        identity: DocumentIdentity::Note(card.note_id),
        title: card.title.clone(),
        excerpt: card.excerpt.clone(),
        timestamp: format_modified_timestamp(card.modified_nanoseconds),
        icon: Some(document_icon(card.kind).to_owned()),
        tag_summary: summary_parts.join(" "),
        closable: false,
    }
}

fn render_external_file_card(session: &ExternalFileSession) -> RenderCard {
    match session {
        ExternalFileSession::Existing { canonical_path, .. } => RenderCard {
            identity: session.identity(),
            title: canonical_path
                .as_path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("未命名")
                .to_owned(),
            excerpt: canonical_path.as_path().display().to_string(),
            timestamp: "外部文件".to_owned(),
            icon: Some(
                document_icon(
                    DocumentKind::from_path(canonical_path.as_path()).unwrap_or(DocumentKind::Text),
                )
                .to_owned(),
            ),
            tag_summary: String::new(),
            closable: true,
        },
        ExternalFileSession::Untitled { kind, .. } => RenderCard {
            identity: session.identity(),
            title: "未命名".to_owned(),
            excerpt: "尚未保存的外部文件".to_owned(),
            timestamp: "外部文件".to_owned(),
            icon: Some(document_icon(*kind).to_owned()),
            tag_summary: String::new(),
            closable: true,
        },
        ExternalFileSession::Missing { last_known_path, .. } => RenderCard {
            identity: session.identity(),
            title: last_known_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("文件已丢失")
                .to_owned(),
            excerpt: last_known_path.display().to_string(),
            timestamp: "已丢失".to_owned(),
            icon: Some("file".to_owned()),
            tag_summary: String::new(),
            closable: true,
        },
    }
}

fn document_icon(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text => "file-text",
        DocumentKind::Markdown => "code",
        DocumentKind::Mindmap => "list-tree",
    }
}

fn format_modified_timestamp(modified_nanoseconds: i64) -> String {
    format_modified_timestamp_at(modified_nanoseconds, SystemTime::now())
}

fn format_modified_timestamp_at(modified_nanoseconds: i64, now: SystemTime) -> String {
    let Ok(modified_nanoseconds) = u64::try_from(modified_nanoseconds) else {
        return "修改时间未知".to_owned();
    };
    let Some(modified_at) = UNIX_EPOCH.checked_add(Duration::from_nanos(modified_nanoseconds))
    else {
        return "修改时间未知".to_owned();
    };
    format_modified_time(modified_at, now)
}

fn pointer_target(event: &Event, shell: &NotoraShell) -> Option<FocusTarget> {
    let (px, py) = event_pointer_position(event)?;
    if shell.search_rect.contains(px, py) {
        return Some(FocusTarget::NavigationSearch);
    }
    if shell.navigation_tree_rect.contains(px, py) {
        return Some(FocusTarget::NavigationTree);
    }
    if shell.card_content_rect.contains(px, py) {
        return Some(FocusTarget::CardList);
    }
    if shell.editor_rect.contains(px, py) {
        return Some(FocusTarget::Editor);
    }
    None
}

fn event_pointer_position(event: &Event) -> Option<(f32, f32)> {
    match event {
        Event::MouseMove { px, py }
        | Event::MouseDown { px, py, .. }
        | Event::MouseUp { px, py, .. }
        | Event::Wheel { px, py, .. } => Some((*px, *py)),
        Event::KeyDown(..)
        | Event::ImePreedit { .. }
        | Event::ImeCommit(_)
        | Event::ImeEnable
        | Event::ImeDisable
        | Event::PointerLeave
        | Event::InteractionCancel => None,
    }
}

fn event_is_keyboard(event: &Event) -> bool {
    matches!(
        event,
        Event::KeyDown(..)
            | Event::ImePreedit { .. }
            | Event::ImeCommit(_)
            | Event::ImeEnable
            | Event::ImeDisable
    )
}

fn escape_dismiss_action(event: &Event) -> Option<NotoraAction> {
    matches!(event, Event::KeyDown(ui::KeyCode::Escape, _))
        .then_some(NotoraAction::OverlayDismissed)
}

fn is_confirmation_overlay(overlay: OverlayState) -> bool {
    matches!(
        overlay,
        OverlayState::TrashPermanentDeletionConfirmation { .. }
            | OverlayState::TrashRestoreConflictConfirmation { .. }
    )
}

fn is_splitter_pointer_event(event: &Event) -> bool {
    matches!(event, Event::MouseMove { .. } | Event::MouseDown { .. } | Event::MouseUp { .. })
}

fn is_left_mouse_down(event: &Event) -> bool {
    matches!(event, Event::MouseDown { button: ui::core::widget::MouseButton::Left, .. })
}

fn splitter_action_to_notora_action(action: &WidgetAction, pane: Pane) -> Option<NotoraAction> {
    match action {
        WidgetAction::Splitter(
            SplitterAction::LogicalPositionChanged(logical_width)
            | SplitterAction::DragEnded(logical_width),
        ) => Some(NotoraAction::SplitterDragged { pane, logical_width: *logical_width }),
        _ => None,
    }
}

fn settings_button_action(event: &Event, settings_rect: Rect) -> Option<NotoraAction> {
    let Event::MouseDown { px, py, button: ui::core::widget::MouseButton::Left } = event else {
        return None;
    };
    settings_rect.contains(*px, *py).then_some(NotoraAction::OpenSettings)
}

fn settings_overlay_action_to_notora_action(action: SettingsOverlayAction) -> NotoraAction {
    match action {
        SettingsOverlayAction::Update(update) => {
            NotoraAction::ProductSettingsUpdateRequested(update)
        }
        SettingsOverlayAction::RetryPersistence => NotoraAction::RetryProductSettingsPersistence,
        SettingsOverlayAction::ViewChanged => NotoraAction::SettingsViewChanged,
        SettingsOverlayAction::Dismiss => NotoraAction::OverlayDismissed,
    }
}

fn new_workspace_dialog_action_to_notora_action(action: NewWorkspaceDialogAction) -> NotoraAction {
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

fn confirmation_overlay_input(overlay: OverlayState) -> Option<ConfirmationOverlayInput> {
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

fn paint_note_tool_button(context: &mut ui::PaintCtx<'_>, rect: Rect, label: &str) {
    let application_theme = context.theme.application_theme();
    context.list.fill_rounded(rect, application_theme.overlay_surface, 4.0 * context.dpi);
    context.text(
        rect.x + 8.0 * context.dpi,
        rect.y + rect.h * 0.5 + 5.0 * context.dpi,
        12.0 * context.dpi,
        application_theme.text_secondary,
        label,
    );
}

fn layout_note_toolbar(
    card_list_rect: Rect,
    padding: f32,
    trailing_inset: f32,
    button_width: f32,
    button_height: f32,
    button_y: f32,
    inputs: &[NoteToolbarButtonInput],
) -> Vec<RenderedToolbarButton> {
    let scale = button_height / NOTE_TOOL_BUTTON_HEIGHT_LOGICAL;
    let gap = NOTE_TOOL_BUTTON_GAP_LOGICAL * scale;
    let available_width = (card_list_rect.w - padding - trailing_inset).max(0.0);
    let count = inputs.len();
    let fitted_width = if count == 0 {
        0.0
    } else {
        ((available_width - gap * count.saturating_sub(1) as f32) / count as f32)
            .max(0.0)
            .min(button_width)
    };
    inputs
        .iter()
        .rev()
        .enumerate()
        .map(|(index, input)| RenderedToolbarButton {
            rect: Rect::new(
                card_list_rect.right()
                    - trailing_inset
                    - fitted_width
                    - index as f32 * (fitted_width + gap),
                button_y,
                fitted_width,
                button_height,
            ),
            label: input.label.clone(),
            action: input.action.clone(),
        })
        .collect()
}

fn note_toolbar_action(event: &Event, buttons: &[RenderedToolbarButton]) -> Option<NotoraAction> {
    let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
        return None;
    };
    buttons.iter().find(|button| button.rect.contains(*px, *py)).map(|button| button.action.clone())
}

fn compact_layout_action(
    event: &Event,
    compact_navigation_rect: Rect,
    compact_back_rect: Rect,
) -> Option<NotoraAction> {
    let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
        return None;
    };
    if compact_navigation_rect.contains(*px, *py) {
        return Some(NotoraAction::CompactNavigationRequested);
    }
    compact_back_rect.contains(*px, *py).then_some(NotoraAction::CompactBackRequested)
}

fn new_document_menu_action(action: &WidgetAction) -> Option<NotoraAction> {
    let WidgetAction::Popup(outcome) = action else {
        return None;
    };
    match outcome {
        PopupOutcome::Selected(PopupMenuAction::NewDocument(kind)) => {
            Some(NotoraAction::CreateRequested(match kind {
                NewDocumentKind::Text => DocumentKind::Text,
                NewDocumentKind::Mindmap => DocumentKind::Mindmap,
                NewDocumentKind::Markdown => DocumentKind::Markdown,
            }))
        }
        PopupOutcome::Dismiss => Some(NotoraAction::OverlayDismissed),
        PopupOutcome::Selected(_) => None,
    }
}

fn card_header_layout(
    card_list_rect: Rect,
    dpi: f32,
    title: &str,
    new_note_state: NewNoteControlState,
    toolbar_button_count: usize,
) -> CardHeaderLayout {
    let padding = SHELL_PADDING_LOGICAL * dpi;
    let gap = NOTE_TOOL_BUTTON_GAP_LOGICAL * dpi;
    let title_width = ui::core::text_util::estimate_text_width_px(
        title,
        CARD_HEADER_TITLE_FONT_SIZE_LOGICAL * dpi,
    );
    let toolbar_width = if toolbar_button_count == 0 {
        0.0
    } else {
        NOTE_TOOL_BUTTON_WIDTH_LOGICAL * dpi * toolbar_button_count as f32
            + gap * toolbar_button_count.saturating_sub(1) as f32
    };
    let new_note_width =
        if new_note_state.is_visible() { NEW_NOTE_BUTTON_WIDTH_LOGICAL * dpi } else { 0.0 };
    let control_group_gap = if toolbar_width > 0.0 && new_note_width > 0.0 { gap } else { 0.0 };
    let controls_width = toolbar_width + control_group_gap + new_note_width;
    let title_control_gap = if controls_width > 0.0 { gap } else { 0.0 };
    let inner_width = (card_list_rect.w - padding * 2.0).max(0.0);
    let wraps = title_width + title_control_gap + controls_width > inner_width;
    let control_top_logical = if wraps {
        CARD_HEADER_WRAPPED_CONTROL_TOP_LOGICAL
    } else {
        CARD_HEADER_CONTROL_TOP_LOGICAL
    };
    let content_top_logical = if wraps {
        CARD_HEADER_WRAPPED_CONTENT_TOP_LOGICAL
    } else {
        CARD_HEADER_CONTENT_TOP_LOGICAL
    };

    CardHeaderLayout {
        title_baseline_y: card_list_rect.y + CARD_HEADER_TITLE_BASELINE_LOGICAL * dpi,
        control_top_y: card_list_rect.y + control_top_logical * dpi,
        content_top_y: card_list_rect.y + content_top_logical * dpi,
    }
}

fn new_note_button_rect(
    card_list_rect: Rect,
    dpi: f32,
    state: NewNoteControlState,
    control_top_y: f32,
) -> Rect {
    if !state.is_visible() || card_list_rect.w <= 0.0 || card_list_rect.h <= 0.0 {
        return Rect::ZERO;
    }
    let padding = SHELL_PADDING_LOGICAL * dpi;
    let width =
        (NEW_NOTE_BUTTON_WIDTH_LOGICAL * dpi).min((card_list_rect.w - padding * 2.0).max(0.0));
    Rect::new(
        card_list_rect.right() - padding - width,
        control_top_y,
        width,
        NOTE_TOOL_BUTTON_HEIGHT_LOGICAL * dpi,
    )
}

fn local_rect(rect: Rect) -> Rect {
    Rect::new(0.0, 0.0, rect.w, rect.h)
}

fn offset_tooltip_hint(mut hint: TooltipHint, offset: Rect) -> TooltipHint {
    hint.target_rect.x += offset.x;
    hint.target_rect.y += offset.y;
    hint
}

fn paint_at(context: &mut ui::PaintCtx<'_>, rect: Rect, paint: impl FnOnce(&mut ui::PaintCtx<'_>)) {
    let saved_offset = context.list.offset;
    context.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
    paint(context);
    context.list.offset = saved_offset;
}

fn translate_event(event: &Event, offset_x: f32, offset_y: f32) -> Event {
    match event {
        Event::MouseMove { px, py } => Event::MouseMove { px: *px - offset_x, py: *py - offset_y },
        Event::PointerLeave => Event::PointerLeave,
        Event::MouseDown { px, py, button } => {
            Event::MouseDown { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
        Event::MouseUp { px, py, button } => {
            Event::MouseUp { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
        Event::InteractionCancel => Event::InteractionCancel,
        Event::Wheel { dx, dy, px, py } => {
            Event::Wheel { dx: *dx, dy: *dy, px: *px - offset_x, py: *py - offset_y }
        }
        Event::KeyDown(key, modifiers) => Event::KeyDown(*key, *modifiers),
        Event::ImePreedit { text, cursor } => {
            Event::ImePreedit { text: text.clone(), cursor: *cursor }
        }
        Event::ImeCommit(text) => Event::ImeCommit(text.clone()),
        Event::ImeEnable => Event::ImeEnable,
        Event::ImeDisable => Event::ImeDisable,
    }
}

fn card_list_title(scope: &NavigationScope) -> &'static str {
    match scope {
        NavigationScope::Search { .. } => "搜索结果",
        NavigationScope::WorkspaceRoot => "工作区",
        NavigationScope::Directory { .. } => "文件夹",
        NavigationScope::Starred => "星标",
        NavigationScope::Trash => "回收站",
        NavigationScope::Tag { .. } => "标签",
        NavigationScope::ExternalFiles => "文件",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn lifecycle_events_have_no_pointer_position_and_survive_coordinate_translation() {
        for event in [Event::PointerLeave, Event::InteractionCancel] {
            assert_eq!(event_pointer_position(&event), None);
            assert_eq!(translate_event(&event, 10.0, 20.0), event);
        }
    }
    use crate::NotoraState;
    use crate::action::CardQuery;
    use crate::state::CardPageState;
    use notora_core::{CatalogCard, DocumentIdentity, DocumentKind, NavigationScope, NoteId};

    #[test]
    fn builds_a_static_render_model() {
        let model = NotoraRenderModel::from_state(&NotoraState::default());
        assert_eq!(model.navigation_rows.len(), 4);
        assert_eq!(model.navigation_rows[0].icon.as_deref(), Some("folder-open"));
        assert_eq!(model.navigation_rows[0].label, "工作区");
        assert_eq!(model.navigation_rows[1].label, "星标");
        assert_eq!(model.navigation_rows[2].label, "回收站");
        assert_eq!(model.navigation_rows[3].label, "文件");
        assert_eq!(model.card_list_title, "工作区");
        assert!(model.cards.is_empty());
    }

    #[test]
    fn editor_times_use_calendar_dates_and_bounded_relative_labels() {
        let now = UNIX_EPOCH + std::time::Duration::from_secs(4 * 86_400);

        assert_eq!(format_created_time(UNIX_EPOCH), "创建 1970/01/01 UTC");
        assert_eq!(format_modified_time(now, now), "修改 刚刚");
        assert_eq!(
            format_modified_time(now - std::time::Duration::from_secs(5 * 60), now),
            "修改 5 分钟前"
        );
        assert_eq!(
            format_modified_time(now - std::time::Duration::from_secs(2 * 3_600), now),
            "修改 2 小时前"
        );
        assert_eq!(
            format_modified_time(now - std::time::Duration::from_secs(3 * 86_400), now),
            "修改 3 天前"
        );
        assert_eq!(
            format_modified_time(now + std::time::Duration::from_secs(60), now),
            "修改 刚刚"
        );
    }

    #[test]
    fn catalog_modified_timestamp_uses_the_same_relative_label_as_the_editor() {
        let now = UNIX_EPOCH + std::time::Duration::from_secs(10 * 60);
        let modified_nanoseconds = 5 * 60 * 1_000_000_000;

        assert_eq!(format_modified_timestamp_at(modified_nanoseconds, now), "修改 5 分钟前");
    }

    #[test]
    fn document_kinds_use_icons_registered_by_the_ui_renderer() {
        assert_eq!(document_icon(DocumentKind::Text), "file-text");
        assert_eq!(document_icon(DocumentKind::Markdown), "code");
        assert_eq!(document_icon(DocumentKind::Mindmap), "list-tree");
    }

    #[test]
    fn new_note_control_is_hidden_for_non_creation_scopes() {
        for scope in [
            NavigationScope::Search { query: "roadmap".to_owned() },
            NavigationScope::Starred,
            NavigationScope::Trash,
            NavigationScope::Tag { tag_id: notora_core::TagId::generate() },
            NavigationScope::ExternalFiles,
        ] {
            let mut state = NotoraState::default();
            state.library.navigation_scope = scope;

            assert_eq!(
                NotoraRenderModel::from_state(&state).new_note_control,
                NewNoteControlState::Hidden
            );
        }
    }

    #[test]
    fn missing_workspace_disables_new_note_and_exposes_a_separate_root_action() {
        let model = NotoraRenderModel::from_state(&NotoraState::default());

        assert_eq!(model.new_note_control, NewNoteControlState::Disabled);
        assert_eq!(model.card_empty_state.title, "尚未设置工作区根目录");
        assert_eq!(model.card_empty_state.action_label.as_deref(), Some("设置根目录"));

        let mut shell = NotoraShell::new();
        shell.update_model(&model);
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::Activated {
                id: SET_WORKSPACE_ROOT_BUTTON_ID,
            })),
            Some(NotoraAction::WorkspaceRootSelectionRequested)
        );
    }

    #[test]
    fn new_note_control_is_laid_out_inside_the_middle_column() {
        let navigation_rect = Rect::new(0.0, 0.0, 220.0, 600.0);
        let card_list_rect = Rect::new(228.0, 0.0, 340.0, 600.0);

        let button_rect = new_note_button_rect(
            card_list_rect,
            1.0,
            NewNoteControlState::Enabled,
            CARD_HEADER_CONTROL_TOP_LOGICAL,
        );

        assert!(button_rect != Rect::ZERO);
        assert!(card_list_rect.contains(button_rect.x, button_rect.y));
        assert!(card_list_rect.contains(button_rect.right(), button_rect.bottom()));
        assert!(!navigation_rect.contains(button_rect.x, button_rect.y));
        assert_eq!(
            new_note_button_rect(
                card_list_rect,
                1.0,
                NewNoteControlState::Hidden,
                CARD_HEADER_CONTROL_TOP_LOGICAL,
            ),
            Rect::ZERO
        );
    }

    #[test]
    fn narrow_card_header_keeps_the_title_clear_of_toolbar_buttons() {
        let card_list_rect = Rect::new(228.0, 0.0, 268.0, 600.0);
        let padding = SHELL_PADDING_LOGICAL;
        let header = card_header_layout(
            card_list_rect,
            1.0,
            "一个很长的中栏标题",
            NewNoteControlState::Enabled,
            1,
        );
        let new_note_rect = new_note_button_rect(
            card_list_rect,
            1.0,
            NewNoteControlState::Enabled,
            header.control_top_y,
        );
        let toolbar = layout_note_toolbar(
            card_list_rect,
            padding,
            card_list_rect.right() - new_note_rect.x + NOTE_TOOL_BUTTON_GAP_LOGICAL,
            NOTE_TOOL_BUTTON_WIDTH_LOGICAL,
            NOTE_TOOL_BUTTON_HEIGHT_LOGICAL,
            header.control_top_y,
            &[NoteToolbarButtonInput {
                label: "操作".to_owned(),
                action: NotoraAction::SettingsViewChanged,
            }],
        );
        let title_right = card_list_rect.x
            + padding
            + ui::core::text_util::estimate_text_width_px("一个很长的中栏标题", 16.0);
        let title_bottom = card_list_rect.y + 36.0;
        let first_button = &toolbar[0].rect;

        assert!(
            first_button.x >= title_right + NOTE_TOOL_BUTTON_GAP_LOGICAL
                || first_button.y >= title_bottom
        );
        assert!(header.content_top_y >= first_button.bottom());
    }

    #[test]
    fn files_toolbar_exposes_open_external_file_action() {
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::ExternalFiles;

        let model = NotoraRenderModel::from_state(&state);

        assert_eq!(
            model.note_toolbar,
            vec![NoteToolbarButtonInput {
                label: "打开".to_owned(),
                action: NotoraAction::OpenExternalFileDialogRequested,
            }]
        );
    }

    #[test]
    fn files_toolbar_exposes_clear_all_when_external_records_exist() {
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::ExternalFiles;
        let _ = state.external_files.create_untitled(DocumentKind::Markdown);

        let model = NotoraRenderModel::from_state(&state);

        assert_eq!(
            model.note_toolbar,
            vec![
                NoteToolbarButtonInput {
                    label: "打开".to_owned(),
                    action: NotoraAction::OpenExternalFileDialogRequested,
                },
                NoteToolbarButtonInput {
                    label: "清空".to_owned(),
                    action: NotoraAction::ExternalFilesClearRequested,
                },
            ]
        );
    }

    #[test]
    fn search_toolbar_exposes_clear_without_a_new_note_control() {
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::Search { query: "roadmap".to_owned() };

        let model = NotoraRenderModel::from_state(&state);

        assert_eq!(model.new_note_control, NewNoteControlState::Hidden);
        assert_eq!(
            model.note_toolbar,
            vec![NoteToolbarButtonInput {
                label: "清除".to_owned(),
                action: NotoraAction::SearchCommitted {
                    query: String::new(),
                    search_generation: None,
                },
            }]
        );
    }

    #[test]
    fn search_box_focus_request_paints_the_caret_after_state_sync() {
        use ui::core::paint::{DrawCmd, DrawList};

        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.search_rect = Rect::new(8.0, 8.0, 240.0, 32.0);
        shell.search_box.set_rect(shell.search_rect, &mut layout_context);
        let click = Event::MouseDown { px: 24.0, py: 24.0, button: ui::core::MouseButton::Left };

        let route = shell.route_event(&click, FocusTarget::Editor, &theme, 1.0);
        assert_eq!(
            route.actions,
            vec![NotoraAction::FocusRequested(FocusTarget::NavigationSearch)]
        );
        shell.synchronize_focus(FocusTarget::NavigationSearch, Instant::now());
        assert!(shell.search_box.is_focused());

        let mut draw_list = DrawList::new();
        let mut paint_context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);
        shell.search_box.paint(&mut paint_context);

        assert!(draw_list.cmds.iter().any(|command| {
            matches!(command, DrawCmd::FillRect { radius, .. } if *radius == 0.0)
        }));
    }

    #[test]
    fn notora_shell_routes_context_clipboard_to_search_and_editor_chrome() {
        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        let mut clipboard = TestClipboard("剪贴板内容".to_owned());
        let mut event_context = EventCtx::with_clipboard(&theme, 1.0, &mut clipboard);
        let command = ui::core::Modifiers { cmd: true, ..ui::core::Modifiers::NONE };

        shell.synchronize_focus(FocusTarget::NavigationSearch, Instant::now());
        let search_route = shell.route_event_with_context(
            &Event::KeyDown(ui::KeyCode::Char('v'), command),
            FocusTarget::NavigationSearch,
            None,
            &mut event_context,
        );
        assert_eq!(
            search_route.actions,
            vec![NotoraAction::SearchTextChanged("剪贴板内容".to_owned())]
        );

        shell.editor_pane.set_input(EditorPaneInput {
            mode: EditorPaneMode::WorkspaceNote,
            header: ui::editor_header::EditorHeaderInput {
                title: "旧标题".to_owned(),
                title_editable: true,
                ..ui::editor_header::EditorHeaderInput::default()
            },
            ..EditorPaneInput::default()
        });
        shell.synchronize_focus(FocusTarget::EditorTitle, Instant::now());
        let title_route = shell.route_event_with_context(
            &Event::KeyDown(ui::KeyCode::Char('v'), command),
            FocusTarget::EditorTitle,
            None,
            &mut event_context,
        );
        assert_eq!(shell.editor_title_text(), "剪贴板内容旧标题");
        assert_eq!(
            title_route.actions,
            vec![NotoraAction::TitleTextChanged("剪贴板内容旧标题".to_owned())]
        );
    }

    #[test]
    fn focused_title_exposes_a_window_space_ime_cursor_rect() {
        let mut shell = NotoraShell::new();
        let editor_input = EditorPaneInput {
            mode: EditorPaneMode::WorkspaceNote,
            header: ui::editor_header::EditorHeaderInput {
                title_editable: true,
                ..ui::editor_header::EditorHeaderInput::default()
            },
            ..EditorPaneInput::default()
        };
        shell.editor_pane.set_input(editor_input);
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.editor_pane.set_rects(
            EditorPaneRects {
                header: Rect::new(420.0, 48.0, 640.0, 108.0),
                toolbar: Rect::new(420.0, 156.0, 640.0, 40.0),
                body: Rect::new(420.0, 196.0, 640.0, 400.0),
            },
            &mut layout_context,
        );
        shell.synchronize_focus(FocusTarget::EditorTitle, Instant::now());

        let ime_rect = shell
            .focused_text_input_ime_cursor_rect()
            .expect("focused title must provide an IME candidate anchor");

        assert!(ime_rect.x >= 420.0 + 16.0);
        assert!(ime_rect.y >= 48.0);
    }

    #[test]
    fn focused_tag_exposes_an_ime_cursor_rect_until_focus_moves_away() {
        let mut shell = NotoraShell::new();
        shell.editor_pane.set_input(EditorPaneInput {
            mode: EditorPaneMode::WorkspaceNote,
            tags: ui::tag_editor::TagEditorInput {
                enabled: true,
                ..ui::tag_editor::TagEditorInput::default()
            },
            ..EditorPaneInput::default()
        });
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.editor_pane.set_rects(
            EditorPaneRects {
                header: Rect::new(420.0, 48.0, 640.0, 108.0),
                toolbar: Rect::new(420.0, 156.0, 640.0, 40.0),
                body: Rect::new(420.0, 196.0, 640.0, 400.0),
            },
            &mut layout_context,
        );

        shell.synchronize_focus(FocusTarget::EditorTag, Instant::now());
        let ime_rect = shell
            .focused_text_input_ime_cursor_rect()
            .expect("focused tag should provide an IME candidate anchor");
        assert!(ime_rect.x >= 420.0);
        assert!(ime_rect.y >= 48.0);

        shell.synchronize_focus(FocusTarget::NavigationTree, Instant::now());
        assert!(!shell.editor_pane.tag_editor_has_keyboard_focus());
        assert!(shell.focused_text_input_ime_cursor_rect().is_none());
    }

    #[test]
    fn focused_search_box_toggles_the_caret_every_blink_interval() {
        use ui::core::paint::{DrawCmd, DrawList};

        fn paints_caret(shell: &NotoraShell, theme: &ui::Theme) -> bool {
            let mut draw_list = DrawList::new();
            let mut paint_context = ui::PaintCtx::new(&mut draw_list, theme, 1.0);
            shell.search_box.paint(&mut paint_context);
            draw_list.cmds.iter().any(
                |command| matches!(command, DrawCmd::FillRect { radius, .. } if *radius == 0.0),
            )
        }

        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.search_box.set_rect(Rect::new(8.0, 8.0, 240.0, 32.0), &mut layout_context);
        let focused_at = Instant::now();

        shell.synchronize_focus(FocusTarget::NavigationSearch, focused_at);
        assert_eq!(
            shell.next_text_cursor_blink_at(),
            Some(focused_at + TEXT_CURSOR_BLINK_INTERVAL)
        );
        assert!(paints_caret(&shell, &theme));

        assert!(shell.advance_text_cursor_blink(focused_at + TEXT_CURSOR_BLINK_INTERVAL));
        assert!(!paints_caret(&shell, &theme));

        assert!(shell.advance_text_cursor_blink(
            focused_at + TEXT_CURSOR_BLINK_INTERVAL + TEXT_CURSOR_BLINK_INTERVAL
        ));
        assert!(paints_caret(&shell, &theme));
    }

    #[test]
    fn direction_keys_do_not_resize_splitters_without_splitter_focus() {
        let mut shell = NotoraShell::new();
        shell.navigation_splitter.set_input(SplitterInput {
            logical_position: 220.0,
            minimum_logical_position: 180.0,
            maximum_logical_position: 320.0,
            enabled: true,
        });
        shell.card_list_splitter.set_input(SplitterInput {
            logical_position: 340.0,
            minimum_logical_position: 280.0,
            maximum_logical_position: 480.0,
            enabled: true,
        });
        let theme = ui::theme::test_theme();

        let route = shell.route_event(
            &Event::KeyDown(ui::KeyCode::Left, ui::core::Modifiers::NONE),
            FocusTarget::Editor,
            &theme,
            1.0,
        );

        assert!(!route.consumed);
        assert!(route.actions.is_empty());
        assert_eq!(shell.navigation_splitter.logical_position(), 220.0);
        assert_eq!(shell.card_list_splitter.logical_position(), 340.0);
    }

    #[test]
    fn confirmation_overlay_blocks_keyboard_and_background_pointer_input() {
        let mut shell = NotoraShell::new();
        shell.navigation_splitter.set_input(SplitterInput {
            logical_position: 220.0,
            minimum_logical_position: 180.0,
            maximum_logical_position: 320.0,
            enabled: true,
        });
        shell.card_list_splitter.set_input(SplitterInput {
            logical_position: 340.0,
            minimum_logical_position: 280.0,
            maximum_logical_position: 480.0,
            enabled: true,
        });
        shell.confirmation_action = Some(NotoraAction::OverlayDismissed);
        shell.confirmation_panel_rect = Rect::new(200.0, 100.0, 400.0, 240.0);
        let theme = ui::theme::test_theme();

        let keyboard_route = shell.route_event_with_overlay(
            &Event::KeyDown(ui::KeyCode::Right, ui::core::Modifiers::NONE),
            FocusTarget::Editor,
            OverlayState::TrashPermanentDeletionConfirmation {
                operation: crate::action::TrashOperation::Empty,
            },
            &theme,
            1.0,
        );
        let pointer_route = shell.route_event_with_overlay(
            &Event::MouseDown { px: 300.0, py: 180.0, button: ui::MouseButton::Left },
            FocusTarget::Editor,
            OverlayState::TrashPermanentDeletionConfirmation {
                operation: crate::action::TrashOperation::Empty,
            },
            &theme,
            1.0,
        );

        assert!(keyboard_route.consumed);
        assert!(pointer_route.consumed);
        assert_eq!(shell.navigation_splitter.logical_position(), 220.0);
        assert_eq!(shell.card_list_splitter.logical_position(), 340.0);
    }

    #[test]
    fn new_document_menu_blocks_compact_layout_controls() {
        let mut shell = NotoraShell::new();
        shell.new_document_menu_open = true;
        shell.compact_navigation_rect = Rect::new(8.0, 8.0, 40.0, 40.0);
        let theme = ui::theme::test_theme();

        let route = shell.route_event_with_overlay(
            &Event::MouseDown { px: 24.0, py: 24.0, button: ui::MouseButton::Left },
            FocusTarget::Overlay,
            OverlayState::NewDocumentMenu,
            &theme,
            1.0,
        );

        assert!(route.consumed);
        assert!(route.actions.is_empty());
    }

    #[test]
    fn editor_popup_blocks_canvas_scrollbar_pointer_input() {
        use ui::canvas_scrollbars::CanvasScrollbarsInput;
        use ui::scrollbar::ScrollbarInput;

        let mut shell = NotoraShell::new();
        shell.editor_pane.set_input(EditorPaneInput {
            mode: EditorPaneMode::WorkspaceNote,
            tags: ui::tag_editor::TagEditorInput {
                enabled: true,
                suggestions_open: true,
                ..ui::tag_editor::TagEditorInput::default()
            },
            ..EditorPaneInput::default()
        });
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let editor_rect = Rect::new(200.0, 80.0, 800.0, 600.0);
        shell.set_canvas_scrollbars_input(
            Some(CanvasScrollbarsInput {
                horizontal: None,
                vertical: Some(ScrollbarInput {
                    viewport_height_px: 600.0,
                    total_display_rows: 2_400,
                    scroll_top_rows: 0.0,
                }),
            }),
            editor_rect,
            &mut layout_context,
        );

        let route = shell.route_event(
            &Event::MouseDown {
                px: editor_rect.right() - 2.0,
                py: editor_rect.y + 24.0,
                button: ui::MouseButton::Left,
            },
            FocusTarget::EditorTag,
            &theme,
            1.0,
        );

        assert!(route.consumed);
        assert_eq!(route.canvas_scrollbar_action, None);
    }

    #[test]
    fn canvas_scrollbar_captures_drag_and_reports_axis_action() {
        use ui::canvas::CanvasAxis;
        use ui::canvas_scrollbars::CanvasScrollbarsInput;
        use ui::scrollbar::{ScrollbarAction, ScrollbarInput};

        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let editor_rect = Rect::new(200.0, 80.0, 800.0, 600.0);
        shell.set_canvas_scrollbars_input(
            Some(CanvasScrollbarsInput {
                horizontal: None,
                vertical: Some(ScrollbarInput {
                    viewport_height_px: 600.0,
                    total_display_rows: 2_400,
                    scroll_top_rows: 0.0,
                }),
            }),
            editor_rect,
            &mut layout_context,
        );

        let hover = Event::MouseMove { px: editor_rect.right() - 2.0, py: editor_rect.y + 24.0 };
        let route = shell.route_event(&hover, FocusTarget::Editor, &theme, 1.0);
        assert_eq!(route.cursor_hint, Some(winit::window::CursorIcon::Default));

        let press = Event::MouseDown {
            px: editor_rect.right() - 2.0,
            py: editor_rect.y + 24.0,
            button: ui::MouseButton::Left,
        };
        let route = shell.route_event(&press, FocusTarget::Editor, &theme, 1.0);

        assert!(route.consumed);
        assert_eq!(
            route.canvas_scrollbar_action,
            Some(ui::canvas_scrollbars::CanvasScrollbarsAction {
                axis: CanvasAxis::Vertical,
                action: ScrollbarAction::StartDrag,
            })
        );

        let release = Event::MouseUp {
            px: editor_rect.x - 200.0,
            py: editor_rect.bottom() + 200.0,
            button: ui::MouseButton::Left,
        };
        let route = shell.route_event(&release, FocusTarget::Editor, &theme, 1.0);
        assert_eq!(
            route.canvas_scrollbar_action,
            Some(ui::canvas_scrollbars::CanvasScrollbarsAction {
                axis: CanvasAxis::Vertical,
                action: ScrollbarAction::EndDrag,
            })
        );

        let wheel = Event::Wheel {
            dx: 0.0,
            dy: -40.0,
            px: editor_rect.right() - 2.0,
            py: editor_rect.y + 120.0,
        };
        let route = shell.route_event(&wheel, FocusTarget::Editor, &theme, 1.0);
        assert!(!route.consumed, "wheel over the track must fall through to canvas panning");
    }

    #[test]
    fn canvas_scrollbar_paints_over_the_editor_surface() {
        use ui::canvas_scrollbars::CanvasScrollbarsInput;
        use ui::scrollbar::ScrollbarInput;

        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let editor_rect = Rect::new(200.0, 80.0, 800.0, 600.0);
        shell.set_canvas_scrollbars_input(
            Some(CanvasScrollbarsInput {
                horizontal: Some(ScrollbarInput {
                    viewport_height_px: 800.0,
                    total_display_rows: 3_200,
                    scroll_top_rows: 0.0,
                }),
                vertical: None,
            }),
            editor_rect,
            &mut layout_context,
        );
        let mut draw_list = ui::DrawList::new();
        let mut paint_context = ui::PaintCtx::new(&mut draw_list, &theme, 1.0);

        shell.paint_canvas_scrollbars(&mut paint_context);

        assert!(!draw_list.cmds.is_empty());
    }

    #[test]
    fn new_document_menu_exposes_only_direct_creation_actions() {
        let mut state =
            NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
        state.library.navigation_tree.directories = vec!["plans".into()];
        let _ = state.reduce(NotoraAction::OpenNewDocumentMenu);

        let model = NotoraRenderModel::from_state(&state);

        assert!(model.show_new_document_menu);

        let metrics = ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0);
        let button_rect = Rect::new(300.0, 20.0, 128.0, 28.0);
        let menu = ui::sidebar::build_new_document_menu(button_rect, (800.0, 600.0), &metrics);
        let labels: Vec<&str> = menu.items.iter().map(|item| item.label.as_str()).collect();

        assert_eq!(labels, vec!["新建 TXT", "新建 MMAP", "新建 MD"]);
        assert!(menu.menu_rect.y > button_rect.bottom());
    }

    #[test]
    fn new_document_menu_hover_survives_the_following_render_synchronization() {
        use ui::core::paint::{DrawCmd, DrawList};

        fn build_menu() -> ui::popup_menu::PopupMenu {
            let settings = ui::settings::Settings::new();
            let metrics = ui::settings::UiMetrics::from_settings(&settings, 1.0);
            ui::sidebar::build_new_document_menu(
                Rect::new(300.0, 20.0, 128.0, 28.0),
                (800.0, 600.0),
                &metrics,
            )
        }

        fn paints_second_item_hover(shell: &NotoraShell, theme: &ui::Theme) -> bool {
            let menu = shell.new_document_menu.as_ref().expect("new document menu should exist");
            let expected_rect = menu.menu().item_rects[1].shrink(1.0, 1.0, 1.0, 1.0);
            let mut draw_list = DrawList::new();
            let mut paint_context = ui::PaintCtx::new(&mut draw_list, theme, 1.0);
            menu.paint(&mut paint_context);
            draw_list.cmds.iter().any(|command| {
                matches!(
                    command,
                    DrawCmd::FillRect { rect, color, .. }
                        if *rect == expected_rect && *color == theme.palette.sidebar_hover_bg
                )
            })
        }

        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        shell.synchronize_new_document_menu(build_menu());
        let menu_rect = shell.new_document_menu_rect;
        let second_item_rect = shell
            .new_document_menu
            .as_ref()
            .expect("new document menu should exist")
            .menu()
            .item_rects[1];
        let mut event_context = EventCtx::new(&theme, 1.0);

        shell.route_new_document_menu_event(
            &Event::MouseMove {
                px: menu_rect.x + second_item_rect.x + second_item_rect.w * 0.5,
                py: menu_rect.y + second_item_rect.y + second_item_rect.h * 0.5,
            },
            &mut event_context,
        );
        assert!(paints_second_item_hover(&shell, &theme));

        shell.synchronize_new_document_menu(build_menu());

        assert!(paints_second_item_hover(&shell, &theme));
    }

    #[test]
    fn new_note_main_button_creates_markdown_directly() {
        let shell = NotoraShell::new();

        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::Activated {
                id: NEW_NOTE_BUTTON_ID,
            })),
            Some(NotoraAction::CreateRequested(DocumentKind::Markdown))
        );
    }

    #[test]
    fn popup_menu_actions_map_to_typed_creation_requests() {
        for (kind, expected) in [
            (NewDocumentKind::Text, DocumentKind::Text),
            (NewDocumentKind::Mindmap, DocumentKind::Mindmap),
            (NewDocumentKind::Markdown, DocumentKind::Markdown),
        ] {
            let action =
                WidgetAction::Popup(PopupOutcome::Selected(PopupMenuAction::NewDocument(kind)));

            assert_eq!(
                new_document_menu_action(&action),
                Some(NotoraAction::CreateRequested(expected))
            );
        }
    }

    #[test]
    fn dynamic_navigation_rows_keep_domain_values_out_of_the_ui_widget_keys() {
        let tag_id = notora_core::TagId::generate();
        let mut state =
            NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
        state.library.navigation_tree.directories = vec!["plans".into(), "plans/q3".into()];
        state.library.navigation_tree.expanded_directories.insert("plans".into());
        state.library.navigation_tree.tags = vec![notora_core::TagWithActiveNoteCount {
            tag_id,
            display_name: "Plan".to_owned(),
            active_note_count: 2,
        }];

        let model = NotoraRenderModel::from_state(&state);
        assert_eq!(model.navigation_rows.len(), 7);
        assert_eq!(model.navigation_rows[1].label, "plans");
        assert_eq!(model.navigation_rows[1].depth, 1);
        assert_eq!(model.navigation_rows[2].depth, 2);
        assert_eq!(model.navigation_rows[4].badge, Some(2));
        assert!(matches!(
            model.navigation_actions.get(&model.navigation_rows[4].key),
            Some(NotoraAction::NavigationSelected(NavigationScope::Tag { tag_id: selected_tag_id }))
                if *selected_tag_id == tag_id
        ));
    }

    #[test]
    fn workspace_root_is_the_expandable_parent_of_directory_rows() {
        let mut state =
            NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
        state.workspace_root_path = Some("/tmp/notora-workspace".into());
        state.library.navigation_tree.directories = vec!["docs".into(), "docs/plans".into()];
        state.library.navigation_tree.expanded_directories.insert("docs".into());

        let expanded_model = NotoraRenderModel::from_state(&state);
        assert_eq!(expanded_model.navigation_rows[0].label, "工作区");
        assert_eq!(
            expanded_model.navigation_rows[0].tooltip.as_deref(),
            Some("/tmp/notora-workspace")
        );
        assert_eq!(expanded_model.navigation_rows[0].depth, 0);
        assert_eq!(expanded_model.navigation_rows[0].expansion, TreeRowExpansion::Expanded);
        assert_eq!(expanded_model.navigation_rows[1].depth, 1);
        assert_eq!(expanded_model.navigation_rows[2].depth, 2);

        state.library.navigation_tree.workspace_root_expanded = false;
        let collapsed_model = NotoraRenderModel::from_state(&state);
        assert_eq!(collapsed_model.navigation_rows[0].expansion, TreeRowExpansion::Collapsed);
        assert!(collapsed_model.navigation_rows.iter().all(|row| row.depth == 0));
    }

    #[test]
    fn hovering_workspace_actions_exposes_their_tooltips_in_the_product_shell() {
        let mut state =
            NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
        state.library.navigation_tree.directories = vec!["docs".into()];
        let model = NotoraRenderModel::from_state(&state);
        let mut shell = NotoraShell::new();
        shell.update_model(&model);

        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        let tree_rect = Rect::new(12.0, 52.0, 240.0, 240.0);
        shell.navigation_tree_rect = tree_rect;
        shell.navigation_tree.set_rect(tree_rect, &mut layout_context);
        for expected_label in ["新建工作区", "打开工作区", "新建目录"] {
            let pointer_position = (tree_rect.x as usize..tree_rect.right() as usize)
                .flat_map(|px| {
                    (tree_rect.y as usize..tree_rect.bottom() as usize)
                        .map(move |py| (px as f32, py as f32))
                })
                .find(|(px, py)| {
                    shell
                        .navigation_tree
                        .tooltip_at(*px, *py)
                        .is_some_and(|hint| hint.label == expected_label)
                })
                .expect("workspace action should have a tooltip target");

            shell.route_event(
                &Event::MouseMove { px: pointer_position.0, py: pointer_position.1 },
                FocusTarget::Editor,
                &theme,
                1.0,
            );

            assert_eq!(
                shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()),
                Some(expected_label)
            );
        }

        shell.route_event(
            &Event::MouseMove { px: tree_rect.x, py: tree_rect.bottom() - 1.0 },
            FocusTarget::Editor,
            &theme,
            1.0,
        );
        assert_eq!(shell.active_tooltip, None);
    }

    #[test]
    fn hovering_a_closable_card_exposes_its_tooltip_in_the_product_shell() {
        let mut shell = NotoraShell::new();
        shell.card_list.set_input(VirtualCardListInput {
            cards: vec![CardInput {
                key: CardKey(7),
                title: "outside.md".to_owned(),
                excerpt: String::new(),
                timestamp: "外部文件".to_owned(),
                icon: Some("file-text".to_owned()),
                tag_summary: String::new(),
                selection: CardSelection::Unselected,
                closable: true,
            }],
            scroll_offset_px: 0.0,
        });
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.card_content_rect = Rect::new(240.0, 60.0, 320.0, 400.0);
        shell.card_list.set_rect(shell.card_content_rect, &mut layout_context);
        let close_rect = shell.card_list.layout().card_geometry(0).close_rect;
        let (px, py) = (close_rect.x + close_rect.w * 0.5, close_rect.y + close_rect.h * 0.5);

        shell.route_event(&Event::MouseMove { px, py }, FocusTarget::Editor, &theme, 1.0);

        assert_eq!(
            shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()),
            Some("关闭 outside.md")
        );
    }

    #[test]
    fn product_shell_collects_editor_header_and_split_button_tooltips() {
        let mut shell = NotoraShell::new();
        shell.editor_pane.set_input(EditorPaneInput {
            mode: EditorPaneMode::WorkspaceNote,
            header: ui::editor_header::EditorHeaderInput {
                title: "路线图".to_owned(),
                star_enabled: true,
                ..ui::editor_header::EditorHeaderInput::default()
            },
            ..EditorPaneInput::default()
        });
        shell
            .new_note_button
            .set_input(SplitButtonInput { label: "新建笔记".to_owned(), enabled: true });
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.editor_rect = Rect::new(200.0, 0.0, 640.0, 600.0);
        shell.editor_pane.set_rects(
            EditorPaneRects {
                header: Rect::new(200.0, 0.0, 640.0, 108.0),
                toolbar: Rect::new(200.0, 108.0, 640.0, 40.0),
                body: Rect::new(200.0, 148.0, 640.0, 452.0),
            },
            &mut layout_context,
        );
        shell.new_note_button.set_rect(Rect::new(40.0, 20.0, 128.0, 28.0), &mut layout_context);

        let star_pointer = (200..840)
            .flat_map(|px| (0..108).map(move |py| (px as f32, py as f32)))
            .find(|(px, py)| {
                shell.editor_pane.tooltip_at(*px, *py).is_some_and(|hint| hint.label == "添加星标")
            })
            .expect("editor star action should have a tooltip target");
        shell.route_event(
            &Event::MouseMove { px: star_pointer.0, py: star_pointer.1 },
            FocusTarget::Editor,
            &theme,
            1.0,
        );
        assert_eq!(shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()), Some("添加星标"));

        let menu_rect = shell.new_note_button.menu_rect();
        shell.route_event(
            &Event::MouseMove {
                px: menu_rect.x + menu_rect.w * 0.5,
                py: menu_rect.y + menu_rect.h * 0.5,
            },
            FocusTarget::Editor,
            &theme,
            1.0,
        );
        assert_eq!(
            shell.active_tooltip.as_ref().map(|hint| hint.label.as_str()),
            Some("更多新建笔记选项")
        );
    }

    #[test]
    fn directory_rows_expose_typed_creation_actions_and_inline_editor_input() {
        let mut state =
            NotoraState { workspace_root: WorkspaceRootState::Active, ..NotoraState::default() };
        state.library.navigation_tree.directories = vec!["docs".into()];
        state.directory_creation = DirectoryCreationState::Editing {
            parent_relative_path: "docs".into(),
            draft_name: "plans".to_owned(),
        };

        let model = NotoraRenderModel::from_state(&state);
        let root_row = &model.navigation_rows[0];
        let directory_row = &model.navigation_rows[1];

        assert_eq!(root_row.trailing_actions.len(), 3);
        assert_eq!(root_row.trailing_actions[0].key, NEW_WORKSPACE_ACTION_KEY);
        assert_eq!(root_row.trailing_actions[1].key, OPEN_WORKSPACE_ACTION_KEY);
        assert_eq!(root_row.trailing_actions[2].key, NEW_DIRECTORY_ACTION_KEY);
        assert_eq!(directory_row.trailing_actions.len(), 1);
        assert_eq!(directory_row.trailing_actions[0].key, NEW_DIRECTORY_ACTION_KEY);
        assert_eq!(
            model.navigation_trailing_actions.get(&(directory_row.key, NEW_DIRECTORY_ACTION_KEY)),
            Some(&NotoraAction::BeginDirectoryCreation { parent_relative_path: "docs".into() })
        );
        assert_eq!(
            model.navigation_editor,
            Some(TreeRowEditorInput {
                key: DIRECTORY_EDITOR_KEY,
                parent_key: directory_row.key,
                depth: 2,
                value: "plans".to_owned(),
                placeholder: "新目录名称".to_owned(),
            })
        );
    }

    #[test]
    fn missing_workspace_keeps_the_root_directory_action_disabled() {
        let model = NotoraRenderModel::from_state(&NotoraState::default());
        let root_row = &model.navigation_rows[0];

        assert_eq!(root_row.label, "工作区");
        assert_eq!(root_row.trailing_actions.len(), 3);
        assert!(root_row.trailing_actions[0].enabled);
        assert!(root_row.trailing_actions[1].enabled);
        assert!(!root_row.trailing_actions[2].enabled);
        assert_eq!(
            model.navigation_trailing_actions.get(&(root_row.key, NEW_WORKSPACE_ACTION_KEY)),
            Some(&NotoraAction::OpenWorkspaceCreationRequested)
        );
        assert_eq!(
            model.navigation_trailing_actions.get(&(root_row.key, OPEN_WORKSPACE_ACTION_KEY)),
            Some(&NotoraAction::WorkspaceRootSelectionRequested)
        );
        assert!(
            !model
                .navigation_trailing_actions
                .contains_key(&(root_row.key, NEW_DIRECTORY_ACTION_KEY))
        );
    }

    #[test]
    fn current_frame_keys_map_to_typed_navigation_actions() {
        let model = NotoraRenderModel::default();
        let mut shell = NotoraShell::new();
        shell.update_model(&model);
        shell.navigation_actions.insert(
            TreeRowKey(1),
            NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot),
        );

        assert_eq!(
            shell.translate_widget_action(&WidgetAction::TreeList(TreeListAction::Selected(
                TreeRowKey(1),
            ))),
            Some(NotoraAction::NavigationSelected(NavigationScope::WorkspaceRoot))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(
                ui::core::widget::ControlAction::Activated { id: shell.settings_button_id() },
            )),
            Some(NotoraAction::OpenSettings)
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextEdited {
                id: GLOBAL_SEARCH_BOX_ID,
                value: TextPayload::Plain("roadmap".to_owned()),
            })),
            Some(NotoraAction::SearchTextChanged("roadmap".to_owned()))
        );
    }

    #[test]
    fn tree_widget_directory_events_map_to_product_actions() {
        let mut shell = NotoraShell::new();
        shell.navigation_trailing_actions.insert(
            (TreeRowKey(7), NEW_DIRECTORY_ACTION_KEY),
            NotoraAction::BeginDirectoryCreation { parent_relative_path: "docs".into() },
        );

        assert_eq!(
            shell.translate_widget_action(&WidgetAction::TreeList(
                TreeListAction::TrailingActionActivated {
                    row_key: TreeRowKey(7),
                    action_key: NEW_DIRECTORY_ACTION_KEY,
                }
            )),
            Some(NotoraAction::BeginDirectoryCreation { parent_relative_path: "docs".into() })
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::TreeList(
                TreeListAction::EditorTextChanged {
                    key: DIRECTORY_EDITOR_KEY,
                    value: "plans".to_owned(),
                }
            )),
            Some(NotoraAction::DirectoryCreationTextChanged("plans".to_owned()))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::TreeList(
                TreeListAction::EditorCommitRequested {
                    key: DIRECTORY_EDITOR_KEY,
                    value: "plans".to_owned(),
                }
            )),
            Some(NotoraAction::DirectoryCreationCommitRequested)
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::TreeList(
                TreeListAction::EditorCancelled { key: DIRECTORY_EDITOR_KEY }
            )),
            Some(NotoraAction::DirectoryCreationCancelled)
        );
    }

    #[test]
    fn confirmation_overlay_maps_only_to_product_confirmation_actions() {
        let trash_confirmation =
            confirmation_overlay_input(OverlayState::TrashPermanentDeletionConfirmation {
                operation: crate::action::TrashOperation::Empty,
            })
            .expect("empty Trash confirmation should render");
        assert_eq!(
            trash_confirmation.confirm_action,
            NotoraAction::TrashPermanentDeletionConfirmed
        );

        let restore_conflict =
            confirmation_overlay_input(OverlayState::TrashRestoreConflictConfirmation {
                note_id: notora_core::NoteId::generate(),
            })
            .expect("restore conflict should render a decision overlay");
        assert_eq!(
            restore_conflict.confirm_action,
            NotoraAction::TrashRestoreWithRenamedPathConfirmed
        );
    }

    #[test]
    fn save_conflict_is_exposed_as_a_four_way_product_decision() {
        let identity = DocumentIdentity::Note(NoteId::generate());
        let mut state = NotoraState::default();
        let _ = state.reduce(NotoraAction::SaveConflictDetected { identity, content_revision: 9 });

        let model = NotoraRenderModel::from_state(&state);
        let conflict = model.save_conflict.expect("save conflict overlay should render");
        assert_eq!(conflict.identity, identity);
        assert_eq!(
            conflict.actions,
            [
                NotoraAction::SaveConflictResolutionRequested(
                    crate::action::ConflictResolution::ReloadFromDisk,
                ),
                NotoraAction::SaveConflictResolutionRequested(
                    crate::action::ConflictResolution::SaveCopy,
                ),
                NotoraAction::SaveConflictResolutionRequested(
                    crate::action::ConflictResolution::RetrySave,
                ),
                NotoraAction::SaveConflictResolutionRequested(
                    crate::action::ConflictResolution::Cancel,
                ),
            ]
        );
    }

    #[test]
    fn virtual_cards_map_catalog_dtos_with_stable_keys_and_selection() {
        let note_id = NoteId::generate();
        let identity = DocumentIdentity::Note(note_id);
        let mut state = NotoraState::default();
        state.library.selected_card = Some(identity);
        state.library.card_scroll_offset_px = 36.0;
        state.library.card_page = CardPageState::Ready {
            query: CardQuery::from(NavigationScope::WorkspaceRoot),
            cards: vec![CatalogCard {
                note_id,
                relative_path: "notes/roadmap.md".into(),
                kind: DocumentKind::Markdown,
                title: "Roadmap".to_owned(),
                excerpt: "Ship virtual cards".to_owned(),
                modified_nanoseconds: 42,
                starred: true,
                tags: vec!["plan".to_owned()],
            }],
            next_cursor: None,
        };
        let model = NotoraRenderModel::from_state(&state);
        assert_eq!(model.cards.len(), 1);
        assert_eq!(model.cards[0].tag_summary, "★ #plan");

        let mut shell = NotoraShell::new();
        shell.update_model(&model);
        let first_key = shell.card_list.input().cards[0].key;
        assert_eq!(shell.card_list.input().scroll_offset_px, 36.0);
        assert_eq!(shell.card_list.input().cards[0].selection, CardSelection::Selected);
        assert!(!shell.card_list.input().cards[0].closable);
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::VirtualCardList(
                VirtualCardListAction::Selected(first_key),
            )),
            Some(NotoraAction::CardSelected(identity))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::VirtualCardList(
                VirtualCardListAction::Activated(first_key),
            )),
            Some(NotoraAction::CardActivated(identity))
        );

        shell.update_model(&model);
        assert_eq!(shell.card_list.input().cards[0].key, first_key);
    }

    #[test]
    fn external_file_cards_expose_close_actions_without_leaking_domain_state_to_ui() {
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::ExternalFiles;
        let identity = state.external_files.create_untitled(DocumentKind::Markdown);
        let DocumentIdentity::ExternalFile(external_file_id) = identity else {
            panic!("external card fixture must have an external identity");
        };
        let model = NotoraRenderModel::from_state(&state);
        let mut shell = NotoraShell::new();

        shell.update_model(&model);

        let card = &shell.card_list.input().cards[0];
        assert!(card.closable);
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::VirtualCardList(
                VirtualCardListAction::CloseRequested(card.key),
            )),
            Some(NotoraAction::ExternalFileCloseRequested(external_file_id))
        );
    }

    #[test]
    fn title_edit_updates_the_selected_middle_pane_card_before_catalog_refresh() {
        let note_id = NoteId::generate();
        let identity = DocumentIdentity::Note(note_id);
        let mut state = NotoraState::default();
        state.library.selected_card = Some(identity);
        state.library.card_page = CardPageState::Ready {
            query: CardQuery::from(NavigationScope::WorkspaceRoot),
            cards: vec![CatalogCard {
                note_id,
                relative_path: "notes/old-title.md".into(),
                kind: DocumentKind::Markdown,
                title: "旧标题".to_owned(),
                excerpt: "正文摘要".to_owned(),
                modified_nanoseconds: 42,
                starred: false,
                tags: Vec::new(),
            }],
            next_cursor: None,
        };

        let _ = state.reduce(NotoraAction::TitleTextChanged("编辑中的标题".to_owned()));

        let editing_model = NotoraRenderModel::from_state(&state);
        assert_eq!(editing_model.cards[0].title, "编辑中的标题");

        let _ = state.reduce(NotoraAction::TitleCommitRequested("新标题".to_owned()));

        let pending_model = NotoraRenderModel::from_state(&state);
        assert_eq!(pending_model.cards[0].title, "新标题");
    }

    #[test]
    fn moving_pointer_to_other_pane_clears_card_hover() {
        use ui::core::paint::{DrawCmd, DrawList};

        fn paints_card_hover(shell: &NotoraShell, theme: &ui::Theme) -> bool {
            let mut draw_list = DrawList::new();
            let mut paint_context = ui::PaintCtx::new(&mut draw_list, theme, 1.0);
            shell.card_list.paint(&mut paint_context);
            draw_list.cmds.iter().any(|command| {
                matches!(
                    command,
                    DrawCmd::FillRect { color, .. }
                        if *color == theme.palette.sidebar_hover_bg
                )
            })
        }

        let mut shell = NotoraShell::new();
        let theme = ui::theme::test_theme();
        let mut measure = ui::NoopMeasure;
        let mut layout_context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
        shell.navigation_tree_rect = Rect::new(0.0, 0.0, 200.0, 600.0);
        shell.card_content_rect = Rect::new(220.0, 0.0, 300.0, 600.0);
        shell.editor_rect = Rect::new(540.0, 0.0, 600.0, 600.0);
        shell.card_list.set_input(VirtualCardListInput {
            cards: vec![CardInput {
                key: CardKey(1),
                title: "Card".to_owned(),
                excerpt: String::new(),
                timestamp: String::new(),
                icon: None,
                tag_summary: String::new(),
                selection: CardSelection::Unselected,
                closable: false,
            }],
            scroll_offset_px: 0.0,
        });
        shell.card_list.set_rect(shell.card_content_rect, &mut layout_context);
        let card_rect = shell.card_list.layout().card_geometry(0).card_rect;
        let hover_card = Event::MouseMove {
            px: card_rect.x + card_rect.w * 0.5,
            py: card_rect.y + card_rect.h * 0.5,
        };

        for destination in
            [Event::MouseMove { px: 100.0, py: 100.0 }, Event::MouseMove { px: 700.0, py: 100.0 }]
        {
            shell.route_event(&hover_card, FocusTarget::Editor, &theme, 1.0);
            assert!(paints_card_hover(&shell, &theme));

            shell.route_event(&destination, FocusTarget::Editor, &theme, 1.0);
            assert!(!paints_card_hover(&shell, &theme));
        }
    }

    #[test]
    fn workspace_note_toolbar_has_no_note_level_commands() {
        let note_id = NoteId::generate();
        assert!(
            note_toolbar_buttons(&NavigationScope::WorkspaceRoot, Some(note_id), false).is_empty()
        );
    }

    #[test]
    fn tag_scope_has_no_manual_tag_mutation_toolbar_actions() {
        let tag_id = notora_core::TagId::generate();
        let note_id = NoteId::generate();
        let tag_buttons =
            note_toolbar_buttons(&NavigationScope::Tag { tag_id }, Some(note_id), false);
        assert!(tag_buttons.is_empty());
    }

    #[test]
    fn trash_toolbar_ignores_a_selection_that_is_not_in_the_current_card_page() {
        let note_id = NoteId::generate();
        let mut state = NotoraState::default();
        state.library.navigation_scope = NavigationScope::Trash;
        state.library.selected_card = Some(DocumentIdentity::Note(note_id));

        let model = NotoraRenderModel::from_state(&state);
        assert_eq!(model.note_toolbar.len(), 1);
        assert_eq!(
            model.note_toolbar[0].action,
            NotoraAction::TrashOperationRequested(TrashOperation::Empty)
        );
    }

    #[test]
    fn editor_pane_input_matrix_keeps_external_and_trash_metadata_read_only() {
        let note_id = NoteId::generate();
        let external_id = notora_core::ExternalFileId::generate();

        let empty_state = NotoraState::default();
        assert_eq!(editor_pane_input(&empty_state, &[]).mode, EditorPaneMode::Empty);

        let mut note_state = NotoraState::default();
        note_state.library.selected_card = Some(DocumentIdentity::Note(note_id));
        let note_input = editor_pane_input(
            &note_state,
            &[RenderCard {
                identity: DocumentIdentity::Note(note_id),
                title: "路线图".to_owned(),
                excerpt: String::new(),
                timestamp: String::new(),
                icon: None,
                tag_summary: "★ #产品/Notora".to_owned(),
                closable: false,
            }],
        );
        assert_eq!(note_input.mode, EditorPaneMode::WorkspaceNote);
        assert!(note_input.header.title_editable);
        assert!(note_input.header.star_enabled);
        assert_eq!(note_input.tags.chips[0].label, "产品/Notora");

        note_state.library.navigation_scope = NavigationScope::Trash;
        let trash_input = editor_pane_input(&note_state, &[]);
        assert_eq!(trash_input.mode, EditorPaneMode::TrashNote);
        assert!(!trash_input.header.title_editable);
        assert!(!trash_input.tags.enabled);

        let mut external_state = NotoraState::default();
        external_state.library.selected_card = Some(DocumentIdentity::ExternalFile(external_id));
        let external_input = editor_pane_input(&external_state, &[]);
        assert_eq!(external_input.mode, EditorPaneMode::ExternalFile);
        assert!(!external_input.header.title_editable);
        assert_eq!(
            external_input.header.encryption,
            ui::editor_header::EncryptionStatusInput::Hidden
        );
        assert!(!external_input.location.open);
    }

    #[test]
    fn editor_toolbar_commands_follow_the_active_plugin_capabilities() {
        let command_keys = |input: ui::editor_toolbar::EditorToolbarInput| {
            input
                .groups
                .into_iter()
                .flat_map(|group| group.commands)
                .map(|command| command.command_key)
                .collect::<Vec<_>>()
        };
        assert!(
            command_keys(editor_toolbar_input_for_plugin(
                EditorPaneMode::WorkspaceNote,
                ui::plugin::PLUGIN_MARKDOWN_EDITOR,
            ))
            .contains(&"bold".to_owned())
        );
        assert_eq!(
            command_keys(editor_toolbar_input_for_plugin(
                EditorPaneMode::WorkspaceNote,
                ui::plugin::PLUGIN_EDITOR,
            )),
            vec!["undo".to_owned(), "redo".to_owned()]
        );
        assert_eq!(
            command_keys(editor_toolbar_input_for_plugin(
                EditorPaneMode::WorkspaceNote,
                ui::plugin::PLUGIN_MINDMAP,
            )),
            vec![
                "undo".to_owned(),
                "redo".to_owned(),
                "mindmap_style".to_owned(),
                "promote".to_owned(),
                "demote".to_owned(),
            ]
        );
        let mut compact_toolbar = editor_toolbar_input_for_plugin(
            EditorPaneMode::WorkspaceNote,
            ui::plugin::PLUGIN_MARKDOWN_EDITOR,
        );
        add_compact_editor_toolbar_commands(&mut compact_toolbar);
        assert!(command_keys(compact_toolbar).contains(&"delete".to_owned()));
    }

    #[test]
    fn source_toggle_command_stays_visible_and_reflects_the_current_view() {
        let mut visual_toolbar = editor_toolbar_input_for_plugin(
            EditorPaneMode::WorkspaceNote,
            ui::plugin::PLUGIN_MARKDOWN_EDITOR,
        );
        add_source_toggle_command(&mut visual_toolbar, false);
        let source_command = visual_toolbar.groups[0]
            .commands
            .iter()
            .find(|command| command.command_key == "toggle_source")
            .expect("source toggle should be present");
        assert_eq!(source_command.command_key, "toggle_source");
        assert_eq!(source_command.label, "源码");
        assert_eq!(source_command.overflow_priority, 0);
        assert_eq!(visual_toolbar.groups[0].commands[2].command_key, "toggle_source");

        let mut source_toolbar = history_toolbar_input();
        add_source_toggle_command(&mut source_toolbar, true);
        assert_eq!(
            source_toolbar.groups[0]
                .commands
                .iter()
                .find(|command| command.command_key == "toggle_source")
                .expect("visual toggle should be present")
                .label,
            "可视化"
        );
    }

    #[test]
    fn editor_widget_keys_translate_to_typed_note_actions() {
        let note_id = NoteId::generate();
        let tag_id = notora_core::TagId::generate();
        let suggestion_tag_id = notora_core::TagId::generate();
        let mut state = NotoraState::default();
        state.library.selected_card = Some(DocumentIdentity::Note(note_id));
        state.library.selected_document_generation = 4;
        state.library.active_editor_metadata = Some(crate::state::ActiveEditorMetadata {
            identity: DocumentIdentity::Note(note_id),
            selection_generation: 4,
            metadata: notora_core::NoteEditorMetadata {
                note_id,
                created_at: SystemTime::UNIX_EPOCH,
                modified_at: SystemTime::UNIX_EPOCH,
                encryption: notora_core::NoteEncryption::Unencrypted,
                title_initialization: notora_core::TitleInitialization::Independent,
                file_name_binding: notora_core::NoteFileNameBinding::TitleBound {
                    disambiguator: 1,
                },
                title_revision: 0,
            },
            tags: vec![notora_core::TagSummary {
                tag_id, display_name: "产品/Notora".to_owned()
            }],
        });
        state.library.navigation_tree.tags = vec![
            notora_core::TagWithActiveNoteCount {
                tag_id,
                display_name: "产品/Notora".to_owned(),
                active_note_count: 1,
            },
            notora_core::TagWithActiveNoteCount {
                tag_id: suggestion_tag_id,
                display_name: "设计/UI".to_owned(),
                active_note_count: 2,
            },
        ];
        let model = NotoraRenderModel::from_state(&state);
        assert_eq!(
            model.editor_chrome.tags.suggestions,
            vec![ui::tag_editor::TagSuggestionInput {
                option_key: format!("suggestion:{suggestion_tag_id}"),
                label: "设计/UI".to_owned(),
                enabled: true,
            }]
        );
        let mut shell = NotoraShell::new();
        shell.update_model(&model);

        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::tag_editor::TAG_EDITOR_SUBMIT_ID,
                value: TextPayload::Plain("设计/UI".to_owned()),
            })),
            Some(NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTagByName {
                note_id,
                display_name: "设计/UI".to_owned(),
            }))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::FocusRequested {
                id: ui::editor_header::EDITOR_HEADER_TITLE_ID,
            })),
            Some(NotoraAction::FocusRequested(FocusTarget::EditorTitle))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
                value: TextPayload::Plain("mindmap_style".to_owned()),
            })),
            Some(NotoraAction::ToggleMindmapStylePanelRequested)
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::MindmapStylePanel(
                ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".to_owned()),
            )),
            Some(NotoraAction::MindmapStylePanel(
                ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".to_owned()),
            ))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextEdited {
                id: ui::editor_header::EDITOR_HEADER_TITLE_ID,
                value: TextPayload::Plain("新的标题".to_owned()),
            })),
            Some(NotoraAction::TitleTextChanged("新的标题".to_owned()))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::FocusRequested {
                id: ui::tag_editor::TAG_EDITOR_INPUT_ID,
            })),
            Some(NotoraAction::FocusRequested(FocusTarget::EditorTag))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::Activated {
                id: ui::tag_editor::TAG_EDITOR_CANCEL_ID,
            })),
            Some(NotoraAction::FocusRequested(FocusTarget::Editor))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::tag_editor::TAG_EDITOR_SUGGESTION_ID,
                value: TextPayload::Plain(format!("suggestion:{suggestion_tag_id}")),
            })),
            Some(NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTagByName {
                note_id,
                display_name: "设计/UI".to_owned(),
            }))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::tag_editor::TAG_EDITOR_REMOVE_ID,
                value: TextPayload::Plain(tag_id.to_string()),
            })),
            Some(NotoraAction::MetadataMutationRequested(MetadataMutation::DetachTag {
                note_id,
                tag_id,
            }))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::location_picker::LOCATION_PICKER_SELECT_ID,
                value: TextPayload::Plain(EDITOR_ROOT_DIRECTORY_ROW_KEY.to_owned()),
            })),
            Some(NotoraAction::MoveRequested {
                note_id,
                target_directory: std::path::PathBuf::new(),
            })
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
                value: TextPayload::Plain("bold".to_owned()),
            })),
            Some(NotoraAction::SemanticEditRequested(ui::plugin::SemanticEditCommand::ToggleBold,))
        );
        assert_eq!(
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::TextCommitted {
                id: ui::editor_toolbar::EDITOR_TOOLBAR_COMMAND_ID,
                value: TextPayload::Plain("toggle_source".to_owned()),
            })),
            Some(NotoraAction::ToggleSourceViewRequested)
        );
    }
}
