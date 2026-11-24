use crate::app::App;
use crate::app_effect::AppEffect;
use appkit_shell::editor_runtime::{EditorNotification, EditorOutcome};
#[cfg(feature = "markdown")]
use ui::plugin::EditHitTarget;
use winit::event::ElementState;

impl App {
    pub(crate) fn dispatch_editor_mouse_input(
        &mut self,
        state: ElementState,
        px: f32,
        py: f32,
    ) -> AppEffect {
        let button = ui::MouseButton::Left;
        let event = if state.is_pressed() {
            if self.active_handles_own_rendering() {
                self.sync_plugin_state();
            }
            ui::Event::MouseDown { px, py, button }
        } else {
            ui::Event::MouseUp { px, py, button }
        };
        let outcome = self.editor_runtime.handle_pointer_event(self.editor_input_context(), &event);
        self.apply_pointer_outcome(outcome.editor)
    }

    pub(crate) fn dispatch_editor_cursor_moved(
        &mut self,
        px: f32,
        py: f32,
        use_editor_cursor: bool,
    ) -> AppEffect {
        let outcome = self
            .editor_runtime
            .handle_pointer_event(self.editor_input_context(), &ui::Event::MouseMove { px, py });
        if use_editor_cursor && let Some(window) = self.editor_runtime.window() {
            window.set_cursor(outcome.cursor_icon.unwrap_or(winit::window::CursorIcon::Default));
        }
        self.apply_pointer_outcome(outcome.editor)
    }

    fn apply_pointer_outcome(&self, outcome: EditorOutcome) -> AppEffect {
        for notification in outcome.notifications {
            if let EditorNotification::DirtyChanged { dirty, .. } = notification {
                self.update_document_edited(dirty);
            }
        }
        outcome.shell_effect
    }

    pub(crate) fn cancel_editor_pointer_capture(&mut self) -> AppEffect {
        let outcome = self.editor_runtime.cancel_pointer_capture();
        self.apply_pointer_outcome(outcome)
    }

    #[cfg(all(test, feature = "markdown"))]
    pub(crate) fn hit_test_canvas_control(
        &self,
        px: f32,
        py: f32,
    ) -> Option<std::ops::Range<usize>> {
        self.active_tab_session()?.hit_test_canvas_control(ui::canvas::CanvasPoint::new(px, py))
    }

