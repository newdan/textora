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

use crate::action::NotoraAction;
use crate::external_files::ExternalFileSession;
use crate::shell::layout::ShellLayout;
use crate::state::CardPageState;
use crate::{FocusTarget, NotoraState, OverlayState, Pane, ResponsiveLayoutMode};

const GLOBAL_SEARCH_BOX_ID: WidgetId = WidgetId(9_000);
const SETTINGS_BUTTON_ID: WidgetId = WidgetId(9_001);
const NEW_NOTE_BUTTON_ID: WidgetId = WidgetId(9_002);
const NEW_NOTE_MENU_BUTTON_ID: WidgetId = WidgetId(9_003);
const NOTE_TOOL_BUTTON_WIDTH_LOGICAL: f32 = 64.0;
const NOTE_TOOL_BUTTON_HEIGHT_LOGICAL: f32 = 28.0;
const SEARCH_BAR_HEIGHT_LOGICAL: f32 = 32.0;
const SEARCH_ICON_AREA_WIDTH_LOGICAL: f32 = 32.0;
const SHELL_PADDING_LOGICAL: f32 = 12.0;
const SIDEBAR_CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
const SIDEBAR_ICON_SIZE_LOGICAL: f32 = 16.0;
const SIDEBAR_LABEL_FONT_SIZE_LOGICAL: f32 = 15.0;
const CARD_LOAD_MORE_THRESHOLD_LOGICAL: f32 = 160.0;
const NEW_DOCUMENT_MENU_ITEM_HEIGHT_LOGICAL: f32 = 34.0;

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

/// 静态产品壳所需的纯展示输入。领域状态在此映射后不再传入 widget。
#[derive(Clone, Debug, Default)]
pub struct NotoraRenderModel {
    pub search_query: String,
    pub navigation_rows: Vec<TreeRowInput>,
    pub cards: Vec<RenderCard>,
    pub selected_card: Option<DocumentIdentity>,
    pub card_scroll_offset_px: f32,
    pub card_list_title: String,
    pub show_settings_overlay: bool,
    pub show_menu: bool,
    pub show_tooltip: bool,
    pub can_create_note: bool,
    pub selected_note_id: Option<NoteId>,
}

