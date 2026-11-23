//! Product viewport adaptation for the shared visual editor.

use crate::app::App;
use crate::app_effect::AppEffect;
#[cfg(test)]
use appkit_shell::editor_runtime::apply_target_to_document;
pub(crate) use appkit_shell::editor_runtime::{
    apply_edit_hit_target, query_visual_caret_hit, set_wysiwyg_cursor_and_selection,
    set_wysiwyg_visual_caret_and_selection,
};
use appkit_shell::input_mapper::EditCommand;

impl App {
    pub(crate) fn dispatch_wysiwyg_navigation(&mut self, command: &EditCommand) -> AppEffect {
        let reveal_viewport_height = self.plugin_viewport_h();
        let page_viewport_height =
            self.visible_height_lines(self.screen_height()) as f32 * self.ui_metrics().line_height;
        self.editor_runtime.execute_visual_navigation(
            command,
            reveal_viewport_height,
            page_viewport_height,
        )
    }

    pub(crate) fn ensure_wysiwyg_cursor_visible(&mut self) {
        let viewport_height = self.plugin_viewport_h();
        self.editor_runtime.reveal_visual_caret(viewport_height);
    }
}

#[cfg(test)]
pub(crate) mod semantic_test_support {
    use crate::app::App;
    use crate::document_fixture::DocumentFixture;

    use std::cell::RefCell;
    use std::rc::Rc;
    use ui::plugin::{EditHitTarget, PluginMessage, PluginQuery, PluginResponse, ViewPlugin};

    #[derive(Clone, Debug, Default)]
    pub(crate) enum SemanticQueryResponse {
        #[default]
        Unsupported,
        Target(Option<EditHitTarget>),
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum RecordedSyncMessage {
        Cursor(usize),
        Anchor(Option<usize>),
        SelectionCursor(Option<usize>),
    }

    #[derive(Default)]
    pub(crate) struct SemanticPluginState {
        pub(crate) move_target_response: SemanticQueryResponse,
        pub(crate) hit_target_response: SemanticQueryResponse,
        pub(crate) visual_move_result: Option<usize>,
        pub(crate) hit_test_byte_result: Option<usize>,
        pub(crate) cursor_screen_positions: Vec<(usize, f32)>,
        pub(crate) queried_operations: Vec<&'static str>,
        pub(crate) sync_messages: Vec<RecordedSyncMessage>,
    }

    pub(crate) struct SemanticTestPlugin {
        state: Rc<RefCell<SemanticPluginState>>,
    }

    impl SemanticTestPlugin {
        pub(crate) fn new(state: Rc<RefCell<SemanticPluginState>>) -> Self {
            Self { state }
        }
    }

    impl ViewPlugin for SemanticTestPlugin {
        fn name(&self) -> &str {
            "semantic_test"
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

        fn allows_editing(&self) -> bool {
            true
        }

        fn handles_own_rendering(&self) -> bool {
            true
        }

        fn query(
            &self,
            query: PluginQuery,
            _doc: &dyn textora_core::document::DocView,
        ) -> PluginResponse {
            let mut state = self.state.borrow_mut();
            match query {
                PluginQuery::NeedsSourceUpdate(_) => PluginResponse::Bool(false),
                PluginQuery::MoveEditTarget { .. } => {
                    state.queried_operations.push("move_target");
                    match &state.move_target_response {
                        SemanticQueryResponse::Unsupported => PluginResponse::None,
                        SemanticQueryResponse::Target(target) => {
                            PluginResponse::EditHitTarget(target.clone())
                        }
                    }
                }
                PluginQuery::VisualMove { .. } => {
                    state.queried_operations.push("visual_move");
                    PluginResponse::BytePosition(state.visual_move_result)
                }
                PluginQuery::HitTestEditTarget { .. } => {
                    state.queried_operations.push("hit_target");
                    match &state.hit_target_response {
                        SemanticQueryResponse::Unsupported => PluginResponse::None,
                        SemanticQueryResponse::Target(target) => {
                            PluginResponse::EditHitTarget(target.clone())
                        }
                    }
                }
                PluginQuery::HitTestByte { .. } => {
                    state.queried_operations.push("hit_byte");
                    PluginResponse::BytePosition(state.hit_test_byte_result)
                }
                PluginQuery::CursorScreenPos(byte) => {
                    let rect = state
                        .cursor_screen_positions
                        .iter()
                        .find(|(position_byte, _)| *position_byte == byte)
                        .map(|(_, x)| (*x, 0.0, 2.0, 16.0));
                    PluginResponse::CursorScreenRect(rect)
                }
                PluginQuery::ContentHeight => PluginResponse::Float(1_000.0),
                _ => PluginResponse::None,
            }
        }

        fn handle_message(
            &mut self,
            message: PluginMessage,
            _doc: &mut dyn textora_core::document::DocViewMut,
        ) -> bool {
            let recorded_message = match message {
                PluginMessage::SetCursorByte(byte_offset) => {
                    Some(RecordedSyncMessage::Cursor(byte_offset))
                }
                PluginMessage::SetSelAnchorByte(byte_offset) => {
                    Some(RecordedSyncMessage::Anchor(byte_offset))
                }
                PluginMessage::SetSelCursorByte(byte_offset) => {
                    Some(RecordedSyncMessage::SelectionCursor(byte_offset))
                }
                _ => None,
            };
            let consumed = recorded_message.is_some();
            if let Some(recorded_message) = recorded_message {
                self.state.borrow_mut().sync_messages.push(recorded_message);
            }
            consumed
        }
    }

