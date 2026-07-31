use std::collections::HashMap;

use appkit_shell::editor_runtime::{EditorFrame, RenderError};
use notora_core::NavigationScope;
use ui::core::WidgetAction;
use ui::core::widget::WidgetId;
use ui::search_bar::{SearchBarAction, SearchBarSnapshot, SearchBarWidget};
use ui::splitter::{SplitterAction, SplitterInput, SplitterWidget};
use ui::status_state::{StatusStateInput, StatusStateKind, StatusStateWidget};
use ui::tree_list::{
    TreeListAction, TreeListInput, TreeListWidget, TreeRowExpansion, TreeRowInput, TreeRowKey,
    TreeRowSelection,
};
use ui::virtual_card_list::{
    CardInput, CardKey, VirtualCardListAction, VirtualCardListInput, VirtualCardListWidget,
};
use ui::{Event, EventCtx, Rect, Widget};

use crate::action::NotoraAction;
use crate::shell::layout::ShellLayout;
use crate::{FocusTarget, NotoraState, OverlayState, Pane, ResponsiveLayoutMode};

const SETTINGS_BUTTON_ID: WidgetId = WidgetId(9_001);
const SEARCH_BAR_HEIGHT_LOGICAL: f32 = 36.0;
const SHELL_PADDING_LOGICAL: f32 = 12.0;

/// 静态产品壳所需的纯展示输入。领域状态在此映射后不再传入 widget。
#[derive(Clone, Debug, Default)]
pub struct NotoraRenderModel {
    pub search_query: String,
    pub navigation_rows: Vec<TreeRowInput>,
    pub cards: Vec<CardInput>,
    pub card_list_title: String,
    pub show_settings_overlay: bool,
    pub show_menu: bool,
    pub show_tooltip: bool,
}

impl NotoraRenderModel {
    pub fn from_state(state: &NotoraState) -> Self {
        let selected_scope = &state.library.navigation_scope;
        let navigation_rows = vec![
            navigation_row(
                1,
                "Workspace",
                "folder",
                NavigationScope::WorkspaceRoot,
                selected_scope,
            ),
            navigation_row(2, "Starred", "star", NavigationScope::Starred, selected_scope),
            navigation_row(3, "Trash", "trash-2", NavigationScope::Trash, selected_scope),
            navigation_row(4, "Files", "file", NavigationScope::ExternalFiles, selected_scope),
        ];
        let search_query = match selected_scope {
            NavigationScope::Search { query } => query.clone(),
            _ => String::new(),
        };
        Self {
            search_query,
            navigation_rows,
            cards: Vec::new(),
            card_list_title: card_list_title(selected_scope).to_owned(),
            show_settings_overlay: state.layout.overlay == OverlayState::Settings,
            show_menu: false,
            show_tooltip: false,
        }
    }
}

/// 三栏静态壳；仅持有通用 widget 与当帧键到产品动作的映射。
pub struct NotoraShell {
    search_bar: SearchBarWidget,
    navigation_tree: TreeListWidget,
    card_list: VirtualCardListWidget,
    card_empty_state: StatusStateWidget,
    editor_empty_state: StatusStateWidget,
    navigation_splitter: SplitterWidget,
    card_list_splitter: SplitterWidget,
    navigation_actions: HashMap<TreeRowKey, NotoraAction>,
    card_actions: HashMap<CardKey, NotoraAction>,
    search_rect: Rect,
    navigation_tree_rect: Rect,
    card_content_rect: Rect,
    settings_rect: Rect,
}

impl Default for NotoraShell {
    fn default() -> Self {
        Self::new()
    }
}

impl NotoraShell {
    pub fn new() -> Self {
        Self {
            search_bar: SearchBarWidget::new(),
            navigation_tree: TreeListWidget::new(),
            card_list: VirtualCardListWidget::new(),
            card_empty_state: StatusStateWidget::new(),
            editor_empty_state: StatusStateWidget::new(),
            navigation_splitter: SplitterWidget::new(),
            card_list_splitter: SplitterWidget::new(),
            navigation_actions: HashMap::new(),
            card_actions: HashMap::new(),
            search_rect: Rect::ZERO,
            navigation_tree_rect: Rect::ZERO,
            card_content_rect: Rect::ZERO,
            settings_rect: Rect::ZERO,
        }
    }