    #[cfg(feature = "markdown")]
    pub(crate) fn query_plugin_edit_hit_target(
        &self,
        px: f32,
        py: f32,
    ) -> Option<Option<EditHitTarget>> {
        let bounds = self.plugin_render_bounds();
        self.active_tab_session()?.hit_test_edit_target(px, py, bounds.x, bounds.y)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::app_effect::AppEffect;
    use crate::dispatch::wysiwyg::semantic_test_support::{
        RecordedSyncMessage, SemanticPluginState, SemanticQueryResponse, app_with_semantic_plugin,
    };
    use crate::document_fixture::DocumentFixture;

    use std::cell::RefCell;
    use std::rc::Rc;
    use ui::canvas::CanvasPoint;
    use ui::plugin::{
        CanvasDragPhase, CanvasDragRequest, CanvasDragResponse, EditHitTarget, EditPlan,
        EditTransaction, ViewPlugin,
    };
    use winit::event::ElementState;

    #[derive(Clone, Debug)]
    struct CanvasDragTestState {
        hit_target: EditHitTarget,
        responses: Vec<(CanvasDragPhase, CanvasDragResponse)>,
        requests: Vec<CanvasDragRequest>,
    }

    impl CanvasDragTestState {
        fn with_response(response: CanvasDragResponse) -> Self {
            Self {
                hit_target: EditHitTarget::SourceObject { source_range: 1..2 },
                responses: vec![(CanvasDragPhase::Drop, response)],
                requests: Vec::new(),
            }
        }

        fn response_for(&self, phase: CanvasDragPhase) -> CanvasDragResponse {
            self.responses
                .iter()
                .find_map(|(configured_phase, response)| {
                    (*configured_phase == phase).then(|| response.clone())
                })
                .unwrap_or(CanvasDragResponse::Ignore)
        }

        fn request_count(&self, phase: CanvasDragPhase) -> usize {
            self.requests.iter().filter(|request| request.phase == phase).count()
        }
    }

    struct CanvasDragTestPlugin {
        state: Rc<RefCell<CanvasDragTestState>>,
        plugin_name: &'static str,
    }

    impl ViewPlugin for CanvasDragTestPlugin {
        fn name(&self) -> &str {
            self.plugin_name
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

    impl ui::plugin::EditPolicy for CanvasDragTestPlugin {
        fn plan_edit(&self, _: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
            ui::plugin::EditPlan::UseDefault
        }
    }

    impl ui::plugin::EditableView for CanvasDragTestPlugin {
        fn hit_test_edit_target(
            &self,
            _x: f32,
            _y: f32,
            _offset_x: f32,
            _offset_y: f32,
        ) -> Option<Option<ui::plugin::EditHitTarget>> {
            Some(Some(self.state.borrow().hit_target.clone()))
        }
    }

    impl ui::plugin::ScrollableView for CanvasDragTestPlugin {
        fn content_height(&self) -> f32 {
            1_000.0
        }
    }

    impl ui::plugin::ViewPluginCapabilities for CanvasDragTestPlugin {
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

    impl ui::plugin::CanvasView for CanvasDragTestPlugin {
        fn handle_canvas_drag(
            &mut self,
            request: CanvasDragRequest,
            _doc: &dyn textora_core::document::DocView,
        ) -> CanvasDragResponse {
            let mut state = self.state.borrow_mut();
            let response = state.response_for(request.phase);
            state.requests.push(request);
            response
        }
    }

    struct CanvasControlTestState {
        edit_plan: EditPlan,
        control_range: std::ops::Range<usize>,
        planned_control_ranges: Vec<std::ops::Range<usize>>,
        pointer_positions: Vec<Option<CanvasPoint>>,
        drag_requests: Vec<CanvasDragRequest>,
    }

    struct CanvasControlTestPlugin {
        state: Rc<RefCell<CanvasControlTestState>>,
    }

    impl ViewPlugin for CanvasControlTestPlugin {
        fn name(&self) -> &str {
            "canvas_control_test"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::Canvas
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

    impl ui::plugin::ScrollableView for CanvasControlTestPlugin {
        fn content_height(&self) -> f32 {
            1_000.0
        }
    }

    impl ui::plugin::ViewPluginCapabilities for CanvasControlTestPlugin {
        fn scrollable(&self) -> Option<&dyn ui::plugin::ScrollableView> {
            Some(self)
        }

        fn allows_editing(&self) -> bool {
            true
        }

        fn canvas(&self) -> Option<&dyn ui::plugin::CanvasView> {
            Some(self)
        }

        fn canvas_mut(&mut self) -> Option<&mut dyn ui::plugin::CanvasView> {
            Some(self)
        }
    }

    impl ui::plugin::CanvasView for CanvasControlTestPlugin {
        fn hit_test_canvas_control(
            &self,
            _position: CanvasPoint,
        ) -> Option<std::ops::Range<usize>> {
            Some(self.state.borrow().control_range.clone())
        }

        fn plan_canvas_control(
            &self,
            source_range: std::ops::Range<usize>,
            _source_generation: u32,
        ) -> EditPlan {
            let mut state = self.state.borrow_mut();
            state.planned_control_ranges.push(source_range);
            state.edit_plan.clone()
        }

        fn set_canvas_pointer(&mut self, point: Option<CanvasPoint>) -> bool {
            self.state.borrow_mut().pointer_positions.push(point);
            true
        }

        fn handle_canvas_drag(
            &mut self,
            request: CanvasDragRequest,
            _doc: &dyn textora_core::document::DocView,
        ) -> CanvasDragResponse {
            self.state.borrow_mut().drag_requests.push(request);
            CanvasDragResponse::Ignore
        }
    }

    fn app_with_canvas_drag_plugin(text: &str, state: Rc<RefCell<CanvasDragTestState>>) -> App {
        let mut app = App::new(None);
        let document = DocumentFixture::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(
            document,
            Box::new(CanvasDragTestPlugin { state, plugin_name: "canvas_drag_test" }),
        );
        app.switch_workspace_for_test(0);
        app.prepare_editor_pointer_frame_for_test();
        app
    }

    fn app_with_mindmap_drag_plugin(text: &str, state: Rc<RefCell<CanvasDragTestState>>) -> App {
        let mut app = App::new(None);
        let document = DocumentFixture::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(
            document,
            Box::new(CanvasDragTestPlugin { state, plugin_name: ui::plugin::PLUGIN_MINDMAP }),
        );
        app.switch_workspace_for_test(0);
        app.prepare_editor_pointer_frame_for_test();
        app
    }

    fn app_with_canvas_control_plugin(
        text: &str,
        state: Rc<RefCell<CanvasControlTestState>>,
    ) -> App {
        let mut app = App::new(None);
        let document = DocumentFixture::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(CanvasControlTestPlugin { state }));
        app.switch_workspace_for_test(0);
        app.prepare_editor_pointer_frame_for_test();
        app
    }

    fn active_text(app: &App) -> String {
        app.active_tab_session().expect("active entry").document().full_text()
    }

    #[cfg(feature = "markdown")]
    fn render_mindmap_canvas_for_control_test(app: &mut App) -> ui::core::paint::DrawList {
        let font_size = app.ui_metrics().font_size;
        let dpi = app.ui_metrics().dpi;
        let theme = app.editor_runtime.theme().clone();
        let mut shaper = app
            .editor_runtime
            .new_shaper(font_size, "")
            .unwrap_or_else(|| shaping::Shaper::new().expect("test shaper should initialize"));
        let snapshot = app
            .sync_and_prepare_canvas_frame(&mut shaper)
            .expect("mindmap source must resolve a canvas viewport");
        let mut tab = app.active_tab_session_mut().expect("active mindmap tab");
        tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi)
    }

    #[test]
    fn canvas_control_press_applies_edit_plan_without_starting_drag() {
        let state = Rc::new(RefCell::new(CanvasControlTestState {
            edit_plan: EditPlan::Apply(EditTransaction::replace(1, 1..2, "TRUE".into(), 5)),
            control_range: 1..4,
            planned_control_ranges: Vec::new(),
            pointer_positions: Vec::new(),
            drag_requests: Vec::new(),
        }));
        let mut app = app_with_canvas_control_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);

        let state = state.borrow();
        assert_eq!(state.planned_control_ranges, vec![1..4]);
        assert!(state.drag_requests.is_empty());
        assert_eq!(active_text(&app), "aTRUEc");
        assert_eq!(
            app.editor_runtime.pointer_capture(),
            appkit_shell::editor_runtime::MouseCapture::None
        );
    }

    #[test]
    fn canvas_control_pointer_move_notifies_plugin() {
        let state = Rc::new(RefCell::new(CanvasControlTestState {
            edit_plan: EditPlan::Consume,
            control_range: 1..4,
            planned_control_ranges: Vec::new(),
            pointer_positions: Vec::new(),
            drag_requests: Vec::new(),
        }));
        let mut app = app_with_canvas_control_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();
        let point = CanvasPoint::new(bounds.x + 12.0, bounds.y + 20.0);

        let effect = app.dispatch_editor_cursor_moved(point.x, point.y, false);

        assert_eq!(state.borrow().pointer_positions, vec![Some(point)]);
        assert_eq!(effect, AppEffect::REDRAW);
    }

    #[test]
    #[cfg(feature = "markdown")]
    fn canvas_control_end_to_end() {
        let source = "# Root\n## Parent\n### Child\n#### Grandchild";
        let tree = textora_mindmap::mmf::parser::parse(source).expect("mmap fixture must parse");
        let parent_range = tree.root().children[0].subtree_source_range.clone();
        let root_range = tree.root().subtree_source_range.clone();
        let child_range = tree.root().children[0].children[0].subtree_source_range.clone();

        let mut app = App::new(None);
        let document = DocumentFixture::new(source.lines().map(str::to_owned).collect(), 80, 10.0);
        app.push_entry_for_test(
            document,
            Box::new(textora_mindmap::mindmap_view::MindmapView::new()),
        );
        app.switch_workspace_for_test(0);
        app.prepare_editor_pointer_frame_for_test();
        let _ = render_mindmap_canvas_for_control_test(&mut app);

        let bounds = app.plugin_render_bounds();
        let mut parent_control_point = None;
        let mut root_control_seen = false;
        for y in (0..=bounds.h.ceil() as usize).step_by(4) {
            for x in (0..=bounds.w.ceil() as usize).step_by(4) {
                let point = (bounds.x + x as f32, bounds.y + y as f32);
                let Some(source_range) = app.hit_test_canvas_control(point.0, point.1) else {
                    continue;
                };
                if source_range == parent_range {
                    parent_control_point = Some(point);
                }
                if source_range == root_range {
                    root_control_seen = true;
                }
            }
        }
        assert!(parent_control_point.is_some(), "parent control must be hit-testable");
        assert!(!root_control_seen, "root must not expose a collapse control");

        let (control_x, control_y) = parent_control_point.expect("parent control point");
        let _ = app.dispatch_editor_mouse_input(ElementState::Pressed, control_x, control_y);
        let collapsed_source = active_text(&app);
        assert!(collapsed_source.contains("collapsed = true"));

        let draw_list = render_mindmap_canvas_for_control_test(&mut app);
        let rendered_text = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, .. } => Some(layout.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(rendered_text.contains(&"Parent"));
        assert!(rendered_text.contains(&"2"));
        assert!(!rendered_text.contains(&" · 2"));

        let mut child_target_seen = false;
        let bounds = app.plugin_render_bounds();
        for y in (0..=bounds.h.ceil() as usize).step_by(4) {
            for x in (0..=bounds.w.ceil() as usize).step_by(4) {
                let point = (bounds.x + x as f32, bounds.y + y as f32);
                let target = app.query_plugin_edit_hit_target(point.0, point.1);
                if matches!(
                    target,
                    Some(Some(EditHitTarget::SourceObject { source_range })) if source_range == child_range
                ) {
                    child_target_seen = true;
                }
            }
        }
        assert!(!child_target_seen, "collapsed descendants must not be drag candidates");
    }

    #[test]
    fn canvas_drag_starts_after_threshold_and_applies_one_drop_transaction() {
        let state = Rc::new(RefCell::new(CanvasDragTestState::with_response(
            CanvasDragResponse::Apply(EditTransaction::replace(1, 1..2, "Z".into(), 2)),
        )));
        let mut app = app_with_mindmap_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        let Some(mut entry) = app.active_tab_session_mut() else {
            panic!("canvas drag test requires an active entry");
        };
        entry.document_mut().cursor_move_to_offset(2);
        entry.document_mut().cursor_mut().selection_anchor = Some(1);

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 6.0, bounds.y + 6.0, false);
        assert!(state.borrow().requests.is_empty());

        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);
        let _ = app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
        );

        assert_eq!(active_text(&app), "aZc");
        assert_eq!(state.borrow().request_count(CanvasDragPhase::Drop), 1);
        let state = state.borrow();
        assert!(state.requests.iter().all(|request| {
            request.pressed_x == bounds.x + 4.0 && request.pressed_y == bounds.y + 4.0
        }));
    }

