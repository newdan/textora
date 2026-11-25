//! Product action routing and effect application.

pub(crate) mod chrome;
pub(crate) mod commands;
pub(crate) mod editor;
pub(crate) mod mouse;
pub(crate) mod search;
mod settings;
pub(crate) mod tabs;
pub(crate) mod viewport;
pub(crate) mod wysiwyg;

use crate::actions::AppAction;
use crate::app::App;
use crate::app_effect::AppEffect;
use crate::dispatch::chrome::{ChromeDispatchAction, SettingsDispatchAction, TabScrollDirection};
use crate::dispatch::viewport::ViewportDispatchAction;
use appkit_shell::DrainStart;

impl App {
    pub(crate) fn execute_commands(
        &mut self,
        commands: Vec<crate::menu_handler::AppCommand>,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> AppEffect {
        commands.into_iter().fold(AppEffect::NONE, |effect, command| {
            effect.merge(self.dispatch_app_command(command, event_loop))
        })
    }

    /// Dispatch a native menu action to the appropriate handler.
    pub(crate) fn dispatch_menu_action(
        &mut self,
        action: crate::native_menu::MenuAction,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        if let Some(routed_action) = self.route_search_menu_action(action) {
            self.dispatch(routed_action, event_loop);
            return;
        }
        let commands = crate::menu_handler::dispatch_menu_action(action);
        self.dispatch(AppAction::ExecuteAppCommands(commands), event_loop);
    }

    fn route_search_menu_action(
        &mut self,
        action: crate::native_menu::MenuAction,
    ) -> Option<AppAction> {
        use crate::native_menu::MenuAction;
        use ui::core::{KeyCode, Modifiers, WidgetAction};

        if !self.ui_shell.search_bar_has_keyboard_focus() {
            return None;
        }
        let key = match action {
            MenuAction::SelectAll => 'a',
            MenuAction::Copy => 'c',
            MenuAction::Cut => 'x',
            MenuAction::Paste | MenuAction::PastePlainText => 'v',
            MenuAction::Undo | MenuAction::Redo => 'z',
            _ => return None,
        };
        let modifiers =
            Modifiers { cmd: true, shift: action == MenuAction::Redo, ..Modifiers::NONE };
        let widget_action = self.ui_shell.forward_key(
            KeyCode::Char(key),
            modifiers,
            self.editor_runtime.theme(),
            self.ui_metrics().dpi,
        );
        Some(match widget_action {
            Some(WidgetAction::SearchBar(search_action)) => {
                AppAction::SearchBarAction(search_action)
            }
            _ => AppAction::RequestRedraw,
        })
    }

    /// Top-level single-apply router: `reduce_action` → `apply_effect` → IME.
    pub(crate) fn dispatch(
        &mut self,
        action: AppAction,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        self.action_pump.enqueue(action);
        if self.action_pump.start_draining() == DrainStart::AlreadyDraining {
            return;
        }
        while let Some(action) = self.action_pump.next_action() {
            let effect = self.reduce_action(action, Some(event_loop));
            self.apply_effect(effect);
            self.update_ime_cursor_area();
        }
        self.action_pump.finish_draining();
    }

    /// Pure routing: match on `AppAction`, return the aggregated `AppEffect`
    /// without applying any global follow-ups.
    pub(crate) fn reduce_action(
        &mut self,
        action: AppAction,
        event_loop: Option<&winit::event_loop::ActiveEventLoop>,
    ) -> AppEffect {
        match action {
            AppAction::RequestRedraw => AppEffect::REDRAW,
            AppAction::SetCursor(cursor) => {
                if let Some(window) = self.editor_runtime.window() {
                    window.set_cursor(cursor);
                }
                AppEffect::NONE
            }
            AppAction::DragWindow => {
                if let Some(window) = self.editor_runtime.window() {
                    let _ = window.drag_window();
                }
                AppEffect::NONE
            }
            AppAction::ResizeWindow(direction) => {
                if let Some(window) = self.editor_runtime.window() {
                    let _ = window.drag_resize_window(direction);
                }
                AppEffect::NONE
            }
            AppAction::MinimizeWindow => {
                if let Some(window) = self.editor_runtime.window() {
                    window.set_minimized(true);
                }
                AppEffect::NONE
            }
            AppAction::ToggleMaximizeWindow => {
                if let Some(window) = self.editor_runtime.window() {
                    window.set_maximized(!window.is_maximized());
                }
                AppEffect::REDRAW
            }
            AppAction::CloseWindow => {
                self.close_window(event_loop.expect("window close requires an active event loop"));
                AppEffect::NONE
            }
            AppAction::ExecuteAppCommands(commands) => self.execute_commands(
                commands,
                event_loop
                    .expect("executing app commands requires the event loop supplied by dispatch"),
            ),

            AppAction::OpenPopupMenu(menu) => {
                self.dispatch_chrome_action(ChromeDispatchAction::OpenPopup(menu))
            }
            AppAction::ExecuteContextMenuAction(action, id) => {
                self.dispatch_context_menu_action(action, id)
            }
            AppAction::OpenPopupOverflow => {
                self.dispatch_chrome_action(ChromeDispatchAction::OpenOverflow)
            }
            AppAction::ClearPopupMenu => {
                self.dispatch_chrome_action(ChromeDispatchAction::ClearPopup)
            }
            AppAction::DismissOverlay => {
                self.ui_shell.pop_overlay();
                AppEffect::REDRAW
            }
            AppAction::Settings(action) => self.dispatch_settings_view_action(action),
            #[cfg(feature = "sync")]
            AppAction::Sync(action) => self.dispatch_sync_settings_action(action),
            AppAction::UpdateMousePos(x, y) => {
                self.platform.mouse_mut().pos = (x, y);
                AppEffect::NONE
            }
            AppAction::HandleScroll(delta) => self.dispatch_wheel_scroll(delta),
            AppAction::EditorMouseInput { state, px, py } => {
                self.dispatch_editor_mouse_input(state, px, py)
            }
            AppAction::EditorCursorMoved { px, py, use_editor_cursor } => {
                self.dispatch_editor_cursor_moved(px, py, use_editor_cursor)
            }
            AppAction::SwitchTab(id) => self.dispatch_tab_switch(id),
            AppAction::CloseTab(id) => self.try_close_entry_with_prompt(id),
            AppAction::NewEmptyTab => self.new_untitled_doc(),
            AppAction::NewDocument(kind) => self.new_typed_untitled_doc(kind),
            AppAction::TogglePin => {
                let workspace_effect = self.toggle_active_editor_pin();
                self.handle_nav_effect(workspace_effect)
            }
            AppAction::ScrollTabLeft => self
                .dispatch_chrome_action(ChromeDispatchAction::ScrollTab(TabScrollDirection::Left)),
            AppAction::ScrollTabRight => self
                .dispatch_chrome_action(ChromeDispatchAction::ScrollTab(TabScrollDirection::Right)),
            AppAction::HoverTab(id_opt) => {
                let index = id_opt.and_then(|id| self.editor_tab_index(id));
                self.dispatch_chrome_action(ChromeDispatchAction::HoverTab(index))
            }
            AppAction::ScrollbarAction(action) => {
                self.dispatch_viewport_action(ViewportDispatchAction::Scrollbar(action))
            }
            AppAction::UpdateScrollTop(scroll_top) => {
                self.dispatch_viewport_action(ViewportDispatchAction::UpdateScrollTop(scroll_top))
            }
            AppAction::CanvasScrollbar { axis, action } => {
                self.dispatch_canvas_scrollbar_action(axis, action)
            }
            AppAction::CanvasPinch { delta, screen_anchor } => {
                self.dispatch_canvas_pinch(delta, screen_anchor)
            }
            AppAction::ScrollViewportBy(amount) => {
                self.dispatch_viewport_action(ViewportDispatchAction::ScrollViewportBy(amount))
            }
            AppAction::SetViewMode(mode) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetViewMode(mode))
            }
            AppAction::OpenSettingsFile => self.open_settings_file(),
            AppAction::ToggleLineNumbers => {
                self.dispatch_settings_action(SettingsDispatchAction::ToggleLineNumbers)
            }
            AppAction::ToggleWordWrap => {
                self.dispatch_settings_action(SettingsDispatchAction::ToggleWordWrap)
            }
            AppAction::ToggleStatusBar => {
                self.dispatch_settings_action(SettingsDispatchAction::ToggleStatusBar)
            }
            AppAction::SetThemeMode(mode) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetThemeMode(mode))
            }
            AppAction::ToggleInspectorPanel => {
                self.editor_runtime.toggle_active_inspector_panel().shell_effect
            }
            AppAction::InspectorPanel(action) => {
                let outcome = self.editor_runtime.dispatch_inspector_panel_action(action);
                for notification in outcome.notifications {
                    if let appkit_shell::editor_runtime::EditorNotification::DirtyChanged {
                        dirty,
                        ..
                    } = notification
                    {
                        self.update_document_edited(dirty);
                    }
                }
                outcome.shell_effect
            }
            AppAction::TablePicker(action) => self.dispatch_table_picker_action(action),
            AppAction::TableStructureEdit { command, cursor_byte, source_generation } => {
                self.dispatch_table_structure_edit(command, cursor_byte, source_generation)
            }
            AppAction::SidebarResizeStart => {
                self.dispatch_chrome_action(ChromeDispatchAction::SidebarResizeStart)
            }
            AppAction::SidebarResizeEnd => {
                self.dispatch_chrome_action(ChromeDispatchAction::SidebarResizeEnd)
            }
            AppAction::SetSidebarWidth(width) => {
                self.dispatch_chrome_action(ChromeDispatchAction::SetSidebarWidth(width))
            }
            AppAction::OpenSidebarSettingsMenu => self.open_settings_from_sidebar(),
            AppAction::ToggleSidebarPin => {
                self.dispatch_chrome_action(ChromeDispatchAction::ToggleSidebarPin)
            }
            AppAction::SearchBarAction(action) => self.dispatch_search_action(action),
            AppAction::JumpToHeading(index) => {
                self.dispatch_viewport_action(ViewportDispatchAction::JumpToHeading(index))
            }
        }
    }

    fn open_settings_from_sidebar(&mut self) -> AppEffect {
        self.open_settings_overlay()
    }

    fn dispatch_table_picker_action(
        &mut self,
        action: ui::table_picker::TablePickerAction,
    ) -> AppEffect {
        use ui::table_picker::TablePickerAction;

        match action {
            TablePickerAction::PreviewChanged(_) => AppEffect::REDRAW,
            TablePickerAction::Cancelled => {
                if !self.table_picker_overlay_is_active() {
                    return AppEffect::NONE;
                }
                self.ui_shell.pop_overlay();
                AppEffect::REDRAW
            }
            TablePickerAction::Confirmed(size) => {
                if !self.table_picker_overlay_is_active()
                    || self.active_plugin_name() != Some(ui::plugin::PLUGIN_MARKDOWN_EDITOR)
                {
                    return AppEffect::NONE;
                }
                let effect =
                    self.dispatch_semantic_edit(ui::plugin::SemanticEditCommand::InsertTable {
                        columns: size.columns,
                        rows: size.rows,
                    });
                self.ui_shell.pop_overlay();
                effect.merge(AppEffect::REDRAW)
            }
        }
    }

    fn table_picker_overlay_is_active(&self) -> bool {
        self.ui_shell
            .active_overlay_widget_ref::<ui::modal_frame::ModalFrame>()
            .is_some_and(|frame| frame.content_as_any().is::<ui::table_picker::TablePickerWidget>())
    }

    fn dispatch_table_structure_edit(
        &mut self,
        command: ui::plugin::TableStructureCommand,
        cursor_byte: usize,
        source_generation: u32,
    ) -> AppEffect {
        if self.active_plugin_name() != Some(ui::plugin::PLUGIN_MARKDOWN_EDITOR)
            || !self.active_allows_editing()
        {
            return AppEffect::NONE;
        }
        {
            let Some(mut tab) = self.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            if tab.document().generation() != source_generation
                || cursor_byte > tab.document().buffer_len()
                || !tab.document().full_text().is_char_boundary(cursor_byte)
            {
                return AppEffect::NONE;
            }
            tab.document_mut().cursor_move_to_offset(cursor_byte);
        }
        self.dispatch_semantic_edit(ui::plugin::SemanticEditCommand::TableStructure(command))
    }
}

