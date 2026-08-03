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

use crate::action::{ConflictResolution, MetadataMutation, NotoraAction, TrashOperation};
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
const NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL: f32 = 34.0;
const NEW_DOCUMENT_MENU_ITEMS: [(&str, DocumentKind); 3] = [
    ("TXT", DocumentKind::Text),
    ("MD", DocumentKind::Markdown),
    ("MMAP.MD", DocumentKind::Mindmap),
];
const COMPACT_NAVIGATION_BUTTON_WIDTH_LOGICAL: f32 = 72.0;
const COMPACT_BACK_BUTTON_WIDTH_LOGICAL: f32 = 64.0;
const CONFIRMATION_PANEL_WIDTH_LOGICAL: f32 = 360.0;
const CONFIRMATION_PANEL_HEIGHT_LOGICAL: f32 = 160.0;
const CONFIRMATION_BUTTON_WIDTH_LOGICAL: f32 = 88.0;
const CONFIRMATION_BUTTON_HEIGHT_LOGICAL: f32 = 32.0;
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
    Visible,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CardHeaderLayout {
    title_baseline_y: f32,
    control_top_y: f32,
    content_top_y: f32,
}

impl NewNoteControlState {
    fn is_visible(self) -> bool {
        self == Self::Visible
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
    pub show_settings_overlay: bool,
    pub settings_overlay: SettingsOverlayInput,
    pub confirmation: Option<ConfirmationOverlayInput>,
    pub show_menu: bool,
    pub show_tooltip: bool,
    pub new_note_control: NewNoteControlState,
    pub note_toolbar: Vec<NoteToolbarButtonInput>,
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
            new_note_control: new_note_control_state(selected_scope),
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
        }
    }
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
    let Some(note_id) = selected_note_id else {
        return Vec::new();
    };
    vec![
        NoteToolbarButtonInput {
            label: "重命名".to_owned(),
            action: NotoraAction::RenameDialogRequested(note_id),
        },
        NoteToolbarButtonInput {
            label: "移动".to_owned(),
            action: NotoraAction::MoveDialogRequested(note_id),
        },
        NoteToolbarButtonInput {
            label: "星标".to_owned(),
            action: NotoraAction::MetadataMutationRequested(MetadataMutation::ToggleStar {
                note_id,
            }),
        },
        NoteToolbarButtonInput {
            label: "回收站".to_owned(),
            action: NotoraAction::TrashOperationRequested(TrashOperation::MoveToTrash { note_id }),
        },
    ]
}