    #[test]
    fn mindmap_canvas_drag_starts_without_prior_selection() {
        let state =
            Rc::new(RefCell::new(CanvasDragTestState::with_response(CanvasDragResponse::Ignore)));
        let mut app = app_with_mindmap_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);

        assert_eq!(state.borrow().request_count(CanvasDragPhase::Start), 1);
    }

    #[test]
    fn canvas_drag_does_not_start_for_text_caret_press() {
        let state = Rc::new(RefCell::new(CanvasDragTestState {
            hit_target: EditHitTarget::TextCaret { byte_offset: 1, selection_scope: None },
            responses: Vec::new(),
            requests: Vec::new(),
        }));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);

        assert_ne!(
            app.editor_runtime.pointer_capture(),
            appkit_shell::editor_runtime::MouseCapture::CanvasDrag
        );
    }

    #[test]
    fn canvas_drag_ignored_drop_redraws_without_modifying_document() {
        let state =
            Rc::new(RefCell::new(CanvasDragTestState::with_response(CanvasDragResponse::Ignore)));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);
        let release_effect = app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
        );

        assert_eq!(release_effect, AppEffect::REDRAW);
        assert_eq!(active_text(&app), "abc");
    }

    #[test]
    fn canvas_drag_cancel_sends_cancel_without_executing_transaction() {
        let state = Rc::new(RefCell::new(CanvasDragTestState::with_response(
            CanvasDragResponse::Apply(EditTransaction::replace(0, 1..2, "Z".into(), 2)),
        )));
        let mut app = app_with_canvas_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);
        let _ = app.cancel_editor_pointer_capture();

        assert_eq!(active_text(&app), "abc");
        assert_eq!(state.borrow().request_count(CanvasDragPhase::Cancel), 1);
    }

    #[test]
    fn canvas_drag_apply_before_drop_does_not_modify_document() {
        let transaction = EditTransaction::replace(1, 1..2, "Z".into(), 2);
        let state = Rc::new(RefCell::new(CanvasDragTestState {
            hit_target: EditHitTarget::SourceObject { source_range: 1..2 },
            responses: vec![
                (CanvasDragPhase::Start, CanvasDragResponse::Apply(transaction.clone())),
                (CanvasDragPhase::Update, CanvasDragResponse::Apply(transaction.clone())),
                (CanvasDragPhase::Cancel, CanvasDragResponse::Apply(transaction)),
            ],
            requests: Vec::new(),
        }));
        let mut app = app_with_canvas_drag_plugin("abc", state.clone());
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);
        assert_eq!(active_text(&app), "abc");

        let _ = app.dispatch_editor_cursor_moved(bounds.x + 30.0, bounds.y + 30.0, false);
        assert_eq!(active_text(&app), "abc");

        let _ = app.cancel_editor_pointer_capture();

        assert_eq!(active_text(&app), "abc");
        let state = state.borrow();
        assert_eq!(state.request_count(CanvasDragPhase::Start), 1);
        assert_eq!(state.request_count(CanvasDragPhase::Update), 1);
        assert_eq!(state.request_count(CanvasDragPhase::Cancel), 1);
    }

    #[test]
    fn canvas_drag_rejects_stale_generation_transaction() {
        let state = Rc::new(RefCell::new(CanvasDragTestState::with_response(
            CanvasDragResponse::Apply(EditTransaction::replace(0, 1..2, "Z".into(), 2)),
        )));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);
        let _ = app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
        );

        assert_eq!(active_text(&app), "abc");
    }

    #[test]
    fn canvas_drag_release_clears_session() {
        let state =
            Rc::new(RefCell::new(CanvasDragTestState::with_response(CanvasDragResponse::Ignore)));
        let mut app = app_with_canvas_drag_plugin("abc", state);
        let bounds = app.plugin_render_bounds();

        let _ =
            app.dispatch_editor_mouse_input(ElementState::Pressed, bounds.x + 4.0, bounds.y + 4.0);
        let _ = app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, false);
        let _ = app.dispatch_editor_mouse_input(
            ElementState::Released,
            bounds.x + 20.0,
            bounds.y + 20.0,
        );

        assert_ne!(
            app.editor_runtime.pointer_capture(),
            appkit_shell::editor_runtime::MouseCapture::CanvasDrag
        );
    }

    #[test]
    fn source_object_drag_ignore_keeps_object_selection() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(Some(EditHitTarget::SourceObject {
                source_range: 1..5,
            })),
            hit_test_byte_result: Some(3),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());
        app.prepare_editor_pointer_frame_for_test();
        let bounds = app.plugin_render_bounds();
        let press_x = bounds.x + 10.0;
        let press_y = bounds.y + 10.0;

        let _ = app.dispatch_editor_mouse_input(ElementState::Pressed, press_x, press_y);
        state.borrow_mut().hit_target_response =
            SemanticQueryResponse::Target(Some(EditHitTarget::TextCaret {
                byte_offset: 4,
                selection_scope: None,
            }));
        let _ = app.dispatch_editor_cursor_moved(press_x + 10.0, press_y, false);

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document().selection_range(), Some((1, 5)));
        let state = state.borrow();
        assert_eq!(state.queried_operations, ["hit_target"]);
        assert!(state.sync_messages.ends_with(&[
            RecordedSyncMessage::Anchor(Some(1)),
            RecordedSyncMessage::SelectionCursor(Some(5)),
            RecordedSyncMessage::Cursor(5),
        ]));
    }

    #[test]
    fn mouse_semantic_none_consumes_press_without_byte_fallback() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(None),
            hit_test_byte_result: Some(4),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());
        app.prepare_editor_pointer_frame_for_test();
        {
            let mut entry = app.active_tab_session_mut().expect("active entry");
            entry.document_mut().cursor_move_to_offset(3);
            entry.document_mut().cursor_mut().selection_anchor = Some(1);
        }
        let bounds = app.plugin_render_bounds();

        let _ = app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 10.0,
            bounds.y + 10.0,
        );

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document().selection_range(), Some((1, 3)));
        let state = state.borrow();
        assert_eq!(state.queried_operations, ["hit_target"]);
        assert!(!state.sync_messages.contains(&RecordedSyncMessage::Cursor(4)));
    }

    #[test]
    fn mouse_uses_byte_hit_fallback_when_semantic_query_is_unsupported() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_test_byte_result: Some(4),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());
        app.prepare_editor_pointer_frame_for_test();
        let bounds = app.plugin_render_bounds();

        let _ = app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 10.0,
            bounds.y + 10.0,
        );

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document().cursor_offset().to_usize(), 4);
        assert_eq!(entry.document().cursor().selection_anchor, Some(4));
        let state = state.borrow();
        assert!(state.queried_operations.starts_with(&["hit_target", "hit_byte"]));
        assert!(state.sync_messages.contains(&RecordedSyncMessage::Cursor(4)));
    }

    #[test]
    fn semantic_text_caret_press_keeps_an_anchor_for_drag_selection() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(Some(EditHitTarget::TextCaret {
                byte_offset: 1,
                selection_scope: None,
            })),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state);
        app.prepare_editor_pointer_frame_for_test();
        let bounds = app.plugin_render_bounds();

        let _ = app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 10.0,
            bounds.y + 10.0,
        );

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document().cursor_offset().to_usize(), 1);
        assert_eq!(entry.document().cursor().selection_anchor, Some(1));
    }

    #[test]
    fn semantic_text_caret_double_click_selects_its_source_word() {
        let state = Rc::new(RefCell::new(SemanticPluginState {
            hit_target_response: SemanticQueryResponse::Target(Some(EditHitTarget::TextCaret {
                byte_offset: 1,
                selection_scope: None,
            })),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("title other", state);
        app.prepare_editor_pointer_frame_for_test();
        let bounds = app.plugin_render_bounds();
        let x = bounds.x + 10.0;
        let y = bounds.y + 10.0;

        let _ = app.dispatch_editor_mouse_input(ElementState::Pressed, x, y);
        let _ = app.dispatch_editor_mouse_input(ElementState::Released, x, y);
        let _ = app.dispatch_editor_mouse_input(ElementState::Pressed, x, y);

        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document().selection_range(), Some((0, "title".len())));
    }
}