    pub(crate) fn app_with_semantic_plugin(
        text: &str,
        state: Rc<RefCell<SemanticPluginState>>,
    ) -> App {
        let mut app = App::new(None);
        let document = DocumentFixture::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(SemanticTestPlugin::new(state)));
        app.switch_workspace_for_test(0);
        app
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::commands::execute_edit_command_v2;
    use crate::document_fixture::DocumentFixture;
    use appkit_shell::input_mapper::EditCommand;

    #[test]
    fn replace_range_empty_moves_insert_point_without_selecting_text() {
        let mut doc = DocumentFixture::new(vec!["# hello world".to_string()], 80, 10.0);
        doc.cursor_move_to_offset(4);

        let outcome = execute_edit_command_v2(
            &EditCommand::ReplaceRange { range: 13..13, text: "\nnext".into() },
            &mut doc,
            &[],
        );

        assert!(outcome.executed, "ReplaceRange with empty range should execute");
        assert_eq!(
            doc.cursor().selection_anchor,
            None,
            "ReplaceRange must not leave a selection anchor"
        );
        assert_eq!(
            doc.document_lines(),
            &["# hello world", "next"],
            "inserted text should land at range.start, not the old cursor"
        );
    }

    #[test]
    #[cfg(feature = "markdown")]
    fn markdown_augmented_insert_text_preserves_empty_separator_blocks() {
        let mut app = App::new(None);
        let mut doc = DocumentFixture::new(
            "para1\n\npara2".split('\n').map(str::to_string).collect(),
            80,
            10.0,
        );
        doc.cursor_move_to_offset(6);
        app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
        app.switch_workspace_for_test(0);
        app.sync_plugin_state();

        let effect = app.dispatch_transactional_edit_for_test(EditCommand::InsertText("中".into()));

        let doc = &app.active_tab_session().expect("active entry").document;
        assert!(effect.redraw, "WYSIWYG content must redraw after augmented text input");
        assert_eq!(doc.full_text(), "para1\n\n中\n\npara2");
        assert_eq!(doc.cursor_offset().to_usize(), "para1\n\n中".len());
    }

    /// 护栏：事务路径执行插件增强计划后不能留下 selection anchor。
    /// 老代码依赖 position_document_for_wysiwyg_replace_range 展开 anchor，
    /// 然后期望 InsertText 覆盖它；有边界情况 anchor 会被保留下来。
    /// 事务计划是原子替换，且尾部显式清空 anchor。
    #[test]
    #[cfg(feature = "markdown")]
    fn transactional_augmented_edit_leaves_no_selection_anchor() {
        let mut app = App::new(None);
        let mut doc = DocumentFixture::new(
            "para1\n\npara2".split('\n').map(str::to_string).collect(),
            80,
            10.0,
        );
        doc.cursor_move_to_offset(6);
        app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
        app.switch_workspace_for_test(0);
        app.sync_plugin_state();

        let _ = app.dispatch_transactional_edit_for_test(EditCommand::InsertText("中".into()));

        let doc = &app.active_tab_session().expect("active entry").document;
        assert!(
            doc.cursor().selection_anchor.is_none(),
            "augment must leave no selection anchor; found {:?}",
            doc.cursor().selection_anchor
        );
    }
}

#[cfg(test)]
mod semantic_target_tests {
    use super::*;
    use appkit_core::document::DocumentModel;
    use ui::plugin::EditHitTarget;

    #[test]
    fn shared_keyboard_navigation_prefers_visual_targets_with_source_fallback() {
        use super::semantic_test_support::{SemanticPluginState, app_with_semantic_plugin};
        use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext};
        use std::cell::RefCell;
        use std::rc::Rc;

