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
use ui::popup_menu::{PopupMenuAction, PopupMenuWidget, PopupOutcome};
use ui::sidebar::NewDocumentKind;
use ui::split_button::{SplitButtonInput, SplitButtonWidget};
use ui::splitter::{SplitterAction, SplitterInput, SplitterWidget};
use ui::status_state::{StatusStateInput, StatusStateKind, StatusStateWidget};
use ui::text_box::TextBox;
use ui::tree_list::{
    TreeListAction, TreeListInput, TreeListWidget, TreeRowExpansion, TreeRowInput, TreeRowKey,
    TreeRowSelection,
};
use ui::virtual_card_list::{
    CardInput, CardKey, CardSelection, VirtualCardListAction, VirtualCardListInput,
    VirtualCardListWidget,
};
use ui::{Event, EventCtx, Rect, Widget};

use crate::action::{ConflictResolution, MetadataMutation, NotoraAction, TrashOperation};
use crate::editor_pane::{EditorPaneChrome, EditorPaneInput, EditorPaneMode, EditorPaneRects};
use crate::external_files::ExternalFileSession;
use crate::settings::ProductSettings;
use crate::settings_overlay::{SettingsOverlay, SettingsOverlayAction, SettingsOverlayInput};
use crate::shell::layout::ShellLayout;
use crate::state::CardPageState;
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
    pub navigation_expansion_paths: HashMap<TreeRowKey, std::path::PathBuf>,
    pub cards: Vec<RenderCard>,
    pub selected_card: Option<DocumentIdentity>,
    pub card_scroll_offset_px: f32,
    pub card_list_title: String,
    pub card_empty_state: StatusStateInput,
    pub show_settings_overlay: bool,
    pub settings_overlay: SettingsOverlayInput,
    pub confirmation: Option<ConfirmationOverlayInput>,
    pub show_new_document_menu: bool,
    pub show_tooltip: bool,
    pub new_note_control: NewNoteControlState,
    pub note_toolbar: Vec<NoteToolbarButtonInput>,
    pub save_conflict: Option<SaveConflictOverlayInput>,
    pub editor_chrome: EditorPaneInput,
    pub editor_note_id: Option<NoteId>,
    pub editor_location_actions: HashMap<String, NotoraAction>,
    pub editor_tag_actions: HashMap<String, NotoraAction>,
    pub editor_command_actions: HashMap<String, NotoraAction>,
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
        let mut navigation_expansion_paths = HashMap::new();
        push_navigation_row(
            &mut navigation_rows,
            &mut navigation_actions,
            WORKSPACE_NAVIGATION_KEY,
            "工作区".to_owned(),
            "folder-open",
            0,
            None,
            NavigationScope::WorkspaceRoot,
            selected_scope,
        );
        for (index, directory) in state.library.navigation_tree.directories.iter().enumerate() {
            if !directory_is_visible(directory, &state.library.navigation_tree.expanded_directories)
            {
                continue;
            }
            let key = DIRECTORY_NAVIGATION_KEY_START + index as u64;
            let label = directory
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| directory.to_string_lossy().to_string());
            let depth = directory.components().count().saturating_sub(1);
            let row_key = TreeRowKey(key);
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
        }
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
            navigation_expansion_paths,
            cards,
            selected_card: state.library.selected_card,
            card_scroll_offset_px: state.library.card_scroll_offset_px,
            card_list_title: card_list_title(selected_scope).to_owned(),
            card_empty_state: card_empty_state_input(state),
            show_settings_overlay: state.layout.overlay == OverlayState::Settings,
            settings_overlay: SettingsOverlayInput::from_product_settings(product_settings),
            confirmation: confirmation_overlay_input(state.layout.overlay),
            show_new_document_menu: state.layout.overlay == OverlayState::NewDocumentMenu,
            show_tooltip: false,
            new_note_control: new_note_control_state(selected_scope, state.workspace_root),
            note_toolbar: note_toolbar_buttons(selected_scope, selected_note_id),
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
    let title = selected_card.map(|card| card.title.clone()).unwrap_or_else(|| match mode {
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
    .collect()
}

fn note_toolbar_buttons(
    scope: &NavigationScope,
    selected_note_id: Option<NoteId>,
) -> Vec<NoteToolbarButtonInput> {
    if *scope == NavigationScope::ExternalFiles {
        return vec![NoteToolbarButtonInput {
            label: "打开".to_owned(),
            action: NotoraAction::OpenExternalFileDialogRequested,
        }];
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
        NavigationScope::Search { .. } | NavigationScope::Trash => NewNoteControlState::Hidden,
        NavigationScope::WorkspaceRoot
        | NavigationScope::Directory { .. }
        | NavigationScope::Starred
        | NavigationScope::Tag { .. }
        | NavigationScope::ExternalFiles => match workspace_root {
            WorkspaceRootState::Missing => NewNoteControlState::Disabled,
            WorkspaceRootState::Active => NewNoteControlState::Enabled,
        },
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
    settings_overlay: SettingsOverlay,
    settings_overlay_open: bool,
    navigation_actions: HashMap<TreeRowKey, NotoraAction>,
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
            navigation_tree: TreeListWidget::new(),
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
            settings_overlay: SettingsOverlay::new(),
            settings_overlay_open: false,
            navigation_actions: HashMap::new(),
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
        let focused_text_input =
            if focus_target == FocusTarget::EditorTag && self.editor_pane.tag_editor_is_active() {
                Some(FocusTarget::EditorTag)
            } else {
                match focus_target {
                    FocusTarget::NavigationSearch | FocusTarget::EditorTitle => Some(focus_target),
                    _ => None,
                }
            };
        if self.focused_text_input != focused_text_input {
            self.focused_text_input = focused_text_input;
            self.text_cursor_visible = true;
            self.next_text_cursor_blink_at =
                focused_text_input.map(|_| now + TEXT_CURSOR_BLINK_INTERVAL);
        }
        self.search_box.set_focus(focus_target == FocusTarget::NavigationSearch);
        self.editor_pane.set_title_focus(focus_target == FocusTarget::EditorTitle);
        self.apply_text_cursor_visibility();
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

    #[cfg(test)]
    pub(crate) fn search_box_is_focused(&self) -> bool {
        self.search_box.is_focused()
    }

    fn apply_text_cursor_visibility(&mut self) {
        self.search_box.set_blink(self.text_cursor_visible);
        self.editor_pane.set_title_blink_visible(self.text_cursor_visible);
        self.editor_pane.set_tag_blink_visible(self.text_cursor_visible);
    }

    pub fn update_model(&mut self, model: &NotoraRenderModel) {
        self.navigation_actions.clone_from(&model.navigation_actions);
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
                }
            })
            .collect();
        self.navigation_tree.set_input(TreeListInput {
            rows: model.navigation_rows.clone(),
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
        frame.with_layout_context(|context| {
            self.editor_pane.set_rects(
                EditorPaneRects {
                    header: layout.editor_header_rect,
                    toolbar: layout.editor_toolbar_rect,
                    body: layout.editor_body_rect,
                },
                context,
            );
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
            self.new_document_menu_rect = menu.menu_rect;
            self.new_document_menu = Some(PopupMenuWidget::new(menu));
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
        if model.show_tooltip {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.tooltip_rect,
                    context.theme.application_theme().overlay_surface,
                    4.0 * context.dpi,
                );
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
                .map(NotoraAction::NavigationExpansionToggled),
            WidgetAction::VirtualCardList(VirtualCardListAction::Selected(key)) => {
                self.card_identities.get(key).copied().map(NotoraAction::CardSelected)
            }
            WidgetAction::VirtualCardList(VirtualCardListAction::Activated(key)) => {
                self.card_identities.get(key).copied().map(NotoraAction::CardActivated)
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
        let mut event_context = EventCtx { theme, dpi, cursor_hint: None };
        self.synchronize_focus(focus_target, Instant::now());
        let mut route = self.route_event_with_context(event, focus_target, &mut event_context);
        route.cursor_hint = event_context.cursor_hint;
        route
    }

    fn route_event_with_context(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        event_context: &mut EventCtx,
    ) -> NotoraEventRoute {
        if self.settings_overlay_open() {
            let action = self
                .settings_overlay
                .route_event(event, event_context)
                .map(settings_overlay_action_to_notora_action);
            return NotoraEventRoute::consumed(action);
        }
        if self.save_conflict_actions.is_some() {
            let action = self.route_save_conflict_event(event);
            return NotoraEventRoute::consumed(action);
        }
        if let Some(action) = self.confirmation_overlay_action(event) {
            return NotoraEventRoute::consumed(Some(action));
        }
        if let Some(action) =
            compact_layout_action(event, self.compact_navigation_rect, self.compact_back_rect)
        {
            return NotoraEventRoute::consumed(Some(action));
        }
        if self.new_document_menu_open {
            let local_event = translate_event(
                event,
                self.new_document_menu_rect.x,
                self.new_document_menu_rect.y,
            );
            if let Some(widget_action) = self
                .new_document_menu
                .as_mut()
                .and_then(|menu| menu.on_event(&local_event, event_context))
            {
                let action = new_document_menu_action(&widget_action);
                return NotoraEventRoute::consumed(action);
            }
            return NotoraEventRoute::consumed(None);
        }
        if let Some(route) = self.route_canvas_scrollbars_event(event, event_context) {
            return route;
        }
        if let Some(widget_action) = self.editor_pane.route_event(event, event_context) {
            let action = self.translate_widget_action(&widget_action);
            return NotoraEventRoute::consumed(action);
        }
        if let Some(action) = note_toolbar_action(event, &self.note_toolbar_buttons) {
            return NotoraEventRoute::consumed(Some(action));
        }
        if matches!(
            event,
            Event::MouseMove { .. } | Event::MouseDown { .. } | Event::MouseUp { .. }
        ) && let Some(widget_action) = self.new_note_button.on_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return NotoraEventRoute::consumed(action);
        }
        if let Some(action) = self.route_splitter_event(event, event_context) {
            return NotoraEventRoute::consumed(action);
        }
        if let Some(action) = settings_button_action(event, self.settings_rect) {
            return NotoraEventRoute::consumed(Some(action));
        }
        if self.card_empty_state_visible
            && let Some(widget_action) = self.card_empty_state.on_event(event, event_context)
        {
            let action = self.translate_widget_action(&widget_action);
            return NotoraEventRoute::consumed(action);
        }
        let widget_action = match pointer_target(event, self) {
            Some(FocusTarget::NavigationSearch) => self.search_box.on_event(event, event_context),
            Some(FocusTarget::NavigationTree) => {
                self.navigation_tree.on_event(event, event_context)
            }
            Some(FocusTarget::CardList) => self.card_list.on_event(event, event_context),
            Some(
                FocusTarget::Editor
                | FocusTarget::EditorTitle
                | FocusTarget::EditorTag
                | FocusTarget::Overlay,
            ) => None,
            None => match focus_target {
                FocusTarget::NavigationSearch => self.search_box.on_event(event, event_context),
                FocusTarget::NavigationTree => self.navigation_tree.on_event(event, event_context),
                FocusTarget::CardList => self.card_list.on_event(event, event_context),
                FocusTarget::Editor
                | FocusTarget::EditorTitle
                | FocusTarget::EditorTag
                | FocusTarget::Overlay => {
                    return NotoraEventRoute::ignored();
                }
            },
        };
        let action = widget_action
            .as_ref()
            .and_then(|widget_action| self.translate_widget_action(widget_action));
        if action.is_some() {
            return NotoraEventRoute::consumed(action);
        }
        if is_left_mouse_down(event)
            && let Some(focus_target) = pointer_target(event, self)
        {
            let focus_action = NotoraAction::FocusRequested(focus_target);
            if focus_target == FocusTarget::Editor {
                return NotoraEventRoute::passthrough(focus_action);
            }
            return NotoraEventRoute::consumed(Some(focus_action));
        }
        if widget_action.is_some() {
            return NotoraEventRoute::consumed(None);
        }
        NotoraEventRoute::ignored()
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
    match &state.library.card_page {
        CardPageState::Ready { cards, .. }
        | CardPageState::LoadingNextPage { cards, .. }
        | CardPageState::Refreshing { cards, .. }
        | CardPageState::Failed { cards, .. } => cards.iter().map(render_catalog_card).collect(),
        CardPageState::Idle
        | CardPageState::LoadingInitial { .. }
        | CardPageState::Empty { .. } => Vec::new(),
    }
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
        },
        ExternalFileSession::Untitled { kind, .. } => RenderCard {
            identity: session.identity(),
            title: "未命名".to_owned(),
            excerpt: "尚未保存的外部文件".to_owned(),
            timestamp: "外部文件".to_owned(),
            icon: Some(document_icon(*kind).to_owned()),
            tag_summary: String::new(),
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
            icon: Some("file-warning".to_owned()),
            tag_summary: String::new(),
        },
    }
}

fn document_icon(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text => "file-text",
        DocumentKind::Markdown => "file-code-2",
        DocumentKind::Mindmap => "git-fork",
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
        | Event::ImeDisable => None,
    }
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

fn paint_at(context: &mut ui::PaintCtx<'_>, rect: Rect, paint: impl FnOnce(&mut ui::PaintCtx<'_>)) {
    let saved_offset = context.list.offset;
    context.list.offset = (saved_offset.0 + rect.x, saved_offset.1 + rect.y);
    paint(context);
    context.list.offset = saved_offset;
}

fn translate_event(event: &Event, offset_x: f32, offset_y: f32) -> Event {
    match event {
        Event::MouseMove { px, py } => Event::MouseMove { px: *px - offset_x, py: *py - offset_y },
        Event::MouseDown { px, py, button } => {
            Event::MouseDown { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
        Event::MouseUp { px, py, button } => {
            Event::MouseUp { px: *px - offset_x, py: *py - offset_y, button: *button }
        }
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
    fn new_note_control_is_hidden_for_search_and_trash() {
        for scope in
            [NavigationScope::Search { query: "roadmap".to_owned() }, NavigationScope::Trash]
        {
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
        let mut state = NotoraState::default();
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
        assert_eq!(model.navigation_rows[2].depth, 1);
        assert_eq!(model.navigation_rows[4].badge, Some(2));
        assert!(matches!(
            model.navigation_actions.get(&model.navigation_rows[4].key),
            Some(NotoraAction::NavigationSelected(NavigationScope::Tag { tag_id: selected_tag_id }))
                if *selected_tag_id == tag_id
        ));
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
    fn workspace_note_toolbar_has_no_note_level_commands() {
        let note_id = NoteId::generate();
        assert!(note_toolbar_buttons(&NavigationScope::WorkspaceRoot, Some(note_id)).is_empty());
    }

    #[test]
    fn tag_scope_has_no_manual_tag_mutation_toolbar_actions() {
        let tag_id = notora_core::TagId::generate();
        let note_id = NoteId::generate();
        let tag_buttons = note_toolbar_buttons(&NavigationScope::Tag { tag_id }, Some(note_id));
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
            vec!["undo".to_owned(), "redo".to_owned(), "promote".to_owned(), "demote".to_owned()]
        );
        let mut compact_toolbar = editor_toolbar_input_for_plugin(
            EditorPaneMode::WorkspaceNote,
            ui::plugin::PLUGIN_MARKDOWN_EDITOR,
        );
        add_compact_editor_toolbar_commands(&mut compact_toolbar);
        assert!(command_keys(compact_toolbar).contains(&"delete".to_owned()));
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
            shell.translate_widget_action(&WidgetAction::Control(ControlAction::FocusRequested {
                id: ui::tag_editor::TAG_EDITOR_INPUT_ID,
            })),
            Some(NotoraAction::FocusRequested(FocusTarget::EditorTag))
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
    }
}