impl NotoraRenderModel {
    pub fn from_state(state: &NotoraState) -> Self {
        let selected_scope = &state.library.navigation_scope;
        let navigation_rows = vec![
            navigation_row(
                1,
                "Workspace",
                "folder-open",
                NavigationScope::WorkspaceRoot,
                selected_scope,
            ),
            navigation_row(2, "Starred", "star", NavigationScope::Starred, selected_scope),
            navigation_row(3, "Trash", "trash-2", NavigationScope::Trash, selected_scope),
            navigation_row(4, "Files", "file", NavigationScope::ExternalFiles, selected_scope),
        ];
        let search_query = state.library.search_text.clone();
        Self {
            search_query,
            navigation_rows,
            cards: render_cards(state),
            selected_card: state.library.selected_card,
            card_scroll_offset_px: state.library.card_scroll_offset_px,
            card_list_title: card_list_title(selected_scope).to_owned(),
            show_settings_overlay: state.layout.overlay == OverlayState::Settings,
            show_menu: state.layout.overlay == OverlayState::NewDocumentMenu,
            show_tooltip: false,
            can_create_note: !matches!(
                selected_scope,
                NavigationScope::Trash | NavigationScope::ExternalFiles
            ),
            selected_note_id: state.library.selected_card.and_then(|identity| match identity {
                DocumentIdentity::Note(note_id) => Some(note_id),
                DocumentIdentity::ExternalFile(_) => None,
            }),
        }
    }
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
    navigation_actions: HashMap<TreeRowKey, NotoraAction>,
    card_identities: HashMap<CardKey, DocumentIdentity>,
    card_keys: HashMap<DocumentIdentity, CardKey>,
    next_card_key: u64,
    search_rect: Rect,
    navigation_tree_rect: Rect,
    card_content_rect: Rect,
    settings_rect: Rect,
    new_document_menu_rect: Rect,
    new_document_menu_open: bool,
    selected_note_id: Option<NoteId>,
    rename_note_rect: Rect,
    move_note_rect: Rect,
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
        Self {
            search_box,
            navigation_tree: TreeListWidget::new(),
            card_list: VirtualCardListWidget::new(),
            card_empty_state: StatusStateWidget::new(),
            editor_empty_state: StatusStateWidget::new(),
            navigation_splitter: SplitterWidget::new(),
            card_list_splitter: SplitterWidget::new(),
            new_note_button,
            navigation_actions: HashMap::new(),
            card_identities: HashMap::new(),
            card_keys: HashMap::new(),
            next_card_key: 1,
            search_rect: Rect::ZERO,
            navigation_tree_rect: Rect::ZERO,
            card_content_rect: Rect::ZERO,
            settings_rect: Rect::ZERO,
            new_document_menu_rect: Rect::ZERO,
            new_document_menu_open: false,
            selected_note_id: None,
            rename_note_rect: Rect::ZERO,
            move_note_rect: Rect::ZERO,
        }
    }

    pub fn update_model(&mut self, model: &NotoraRenderModel) {
        self.navigation_actions.clear();
        for row in &model.navigation_rows {
            if let Some(scope) = navigation_scope_for_key(row.key) {
                self.navigation_actions.insert(row.key, NotoraAction::NavigationSelected(scope));
            }
        }
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
        self.new_document_menu_open = model.show_menu;
        self.selected_note_id = model.selected_note_id;
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
        self.move_note_rect = Rect::new(
            layout.card_list_rect.right() - padding - tool_button_width,
            layout.card_list_rect.y + 8.0 * dpi,
            tool_button_width,
            tool_button_height,
        );
        self.rename_note_rect = Rect::new(
            self.move_note_rect.x - tool_button_width - 6.0 * dpi,
            self.move_note_rect.y,
            tool_button_width,
            tool_button_height,
        );
        frame.with_layout_context(|context| {
            self.search_box.set_rect(search_rect, context);
            self.navigation_tree.set_rect(tree_rect, context);
            self.card_list.set_rect(card_content_rect, context);
            self.card_empty_state.set_rect(card_content_rect, context);
            self.editor_empty_state.set_rect(layout.editor_rect, context);
            self.navigation_splitter.set_rect(layout.navigation_splitter_rect, context);
            self.card_list_splitter.set_rect(layout.card_list_splitter_rect, context);
            self.new_note_button.set_rect(new_note_rect, context);
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
            if model.selected_note_id.is_some() {
                paint_note_tool_button(context, self.rename_note_rect, "Rename");
                paint_note_tool_button(context, self.move_note_rect, "Move");
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
        if model.show_settings_overlay {
            frame.with_paint_context(|context| {
                context.list.fill_rounded(layout.overlay_rect, [0.0, 0.0, 0.0, 0.45], 0.0);
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
        if self.new_document_menu_open {
            if let Some(action) = new_document_menu_action(event, self.new_document_menu_rect) {
                return vec![action];
            }
            return Vec::new();
        }
        if let Some(action) = note_tool_action(
            event,
            self.selected_note_id,
            self.rename_note_rect,
            self.move_note_rect,
        ) {
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

fn navigation_row(
    key: u64,
    label: &str,
    icon: &str,
    scope: NavigationScope,
    selected_scope: &NavigationScope,
) -> TreeRowInput {
    TreeRowInput {
        key: TreeRowKey(key),
        label: label.to_owned(),
        icon: Some(icon.to_owned()),
        depth: 0,
        expansion: TreeRowExpansion::Leaf,
        selection: if scope == *selected_scope {
            TreeRowSelection::Selected
        } else {
            TreeRowSelection::Unselected
        },
        badge: None,
    }
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

fn note_tool_action(
    event: &Event,
    selected_note_id: Option<NoteId>,
    rename_rect: Rect,
    move_rect: Rect,
) -> Option<NotoraAction> {
    let note_id = selected_note_id?;
    let Event::MouseDown { px, py, button: ui::core::MouseButton::Left } = event else {
        return None;
    };
    if rename_rect.contains(*px, *py) {
        return Some(NotoraAction::RenameDialogRequested(note_id));
    }
    move_rect.contains(*px, *py).then_some(NotoraAction::MoveDialogRequested(note_id))
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

fn navigation_scope_for_key(key: TreeRowKey) -> Option<NavigationScope> {
    match key.0 {
        1 => Some(NavigationScope::WorkspaceRoot),
        2 => Some(NavigationScope::Starred),
        3 => Some(NavigationScope::Trash),
        4 => Some(NavigationScope::ExternalFiles),
        _ => None,
    }
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
    fn selected_note_toolbar_produces_rename_and_move_dialog_actions() {
        let note_id = NoteId::generate();
        let rename_rect = Rect::new(10.0, 10.0, 60.0, 28.0);
        let move_rect = Rect::new(76.0, 10.0, 60.0, 28.0);
        let click = |px| Event::MouseDown { px, py: 20.0, button: ui::core::MouseButton::Left };

        assert_eq!(
            note_tool_action(&click(20.0), Some(note_id), rename_rect, move_rect),
            Some(NotoraAction::RenameDialogRequested(note_id))
        );
        assert_eq!(
            note_tool_action(&click(90.0), Some(note_id), rename_rect, move_rect),
            Some(NotoraAction::MoveDialogRequested(note_id))
        );
        assert_eq!(note_tool_action(&click(20.0), None, rename_rect, move_rect), None);
    }
}