        let state = Rc::new(RefCell::new(SemanticPluginState {
            visual_move_result: Some(4),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());
        let mut frame = app.editor_runtime.begin_frame().expect("editor frame should begin");
        frame
            .paint_editor(ui::Rect::new(0.0, 0.0, 640.0, 480.0))
            .expect("editor bounds should be painted");
        let context = EditorInputContext { focus: EditorFocus::Active, modal_blocked: false };

        app.editor_runtime.handle_key_input(context, ui::KeyCode::Right, ui::core::Modifiers::NONE);

        let tab = app.active_tab_session().expect("active visual tab");
        assert_eq!(tab.document.cursor_offset().to_usize(), 4);

        state.borrow_mut().visual_move_result = None;
        app.editor_runtime.handle_key_input(context, ui::KeyCode::Right, ui::core::Modifiers::NONE);

        let tab = app.active_tab_session().expect("active visual tab");
        assert_eq!(tab.document.cursor_offset().to_usize(), 5);
    }

    #[test]
    fn text_target_clears_selection_and_places_caret() {
        let mut doc = DocumentModel::from_lines(&["abcdef".into()]);

        apply_target_to_document(
            &mut doc,
            EditHitTarget::TextCaret { byte_offset: 3, selection_scope: None },
        );

        assert_eq!(doc.cursor_offset().to_usize(), 3);
        assert!(doc.cursor().selection_anchor.is_none());
    }

    #[test]
    fn source_object_target_selects_exact_source_range() {
        let mut doc = DocumentModel::from_lines(&["abcdef".into()]);

        apply_target_to_document(&mut doc, EditHitTarget::SourceObject { source_range: 1..5 });

        assert_eq!(doc.selection_range(), Some((1, 5)));
    }

    #[test]
    fn clear_focus_moves_cursor_outside_titles_and_clears_selection() {
        let mut doc = DocumentModel::from_lines(&["abcdef".into()]);
        doc.cursor_move_to_offset(3);
        doc.cursor_mut().selection_anchor = Some(1);

        apply_target_to_document(&mut doc, EditHitTarget::ClearFocus);

        assert_eq!(doc.cursor_offset().to_usize(), doc.buffer_len());
        assert!(doc.cursor().selection_anchor.is_none());
    }

    #[test]
    fn source_object_target_rejects_invalid_byte_and_grapheme_boundaries() {
        let mut doc = DocumentModel::from_lines(&["a👍🏼b".into()]);
        let invalid_ranges = [2..9, 1..5, std::ops::Range { start: 9, end: 1 }, 0..usize::MAX];

        for source_range in invalid_ranges {
            doc.cursor_move_to_offset(0);
            doc.cursor_mut().selection_anchor = None;

            apply_target_to_document(&mut doc, EditHitTarget::SourceObject { source_range });

            assert_eq!(doc.cursor_offset().to_usize(), 0);
            assert!(doc.cursor().selection_anchor.is_none());
        }
    }

    #[test]
    fn navigation_prefers_semantic_target_and_synchronizes_plugin_selection() {
        use crate::dispatch::wysiwyg::semantic_test_support::{
            RecordedSyncMessage, SemanticPluginState, SemanticQueryResponse,
            app_with_semantic_plugin,
        };
        use std::cell::RefCell;
        use std::rc::Rc;

        let state = Rc::new(RefCell::new(SemanticPluginState {
            move_target_response: SemanticQueryResponse::Target(Some(
                EditHitTarget::SourceObject { source_range: 1..5 },
            )),
            visual_move_result: Some(4),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());

        let effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveRight);

        assert_eq!(effect, AppEffect::REDRAW);
        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document.selection_range(), Some((1, 5)));
        let state = state.borrow();
        assert_eq!(state.queried_operations, ["move_target"]);
        assert!(state.sync_messages.ends_with(&[
            RecordedSyncMessage::Anchor(Some(1)),
            RecordedSyncMessage::SelectionCursor(Some(5)),
            RecordedSyncMessage::Cursor(5),
        ]));
    }

    #[test]
    fn navigation_falls_back_to_visual_move_when_semantic_target_is_absent() {
        use crate::dispatch::wysiwyg::semantic_test_support::{
            RecordedSyncMessage, SemanticPluginState, SemanticQueryResponse,
            app_with_semantic_plugin,
        };
        use std::cell::RefCell;
        use std::rc::Rc;

        let state = Rc::new(RefCell::new(SemanticPluginState {
            move_target_response: SemanticQueryResponse::Target(None),
            visual_move_result: Some(4),
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state.clone());

        let effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveRight);

        assert_eq!(effect, AppEffect::REDRAW);
        let entry = app.active_tab_session().expect("active entry");
        assert_eq!(entry.document.cursor_offset().to_usize(), 4);
        let state = state.borrow();
        assert_eq!(state.queried_operations, ["move_target", "visual_move"]);
        assert!(state.sync_messages.ends_with(&[
            RecordedSyncMessage::Cursor(4),
            RecordedSyncMessage::Anchor(None),
            RecordedSyncMessage::SelectionCursor(None),
        ]));
    }

    #[test]
    fn first_vertical_move_seeds_preferred_x_from_resolved_target_geometry() {
        use crate::dispatch::wysiwyg::semantic_test_support::{
            SemanticPluginState, SemanticQueryResponse, app_with_semantic_plugin,
        };
        use std::cell::RefCell;
        use std::rc::Rc;

        const TARGET_BYTE: usize = 4;
        const TARGET_X: f32 = 72.0;

        let state = Rc::new(RefCell::new(SemanticPluginState {
            move_target_response: SemanticQueryResponse::Target(None),
            visual_move_result: Some(TARGET_BYTE),
            cursor_screen_positions: vec![(TARGET_BYTE, TARGET_X)],
            ..SemanticPluginState::default()
        }));
        let mut app = app_with_semantic_plugin("abcdef", state);

        let effect = app.dispatch_wysiwyg_navigation(&EditCommand::MoveDown);

        assert_eq!(effect, AppEffect::REDRAW);
        assert_eq!(app.editor_runtime.preferred_x(), Some(TARGET_X));
    }
}