#[cfg(test)]
impl App {
    /// Compute preview render offsets (shared by mouse input and cursor moved).
    pub(crate) fn preview_offsets(&self) -> (f32, f32) {
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let preview_top_pad = 16.0 * dpi;
        let line_count = self.active_tab_session().map_or(0, |tab| tab.document().line_count());
        let gutter_left_margin = self.editor_left_margin(line_count);
        let content_top = self.content_top_offset();
        (gutter_left_margin, content_top + preview_top_pad)
    }
}

#[cfg(test)]
pub(crate) mod canvas_drag_test_support {
    use crate::app::App;
    use crate::document_fixture::DocumentFixture;

    use std::cell::RefCell;
    use std::rc::Rc;

    use ui::plugin::{CanvasDragRequest, CanvasDragResponse, EditHitTarget, ViewPlugin};
    use winit::event::ElementState;

    #[derive(Default)]
    pub(crate) struct CanvasDragState {
        pub(crate) requests: Vec<CanvasDragRequest>,
        cancelled_sources: Vec<String>,
    }

    struct CanvasDragPlugin {
        state: Rc<RefCell<CanvasDragState>>,
    }

    impl ViewPlugin for CanvasDragPlugin {
        fn name(&self) -> &str {
            "canvas_drag_lifecycle_test"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::RichText
        }

