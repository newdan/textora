use std::collections::HashMap;

use appkit_shell::editor_runtime::{EditorFrame, RenderError};
use notora_core::{DocumentIdentity, DocumentKind, NavigationScope, NoteId};
use ui::core::WidgetAction;
use ui::core::widget::{ControlAction, TextPayload, WidgetId};
use ui::icon::draw_icon;
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

use crate::action::{
    ConflictResolution, MetadataMutation, NotoraAction, TagEditorMode, TrashOperation,
};
use crate::external_files::ExternalFileSession;
use crate::settings::ProductSettings;
use crate::settings_overlay::{SettingsOverlay, SettingsOverlayAction, SettingsOverlayInput};
use crate::shell::layout::ShellLayout;
use crate::state::CardPageState;
use crate::{FocusTarget, NotoraState, OverlayState, Pane, ResponsiveLayoutMode};

const GLOBAL_SEARCH_BOX_ID: WidgetId = WidgetId(9_000);
const SETTINGS_BUTTON_ID: WidgetId = WidgetId(9_001);
const NEW_NOTE_BUTTON_ID: WidgetId = WidgetId(9_002);
const NEW_NOTE_MENU_BUTTON_ID: WidgetId = WidgetId(9_003);
const NOTE_TOOL_BUTTON_WIDTH_LOGICAL: f32 = 64.0;
const NOTE_TOOL_BUTTON_HEIGHT_LOGICAL: f32 = 28.0;
const NOTE_TOOL_BUTTON_GAP_LOGICAL: f32 = 6.0;
const SEARCH_BAR_HEIGHT_LOGICAL: f32 = 32.0;
const SEARCH_ICON_AREA_WIDTH_LOGICAL: f32 = 32.0;
const SHELL_PADDING_LOGICAL: f32 = 12.0;
const SIDEBAR_CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
const SIDEBAR_ICON_SIZE_LOGICAL: f32 = 16.0;
const SIDEBAR_LABEL_FONT_SIZE_LOGICAL: f32 = 15.0;
const CARD_LOAD_MORE_THRESHOLD_LOGICAL: f32 = 160.0;
const NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL: f32 = 34.0;
const COMPACT_NAVIGATION_BUTTON_WIDTH_LOGICAL: f32 = 72.0;
const COMPACT_BACK_BUTTON_WIDTH_LOGICAL: f32 = 64.0;
const CONFIRMATION_PANEL_WIDTH_LOGICAL: f32 = 360.0;
const CONFIRMATION_PANEL_HEIGHT_LOGICAL: f32 = 160.0;
const CONFIRMATION_BUTTON_WIDTH_LOGICAL: f32 = 88.0;
const CONFIRMATION_BUTTON_HEIGHT_LOGICAL: f32 = 32.0;
const TAG_EDITOR_PANEL_WIDTH_LOGICAL: f32 = 360.0;
const TAG_EDITOR_PANEL_HEIGHT_LOGICAL: f32 = 176.0;
const TAG_EDITOR_TEXT_BOX_HEIGHT_LOGICAL: f32 = 32.0;
const TAG_EDITOR_TEXT_BOX_ID: WidgetId = WidgetId(9_004);
const SAVE_CONFLICT_PANEL_WIDTH_LOGICAL: f32 = 440.0;
const SAVE_CONFLICT_PANEL_HEIGHT_LOGICAL: f32 = 196.0;
const WORKSPACE_NAVIGATION_KEY: u64 = 1;
const STARRED_NAVIGATION_KEY: u64 = 2;
const TRASH_NAVIGATION_KEY: u64 = 3;
const EXTERNAL_FILES_NAVIGATION_KEY: u64 = 4;
const DIRECTORY_NAVIGATION_KEY_START: u64 = 100;
const TAG_NAVIGATION_KEY_START: u64 = 10_000;

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

/// 标签名称编辑弹层的纯输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEditorOverlayInput {
    pub title: String,
    pub display_name: String,
}

/// 保存竞态发生后展示的四路显式决策；按钮只保存产品 action，不保存 runtime tab。
#[derive(Clone, Debug, PartialEq)]
pub struct SaveConflictOverlayInput {
    pub identity: DocumentIdentity,
    pub content_revision: u64,
    pub actions: [NotoraAction; 4],
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
    pub show_settings_overlay: bool,
    pub settings_overlay: SettingsOverlayInput,
    pub confirmation: Option<ConfirmationOverlayInput>,
    pub show_menu: bool,
    pub show_tooltip: bool,
    pub can_create_note: bool,
    pub note_toolbar: Vec<NoteToolbarButtonInput>,
    pub tag_editor: Option<TagEditorOverlayInput>,
    pub save_conflict: Option<SaveConflictOverlayInput>,
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
            "Workspace".to_owned(),
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
            "Starred".to_owned(),
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
            "Trash".to_owned(),
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
            "Files".to_owned(),
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
        Self {
            search_query,
            navigation_rows,
            navigation_actions,
            navigation_expansion_paths,
            cards,
            selected_card: state.library.selected_card,
            card_scroll_offset_px: state.library.card_scroll_offset_px,
            card_list_title: card_list_title(selected_scope).to_owned(),
            show_settings_overlay: state.layout.overlay == OverlayState::Settings,
            settings_overlay: SettingsOverlayInput::from_product_settings(product_settings),
            confirmation: confirmation_overlay_input(state.layout.overlay),
            show_menu: state.layout.overlay == OverlayState::NewDocumentMenu,
            show_tooltip: false,
            can_create_note: *selected_scope != NavigationScope::Trash,
            note_toolbar: note_toolbar_buttons(selected_scope, selected_note_id),
            tag_editor: state.library.tag_editor.as_ref().map(tag_editor_input),
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
        }
    }
}

