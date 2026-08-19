//! WYSIWYG dispatch — visual navigation and plugin cursor synchronization.
//!
//! When the active plugin is a WYSIWYG editor (`handles_own_rendering() == true`),
//! the dispatch path in [`super::editor`] routes visual-navigation commands here
//! so the plugin can resolve movement targets from its own layout.
//!
//! Text-editing commands never reach this module: `dispatch_edit_command`
//! converts them to an [`ui::plugin::EditIntent`] up front and executes them
//! through the transactional edit pipeline (`crate::edit_transaction`), which
//! validates source generation, grapheme boundaries, and selection collapse
//! before applying one atomic transaction.

use crate::app::App;
use crate::app_effect::AppEffect;
#[cfg(test)]
use crate::document_view::DocumentView;
use crate::input::EditCommand;
use crate::tab_session::{TabSession, TabSessionMut};
use appkit_core::document::DocumentModel;
use core::types::ByteIndex;
use ui::plugin::{EditHitTarget, MoveDirection, PluginMessage};

fn wysiwyg_cursor_x(tab: &TabSession<'_>, byte: usize) -> Option<f32> {
    tab.query_cursor_screen_rect(byte).map(|(x, _y, _w, _h)| x)
}

// ────────────────────────────────────────────────────────────────────────────
// Visual navigation
// ────────────────────────────────────────────────────────────────────────────

impl App {
    /// Route a visual-navigation command to the WYSIWYG plugin.
    ///
    /// Queries [`PluginQuery::VisualMove`] to translate the requested direction
    /// into a target byte offset, then updates the DocumentView cursor.
    /// Returns `AppEffect::REDRAW` on success (no text modification → no
    /// cache invalidation needed).
    pub(crate) fn dispatch_wysiwyg_navigation(&mut self, cmd: &EditCommand) -> AppEffect {
        let extend_selection = matches!(
            cmd,
            EditCommand::ExtendLeft
                | EditCommand::ExtendRight
                | EditCommand::ExtendUp
                | EditCommand::ExtendDown
                | EditCommand::ExtendToLineStart
                | EditCommand::ExtendToLineEnd
                | EditCommand::ExtendToDocStart
                | EditCommand::ExtendToDocEnd
        );
        let direction = match cmd {
            EditCommand::MoveLeft | EditCommand::ExtendLeft => MoveDirection::Left,
            EditCommand::MoveRight | EditCommand::ExtendRight => MoveDirection::Right,
            EditCommand::MoveUp | EditCommand::ExtendUp => MoveDirection::Up,
            EditCommand::MoveDown | EditCommand::ExtendDown => MoveDirection::Down,
            EditCommand::MoveToLineStart | EditCommand::ExtendToLineStart => {
                MoveDirection::LineStart
            }
            EditCommand::MoveToLineEnd | EditCommand::ExtendToLineEnd => MoveDirection::LineEnd,
            EditCommand::MoveToDocStart | EditCommand::ExtendToDocStart => {
                return self.wysiwyg_navigate_to_doc_boundary(0, extend_selection);
            }
            EditCommand::MoveToDocEnd | EditCommand::ExtendToDocEnd => {
                return self.wysiwyg_navigate_to_doc_end(extend_selection);
            }
            EditCommand::PageUp => {
                return self.wysiwyg_page(-1);
            }
            EditCommand::PageDown => {
                return self.wysiwyg_page(1);
            }
            _ => return AppEffect::NONE,
        };
        let vertical_navigation = matches!(direction, MoveDirection::Up | MoveDirection::Down);
        let preferred_x = self.editor_runtime.preferred_x();

        // Phase 1: query the semantic target first, then retain byte navigation
        // as the fallback for existing Markdown WYSIWYG plugins.
        let (semantic_target, visual_target, selection_anchor, vertical_anchor_x) = {
            let Some(tab) = self.active_tab_session() else {
                return AppEffect::NONE;
            };
            let current_byte = tab.document.cursor_offset().to_usize();
            let vertical_anchor_x = if vertical_navigation {
                preferred_x.or_else(|| wysiwyg_cursor_x(&tab, current_byte))
            } else {
                None
            };
            let target_x = vertical_anchor_x;
            let selection_anchor = if extend_selection {
                Some(tab.document.cursor().selection_anchor.unwrap_or(current_byte))
            } else {
                None
            };
            let semantic_target = tab.move_edit_target(current_byte, direction, target_x);
            let visual_target = semantic_target
                .is_none()
                .then(|| tab.visual_move_byte(current_byte, direction, target_x));
            (semantic_target, visual_target.flatten(), selection_anchor, vertical_anchor_x)
        };

        // Phase 2: apply cursor + notify plugin with snapped byte (mutable borrow).
        let cursor_after = {
            let Some(mut tab) = self.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            if let Some(target) = semantic_target {
                apply_edit_hit_target(&mut tab, target);
            } else if let Some(new_byte) = visual_target {
                set_wysiwyg_cursor_and_selection(&mut tab, new_byte, selection_anchor);
            } else {
                return AppEffect::NONE;
            }
            tab.document.cursor_offset().to_usize()
        };

        if vertical_navigation {
            let resolved_anchor_x = vertical_anchor_x.or_else(|| {
                self.active_tab_session()
                    .and_then(|active_tab| wysiwyg_cursor_x(&active_tab, cursor_after))
            });
            self.editor_runtime.set_preferred_x(resolved_anchor_x);
            self.ensure_wysiwyg_cursor_visible();
        } else {
            self.editor_runtime.set_preferred_x(None);
        }

        AppEffect::REDRAW
    }