        fn render(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }
    }

    impl ui::plugin::EditPolicy for CanvasDragPlugin {
        fn plan_edit(&self, _: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
            ui::plugin::EditPlan::UseDefault
        }
    }

    impl ui::plugin::EditableView for CanvasDragPlugin {
        fn hit_test_edit_target(
            &self,
            _x: f32,
            _y: f32,
            _offset_x: f32,
            _offset_y: f32,
        ) -> Option<Option<ui::plugin::EditHitTarget>> {
            Some(Some(EditHitTarget::SourceObject { source_range: 1..2 }))
        }
    }

    impl ui::plugin::ScrollableView for CanvasDragPlugin {
        fn content_height(&self) -> f32 {
            1_000.0
        }
    }

    impl ui::plugin::ViewPluginCapabilities for CanvasDragPlugin {
        fn scrollable(&self) -> Option<&dyn ui::plugin::ScrollableView> {
            Some(self)
        }

        fn editable(&self) -> Option<&dyn ui::plugin::EditableView> {
            Some(self)
        }

        fn allows_editing(&self) -> bool {
            true
        }

        fn canvas_mut(&mut self) -> Option<&mut dyn ui::plugin::CanvasView> {
            Some(self)
        }
    }

    impl ui::plugin::CanvasView for CanvasDragPlugin {
        fn handle_canvas_drag(
            &mut self,
            request: CanvasDragRequest,
            doc: &dyn textora_core::document::DocView,
        ) -> CanvasDragResponse {
            let mut state = self.state.borrow_mut();
            if request.phase == ui::plugin::CanvasDragPhase::Cancel {
                state.cancelled_sources.push(doc.doc_line_text(0).into_owned());
            }
            state.requests.push(request);
            CanvasDragResponse::Ignore
        }
    }

    pub(crate) fn app_with_canvas_drag_tabs() -> (App, Rc<RefCell<CanvasDragState>>) {
        let state = Rc::new(RefCell::new(CanvasDragState::default()));
        let mut app = App::new(None);
        for text in ["abc", "def"] {
            let document = DocumentFixture::new(vec![text.to_string()], 80, 10.0);
            app.push_entry_for_test(document, Box::new(CanvasDragPlugin { state: state.clone() }));
        }
        app.switch_workspace_for_test(0);
        (app, state)
    }