fn note_toolbar_buttons(
    scope: &NavigationScope,
    selected_note_id: Option<NoteId>,
) -> Vec<NoteToolbarButtonInput> {
    let selected_tag_id = match scope {
        NavigationScope::Tag { tag_id } => Some(*tag_id),
        _ => None,
    };
    if *scope == NavigationScope::Trash {
        let Some(note_id) = selected_note_id else {
            return vec![NoteToolbarButtonInput {
                label: "Empty".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::Empty),
            }];
        };
        return vec![
            NoteToolbarButtonInput {
                label: "Restore".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::Restore { note_id }),
            },
            NoteToolbarButtonInput {
                label: "Delete".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::PermanentlyDelete {
                    note_id,
                }),
            },
        ];
    }
    if let Some(tag_id) = selected_tag_id {
        let mut buttons = vec![
            NoteToolbarButtonInput {
                label: "Rename".to_owned(),
                action: NotoraAction::TagEditorRequested(TagEditorMode::Rename { tag_id }),
            },
            NoteToolbarButtonInput {
                label: "Delete".to_owned(),
                action: NotoraAction::TagDeletionRequested(tag_id),
            },
        ];
        if let Some(note_id) = selected_note_id {
            buttons.extend([
                NoteToolbarButtonInput {
                    label: "Add".to_owned(),
                    action: NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTag {
                        note_id,
                        tag_id,
                    }),
                },
                NoteToolbarButtonInput {
                    label: "Remove".to_owned(),
                    action: NotoraAction::MetadataMutationRequested(MetadataMutation::DetachTag {
                        note_id,
                        tag_id,
                    }),
                },
            ]);
        }
        return buttons;
    }
    let Some(note_id) = selected_note_id else {
        return vec![NoteToolbarButtonInput {
            label: "+Tag".to_owned(),
            action: NotoraAction::TagEditorRequested(TagEditorMode::Create),
        }];
    };
    vec![
        NoteToolbarButtonInput {
            label: "+Tag".to_owned(),
            action: NotoraAction::TagEditorRequested(TagEditorMode::Create),
        },
        NoteToolbarButtonInput {
            label: "Rename".to_owned(),
            action: NotoraAction::RenameDialogRequested(note_id),
        },
        NoteToolbarButtonInput {
            label: "Move".to_owned(),
            action: NotoraAction::MoveDialogRequested(note_id),
        },
        NoteToolbarButtonInput {
            label: "Star".to_owned(),
            action: NotoraAction::MetadataMutationRequested(MetadataMutation::ToggleStar {
                note_id,
            }),
        },
        NoteToolbarButtonInput {
            label: "Trash".to_owned(),
            action: NotoraAction::TrashOperationRequested(TrashOperation::MoveToTrash { note_id }),
        },
    ]
}

