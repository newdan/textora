//! WYSIWYG edit dispatch — intercept → augment → relay.
//!
//! When the active plugin is a WYSIWYG editor (`handles_own_rendering() == true`), the
//! dispatch path in [`super::editor`] routes visual-navigation, smart-enter,
//! and smart-backspace commands here so they can query the plugin before the
//! standard edit pipeline executes.
//!
//! **Core invariant:** text-modifying commands MUST flow through
//! `execute_edit_command_v2` (via recursive `dispatch_edit_command`) to
//! produce the `Outcome` that drives `advance_cache` invalidation.

use crate::app::App;
use crate::app_effect::AppEffect;
#[cfg(test)]
use crate::document_view::DocumentView;
use crate::input::EditCommand;
use crate::tab_session::{TabSession, TabSessionMut};
use appkit_core::document::DocumentModel;
use core::types::ByteIndex;
use std::sync::OnceLock;
use ui::plugin::{AugmentKind, EditAugmentation, EditHitTarget, MoveDirection, PluginMessage};
use winit::event_loop::ActiveEventLoop;

const WYSIWYG_CURSOR_LOG_ENV: &str = "EDIT_PLUS_WYSIWYG_CURSOR_LOG";

fn wysiwyg_cursor_logging_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(WYSIWYG_CURSOR_LOG_ENV).is_ok_and(|value| {
            !matches!(value.as_str(), "" | "0" | "false" | "FALSE" | "off" | "OFF")
        })
    })
}

fn log_wysiwyg_cursor_state(label: &str, dv: &DocumentModel) {
    if !wysiwyg_cursor_logging_enabled() {
        return;
    }

    eprintln!(
        "[wysiwyg:cursor] {label} cursor={} selection_anchor={:?} selection_range={:?} buffer_len={}",
        dv.cursor_offset().to_usize(),
        dv.cursor().selection_anchor,
        dv.selection_range(),
        dv.buffer_len()
    );
}

fn log_wysiwyg_augmentation(
    kind: &AugmentKind,
    current_byte: usize,
    aug: Option<&EditAugmentation>,
) {
    if !wysiwyg_cursor_logging_enabled() {
        return;
    }

    match aug {
        Some(augmented) => eprintln!(
            "[wysiwyg:augment] kind={kind:?} current={} replace_range={:?} insert_len={:?} cursor_after={}",
            current_byte,
            augmented.replace_range,
            augmented.insert_text.as_ref().map(|text| text.len()),
            augmented.cursor_byte_after
        ),
        None => {
            eprintln!("[wysiwyg:augment] kind={kind:?} current={} augmentation=None", current_byte)
        }
    }
}

fn log_wysiwyg_effect(label: &str, effect: AppEffect) {
    if wysiwyg_cursor_logging_enabled() {
        eprintln!("[wysiwyg:effect] {label} {effect:?}");
    }
}

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

        if vertical_navigation {
            self.editor_runtime.set_preferred_x(vertical_anchor_x);
        } else {
            self.editor_runtime.set_preferred_x(None);
        }

        AppEffect::REDRAW
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
// Smart Enter / Backspace — augment → recurse
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

fn dispatch_wysiwyg_command(
    app: &mut App,
    command: EditCommand,
    event_loop: Option<&ActiveEventLoop>,
) -> AppEffect {
    match event_loop {
        Some(event_loop) => app.dispatch_edit_command(command, event_loop),
        None => {
            let Some(tab) = app.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            let mut tab = tab;
            let mut presentation = tab.take_presentation();
            let page_step_rows =
                presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
            let outcome = crate::commands::execute_edit_command_v2_with_presentation(
                &command,
                tab.document,
                &[],
                &mut presentation.cursor_render_state,
                page_step_rows,
            );
            tab.restore_presentation(presentation);
            if outcome.dirty_lines.is_some() { AppEffect::REDRAW } else { AppEffect::NONE }
        }
    }
}

/// Applies the text-editing part of a WYSIWYG augmentation as a single atomic
/// [`EditCommand::ReplaceRange`] (方案 2026-07-06 阶段 3b).
///
/// - `replace_range = Some(range) + insert_text = Some(text)`：合并到一次
///   ReplaceRange { range, text }。
/// - `replace_range = None + insert_text = Some(text)`：等价于 ReplaceRange
///   { range: cursor..cursor, text }。
/// - `replace_range = Some(range) + insert_text = Some("")`：ReplaceRange
///   { range, text: "" } → 纯删除。
/// - `replace_range = None + insert_text = None`：无源码变化（仅光标跳转，如
///   TableCell 场景），跳过命令派发。
/// - 其他情况（None/None with fallback）：走 fallback 命令。
fn execute_augmentation_text_change(
    app: &mut App,
    augmented: &EditAugmentation,
    fallback: EditCommand,
    current_byte: usize,
    event_loop: Option<&ActiveEventLoop>,
) -> AppEffect {
    let result = AppEffect::NONE;
    match (augmented.replace_range.as_ref(), augmented.insert_text.as_ref()) {
        (None, None) => result.merge(dispatch_wysiwyg_command(app, fallback, event_loop)),
        (None, Some(text)) if text.is_empty() => result,
        (range_opt, Some(text)) => {
            let range = range_opt.cloned().unwrap_or(current_byte..current_byte);
            result.merge(dispatch_wysiwyg_command(
                app,
                EditCommand::ReplaceRange { range, text: text.clone() },
                event_loop,
            ))
        }
        (Some(range), None) => result.merge(dispatch_wysiwyg_command(
            app,
            EditCommand::ReplaceRange { range: range.clone(), text: String::new() },
            event_loop,
        )),
    }
}

