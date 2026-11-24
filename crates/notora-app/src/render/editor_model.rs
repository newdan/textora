//! 编辑器窗格、工具栏及其产品动作的展示输入。

use std::collections::HashMap;
use std::time::SystemTime;

use notora_core::{DocumentIdentity, NavigationScope};

use crate::NotoraState;
use crate::action::{MetadataMutation, NotoraAction};
use crate::editor_pane::{EditorPaneInput, EditorPaneMode};
use crate::state::ActiveEditorMetadata;

use super::DOCUMENT_SEARCH_COMMAND_KEY;
use super::card_model::{RenderCard, format_created_time, format_modified_time};

pub(super) const EDITOR_ROOT_DIRECTORY_ROW_KEY: &str = "root";
const EDITOR_TAG_SUGGESTION_KEY_PREFIX: &str = "suggestion:";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorPaneState {
    #[default]
    Empty,
    Active,
    EncryptedUnlock,
}

pub(crate) fn selected_editor_pane_mode(state: &NotoraState) -> EditorPaneMode {
    let Some(identity) = state.library.selected_card else {
        return EditorPaneMode::Empty;
    };
    match identity {
        DocumentIdentity::ExternalFile(_) => EditorPaneMode::ExternalFile,
        DocumentIdentity::Note(_) if state.library.navigation_scope == NavigationScope::Trash => {
            EditorPaneMode::TrashNote
        }
        DocumentIdentity::Note(_) => EditorPaneMode::WorkspaceNote,
    }
}

pub(super) fn editor_pane_input(state: &NotoraState, cards: &[RenderCard]) -> EditorPaneInput {
    let Some(identity) = state.library.selected_card else {
        return EditorPaneInput::default();
    };
    let selected_card = cards.iter().find(|card| card.identity == identity);
    let mode = selected_editor_pane_mode(state);
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
            starred: false,
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
            enabled: mode == EditorPaneMode::WorkspaceNote,
            ..ui::tag_editor::TagEditorInput::default()
        },
        toolbar: editor_toolbar_input(mode),
    };
    if let Some(snapshot) = metadata_for_selection(state, identity) {
        input.header.starred = snapshot.metadata.starred;
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

pub(super) fn metadata_for_selection(
    state: &NotoraState,
    identity: DocumentIdentity,
) -> Option<&ActiveEditorMetadata> {
    state.library.active_editor_metadata.as_ref().filter(|snapshot| {
        snapshot.identity == identity
            && snapshot.selection_generation == state.library.selected_document_generation
    })
}

pub(super) fn editor_action_maps(
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
    let mut tag_actions: HashMap<String, NotoraAction> =
        metadata_for_selection(state, DocumentIdentity::Note(note_id))
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

const INSPECTOR_PANEL_COMMAND_KEY: &str = "inspector_panel";

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
    inspector_available: bool,
) -> ui::editor_toolbar::EditorToolbarInput {
    if mode != EditorPaneMode::WorkspaceNote {
        return editor_toolbar_input(mode);
    }
    let inspector_command = inspector_available.then_some((INSPECTOR_PANEL_COMMAND_KEY, "主题", 0));
    if plugin_name == ui::plugin::PLUGIN_MINDMAP {
        return mindmap_toolbar_input(inspector_command);
    }
    let mut toolbar = if plugin_name == ui::plugin::PLUGIN_MARKDOWN_EDITOR {
        markdown_toolbar_input()
    } else {
        history_toolbar_input()
    };
    if let Some(command) = inspector_command {
        toolbar.groups.push(toolbar_group("属性", &[command]));
    }
    toolbar
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

pub(crate) fn add_document_search_command(toolbar: &mut ui::editor_toolbar::EditorToolbarInput) {
    if toolbar.groups.is_empty() {
        toolbar.groups.push(toolbar_group("查找", &[]));
    }
    let Some(group) = toolbar.groups.first_mut() else {
        return;
    };
    group.commands.push(ui::editor_toolbar::EditorToolbarCommandInput {
        command_key: DOCUMENT_SEARCH_COMMAND_KEY.to_owned(),
        label: "文内查找".to_owned(),
        enabled: true,
        overflow_priority: 0,
    });
}

pub(super) fn history_toolbar_input() -> ui::editor_toolbar::EditorToolbarInput {
    ui::editor_toolbar::EditorToolbarInput {
        groups: vec![toolbar_group("历史", &[("undo", "撤销", 0), ("redo", "重做", 0)])],
        overflow_open: false,
    }
}

pub(super) fn markdown_toolbar_input() -> ui::editor_toolbar::EditorToolbarInput {
    ui::editor_toolbar::EditorToolbarInput {
        groups: vec![
            toolbar_group("历史", &[("undo", "撤销", 0), ("redo", "重做", 0)]),
            toolbar_group(
                "文字",
                &[
                    ("heading", "标题", 3),
                    ("bold", "粗体", 4),
                    ("italic", "斜体", 5),
                    ("strike", "删除线", 6),
                    ("inline_code", "行内代码", 7),
                ],
            ),
            toolbar_group(
                "列表",
                &[
                    ("unordered_list", "项目列表", 8),
                    ("ordered_list", "编号列表", 9),
                    ("task_list", "任务列表", 10),
                ],
            ),
            toolbar_group(
                "插入",
                &[("quote", "引用", 11), ("code_block", "代码块", 12), ("link", "链接", 13)],
            ),
        ],
        overflow_open: false,
    }
}

pub(super) fn mindmap_toolbar_input(
    inspector_command: Option<(&str, &str, u8)>,
) -> ui::editor_toolbar::EditorToolbarInput {
    let mindmap_commands = inspector_command
        .into_iter()
        .chain([("promote", "提升层级", 3), ("demote", "降低层级", 4)])
        .collect::<Vec<_>>();
    ui::editor_toolbar::EditorToolbarInput {
        groups: vec![
            toolbar_group("历史", &[("undo", "撤销", 0), ("redo", "重做", 0)]),
            toolbar_group("脑图", &mindmap_commands),
            toolbar_group(
                "视图",
                &[
                    ("canvas_zoom_out", "缩小", 1),
                    ("canvas_zoom_in", "放大", 1),
                    ("canvas_fit", "适应窗口", 0),
                ],
            ),
        ],
        overflow_open: false,
    }
}

fn toolbar_group(
    label: &str,
    commands: &[(&str, &str, u8)],
) -> ui::editor_toolbar::EditorToolbarGroupInput {
    ui::editor_toolbar::EditorToolbarGroupInput {
        label: label.to_owned(),
        commands: commands
            .iter()
            .map(|(command_key, label, priority)| ui::editor_toolbar::EditorToolbarCommandInput {
                command_key: (*command_key).to_owned(),
                label: (*label).to_owned(),
                enabled: true,
                overflow_priority: *priority,
            })
            .collect(),
    }
}

pub(super) fn editor_command_actions() -> HashMap<String, NotoraAction> {
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
        (DOCUMENT_SEARCH_COMMAND_KEY.to_owned(), NotoraAction::DocumentSearchRequested),
        ("toggle_source".to_owned(), NotoraAction::ToggleSourceViewRequested),
        (INSPECTOR_PANEL_COMMAND_KEY.to_owned(), NotoraAction::ToggleInspectorPanelRequested),
    ])
    .collect()
}