fn tag_editor_input(editor: &crate::state::TagEditorState) -> TagEditorOverlayInput {
    let title = match editor.mode {
        TagEditorMode::Create => "Create tag",
        TagEditorMode::Rename { .. } => "Rename tag",
    };
    TagEditorOverlayInput { title: title.to_owned(), display_name: editor.display_name.clone() }
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

/// 三栏静态壳；仅持有通用 widget 与当帧键到产品动作的映射。
pub struct NotoraShell {
    search_box: TextBox,
    navigation_tree: TreeListWidget,
    card_list: VirtualCardListWidget,
    card_empty_state: StatusStateWidget,
    editor_empty_state: StatusStateWidget,
    navigation_splitter: SplitterWidget,
    card_list_splitter: SplitterWidget,
    new_note_button: SplitButtonWidget,
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
    settings_rect: Rect,
    new_document_menu_rect: Rect,
    new_document_menu_open: bool,
    note_toolbar_buttons: Vec<RenderedToolbarButton>,
    tag_editor_box: TextBox,
    tag_editor_open: bool,
    tag_editor_panel_rect: Rect,
    tag_editor_confirm_rect: Rect,
    tag_editor_cancel_rect: Rect,
    compact_navigation_rect: Rect,
    compact_back_rect: Rect,
    confirmation_panel_rect: Rect,
    confirmation_confirm_rect: Rect,
    confirmation_cancel_rect: Rect,
    confirmation_action: Option<NotoraAction>,
    save_conflict_panel_rect: Rect,
    save_conflict_button_rects: [Rect; 4],
    save_conflict_actions: Option<[NotoraAction; 4]>,
}

impl Default for NotoraShell {
    fn default() -> Self {
        Self::new()
    }
}

impl NotoraShell {
    pub fn new() -> Self {
        let mut search_box = TextBox::with_id(GLOBAL_SEARCH_BOX_ID);
        search_box.set_placeholder("Search notes...");
        search_box.set_max_len_bytes(2_048);
        search_box.set_leading_content_inset_logical(SEARCH_ICON_AREA_WIDTH_LOGICAL);
        let mut new_note_button = SplitButtonWidget::new();
        new_note_button.set_action_ids(NEW_NOTE_BUTTON_ID, NEW_NOTE_MENU_BUTTON_ID);
        let mut tag_editor_box = TextBox::with_id(TAG_EDITOR_TEXT_BOX_ID);
        tag_editor_box.set_placeholder("Tag name");
        tag_editor_box.set_max_len_bytes(512);
        Self {
            search_box,
            navigation_tree: TreeListWidget::new(),
            card_list: VirtualCardListWidget::new(),
            card_empty_state: StatusStateWidget::new(),
            editor_empty_state: StatusStateWidget::new(),
            navigation_splitter: SplitterWidget::new(),
            card_list_splitter: SplitterWidget::new(),
            new_note_button,
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
            settings_rect: Rect::ZERO,
            new_document_menu_rect: Rect::ZERO,
            new_document_menu_open: false,
            note_toolbar_buttons: Vec::new(),
            tag_editor_box,
            tag_editor_open: false,
            tag_editor_panel_rect: Rect::ZERO,
            tag_editor_confirm_rect: Rect::ZERO,
            tag_editor_cancel_rect: Rect::ZERO,
            compact_navigation_rect: Rect::ZERO,
            compact_back_rect: Rect::ZERO,
            confirmation_panel_rect: Rect::ZERO,
            confirmation_confirm_rect: Rect::ZERO,
            confirmation_cancel_rect: Rect::ZERO,
            confirmation_action: None,
            save_conflict_panel_rect: Rect::ZERO,
            save_conflict_button_rects: [Rect::ZERO; 4],
            save_conflict_actions: None,
        }
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
            label: "New note".to_owned(),
            enabled: model.can_create_note,
        });
        self.settings_overlay.set_input(model.settings_overlay.clone());
        self.settings_overlay_open = model.show_settings_overlay;
        self.confirmation_action =
            model.confirmation.as_ref().map(|input| input.confirm_action.clone());
        self.new_document_menu_open = model.show_menu;
        self.tag_editor_open = model.tag_editor.is_some();
        self.save_conflict_actions =
            model.save_conflict.as_ref().map(|conflict| conflict.actions.clone());
        if let Some(editor) = &model.tag_editor {
            self.tag_editor_box.sync_text(&editor.display_name);
        }
        self.card_empty_state.set_input(StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "No notes here".to_owned(),
            description: "Create a note or choose another location.".to_owned(),
            icon: Some("notebook-pen".to_owned()),
            action_label: None,
            action_id: None,
        });
        self.editor_empty_state.set_input(StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "Select a note".to_owned(),
            description: "Its editor will appear here.".to_owned(),
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
        let tree_rect = Rect::new(
            layout.navigation_rect.x + padding,
            search_rect.bottom() + padding,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            (layout.navigation_rect.bottom() - search_rect.bottom() - padding * 3.0).max(0.0),
        );
        self.navigation_tree_rect = tree_rect;
        let settings_rect = Rect::new(
            layout.navigation_rect.x + padding,
            layout.navigation_rect.bottom() - (SIDEBAR_CONTROL_HEIGHT_LOGICAL + 10.0) * dpi,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            SIDEBAR_CONTROL_HEIGHT_LOGICAL * dpi,
        );
        self.search_rect = search_rect;
        self.settings_rect = settings_rect;
        let new_note_rect = Rect::new(
            settings_rect.x,
            settings_rect.y - (SIDEBAR_CONTROL_HEIGHT_LOGICAL + 8.0) * dpi,
            settings_rect.w,
            SIDEBAR_CONTROL_HEIGHT_LOGICAL * dpi,
        );
        self.new_document_menu_rect = Rect::new(
            new_note_rect.x,
            new_note_rect.y - (NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL * 3.0 + 4.0) * dpi,
            new_note_rect.w,
            NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL * 3.0 * dpi,
        );
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
            layout.card_list_rect.y + 44.0 * dpi,
            (layout.card_list_rect.w - padding * 2.0).max(0.0),
            (layout.card_list_rect.h - 56.0 * dpi).max(0.0),
        );
        self.card_content_rect = card_content_rect;
        let tool_button_width = NOTE_TOOL_BUTTON_WIDTH_LOGICAL * dpi;
        let tool_button_height = NOTE_TOOL_BUTTON_HEIGHT_LOGICAL * dpi;
        self.note_toolbar_buttons = layout_note_toolbar(
            layout.card_list_rect,
            padding,
            tool_button_width,
            tool_button_height,
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
        self.layout_tag_editor_overlay(layout.overlay_rect, dpi, model.tag_editor.is_some());
        self.layout_save_conflict_overlay(layout.overlay_rect, dpi, model.save_conflict.is_some());
        frame.with_layout_context(|context| {
            self.search_box.set_rect(search_rect, context);
            self.navigation_tree.set_rect(tree_rect, context);
            self.card_list.set_rect(card_content_rect, context);
            self.card_empty_state.set_rect(card_content_rect, context);
            self.editor_empty_state.set_rect(layout.editor_rect, context);
            self.navigation_splitter.set_rect(layout.navigation_splitter_rect, context);
            self.card_list_splitter.set_rect(layout.card_list_splitter_rect, context);
            self.new_note_button.set_rect(new_note_rect, context);
            if model.show_settings_overlay {
                self.settings_overlay.set_rect(layout.overlay_rect, context);
            }
            if model.tag_editor.is_some() {
                self.tag_editor_box
                    .set_rect(tag_editor_text_box_rect(self.tag_editor_panel_rect, dpi), context);
            }
        });
        frame.with_paint_context(|context| {
            context.list.fill(layout.navigation_rect, context.theme.palette.bg_surface);
            context.list.fill(layout.card_list_rect, context.theme.palette.bg_base);
            context.list.fill(layout.editor_rect, context.theme.editor.background);
            self.search_box.paint(context);
            let search_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
            draw_icon(
                context.list,
                "search",
                search_rect.x + (search_icon_area_width - search_icon_size) * 0.5,
                search_rect.y + (search_rect.h - search_icon_size) * 0.5,
                search_icon_size,
                context.theme.palette.text_muted,
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
                context.theme.palette.text_muted,
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
                context.theme.palette.text_muted,
                "Settings",
            );
            context.text(
                layout.card_list_rect.x + padding,
                layout.card_list_rect.y + 28.0 * context.dpi,
                16.0 * context.dpi,
                context.theme.palette.text_main,
                &model.card_list_title,
            );
            if self.compact_navigation_rect != Rect::ZERO {
                paint_note_tool_button(context, self.compact_navigation_rect, "Library");
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
        frame.paint_editor_with(layout.editor_rect, |context| {
            self.editor_empty_state.paint(context)
        })?;
        if layout.responsive_mode != ResponsiveLayoutMode::ThreePane
            && layout.navigation_rect != Rect::ZERO
        {
            frame.with_paint_context(|context| self.paint_compact_navigation_overlay(context));
        }
        if self.compact_back_rect != Rect::ZERO {
            frame.with_paint_context(|context| {
                paint_note_tool_button(context, self.compact_back_rect, "Back");
            });
        }
        if model.show_settings_overlay {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(layout.overlay_rect, [0.0, 0.0, 0.0, 0.45], 0.0);
                self.settings_overlay.paint(context);
            });
        }
        if let Some(confirmation) = &model.confirmation {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(layout.overlay_rect, [0.0, 0.0, 0.0, 0.45], 0.0);
                context.list.fill_rounded(
                    self.confirmation_panel_rect,
                    context.theme.palette.bg_elevated,
                    10.0 * context.dpi,
                );
                context.text(
                    self.confirmation_panel_rect.x + 20.0 * context.dpi,
                    self.confirmation_panel_rect.y + 38.0 * context.dpi,
                    17.0 * context.dpi,
                    context.theme.palette.text_main,
                    &confirmation.title,
                );
                context.text(
                    self.confirmation_panel_rect.x + 20.0 * context.dpi,
                    self.confirmation_panel_rect.y + 72.0 * context.dpi,
                    13.0 * context.dpi,
                    context.theme.palette.text_muted,
                    &confirmation.description,
                );
                paint_note_tool_button(context, self.confirmation_cancel_rect, "Cancel");
                paint_note_tool_button(
                    context,
                    self.confirmation_confirm_rect,
                    &confirmation.confirm_label,
                );
            });
        }
        if let Some(editor) = &model.tag_editor {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(layout.overlay_rect, [0.0, 0.0, 0.0, 0.45], 0.0);
                self.paint_tag_editor_overlay(context, editor);
            });
        }
        if model.save_conflict.is_some() {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(layout.overlay_rect, [0.0, 0.0, 0.0, 0.45], 0.0);
                self.paint_save_conflict_overlay(context);
            });
        }
        if model.show_menu {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    self.new_document_menu_rect,
                    context.theme.palette.bg_elevated,
                    6.0 * context.dpi,
                );
                for (index, label) in ["Markdown", "Text", "Mind map"].iter().enumerate() {
                    let item_top = self.new_document_menu_rect.y
                        + index as f32 * NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL * context.dpi;
                    context.text(
                        self.new_document_menu_rect.x + 10.0 * context.dpi,
                        item_top + 22.0 * context.dpi,
                        14.0 * context.dpi,
                        context.theme.palette.text_main,
                        label,
                    );
                }
            });
        }
        if model.show_tooltip {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(
                    layout.tooltip_rect,
                    context.theme.palette.bg_elevated,
                    4.0 * context.dpi,
                );
            });
        }
        Ok(())
    }

    pub fn translate_widget_action(&self, action: &WidgetAction) -> Option<NotoraAction> {
        match action {
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
            WidgetAction::Control(ControlAction::TextCommitted {
                id: GLOBAL_SEARCH_BOX_ID,
                ..
            }) => Some(NotoraAction::FocusRequested(FocusTarget::CardList)),
            _ => None,
        }
    }

    /// 产品事件先路由给本产品 widget；返回空表示可继续交给 editor runtime。
    pub fn route_event(
        &mut self,
        event: &Event,
        focus_target: FocusTarget,
        theme: &ui::Theme,
        dpi: f32,
    ) -> Vec<NotoraAction> {
        let mut event_context = EventCtx { theme, dpi, cursor_hint: None };
        self.search_box.set_focus(focus_target == FocusTarget::NavigationSearch);
        if self.settings_overlay_open() {
            return self
                .settings_overlay
                .route_event(event, &mut event_context)
                .map(settings_overlay_action_to_notora_action)
                .into_iter()
                .collect();
        }
        if self.tag_editor_open {
            return self.route_tag_editor_event(event, &mut event_context).into_iter().collect();
        }
        if self.save_conflict_actions.is_some() {
            return self.route_save_conflict_event(event).into_iter().collect();
        }
        if let Some(action) = self.confirmation_overlay_action(event) {
            return vec![action];
        }
        if let Some(action) =
            compact_layout_action(event, self.compact_navigation_rect, self.compact_back_rect)
        {
            return vec![action];
        }
        if self.new_document_menu_open {
            if let Some(action) = new_document_menu_action(event, self.new_document_menu_rect) {
                return vec![action];
            }
            return Vec::new();
        }
        if let Some(action) = note_toolbar_action(event, &self.note_toolbar_buttons) {
            return vec![action];
        }
        if matches!(
            event,
            Event::MouseMove { .. } | Event::MouseDown { .. } | Event::MouseUp { .. }
        ) && let Some(widget_action) = self.new_note_button.on_event(event, &mut event_context)
        {
            if let Some(action) = self.translate_widget_action(&widget_action) {
                return vec![action];
            }
            if widget_action == WidgetAction::Consumed {
                return Vec::new();
            }
        }
        if let Some(action) = self.route_splitter_event(event, &mut event_context) {
            return action.into_iter().collect();
        }
        if let Some(action) = settings_button_action(event, self.settings_rect) {
            return vec![action];
        }
        let widget_action = match pointer_target(event, self) {
            Some(FocusTarget::NavigationSearch) => {
                self.search_box.on_event(event, &mut event_context)
            }
            Some(FocusTarget::NavigationTree) => {
                self.navigation_tree.on_event(event, &mut event_context)
            }
            Some(FocusTarget::CardList) => self.card_list.on_event(event, &mut event_context),
            Some(FocusTarget::Editor | FocusTarget::Overlay) => None,
            None => match focus_target {
                FocusTarget::NavigationSearch => {
                    self.search_box.on_event(event, &mut event_context)
                }
                FocusTarget::NavigationTree => {
                    self.navigation_tree.on_event(event, &mut event_context)
                }
                FocusTarget::CardList => self.card_list.on_event(event, &mut event_context),
                FocusTarget::Editor | FocusTarget::Overlay => return Vec::new(),
            },
        };
        let actions: Vec<_> = widget_action
            .as_ref()
            .and_then(|action| self.translate_widget_action(action))
            .into_iter()
            .collect();
        if !actions.is_empty() || !is_left_mouse_down(event) {
            return actions;
        }
        pointer_target(event, self).map(NotoraAction::FocusRequested).into_iter().collect()
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

    fn layout_tag_editor_overlay(&mut self, overlay_rect: Rect, dpi: f32, visible: bool) {
        if !visible {
            self.tag_editor_panel_rect = Rect::ZERO;
            self.tag_editor_confirm_rect = Rect::ZERO;
            self.tag_editor_cancel_rect = Rect::ZERO;
            return;
        }
        let panel_width = (TAG_EDITOR_PANEL_WIDTH_LOGICAL * dpi).min(overlay_rect.w.max(0.0));
        let panel_height = (TAG_EDITOR_PANEL_HEIGHT_LOGICAL * dpi).min(overlay_rect.h.max(0.0));
        self.tag_editor_panel_rect = Rect::new(
            overlay_rect.x + (overlay_rect.w - panel_width) * 0.5,
            overlay_rect.y + (overlay_rect.h - panel_height) * 0.5,
            panel_width,
            panel_height,
        );
        let button_width = CONFIRMATION_BUTTON_WIDTH_LOGICAL * dpi;
        let button_height = CONFIRMATION_BUTTON_HEIGHT_LOGICAL * dpi;
        let button_y = self.tag_editor_panel_rect.bottom() - button_height - 16.0 * dpi;
        self.tag_editor_confirm_rect = Rect::new(
            self.tag_editor_panel_rect.right() - button_width - 20.0 * dpi,
            button_y,
            button_width,
            button_height,
        );
        self.tag_editor_cancel_rect = Rect::new(
            self.tag_editor_confirm_rect.x - button_width - 8.0 * dpi,
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
        context.list.fill_rounded(
            self.save_conflict_panel_rect,
            context.theme.palette.bg_elevated,
            10.0 * context.dpi,
        );
        context.text(
            self.save_conflict_panel_rect.x + 20.0 * context.dpi,
            self.save_conflict_panel_rect.y + 38.0 * context.dpi,
            17.0 * context.dpi,
            context.theme.palette.text_main,
            "File changed on disk",
        );
        context.text(
            self.save_conflict_panel_rect.x + 20.0 * context.dpi,
            self.save_conflict_panel_rect.y + 72.0 * context.dpi,
            13.0 * context.dpi,
            context.theme.palette.text_muted,
            "Choose how to resolve the local edits without silently overwriting the file.",
        );
        for (rect, label) in
            self.save_conflict_button_rects.iter().zip(["Reload", "Save copy", "Retry", "Cancel"])
        {
            paint_note_tool_button(context, *rect, label);
        }
    }

    fn route_tag_editor_event(
        &mut self,
        event: &Event,
        context: &mut EventCtx<'_>,
    ) -> Option<NotoraAction> {
        let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
            self.tag_editor_box.set_focus(true);
            return self
                .tag_editor_box
                .on_event(event, context)
                .and_then(tag_editor_text_box_action);
        };
        if self.tag_editor_confirm_rect.contains(*px, *py) {
            return Some(NotoraAction::TagEditorConfirmed);
        }
        if self.tag_editor_cancel_rect.contains(*px, *py)
            || !self.tag_editor_panel_rect.contains(*px, *py)
        {
            return Some(NotoraAction::OverlayDismissed);
        }
        self.tag_editor_box.set_focus(true);
        self.tag_editor_box.on_event(event, context).and_then(tag_editor_text_box_action)
    }

    fn paint_tag_editor_overlay(
        &self,
        context: &mut ui::PaintCtx<'_>,
        editor: &TagEditorOverlayInput,
    ) {
        context.list.fill_rounded(
            self.tag_editor_panel_rect,
            context.theme.palette.bg_elevated,
            10.0 * context.dpi,
        );
        context.text(
            self.tag_editor_panel_rect.x + 20.0 * context.dpi,
            self.tag_editor_panel_rect.y + 36.0 * context.dpi,
            16.0 * context.dpi,
            context.theme.palette.text_main,
            &editor.title,
        );
        self.tag_editor_box.paint(context);
        paint_note_tool_button(context, self.tag_editor_cancel_rect, "Cancel");
        paint_note_tool_button(context, self.tag_editor_confirm_rect, "Save");
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
        let panel_rect = Rect::new(
            self.search_rect.x - SHELL_PADDING_LOGICAL * context.dpi,
            0.0,
            self.search_rect.w + SHELL_PADDING_LOGICAL * context.dpi * 2.0,
            self.settings_rect.bottom() + 10.0 * context.dpi,
        );
        context.list.fill(panel_rect, context.theme.palette.bg_surface);
        self.search_box.paint(context);
        let search_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
        draw_icon(
            context.list,
            "search",
            self.search_rect.x
                + (SEARCH_ICON_AREA_WIDTH_LOGICAL * context.dpi - search_icon_size) * 0.5,
            self.search_rect.y + (self.search_rect.h - search_icon_size) * 0.5,
            search_icon_size,
            context.theme.palette.text_muted,
        );
        self.navigation_tree.paint(context);
        self.new_note_button.paint(context);
        let settings_icon_size = SIDEBAR_ICON_SIZE_LOGICAL * context.dpi;
        let settings_horizontal_inset = SHELL_PADDING_LOGICAL * context.dpi;
        draw_icon(
            context.list,
            "settings",
            self.settings_rect.x + settings_horizontal_inset,
            self.settings_rect.y + (self.settings_rect.h - settings_icon_size) * 0.5,
            settings_icon_size,
            context.theme.palette.text_muted,
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
            context.theme.palette.text_muted,
            "Settings",
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
                .unwrap_or("Untitled")
                .to_owned(),
            excerpt: canonical_path.as_path().display().to_string(),
            timestamp: "External file".to_owned(),
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
            title: "Untitled".to_owned(),
            excerpt: "Unsaved external file".to_owned(),
            timestamp: "External file".to_owned(),
            icon: Some(document_icon(*kind).to_owned()),
            tag_summary: String::new(),
        },
        ExternalFileSession::Missing { last_known_path, .. } => RenderCard {
            identity: session.identity(),
            title: last_known_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Missing file")
                .to_owned(),
            excerpt: last_known_path.display().to_string(),
            timestamp: "Missing".to_owned(),
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
    let seconds = modified_nanoseconds.div_euclid(1_000_000_000);
    format!("Modified {seconds}")
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
        SettingsOverlayAction::Dismiss => NotoraAction::OverlayDismissed,
    }
}

fn confirmation_overlay_input(overlay: OverlayState) -> Option<ConfirmationOverlayInput> {
    match overlay {
        OverlayState::DeleteTagConfirmation { .. } => Some(ConfirmationOverlayInput {
            title: "Delete tag?".to_owned(),
            description: "Notes keep their content; only this tag and its links are removed."
                .to_owned(),
            confirm_label: "Delete".to_owned(),
            confirm_action: NotoraAction::TagDeletionConfirmed,
        }),
        OverlayState::TrashPermanentDeletionConfirmation { operation } => {
            let (title, description) = match operation {
                crate::action::TrashOperation::PermanentlyDelete { .. } => (
                    "Delete note permanently?",
                    "This removes the selected Trash entry and cannot be undone.",
                ),
                crate::action::TrashOperation::Empty => {
                    ("Empty Trash?", "This removes every current Trash entry and cannot be undone.")
                }
                crate::action::TrashOperation::MoveToTrash { .. }
                | crate::action::TrashOperation::Restore { .. }
                | crate::action::TrashOperation::RestoreWithRenamedPath { .. } => return None,
            };
            Some(ConfirmationOverlayInput {
                title: title.to_owned(),
                description: description.to_owned(),
                confirm_label: "Delete".to_owned(),
                confirm_action: NotoraAction::TrashPermanentDeletionConfirmed,
            })
        }
        OverlayState::TrashRestoreConflictConfirmation { .. } => Some(ConfirmationOverlayInput {
            title: "A file already exists".to_owned(),
            description:
                "Restore this note using a distinct name, leaving the existing file unchanged."
                    .to_owned(),
            confirm_label: "Restore copy".to_owned(),
            confirm_action: NotoraAction::TrashRestoreWithRenamedPathConfirmed,
        }),
        OverlayState::None
        | OverlayState::Settings
        | OverlayState::NewDocumentMenu
        | OverlayState::TagEditor
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
    context.list.fill_rounded(rect, context.theme.palette.bg_elevated, 4.0 * context.dpi);
    context.text(
        rect.x + 8.0 * context.dpi,
        rect.y + rect.h * 0.5 + 5.0 * context.dpi,
        12.0 * context.dpi,
        context.theme.palette.text_muted,
        label,
    );
}

fn layout_note_toolbar(
    card_list_rect: Rect,
    padding: f32,
    button_width: f32,
    button_height: f32,
    inputs: &[NoteToolbarButtonInput],
) -> Vec<RenderedToolbarButton> {
    let scale = button_height / NOTE_TOOL_BUTTON_HEIGHT_LOGICAL;
    let gap = NOTE_TOOL_BUTTON_GAP_LOGICAL * scale;
    let available_width = (card_list_rect.w - padding * 2.0).max(0.0);
    let count = inputs.len();
    let fitted_width = if count == 0 {
        0.0
    } else {
        ((available_width - gap * count.saturating_sub(1) as f32) / count as f32)
            .max(0.0)
            .min(button_width)
    };
    let button_y = card_list_rect.y + 8.0 * scale;
    inputs
        .iter()
        .rev()
        .enumerate()
        .map(|(index, input)| RenderedToolbarButton {
            rect: Rect::new(
                card_list_rect.right()
                    - padding
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

fn tag_editor_text_box_rect(panel_rect: Rect, dpi: f32) -> Rect {
    Rect::new(
        panel_rect.x + 20.0 * dpi,
        panel_rect.y + 54.0 * dpi,
        (panel_rect.w - 40.0 * dpi).max(0.0),
        TAG_EDITOR_TEXT_BOX_HEIGHT_LOGICAL * dpi,
    )
}

fn tag_editor_text_box_action(action: WidgetAction) -> Option<NotoraAction> {
    match action {
        WidgetAction::Control(ControlAction::TextEdited {
            id: TAG_EDITOR_TEXT_BOX_ID,
            value: TextPayload::Plain(display_name),
        }) => Some(NotoraAction::TagEditorNameChanged(display_name)),
        WidgetAction::Control(ControlAction::TextCommitted {
            id: TAG_EDITOR_TEXT_BOX_ID, ..
        }) => Some(NotoraAction::TagEditorConfirmed),
        _ => None,
    }
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

fn new_document_menu_action(event: &Event, menu_rect: Rect) -> Option<NotoraAction> {
    let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
        return None;
    };
    if !menu_rect.contains(*px, *py) || menu_rect.h <= 0.0 {
        return None;
    }
    let item_height = menu_rect.h / 3.0;
    let item_index = ((*py - menu_rect.y) / item_height).floor() as usize;
    let kind = match item_index {
        0 => DocumentKind::Markdown,
        1 => DocumentKind::Text,
        2 => DocumentKind::Mindmap,
        _ => return None,
    };
    Some(NotoraAction::CreateRequested(kind))
}

fn card_list_title(scope: &NavigationScope) -> &'static str {
    match scope {
        NavigationScope::Search { .. } => "Search",
        NavigationScope::WorkspaceRoot => "Workspace",
        NavigationScope::Directory { .. } => "Folder",
        NavigationScope::Starred => "Starred",
        NavigationScope::Trash => "Trash",
        NavigationScope::Tag { .. } => "Tag",
        NavigationScope::ExternalFiles => "Files",
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
        assert_eq!(model.card_list_title, "Workspace");
        assert!(model.cards.is_empty());
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
        let tag_confirmation = confirmation_overlay_input(OverlayState::DeleteTagConfirmation {
            tag_id: notora_core::TagId::generate(),
        })
        .expect("tag confirmation should render");
        assert_eq!(tag_confirmation.confirm_action, NotoraAction::TagDeletionConfirmed);

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
    fn selected_note_toolbar_keeps_product_actions_out_of_the_widget_state() {
        let note_id = NoteId::generate();
        let buttons = vec![
            RenderedToolbarButton {
                rect: Rect::new(10.0, 10.0, 60.0, 28.0),
                label: "Rename".to_owned(),
                action: NotoraAction::RenameDialogRequested(note_id),
            },
            RenderedToolbarButton {
                rect: Rect::new(76.0, 10.0, 60.0, 28.0),
                label: "Trash".to_owned(),
                action: NotoraAction::TrashOperationRequested(TrashOperation::MoveToTrash {
                    note_id,
                }),
            },
        ];
        let click = |px| Event::MouseDown { px, py: 20.0, button: ui::core::MouseButton::Left };

        assert_eq!(
            note_toolbar_action(&click(20.0), &buttons),
            Some(NotoraAction::RenameDialogRequested(note_id))
        );
        assert_eq!(
            note_toolbar_action(&click(90.0), &buttons),
            Some(NotoraAction::TrashOperationRequested(TrashOperation::MoveToTrash { note_id }))
        );
        assert_eq!(note_toolbar_action(&click(160.0), &buttons), None);
    }

    #[test]
    fn tag_and_trash_toolbars_map_to_typed_domain_actions() {
        let tag_id = notora_core::TagId::generate();
        let note_id = NoteId::generate();
        let tag_buttons = note_toolbar_buttons(&NavigationScope::Tag { tag_id }, Some(note_id));
        assert!(tag_buttons.iter().any(|button| {
            button.action
                == NotoraAction::MetadataMutationRequested(MetadataMutation::AttachTag {
                    note_id,
                    tag_id,
                })
        }));
        assert!(tag_buttons.iter().any(|button| {
            button.action
                == NotoraAction::MetadataMutationRequested(MetadataMutation::DetachTag {
                    note_id,
                    tag_id,
                })
        }));

        let trash_buttons = note_toolbar_buttons(&NavigationScope::Trash, Some(note_id));
        assert_eq!(
            trash_buttons[0].action,
            NotoraAction::TrashOperationRequested(TrashOperation::Restore { note_id })
        );
        assert_eq!(
            trash_buttons[1].action,
            NotoraAction::TrashOperationRequested(TrashOperation::PermanentlyDelete { note_id })
        );
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
    fn tag_editor_text_box_keeps_the_name_update_at_the_product_boundary() {
        let action = tag_editor_text_box_action(WidgetAction::Control(ControlAction::TextEdited {
            id: TAG_EDITOR_TEXT_BOX_ID,
            value: TextPayload::Plain("Roadmap".to_owned()),
        }));
        assert_eq!(action, Some(NotoraAction::TagEditorNameChanged("Roadmap".to_owned())));
    }
}