    /// Scroll the plugin viewport by the minimal delta that reveals the cursor
    /// after vertical navigation. No-op when the cursor is already inside the
    /// viewport (avoids scroll jitter).
    fn ensure_wysiwyg_cursor_visible(&mut self) {
        let viewport_h = self.plugin_viewport_h();
        let Some(delta) = self.wysiwyg_cursor_visibility_delta(viewport_h) else {
            return;
        };
        let Some(mut tab) = self.active_tab_session_mut() else {
            return;
        };
        tab.send_message(PluginMessage::Scroll { delta, viewport_h });
    }

    /// Minimal signed scroll delta needed to bring the cursor into the plugin
    /// viewport; `None` when the cursor is already visible or its on-screen
    /// geometry is unavailable.
    fn wysiwyg_cursor_visibility_delta(&self, viewport_h: f32) -> Option<f32> {
        let tab = self.active_tab_session()?;
        let cursor_byte = tab.document.cursor_offset().to_usize();
        let (_x, cursor_y, _w, cursor_h) = tab.query_cursor_screen_rect(cursor_byte)?;
        if cursor_y < 0.0 {
            return Some(cursor_y);
        }
        let overflow_below = cursor_y + cursor_h - viewport_h;
        (overflow_below > 0.0).then_some(overflow_below)
    }

    /// Move cursor to byte offset 0 (document start).
    fn wysiwyg_navigate_to_doc_boundary(
        &mut self,
        byte: usize,
        extend_selection: bool,
    ) -> AppEffect {
        let Some(mut tab) = self.active_tab_session_mut() else {
            return AppEffect::NONE;
        };
        let current_byte = tab.document.cursor_offset().to_usize();
        let selection_anchor = extend_selection
            .then(|| tab.document.cursor().selection_anchor.unwrap_or(current_byte));
        let _snapped = set_wysiwyg_cursor_and_selection(&mut tab, byte, selection_anchor);
        self.editor_runtime.set_preferred_x(None);
        AppEffect::REDRAW
    }

    /// Move cursor to the last byte of the document.
    fn wysiwyg_navigate_to_doc_end(&mut self, extend_selection: bool) -> AppEffect {
        let end =
            self.active_tab_session().map(|session| session.document.buffer_len()).unwrap_or(0);
        self.wysiwyg_navigate_to_doc_boundary(end, extend_selection)
    }