    pub fn update_model(&mut self, model: &NotoraRenderModel) {
        self.navigation_actions.clear();
        for row in &model.navigation_rows {
            if let Some(scope) = navigation_scope_for_key(row.key) {
                self.navigation_actions.insert(row.key, NotoraAction::NavigationSelected(scope));
            }
        }
        self.card_actions.clear();
        self.navigation_tree.set_input(TreeListInput {
            rows: model.navigation_rows.clone(),
            scroll_offset_px: 0.0,
        });
        self.card_list
            .set_input(VirtualCardListInput { cards: model.cards.clone(), scroll_offset_px: 0.0 });
        self.search_bar.set_input(SearchBarSnapshot {
            query: model.search_query.clone(),
            visible: true,
            ..SearchBarSnapshot::default()
        });
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
        let tree_rect = Rect::new(
            layout.navigation_rect.x,
            search_rect.bottom() + padding,
            layout.navigation_rect.w,
            (layout.navigation_rect.bottom() - search_rect.bottom() - padding * 3.0).max(0.0),
        );
        self.navigation_tree_rect = tree_rect;
        let settings_rect = Rect::new(
            layout.navigation_rect.x + padding,
            layout.navigation_rect.bottom() - 40.0 * dpi,
            (layout.navigation_rect.w - padding * 2.0).max(0.0),
            28.0 * dpi,
        );
        self.search_rect = search_rect;
        self.settings_rect = settings_rect;
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
        frame.with_layout_context(|context| {
            self.search_bar.set_rect(search_rect, context);
            self.navigation_tree.set_rect(tree_rect, context);
            self.card_list.set_rect(card_content_rect, context);
            self.card_empty_state.set_rect(card_content_rect, context);
            self.editor_empty_state.set_rect(layout.editor_rect, context);
            self.navigation_splitter.set_rect(layout.navigation_splitter_rect, context);
            self.card_list_splitter.set_rect(layout.card_list_splitter_rect, context);
        });
        frame.with_paint_context(|context| {
            context.list.fill(layout.navigation_rect, context.theme.palette.bg_base);
            context.list.fill(layout.card_list_rect, context.theme.palette.bg_surface);
            context.list.fill(layout.editor_rect, context.theme.editor.background);
            let previous_offset = context.list.offset;
            context.list.offset =
                (previous_offset.0 + search_rect.x, previous_offset.1 + search_rect.y);
            self.search_bar.paint(context);
            context.list.offset = previous_offset;
            self.navigation_tree.paint(context);
            self.navigation_splitter.paint(context);
            self.card_list_splitter.paint(context);
            context.list.fill_rounded(
                settings_rect,
                context.theme.palette.bg_elevated,
                6.0 * context.dpi,
            );
            context.text(
                settings_rect.x + 10.0 * context.dpi,
                settings_rect.y + settings_rect.h * 0.5 + 5.0 * context.dpi,
                13.0 * context.dpi,
                context.theme.palette.text_main,
                "Settings",
            );
            context.text(
                layout.card_list_rect.x + padding,
                layout.card_list_rect.y + 28.0 * context.dpi,
                16.0 * context.dpi,
                context.theme.palette.text_main,
                &model.card_list_title,
            );
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
                    layout.menu_rect,
                    context.theme.palette.bg_elevated,
                    6.0 * context.dpi,
                );
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
                self.card_actions.get(key).cloned()
            }
            WidgetAction::Control(ui::core::widget::ControlAction::Activated { id })
                if *id == SETTINGS_BUTTON_ID =>
            {
                Some(NotoraAction::OpenSettings)
            }
            WidgetAction::SearchBar(SearchBarAction::QueryChanged(query)) => {
                Some(NotoraAction::SearchCommitted(query.clone()))
            }
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
        if let Some(action) = self.route_splitter_event(event, &mut event_context) {
            return action.into_iter().collect();
        }
        if let Some(action) = settings_button_action(event, self.settings_rect) {
            return vec![action];
        }
        let widget_action = match pointer_target(event, self) {
            Some(FocusTarget::NavigationSearch) => self
                .search_bar
                .on_event(&search_local_event(event, self.search_rect), &mut event_context),
            Some(FocusTarget::NavigationTree) => {
                self.navigation_tree.on_event(event, &mut event_context)
            }
            Some(FocusTarget::CardList) => self.card_list.on_event(event, &mut event_context),
            Some(FocusTarget::Editor | FocusTarget::Overlay) => None,
            None => match focus_target {
                FocusTarget::NavigationSearch => self
                    .search_bar
                    .on_event(&search_local_event(event, self.search_rect), &mut event_context),
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

fn search_local_event(event: &Event, search_rect: Rect) -> Event {
    match event {
        Event::MouseMove { px, py } => {
            Event::MouseMove { px: *px - search_rect.x, py: *py - search_rect.y }
        }
        Event::MouseDown { px, py, button } => {
            Event::MouseDown { px: *px - search_rect.x, py: *py - search_rect.y, button: *button }
        }
        Event::MouseUp { px, py, button } => {
            Event::MouseUp { px: *px - search_rect.x, py: *py - search_rect.y, button: *button }
        }
        Event::Wheel { dx, dy, px, py } => {
            Event::Wheel { dx: *dx, dy: *dy, px: *px - search_rect.x, py: *py - search_rect.y }
        }
        _ => event.clone(),
    }
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

    #[test]
    fn builds_a_static_render_model() {
        let model = NotoraRenderModel::from_state(&NotoraState::default());
        assert_eq!(model.navigation_rows.len(), 4);
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
    }
}