    pub(crate) fn start_canvas_drag(app: &mut App) {
        app.prepare_editor_pointer_frame_for_test();
        let bounds = app.plugin_render_bounds();
        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);
    }

    pub(crate) fn cancel_request_count(state: &CanvasDragState) -> usize {
        state
            .requests
            .iter()
            .filter(|request| request.phase == ui::plugin::CanvasDragPhase::Cancel)
            .count()
    }

    pub(crate) fn cancelled_sources(state: &CanvasDragState) -> &[String] {
        &state.cancelled_sources
    }

    pub(crate) fn document_texts(app: &App) -> Vec<String> {
        app.editor_tab_ids_in_order()
            .into_iter()
            .map(|tab_id| app.tab_session(tab_id).expect("tab session").document().full_text())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::canvas_drag_test_support::{
        app_with_canvas_drag_tabs, cancel_request_count, document_texts, start_canvas_drag,
    };
    use super::viewport::{MIN_PLUGIN_VIEWPORT_PX, projected_editor_rect};
    use super::*;
    use crate::document_fixture::DocumentFixture;

    fn app_with_focused_search(replace: bool) -> App {
        let mut app = App::new(None);
        let document = DocumentFixture::new(vec!["正文保持原样".to_owned()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(WysiwygBoundsPlugin));
        app.switch_workspace_for_test(0);
        app.active_tab_session_mut()
            .expect("test document must be active")
            .search_state_mut()
            .panel_visible = true;
        app.ui_shell.set_search_input(ui::search_bar::SearchBarSnapshot {
            visible: true,
            query: "旧查询".to_owned(),
            replace_query: "旧替换".to_owned(),
            replace_mode: replace,
            focus_replace: replace,
            ..Default::default()
        });
        let inputs = app.build_shell_inputs();
        app.ui_shell.update_frame(
            ui::core::Screen::new(800.0, 600.0),
            app.editor_runtime.theme(),
            &mut ui::core::NoopMeasure,
            &inputs,
        );
        assert!(app.ui_shell.search_bar_has_keyboard_focus());
        app
    }

    #[test]
    fn native_menu_select_all_targets_focused_search_field() {
        for replace in [false, true] {
            let mut app = app_with_focused_search(replace);
            assert!(
                app.route_search_menu_action(crate::native_menu::MenuAction::SelectAll).is_some(),
                "native selection must be consumed by the focused search field"
            );
            let action = app.ui_shell.forward_ime(
                ui::core::Event::ImeCommit("新内容".to_owned()),
                app.editor_runtime.theme(),
                app.ui_metrics().dpi,
            );
            let expected = if replace {
                ui::search_bar::SearchBarAction::ReplaceQueryChanged("新内容".to_owned())
            } else {
                ui::search_bar::SearchBarAction::QueryChanged("新内容".to_owned())
            };
            assert_eq!(action, Some(ui::core::WidgetAction::SearchBar(expected)));
            assert_eq!(document_texts(&app), ["正文保持原样"]);
        }
    }

    #[test]
    fn native_edit_menu_falls_through_when_editor_has_focus() {
        let mut app = app_with_focused_search(false);
        app.ui_shell.focus_editor();
        assert!(app.route_search_menu_action(crate::native_menu::MenuAction::Paste).is_none());
    }

    #[cfg(feature = "markdown")]
    fn app_with_mindmap_inspector(source: &str) -> App {
        let mut app = App::new(None);
        push_mindmap_inspector_tab(&mut app, source);
        app.switch_workspace_for_test(0);
        app.sync_plugin_state();
        app
    }

    #[cfg(feature = "markdown")]
    fn push_mindmap_inspector_tab(app: &mut App, source: &str) {
        app.push_entry_for_test(
            DocumentFixture::new(source.lines().map(str::to_owned).collect(), 80, 10.0),
            Box::new(textora_mindmap::mindmap_view::MindmapView::new()),
        );
    }

    fn inspector_action(
        app: &App,
        intent: impl FnOnce(
            &ui::inspector_panel::InspectorPanelDescriptor,
        ) -> ui::inspector_panel::InspectorPanelIntent,
    ) -> AppAction {
        use ui::inspector_panel::InspectorPanelAction;

        let panel = app.editor_runtime.active_inspector_panel().expect("inspector must be open");
        let action = InspectorPanelAction {
            panel_id: panel.descriptor.id.clone(),
            intent: intent(&panel.descriptor),
        };
        AppAction::InspectorPanel(panel.stamp.stamp(action).expect("action uses mounted panel ID"))
    }

    fn inspector_select(app: &App, item_id: &str) -> AppAction {
        use ui::inspector_panel::{InspectorItemTarget, InspectorPanelIntent, ItemId};

        inspector_action(app, |descriptor| {
            InspectorPanelIntent::SelectItem(InspectorItemTarget {
                group_id: descriptor.presets.id.clone(),
                item_id: ItemId(item_id.to_owned()),
            })
        })
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn mindmap_inspector_toggle_opens_expanded_and_closes_active_panel() {
        let mut app = app_with_mindmap_inspector("# Root");
        assert_eq!(app.reduce_action(AppAction::ToggleInspectorPanel, None), AppEffect::REDRAW);
        let panel = app.editor_runtime.active_inspector_panel().expect("opened panel");
        assert!(panel.presentation.presets_expanded);

        assert_eq!(app.reduce_action(AppAction::ToggleInspectorPanel, None), AppEffect::REDRAW);
        assert!(app.editor_runtime.active_inspector_panel().is_none());
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn switching_tabs_restores_each_inspector_panel_session() {
        use ui::inspector_panel::InspectorPanelIntent;

        let mut app = app_with_mindmap_inspector("# First");
        push_mindmap_inspector_tab(&mut app, "# Second");
        app.switch_workspace_for_test(0);
        let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);
        let collapse = inspector_action(&app, |descriptor| {
            InspectorPanelIntent::ToggleGroup(descriptor.presets.id.clone())
        });
        assert_eq!(app.reduce_action(collapse, None), AppEffect::REDRAW);

        app.switch_workspace_for_test(1);
        assert!(app.editor_runtime.active_inspector_panel().is_none());
        let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);
        let second = app.editor_runtime.active_inspector_panel().expect("second panel");
        assert!(second.presentation.presets_expanded);
        let close = inspector_action(&app, |_| InspectorPanelIntent::Close);
        assert_eq!(app.reduce_action(close, None), AppEffect::REDRAW);
        assert!(app.editor_runtime.active_inspector_panel().is_none());

        app.switch_workspace_for_test(0);
        let first = app.editor_runtime.active_inspector_panel().expect("first panel restored");
        assert!(!first.presentation.presets_expanded);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn mindmap_inspector_selection_commits_once_refreshes_panel_and_undoes_once() {
        let source = "# Root";
        let mut app = app_with_mindmap_inspector(source);
        let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);
        let before = app.active_tab_session().expect("active tab").document().generation();
        let selection = inspector_select(&app, "tide");
        let repeated_selection = inspector_select(&app, "tide");

        let effect = app.reduce_action(selection, None);
        assert!(effect.redraw && effect.reshape);
        assert_eq!(app.reduce_action(repeated_selection, None), AppEffect::NONE);
        let tab = app.active_tab_session().expect("active tab");
        assert!(tab.document().full_text().contains("theme = \"tide\""));
        assert_eq!(tab.document().generation(), before + 1);
        assert!(tab.document().dirty);
        let panel = app.editor_runtime.active_inspector_panel().expect("panel stays open");
        assert_eq!(
            panel.descriptor.presets.selected_item,
            Some(ui::inspector_panel::ItemId("tide".into()))
        );

        let _ = app.dispatch_semantic_edit(ui::plugin::SemanticEditCommand::Undo);
        let tab = app.active_tab_session().expect("active tab");
        assert_eq!(tab.document().full_text(), source);
    }

    #[test]
    fn table_picker_confirmation_executes_one_semantic_insert_table_transaction() {
        let directory = tempfile::tempdir().expect("table-picker test directory should exist");
        let path = directory.path().join("insert-table.md");
        std::fs::write(&path, "existing paragraph")
            .expect("table-picker fixture should be writable");
        let mut app = App::new(None);
        let _ = app.open_file(&path).expect("Markdown fixture should open");
        let generation_before = app
            .active_tab_session()
            .expect("Markdown tab should be active")
            .document()
            .generation();
        let mut picker = ui::table_picker::TablePickerWidget::new();
        picker.set_input(ui::table_picker::TablePickerInput { open: true });
        app.ui_shell.push_overlay_with_policy(
            Box::new(ui::modal_frame::ModalFrame::new("插入表格", Box::new(picker))),
            ui::OverlayLayout::Centered {
                preferred_size: (300.0, 270.0),
                min_margin: 16.0,
                max_width_ratio: 0.92,
                max_height_ratio: 0.90,
            },
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::EscapeOrExplicit,
        );

        let effect = app.reduce_action(
            AppAction::TablePicker(ui::table_picker::TablePickerAction::Confirmed(
                ui::table_picker::TableSize { columns: 2, rows: 3 },
            )),
            None,
        );

        let active_document = app.active_tab_session().expect("Markdown tab should remain active");
        assert_eq!(active_document.document().generation(), generation_before + 1);
        assert!(!app.ui_shell.active_overlay_is_modal());
        let parsed =
            textora_markdown::parser::parse_markdown(&active_document.document().full_text());
        assert!(parsed.events.iter().any(|event| matches!(
            event,
            textora_markdown::parser::MarkdownEvent::Start(
                textora_markdown::parser::MarkdownTag::Table(alignments)
            ) if alignments.len() == 2
        )));
        assert!(effect.redraw);
    }

    #[test]
    #[cfg(feature = "markdown")]
    fn table_structure_menu_action_is_one_undoable_saved_transaction() {
        let directory = tempfile::tempdir().expect("table-structure test directory should exist");
        let path = directory.path().join("table-structure.md");
        let original = "| first | second |\n| --- | --- |\n| left | right |";
        std::fs::write(&path, original).expect("table-structure fixture should be writable");
        let mut app = App::new(None);
        let _ = app.open_file(&path).expect("Markdown fixture should open");
        let generation_before = app
            .active_tab_session()
            .expect("Markdown tab should be active")
            .document()
            .generation();
        let cursor_byte = original.find("right").expect("clicked second-column cell exists");

        let effect = app.reduce_action(
            AppAction::TableStructureEdit {
                command: ui::plugin::TableStructureCommand::SetColumnAlignment(
                    ui::plugin::TableColumnAlignment::Right,
                ),
                cursor_byte,
                source_generation: generation_before,
            },
            None,
        );

        let expected = "| first | second |\n| --- | ---: |\n| left | right |";
        let active = app.active_tab_session().expect("Markdown tab should remain active");
        assert_eq!(active.document().full_text(), expected);
        assert_eq!(active.document().generation(), generation_before + 1);
        assert!(effect.redraw);

        let _ = app.dispatch_semantic_edit(ui::plugin::SemanticEditCommand::Undo);
        assert_eq!(
            app.active_tab_session().expect("Markdown tab remains").document().full_text(),
            original
        );
        let _ = app.dispatch_semantic_edit(ui::plugin::SemanticEditCommand::Redo);
        assert_eq!(
            app.active_tab_session().expect("Markdown tab remains").document().full_text(),
            expected
        );

        let tab_id = app.editor_tab_id_at(0).expect("saved Markdown tab has an id");
        let save =
            app.editor_runtime.prepare_save(tab_id).expect("changed Markdown tab should save");
        std::fs::write(&save.path, save.serialized_contents)
            .expect("prepared Markdown save should write to disk");
        let mut reopened_app = App::new(None);
        let _ = reopened_app.open_file(&path).expect("saved Markdown file should reopen");
        let reopened = reopened_app
            .active_tab_session()
            .expect("reopened Markdown tab should be active")
            .document()
            .full_text();
        assert_eq!(reopened, expected);
        let parsed = textora_markdown::parser::parse_markdown(&reopened);
        assert!(parsed.events.iter().any(|event| matches!(
            event,
            textora_markdown::parser::MarkdownEvent::Start(
                textora_markdown::parser::MarkdownTag::Table(alignments)
            ) if alignments.len() == 2 && alignments[0] != alignments[1]
        )));
    }

    #[test]
    fn table_picker_cancellation_closes_without_changing_the_document() {
        let directory = tempfile::tempdir().expect("table-picker test directory should exist");
        let path = directory.path().join("cancel-table-picker.md");
        std::fs::write(&path, "existing paragraph")
            .expect("table-picker fixture should be writable");
        let mut app = App::new(None);
        let _ = app.open_file(&path).expect("Markdown fixture should open");
        let generation_before = app
            .active_tab_session()
            .expect("Markdown tab should be active")
            .document()
            .generation();
        let source_before = app
            .active_tab_session()
            .expect("Markdown tab should be active")
            .document()
            .full_text()
            .to_owned();
        app.ui_shell.push_overlay_with_policy(
            Box::new(ui::modal_frame::ModalFrame::new(
                "插入表格",
                Box::new(ui::table_picker::TablePickerWidget::new()),
            )),
            ui::OverlayLayout::Centered {
                preferred_size: (300.0, 270.0),
                min_margin: 16.0,
                max_width_ratio: 0.92,
                max_height_ratio: 0.90,
            },
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::EscapeOrExplicit,
        );

        let effect = app.reduce_action(
            AppAction::TablePicker(ui::table_picker::TablePickerAction::Cancelled),
            None,
        );

        assert!(!app.ui_shell.active_overlay_is_modal());
        let active_document = app.active_tab_session().expect("Markdown tab should remain active");
        assert_eq!(active_document.document().full_text(), source_before);
        assert_eq!(active_document.document().generation(), generation_before);
        assert!(effect.redraw);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn mindmap_inspector_unknown_invalid_and_current_selections_do_not_write() {
        use ui::inspector_panel::ItemId;

        for (source, disabled, selected, item) in [
            ("```toml mindmap\ntheme = \"retired\"\n```\n# Root", false, None, "retired"),
            ("```toml mindmap\ntheme = [\n```\n# Root", true, None, "tide"),
            ("```toml mindmap\ntheme = \"tide\"\n```\n# Root", false, Some("tide"), "tide"),
        ] {
            let mut app = app_with_mindmap_inspector(source);
            let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);
            let panel = app.editor_runtime.active_inspector_panel().expect("panel opens");
            assert_eq!(panel.descriptor.selection_disabled, disabled);
            assert_eq!(
                panel.descriptor.presets.selected_item,
                selected.map(|id| ItemId(id.into()))
            );
            if selected.is_none() {
                assert!(panel.descriptor.notice.is_some());
                assert!(panel.descriptor.presets.fallback_summary_item.is_some());
            }
            let generation = app.active_tab_session().expect("active tab").document().generation();
            let action = inspector_select(&app, item);

            assert_eq!(app.reduce_action(action, None), AppEffect::NONE);
            let tab = app.active_tab_session().expect("active tab");
            assert_eq!(tab.document().full_text(), source);
            assert_eq!(tab.document().generation(), generation);
            assert!(!tab.document().dirty);
        }
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn mindmap_inspector_rejects_expired_layout_action_after_reopening() {
        let source = "# Root";
        let mut app = app_with_mindmap_inspector(source);
        let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);
        let stale = inspector_select(&app, "tide");
        let generation = app.active_tab_session().expect("active tab").document().generation();
        let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);
        let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);

        assert_eq!(app.reduce_action(stale, None), AppEffect::NONE);
        let tab = app.active_tab_session().expect("active tab");
        assert_eq!(tab.document().full_text(), source);
        assert_eq!(tab.document().generation(), generation);
        assert!(!tab.document().dirty);
    }

    #[test]
    fn source_view_has_no_inspector_permission() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["source".into()], 80, 10.0),
            Box::new(crate::plugins::editor::EditorPlugin::new()),
        );
        app.switch_workspace_for_test(0);

        assert!(!app.editor_runtime.active_inspector_available());
        assert_eq!(app.reduce_action(AppAction::ToggleInspectorPanel, None), AppEffect::NONE);
        assert!(app.editor_runtime.active_inspector_panel().is_none());
    }

    struct StaleInspectorPlanPlugin;

    impl ui::plugin::ViewPlugin for StaleInspectorPlanPlugin {
        fn name(&self) -> &str {
            "stale-inspector-plan"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::Source
        }

        fn render(
            &mut self,
            _document: &dyn textora_core::document::DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }
    }

    impl ui::plugin::ViewPluginCapabilities for StaleInspectorPlanPlugin {
        fn inspector_panel_provider(&self) -> Option<&dyn ui::plugin::InspectorPanelProvider> {
            Some(self)
        }
    }

    impl ui::plugin::InspectorPanelProvider for StaleInspectorPlanPlugin {
        fn inspector_descriptor(
            &self,
            _source_generation: u32,
        ) -> Option<ui::inspector_panel::InspectorPanelDescriptor> {
            use ui::inspector_panel::{
                GroupId, InspectorPanelDescriptor, InspectorPresetGroup, InspectorPresetItem,
                ItemId, PanelId,
            };

            Some(InspectorPanelDescriptor {
                id: PanelId("appearance".into()),
                presets: InspectorPresetGroup {
                    id: GroupId("colors".into()),
                    items: vec![InspectorPresetItem {
                        id: ItemId("tide".into()),
                        display_name: "Tide".into(),
                        canvas_background: [0.0; 4],
                        root_fill: [0.0; 4],
                        branch_colors: Vec::new(),
                        disabled: false,
                    }],
                    ..InspectorPresetGroup::default()
                },
                ..InspectorPanelDescriptor::default()
            })
        }

        fn plan_inspector_action(
            &self,
            _panel_id: &ui::inspector_panel::PanelId,
            _target: &ui::inspector_panel::InspectorItemTarget,
            source_generation: u32,
        ) -> ui::plugin::EditPlan {
            ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
                source_generation.wrapping_sub(1),
                0..4,
                "changed".into(),
                7,
            ))
        }
    }

    #[test]
    fn inspector_rejects_a_stale_transaction_from_the_active_provider() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["root".into()], 80, 10.0),
            Box::new(StaleInspectorPlanPlugin),
        );
        app.switch_workspace_for_test(0);
        let _ = app.reduce_action(AppAction::ToggleInspectorPanel, None);
        let generation = app.active_tab_session().expect("active tab").document().generation();
        let action = inspector_select(&app, "tide");

        assert_eq!(app.reduce_action(action, None), AppEffect::NONE);
        let document = app.active_tab_session().expect("active tab").document();
        assert_eq!(document.full_text(), "root");
        assert_eq!(document.generation(), generation);
        assert!(!document.dirty);
    }

    struct WysiwygBoundsPlugin;

    #[test]
    fn plugin_bounds_projected_editor_rect_reserves_mmap_style_panel_before_scrollbar() {
        let mut inputs = crate::ui_shell::ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 12.0,
            toc_visible: false,
            toc_thickness: 0.0,
            metrics: ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0),
            sidebar_settings: Default::default(),
        };

        let without_panel = projected_editor_rect(1_200.0, 800.0, &inputs, 0.0);
        let panel_thickness = ui::inspector_panel::PANEL_WIDTH_LOGICAL;
        let with_panel = projected_editor_rect(1_200.0, 800.0, &inputs, panel_thickness);

        assert_eq!(without_panel, ui::core::geom::Rect::new(0.0, 36.0, 1_188.0, 740.0));
        assert_eq!(with_panel, ui::core::geom::Rect::new(0.0, 36.0, 908.0, 740.0));
        inputs.scrollbar_thickness = 0.0;
        assert_eq!(projected_editor_rect(1_200.0, 800.0, &inputs, panel_thickness).w, 920.0);
    }

    impl ui::plugin::ViewPlugin for WysiwygBoundsPlugin {
        fn name(&self) -> &str {
            "wysiwyg_bounds"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::RichText
        }

        fn render(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }
    }

    impl ui::plugin::ViewPluginCapabilities for WysiwygBoundsPlugin {}

    struct CanvasViewportRouterPlugin;

    impl ui::plugin::ViewPlugin for CanvasViewportRouterPlugin {
        fn name(&self) -> &str {
            "canvas_viewport_router"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::Canvas
        }

        fn render(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }
    }

    impl ui::plugin::ViewPluginCapabilities for CanvasViewportRouterPlugin {}

    fn app_with_prepared_canvas_viewport() -> App {
        let mut app = App::new(None);
        let document = DocumentFixture::new(vec!["canvas".to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(CanvasViewportRouterPlugin));
        app.switch_workspace_for_test(0);
        let tab = app.active_tab_session_mut().expect("test canvas tab must be active");
        let snapshot = tab.runtime.canvas_viewport.prepare(
            ui::plugin::CanvasContentMetrics {
                content_bounds: ui::core::geom::Rect::new(0.0, 0.0, 5_000.0, 5_000.0),
                focus_anchor: None,
            },
            ui::core::geom::Rect::new(0.0, 0.0, 1_000.0, 800.0),
            ui::canvas::CanvasViewportConfig::for_dpi(1.0),
        );
        assert!(snapshot.is_some(), "test canvas viewport must prepare a snapshot");
        app
    }

    #[test]
    fn canvas_scrollbar_routes_drag_and_page_actions_by_axis() {
        let mut app = app_with_prepared_canvas_viewport();

        assert_eq!(
            app.dispatch_canvas_scrollbar_action(
                ui::canvas::CanvasAxis::Horizontal,
                ui::scrollbar::ScrollbarAction::DragTo(320.0),
            ),
            AppEffect::REDRAW
        );
        assert_eq!(
            app.active_tab_session()
                .expect("test canvas tab must remain active")
                .runtime
                .canvas_viewport
                .snapshot()
                .expect("prepared viewport snapshot must remain available")
                .scroll
                .x,
            320.0
        );

        assert_eq!(
            app.dispatch_canvas_scrollbar_action(
                ui::canvas::CanvasAxis::Vertical,
                ui::scrollbar::ScrollbarAction::PageDown,
            ),
            AppEffect::REDRAW
        );
        assert_eq!(
            app.active_tab_session()
                .expect("test canvas tab must remain active")
                .runtime
                .canvas_viewport
                .snapshot()
                .expect("prepared viewport snapshot must remain available")
                .scroll
                .y,
            800.0
        );
    }

    #[test]
    fn canvas_scrollbar_redraw_only_actions_do_not_change_viewport() {
        let mut app = app_with_prepared_canvas_viewport();
        let before = app
            .active_tab_session()
            .expect("test canvas tab must remain active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared viewport snapshot must remain available");

        assert_eq!(
            app.dispatch_canvas_scrollbar_action(
                ui::canvas::CanvasAxis::Horizontal,
                ui::scrollbar::ScrollbarAction::StartDrag,
            ),
            AppEffect::REDRAW
        );
        let after = app
            .active_tab_session()
            .expect("test canvas tab must remain active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared viewport snapshot must remain available");
        assert_eq!(after, before);
    }

    #[test]
    fn canvas_viewport_actions_are_noops_without_a_canvas_snapshot() {
        let mut app = App::new(None);
        let document = DocumentFixture::new(vec!["canvas".to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(CanvasViewportRouterPlugin));
        app.switch_workspace_for_test(0);

        assert_eq!(
            app.dispatch_canvas_scrollbar_action(
                ui::canvas::CanvasAxis::Horizontal,
                ui::scrollbar::ScrollbarAction::HoverChanged(true),
            ),
            AppEffect::NONE
        );
        assert_eq!(
            app.dispatch_canvas_pinch(0.25, ui::canvas::CanvasPoint::new(500.0, 400.0)),
            AppEffect::NONE
        );
    }

    #[test]
    fn canvas_pinch_zooms_a_prepared_canvas_viewport() {
        let mut app = app_with_prepared_canvas_viewport();
        let anchor = ui::canvas::CanvasPoint::new(500.0, 400.0);
        let zoom_before = app
            .active_tab_session()
            .expect("test canvas tab must remain active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared viewport snapshot must remain available")
            .zoom;

        assert_eq!(app.dispatch_canvas_pinch(0.25, anchor), AppEffect::REDRAW);
        let zoom_after = app
            .active_tab_session()
            .expect("test canvas tab must remain active")
            .runtime
            .canvas_viewport
            .snapshot()
            .expect("prepared viewport snapshot must remain available")
            .zoom;
        assert!(zoom_after > zoom_before);
    }

    #[test]
    fn wysiwyg_initial_plugin_bounds_do_not_depend_on_cached_shell_layout() {
        let mut app = App::new(None);
        let doc = DocumentFixture::new(vec!["hello world".to_string()], 80, 10.0);
        app.push_entry_for_test(doc, Box::new(WysiwygBoundsPlugin));
        app.switch_workspace_for_test(0);

        assert_eq!(app.ui_shell.editor_rect(), ui::core::geom::Rect::ZERO);

        let bounds = app.plugin_render_bounds();

        assert!(
            bounds.w > MIN_PLUGIN_VIEWPORT_PX
                && bounds.x > app.ui_shell.sidebar_editor_left_offset(app.ui_metrics().dpi),
            "initial WYSIWYG render bounds should use current screen width before shell layout, got {bounds:?}"
        );
    }

    #[test]
    fn wysiwyg_plugin_bounds_account_for_sidebar_during_first_shell_frame() {
        let mut app = App::new(None);
        app.editor_runtime
            .edit_settings(|settings| settings.view_mode = ui::view_mode::ViewMode::Sidebar);
        let doc = DocumentFixture::new(vec!["hello world".to_string()], 80, 10.0);
        app.push_entry_for_test(doc, Box::new(WysiwygBoundsPlugin));
        app.switch_workspace_for_test(0);

        let inputs = app.build_shell_inputs();
        let theme = ui::theme::test_theme();
        let mut measure = ui::core::NoopMeasure;
        app.ui_shell.update_frame(
            ui::core::Screen::new(800.0, 600.0),
            &theme,
            &mut measure,
            &inputs,
        );
        assert_eq!(app.ui_shell.editor_rect().x, 0.0);

        let bounds = app.plugin_render_bounds();

        assert!(
            bounds.x > app.ui_shell.sidebar_editor_left_offset(app.ui_metrics().dpi),
            "initial WYSIWYG render bounds should reserve the pinned sidebar, got {bounds:?}"
        );
    }

    #[test]
    fn app_action_tab_switch_cancels_started_canvas_drag_once() {
        let (mut app, state) = app_with_canvas_drag_tabs();
        start_canvas_drag(&mut app);

        let effect = app.dispatch_tab_switch(app.editor_tab_id_at(1).unwrap());

        assert_eq!(app.active_editor_index(), Some(1));
        assert!(effect.redraw);
        assert_eq!(cancel_request_count(&state.borrow()), 1);
        assert_eq!(canvas_drag_test_support::cancelled_sources(&state.borrow()), ["abc"]);
        assert_eq!(document_texts(&app), ["abc", "def"]);

        let _ = app.dispatch_tab_switch(app.editor_tab_id_at(0).unwrap());

        assert_eq!(cancel_request_count(&state.borrow()), 1);
    }
}