fn cursor_changed_after_augmentation(dv: &DocumentModel, cursor_byte_after: usize) -> bool {
    dv.cursor_offset().to_usize() != cursor_byte_after
}

impl App {
    fn dispatch_wysiwyg_augmented_edit(
        &mut self,
        event_loop: Option<&ActiveEventLoop>,
        kind: AugmentKind,
        fallback: EditCommand,
    ) -> AppEffect {
        let current_byte = match self.active_tab_session() {
            Some(tab) => tab.document.cursor_offset().to_usize(),
            None => return dispatch_wysiwyg_command(self, fallback, event_loop),
        };

        let aug = self.wysiwyg_query_augment(current_byte, kind.clone());
        log_wysiwyg_augmentation(&kind, current_byte, aug.as_ref());
        self.editor_runtime.set_wysiwyg_recursing(true);
        let mut result = AppEffect::NONE;

        if let Some(augmented) = aug {
            result = result.merge(execute_augmentation_text_change(
                self,
                &augmented,
                fallback,
                current_byte,
                event_loop,
            ));
            log_wysiwyg_effect("augment.after_text_change", result);

            if let Some(tab) = self.active_tab_session_mut()
                && cursor_changed_after_augmentation(tab.document, augmented.cursor_byte_after)
            {
                tab.document.cursor_move_to_offset(augmented.cursor_byte_after);
                tab.document.cursor_mut().selection_anchor = None;
                result = result.merge(AppEffect::REDRAW);
            }
            if let Some(tab) = self.active_tab_session() {
                log_wysiwyg_cursor_state("augment.before_sync", tab.document);
            }
        } else {
            result = result.merge(dispatch_wysiwyg_command(self, fallback, event_loop));
            log_wysiwyg_effect("augment.fallback", result);
        }

        self.editor_runtime.set_wysiwyg_recursing(false);
        self.sync_plugin_state();
        if let Some(tab) = self.active_tab_session() {
            log_wysiwyg_cursor_state("augment.after_sync", tab.document);
        }
        result
    }

    /// Smart Enter for WYSIWYG: query the plugin for an edit augmentation and
    /// fall back to the standard newline command when none is available.
    pub(crate) fn dispatch_wysiwyg_augmented_enter(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect {
        self.dispatch_wysiwyg_augmented_edit(
            Some(event_loop),
            AugmentKind::Enter,
            EditCommand::InsertNewline,
        )
    }

    /// Smart Backspace for WYSIWYG: query the plugin for an edit augmentation
    /// and fall back to the standard backspace command when none is available.
    pub(crate) fn dispatch_wysiwyg_augmented_backspace(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect {
        self.dispatch_wysiwyg_augmented_edit(
            Some(event_loop),
            AugmentKind::Backspace,
            EditCommand::Backspace,
        )
    }

    /// Smart InsertText for WYSIWYG: query the plugin for an edit augmentation
    /// and fall back to the standard insert command when none is available.
    pub(crate) fn dispatch_wysiwyg_augmented_insert_text(
        &mut self,
        text: String,
        fallback: EditCommand,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect {
        self.dispatch_wysiwyg_augmented_edit(
            Some(event_loop),
            AugmentKind::InsertText(text),
            fallback,
        )
    }

    /// Shared helper: query the active WYSIWYG plugin for an edit augmentation.
    fn wysiwyg_query_augment(
        &self,
        current_byte: usize,
        kind: AugmentKind,
    ) -> Option<EditAugmentation> {
        let tab = self.active_tab_session()?;
        tab.augment_edit(current_byte, kind)
    }
}

#[cfg(test)]
impl App {
    pub(crate) fn dispatch_wysiwyg_augmented_enter_for_test(&mut self) -> AppEffect {
        self.dispatch_wysiwyg_augmented_edit(None, AugmentKind::Enter, EditCommand::InsertNewline)
    }

    pub(crate) fn dispatch_wysiwyg_augmented_backspace_for_test(&mut self) -> AppEffect {
        self.dispatch_wysiwyg_augmented_edit(None, AugmentKind::Backspace, EditCommand::Backspace)
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

    use ui::plugin::AugmentKind;

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

        let effect = app.dispatch_wysiwyg_augmented_edit(
            None,
            AugmentKind::InsertText("中".into()),
            EditCommand::InsertText("中".into()),
        );

        let doc = &app.active_tab_session().expect("active entry").document;
        assert!(effect.redraw, "WYSIWYG content must redraw after augmented text input");
        assert_eq!(doc.full_text(), "para1\n\n中\n\npara2");
        assert_eq!(doc.cursor_offset().to_usize(), "para1\n\n中".len());
    }

    /// 阶段 3b 护栏：augment 派发后不能留下 selection anchor。
    /// 老代码依赖 position_document_for_wysiwyg_replace_range 展开 anchor，
    /// 然后期望 InsertText 覆盖它；有边界情况 anchor 会被保留下来。
    /// ReplaceRange 是原子命令，且尾部显式清空 anchor。
    #[test]
    #[cfg(feature = "markdown")]
    fn augment_dispatch_leaves_no_selection_after_replace_range() {
        let mut app = App::new(None);
        let mut doc =
            DocumentView::new("para1\n\npara2".split('\n').map(str::to_string).collect(), 80, 10.0);
        doc.cursor_move_to_offset(6);
        app.push_entry_for_test(doc, Box::new(textora_markdown::view::MarkdownEditorView::new()));
        app.switch_workspace_for_test(0);
        app.sync_plugin_state();

        app.dispatch_wysiwyg_augmented_edit(
            None,
            AugmentKind::InsertText("中".into()),
            EditCommand::InsertText("中".into()),
        );

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
}