fn new_note_control_state(scope: &NavigationScope) -> NewNoteControlState {
    match scope {
        NavigationScope::Search { .. } | NavigationScope::Trash => NewNoteControlState::Hidden,
        NavigationScope::WorkspaceRoot
        | NavigationScope::Directory { .. }
        | NavigationScope::Starred
        | NavigationScope::Tag { .. }
        | NavigationScope::ExternalFiles => NewNoteControlState::Visible,
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
        search_box.set_placeholder("搜索笔记...");
        search_box.set_max_len_bytes(2_048);
        search_box.set_leading_content_inset_logical(SEARCH_ICON_AREA_WIDTH_LOGICAL);
        let mut new_note_button = SplitButtonWidget::new();
        new_note_button.set_action_ids(NEW_NOTE_BUTTON_ID, NEW_NOTE_MENU_BUTTON_ID);
        new_note_button.set_icon(Some("plus".to_owned()));
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
            label: "新建笔记".to_owned(),
            enabled: model.new_note_control.is_visible(),
        });
        self.new_note_button.set_menu_open(model.show_menu);
        self.settings_overlay.set_input(model.settings_overlay.clone());
        self.settings_overlay_open = model.show_settings_overlay;
        self.confirmation_action =
            model.confirmation.as_ref().map(|input| input.confirm_action.clone());
        self.new_document_menu_open = model.show_menu;
        self.save_conflict_actions =
            model.save_conflict.as_ref().map(|conflict| conflict.actions.clone());
        self.card_empty_state.set_input(StatusStateInput {
            kind: StatusStateKind::Empty,
            title: "暂无笔记".to_owned(),
            description: "新建一篇笔记，或者从左侧选择其他位置。".to_owned(),
            icon: Some("notebook-pen".to_owned()),
            action_label: None,
            action_id: None,
        });
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
        self.new_document_menu_rect = new_document_menu_rect(new_note_rect, dpi);
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
            self.editor_empty_state.set_rect(layout.editor_rect, context);
            self.navigation_splitter.set_rect(layout.navigation_splitter_rect, context);
            self.card_list_splitter.set_rect(layout.card_list_splitter_rect, context);
            self.new_note_button.set_rect(new_note_rect, context);
            if model.show_settings_overlay {
                self.settings_overlay.set_rect(layout.overlay_rect, context);
            }
        });
        frame.with_paint_context(|context| {
            let application_theme = context.theme.application_theme();
            context.list.fill(layout.navigation_rect, application_theme.navigation_surface);
            context.list.fill(layout.card_list_rect, application_theme.content_surface);
            context.list.fill(layout.editor_rect, application_theme.editor_surface);
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
        if model.show_menu {
            frame.with_paint_context(|context| {
                let application_theme = context.theme.application_theme();
                context.list.fill_rounded(
                    self.new_document_menu_rect,
                    application_theme.overlay_surface,
                    6.0 * context.dpi,
                );
                for (index, (label, _)) in NEW_DOCUMENT_MENU_ITEMS.iter().enumerate() {
                    let item_top = self.new_document_menu_rect.y
                        + index as f32 * NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL * context.dpi;
                    context.text(
                        self.new_document_menu_rect.x + 10.0 * context.dpi,
                        item_top + 22.0 * context.dpi,
                        14.0 * context.dpi,
                        application_theme.text_primary,
                        label,
                    );
                }
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
            if should_dismiss_new_document_menu(
                event,
                self.new_document_menu_rect,
                self.new_note_button.menu_rect(),
            ) {
                return vec![NotoraAction::OverlayDismissed];
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
    let seconds = modified_nanoseconds.div_euclid(1_000_000_000);
    format!("修改时间 {seconds}")
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

fn new_document_menu_action(event: &Event, menu_rect: Rect) -> Option<NotoraAction> {
    let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
        return None;
    };
    if !menu_rect.contains(*px, *py) || menu_rect.h <= 0.0 {
        return None;
    }
    let item_height = menu_rect.h / 3.0;
    let item_index = ((*py - menu_rect.y) / item_height).floor() as usize;
    let (_, kind) = NEW_DOCUMENT_MENU_ITEMS.get(item_index)?;
    Some(NotoraAction::CreateRequested(*kind))
}

fn should_dismiss_new_document_menu(event: &Event, menu_rect: Rect, trigger_rect: Rect) -> bool {
    let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
        return false;
    };
    trigger_rect.contains(*px, *py) || !menu_rect.contains(*px, *py)
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

fn new_document_menu_rect(button_rect: Rect, dpi: f32) -> Rect {
    if button_rect == Rect::ZERO {
        return Rect::ZERO;
    }
    Rect::new(
        button_rect.x,
        button_rect.bottom() + 4.0 * dpi,
        button_rect.w,
        NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL * 3.0 * dpi,
    )
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
    fn new_note_control_is_laid_out_inside_the_middle_column() {
        let navigation_rect = Rect::new(0.0, 0.0, 220.0, 600.0);
        let card_list_rect = Rect::new(228.0, 0.0, 340.0, 600.0);

        let button_rect = new_note_button_rect(
            card_list_rect,
            1.0,
            NewNoteControlState::Visible,
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
            NewNoteControlState::Visible,
            1,
        );
        let new_note_rect = new_note_button_rect(
            card_list_rect,
            1.0,
            NewNoteControlState::Visible,
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
    fn new_document_menu_uses_text_markdown_mindmap_order() {
        let menu_rect = Rect::new(10.0, 20.0, 120.0, 102.0);
        let click_item = |index: usize| Event::MouseDown {
            px: 20.0,
            py: menu_rect.y + (index as f32 + 0.5) * menu_rect.h / 3.0,
            button: ui::core::MouseButton::Left,
        };

        assert_eq!(
            new_document_menu_action(&click_item(0), menu_rect),
            Some(NotoraAction::CreateRequested(DocumentKind::Text))
        );
        assert_eq!(
            new_document_menu_action(&click_item(1), menu_rect),
            Some(NotoraAction::CreateRequested(DocumentKind::Markdown))
        );
        assert_eq!(
            new_document_menu_action(&click_item(2), menu_rect),
            Some(NotoraAction::CreateRequested(DocumentKind::Mindmap))
        );
    }

    #[test]
    fn new_document_menu_dismisses_from_its_trigger_or_backdrop_only() {
        let menu_rect = Rect::new(100.0, 50.0, 120.0, 102.0);
        let trigger_rect = Rect::new(192.0, 14.0, 28.0, 28.0);
        let click = |px, py| Event::MouseDown { px, py, button: ui::core::MouseButton::Left };

        assert!(should_dismiss_new_document_menu(
            &click(trigger_rect.x + 4.0, trigger_rect.y + 4.0),
            menu_rect,
            trigger_rect,
        ));
        assert!(should_dismiss_new_document_menu(&click(20.0, 20.0), menu_rect, trigger_rect,));
        assert!(!should_dismiss_new_document_menu(
            &click(menu_rect.x + 4.0, menu_rect.y + 4.0),
            menu_rect,
            trigger_rect,
        ));
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
    fn tag_scope_has_no_manual_tag_mutation_toolbar_actions() {
        let tag_id = notora_core::TagId::generate();
        let note_id = NoteId::generate();
        let tag_buttons = note_toolbar_buttons(&NavigationScope::Tag { tag_id }, Some(note_id));
        assert_eq!(
            tag_buttons.iter().map(|button| button.label.as_str()).collect::<Vec<_>>(),
            vec!["重命名", "移动", "星标", "回收站"]
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
}