    /// Scroll one page up (`direction < 0`) or down (`direction > 0`).
    /// Uses plugin scroll message for accurate paging in WYSIWYG mode.
    fn wysiwyg_page(&mut self, direction: isize) -> AppEffect {
        let screen_h = self.screen_height();
        let line_height = self.ui_metrics().line_height;
        let viewport_h = self.visible_height_lines(screen_h) as f32 * line_height;
        let delta = if direction < 0 { -viewport_h } else { viewport_h };

        // Phase 1: scroll plugin (mutable borrow).
        {
            let Some(mut tab) = self.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            tab.send_message(PluginMessage::Scroll { delta, viewport_h });
        }

        // Phase 2: find cursor target via hit test (immutable borrow).
        let new_byte = {
            let Some(tab) = self.active_tab_session() else {
                return AppEffect::NONE;
            };
            tab.hit_test_byte(0.0, 0.0, 0.0, 0.0)
                .unwrap_or_else(|| tab.document.cursor_offset().to_usize())
        };

        // Phase 3: apply cursor + notify plugin with snapped byte (mutable borrow).
        let Some(mut tab) = self.active_tab_session_mut() else {
            return AppEffect::NONE;
        };
        let _snapped = set_wysiwyg_cursor_and_selection(&mut tab, new_byte, None);
        self.editor_runtime.set_preferred_x(None);
        AppEffect::REDRAW
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Cursor / selection synchronization helpers
// ────────────────────────────────────────────────────────────────────────────

pub(crate) fn set_wysiwyg_cursor_and_selection(
    tab: &mut TabSessionMut<'_>,
    requested_cursor_byte: usize,
    selection_anchor: Option<usize>,
) -> usize {
    tab.document.cursor_move_to_offset(requested_cursor_byte);
    let snapped_cursor_byte = tab.document.cursor_offset().to_usize();
    tab.document.cursor_mut().selection_anchor = selection_anchor;
    tab.send_message(PluginMessage::SetCursorByte(snapped_cursor_byte));
    tab.send_message(PluginMessage::SetSelAnchorByte(selection_anchor));
    tab.send_message(PluginMessage::SetSelCursorByte(
        selection_anchor.map(|_| snapped_cursor_byte),
    ));
    snapped_cursor_byte
}

pub(crate) fn apply_target_to_document(
    doc: &mut impl crate::edit_transaction::DocumentModelMut,
    target: EditHitTarget,
) {
    let doc = doc.document_model_mut();
    match target {
        EditHitTarget::TextCaret { byte_offset, .. } => {
            doc.cursor_move_to_offset(byte_offset);
            doc.cursor_mut().selection_anchor = None;
        }
        EditHitTarget::SourceObject { source_range } => {
            let Some(source_range) = validated_source_object_range(doc, source_range) else {
                return;
            };
            doc.cursor_move_to_offset(source_range.end);
            doc.cursor_mut().selection_anchor = Some(source_range.start);
        }
        EditHitTarget::CanvasControl { .. } => {}
        EditHitTarget::ClearFocus => {
            doc.cursor_move_to_offset(doc.buffer_len());
            doc.cursor_mut().selection_anchor = None;
        }
    }
}

fn validated_source_object_range(
    doc: &DocumentModel,
    source_range: std::ops::Range<usize>,
) -> Option<std::ops::Range<usize>> {
    if source_range.start > source_range.end || source_range.end > doc.buffer_len() {
        return None;
    }

    let source = doc.full_text();
    if !source.is_char_boundary(source_range.start) || !source.is_char_boundary(source_range.end) {
        return None;
    }

    let start = ByteIndex(source_range.start);
    let end = ByteIndex(source_range.end);
    if !doc.tb().is_grapheme_boundary(start) || !doc.tb().is_grapheme_boundary(end) {
        return None;
    }

    Some(source_range)
}

pub(crate) fn apply_edit_hit_target(tab: &mut TabSessionMut<'_>, target: EditHitTarget) {
    let clears_edit_focus = matches!(target, EditHitTarget::ClearFocus);
    apply_target_to_document(tab.document, target);
    let cursor_byte = tab.document.cursor_offset().to_usize();
    let selection_anchor = tab.document.cursor().selection_anchor;
    tab.send_message(PluginMessage::SetSelAnchorByte(selection_anchor));
    tab.send_message(PluginMessage::SetSelCursorByte(Some(cursor_byte)));
    tab.send_message(PluginMessage::SetCursorByte(cursor_byte));
    if clears_edit_focus {
        tab.send_message(PluginMessage::ClearEditFocus);
    }
}

#[cfg(test)]
pub(crate) mod semantic_test_support {
    use crate::app::App;
    use crate::document_view::DocumentView;

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
            _doc: &dyn core::document::DocView,
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

        fn query(&self, query: PluginQuery, _doc: &dyn core::document::DocView) -> PluginResponse {
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
            _doc: &mut dyn core::document::DocViewMut,
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
        let document = DocumentView::new(vec![text.to_string()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(SemanticTestPlugin::new(state)));
        app.switch_workspace_for_test(0);
        app
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::commands::execute_edit_command_v2;
    use crate::document_view::DocumentView;
    use crate::input::EditCommand;

    #[test]
    fn replace_range_empty_moves_insert_point_without_selecting_text() {
        let mut doc = DocumentView::new(vec!["# hello world".to_string()], 80, 10.0);
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
            doc.visible_lines_with_line_height(24.0),
            &["# hello world", "next"],
            "inserted text should land at range.start, not the old cursor"
        );
    }

    #[test]
    #[cfg(feature = "markdown")]
    fn markdown_augmented_insert_text_preserves_empty_separator_blocks() {
        let mut app = App::new(None);
        let mut doc =
            DocumentView::new("para1\n\npara2".split('\n').map(str::to_string).collect(), 80, 10.0);
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
        let mut doc =
            DocumentView::new("para1\n\npara2".split('\n').map(str::to_string).collect(), 80, 10.0);
        doc.cursor_move_to_offset(6);
        app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
        app.switch_workspace_for_test(0);
        app.sync_plugin_state();

        app.dispatch_transactional_edit_for_test(EditCommand::InsertText("中".into()));

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
    use ui::plugin::EditHitTarget;

    #[test]
    fn text_target_clears_selection_and_places_caret() {
        let mut doc = DocumentView::new(vec!["abcdef".into()], 80, 10.0);

        apply_target_to_document(
            &mut doc,
            EditHitTarget::TextCaret { byte_offset: 3, selection_scope: None },
        );

        assert_eq!(doc.cursor_offset().to_usize(), 3);
        assert!(doc.cursor().selection_anchor.is_none());
    }

    #[test]
    fn source_object_target_selects_exact_source_range() {
        let mut doc = DocumentView::new(vec!["abcdef".into()], 80, 10.0);

        apply_target_to_document(&mut doc, EditHitTarget::SourceObject { source_range: 1..5 });

        assert_eq!(doc.selection_range(), Some((1, 5)));
    }

    #[test]
    fn clear_focus_moves_cursor_outside_titles_and_clears_selection() {
        let mut doc = DocumentView::new(vec!["abcdef".into()], 80, 10.0);
        doc.cursor_move_to_offset(3);
        doc.cursor_mut().selection_anchor = Some(1);

        apply_target_to_document(&mut doc, EditHitTarget::ClearFocus);

        assert_eq!(doc.cursor_offset().to_usize(), doc.buffer_len());
        assert!(doc.cursor().selection_anchor.is_none());
    }

    #[test]
    fn source_object_target_rejects_invalid_byte_and_grapheme_boundaries() {
        let mut doc = DocumentView::new(vec!["a👍🏼b".into()], 80, 10.0);
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
